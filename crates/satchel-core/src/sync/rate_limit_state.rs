//! Provider rate-limit blocks that outlive the process.
//!
//! The pacer that enforces a block lives in the provider, next to the request
//! it has to hold back, and has no database of its own. Persistence therefore
//! lives here, in the sync layer, which owns the pool and brackets every piece
//! of provider work: [`restore`] runs once at startup before any worker can
//! issue a request, [`persist`] after work that may have earned a block.
//!
//! Forgetting a block is not a cosmetic loss. Gmail measures on a moving
//! window, so a request sent while a block is still running renews it — an app
//! that forgets on restart re-earns the penalty every launch instead of ever
//! waiting one out.

use sqlx::SqlitePool;

use crate::providers::gmail;

const GMAIL_BLOCK_UNTIL: &str = "gmail_rate_limit_block_until";
const GMAIL_BLOCK_STREAK: &str = "gmail_rate_limit_block_streak";

async fn read_setting(pool: &SqlitePool, key: &str) -> Option<String> {
    sqlx::query_scalar::<_, String>("SELECT value FROM app_settings WHERE key = ?")
        .bind(key)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
}

async fn write_setting(pool: &SqlitePool, key: &str, value: &str) {
    let _ = sqlx::query(
        "INSERT INTO app_settings (key, value, updated_at) VALUES (?, ?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
    )
    .bind(key)
    .bind(value)
    .bind(chrono::Utc::now().to_rfc3339())
    .execute(pool)
    .await;
}

/// Put back whatever is left of the block the last run recorded.
pub async fn restore(pool: &SqlitePool) {
    let Some(until) = read_setting(pool, GMAIL_BLOCK_UNTIL).await else {
        return;
    };
    let streak = read_setting(pool, GMAIL_BLOCK_STREAK)
        .await
        .and_then(|value| value.trim().parse::<u32>().ok())
        .unwrap_or(1);
    let Some(remaining) = chrono::DateTime::parse_from_rfc3339(until.trim())
        .ok()
        .map(|until| until.signed_duration_since(chrono::Utc::now()))
        .and_then(|delta| delta.to_std().ok())
    else {
        return;
    };
    gmail::restore_block(remaining, streak).await;
}

/// Record the current block, or clear the stored one once it has lapsed.
///
/// Cheap enough to call after every sync run: two upserts at most, and only
/// when the state actually differs from what is on disk.
pub async fn persist(pool: &SqlitePool) {
    let stored = read_setting(pool, GMAIL_BLOCK_UNTIL).await;
    match gmail::block_snapshot().await {
        Some((remaining, streak)) => {
            let until = chrono::Utc::now()
                + chrono::Duration::from_std(remaining)
                    .unwrap_or_else(|_| chrono::Duration::zero());
            write_setting(pool, GMAIL_BLOCK_UNTIL, &until.to_rfc3339()).await;
            write_setting(pool, GMAIL_BLOCK_STREAK, &streak.to_string()).await;
        }
        // Leaving a lapsed deadline behind would be harmless but confusing in
        // the settings table, and a stale streak would make the next block
        // start halfway up the escalation ladder.
        None if stored.is_some() => {
            write_setting(pool, GMAIL_BLOCK_UNTIL, "").await;
            write_setting(pool, GMAIL_BLOCK_STREAK, "0").await;
        }
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    /// The round trip a restart makes: a block recorded by one run has to be
    /// back in force in the next, streak included, so the escalation continues
    /// instead of starting over at the server's shortest figure.
    #[tokio::test]
    async fn a_block_survives_a_restart() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("pool");
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .expect("migrations");

        gmail::restore_block(std::time::Duration::from_secs(900), 3).await;
        persist(&pool).await;
        let stored = read_setting(&pool, GMAIL_BLOCK_UNTIL)
            .await
            .expect("deadline stored");
        assert!(chrono::DateTime::parse_from_rfc3339(stored.trim()).is_ok());
        assert_eq!(
            read_setting(&pool, GMAIL_BLOCK_STREAK).await.as_deref(),
            Some("3")
        );

        // A fresh process starts with an empty pacer.
        gmail::clear_block_for_test().await;
        assert!(gmail::block_snapshot().await.is_none());

        restore(&pool).await;
        let (remaining, streak) = gmail::block_snapshot().await.expect("block back in force");
        assert!(remaining > std::time::Duration::from_secs(880));
        assert_eq!(streak, 3);

        gmail::clear_block_for_test().await;
        persist(&pool).await;
        assert_eq!(
            read_setting(&pool, GMAIL_BLOCK_UNTIL).await.as_deref(),
            Some(""),
            "a lapsed block is cleared, not left to be restored again"
        );
    }
}
