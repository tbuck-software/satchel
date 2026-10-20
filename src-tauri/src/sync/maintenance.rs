//! Periodic maintenance worker:
//! - Deletes trashed threads older than `maintenance.trash_auto_delete_days`.
//! - Runs `VACUUM` on desktop once a week so the file shrinks after big purges.
//! - Cleans up orphaned attachment cache files.
//!
//! Cleanup runs every six hours; checkpoints have an independent hourly loop.

use std::time::Duration;

use satchel_core::db::followups::FollowUpRepository;
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter, Manager};
use tracing::{info, warn};

use crate::db::topics::TopicRepository;

const RUN_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
#[cfg(not(any(target_os = "android", target_os = "ios")))]
const VACUUM_INTERVAL_HOURS: i64 = 24 * 7;
const CLEANUP_INTERVAL_HOURS: i64 = 24;

pub fn spawn_maintenance_worker(pool: SqlitePool, checkpoint_pool: SqlitePool, app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        // Delay first run by 30s so it doesn't compete with startup sync.
        tokio::time::sleep(Duration::from_secs(30)).await;
        // A failed pass waits for the next one. Retrying it within the minute
        // repeated whatever failed, VACUUM included, for as long as it kept
        // failing; the steps' own `last_*_at` markers decide what reruns.
        loop {
            if let Err(err) = run_once(&pool, &checkpoint_pool, &app).await {
                warn!(target: "satchel::maintenance", %err, "maintenance pass failed");
            }
            tokio::time::sleep(RUN_INTERVAL).await;
        }
    });
}

/// Independent of cleanup failures and slow network maintenance.
pub fn spawn_checkpoint_worker(pool: SqlitePool) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(30)).await;
        loop {
            let result = satchel_core::db::maintenance::checkpoint(&pool).await;
            match &result {
                Ok(row) => {
                    info!(target: "satchel::maintenance", busy = row.busy, log = row.log, checkpointed = row.checkpointed, "WAL checkpoint returned")
                }
                Err(err) => warn!(target: "satchel::maintenance", %err, "WAL checkpoint failed"),
            }
            tokio::time::sleep(satchel_core::db::maintenance::checkpoint_delay(&result)).await;
        }
    });
}

/// Data repairs a migration left a marker for, run after startup instead of
/// inside it. See `satchel_core::db::data_repairs` for the pattern.
///
/// A few seconds in, so the first frame and the first sync are not competing
/// with a table rewrite; the UI hears `data-repair:progress` while it runs.
pub(crate) async fn run_startup_data_repairs(pool: &SqlitePool, app: &AppHandle) {
    if let Err(err) = run_data_repairs(pool, |progress| {
        let _ = app.emit("data-repair:progress", progress);
    })
    .await
    {
        warn!(target: "satchel::maintenance", %err, "startup data repairs failed");
    }
}

pub(crate) fn spawn_data_repair_worker(pool: SqlitePool, app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(15)).await;
        loop {
            satchel_core::sync::worker::retry_with_backoff(
                "data repairs",
                Duration::from_secs(1),
                Duration::from_secs(60),
                || {
                    run_data_repairs(&pool, |progress| {
                        let _ = app.emit("data-repair:progress", progress);
                    })
                },
            )
            .await;
            if let Err(err) = run_pending_fts_check(&pool).await {
                warn!(target: "satchel::maintenance", %err, "requested search index check failed");
            }
            tokio::time::sleep(Duration::from_secs(30)).await;
        }
    });
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/sync/maintenance/"))]
struct DataRepairProgress {
    name: &'static str,
    running: bool,
    blocks: u64,
}

async fn run_data_repairs(
    pool: &SqlitePool,
    mut report: impl FnMut(DataRepairProgress),
) -> Result<(), String> {
    let mut failure = None;
    use satchel_core::db::data_repairs::{
        is_pending, registered_repairs, run_repair, DEFAULT_BLOCK_SIZE,
    };
    for repair in registered_repairs() {
        match is_pending(pool, repair).await {
            Ok(true) => {}
            Ok(false) => continue,
            Err(err) => {
                warn!(target: "satchel::maintenance", repair = repair.name, "repair marker unreadable: {err}");
                failure = Some(err);
                continue;
            }
        }
        report(DataRepairProgress {
            name: repair.name,
            running: true,
            blocks: 0,
        });
        let result = run_repair(pool, repair, DEFAULT_BLOCK_SIZE, None, |blocks| {
            report(DataRepairProgress {
                name: repair.name,
                running: true,
                blocks,
            });
        })
        .await;
        // The marker keeps the cursor: a failed block is retried from there on
        // the next start, not from the beginning.
        let blocks = match result {
            Ok(outcome) => {
                info!(target: "satchel::maintenance", repair = repair.name, ?outcome, "data repair ran");
                match outcome {
                    satchel_core::db::data_repairs::RepairRun::Completed { blocks }
                    | satchel_core::db::data_repairs::RepairRun::Paused { blocks } => blocks,
                    satchel_core::db::data_repairs::RepairRun::NotPending => 0,
                }
            }
            Err(err) => {
                warn!(target: "satchel::maintenance", repair = repair.name, "data repair stopped: {err}");
                failure = Some(err);
                0
            }
        };
        report(DataRepairProgress {
            name: repair.name,
            running: false,
            blocks,
        });
    }
    match failure {
        Some(error) => Err(error.to_string()),
        None => Ok(()),
    }
}

