use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

use crate::commands::error::CommandResult;

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/diagnostics/", optional_fields = nullable))]
pub struct FrontendDiagnosticInput {
    pub event: String,
    pub detail: String,
    pub cache_threads: u64,
    pub cached_messages: u64,
    pub body_chars: u64,
    pub dom_nodes: u64,
    pub iframe_count: u64,
    pub rendered_mail_count: u64,
    pub js_heap_bytes: u64,
    /// `error` and `warn` land at that level, so the copied report picks them up.
    #[serde(default)]
    pub level: Option<String>,
}

macro_rules! frontend_event {
    ($level:expr, $input:expr) => {
        tracing::event!(
            target: "satchel::frontend",
            $level,
            event = %$input.event,
            detail = ?$input.detail,
            cache_threads = $input.cache_threads,
            cached_messages = $input.cached_messages,
            body_chars = $input.body_chars,
            dom_nodes = $input.dom_nodes,
            iframe_count = $input.iframe_count,
            rendered_mail_count = $input.rendered_mail_count,
            js_heap_bytes = $input.js_heap_bytes,
            "frontend diagnostic"
        )
    };
}

#[tauri::command]
pub fn record_frontend_diagnostic(input: FrontendDiagnosticInput) {
    match input.level.as_deref() {
        Some("error") => frontend_event!(tracing::Level::ERROR, input),
        Some("warn") => frontend_event!(tracing::Level::WARN, input),
        _ => frontend_event!(tracing::Level::INFO, input),
    }
}

/// Read-only snapshot for the „Diagnose" settings pane.
///
/// # Two things called "notification" live in the database
///
/// `sync_queue` rows whose id starts with `notification:` are **outbound**:
/// they are the provider mutations a notification *quick action* produced
/// (Erledigt, Gelesen, Löschen). `notification_actions.rs` derives the id from
/// a digest of event/action/thread so a WorkManager retry re-enters the same
/// row instead of archiving twice. They say nothing about whether a
/// notification was ever shown.
///
/// `notification_outbox` (migration 0057, reasons since 0060) is **inbound**:
/// one row per ingested message, written by a SQLite trigger in the same
/// transaction as the message, carrying the delivery state and — when nothing
/// was shown — the reason code in `last_error`. That is the table to read when
/// the question is „warum kam für diese Mail keine Meldung?".
///
/// The prefix below splits the two `sync_queue` views, the same way
/// `sync::queue` does.
const NOTIFICATION_ID_PREFIX: &str = "notification:%";

const GMAIL_BLOCK_UNTIL_KEY: &str = "gmail_rate_limit_block_until";
const GMAIL_BLOCK_STREAK_KEY: &str = "gmail_rate_limit_block_streak";

const RECENT_LIMIT: i64 = 20;

/// Enough rows to cover a morning of mail without turning the pane into a log.
const OUTBOX_RECENT_LIMIT: i64 = 30;

#[derive(Debug, Clone, Serialize, FromRow)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/diagnostics/")
)]
pub struct DiagnosticsAccountDto {
    pub id: String,
    pub email: String,
    pub display_name: String,
    pub provider: String,
    pub sync_status: String,
    pub last_sync_at: Option<String>,
    pub last_sync_error: Option<String>,
    pub last_sync_error_at: Option<String>,
    pub last_oauth_error: Option<String>,
    pub last_oauth_error_at: Option<String>,
}

#[derive(Debug, FromRow)]
struct StatusCountRow {
    status: String,
    entries: i64,
    total_retries: i64,
    max_retries: i64,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/diagnostics/")
)]
pub struct DiagnosticsStatusCountDto {
    pub status: String,
    pub entries: u64,
    pub total_retries: u64,
    pub max_retries: u64,
}

impl From<StatusCountRow> for DiagnosticsStatusCountDto {
    fn from(row: StatusCountRow) -> Self {
        Self {
            status: row.status,
            entries: row.entries.max(0) as u64,
            total_retries: row.total_retries.max(0) as u64,
            max_retries: row.max_retries.max(0) as u64,
        }
    }
}

#[derive(Debug, FromRow)]
struct QueueEntryRow {
    id: String,
    account_id: String,
    action_type: String,
    status: String,
    error: Option<String>,
    attempts: i64,
    next_due: Option<String>,
    updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/diagnostics/")
)]
pub struct DiagnosticsQueueEntryDto {
    pub id: String,
    pub account_id: String,
    pub action_type: String,
    pub status: String,
    pub error: Option<String>,
    pub attempts: u64,
    pub next_due: Option<String>,
    pub updated_at: String,
}

