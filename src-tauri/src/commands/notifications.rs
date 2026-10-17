//! Mail notification preferences and the delivery dispatcher.
//!
//! # The path from ingestion to a banner
//!
//! 1. `sync::message_ingest::upsert_provider_message` inserts into `messages`.
//! 2. The SQLite trigger `messages_notification_outbox_ai` (migration 0060)
//!    writes a `notification_outbox` row in the *same* transaction — `pending`
//!    when the message is notifiable, `dismissed` with a reason otherwise.
//! 3. `publish_pending_mail_notifications` claims pending rows, re-checks the
//!    mailbox state, and publishes `AppEvent::NewMail`.
//! 4. `lib.rs`'s event bridge calls [`show_mail_notification`], which takes the
//!    native macOS path (`utils::mac_notify`) or falls back to
//!    `tauri-plugin-notification`.
//! 5. The row ends as `delivered` or `dismissed`.
//!
//! # Every branch that ends in no notification
//!
//! Each one records `last_error = "<reason_code>: <detail>"` on the outbox row,
//! so „warum hat Satchel das nicht gezeigt?" is answerable from the Diagnose
//! pane instead of from a log nobody kept.
//!
//! Decided by the trigger, before a row would otherwise exist:
//!
//! - `filter_own_sent` — `is_sent`, `is_draft`, or the sender is the account
//!   itself.
//! - `filter_account_inactive` — account inactive, or not `kind = 'email'`.
//! - `filter_not_inbox` — the message's folder is not the account's Inbox.
//! - `setting_disabled` — `notifications.enabled` off, or
//!   `notifications.scope = 'off'`.
//! - `filter_no_date` — the receive timestamp or fallback date does not parse.
//! - `filter_too_old` — the arrival predates the previous successful INBOX fetch.
//!   Mail older than that watermark by more than a day writes no row at all:
//!   that is backfill, and a row per backfilled message would double the
//!   mailbox on disk.
//!
//! Decided by the dispatcher, on a row that already exists:
//!
//! - `filter_before_session` — mail predates this app session, including retries
//!   left by the previous process. This filter applies only to foreground delivery.
//! - `setting_disabled` — the preference changed between ingestion and delivery.
//! - `filter_message_gone` — the message was deleted before delivery.
//! - `filter_account_inactive`, `filter_not_inbox` — moved or disabled since.
//! - `filter_own_sent` — reclassified since ingestion.
//! - `filter_already_read` — read on another client first. This is the normal
//!   case for a phone that got there before the desktop.
//! - `filter_muted_thread` — `threads.is_muted`.
//! - `filter_muted_sender` — a `blocked_senders` address or domain pattern.
//! - `dispatch_timeout` — claimed but never acknowledged for ten minutes,
//!   because the process died or the event bus dropped the event under load
//!   (`broadcast` lag). The row returns to `pending` and is retried.
//!
//! Decided at the moment of delivery:
//!
//! - `permission_denied` — macOS refused notification authorization. The row is
//!   dismissed; nothing was shown.
//! - `not_bundled_fallback` — no `.app` bundle (`cargo tauri dev`), or
//!   `UNUserNotificationCenter` was unavailable. The banner *is* shown through
//!   the plugin, but clicks and quick actions do nothing.
//! - `dispatch_error` — the notification backend returned an error. The row goes
//!   back to `pending` and the next sync retries it.
//!
//! Not a branch here, but the same user-visible symptom: a provider rate-limit
//! block stops the sync run, so the message is never ingested and no outbox row
//! exists. That block is its own section in the Diagnose pane.
//!
//! - `filter_coalesced` — (desktop) more than [`BURST_LIMIT`] mails within
//!   [`BURST_WINDOW`]. The first ones get their banners; the rest share one
//!   summary banner ("{count} new emails") instead of one each.
//!
//! There are deliberately no quiet hours and no per-thread "already notified"
//! dedupe. The burst limit is the only rate limit, and every notifiable
//! message still gets its own row.

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;

use crate::{
    commands::error::CommandResult,
    events::bus::{AppEvent, EventBus},
    utils::mime::decode_html_entities,
};

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/notifications/")
)]
pub struct PendingMailNotification {
    pub event_id: String,
    pub thread_id: String,
    pub account_id: String,
    pub account_label: String,
    pub from_name: Option<String>,
    pub from_address: String,
    pub subject: String,
    pub preview: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/notifications/")
)]
pub struct NotificationPreferences {
    /// Master toggle.
    pub enabled: bool,
    /// "all" | "inbox" | "off"
    pub scope: String,
    pub sound: bool,
    /// Number of seconds the user has to undo a send before the network call fires.
    /// 0 disables Undo Send.
    pub undo_send_seconds: i64,
    /// Number of days a trashed thread stays around before the maintenance worker
    /// hard-deletes it. 0 disables auto-cleanup.
    pub trash_auto_delete_days: i64,
    /// Whether to add a `Disposition-Notification-To` header to outgoing mail
    /// requesting a read receipt. Off by default for privacy.
    pub send_read_receipts: bool,
    /// How many days of mail are stored with their body. 0 keeps everything.
    ///
    /// Older mail keeps its sender, subject, date and place in the thread — all
    /// of that comes from the header — and fetches its body when opened. A
    /// phone defaults to a month — what Gmail and Outlook Mobile both do —
    /// because a mailbox stored whole costs what a mailbox costs; a desktop
    /// keeps everything, which is what offline search over years of mail rests
    /// on.
    pub body_window_days: i64,
}

impl Default for NotificationPreferences {
    fn default() -> Self {
        Self {
            enabled: true,
            scope: "inbox".into(),
            sound: true,
            undo_send_seconds: 5,
            trash_auto_delete_days: 30,
            send_read_receipts: false,
            body_window_days: default_body_window_days(),
        }
    }
}

/// A month on a phone, everything on a desktop.
///
/// Thirty days is what Gmail for Android and Outlook Mobile both default to,
/// and Outlook refuses to go beyond ninety at all.
fn default_body_window_days() -> i64 {
    if cfg!(any(target_os = "android", target_os = "ios")) {
        30
    } else {
        0
    }
}

/// Stable snake_case reason codes stored as the prefix of
/// `notification_outbox.last_error`. The Diagnose pane groups on them, so they
/// are renamed only together with its labels.
pub mod reason {
    pub const SETTING_DISABLED: &str = "setting_disabled";
    pub const FILTER_MESSAGE_GONE: &str = "filter_message_gone";
    pub const FILTER_ACCOUNT_INACTIVE: &str = "filter_account_inactive";
    pub const FILTER_NOT_INBOX: &str = "filter_not_inbox";
    pub const FILTER_OWN_SENT: &str = "filter_own_sent";
    pub const FILTER_ALREADY_READ: &str = "filter_already_read";
    pub const FILTER_MUTED_THREAD: &str = "filter_muted_thread";
    pub const FILTER_MUTED_SENDER: &str = "filter_muted_sender";
    /// Trigger-only (migration 0060); listed here so the pane's label set is
    /// complete.
    pub const FILTER_NO_DATE: &str = "filter_no_date";
    pub const FILTER_TOO_OLD: &str = "filter_too_old";
    pub const FILTER_BEFORE_SESSION: &str = "filter_before_session";
    pub const DISPATCH_TIMEOUT: &str = "dispatch_timeout";
    pub const DISPATCH_ERROR: &str = "dispatch_error";
    pub const PERMISSION_DENIED: &str = "permission_denied";
    pub const NOT_BUNDLED_FALLBACK: &str = "not_bundled_fallback";
    pub const FILTER_COALESCED: &str = "filter_coalesced";
}

/// Mails a burst may show individually before the summary takes over.
pub const BURST_LIMIT: usize = 5;
/// How long a burst lasts after its latest mail.
pub const BURST_WINDOW: std::time::Duration = std::time::Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BurstDecision {
    Deliver,
    /// Show the summary instead; `count` mails of this burst went without a banner.
    Coalesce {
        count: u32,
    },
}

/// Counts deliveries in a sliding window. A mailing list dropping 200 mails
/// otherwise filled the notification centre with 200 entries.
#[derive(Debug, Default)]
pub struct BurstCoalescer {
    recent: std::collections::VecDeque<std::time::Instant>,
    coalesced: u32,
}

impl BurstCoalescer {
    pub fn decide(&mut self, now: std::time::Instant) -> BurstDecision {
        while self
            .recent
            .front()
            .is_some_and(|at| now.saturating_duration_since(*at) >= BURST_WINDOW)
        {
            self.recent.pop_front();
        }
        if self.recent.is_empty() {
            self.coalesced = 0;
        }
        self.recent.push_back(now);
        if self.recent.len() <= BURST_LIMIT {
            return BurstDecision::Deliver;
        }
        self.coalesced += 1;
        BurstDecision::Coalesce {
            count: self.coalesced,
        }
    }
}