pub fn spawn_snooze_worker(pool: SqlitePool, app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            interval.tick().await;
            match wake_due_snoozes(&pool).await {
                Ok(thread_ids) if !thread_ids.is_empty() => {
                    let _ = app.emit(
                        "snooze:released",
                        serde_json::json!({ "thread_ids": thread_ids }),
                    );
                }
                Ok(_) => {}
                Err(err) => warn!(target: "satchel::maintenance", "snooze wake failed: {err}"),
            }
        }
    });
}

pub fn spawn_followup_worker(pool: SqlitePool, app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            interval.tick().await;
            match FollowUpRepository::new(&pool).claim_due().await {
                Ok(items) if !items.is_empty() => {
                    let thread_ids = items
                        .iter()
                        .map(|item| item.thread_id.clone())
                        .collect::<Vec<_>>();
                    let _ = app.emit(
                        "followup:due",
                        serde_json::json!({ "thread_ids": thread_ids, "count": items.len() }),
                    );
                    for item in items {
                        let strings = crate::commands::ui_strings::current();
                        let subject = item
                            .subject
                            .unwrap_or_else(|| strings.notification_no_subject.clone());
                        crate::commands::notifications::show_notification(
                            &app,
                            &strings.notification_follow_up_due,
                            &subject,
                        );
                    }
                }
                Ok(_) => {}
                Err(err) => warn!(target: "satchel::maintenance", "follow-up wake failed: {err}"),
            }
        }
    });
}

pub fn spawn_webhook_worker(pool: SqlitePool) {
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(15));
        loop {
            interval.tick().await;
            if let Err(err) = deliver_due_webhooks(&pool).await {
                warn!(target: "satchel::maintenance", "webhook delivery failed: {err}");
            }
        }
    });
}

async fn run_once(
    pool: &SqlitePool,
    checkpoint_pool: &SqlitePool,
    app: &AppHandle,
) -> Result<(), String> {
    #[cfg(any(target_os = "android", target_os = "ios"))]
    let _ = checkpoint_pool;
    if should_run("maintenance.last_cleanup_at", CLEANUP_INTERVAL_HOURS, pool).await {
        let removed = trash_cleanup(pool).await?;
        if removed > 0 {
            info!(
                target: "satchel::maintenance",
                removed, "deleted aged trash threads"
            );
        }
        purge_orphan_attachments(pool, app).await.ok();
        // Topic links have no FK on item_id (polymorphic); sweep the ones
        // whose thread/message has been deleted meanwhile. The timeline
        // already hides them, this keeps the table from growing stale.
        // A reply that synced before its parent starts its own thread; the
        // merge stitches those back together once both sides exist.
        match crate::sync::thread_resolution::merge_reply_split_threads(pool).await {
            Ok(moved) if moved > 0 => info!(
                target: "satchel::maintenance",
                moved, "merged reply-split threads"
            ),
            Ok(_) => {}
            Err(err) => info!(
                target: "satchel::maintenance",
                error = %err, "reply-thread merge skipped"
            ),
        }
        match TopicRepository::new(pool).cleanup_orphaned_items().await {
            Ok(removed) if removed > 0 => info!(
                target: "satchel::maintenance",
                removed, "removed orphaned topic links"
            ),
            Ok(_) => {}
            Err(err) => info!(
                target: "satchel::maintenance",
                error = %err, "topic link cleanup skipped"
            ),
        }
        // Refresh the query planner's statistics. There are fourteen indexes on
        // `messages` alone, several with overlapping `thread_id` prefixes, and
        // nothing had ever run ANALYZE — so the planner chose between them on
        // built-in guesses. `optimize` only does work when it judges it worth
        // doing, which is why it belongs on this pass rather than at startup.
        if let Err(err) = sqlx::query("PRAGMA optimize").execute(pool).await {
            warn!(
                target: "satchel::maintenance",
                error = %err, "PRAGMA optimize skipped"
            );
        }
        record_setting(pool, "maintenance.last_cleanup_at").await;
    }

    // A full SQLite VACUUM builds a second copy of the database. On Android a
    // large mailbox repeatedly exhausted the process address space here; the
    // native abort also killed the foreground service and therefore IMAP IDLE.
    // Keep the cheap WAL checkpoint and row/file cleanup above on mobile, but
    // reserve full compaction for desktop where the process has enough headroom.
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    if should_run("maintenance.last_vacuum_at", VACUUM_INTERVAL_HOURS, pool).await
        && satchel_core::db::maintenance::needs_vacuum(pool)
            .await
            .map_err(|err| err.to_string())?
    {
        let db_path = db_path(app);
        let pre = db_path.as_deref().and_then(file_size).unwrap_or(0);
        let free = db_path.as_deref().and_then(free_disk_bytes);
        if !vacuum_fits(pre, free) {
            // Not recorded: the next pass checks the disk again.
            warn!(
                target: "satchel::maintenance",
                pre, ?free, "VACUUM skipped: less free disk space than the database needs"
            );
            return Ok(());
        }
        // Recorded before the attempt. A VACUUM that fails (SQLITE_FULL, a
        // lock) then waits its week like one that succeeded, instead of
        // rewriting the whole file under an exclusive lock on every pass.
        record_setting(pool, "maintenance.last_vacuum_at").await;
        // SQLite preserves rowids for indexed tables; messages has its primary-key index.
        // The gap-preservation regression pins this against the linked SQLite version.
        sqlx::query("VACUUM")
            .execute(pool)
            .await
            .map_err(|err| err.to_string())?;
        let post = db_path.as_deref().and_then(file_size).unwrap_or(0);
        info!(
            target: "satchel::maintenance",
            pre, post, "VACUUM done"
        );
        let result = satchel_core::db::maintenance::checkpoint(checkpoint_pool).await;
        if !result.as_ref().is_ok_and(|row| row.complete()) {
            info!(target: "satchel::maintenance", ?result, "post-VACUUM checkpoint needs retry");
            let checkpoint_pool = checkpoint_pool.clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    tokio::time::sleep(satchel_core::db::maintenance::CHECKPOINT_RETRY).await;
                    if satchel_core::db::maintenance::checkpoint(&checkpoint_pool)
                        .await
                        .is_ok_and(|row| row.complete())
                    {
                        break;
                    }
                }
            });
        }
    }

    Ok(())
}

