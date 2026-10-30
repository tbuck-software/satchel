//! Recurring, resumable eviction of refetchable bodies outside the retention window.

use super::data_repairs::{DataRepair, Prepared};
use futures::future::BoxFuture;
use sqlx::{SqliteConnection, SqlitePool};

const NEXT_RUN_KEY: &str = "maintenance.body_window_next_at";
const MAX_WINDOW_DAYS: i64 = 3650;

pub const BODY_WINDOW: DataRepair = DataRepair {
    name: "body_window",
    block,
    prepare: None,
};

#[derive(serde::Serialize, serde::Deserialize)]
struct Cursor {
    cutoff: String,
    after: String,
}

async fn configured_days(
    connection: &mut SqliteConnection,
    default: i64,
) -> Result<i64, sqlx::Error> {
    let raw: Option<String> =
        sqlx::query_scalar("SELECT value FROM app_settings WHERE key = 'sync.body_window_days'")
            .fetch_optional(connection)
            .await?;
    Ok(raw
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(default)
        .clamp(0, MAX_WINDOW_DAYS))
}

/// Store the setting and reset or cancel its job in the same transaction.
pub async fn set_days(pool: &SqlitePool, days: i64, default: i64) -> Result<bool, sqlx::Error> {
    let days = days.clamp(0, MAX_WINDOW_DAYS);
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let previous = configured_days(&mut tx, default).await?;
    if previous == days {
        tx.rollback().await?;
        return Ok(false);
    }
    let now = chrono::Utc::now();
    sqlx::query("INSERT INTO app_settings (key, value, updated_at) VALUES ('sync.body_window_days', ?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at")
        .bind(days.to_string()).bind(now.to_rfc3339()).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM app_settings WHERE key = ?")
        .bind(BODY_WINDOW.marker_key())
        .execute(&mut *tx)
        .await?;
    if days == 0 {
        sqlx::query("DELETE FROM app_settings WHERE key = ?")
            .bind(NEXT_RUN_KEY)
            .execute(&mut *tx)
            .await?;
    } else {
        enqueue(&mut tx, days, now).await?;
    }
    tx.commit().await?;
    Ok(true)
}

/// Schedule at most one pass a day, without touching an active repair cursor.
/// `now` is supplied by the worker so elapsed policy time is testable without
/// sleeping. Scheduling only touches settings; message scans stay in repair blocks.
pub async fn schedule_due(
    pool: &SqlitePool,
    default: i64,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<bool, sqlx::Error> {
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let days = configured_days(&mut tx, default).await?;
    let pending: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM app_settings WHERE key = ?)")
            .bind(BODY_WINDOW.marker_key())
            .fetch_one(&mut *tx)
            .await?;
    if days == 0 || pending {
        tx.rollback().await?;
        return Ok(false);
    }
    let next: Option<String> = sqlx::query_scalar("SELECT value FROM app_settings WHERE key = ?")
        .bind(NEXT_RUN_KEY)
        .fetch_optional(&mut *tx)
        .await?;
    if next
        .and_then(|v| chrono::DateTime::parse_from_rfc3339(&v).ok())
        .is_some_and(|next| now < next)
    {
        tx.rollback().await?;
        return Ok(false);
    }
    enqueue(&mut tx, days, now).await?;
    tx.commit().await?;
    Ok(true)
}

async fn enqueue(
    connection: &mut SqliteConnection,
    days: i64,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<(), sqlx::Error> {
    let cursor = Cursor {
        cutoff: (now - chrono::Duration::days(days)).to_rfc3339(),
        after: String::new(),
    };
    sqlx::query("INSERT INTO app_settings (key, value, updated_at) VALUES (?, ?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at")
        .bind(BODY_WINDOW.marker_key()).bind(serde_json::to_string(&cursor).expect("body cursor"))
        .bind(now.to_rfc3339()).execute(&mut *connection).await?;
    sqlx::query("INSERT INTO app_settings (key, value, updated_at) VALUES (?, ?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at")
        .bind(NEXT_RUN_KEY).bind((now + chrono::Duration::days(1)).to_rfc3339())
        .bind(now.to_rfc3339()).execute(&mut *connection).await?;
    Ok(())
}

fn block<'c>(
    connection: &'c mut SqliteConnection,
    cursor: &'c str,
    limit: i64,
    _prepared: Option<Prepared>,
) -> BoxFuture<'c, Result<Option<String>, sqlx::Error>> {
    Box::pin(async move {
        let mut cursor: Cursor =
            serde_json::from_str(cursor).map_err(|err| sqlx::Error::Protocol(err.to_string()))?;
        // Limit the scan itself, including rows that need no eviction.
        let last: Option<String> = sqlx::query_scalar(
            "SELECT MAX(id) FROM (SELECT id FROM messages WHERE id > ? ORDER BY id LIMIT ?)",
        )
        .bind(&cursor.after)
        .bind(limit)
        .fetch_one(&mut *connection)
        .await?;
        let Some(last) = last else {
            return Ok(None);
        };
        // The unsanitized original and the raw source are usually the largest
        // copies of the body; the provider still holds all of them. Imported
        // messages carry a synthetic `import-` remote id and exist nowhere
        // else, so their copies stay. POP3 may delete the server copy after
        // ingestion; its original and attachment bytes must stay local too.
        // Only known refetchable providers qualify; unknown/local sources stay.
        sqlx::query("UPDATE messages SET body_text = NULL, body_html = NULL, original_body_html = NULL, original_source = NULL, updated_at = ? WHERE id > ? AND id <= ? AND date < ? AND is_draft = 0 AND NULLIF(remote_id, '') IS NOT NULL AND remote_id NOT LIKE 'import-%' AND account_id IN (SELECT id FROM accounts WHERE provider IN ('imap', 'gmail', 'outlook')) AND (NULLIF(body_text, '') IS NOT NULL OR NULLIF(body_html, '') IS NOT NULL OR NULLIF(original_body_html, '') IS NOT NULL OR original_source IS NOT NULL)")
            .bind(chrono::Utc::now().to_rfc3339()).bind(&cursor.after).bind(&last).bind(&cursor.cutoff).execute(&mut *connection).await?;
        sqlx::query("UPDATE attachments SET cached_path = NULL WHERE cached_path IS NOT NULL AND message_id IN (SELECT id FROM messages WHERE id > ? AND id <= ? AND date < ? AND is_draft = 0 AND NULLIF(remote_id, '') IS NOT NULL AND remote_id NOT LIKE 'import-%' AND account_id IN (SELECT id FROM accounts WHERE provider IN ('imap', 'gmail', 'outlook')))")
            .bind(&cursor.after).bind(&last).bind(&cursor.cutoff).execute(&mut *connection).await?;
        cursor.after = last;
        Ok(Some(serde_json::to_string(&cursor).expect("body cursor")))
    })
}