/// The process-wide burst state for mail banners.
pub fn decide_mail_burst() -> BurstDecision {
    static COALESCER: std::sync::Mutex<BurstCoalescer> = std::sync::Mutex::new(BurstCoalescer {
        recent: std::collections::VecDeque::new(),
        coalesced: 0,
    });
    COALESCER
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .decide(std::time::Instant::now())
}

/// One banner for the mails of a burst; each call replaces the previous one.
pub fn show_mail_summary(app: &AppHandle, count: u32, sound: bool) {
    let text = crate::commands::ui_strings::current()
        .notification_summary
        .replace("{count}", &count.to_string());
    #[cfg(target_os = "macos")]
    {
        use crate::utils::mac_notify::NativeDelivery;
        match crate::utils::mac_notify::deliver_summary(&text, sound) {
            NativeDelivery::Delivered | NativeDelivery::PermissionDenied => return,
            NativeDelivery::NotBundled | NativeDelivery::Unavailable => {}
        }
    }
    let _ = sound;
    show_notification(app, &text, "");
}

/// `<reason_code>: <detail>` — the one format the Diagnose pane parses.
pub fn reason_note(code: &str, detail: impl std::fmt::Display) -> String {
    format!("{code}: {detail}")
}

/// What actually happened when a mail notification was handed to the OS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MailDeliveryOutcome {
    /// Shown natively. Clicks and quick actions reach the app.
    Native,
    /// Shown, but through a path that cannot report clicks. Carries the
    /// reason note explaining why the native path was skipped.
    Fallback(String),
    /// Not shown at all. Carries the reason note.
    Suppressed(String),
}

/// What the Android plugin's answer for one mail means for the outbox row.
///
/// The plugin resolves for a notification it dropped as well and says so:
/// `{"shown": false, "reason": "permission_denied"}` when the user refused
/// `POST_NOTIFICATIONS`. Same reason code as the macOS refusal, so the Diagnose
/// page needs no second case. An answer without `shown` predates this contract
/// and means the mail was posted — an old plugin must not turn every delivery
/// into a suppression.
pub fn android_delivery_outcome(response: &serde_json::Value) -> MailDeliveryOutcome {
    if response
        .get("shown")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(true)
    {
        return MailDeliveryOutcome::Native;
    }
    match response.get("reason").and_then(serde_json::Value::as_str) {
        Some(reason::PERMISSION_DENIED) => MailDeliveryOutcome::Suppressed(reason_note(
            reason::PERMISSION_DENIED,
            "Android POST_NOTIFICATIONS permission refused",
        )),
        Some(other) => MailDeliveryOutcome::Suppressed(reason_note(
            reason::DISPATCH_ERROR,
            format!("Android did not show the notification: {other}"),
        )),
        None => MailDeliveryOutcome::Suppressed(reason_note(
            reason::DISPATCH_ERROR,
            "Android did not show the notification and gave no reason",
        )),
    }
}

/// Apply what happened to one mail notification to its outbox row.
///
/// The foreground event bridge and the headless Android worker both end here,
/// so a refusal is `permission_denied` on either path instead of one path
/// recording a delivery and the other a `dispatch_timeout` ten minutes later.
/// `dispatch_error` returns to `pending`; everything else is terminal, because
/// a permission the user refused does not become granted by retrying.
pub async fn record_mail_delivery_outcome(
    pool: &SqlitePool,
    event_id: &str,
    outcome: &MailDeliveryOutcome,
) -> Result<(), String> {
    let ids = [event_id.to_string()];
    match outcome {
        MailDeliveryOutcome::Native => {
            mark_mail_notification_delivered_with_note(pool, &ids, None).await
        }
        MailDeliveryOutcome::Fallback(note) => {
            mark_mail_notification_delivered_with_note(pool, &ids, Some(note)).await
        }
        MailDeliveryOutcome::Suppressed(note) if note.starts_with(reason::DISPATCH_ERROR) => {
            mark_mail_notification_failed(pool, event_id, note).await
        }
        MailDeliveryOutcome::Suppressed(note) => {
            mark_mail_notification_suppressed(pool, event_id, note).await
        }
    }
}

/// Reconcile the acknowledgements the headless Android worker sends back.
///
/// Each entry is one `showMail` answer — `{"eventId": …, "shown": …,
/// "reason": …}` — read by the same [`android_delivery_outcome`] the
/// foreground plugin path uses. The worker used to send bare event ids and
/// acknowledge every one of them, which marked a notification Android never
/// posted as delivered.
pub async fn record_android_worker_outcomes(
    pool: &SqlitePool,
    acknowledgements: &[serde_json::Value],
) -> Result<(), String> {
    for entry in acknowledgements {
        let Some(event_id) = entry.get("eventId").and_then(serde_json::Value::as_str) else {
            return Err("Android acknowledgement without an eventId".to_string());
        };
        record_mail_delivery_outcome(pool, event_id, &android_delivery_outcome(entry)).await?;
    }
    Ok(())
}

/// Category the mail actions hang off. Must match what the frontend registers.
pub const MAIL_ACTION_TYPE: &str = "satchel.mail";
/// Group key so a morning's mail stacks instead of filling the shade.
pub const MAIL_GROUP: &str = "satchel.mail.incoming";
pub fn show_notification(app: &AppHandle, title: &str, body: &str) {
    let title = decode_html_entities(title);
    let body = decode_html_entities(body);
    if let Err(err) = app
        .notification()
        .builder()
        .title(&title)
        .body(&body)
        .show()
    {
        tracing::warn!(target: "notify", "failed: {err}");
    }
}

/// Show a notification for a new mail that navigates to `thread_id` when clicked.
///
/// On macOS the native `UserNotifications` path carries the thread id in the
/// notification's `userInfo`, so the click lands on the right conversation no
/// matter how many notifications are pending or how much later it arrives — see
/// `utils::mac_notify`. Everywhere else (and in unbundled dev builds, where the
/// framework is unavailable) this falls back to `tauri-plugin-notification`,
/// which cannot report clicks at all.
pub async fn show_mail_notification(
    app: &AppHandle,
    event_id: &str,
    account_id: &str,
    account_label: &str,
    sender: &str,
    subject: &str,
    preview: &str,
    thread_id: &str,
    sound: bool,
) -> Result<MailDeliveryOutcome, String> {
    let sender = decode_html_entities(sender);
    let subject = decode_html_entities(subject);
    let preview = decode_html_entities(preview);

    #[cfg(target_os = "android")]
    {
        // Handing the mail over can succeed while Android still shows nothing.
        // The answer says which of the two happened.
        let response = crate::commands::mobile_notifications::show_android_mail_notification(
            app,
            event_id,
            account_id,
            account_label,
            &sender,
            &subject,
            &preview,
            thread_id,
            sound,
        )
        .await?;
        return Ok(android_delivery_outcome(&response));
    }

    #[cfg(not(target_os = "android"))]
    {
        let body = format!("{subject}\n{preview}");

        #[cfg(not(target_os = "macos"))]
        let fallback_reason: Option<String> = None;

        #[cfg(target_os = "macos")]
        let fallback_reason: Option<String> = {
            use crate::utils::mac_notify::NativeDelivery;
            match crate::utils::mac_notify::deliver(
                &sender,
                &body,
                thread_id,
                account_id,
                account_label,
                sound,
            ) {
                NativeDelivery::Delivered => return Ok(MailDeliveryOutcome::Native),
                NativeDelivery::PermissionDenied => {
                    return Ok(MailDeliveryOutcome::Suppressed(reason_note(
                        reason::PERMISSION_DENIED,
                        "UNUserNotificationCenter authorization refused",
                    )))
                }
                NativeDelivery::NotBundled => Some(reason_note(
                    reason::NOT_BUNDLED_FALLBACK,
                    "no .app bundle — click and quick actions are inert",
                )),
                NativeDelivery::Unavailable => Some(reason_note(
                    reason::NOT_BUNDLED_FALLBACK,
                    "UNUserNotificationCenter unavailable — click and quick actions are inert",
                )),
            }
        };

        // The category the frontend registered its actions under, the thread id the
        // action needs to act on, and a group so the system stacks a morning's mail
        // instead of listing it. Ignored by backends that do not support them.
        let mut builder = app
            .notification()
            .builder()
            .title(&sender)
            .body(&body)
            .large_body(&body)
            .action_type_id(MAIL_ACTION_TYPE)
            .group(MAIL_GROUP)
            .extra("threadId", thread_id)
            .auto_cancel();
        if sound {
            builder = builder.sound("default");
        }
        builder
            .show()
            .map_err(|error| format!("show mail notification: {error}"))?;

        let _ = (event_id, account_id, account_label);
        Ok(match fallback_reason {
            Some(note) => MailDeliveryOutcome::Fallback(note),
            None => MailDeliveryOutcome::Native,
        })
    }
}