/// How long one endpoint may hold the maintenance loop.
const WEBHOOK_TIMEOUT: Duration = Duration::from_secs(10);

/// The client every webhook delivery uses.
///
/// Redirects are not followed: a 302 to a foreign host would otherwise carry
/// the payload there, past the https check that runs when the webhook is
/// saved. A redirect is a failed delivery, and is recorded as one.
fn webhook_client(timeout: Duration) -> Result<reqwest::Client, String> {
    satchel_core::utils::http::client_builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(timeout)
        .build()
        .map_err(|err| err.to_string())
}

/// `HMAC-SHA256(secret, "<timestamp>.<body>")`, hex-encoded.
///
/// The timestamp is inside the signed content, so a captured request cannot be
/// replayed against a receiver that checks its age. Signed over the exact bytes
/// that go on the wire, so signature and body cannot drift apart.
fn webhook_signature(secret: &str, timestamp: i64, body: &str) -> String {
    use hmac::Mac;
    let mut mac = hmac::Hmac::<sha2::Sha256>::new_from_slice(secret.as_bytes())
        .expect("HMAC accepts a key of any length");
    mac.update(format!("{timestamp}.").as_bytes());
    mac.update(body.as_bytes());
    mac.finalize()
        .into_bytes()
        .iter()
        .fold(String::with_capacity(64), |mut out, byte| {
            use std::fmt::Write;
            let _ = write!(out, "{byte:02x}");
            out
        })
}

async fn deliver_due_webhooks(pool: &SqlitePool) -> Result<(), String> {
    deliver_due_webhooks_with(pool, WEBHOOK_TIMEOUT).await
}

async fn deliver_due_webhooks_with(pool: &SqlitePool, timeout: Duration) -> Result<(), String> {
    let repository = crate::db::webhooks::WebhookRepository::new(pool);
    let rows = repository.load_due().await.map_err(|err| err.to_string())?;
    let client = webhook_client(timeout)?;
    for delivery in rows {
        let mut request = client
            .post(&delivery.url)
            .header("content-type", "application/json")
            .header("x-satchel-event", &delivery.event_name)
            .header("x-satchel-idempotency-key", &delivery.idempotency_key)
            .body(delivery.payload_json.clone());
        // Was `x-satchel-webhook-secret`, a bearer of the shared secret in the
        // clear. Receivers have to verify the signature instead.
        if let Some(secret) = delivery.secret.filter(|value| !value.trim().is_empty()) {
            let timestamp = chrono::Utc::now().timestamp();
            let signature = webhook_signature(&secret, timestamp, &delivery.payload_json);
            request = request
                .header("x-satchel-webhook-timestamp", timestamp.to_string())
                .header("x-satchel-webhook-signature", format!("sha256={signature}"));
        }
        match request.send().await {
            Ok(response) if response.status().is_success() => {
                repository
                    .mark_delivered(&delivery.id)
                    .await
                    .map_err(|err| err.to_string())?;
            }
            Ok(response) => {
                repository
                    .schedule_retry(
                        &delivery.id,
                        delivery.attempt_count,
                        &format!("http {}", response.status()),
                    )
                    .await
                    .map_err(|err| err.to_string())?;
            }
            Err(err) => {
                let detail = err.to_string();
                repository
                    .schedule_retry(&delivery.id, delivery.attempt_count, &detail)
                    .await
                    .map_err(|err| err.to_string())?;
            }
        }
    }
    Ok(())
}

