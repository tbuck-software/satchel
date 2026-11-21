//! Provider rate-limit blocks shared across app restarts and Android processes.

use chrono::{DateTime, Utc};
use sqlx::{SqliteConnection, SqlitePool};

use crate::providers::gmail;

const GMAIL_BLOCK_UNTIL: &str = "gmail_rate_limit_block_until";
const GMAIL_BLOCK_STREAK: &str = "gmail_rate_limit_block_streak";

async fn stored_block(db: &mut SqliteConnection) -> Result<(Option<String>, u32), sqlx::Error> {
    let (until, streak): (Option<String>, Option<String>) = sqlx::query_as(
        "SELECT (SELECT value FROM app_settings WHERE key = ?),
                (SELECT value FROM app_settings WHERE key = ?)",
    )
    .bind(GMAIL_BLOCK_UNTIL)
    .bind(GMAIL_BLOCK_STREAK)
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
pub async fn restore(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    let mut db = pool.acquire().await?;
    let (until, streak) = stored_block(&mut db).await?;
    drop(db);
    if let Some(remaining) = until
        .as_deref()
        .and_then(parse_until)
        .and_then(|until| (until - Utc::now()).to_std().ok())
    {
        gmail::restore_block(remaining, streak).await;
    }
    Ok(())
}

/// Persist local state without shortening or clearing another process's active block.
pub async fn persist(pool: &SqlitePool) {
    let sampled_at = Utc::now();
    let local = gmail::block_snapshot()
        .await
        .and_then(|(remaining, streak)| {
            let until =
                sampled_at.checked_add_signed(chrono::Duration::from_std(remaining).ok()?)?;
            Some((until, streak))
        });
    if let Err(error) = persist_snapshot(pool, local).await {
        tracing::warn!(%error, "could not persist Gmail rate limit");
    }
}

async fn persist_snapshot(
    pool: &SqlitePool,
    local: Option<(DateTime<Utc>, u32)>,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let (stored_until, stored_streak) = stored_block(&mut tx).await?;
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
        (None, None)
            if stored_until
                .as_deref()
                .is_some_and(|until| !until.is_empty()) =>
        {
            (String::new(), 0)
        }
        (None, _) => return tx.commit().await,
    };
    if stored_until.as_deref() != Some(&until) || stored_streak != streak {
        sqlx::query(
            "INSERT INTO app_settings (key, value, updated_at) VALUES (?, ?, ?), (?, ?, ?)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        )
        .bind(GMAIL_BLOCK_UNTIL)
        .bind(until)
        .bind(now.to_rfc3339())
        .bind(GMAIL_BLOCK_STREAK)
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
    async fn a_block_survives_a_restart_and_older_snapshots_cannot_shorten_it() {
        let pool = pool().await;
        let now = Utc::now();
        let deadline = now + chrono::Duration::minutes(15);
        persist_snapshot(&pool, Some((deadline, 3))).await.unwrap();
        persist_snapshot(&pool, Some((now + chrono::Duration::minutes(5), 1)))
            .await
            .unwrap();
        persist_snapshot(&pool, None).await.unwrap();

        gmail::clear_block_for_test().await;
        restore(&pool).await.unwrap();
        gmail::restore_block(Duration::from_secs(30), 1).await;
        let (remaining, streak) = gmail::block_snapshot().await.unwrap();
        assert!(remaining > Duration::from_secs(880));
        assert_eq!(streak, 3);
        assert!(gmail::as_background(gmail::await_quota(
            1.0,
            gmail::Priority::Background,
            "labels.list"
        ))
        .await
        .unwrap_err()
        .is_rate_limited());
        gmail::clear_block_for_test().await;
    }

    #[tokio::test]
    async fn an_expired_block_can_be_cleared() {
        let pool = pool().await;
        sqlx::query("INSERT INTO app_settings (key, value, updated_at) VALUES (?, ?, '')")
            .bind(GMAIL_BLOCK_UNTIL)
            .bind("2000-01-01T00:00:00Z")
            .execute(&pool)
            .await
            .unwrap();
        persist_snapshot(&pool, None).await.unwrap();
        let mut db = pool.acquire().await.unwrap();
        assert_eq!(
            stored_block(&mut db).await.unwrap(),
            (Some(String::new()), 0)
        );
    }
}