/// One reason the dispatcher can dismiss a claimed-but-not-yet-shown row.
///
/// `detail` and `predicate` are SQL fragments evaluated per row. Keeping the
/// detail in SQL is what makes the Diagnose entry name the folder or the sender
/// that decided the case instead of repeating the reason code.
struct DismissalRule {
    code: &'static str,
    detail: &'static str,
    predicate: &'static str,
}

/// Evaluated in order; the first match owns the row, so the most specific
/// reason wins. Every later rule only sees rows still `pending`.
const DISMISSAL_RULES: &[DismissalRule] = &[
    DismissalRule {
        code: reason::FILTER_MESSAGE_GONE,
        detail: "notification_outbox.message_id",
        predicate: "NOT EXISTS (SELECT 1 FROM messages m
                                WHERE m.id = notification_outbox.message_id)",
    },
    DismissalRule {
        code: reason::FILTER_ACCOUNT_INACTIVE,
        detail: "(SELECT m.account_id FROM messages m
                  WHERE m.id = notification_outbox.message_id)",
        predicate: "NOT EXISTS (SELECT 1 FROM messages m
                                JOIN accounts a ON a.id = m.account_id
                                WHERE m.id = notification_outbox.message_id
                                  AND a.is_active = 1)",
    },
    DismissalRule {
        code: reason::FILTER_NOT_INBOX,
        detail: "(SELECT COALESCE(NULLIF(f.folder_type, ''), f.full_path, m.folder_id)
                  FROM messages m LEFT JOIN folders f ON f.id = m.folder_id
                  WHERE m.id = notification_outbox.message_id)",
        predicate: "NOT EXISTS (SELECT 1 FROM messages m
                                JOIN folders f ON f.id = m.folder_id
                                WHERE m.id = notification_outbox.message_id
                                  AND LOWER(COALESCE(f.folder_type, '')) = 'inbox')",
    },
    DismissalRule {
        code: reason::FILTER_OWN_SENT,
        detail: "(SELECT CASE WHEN COALESCE(m.is_sent, 0) <> 0 THEN 'is_sent' ELSE 'is_draft' END
                  FROM messages m WHERE m.id = notification_outbox.message_id)",
        predicate: "EXISTS (SELECT 1 FROM messages m
                            WHERE m.id = notification_outbox.message_id
                              AND (COALESCE(m.is_sent, 0) <> 0 OR COALESCE(m.is_draft, 0) <> 0))",
    },
    DismissalRule {
        code: reason::FILTER_ALREADY_READ,
        detail: "(SELECT m.from_address FROM messages m
                  WHERE m.id = notification_outbox.message_id)",
        predicate: "EXISTS (SELECT 1 FROM messages m
                            WHERE m.id = notification_outbox.message_id
                              AND COALESCE(m.is_read, 0) <> 0)",
    },
    DismissalRule {
        code: reason::FILTER_MUTED_THREAD,
        detail: "(SELECT m.thread_id FROM messages m
                  WHERE m.id = notification_outbox.message_id)",
        predicate: "EXISTS (SELECT 1 FROM messages m
                            JOIN threads t ON t.id = m.thread_id
                            WHERE m.id = notification_outbox.message_id
                              AND COALESCE(t.is_muted, 0) <> 0)",
    },
    DismissalRule {
        code: reason::FILTER_MUTED_SENDER,
        detail: "(SELECT m.from_address FROM messages m
                  WHERE m.id = notification_outbox.message_id)",
        predicate: "EXISTS (
                      SELECT 1 FROM messages m
                      JOIN blocked_senders b
                        ON (b.account_id IS NULL OR b.account_id = m.account_id)
                       AND (
                         (b.pattern_type = 'address'
                          AND LOWER(TRIM(b.pattern)) = LOWER(TRIM(m.from_address)))
                         OR
                         (b.pattern_type = 'domain'
                          AND INSTR(TRIM(m.from_address), '@') > 0
                          AND LOWER(LTRIM(LTRIM(TRIM(b.pattern), '*'), '@')) =
                              LOWER(SUBSTR(TRIM(m.from_address),
                                           INSTR(TRIM(m.from_address), '@') + 1)))
                       )
                      WHERE m.id = notification_outbox.message_id)",
    },
];

/// Dismiss every still-pending row the rule matches, recording its reason.
async fn dismiss_with_reason(
    pool: &SqlitePool,
    now: &str,
    message_id: Option<&str>,
    rule: &DismissalRule,
) -> Result<u64, String> {
    let sql = format!(
        "UPDATE notification_outbox
         SET state = 'dismissed', dispatch_token = NULL, dispatch_at = NULL,
             delivered_at = ?,
             last_error = '{code}: ' || COALESCE({detail}, '?')
         WHERE state = 'pending'
           AND (? IS NULL OR message_id = ?)
           AND ({predicate})",
        code = rule.code,
        detail = rule.detail,
        predicate = rule.predicate,
    );
    let affected = sqlx::query(&sql)
        .bind(now)
        .bind(message_id)
        .bind(message_id)
        .execute(pool)
        .await
        .map_err(|error| format!("dismiss notification intents ({}): {error}", rule.code))?
        .rows_affected();
    if affected > 0 {
        tracing::debug!(
            target: "notify",
            reason = rule.code,
            dismissed = affected,
            "mail notifications suppressed"
        );
    }
    Ok(affected)
}

/// Bound the outbox. Runs only on a full pass — a per-message call happens once
/// per ingested message, and a backfill would turn this into a quadratic scan.
async fn trim_notification_outbox(pool: &SqlitePool) {
    let _ = sqlx::query(
        "DELETE FROM notification_outbox
         WHERE state <> 'pending'
           AND julianday(COALESCE(delivered_at, created_at)) < julianday('now', '-7 days')",
    )
    .execute(pool)
    .await;
    // A ring buffer for the Diagnose pane: enough history to explain a morning
    // of missing notifications, not enough to grow with the mailbox.
    let _ = sqlx::query(
        "DELETE FROM notification_outbox
         WHERE state = 'dismissed'
           AND message_id NOT IN (
             SELECT message_id FROM notification_outbox
             WHERE state = 'dismissed'
             ORDER BY created_at DESC, message_id DESC
             LIMIT 2000
           )",
    )
    .execute(pool)
    .await;
}

/// INTERNALDATE has second precision; overlap within that second is idempotent.
pub(crate) fn notification_cutoff_at(started_at: chrono::DateTime<chrono::Utc>) -> String {
    started_at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Captured before workers start, so opening the app cannot replay offline mail.
/// Kept in memory: a persisted sync watermark describes the previous session.
pub fn notification_session_started_at() -> &'static str {
    static STARTED_AT: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    STARTED_AT.get_or_init(|| notification_cutoff_at(chrono::Utc::now()))
}

/// Return durable notification intents that still match the current mailbox
/// state. Reading or filing a message on another client before delivery makes
/// its pending row dismissible instead of producing a stale alert.
///
/// Every row this function drops keeps its reason code in `last_error`; see the
/// module docs for the full list.
pub async fn pending_mail_notifications(
    pool: &SqlitePool,
    message_id: Option<&str>,
) -> Result<Vec<PendingMailNotification>, String> {
    pending_mail_notifications_since(pool, message_id, Some(notification_session_started_at()))
        .await
}

/// Headless workers use the persisted Inbox cutoff without a foreground session.
pub async fn pending_headless_mail_notifications(
    pool: &SqlitePool,
) -> Result<Vec<PendingMailNotification>, String> {
    pending_mail_notifications_since(pool, None, None).await
}