impl From<QueueEntryRow> for DiagnosticsQueueEntryDto {
    fn from(row: QueueEntryRow) -> Self {
        Self {
            id: row.id,
            account_id: row.account_id,
            action_type: row.action_type,
            status: row.status,
            error: row.error,
            attempts: row.attempts.max(0) as u64,
            next_due: row.next_due,
            updated_at: row.updated_at,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/diagnostics/")
)]
pub struct DiagnosticsQueueDto {
    pub by_status: Vec<DiagnosticsStatusCountDto>,
    pub recent_failures: Vec<DiagnosticsQueueEntryDto>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/diagnostics/")
)]
pub struct DiagnosticsNotificationsDto {
    pub by_status: Vec<DiagnosticsStatusCountDto>,
    pub recent: Vec<DiagnosticsQueueEntryDto>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/diagnostics/")
)]
pub struct DiagnosticsRateLimitDto {
    pub provider: String,
    pub blocked_until: Option<String>,
    pub streak: u64,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/diagnostics/")
)]
pub struct DiagnosticsOutboxStateCountDto {
    pub state: String,
    pub entries: u64,
}

/// One reason code and how often it decided against a notification.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/diagnostics/")
)]
pub struct DiagnosticsOutboxReasonCountDto {
    pub reason: String,
    pub entries: u64,
}

#[derive(Debug, FromRow)]
struct OutboxEntryRow {
    message_id: String,
    state: String,
    created_at: String,
    delivered_at: Option<String>,
    last_error: Option<String>,
    subject: Option<String>,
    from_name: Option<String>,
    from_address: Option<String>,
    date: Option<String>,
    account_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/diagnostics/")
)]
pub struct DiagnosticsOutboxEntryDto {
    pub message_id: String,
    pub state: String,
    pub created_at: String,
    pub delivered_at: Option<String>,
    /// Reason code, split off the `<code>: <detail>` note.
    pub reason: Option<String>,
    pub detail: Option<String>,
    pub subject: Option<String>,
    pub from_name: Option<String>,
    pub from_address: Option<String>,
    pub date: Option<String>,
    pub account_id: Option<String>,
}

impl From<OutboxEntryRow> for DiagnosticsOutboxEntryDto {
    fn from(row: OutboxEntryRow) -> Self {
        let (reason, detail) = split_reason(row.last_error.as_deref());
        Self {
            message_id: row.message_id,
            state: row.state,
            created_at: row.created_at,
            delivered_at: row.delivered_at,
            reason,
            detail,
            subject: row.subject,
            from_name: row.from_name,
            from_address: row.from_address,
            date: row.date,
            account_id: row.account_id,
        }
    }
}

/// `"filter_not_inbox: Archiv"` → `("filter_not_inbox", "Archiv")`.
fn split_reason(note: Option<&str>) -> (Option<String>, Option<String>) {
    let Some(note) = note.map(str::trim).filter(|note| !note.is_empty()) else {
        return (None, None);
    };
    match note.split_once(':') {
        Some((code, detail)) => (
            Some(code.trim().to_string()),
            Some(detail.trim().to_string()).filter(|detail| !detail.is_empty()),
        ),
        None => (Some(note.to_string()), None),
    }
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/diagnostics/")
)]
pub struct DiagnosticsNotificationOutboxDto {
    pub by_state: Vec<DiagnosticsOutboxStateCountDto>,
    pub reasons_24h: Vec<DiagnosticsOutboxReasonCountDto>,
    pub recent: Vec<DiagnosticsOutboxEntryDto>,
}

#[derive(Debug, FromRow)]
struct SyncLeaseRow {
    account_id: String,
    email: Option<String>,
    owner_token: String,
    lease_expires_at: String,
    updated_at: String,
    expired: i64,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/diagnostics/")
)]
pub struct DiagnosticsSyncLeaseDto {
    pub account_id: String,
    pub email: Option<String>,
    pub owner_token: String,
    pub lease_expires_at: String,
    pub updated_at: String,
    pub expired: bool,
}

