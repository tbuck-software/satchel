//! Resumable body eviction requested by a changed retention setting.

use super::data_repairs::DataRepair;
use futures::future::BoxFuture;
use sqlx::{SqliteConnection, SqlitePool};

pub const BODY_WINDOW: DataRepair = DataRepair {
    name: "body_window",
    block,
};

#[derive(serde::Serialize, serde::Deserialize)]
struct Cursor {
    cutoff: String,
    after: String,
}

/// Store the setting and reset or cancel its job in the same transaction.
pub async fn set_days(pool: &SqlitePool, days: i64, default: i64) -> Result<bool, sqlx::Error> {
    let days = days.clamp(0, 3650);
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let previous: Option<String> =
        sqlx::query_scalar("SELECT value FROM app_settings WHERE key = 'sync.body_window_days'")
            .fetch_optional(&mut *tx)
            .await?;
    let previous = previous
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(default)
        .clamp(0, 3650);
    if previous == days {
        tx.rollback().await?;
        return Ok(false);
    }
    let now = chrono::Utc::now();
    sqlx::query("INSERT INTO app_settings (key, value, updated_at) VALUES ('sync.body_window_days', ?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at")
        .bind(days.to_string()).bind(now.to_rfc3339()).execute(&mut *tx).await?;
    if days == 0 {
        sqlx::query("DELETE FROM app_settings WHERE key = ?")
            .bind(BODY_WINDOW.marker_key())
            .execute(&mut *tx)
            .await?;
    } else {
        let cursor = Cursor {
            cutoff: (now - chrono::Duration::days(days)).to_rfc3339(),
            after: String::new(),
        };
        sqlx::query("INSERT INTO app_settings (key, value, updated_at) VALUES (?, ?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at")
            .bind(BODY_WINDOW.marker_key()).bind(serde_json::to_string(&cursor).expect("body cursor"))
            .bind(now.to_rfc3339()).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(true)
}

fn block<'c>(
    connection: &'c mut SqliteConnection,
    cursor: &'c str,
    limit: i64,
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
        sqlx::query("UPDATE messages SET body_text = NULL, body_html = NULL, updated_at = ? WHERE id > ? AND id <= ? AND date < ? AND is_draft = 0 AND NULLIF(remote_id, '') IS NOT NULL AND (NULLIF(body_text, '') IS NOT NULL OR NULLIF(body_html, '') IS NOT NULL)")
            .bind(chrono::Utc::now().to_rfc3339()).bind(&cursor.after).bind(&last).bind(&cursor.cutoff).execute(&mut *connection).await?;
        sqlx::query("UPDATE attachments SET cached_path = NULL WHERE cached_path IS NOT NULL AND message_id IN (SELECT id FROM messages WHERE id > ? AND id <= ? AND date < ? AND is_draft = 0 AND NULLIF(remote_id, '') IS NOT NULL)")
            .bind(&cursor.after).bind(&last).bind(&cursor.cutoff).execute(&mut *connection).await?;
        cursor.after = last;
        Ok(Some(serde_json::to_string(&cursor).expect("body cursor")))
    })
}