async fn wake_due_snoozes(pool: &SqlitePool) -> Result<Vec<String>, String> {
    let now = chrono::Utc::now().to_rfc3339();
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|err| err.to_string())?;
    let thread_ids: Vec<String> = sqlx::query_scalar(
        "SELECT id
         FROM threads
         WHERE workflow_state = 'snoozed'
           AND snooze_until IS NOT NULL
           AND snooze_until <= ?",
    )
    .bind(&now)
    .fetch_all(&mut *tx)
    .await
    .map_err(|err| err.to_string())?;

    for thread_id in &thread_ids {
        sqlx::query(
            "UPDATE threads
             SET workflow_state = 'inbox',
                 board_column = 'inbox',
                 snooze_until = NULL,
                 workflow_changed_at = ?,
                 updated_at = ?
             WHERE id = ?",
        )
        .bind(&now)
        .bind(&now)
        .bind(thread_id)
        .execute(&mut *tx)
        .await
        .map_err(|err| err.to_string())?;
    }

    tx.commit().await.map_err(|err| err.to_string())?;
    Ok(thread_ids)
}

async fn should_run(setting_key: &str, hours_between: i64, pool: &SqlitePool) -> bool {
    let last: Option<String> = sqlx::query_scalar("SELECT value FROM app_settings WHERE key = ?")
        .bind(setting_key)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
    let Some(last) = last.filter(|s| !s.trim().is_empty()) else {
        return true;
    };
    match chrono::DateTime::parse_from_rfc3339(&last) {
        Ok(dt) => {
            let elapsed = chrono::Utc::now().signed_duration_since(dt.with_timezone(&chrono::Utc));
            elapsed.num_hours() >= hours_between
        }
        Err(_) => true,
    }
}

async fn record_setting(pool: &SqlitePool, key: &str) {
    let now = chrono::Utc::now().to_rfc3339();
    let _ = sqlx::query(
        "INSERT INTO app_settings (key, value, updated_at) VALUES (?, ?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
    )
    .bind(key)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await;
}

/// Check the message search index against its content table, and rebuild it
/// when the two have drifted apart.
///
/// `messages_fts` is external-content FTS5: the index holds terms, `messages`
/// holds the rows, and only the `messages_ai`/`ad`/`au` triggers keep them
/// aligned. Anything that writes `messages` without firing those triggers — a
/// manual `sqlite3` session, a restore of a file assembled elsewhere — leaves
/// an index that answers for messages that are gone or stays silent about ones
/// that are there. `PRAGMA integrity_check` does not look at this; only
/// FTS5's own `integrity-check` does.
///
/// Returns whether a repair was necessary.
pub(crate) async fn verify_and_repair_fts(pool: &SqlitePool) -> Result<bool, String> {
    if satchel_core::db::message_index_intact(pool).await? {
        return Ok(false);
    }
    warn!(
        target: "satchel::maintenance",
        "message search index failed its integrity check; rebuilding"
    );
    sqlx::query("INSERT INTO messages_fts(messages_fts) VALUES('rebuild')")
        .execute(pool)
        .await
        .map_err(|err| err.to_string())?;
    rebuild_search_projection(pool).await?;
    // A rebuild that does not fix it is worse news than the drift.
    if !satchel_core::db::message_index_intact(pool).await? {
        return Err("message search index is still inconsistent after a rebuild".into());
    }
    Ok(true)
}

/// `messages` has no explicit `INTEGER PRIMARY KEY`; `message_search_projection`
/// and `messages_fts` both key off the implicit rowid. Re-derive the projection
/// from `messages` whenever the index next to it had to be rebuilt.
async fn rebuild_search_projection(pool: &SqlitePool) -> Result<(), String> {
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|err| err.to_string())?;
    sqlx::query("DELETE FROM message_search_projection")
        .execute(&mut *tx)
        .await
        .map_err(|err| err.to_string())?;
    sqlx::query(
        "INSERT INTO message_search_projection
             (message_rowid, message_id, thread_id, account_id, date)
         SELECT rowid, id, thread_id, account_id, COALESCE(date, '') FROM messages",
    )
    .execute(&mut *tx)
    .await
    .map_err(|err| err.to_string())?;
    tx.commit().await.map_err(|err| err.to_string())
}

#[cfg(test)]
async fn body_window_cleanup(pool: &SqlitePool) -> Result<u64, String> {
    use satchel_core::db::{
        body_window::{set_days, BODY_WINDOW},
        data_repairs::run_repair,
    };
    let days: String =
        sqlx::query_scalar("SELECT value FROM app_settings WHERE key = 'sync.body_window_days'")
            .fetch_one(pool)
            .await
            .map_err(|err| err.to_string())?;
    let before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE NULLIF(body_text, '') IS NOT NULL OR NULLIF(body_html, '') IS NOT NULL").fetch_one(pool).await.map_err(|err| err.to_string())?;
    sqlx::query("DELETE FROM app_settings WHERE key = 'sync.body_window_days'")
        .execute(pool)
        .await
        .map_err(|err| err.to_string())?;
    set_days(pool, days.parse().unwrap(), 0)
        .await
        .map_err(|err| err.to_string())?;
    run_repair(pool, &BODY_WINDOW, 1000, None, |_| {})
        .await
        .map_err(|err| err.to_string())?;
    let after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE NULLIF(body_text, '') IS NOT NULL OR NULLIF(body_html, '') IS NOT NULL").fetch_one(pool).await.map_err(|err| err.to_string())?;
    Ok((before - after) as u64)
}