async fn pending_mail_notifications_since(
    pool: &SqlitePool,
    message_id: Option<&str>,
    session_started_at: Option<&str>,
) -> Result<Vec<PendingMailNotification>, String> {
    let now = chrono::Utc::now().to_rfc3339();
    if message_id.is_none() {
        trim_notification_outbox(pool).await;
    }
    let preferences = load_preferences(pool).await;
    if !preferences.enabled || preferences.scope.eq_ignore_ascii_case("off") {
        let detail = if preferences.enabled {
            "notifications.scope=off"
        } else {
            "notifications.enabled"
        };
        let dismissed = sqlx::query(
            "UPDATE notification_outbox
             SET state = 'dismissed', dispatch_token = NULL, dispatch_at = NULL,
                 delivered_at = ?, last_error = ?
             WHERE state IN ('pending', 'dispatching')
               AND (? IS NULL OR message_id = ?)",
        )
        .bind(&now)
        .bind(reason_note(reason::SETTING_DISABLED, detail))
        .bind(message_id)
        .bind(message_id)
        .execute(pool)
        .await
        .map_err(|error| format!("dismiss disabled notification intents: {error}"))?
        .rows_affected();
        tracing::debug!(
            target: "notify",
            reason = reason::SETTING_DISABLED,
            %detail,
            dismissed,
            "mail notifications suppressed"
        );
        return Ok(Vec::new());
    }
    if let Some(session_started_at) = session_started_at {
        sqlx::query(
        "UPDATE notification_outbox
         SET state = 'dismissed', dispatch_token = NULL, dispatch_at = NULL,
             delivered_at = ?, last_error = ?
         WHERE state IN ('pending', 'dispatching')
           AND (? IS NULL OR message_id = ?)
           AND EXISTS (SELECT 1 FROM messages m
                       WHERE m.id = notification_outbox.message_id
                         AND (julianday(COALESCE(m.received_at, m.date)) IS NULL OR julianday(COALESCE(m.received_at, m.date)) < julianday(?)))",
    )
    .bind(&now)
    .bind(reason_note(
        reason::FILTER_BEFORE_SESSION,
        session_started_at,
    ))
    .bind(message_id)
    .bind(message_id)
    .bind(session_started_at)
    .execute(pool)
    .await
    .map_err(|error| format!("dismiss mail from before this session: {error}"))?;
    }

    // A claim that was never acknowledged: the process died between claiming and
    // showing, or the event bus dropped the event under load. Re-pend it, but
    // leave the reason on the row — a repeating `dispatch_timeout` is the only
    // trace that delivery, not filtering, is what swallows the notifications.
    let timed_out = sqlx::query(
        "UPDATE notification_outbox
         SET state = 'pending', dispatch_token = NULL, dispatch_at = NULL,
             last_error = ?
         WHERE state = 'dispatching'
           AND julianday(dispatch_at) < julianday('now', '-10 minutes')",
    )
    .bind(reason_note(
        reason::DISPATCH_TIMEOUT,
        "claimed but never acknowledged within 10 minutes",
    ))
    .execute(pool)
    .await
    .map_err(|error| format!("recover interrupted notification delivery: {error}"))?
    .rows_affected();
    if timed_out > 0 {
        tracing::debug!(
            target: "notify",
            reason = reason::DISPATCH_TIMEOUT,
            recovered = timed_out,
            "mail notification claims returned to pending"
        );
    }

    for rule in DISMISSAL_RULES {
        dismiss_with_reason(pool, &now, message_id, rule).await?;
    }

    let dispatch_token = uuid::Uuid::new_v4().to_string();
    let claimed_ids = sqlx::query_scalar::<_, String>(
        "UPDATE notification_outbox
         SET state = 'dispatching', dispatch_token = ?, dispatch_at = ?
         WHERE message_id IN (
           SELECT message_id
           FROM notification_outbox
           WHERE state = 'pending'
             AND (? IS NULL OR message_id = ?)
           ORDER BY created_at, message_id
           LIMIT 500
         )
         RETURNING message_id",
    )
    .bind(&dispatch_token)
    .bind(&now)
    .bind(message_id)
    .bind(message_id)
    .fetch_all(pool)
    .await
    .map_err(|error| format!("claim pending mail notifications: {error}"))?;
    if claimed_ids.is_empty() {
        return Ok(Vec::new());
    }

    let rows = sqlx::query_as::<
        _,
        (
            String,
            String,
            String,
            String,
            Option<String>,
            String,
            Option<String>,
            Option<String>,
        ),
    >(
        "SELECT o.message_id, m.thread_id, m.account_id, a.email,
                m.from_name, m.from_address, m.subject, m.snippet
         FROM notification_outbox o
         JOIN messages m ON m.id = o.message_id
         JOIN accounts a ON a.id = m.account_id
         WHERE o.state = 'dispatching' AND o.dispatch_token = ?
         ORDER BY m.date, o.message_id
         LIMIT 500",
    )
    .bind(&dispatch_token)
    .fetch_all(pool)
    .await
    .map_err(|error| format!("load pending mail notifications: {error}"))?;

    Ok(rows
        .into_iter()
        .map(
            |(
                event_id,
                thread_id,
                account_id,
                account_label,
                from_name,
                from_address,
                subject,
                preview,
            )| PendingMailNotification {
                event_id,
                thread_id,
                account_id,
                account_label,
                from_name,
                from_address,
                subject: subject.unwrap_or_default(),
                preview: preview.unwrap_or_default(),
            },
        )
        .collect())
}

pub async fn publish_pending_mail_notifications(
    pool: &SqlitePool,
    events: &EventBus,
    message_id: Option<&str>,
) -> Result<usize, String> {
    let pending = pending_mail_notifications(pool, message_id).await?;
    let count = pending.len();
    for notification in pending {
        events.publish(AppEvent::NewMail {
            event_id: notification.event_id,
            account_id: notification.account_id,
            from_name: notification.from_name,
            from_address: notification.from_address,
            subject: notification.subject,
            preview: notification.preview,
            thread_id: notification.thread_id,
        });
    }
    Ok(count)
}

pub async fn mark_mail_notification_delivered(
    pool: &SqlitePool,
    event_ids: &[String],
) -> Result<(), String> {
    mark_mail_notification_delivered_with_note(pool, event_ids, None).await
}

/// Acknowledge delivery. `note` records that the banner went out on a degraded
/// path — `not_bundled_fallback` means it was shown but clicking it does
/// nothing, which looks like a broken notification from the outside.
pub async fn mark_mail_notification_delivered_with_note(
    pool: &SqlitePool,
    event_ids: &[String],
    note: Option<&str>,
) -> Result<(), String> {
    if event_ids.is_empty() {
        return Ok(());
    }
    let now = chrono::Utc::now().to_rfc3339();
    for event_id in event_ids {
        sqlx::query(
            "UPDATE notification_outbox
             SET state = 'delivered', dispatch_token = NULL, dispatch_at = NULL,
                 delivered_at = ?, last_error = ?
             WHERE message_id = ? AND state IN ('pending', 'dispatching')",
        )
        .bind(&now)
        .bind(note)
        .bind(event_id)
        .execute(pool)
        .await
        .map_err(|error| format!("acknowledge mail notification: {error}"))?;
    }
    Ok(())
}

/// Record that delivery of a claimed row was refused outright. Nothing was
/// shown, so the row ends `dismissed` rather than `delivered`.
pub async fn mark_mail_notification_suppressed(
    pool: &SqlitePool,
    event_id: &str,
    note: &str,
) -> Result<(), String> {
    tracing::debug!(target: "notify", %event_id, %note, "mail notification suppressed");
    sqlx::query(
        "UPDATE notification_outbox
         SET state = 'dismissed', dispatch_token = NULL, dispatch_at = NULL,
             delivered_at = ?, last_error = ?
         WHERE message_id = ? AND state IN ('pending', 'dispatching')",
    )
    .bind(chrono::Utc::now().to_rfc3339())
    .bind(note)
    .bind(event_id)
    .execute(pool)
    .await
    .map_err(|error| format!("suppress mail notification: {error}"))?;
    Ok(())
}

/// Record a failed delivery attempt and return the row to `pending` so the next
/// sync retries it. The reason survives the retry: the claim no longer clears
/// `last_error`.
pub async fn mark_mail_notification_failed(
    pool: &SqlitePool,
    event_id: &str,
    note: &str,
) -> Result<(), String> {
    tracing::debug!(target: "notify", %event_id, %note, "mail notification could not be shown");
    sqlx::query(
        "UPDATE notification_outbox
         SET state = 'pending', dispatch_token = NULL, dispatch_at = NULL,
             last_error = ?
         WHERE message_id = ? AND state IN ('pending', 'dispatching')",
    )
    .bind(note)
    .bind(event_id)
    .execute(pool)
    .await
    .map_err(|error| format!("record mail notification failure: {error}"))?;
    Ok(())
}

/// Window label for a composer popout. Unique per window; which draft a popout
/// holds is tracked in `draft_editors`, not in the label.
///
/// The `compose-` prefix is the one `capabilities/compose.json` matches on. A
/// label outside it leaves the window without any permission, and the failure
/// is silent: `onCloseRequested` never registers and the draft dies with the
/// window.
#[cfg(desktop)]
pub(crate) fn compose_window_label() -> String {
    format!("compose-{}", uuid::Uuid::new_v4().simple())
}