impl From<SyncLeaseRow> for DiagnosticsSyncLeaseDto {
    fn from(row: SyncLeaseRow) -> Self {
        Self {
            account_id: row.account_id,
            email: row.email,
            owner_token: row.owner_token,
            lease_expires_at: row.lease_expires_at,
            updated_at: row.updated_at,
            expired: row.expired != 0,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/diagnostics/")
)]
pub struct DiagnosticsOverviewDto {
    pub generated_at: String,
    pub accounts: Vec<DiagnosticsAccountDto>,
    pub queue: DiagnosticsQueueDto,
    pub notifications: DiagnosticsNotificationsDto,
    pub notification_outbox: DiagnosticsNotificationOutboxDto,
    pub sync_leases: Vec<DiagnosticsSyncLeaseDto>,
    pub rate_limits: Vec<DiagnosticsRateLimitDto>,
}

const STATUS_COUNT_SQL: &str = "SELECT COALESCE(status, 'pending') AS status,
        COUNT(1) AS entries,
        COALESCE(SUM(retry_count), 0) AS total_retries,
        COALESCE(MAX(retry_count), 0) AS max_retries
 FROM sync_queue";

const QUEUE_ENTRY_SQL: &str = "SELECT id, account_id, action_type,
        COALESCE(status, 'pending') AS status,
        last_error AS error,
        COALESCE(retry_count, 0) AS attempts,
        next_retry_at AS next_due,
        updated_at
 FROM sync_queue";

async fn status_counts(
    pool: &SqlitePool,
    notifications: bool,
) -> Result<Vec<StatusCountRow>, String> {
    let matcher = if notifications { "LIKE" } else { "NOT LIKE" };
    let sql = format!("{STATUS_COUNT_SQL} WHERE id {matcher} ? GROUP BY 1 ORDER BY 1");
    sqlx::query_as::<_, StatusCountRow>(&sql)
        .bind(NOTIFICATION_ID_PREFIX)
        .fetch_all(pool)
        .await
        .map_err(|err| err.to_string())
}

/// Reads the two `app_settings` keys `sync::rate_limit_state` persists. An
/// empty value means the block was cleared, not that it is still armed.
async fn rate_limits(pool: &SqlitePool) -> Result<Vec<DiagnosticsRateLimitDto>, String> {
    let rows = sqlx::query_as::<_, (String, String, String)>(
        "SELECT key, value, updated_at FROM app_settings WHERE key IN (?, ?)",
    )
    .bind(GMAIL_BLOCK_UNTIL_KEY)
    .bind(GMAIL_BLOCK_STREAK_KEY)
    .fetch_all(pool)
    .await
    .map_err(|err| err.to_string())?;

    let mut blocked_until: Option<String> = None;
    let mut streak: u64 = 0;
    let mut updated_at: Option<String> = None;
    let mut seen = false;

    for (key, value, stamp) in rows {
        seen = true;
        let trimmed = value.trim().to_string();
        if key == GMAIL_BLOCK_UNTIL_KEY {
            if !trimmed.is_empty() {
                blocked_until = Some(trimmed);
            }
            updated_at = Some(stamp);
        } else if key == GMAIL_BLOCK_STREAK_KEY {
            streak = trimmed.parse::<u64>().unwrap_or(0);
        }
    }

    if !seen {
        return Ok(Vec::new());
    }

    Ok(vec![DiagnosticsRateLimitDto {
        provider: "gmail".to_string(),
        blocked_until,
        streak,
        updated_at,
    }])
}

/// Rows from the last day, grouped on the reason code in front of the colon.
/// A `not_bundled_fallback` on a delivered row counts too: the banner was
/// shown, but a click on it did nothing, which reads the same from outside.
const OUTBOX_REASON_SQL: &str = "SELECT
        TRIM(CASE WHEN INSTR(last_error, ':') > 0
                  THEN SUBSTR(last_error, 1, INSTR(last_error, ':') - 1)
                  ELSE last_error END) AS reason,
        COUNT(1) AS entries
 FROM notification_outbox
 WHERE last_error IS NOT NULL AND TRIM(last_error) <> ''
   AND julianday(COALESCE(delivered_at, created_at)) >= julianday('now', '-1 day')
 GROUP BY 1
 ORDER BY 2 DESC, 1";

async fn notification_outbox(
    pool: &SqlitePool,
) -> Result<DiagnosticsNotificationOutboxDto, String> {
    let by_state = sqlx::query_as::<_, (String, i64)>(
        "SELECT state, COUNT(1) FROM notification_outbox GROUP BY 1 ORDER BY 1",
    )
    .fetch_all(pool)
    .await
    .map_err(|err| err.to_string())?
    .into_iter()
    .map(|(state, entries)| DiagnosticsOutboxStateCountDto {
        state,
        entries: entries.max(0) as u64,
    })
    .collect();

    let reasons_24h = sqlx::query_as::<_, (String, i64)>(OUTBOX_REASON_SQL)
        .fetch_all(pool)
        .await
        .map_err(|err| err.to_string())?
        .into_iter()
        .map(|(reason, entries)| DiagnosticsOutboxReasonCountDto {
            reason,
            entries: entries.max(0) as u64,
        })
        .collect();

    // LEFT JOIN: `filter_message_gone` rows have no message any more, and the
    // FK cascade may or may not have caught up.
    let recent = sqlx::query_as::<_, OutboxEntryRow>(
        "SELECT o.message_id, o.state, o.created_at, o.delivered_at, o.last_error,
                m.subject, m.from_name, m.from_address, m.date, m.account_id
         FROM notification_outbox o
         LEFT JOIN messages m ON m.id = o.message_id
         ORDER BY COALESCE(o.delivered_at, o.created_at) DESC, o.message_id DESC
         LIMIT ?",
    )
    .bind(OUTBOX_RECENT_LIMIT)
    .fetch_all(pool)
    .await
    .map_err(|err| err.to_string())?
    .into_iter()
    .map(Into::into)
    .collect();

    Ok(DiagnosticsNotificationOutboxDto {
        by_state,
        reasons_24h,
        recent,
    })
}

async fn sync_leases(pool: &SqlitePool) -> Result<Vec<DiagnosticsSyncLeaseDto>, String> {
    Ok(sqlx::query_as::<_, SyncLeaseRow>(
        "SELECT l.account_id, a.email, l.owner_token, l.lease_expires_at, l.updated_at,
                CASE WHEN julianday(l.lease_expires_at) IS NULL
                       OR julianday(l.lease_expires_at) < julianday('now')
                     THEN 1 ELSE 0 END AS expired
         FROM account_sync_leases l
         LEFT JOIN accounts a ON a.id = l.account_id
         ORDER BY l.lease_expires_at DESC",
    )
    .fetch_all(pool)
    .await
    .map_err(|err| err.to_string())?
    .into_iter()
    .map(Into::into)
    .collect())
}

#[tauri::command]
pub async fn diagnostics_overview(
    pool: tauri::State<'_, crate::db::ReadPool>,
) -> CommandResult<DiagnosticsOverviewDto> {
    build_overview(pool.inner().as_ref())
        .await
        .map_err(Into::into)
}

pub async fn build_overview(pool: &SqlitePool) -> Result<DiagnosticsOverviewDto, String> {
    let accounts = sqlx::query_as::<_, DiagnosticsAccountDto>(
        "SELECT id, email, display_name, provider,
                COALESCE(sync_status, 'idle') AS sync_status,
                last_sync_at, last_sync_error, last_sync_error_at,
                last_oauth_error, last_oauth_error_at
         FROM accounts
         WHERE is_active = 1
         ORDER BY email ASC",
    )
    .fetch_all(pool)
    .await
    .map_err(|err| err.to_string())?;

    let queue_by_status = status_counts(pool, false).await?;
    let notification_by_status = status_counts(pool, true).await?;

    let recent_failures = sqlx::query_as::<_, QueueEntryRow>(&format!(
        "{QUEUE_ENTRY_SQL}
         WHERE id NOT LIKE ?
           AND (status = 'failed' OR last_error IS NOT NULL)
         ORDER BY updated_at DESC
         LIMIT ?"
    ))
    .bind(NOTIFICATION_ID_PREFIX)
    .bind(RECENT_LIMIT)
    .fetch_all(pool)
    .await
    .map_err(|err| err.to_string())?;

    let recent_notifications = sqlx::query_as::<_, QueueEntryRow>(&format!(
        "{QUEUE_ENTRY_SQL}
         WHERE id LIKE ?
         ORDER BY updated_at DESC
         LIMIT ?"
    ))
    .bind(NOTIFICATION_ID_PREFIX)
    .bind(RECENT_LIMIT)
    .fetch_all(pool)
    .await
    .map_err(|err| err.to_string())?;

    Ok(DiagnosticsOverviewDto {
        generated_at: chrono::Utc::now().to_rfc3339(),
        accounts,
        queue: DiagnosticsQueueDto {
            by_status: queue_by_status.into_iter().map(Into::into).collect(),
            recent_failures: recent_failures.into_iter().map(Into::into).collect(),
        },
        notifications: DiagnosticsNotificationsDto {
            by_status: notification_by_status.into_iter().map(Into::into).collect(),
            recent: recent_notifications.into_iter().map(Into::into).collect(),
        },
        notification_outbox: notification_outbox(pool).await?,
        sync_leases: sync_leases(pool).await?,
        rate_limits: rate_limits(pool).await?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    #[test]
    fn split_reason_separates_code_from_detail() {
        assert_eq!(
            split_reason(Some("filter_not_inbox: Archiv")),
            (Some("filter_not_inbox".into()), Some("Archiv".into()))
        );
        assert_eq!(
            split_reason(Some("dispatch_error: show: Error: x")),
            (Some("dispatch_error".into()), Some("show: Error: x".into()))
        );
        assert_eq!(
            split_reason(Some("permission_denied")),
            (Some("permission_denied".into()), None)
        );
        assert_eq!(split_reason(Some("  ")), (None, None));
        assert_eq!(split_reason(None), (None, None));
    }

    async fn pool() -> SqlitePool {
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
        .bind(&now)
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

    /// The outbox has a FK on `messages`, so a message goes in first; the
    /// trigger's own row (if any — backfill writes none) is then replaced.
    async fn outbox_row(pool: &SqlitePool, id: &str, state: &str, note: Option<&str>, stamp: &str) {
        sqlx::query(
            "INSERT INTO messages
             (id, account_id, folder_id, thread_id, from_address, from_name,
              to_addresses, subject, date, snippet, is_read, is_sent, is_draft,
              created_at, updated_at)
             VALUES (?, 'account', 'inbox', 'thread', 'sender@example.com', 'Sender',
                     '[]', ?, ?, '', 0, 0, 0, ?, ?)",
        )
        .bind(id)
        .bind(format!("Betreff {id}"))
        .bind(stamp)
        .bind(stamp)
        .bind(stamp)
        .execute(pool)
        .await
        .expect("message");
        sqlx::query(
            "INSERT OR REPLACE INTO notification_outbox
             (message_id, state, created_at, delivered_at, last_error)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(state)
        .bind(stamp)
        .bind(stamp)
        .bind(note)
        .execute(pool)
        .await
        .expect("outbox row");
    }

    #[tokio::test]
    async fn overview_histograms_reasons_and_reports_leases() {
        let pool = pool().await;
        let now = chrono::Utc::now();
        for (id, state, note, age_hours) in [
            ("a", "dismissed", Some("filter_already_read: x@y"), 1),
            ("b", "dismissed", Some("filter_already_read: z@y"), 2),
            ("c", "dismissed", Some("filter_not_inbox: Archiv"), 3),
            ("d", "delivered", Some("not_bundled_fallback: dev"), 4),
            (
                "burst",
                "dismissed",
                Some("filter_coalesced: burst; shown in a summary of 3"),
                4,
            ),
            ("e", "delivered", None, 5),
            ("old", "dismissed", Some("filter_not_inbox: Archiv"), 48),
        ] {
            let stamp = (now - chrono::Duration::hours(age_hours)).to_rfc3339();
            outbox_row(&pool, id, state, note, &stamp).await;
        }
        sqlx::query(
            "INSERT INTO account_sync_leases
             (account_id, owner_token, lease_expires_at, updated_at)
             VALUES ('account', 'worker-1', ?, ?)",
        )
        .bind((now - chrono::Duration::minutes(5)).to_rfc3339())
        .bind(now.to_rfc3339())
        .execute(&pool)
        .await
        .expect("lease");

        let overview = build_overview(&pool).await.expect("overview");

        let reasons: Vec<(String, u64)> = overview
            .notification_outbox
            .reasons_24h
            .iter()
            .map(|r| (r.reason.clone(), r.entries))
            .collect();
        assert_eq!(
            reasons,
            vec![
                ("filter_already_read".to_string(), 2),
                ("filter_coalesced".to_string(), 1),
                ("filter_not_inbox".to_string(), 1),
                ("not_bundled_fallback".to_string(), 1),
            ]
        );

        let by_state: Vec<(String, u64)> = overview
            .notification_outbox
            .by_state
            .iter()
            .map(|r| (r.state.clone(), r.entries))
            .collect();
        assert_eq!(
            by_state,
            vec![("delivered".to_string(), 2), ("dismissed".to_string(), 5)]
        );

        let recent = &overview.notification_outbox.recent;
        assert_eq!(recent.len(), 7);
        assert_eq!(recent[0].message_id, "a");
        assert_eq!(recent[0].reason.as_deref(), Some("filter_already_read"));
        assert_eq!(recent[0].detail.as_deref(), Some("x@y"));
        assert_eq!(recent[0].subject.as_deref(), Some("Betreff a"));
        assert_eq!(
            recent[0].from_address.as_deref(),
            Some("sender@example.com")
        );

        assert_eq!(overview.sync_leases.len(), 1);
        let lease = &overview.sync_leases[0];
        assert_eq!(lease.owner_token, "worker-1");
        assert_eq!(lease.email.as_deref(), Some("me@example.com"));
        assert!(lease.expired);
    }
}
