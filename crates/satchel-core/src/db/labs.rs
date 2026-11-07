//! Labs: per-device switches for experimental features.
//!
//! The registry is a fixed list. An id it does not name is refused rather than
//! stored, so a typo in the frontend cannot leave a row that nothing reads. A
//! flag without a row is off: every Labs feature starts off.
use sqlx::SqlitePool;
use thiserror::Error;

/// Delta Chat–compatible chats (`chat`, `autocrypt`).
pub const CHATS: &str = "chats";

/// Every flag the backend knows. Keep in step with `src/labs/flags.ts`.
pub const LABS_FLAGS: &[&str] = &[CHATS];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabsFlagRecord {
    pub id: String,
    pub enabled: bool,
}

#[derive(Debug, Error)]
pub enum LabsError {
    #[error("unknown labs flag")]
    UnknownFlag,
    #[error("labs persistence failed: {0}")]
    Persistence(#[from] sqlx::Error),
}

pub struct LabsRepository<'a> {
    pool: &'a SqlitePool,
}

impl<'a> LabsRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    /// Every known flag with its current state, in registry order.
    pub async fn list(&self) -> Result<Vec<LabsFlagRecord>, LabsError> {
        let rows: Vec<(String, i64)> = sqlx::query_as("SELECT id, enabled FROM labs_flags")
            .fetch_all(self.pool)
            .await?;
        Ok(LABS_FLAGS
            .iter()
            .map(|id| LabsFlagRecord {
                id: (*id).to_string(),
                enabled: rows
                    .iter()
                    .any(|(row_id, enabled)| row_id == id && *enabled != 0),
            })
            .collect())
    }

    pub async fn set(&self, id: &str, enabled: bool) -> Result<LabsFlagRecord, LabsError> {
        let id = known(id)?;
        sqlx::query(
            "INSERT INTO labs_flags (id, enabled, updated_at)
             VALUES (?, ?, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
             ON CONFLICT(id) DO UPDATE SET enabled = excluded.enabled, updated_at = excluded.updated_at",
        )
        .bind(id)
        .bind(i64::from(enabled))
        .execute(self.pool)
        .await?;
        Ok(LabsFlagRecord {
            id: id.to_string(),
            enabled,
        })
    }

    pub async fn is_enabled(&self, id: &str) -> Result<bool, LabsError> {
        let id = known(id)?;
        Ok(is_enabled_in(self.pool, id).await?)
    }
}

/// Whether a flag is on, on any executor (a transaction during ingest, too).
pub async fn is_enabled_in<'e, E>(executor: E, id: &str) -> Result<bool, sqlx::Error>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    let enabled: Option<i64> = sqlx::query_scalar("SELECT enabled FROM labs_flags WHERE id = ?")
        .bind(id)
        .fetch_optional(executor)
        .await?;
    Ok(enabled.unwrap_or(0) != 0)
}

fn known(id: &str) -> Result<&'static str, LabsError> {
    LABS_FLAGS
        .iter()
        .copied()
        .find(|known| *known == id.trim())
        .ok_or(LabsError::UnknownFlag)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn pool() -> SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::db::init(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn every_flag_starts_off_and_remembers_a_switch() {
        let pool = pool().await;
        let repo = LabsRepository::new(&pool);
        assert_eq!(
            repo.list().await.unwrap(),
            vec![LabsFlagRecord {
                id: CHATS.into(),
                enabled: false
            }]
        );

        repo.set(CHATS, true).await.unwrap();
        assert!(repo.is_enabled(CHATS).await.unwrap());
        repo.set(CHATS, false).await.unwrap();
        assert!(!repo.list().await.unwrap()[0].enabled);
    }

    #[tokio::test]
    async fn an_unknown_flag_is_refused_and_not_stored() {
        let pool = pool().await;
        let repo = LabsRepository::new(&pool);
        assert!(matches!(
            repo.set("teleport", true).await,
            Err(LabsError::UnknownFlag)
        ));
        let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM labs_flags")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(rows, 0);
    }
}
