//! Durable record of provider messages the ingest could not store.
//!
//! A message that fails deterministically used to stall its account forever:
//! the folder cursor never moved past it, so every run refetched it and threw
//! the whole batch away. Quarantine turns that into a bounded number of
//! retries, after which the cursor may step over the message. The row keeps it
//! identifiable — nothing local holds it, so it must never count as delivered
//! (POP3 deletion in particular).

use std::collections::HashMap;

use sqlx::SqlitePool;
use thiserror::Error;

/// How often one message is retried before the cursor is allowed past it.
///
/// Three runs separate a transient failure (busy database, truncated provider
/// response) from a message this build simply cannot store.
pub const MAX_INGEST_ATTEMPTS: i64 = 3;

#[derive(Debug, Error)]
pub enum SyncQuarantineError {
    #[error("sync quarantine persistence failed: {0}")]
    Persistence(#[from] sqlx::Error),
}

/// Quarantine state of one folder, read once per batch.
///
/// The ingest loop asks about every message it is handed, so the answer has to
/// come from memory rather than from a query per message.
#[derive(Debug, Default)]
pub struct FolderQuarantine {
    attempts: HashMap<String, i64>,
}

impl FolderQuarantine {
    /// Whether this message has used up its attempts and should be skipped.
    pub fn is_exhausted(&self, remote_id: &str) -> bool {
        self.attempts
            .get(remote_id)
            .is_some_and(|attempts| *attempts >= MAX_INGEST_ATTEMPTS)
    }

    /// Whether a failure is on record for this message at all.
    pub fn contains(&self, remote_id: &str) -> bool {
        self.attempts.contains_key(remote_id)
    }
}

/// Owns the quarantine rows for failed message ingests.
pub struct SyncQuarantineRepository<'a> {
    pool: &'a SqlitePool,
}