async fn run_pending_fts_check(pool: &SqlitePool) -> Result<(), String> {
    let key = satchel_core::db::maintenance::FTS_CHECK_PENDING;
    let pending: Option<String> =
        sqlx::query_scalar("SELECT value FROM app_settings WHERE key = ?")
            .bind(key)
            .fetch_optional(pool)
            .await
            .map_err(|err| err.to_string())?;
    if let Some(generation) = pending {
        verify_and_repair_fts(pool).await?;
        // Keep the request after a crash, and do not consume a newer import's request.
        sqlx::query("DELETE FROM app_settings WHERE key = ? AND value = ?")
            .bind(key)
            .bind(generation)
            .execute(pool)
            .await
            .map_err(|err| err.to_string())?;
    }
    Ok(())
}

async fn trash_cleanup(pool: &SqlitePool) -> Result<u64, String> {
    let days_setting: Option<String> = sqlx::query_scalar(
        "SELECT value FROM app_settings WHERE key = 'maintenance.trash_auto_delete_days'",
    )
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();
    let days = days_setting
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(30)
        .clamp(0, 3650);
    let removed = if days == 0 {
        0 // 0 = disabled; queue retention below remains independent.
    } else {
        let cutoff = (chrono::Utc::now() - chrono::Duration::days(days)).to_rfc3339();
        crate::db::threads::ThreadRepository::new(pool)
            .purge_trash_before(&cutoff)
            .await
            .map_err(|err| err.to_string())?
    };

    // Drop failed outbox actions older than 60 days independently from the
    // thread purge. Queue rows are durable retry state and must not share the
    // Core repository's aggregate transaction.
    let outbox_cutoff = (chrono::Utc::now() - chrono::Duration::days(60)).to_rfc3339();
    satchel_core::mail_actions::purge_failed_queue_rows(pool, &outbox_cutoff).await?;

    Ok(removed)
}

async fn purge_orphan_attachments(pool: &SqlitePool, app: &AppHandle) -> Result<(), String> {
    let cache_dir = match app
        .path()
        .resolve("satchel/attachments", tauri::path::BaseDirectory::AppData)
    {
        Ok(path) => path,
        Err(_) => return Ok(()),
    };
    if !cache_dir.exists() {
        return Ok(());
    }
    let known: std::collections::HashSet<String> = sqlx::query_scalar::<_, String>(
        "SELECT cached_path FROM attachments WHERE cached_path IS NOT NULL",
    )
    .fetch_all(pool)
    .await
    .map_err(|err| err.to_string())?
    .into_iter()
    .collect();

    let mut entries = match tokio::fs::read_dir(&cache_dir).await {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        let path_str = path.to_string_lossy().to_string();
        if !known.contains(&path_str) {
            let _ = tokio::fs::remove_file(&path).await;
        }
    }
    Ok(())
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn db_path(app: &AppHandle) -> Option<std::path::PathBuf> {
    app.path()
        .resolve("satchel/satchel.db", tauri::path::BaseDirectory::AppData)
        .ok()
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn file_size(path: &std::path::Path) -> Option<u64> {
    std::fs::metadata(path).ok().map(|m| m.len())
}

/// VACUUM writes a complete second copy of the database before it can drop
/// the first. With less free space than that it can only fail, and fail late.
/// An unknown amount of free space does not block it.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn vacuum_fits(db_size: u64, free: Option<u64>) -> bool {
    free.is_none_or(|free| free >= db_size)
}

/// Bytes available to this user on the file system holding `path`.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn free_disk_bytes(path: &std::path::Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let directory = path.parent()?;
    let directory = std::ffi::CString::new(directory.as_os_str().as_bytes()).ok()?;
    let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: `directory` is NUL-terminated and outlives the call; `stats` is
    // only read after statvfs reported that it filled it in.
    let stats = unsafe {
        if libc::statvfs(directory.as_ptr(), stats.as_mut_ptr()) != 0 {
            return None;
        }
        stats.assume_init()
    };
    // The field widths differ between Linux and macOS.
    #[allow(clippy::unnecessary_cast)]
    let free = (stats.f_bavail as u64).saturating_mul(stats.f_frsize as u64);
    Some(free)
}

#[cfg(all(
    not(any(target_os = "android", target_os = "ios")),
    not(any(target_os = "macos", target_os = "linux"))
))]
fn free_disk_bytes(_path: &std::path::Path) -> Option<u64> {
    None
}

#[cfg(all(test, not(any(target_os = "android", target_os = "ios"))))]
mod vacuum_tests {
    use super::vacuum_fits;

    #[test]
    fn vacuum_needs_room_for_a_second_copy() {
        assert!(vacuum_fits(2_000, Some(2_000)));
        assert!(!vacuum_fits(2_000, Some(1_999)));
        assert!(vacuum_fits(2_000, None));
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn free_disk_space_is_read_for_the_database_directory() {
        let directory = tempfile::tempdir().expect("temp directory");
        let free = super::free_disk_bytes(&directory.path().join("satchel.db"));
        assert!(free.is_some_and(|bytes| bytes > 0), "{free:?}");
    }
}

#[cfg(test)]
mod body_window_tests {
    use sqlx::sqlite::SqlitePoolOptions;
    use sqlx::SqlitePool;

