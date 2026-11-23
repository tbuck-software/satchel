//! Provider rate-limit blocks shared across app restarts and Android processes.

use chrono::{DateTime, Utc};
use sqlx::{SqliteConnection, SqlitePool};

use crate::providers::gmail;

pub const GMAIL_BLOCK_UNTIL_PREFIX: &str = "gmail_rate_limit_block_until.";
pub const GMAIL_BLOCK_STREAK_PREFIX: &str = "gmail_rate_limit_block_streak.";

pub async fn clear_legacy(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    // The old rows have no account attribution. Discard them at startup
    // so they do not throttle an unrelated account.
    sqlx::query(
        "DELETE FROM app_settings WHERE key IN (
            'gmail_rate_limit_block_until', 'gmail_rate_limit_block_streak'
        )",
    )
    .execute(pool)
    .await?;
    Ok(())
}

async fn stored_block(
    db: &mut SqliteConnection,
    account_id: &str,
) -> Result<(Option<String>, u32), sqlx::Error> {
    let (until, streak): (Option<String>, Option<String>) = sqlx::query_as(
        "SELECT (SELECT value FROM app_settings WHERE key = ?),
                (SELECT value FROM app_settings WHERE key = ?)",
    )
    .bind(format!("{GMAIL_BLOCK_UNTIL_PREFIX}{account_id}"))
    .bind(format!("{GMAIL_BLOCK_STREAK_PREFIX}{account_id}"))
    .fetch_one(db)
    .await?;
    let streak = streak
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(1);
    Ok((until, streak))
}

fn parse_until(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value.trim())
        .ok()
        .map(|until| until.to_utc())
}

/// Merge an active block before issuing provider requests.
pub async fn restore(pool: &SqlitePool, account_id: &str) -> Result<(), sqlx::Error> {
    let mut db = pool.acquire().await?;
    let (until, streak) = stored_block(&mut db, account_id).await?;
    drop(db);
    if let Some(remaining) = until
        .as_deref()
        .and_then(parse_until)
        .and_then(|until| (until - Utc::now()).to_std().ok())
    {
        gmail::restore_block(account_id, remaining, streak);
    }
    Ok(())
}

/// Persist local state without shortening or clearing another process's active block.
pub async fn persist(pool: &SqlitePool, account_id: &str) {
    let sampled_at = Utc::now();
    let local = gmail::block_snapshot(account_id).and_then(|(remaining, streak)| {
        let until = sampled_at.checked_add_signed(chrono::Duration::from_std(remaining).ok()?)?;
        Some((until, streak))
    });
    if let Err(error) = persist_snapshot(pool, account_id, local).await {
        tracing::warn!(%account_id, %error, "could not persist Gmail rate limit");
    }
}

async fn persist_snapshot(
    pool: &SqlitePool,
    account_id: &str,
    local: Option<(DateTime<Utc>, u32)>,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let (stored_until, stored_streak) = stored_block(&mut tx, account_id).await?;
    let now = Utc::now();
    let active = stored_until
        .as_deref()
        .and_then(parse_until)
        .filter(|until| *until > now);
    let local = local.filter(|(until, _)| *until > now);
    let (until, streak) = match (local, active) {
        (Some((until, streak)), Some(stored)) => {
            (until.max(stored).to_rfc3339(), streak.max(stored_streak))
        }
        (Some((until, streak)), None) => (until.to_rfc3339(), streak),
        (None, _) => return tx.commit().await,
    };
    if stored_until.as_deref() != Some(&until) || stored_streak != streak {
        sqlx::query(
            "INSERT INTO app_settings (key, value, updated_at) VALUES (?, ?, ?), (?, ?, ?)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        )
        .bind(format!("{GMAIL_BLOCK_UNTIL_PREFIX}{account_id}"))
        .bind(until)
        .bind(now.to_rfc3339())
        .bind(format!("{GMAIL_BLOCK_STREAK_PREFIX}{account_id}"))
        .bind(streak.to_string())
        .bind(now.to_rfc3339())
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;
    use std::time::Duration;

    async fn pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
        pool
    }

    #[tokio::test]
    async fn one_accounts_stored_block_survives_restart_without_throttling_others() {
        let pool = pool().await;
        let now = Utc::now();
        let deadline = now + chrono::Duration::minutes(15);
        persist_snapshot(&pool, "limited", Some((deadline, 3)))
            .await
            .unwrap();
        persist_snapshot(
            &pool,
            "limited",
            Some((now + chrono::Duration::minutes(5), 1)),
        )
        .await
        .unwrap();
        persist_snapshot(&pool, "limited", None).await.unwrap();

        restore(&pool, "limited").await.unwrap();
        restore(&pool, "unlimited").await.unwrap();
        gmail::restore_block("limited", Duration::from_secs(30), 1);
        let (remaining, streak) = gmail::block_snapshot("limited").unwrap();
        assert!(remaining > Duration::from_secs(880));
        assert_eq!(streak, 3);
        assert!(gmail::block_snapshot("unlimited").is_none());
    }

    #[tokio::test]
    async fn legacy_and_expired_blocks_do_not_restore() {
        let pool = pool().await;
        sqlx::query(
            "INSERT INTO app_settings (key, value, updated_at) VALUES
                     ('gmail_rate_limit_block_until', '2099-01-01T00:00:00Z', ''),
                     ('gmail_rate_limit_block_streak', '5', ''),
                     (?, '2000-01-01T00:00:00Z', '')",
        )
        .bind(format!("{GMAIL_BLOCK_UNTIL_PREFIX}expired"))
        .execute(&pool)
        .await
        .unwrap();
        restore(&pool, "expired").await.unwrap();
        assert!(gmail::block_snapshot("expired").is_none());

        clear_legacy(&pool).await.unwrap();
        let keys: Vec<String> = sqlx::query_scalar(
            "SELECT key FROM app_settings WHERE key LIKE 'gmail_rate_limit_block_%'",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(keys, [format!("{GMAIL_BLOCK_UNTIL_PREFIX}expired")]);
    }
}