impl<'a> SyncQuarantineRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn load_folder(
        &self,
        account_id: &str,
        folder_id: &str,
    ) -> Result<FolderQuarantine, SyncQuarantineError> {
        let rows = sqlx::query_as::<_, (String, i64)>(
            "SELECT remote_id, attempts FROM sync_quarantine
             WHERE account_id = ? AND folder_id = ?",
        )
        .bind(account_id)
        .bind(folder_id)
        .fetch_all(self.pool)
        .await?;
        Ok(FolderQuarantine {
            attempts: rows.into_iter().collect(),
        })
    }

    /// Count one failed ingest and return how often the message has failed.
    pub async fn record_failure(
        &self,
        account_id: &str,
        folder_id: &str,
        remote_id: &str,
        error: &str,
    ) -> Result<i64, SyncQuarantineError> {
        let now = chrono::Utc::now().to_rfc3339();
        let attempts = sqlx::query_scalar::<_, i64>(
            "INSERT INTO sync_quarantine
                (account_id, folder_id, remote_id, attempts, last_error, first_failed_at, last_failed_at)
             VALUES (?, ?, ?, 1, ?, ?, ?)
             ON CONFLICT(account_id, folder_id, remote_id) DO UPDATE SET
                attempts = sync_quarantine.attempts + 1,
                last_error = excluded.last_error,
                last_failed_at = excluded.last_failed_at
             RETURNING attempts",
        )
        .bind(account_id)
        .bind(folder_id)
        .bind(remote_id)
        .bind(error)
        .bind(&now)
        .bind(&now)
        .fetch_one(self.pool)
        .await?;
        Ok(attempts)
    }

    /// A new IMAP UIDVALIDITY invalidates failures tied to the old UIDs.
    pub async fn clear_folder(
        &self,
        account_id: &str,
        folder_id: &str,
    ) -> Result<(), SyncQuarantineError> {
        sqlx::query("DELETE FROM sync_quarantine WHERE account_id = ? AND folder_id = ?")
            .bind(account_id)
            .bind(folder_id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    /// Give one more attempt to every exhausted entry of an account whose last
    /// failure is older than `failed_before`. Returns how many were rearmed.
    ///
    /// The row keeps its history: a retry that fails again exhausts it at once.
    pub async fn rearm_exhausted(
        &self,
        account_id: &str,
        failed_before: chrono::DateTime<chrono::Utc>,
    ) -> Result<u64, SyncQuarantineError> {
        let rearmed = sqlx::query(
            "UPDATE sync_quarantine SET attempts = ?
             WHERE account_id = ? AND attempts >= ?
               AND julianday(last_failed_at) < julianday(?)",
        )
        .bind(MAX_INGEST_ATTEMPTS - 1)
        .bind(account_id)
        .bind(MAX_INGEST_ATTEMPTS)
        .bind(failed_before.to_rfc3339())
        .execute(self.pool)
        .await?
        .rows_affected();
        Ok(rearmed)
    }

    /// Forget a message that has been stored after all.
    pub async fn clear(
        &self,
        account_id: &str,
        folder_id: &str,
        remote_id: &str,
    ) -> Result<(), SyncQuarantineError> {
        sqlx::query(
            "DELETE FROM sync_quarantine
             WHERE account_id = ? AND folder_id = ? AND remote_id = ?",
        )
        .bind(account_id)
        .bind(folder_id)
        .bind(remote_id)
        .execute(self.pool)
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{SyncQuarantineRepository, MAX_INGEST_ATTEMPTS};
    use sqlx::sqlite::SqlitePoolOptions;

    async fn quarantine_pool() -> sqlx::SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("pool");
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .expect("migrations");
        sqlx::query(
            "INSERT INTO accounts
             (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
             VALUES ('account', 'Account', 'account@example.com', 'imap', '{}', '', '', '')",
        )
        .execute(&pool)
        .await
        .expect("account");
        pool
    }

    #[tokio::test]
    async fn attempts_accumulate_until_the_message_counts_as_exhausted() {
        let pool = quarantine_pool().await;
        let repository = SyncQuarantineRepository::new(&pool);

        for expected in 1..=MAX_INGEST_ATTEMPTS {
            let attempts = repository
                .record_failure("account", "inbox", "42", "broken")
                .await
                .expect("record");
            assert_eq!(attempts, expected);
            let folder = repository
                .load_folder("account", "inbox")
                .await
                .expect("load");
            assert!(folder.contains("42"));
            assert_eq!(folder.is_exhausted("42"), expected >= MAX_INGEST_ATTEMPTS);
        }

        repository
            .clear("account", "inbox", "42")
            .await
            .expect("clear");
        assert!(!repository
            .load_folder("account", "inbox")
            .await
            .expect("load")
            .contains("42"));
    }

    #[tokio::test]
    async fn only_long_exhausted_entries_of_the_account_are_rearmed_once() {
        let pool = quarantine_pool().await;
        sqlx::query(
            "INSERT INTO accounts
             (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
             VALUES ('other', 'Other', 'other@example.com', 'imap', '{}', '', '', '')",
        )
        .execute(&pool)
        .await
        .expect("other account");
        let repository = SyncQuarantineRepository::new(&pool);
        for (account, remote_id, attempts) in [
            ("account", "old", MAX_INGEST_ATTEMPTS),
            ("account", "retrying", MAX_INGEST_ATTEMPTS - 1),
            ("account", "recent", MAX_INGEST_ATTEMPTS),
            ("other", "old", MAX_INGEST_ATTEMPTS),
        ] {
            for _ in 0..attempts {
                repository
                    .record_failure(account, "inbox", remote_id, "broken")
                    .await
                    .expect("record");
            }
        }
        let day_ago = (chrono::Utc::now() - chrono::Duration::hours(25)).to_rfc3339();
        sqlx::query("UPDATE sync_quarantine SET last_failed_at = ? WHERE remote_id != 'recent'")
            .bind(day_ago)
            .execute(&pool)
            .await
            .expect("age entries");
        let cutoff = chrono::Utc::now() - chrono::Duration::hours(24);

        assert_eq!(
            repository
                .rearm_exhausted("account", cutoff)
                .await
                .expect("rearm"),
            1
        );
        let account = repository
            .load_folder("account", "inbox")
            .await
            .expect("load");
        assert!(!account.is_exhausted("old"));
        assert!(account.is_exhausted("recent"));
        assert!(repository
            .load_folder("other", "inbox")
            .await
            .expect("load other")
            .is_exhausted("old"));
        // A failed retry exhausts it again and restarts its wait.
        assert_eq!(
            repository
                .record_failure("account", "inbox", "old", "broken")
                .await
                .expect("record"),
            MAX_INGEST_ATTEMPTS
        );
        assert_eq!(
            repository
                .rearm_exhausted("account", cutoff)
                .await
                .expect("rearm again"),
            0
        );
    }

    #[tokio::test]
    async fn a_generation_reset_clears_only_its_folder() {
        let pool = quarantine_pool().await;
        let repository = SyncQuarantineRepository::new(&pool);
        for folder in ["inbox", "archive"] {
            repository
                .record_failure("account", folder, "42", "busy")
                .await
                .unwrap();
        }
        repository.clear_folder("account", "inbox").await.unwrap();
        assert!(!repository
            .load_folder("account", "inbox")
            .await
            .unwrap()
            .contains("42"));
        assert!(repository
            .load_folder("account", "archive")
            .await
            .unwrap()
            .contains("42"));
    }

    #[tokio::test]
    async fn quarantine_is_scoped_to_its_folder() {
        let pool = quarantine_pool().await;
        let repository = SyncQuarantineRepository::new(&pool);
        repository
            .record_failure("account", "inbox", "42", "broken")
            .await
            .expect("record");

        assert!(!repository
            .load_folder("account", "archive")
            .await
            .expect("load")
            .contains("42"));
    }
}