    async fn pool_with_messages() -> SqlitePool {
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
        let old = (chrono::Utc::now() - chrono::Duration::days(400)).to_rfc3339();
        sqlx::query(
            "INSERT INTO accounts
             (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
             VALUES ('acc', 'Me', 'me@example.com', 'imap', '{}', '', ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("account");
        sqlx::query(
            "INSERT INTO folders
             (id, account_id, name, full_path, remote_id, folder_type, created_at, updated_at)
             VALUES ('fld', 'acc', 'INBOX', 'INBOX', 'INBOX', 'inbox', ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("folder");

        for (id, date, remote, draft) in [
            ("old-remote", &old, Some("12"), 0),
            ("old-local", &old, None, 0),
            ("old-draft", &old, Some("13"), 1),
            ("recent", &now, Some("14"), 0),
        ] {
            sqlx::query(
                "INSERT INTO messages
                 (id, account_id, folder_id, from_address, to_addresses, date, body_text,
                  body_html, snippet, is_draft, remote_id, created_at, updated_at)
                 VALUES (?, 'acc', 'fld', 'a@b.c', '[]', ?, 'Rumpf', '<p>Rumpf</p>',
                         'Vorschau', ?, ?, ?, ?)",
            )
            .bind(id)
            .bind(date)
            .bind(draft)
            .bind(remote)
            .bind(&now)
            .bind(&now)
            .execute(&pool)
            .await
            .expect("message");
        }

        sqlx::query(
            "INSERT INTO app_settings (key, value, updated_at)
             VALUES ('sync.body_window_days', '90', ?)",
        )
        .bind(&now)
        .execute(&pool)
        .await
        .expect("setting");
        pool
    }

    async fn body_of(pool: &SqlitePool, id: &str) -> Option<String> {
        sqlx::query_scalar::<_, Option<String>>("SELECT body_text FROM messages WHERE id = ?")
            .bind(id)
            .fetch_one(pool)
            .await
            .expect("row")
    }

    #[tokio::test]
    async fn clears_old_bodies_that_can_be_fetched_again() {
        let pool = pool_with_messages().await;
        let cleared = super::body_window_cleanup(&pool).await.expect("cleanup");
        assert_eq!(cleared, 1);
        assert_eq!(body_of(&pool, "old-remote").await, None);
        // The preview line lives in its own column and is what keeps the list
        // looking the same after the body is gone.
        let snippet: Option<String> =
            sqlx::query_scalar("SELECT snippet FROM messages WHERE id = 'old-remote'")
                .fetch_one(&pool)
                .await
                .expect("snippet");
        assert_eq!(snippet.as_deref(), Some("Vorschau"));
    }

    /// Nothing else has a copy of these. Emptying them would be deletion.
    #[tokio::test]
    async fn spares_what_cannot_be_fetched_back() {
        let pool = pool_with_messages().await;
        super::body_window_cleanup(&pool).await.expect("cleanup");
        assert_eq!(body_of(&pool, "old-local").await.as_deref(), Some("Rumpf"));
        assert_eq!(body_of(&pool, "old-draft").await.as_deref(), Some("Rumpf"));
    }

    /// The cached file is what the space actually was. The row stays so the
    /// attachment is still listed and can be downloaded again.
    #[tokio::test]
    async fn releases_cached_files_for_pruned_mail() {
        let pool = pool_with_messages().await;
        let now = chrono::Utc::now().to_rfc3339();
        for (id, message) in [("att-old", "old-remote"), ("att-new", "recent")] {
            sqlx::query(
                "INSERT INTO attachments
                 (id, message_id, filename, content_type, size, cached_path, created_at)
                 VALUES (?, ?, 'bild.png', 'image/png', 10, '/cache/bild.png', ?)",
            )
            .bind(id)
            .bind(message)
            .bind(&now)
            .execute(&pool)
            .await
            .expect("attachment");
        }

        super::body_window_cleanup(&pool).await.expect("cleanup");

        let cached = |id: &'static str| {
            let pool = pool.clone();
            async move {
                sqlx::query_scalar::<_, Option<String>>(
                    "SELECT cached_path FROM attachments WHERE id = ?",
                )
                .bind(id)
                .fetch_one(&pool)
                .await
                .expect("row")
            }
        };
        assert_eq!(cached("att-old").await, None);
        assert_eq!(cached("att-new").await.as_deref(), Some("/cache/bild.png"));
        let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM attachments")
            .fetch_one(&pool)
            .await
            .expect("count");
        assert_eq!(rows, 2, "the attachment rows themselves must survive");
    }

    #[tokio::test]
    async fn leaves_mail_inside_the_window_alone() {
        let pool = pool_with_messages().await;
        super::body_window_cleanup(&pool).await.expect("cleanup");
        assert_eq!(body_of(&pool, "recent").await.as_deref(), Some("Rumpf"));
    }

    /// Zero is the user switching the limit off, not an instruction to empty
    /// every message in the database.
    #[tokio::test]
    async fn a_zero_window_clears_nothing() {
        let pool = pool_with_messages().await;
        sqlx::query("UPDATE app_settings SET value = '0' WHERE key = 'sync.body_window_days'")
            .execute(&pool)
            .await
            .expect("setting");
        assert_eq!(super::body_window_cleanup(&pool).await.expect("cleanup"), 0);
        assert_eq!(body_of(&pool, "old-remote").await.as_deref(), Some("Rumpf"));
    }

    /// A second run must find nothing left to do, or the pass rewrites the same
    /// rows every day and the WAL grows for no reason.
    #[tokio::test]
    async fn a_second_pass_touches_nothing() {
        let pool = pool_with_messages().await;
        super::body_window_cleanup(&pool).await.expect("first");
        assert_eq!(super::body_window_cleanup(&pool).await.expect("second"), 0);
    }

    async fn fts_hits(pool: &SqlitePool, term: &str) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM messages_fts WHERE messages_fts MATCH ?")
            .bind(term)
            .fetch_one(pool)
            .await
            .expect("search")
    }

    /// The price of the body window, stated as a test rather than left to be
    /// discovered: the terms go with the body, and only subject and sender
    /// survive it. `body_window_cleanup`'s own doc comment says the same.
    #[tokio::test]
    async fn body_window_cleanup_drops_the_body_from_the_search_index() {
        let pool = pool_with_messages().await;
        let old = (chrono::Utc::now() - chrono::Duration::days(400)).to_rfc3339();
        sqlx::query(
            "INSERT INTO messages
             (id, account_id, folder_id, from_address, to_addresses, subject, date,
              body_text, body_html, snippet, is_draft, remote_id, created_at, updated_at)
             VALUES ('indexed', 'acc', 'fld', 'a@b.c', '[]', 'Quokka', ?,
                     'Zebrafisch', '<p>Zebrafisch</p>', 'Vorschau', 0, '99', ?, ?)",
        )
        .bind(&old)
        .bind(&old)
        .bind(&old)
        .execute(&pool)
        .await
        .expect("message");

        assert_eq!(fts_hits(&pool, "Zebrafisch").await, 1);
        assert_eq!(fts_hits(&pool, "Quokka").await, 1);

        super::body_window_cleanup(&pool).await.expect("cleanup");

        assert_eq!(
            fts_hits(&pool, "Zebrafisch").await,
            0,
            "the body left the index with the column"
        );
        assert_eq!(
            fts_hits(&pool, "Quokka").await,
            1,
            "subject and sender stay searchable"
        );
    }

    /// Drift the triggers cannot see: a message written while the insert
    /// trigger was absent, which is what a manual `sqlite3` session or a
    /// foreign file amounts to.
    #[tokio::test]
    async fn a_corrupted_fts_index_is_detected_and_rebuilt() {
        let pool = pool_with_messages().await;
        assert!(satchel_core::db::message_index_intact(&pool)
            .await
            .expect("healthy index"));

        sqlx::query("DROP TRIGGER messages_ai")
            .execute(&pool)
            .await
            .expect("drop insert trigger");
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO messages
             (id, account_id, folder_id, from_address, to_addresses, subject, date,
              body_text, snippet, is_draft, remote_id, created_at, updated_at)
             VALUES ('unindexed', 'acc', 'fld', 'a@b.c', '[]', 'Narwal', ?,
                     'Narwal im Rumpf', 'Vorschau', 0, '77', ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("unindexed message");

        assert!(!satchel_core::db::message_index_intact(&pool)
            .await
            .expect("check"));
        assert_eq!(fts_hits(&pool, "Narwal").await, 0);

        assert!(
            super::verify_and_repair_fts(&pool).await.expect("repair"),
            "the drift should have triggered a rebuild"
        );

        assert!(satchel_core::db::message_index_intact(&pool)
            .await
            .expect("repaired index"));
        assert_eq!(fts_hits(&pool, "Narwal").await, 1);
        let projected: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM message_search_projection WHERE message_id = 'unindexed'",
        )
        .fetch_one(&pool)
        .await
        .expect("projection");
        assert_eq!(projected, 1);
    }

    #[tokio::test]
    async fn ordinary_launch_does_not_check_fts() {
        let pool = pool_with_messages().await;
        sqlx::query("INSERT INTO messages_fts(messages_fts) VALUES('delete-all')")
            .execute(&pool)
            .await
            .unwrap();
        super::run_pending_fts_check(&pool).await.unwrap();
        assert!(
            !satchel_core::db::message_index_intact(&pool).await.unwrap(),
            "unrequested index must be left alone"
        );
        satchel_core::db::maintenance::request_fts_check(&pool)
            .await
            .unwrap();
        super::run_pending_fts_check(&pool).await.unwrap();
        assert!(satchel_core::db::message_index_intact(&pool).await.unwrap());
        let pending: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM app_settings WHERE key = 'maintenance.fts_check_pending'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(pending, 0);
    }

    #[tokio::test]
    async fn a_failed_requested_fts_check_keeps_its_marker() {
        let pool = pool_with_messages().await;
        sqlx::query("INSERT INTO messages_fts(messages_fts) VALUES('delete-all')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DROP TABLE message_search_projection")
            .execute(&pool)
            .await
            .unwrap();
        satchel_core::db::maintenance::request_fts_check(&pool)
            .await
            .unwrap();
        assert!(super::run_pending_fts_check(&pool).await.is_err());
        let pending: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM app_settings WHERE key = 'maintenance.fts_check_pending'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(pending, 1);
    }

    /// A healthy index must not be rebuilt on every pass.
    #[tokio::test]
    async fn a_healthy_fts_index_is_left_alone() {
        let pool = pool_with_messages().await;
        assert!(!super::verify_and_repair_fts(&pool).await.expect("check"));
    }
}

#[cfg(test)]
mod webhook_tests {
    use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
    use std::time::{Duration, Instant};
    use tokio::net::TcpListener;

    /// One pending delivery pointing at `url`, with a secret.
    async fn pool_with_due_delivery(url: &str) -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("pool");
        sqlx::migrate!("../crates/satchel-core/src/db/migrations")
            .run(&pool)
            .await
            .expect("migrations");
        let past = (chrono::Utc::now() - chrono::Duration::minutes(1)).to_rfc3339();
        sqlx::query(
            "INSERT INTO local_webhooks (id, url, events_json, enabled, secret, created_at, updated_at)
             VALUES ('hook', ?, '[\"mail.received\"]', 1, 'shhh', ?, ?)",
        )
        .bind(url)
        .bind(&past)
        .bind(&past)
        .execute(&pool)
        .await
        .expect("webhook");
        sqlx::query(
            "INSERT INTO webhook_deliveries
             (id, webhook_id, event_name, idempotency_key, payload_json, status,
              attempt_count, next_attempt_at, created_at, updated_at)
             VALUES ('delivery', 'hook', 'mail.received', 'key-1', '{\"a\":1}', 'pending',
                     0, ?, ?, ?)",
        )
        .bind(&past)
        .bind(&past)
        .bind(&past)
        .execute(&pool)
        .await
        .expect("delivery");
        pool
    }

    async fn delivery_state(pool: &SqlitePool) -> (String, i64, Option<String>) {
        sqlx::query_as("SELECT status, attempt_count, last_error FROM webhook_deliveries WHERE id = 'delivery'")
            .fetch_one(pool)
            .await
            .expect("delivery row")
    }

    /// The signature covers the timestamp, so the same body signed a second
    /// later is a different signature — a captured request cannot be replayed.
    #[test]
    fn the_signature_covers_the_timestamp_and_the_body() {
        let first = super::webhook_signature("shhh", 1_700_000_000, "{\"a\":1}");
        assert_eq!(first.len(), 64, "hex-encoded SHA-256");
        assert_eq!(
            first,
            super::webhook_signature("shhh", 1_700_000_000, "{\"a\":1}")
        );
        assert_ne!(
            first,
            super::webhook_signature("shhh", 1_700_000_001, "{\"a\":1}")
        );
        assert_ne!(
            first,
            super::webhook_signature("shhh", 1_700_000_000, "{\"a\":2}")
        );
        assert_ne!(
            first,
            super::webhook_signature("other", 1_700_000_000, "{\"a\":1}")
        );
    }

    /// An endpoint that accepts the connection and never answers used to hold
    /// the maintenance loop for as long as it felt like.
    #[tokio::test]
    async fn a_hanging_endpoint_cannot_hold_the_loop() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
        let url = format!("http://{}/hook", listener.local_addr().expect("addr"));
        // Accept and then say nothing at all.
        tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((stream, _)) = listener.accept().await {
                held.push(stream);
            }
        });