#[cfg(desktop)]
pub(crate) fn build_compose_window(
    app: &AppHandle,
    caller: Option<&str>,
    draft_id: Option<String>,
) -> CommandResult<()> {
    use crate::commands::draft_editors::{editors, focus_window, window_is_live};
    use tauri::{WebviewUrl, WebviewWindowBuilder};
    let caller = caller.unwrap_or_default();
    let label = compose_window_label();
    let mut caller_held = false;
    if let Some(id) = draft_id.as_deref() {
        // One window per draft: a popout for an open draft brings that one up.
        // The claim moves before the build, so the new window's own claim finds it.
        let mut editors = editors()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(owner) = editors.owner_besides(id, caller, window_is_live(app)) {
            drop(editors);
            focus_window(app, &owner);
            return Ok(());
        }
        caller_held = editors.holds(caller, id);
        editors.hand_over(caller, &label, id);
    }
    let path = match draft_id.as_deref() {
        Some(id) => format!("/compose?standalone=1&draftId={id}"),
        None => "/compose?standalone=1".to_string(),
    };
    let url = WebviewUrl::App(path.into());
    let strings = crate::commands::ui_strings::current();
    let built = crate::utils::navigation::guard(WebviewWindowBuilder::new(app, &label, url), app)
        .title(&strings.compose_window_title)
        .inner_size(900.0, 700.0)
        .resizable(true)
        .build();
    if let Err(err) = built {
        if let Some(id) = draft_id.as_deref() {
            let mut editors = editors()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            editors.release(&label, id);
            if caller_held {
                editors.hand_over(&label, caller, id);
            }
        }
        return Err(err.to_string().into());
    }
    Ok(())
}

/// Opens a composer in a window of its own, or focuses the window that already
/// has the draft.
///
/// Desktop only: a phone or tablet has one window, and the frontend routes to
/// `/compose` in place there. Refusing beats an `Ok(())` that opens nothing.
#[tauri::command]
pub async fn open_compose_window(
    app: AppHandle,
    window: tauri::WebviewWindow,
    draft_id: Option<String>,
) -> CommandResult<()> {
    #[cfg(desktop)]
    return build_compose_window(&app, Some(window.label()), draft_id);
    #[cfg(mobile)]
    {
        let _ = (app, window, draft_id);
        Err(crate::commands::error::CommandError::new(
            "backend.errors.composeWindowUnsupported",
        ))
    }
}

#[tauri::command]
pub async fn get_notification_preferences(
    pool: tauri::State<'_, crate::db::ReadPool>,
) -> CommandResult<NotificationPreferences> {
    Ok(load_preferences(pool.inner().as_ref()).await)
}

#[tauri::command]
pub async fn set_notification_preferences(
    pool: tauri::State<'_, SqlitePool>,
    preferences: NotificationPreferences,
) -> CommandResult<NotificationPreferences> {
    let now = chrono::Utc::now().to_rfc3339();
    set_setting(
        pool.inner(),
        "notifications.enabled",
        if preferences.enabled { "true" } else { "false" },
        &now,
    )
    .await?;
    satchel_core::db::body_window::set_days(
        pool.inner(),
        preferences.body_window_days,
        default_body_window_days(),
    )
    .await
    .map_err(|err| err.to_string())?;
    set_setting(
        pool.inner(),
        "notifications.scope",
        &preferences.scope,
        &now,
    )
    .await?;
    set_setting(
        pool.inner(),
        "notifications.sound",
        if preferences.sound { "true" } else { "false" },
        &now,
    )
    .await?;
    let undo = preferences.undo_send_seconds.clamp(0, 60);
    set_setting(
        pool.inner(),
        "compose.undo_send_seconds",
        &undo.to_string(),
        &now,
    )
    .await?;
    let trash_days = preferences.trash_auto_delete_days.clamp(0, 3650);
    set_setting(
        pool.inner(),
        "maintenance.trash_auto_delete_days",
        &trash_days.to_string(),
        &now,
    )
    .await?;
    set_setting(
        pool.inner(),
        "compose.send_read_receipts",
        if preferences.send_read_receipts {
            "true"
        } else {
            "false"
        },
        &now,
    )
    .await?;
    Ok(preferences)
}

pub async fn load_preferences(pool: &SqlitePool) -> NotificationPreferences {
    let enabled = read_bool(pool, "notifications.enabled")
        .await
        .unwrap_or(true);
    let scope = read_string(pool, "notifications.scope")
        .await
        .unwrap_or_else(|| "inbox".into());
    let sound = read_bool(pool, "notifications.sound").await.unwrap_or(true);
    let undo_send_seconds = read_string(pool, "compose.undo_send_seconds")
        .await
        .and_then(|raw| raw.parse::<i64>().ok())
        .map(|n| n.clamp(0, 60))
        .unwrap_or(5);
    let trash_auto_delete_days = read_string(pool, "maintenance.trash_auto_delete_days")
        .await
        .and_then(|raw| raw.parse::<i64>().ok())
        .map(|n| n.clamp(0, 3650))
        .unwrap_or(30);
    let send_read_receipts = read_bool(pool, "compose.send_read_receipts")
        .await
        .unwrap_or(false);
    let body_window_days = read_string(pool, "sync.body_window_days")
        .await
        .and_then(|raw| raw.parse::<i64>().ok())
        .map(|n| n.clamp(0, 3650))
        .unwrap_or_else(default_body_window_days);
    NotificationPreferences {
        body_window_days,
        enabled,
        scope,
        sound,
        undo_send_seconds,
        trash_auto_delete_days,
        send_read_receipts,
    }
}

async fn read_string(pool: &SqlitePool, key: &str) -> Option<String> {
    sqlx::query_scalar::<_, String>("SELECT value FROM app_settings WHERE key = ?")
        .bind(key)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
}

async fn read_bool(pool: &SqlitePool, key: &str) -> Option<bool> {
    read_string(pool, key).await.map(|raw| {
        matches!(
            raw.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes"
        )
    })
}

