//! Database maintenance decisions, independent of the app's worker schedule.

use sqlx::SqlitePool;
use std::time::Duration;

pub const CHECKPOINT_INTERVAL: Duration = Duration::from_secs(60 * 60);
pub const CHECKPOINT_RETRY: Duration = Duration::from_secs(60);
pub const FTS_CHECK_PENDING: &str = "maintenance.fts_check_pending";

#[derive(Debug, PartialEq, Eq)]
pub struct Checkpoint {
    pub busy: i64,
    pub log: i64,
    pub checkpointed: i64,
}

impl Checkpoint {
    pub fn complete(&self) -> bool {
        self.busy == 0
    }
}

pub async fn checkpoint(pool: &SqlitePool) -> Result<Checkpoint, sqlx::Error> {
    let (busy, log, checkpointed) =
        sqlx::query_as::<_, (i64, i64, i64)>("PRAGMA wal_checkpoint(TRUNCATE)")
            .fetch_one(pool)
            .await?;
    Ok(Checkpoint {
        busy,
        log,
        checkpointed,
    })
}

pub fn checkpoint_delay(result: &Result<Checkpoint, sqlx::Error>) -> Duration {
    if result.as_ref().is_ok_and(Checkpoint::complete) {
        CHECKPOINT_INTERVAL
    } else {
        CHECKPOINT_RETRY
    }
}

/// A full rewrite is worthwhile only when more than one fifth is unused.
pub async fn needs_vacuum(pool: &SqlitePool) -> Result<bool, sqlx::Error> {
    let free: i64 = sqlx::query_scalar("PRAGMA freelist_count")
        .fetch_one(pool)
        .await?;
    let pages: i64 = sqlx::query_scalar("PRAGMA page_count")
        .fetch_one(pool)
        .await?;
    Ok(pages > 0 && free > pages / 5)
}

pub async fn request_fts_check(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO app_settings (key, value, updated_at) VALUES (?, ?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at")
        .bind(FTS_CHECK_PENDING)
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(chrono::Utc::now().to_rfc3339())
        .execute(pool).await?;
    Ok(())
}