        let pool = pool_with_due_delivery(&url).await;
        let started = Instant::now();
        super::deliver_due_webhooks_with(&pool, Duration::from_millis(300))
            .await
            .expect("the loop returns rather than hanging");

        assert!(
            started.elapsed() < Duration::from_secs(5),
            "took {:?}",
            started.elapsed()
        );
        let (status, attempts, error) = delivery_state(&pool).await;
        assert_eq!((status.as_str(), attempts), ("pending", 1));
        assert!(error.is_some(), "the timeout must be recorded");
    }

    /// A 302 to somewhere else is a failed delivery, not a forward: following
    /// it would carry the payload past the https check made when saving.
    #[tokio::test]
    async fn a_redirect_is_not_followed() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
        let url = format!("http://{}/hook", listener.local_addr().expect("addr"));
        tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            while let Ok((mut stream, _)) = listener.accept().await {
                let _ = stream
                    .write_all(
                        b"HTTP/1.1 302 Found\r\nLocation: http://198.51.100.7/steal\r\nContent-Length: 0\r\n\r\n",
                    )
                    .await;
                let _ = stream.flush().await;
            }
        });

        let pool = pool_with_due_delivery(&url).await;
        super::deliver_due_webhooks_with(&pool, Duration::from_secs(5))
            .await
            .expect("delivery attempt");

        let (status, _, error) = delivery_state(&pool).await;
        assert_eq!(status, "pending");
        assert_eq!(error.as_deref(), Some("http 302 Found"));
    }
}