async fn set_setting(pool: &SqlitePool, key: &str, value: &str, now: &str) -> Result<(), String> {
    sqlx::query(
        "INSERT INTO app_settings (key, value, updated_at) VALUES (?, ?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
    )
    .bind(key)
    .bind(value)
    .bind(now)
    .execute(pool)
    .await
    .map_err(|err| err.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use satchel_core::db::blocked_senders::BlockedSenderRepository;
    use sqlx::sqlite::SqlitePoolOptions;

    // These tests model an app that stayed open throughout the sync gap.
    // Reopening tests below pass a fresh session boundary explicitly.
    async fn pending_mail_notifications(
        pool: &SqlitePool,
        message_id: Option<&str>,
    ) -> Result<Vec<PendingMailNotification>, String> {
        pending_mail_notifications_since(pool, message_id, Some("2000-01-01T00:00:00Z")).await
    }

    async fn notification_pool(last_sync_at: Option<&str>) -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("pool");
        sqlx::migrate!("../crates/satchel-core/src/db/migrations")
            .run(&pool)
            .await
            .expect("migrations");
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO accounts
             (id, display_name, email, provider, config, credentials_ref,
              last_sync_at, created_at, updated_at)
             VALUES ('account', 'Me', 'me@example.com', 'gmail', '{}', '', ?, ?, ?)",
        )
        .bind(last_sync_at)
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("account");
        sqlx::query(
            "INSERT INTO folders
             (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at)
             VALUES ('inbox', 'account', 'Inbox', 'INBOX', 'inbox', 'INBOX', ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("folder");
        sqlx::query(
            "INSERT INTO threads
             (id, account_id, subject, last_message_date, message_count, unread_count,
              workflow_state, created_at, updated_at)
             VALUES ('thread', 'account', 'Subject', ?, 0, 0, 'inbox', ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("thread");
        pool
    }

    async fn insert_message(pool: &SqlitePool, id: &str, date: &str) {
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO messages
             (id, account_id, folder_id, thread_id, from_address, from_name,
              to_addresses, subject, date, snippet, is_read, is_sent, is_draft,
              created_at, updated_at)
             VALUES (?, 'account', 'inbox', 'thread', 'sender@example.com', 'Sender',
                     '[]', 'Subject', ?, 'Preview', 0, 0, 0, ?, ?)",
        )
        .bind(id)
        .bind(date)
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await
        .expect("message");
    }

    #[tokio::test]
    async fn headless_delivery_keeps_mail_received_before_the_worker_started() {
        let pool = notification_pool(None).await;
        let date = (chrono::Utc::now() - chrono::TimeDelta::minutes(1)).to_rfc3339();
        insert_message(&pool, "headless", &date).await;
        let pending = super::pending_headless_mail_notifications(&pool)
            .await
            .unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].event_id, "headless");
    }

    #[tokio::test]
    async fn an_arrival_in_the_session_start_second_is_deliverable() {
        let pool = notification_pool(Some("2026-10-12T09:59:59Z")).await;
        insert_message(&pool, "same-second", "2026-10-12T10:00:00Z").await;
        let start = chrono::DateTime::parse_from_rfc3339("2026-10-12T10:00:00.500Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let cutoff = super::notification_cutoff_at(start);
        let pending = pending_mail_notifications_since(&pool, None, Some(&cutoff))
            .await
            .unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].event_id, "same-second");
    }

    /// Same as `insert_message`, with the columns the filters look at exposed.
    async fn insert_message_with(
        pool: &SqlitePool,
        id: &str,
        date: &str,
        folder_id: &str,
        from_address: &str,
        flags: (i64, i64, i64),
    ) {
        let now = chrono::Utc::now().to_rfc3339();
        let (is_read, is_sent, is_draft) = flags;
        sqlx::query(
            "INSERT INTO messages
             (id, account_id, folder_id, thread_id, from_address, from_name,
              to_addresses, subject, date, snippet, is_read, is_sent, is_draft,
              created_at, updated_at)
             VALUES (?, 'account', ?, 'thread', ?, 'Sender',
                     '[]', 'Subject', ?, 'Preview', ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(folder_id)
        .bind(from_address)
        .bind(date)
        .bind(is_read)
        .bind(is_sent)
        .bind(is_draft)
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await
        .expect("message");
    }

    async fn outbox(pool: &SqlitePool, id: &str) -> Option<(String, Option<String>)> {
        sqlx::query_as::<_, (String, Option<String>)>(
            "SELECT state, last_error FROM notification_outbox WHERE message_id = ?",
        )
        .bind(id)
        .fetch_optional(pool)
        .await
        .expect("outbox row")
    }

    async fn reason_of(pool: &SqlitePool, id: &str) -> String {
        let (state, note) = outbox(pool, id).await.expect("row exists");
        assert_eq!(state, "dismissed", "{id}: state");
        let note = note.expect("reason note");
        let (code, detail) = note.split_once(':').expect("`code: detail` format");
        assert!(
            !detail.trim().is_empty(),
            "{id}: detail missing in {note:?}"
        );
        code.to_string()
    }

    async fn exec(pool: &SqlitePool, sql: &str) {
        sqlx::query(sql).execute(pool).await.expect(sql);
    }

    async fn setting(pool: &SqlitePool, key: &str, value: &str) {
        set_setting(pool, key, value, "").await.expect(key);
    }

    fn fresh() -> String {
        (chrono::Utc::now() - chrono::Duration::minutes(1)).to_rfc3339()
    }

    /// The Android plugin answers for a dropped notification as well. Reading
    /// that answer as a delivery is what let the Diagnose page claim "zugestellt"
    /// for mail the user never saw.
    #[test]
    fn android_answer_separates_a_posted_mail_from_a_dropped_one() {
        assert_eq!(
            android_delivery_outcome(&serde_json::json!({ "shown": true })),
            MailDeliveryOutcome::Native
        );

        let refused = android_delivery_outcome(
            &serde_json::json!({ "shown": false, "reason": "permission_denied" }),
        );
        let MailDeliveryOutcome::Suppressed(note) = refused else {
            panic!("a refused permission is not a delivery: {refused:?}");
        };
        // Same code macOS records for the same refusal — the Diagnose page reads
        // one label, not one per platform.
        assert!(note.starts_with(reason::PERMISSION_DENIED), "{note}");

        // An unexplained drop is still not a delivery, but it is not a refused
        // permission either.
        let unexplained = android_delivery_outcome(&serde_json::json!({ "shown": false }));
        let MailDeliveryOutcome::Suppressed(note) = unexplained else {
            panic!("a dropped notification is not a delivery: {unexplained:?}");
        };
        assert!(note.starts_with(reason::DISPATCH_ERROR), "{note}");
    }

    /// A plugin build from before the answer existed reports nothing. Reading
    /// that as a suppression would mark every delivered mail as missing.
    #[test]
    fn android_answer_without_the_field_still_counts_as_delivered() {
        assert_eq!(
            android_delivery_outcome(&serde_json::json!({})),
            MailDeliveryOutcome::Native
        );
    }

    /// The headless worker used to acknowledge every event id it was handed.
    /// Its answers now settle the rows through the same routing the foreground
    /// bridge uses, so a refusal reads `permission_denied` on both paths.
    #[tokio::test]
    async fn android_worker_acknowledgements_settle_each_row_by_what_happened() {
        let pool = notification_pool(Some(&fresh())).await;
        for id in ["posted", "refused", "unexplained"] {
            insert_message(&pool, id, &fresh()).await;
        }
        assert_eq!(
            pending_mail_notifications(&pool, None).await.unwrap().len(),
            3
        );

        record_android_worker_outcomes(
            &pool,
            &[
                serde_json::json!({ "eventId": "posted", "shown": true }),
                serde_json::json!({
                    "eventId": "refused",
                    "shown": false,
                    "reason": reason::PERMISSION_DENIED,
                }),
                serde_json::json!({ "eventId": "unexplained", "shown": false }),
            ],
        )
        .await
        .unwrap();

        assert_eq!(
            outbox(&pool, "posted").await,
            Some(("delivered".into(), None))
        );
        let (state, note) = outbox(&pool, "refused").await.unwrap();
        assert_eq!(state, "dismissed", "a refused permission is not delivered");
        assert!(note.unwrap().starts_with(reason::PERMISSION_DENIED));
        // An unexplained drop may be transient, so it goes back in the queue.
        let (state, note) = outbox(&pool, "unexplained").await.unwrap();
        assert_eq!(state, "pending");
        assert!(note.unwrap().starts_with(reason::DISPATCH_ERROR));
    }

    #[tokio::test]
    async fn android_worker_acknowledgement_without_an_event_id_is_refused() {
        let pool = notification_pool(Some(&fresh())).await;
        assert!(
            record_android_worker_outcomes(&pool, &[serde_json::json!({ "shown": true })])
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn trigger_records_a_reason_for_every_filtered_message() {
        let now = chrono::Utc::now();
        let last_sync = (now - chrono::Duration::hours(2)).to_rfc3339();
        let pool = notification_pool(Some(&last_sync)).await;
        exec(
            &pool,
            "INSERT INTO folders
             (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at)
             VALUES ('archive', 'account', 'Archiv', 'Archiv', 'archive', 'Archiv', '', '')",
        )
        .await;

        insert_message_with(&pool, "sent", &fresh(), "inbox", "x@example.com", (0, 1, 0)).await;
        assert_eq!(reason_of(&pool, "sent").await, reason::FILTER_OWN_SENT);

        insert_message_with(
            &pool,
            "draft",
            &fresh(),
            "inbox",
            "x@example.com",
            (0, 0, 1),
        )
        .await;
        assert_eq!(reason_of(&pool, "draft").await, reason::FILTER_OWN_SENT);

        insert_message_with(
            &pool,
            "self",
            &fresh(),
            "inbox",
            "Me@Example.com",
            (0, 0, 0),
        )
        .await;
        assert_eq!(reason_of(&pool, "self").await, reason::FILTER_OWN_SENT);

        insert_message_with(
            &pool,
            "archived",
            &fresh(),
            "archive",
            "x@example.com",
            (0, 0, 0),
        )
        .await;
        assert_eq!(reason_of(&pool, "archived").await, reason::FILTER_NOT_INBOX);
        assert_eq!(
            outbox(&pool, "archived").await.unwrap().1.as_deref(),
            Some("filter_not_inbox: archive")
        );

        insert_message_with(
            &pool,
            "unparseable",
            "gestern",
            "inbox",
            "x@example.com",
            (0, 0, 0),
        )
        .await;
        assert_eq!(
            reason_of(&pool, "unparseable").await,
            reason::FILTER_NO_DATE
        );

        let slightly_old = (now - chrono::Duration::hours(3)).to_rfc3339();
        insert_message_with(
            &pool,
            "late",
            &slightly_old,
            "inbox",
            "x@example.com",
            (0, 0, 0),
        )
        .await;
        assert_eq!(reason_of(&pool, "late").await, reason::FILTER_TOO_OLD);

        let backfill = (now - chrono::Duration::days(3)).to_rfc3339();
        insert_message_with(
            &pool,
            "backfill",
            &backfill,
            "inbox",
            "x@example.com",
            (0, 0, 0),
        )
        .await;
        assert!(
            outbox(&pool, "backfill").await.is_none(),
            "backfill writes no row"
        );

        setting(&pool, "notifications.enabled", "false").await;
        insert_message_with(
            &pool,
            "disabled",
            &fresh(),
            "inbox",
            "x@example.com",
            (0, 0, 0),
        )
        .await;
        assert_eq!(reason_of(&pool, "disabled").await, reason::SETTING_DISABLED);
        exec(
            &pool,
            "UPDATE app_settings SET value = 'true' WHERE key = 'notifications.enabled'",
        )
        .await;

        setting(&pool, "notifications.scope", "off").await;
        insert_message_with(
            &pool,
            "scope-off",
            &fresh(),
            "inbox",
            "x@example.com",
            (0, 0, 0),
        )
        .await;
        assert_eq!(
            reason_of(&pool, "scope-off").await,
            reason::SETTING_DISABLED
        );
        exec(
            &pool,
            "DELETE FROM app_settings WHERE key = 'notifications.scope'",
        )
        .await;

        exec(
            &pool,
            "UPDATE accounts SET is_active = 0 WHERE id = 'account'",
        )
        .await;
        insert_message_with(
            &pool,
            "inactive",
            &fresh(),
            "inbox",
            "x@example.com",
            (0, 0, 0),
        )
        .await;
        assert_eq!(
            reason_of(&pool, "inactive").await,
            reason::FILTER_ACCOUNT_INACTIVE
        );
        exec(
            &pool,
            "UPDATE accounts SET is_active = 1 WHERE id = 'account'",
        )
        .await;

        insert_message_with(&pool, "ok", &fresh(), "inbox", "x@example.com", (0, 0, 0)).await;
        assert_eq!(outbox(&pool, "ok").await, Some(("pending".into(), None)));
    }

    /// One pending row per dispatcher rule; the mailbox changes after ingestion.
    #[tokio::test]
    async fn dispatcher_records_a_reason_for_every_dismissed_row() {
        let pool = notification_pool(Some(&fresh())).await;
        exec(
            &pool,
            "INSERT INTO folders
             (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at)
             VALUES ('archive', 'account', 'Archiv', 'Archiv', 'archive', 'Archiv', '', '')",
        )
        .await;
        exec(
            &pool,
            "INSERT INTO threads
             (id, account_id, subject, last_message_date, message_count, unread_count,
              workflow_state, is_muted, created_at, updated_at)
             VALUES ('muted', 'account', 'S', '', 0, 0, 'inbox', 1, '', '')",
        )
        .await;
        exec(
            &pool,
            "INSERT INTO blocked_senders (id, account_id, pattern, pattern_type, created_at)
             VALUES ('b1', NULL, 'spam.example.com', 'domain', '')",
        )
        .await;

        for id in ["read", "moved", "sent", "muted", "blocked", "gone", "kept"] {
            insert_message(&pool, id, &fresh()).await;
            assert_eq!(outbox(&pool, id).await.unwrap().0, "pending", "{id}");
        }
        exec(&pool, "UPDATE messages SET is_read = 1 WHERE id = 'read'").await;
        exec(
            &pool,
            "UPDATE messages SET folder_id = 'archive' WHERE id = 'moved'",
        )
        .await;
        exec(&pool, "UPDATE messages SET is_sent = 1 WHERE id = 'sent'").await;
        exec(
            &pool,
            "UPDATE messages SET thread_id = 'muted' WHERE id = 'muted'",
        )
        .await;
        exec(
            &pool,
            "UPDATE messages SET from_address = 'x@spam.example.com' WHERE id = 'blocked'",
        )
        .await;
        // FK cascade removes the outbox row with the message; the rule exists
        // for databases opened without `foreign_keys`, so simulate that.
        exec(&pool, "PRAGMA foreign_keys = OFF").await;
        exec(&pool, "DELETE FROM messages WHERE id = 'gone'").await;
        exec(&pool, "PRAGMA foreign_keys = ON").await;

        let claimed = pending_mail_notifications(&pool, None)
            .await
            .expect("claim");
        assert_eq!(
            claimed
                .iter()
                .map(|n| n.event_id.as_str())
                .collect::<Vec<_>>(),
            vec!["kept"]
        );

        assert_eq!(reason_of(&pool, "read").await, reason::FILTER_ALREADY_READ);
        assert_eq!(reason_of(&pool, "moved").await, reason::FILTER_NOT_INBOX);
        assert_eq!(reason_of(&pool, "sent").await, reason::FILTER_OWN_SENT);
        assert_eq!(reason_of(&pool, "muted").await, reason::FILTER_MUTED_THREAD);
        assert_eq!(
            reason_of(&pool, "blocked").await,
            reason::FILTER_MUTED_SENDER
        );
        assert_eq!(reason_of(&pool, "gone").await, reason::FILTER_MESSAGE_GONE);
    }

    #[tokio::test]
    async fn muted_sender_sql_matches_repository_scope_and_exactness() {
        let pool = notification_pool(Some(&fresh())).await;
        let cases = [
            (
                "global-address",
                "global@example.com",
                "address",
                None,
                "global@example.com",
            ),
            (
                "global-domain",
                "*@spam.example.com",
                "domain",
                None,
                "alice@spam.example.com",
            ),
            (
                "account-address",
                "accounted@example.com",
                "address",
                Some("account"),
                "accounted@example.com",
            ),
            (
                "account-domain",
                "local.example.com",
                "domain",
                Some("account"),
                "alice@local.example.com",
            ),
            (
                "domain-suffix",
                "suffix.example.com",
                "domain",
                None,
                "alice@notsuffix.example.com",
            ),
            (
                "internal-marker",
                "sp*m.example.com",
                "domain",
                None,
                "alice@sp*m.example.com",
            ),
            (
                "internal-marker-mismatch",
                "we*ird.example",
                "domain",
                None,
                "alice@weird.example",
            ),
            ("missing-at", "example.net", "domain", None, "example.net"),
        ];
        for (id, pattern, pattern_type, account_id, from_address) in cases {
            let account_sql = if account_id.is_some() { "?" } else { "NULL" };
            let sql = format!(
                "INSERT INTO blocked_senders (id, account_id, pattern, pattern_type, created_at)
                 VALUES (?, {account_sql}, ?, ?, '')"
            );
            let mut query = sqlx::query(&sql).bind(id).bind(pattern).bind(pattern_type);
            if let Some(account_id) = account_id {
                query = sqlx::query(&sql)
                    .bind(id)
                    .bind(account_id)
                    .bind(pattern)
                    .bind(pattern_type);
            }
            query.execute(&pool).await.expect("blocked sender rule");
            insert_message_with(&pool, id, &fresh(), "inbox", from_address, (0, 0, 0)).await;
        }

        let repository = BlockedSenderRepository::new(&pool);
        let claimed = pending_mail_notifications(&pool, None)
            .await
            .expect("claim notifications");
        let claimed_ids = claimed
            .iter()
            .map(|notification| notification.event_id.as_str())
            .collect::<Vec<_>>();

        for (id, _pattern, _pattern_type, _account_id, from_address) in cases {
            let expected = repository
                .is_blocked("account", from_address)
                .await
                .expect("repository match");
            assert_eq!(
                !claimed_ids.contains(&id),
                expected,
                "notification SQL disagrees for {id}"
            );
        }
    }

    #[tokio::test]
    async fn dispatcher_dismisses_with_a_reason_when_the_account_is_deactivated() {
        let pool = notification_pool(Some(&fresh())).await;
        insert_message(&pool, "m", &fresh()).await;
        exec(&pool, "UPDATE accounts SET is_active = 0").await;
        assert!(pending_mail_notifications(&pool, None)
            .await
            .unwrap()
            .is_empty());
        assert_eq!(reason_of(&pool, "m").await, reason::FILTER_ACCOUNT_INACTIVE);
    }

    #[tokio::test]
    async fn dispatcher_dismisses_with_a_reason_when_notifications_are_switched_off() {
        let pool = notification_pool(Some(&fresh())).await;
        insert_message(&pool, "m", &fresh()).await;
        setting(&pool, "notifications.scope", "off").await;
        assert!(pending_mail_notifications(&pool, None)
            .await
            .unwrap()
            .is_empty());
        assert_eq!(reason_of(&pool, "m").await, reason::SETTING_DISABLED);
        assert_eq!(
            outbox(&pool, "m").await.unwrap().1.as_deref(),
            Some("setting_disabled: notifications.scope=off")
        );
    }

    #[tokio::test]
    async fn stale_claims_return_to_pending_and_keep_the_timeout_reason() {
        let pool = notification_pool(Some(&fresh())).await;
        insert_message(&pool, "m", &fresh()).await;
        let claimed = pending_mail_notifications(&pool, None).await.unwrap();
        assert_eq!(claimed.len(), 1);
        exec(
            &pool,
            "UPDATE notification_outbox SET dispatch_at = datetime('now', '-11 minutes')",
        )
        .await;

        let again = pending_mail_notifications(&pool, None).await.unwrap();
        assert_eq!(again.len(), 1, "re-claimed after the timeout");
        let (state, note) = outbox(&pool, "m").await.unwrap();
        assert_eq!(state, "dispatching");
        assert!(note.unwrap().starts_with(reason::DISPATCH_TIMEOUT));
    }

    #[test]
    fn a_burst_is_coalesced_past_the_limit_and_counting_restarts_after_the_window() {
        let start = std::time::Instant::now();
        let second = std::time::Duration::from_secs(1);
        let mut coalescer = BurstCoalescer::default();

        for index in 0..BURST_LIMIT {
            assert_eq!(
                coalescer.decide(start + second * index as u32),
                BurstDecision::Deliver,
                "mail {index} is within the limit"
            );
        }
        assert_eq!(
            coalescer.decide(start + second * 5),
            BurstDecision::Coalesce { count: 1 }
        );
        assert_eq!(
            coalescer.decide(start + second * 6),
            BurstDecision::Coalesce { count: 2 }
        );

        // Quiet for a whole window: the next mail starts a new burst.
        let later = start + second * 6 + BURST_WINDOW;
        assert_eq!(coalescer.decide(later), BurstDecision::Deliver);
    }

    #[tokio::test]
    async fn a_coalesced_mail_is_dismissed_with_its_own_reason() {
        let pool = notification_pool(Some(&fresh())).await;
        insert_message(&pool, "burst", &fresh()).await;
        assert_eq!(
            pending_mail_notifications(&pool, None).await.unwrap().len(),
            1
        );

        let note = reason_note(reason::FILTER_COALESCED, "burst; shown in a summary of 1");
        mark_mail_notification_suppressed(&pool, "burst", &note)
            .await
            .unwrap();

        assert_eq!(reason_of(&pool, "burst").await, reason::FILTER_COALESCED);
        assert_eq!(outbox(&pool, "burst").await.unwrap().0, "dismissed");
    }

    #[tokio::test]
    async fn delivery_outcomes_land_on_the_row() {
        let pool = notification_pool(Some(&fresh())).await;
        for id in ["native", "fallback", "denied", "errored"] {
            insert_message(&pool, id, &fresh()).await;
        }
        assert_eq!(
            pending_mail_notifications(&pool, None).await.unwrap().len(),
            4
        );

        mark_mail_notification_delivered(&pool, &["native".into()])
            .await
            .unwrap();
        assert_eq!(
            outbox(&pool, "native").await,
            Some(("delivered".into(), None))
        );

        let fallback = reason_note(reason::NOT_BUNDLED_FALLBACK, "dev");
        mark_mail_notification_delivered_with_note(&pool, &["fallback".into()], Some(&fallback))
            .await
            .unwrap();
        assert_eq!(
            outbox(&pool, "fallback").await,
            Some(("delivered".into(), Some(fallback)))
        );

        let denied = reason_note(reason::PERMISSION_DENIED, "refused");
        mark_mail_notification_suppressed(&pool, "denied", &denied)
            .await
            .unwrap();
        assert_eq!(reason_of(&pool, "denied").await, reason::PERMISSION_DENIED);

        let errored = reason_note(reason::DISPATCH_ERROR, "boom");
        mark_mail_notification_failed(&pool, "errored", &errored)
            .await
            .unwrap();
        assert_eq!(
            outbox(&pool, "errored").await,
            Some(("pending".into(), Some(errored.clone())))
        );
        // The retry claim keeps the reason until delivery overwrites it.
        let retried = pending_mail_notifications(&pool, None).await.unwrap();
        assert_eq!(retried.len(), 1);
        assert_eq!(outbox(&pool, "errored").await.unwrap().1, Some(errored));
    }

    #[tokio::test]
    async fn reopening_dismisses_catchup_but_delivers_mail_arriving_in_this_session() {
        let now = chrono::Utc::now();
        let pool = notification_pool(Some(&(now - chrono::Duration::days(4)).to_rfc3339())).await;
        insert_message(
            &pool,
            "catchup",
            &(now - chrono::Duration::days(2)).to_rfc3339(),
        )
        .await;
        insert_message(
            &pool,
            "just-before-opening",
            &(now - chrono::Duration::seconds(1)).to_rfc3339(),
        )
        .await;
        insert_message(
            &pool,
            "live",
            &(now + chrono::Duration::seconds(1)).to_rfc3339(),
        )
        .await;

        let claimed = pending_mail_notifications_since(&pool, None, Some(&now.to_rfc3339()))
            .await
            .unwrap();
        assert_eq!(
            claimed
                .iter()
                .map(|row| row.event_id.as_str())
                .collect::<Vec<_>>(),
            vec!["live"]
        );
        assert_eq!(reason_of(&pool, "catchup").await, "filter_before_session");
        assert_eq!(
            reason_of(&pool, "just-before-opening").await,
            "filter_before_session"
        );
    }

    #[tokio::test]
    async fn reopening_dismisses_an_unacknowledged_delivery_from_the_previous_session() {
        let now = chrono::Utc::now();
        let pool = notification_pool(Some(&(now - chrono::Duration::hours(2)).to_rfc3339())).await;
        insert_message(
            &pool,
            "retry",
            &(now - chrono::Duration::minutes(1)).to_rfc3339(),
        )
        .await;
        assert_eq!(
            pending_mail_notifications(&pool, Some("retry"))
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(outbox(&pool, "retry").await.unwrap().0, "dispatching");

        assert!(
            pending_mail_notifications_since(&pool, Some("retry"), Some(&now.to_rfc3339()))
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            reason_of(&pool, "retry").await,
            reason::FILTER_BEFORE_SESSION
        );
    }

    #[tokio::test]
    async fn durable_outbox_keeps_delayed_mail_while_the_app_stays_open() {
        let now = chrono::Utc::now();
        let last_sync = (now - chrono::Duration::days(4)).to_rfc3339();
        let pool = notification_pool(Some(&last_sync)).await;

        insert_message(
            &pool,
            "closed",
            &(now - chrono::Duration::days(3)).to_rfc3339(),
        )
        .await;
        insert_message(
            &pool,
            "comment",
            &(now - chrono::Duration::hours(1)).to_rfc3339(),
        )
        .await;
        insert_message(
            &pool,
            "old-backfill",
            &(now - chrono::Duration::days(5)).to_rfc3339(),
        )
        .await;

        let pending: Vec<String> = sqlx::query_scalar(
            "SELECT message_id FROM notification_outbox WHERE state = 'pending' ORDER BY message_id",
        )
        .fetch_all(&pool)
        .await
        .expect("pending");
        assert_eq!(pending, vec!["closed", "comment"]);

        let claimed = pending_mail_notifications(&pool, None)
            .await
            .expect("claim");
        assert_eq!(claimed.len(), 2);
        assert!(pending_mail_notifications(&pool, None)
            .await
            .expect("second claim")
            .is_empty());
        mark_mail_notification_delivered(
            &pool,
            &claimed
                .iter()
                .map(|notification| notification.event_id.clone())
                .collect::<Vec<_>>(),
        )
        .await
        .expect("acknowledge");
        let delivered: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM notification_outbox WHERE state = 'delivered'",
        )
        .fetch_one(&pool)
        .await
        .expect("delivered");
        assert_eq!(delivered, 2);
    }

    #[tokio::test]
    async fn initial_backfill_stays_quiet_and_read_pending_mail_is_dismissed() {
        let now = chrono::Utc::now();
        let pool = notification_pool(None).await;
        // Without a watermark the first sync notifies only the last 15 minutes.
        // Two hours back is close enough to keep a (dismissed) row; days back
        // is backfill and writes none.
        insert_message(
            &pool,
            "recent-backfill",
            &(now - chrono::Duration::hours(2)).to_rfc3339(),
        )
        .await;
        assert_eq!(
            reason_of(&pool, "recent-backfill").await,
            reason::FILTER_TOO_OLD
        );
        insert_message(
            &pool,
            "backfill",
            &(now - chrono::Duration::days(30)).to_rfc3339(),
        )
        .await;
        assert!(outbox(&pool, "backfill").await.is_none());
        let pending: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM notification_outbox WHERE state = 'pending'")
                .fetch_one(&pool)
                .await
                .expect("pending count");
        assert_eq!(pending, 0);

        insert_message(
            &pool,
            "fresh",
            &(now - chrono::Duration::minutes(1)).to_rfc3339(),
        )
        .await;
        sqlx::query("UPDATE messages SET is_read = 1 WHERE id = 'fresh'")
            .execute(&pool)
            .await
            .expect("read");
        assert!(pending_mail_notifications(&pool, None)
            .await
            .expect("pending")
            .is_empty());
        let state: String =
            sqlx::query_scalar("SELECT state FROM notification_outbox WHERE message_id = 'fresh'")
                .fetch_one(&pool)
                .await
                .expect("state");
        assert_eq!(state, "dismissed");
    }
}
