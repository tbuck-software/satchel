//! Saved identity for on-demand content reads and their eventual writeback.
use sqlx::{FromRow, SqlitePool};

use crate::providers::traits::MessageRef;

pub fn saved_uidvalidity(value: Option<i64>) -> Option<u32> {
    value
        .and_then(|value| u32::try_from(value).ok())
        .filter(|value| *value != 0)
}

#[derive(Debug, Clone, FromRow, PartialEq, Eq)]
pub struct ContentReference {
    pub message_id: String,
    pub account_id: String,
    pub folder_id: Option<String>,
    pub remote_id: Option<String>,
    pub remote_uidvalidity: Option<i64>,
    pub created_at: String,
    pub provider: String,
    pub source_folder: Option<String>,
    pub folder_uidvalidity: Option<i64>,
}

impl ContentReference {
    pub async fn load(pool: &SqlitePool, message_id: &str) -> Result<Self, String> {
        sqlx::query_as(
            "SELECT m.id AS message_id, m.account_id, m.folder_id, m.remote_id,
                    m.remote_uidvalidity, m.created_at, a.provider,
                    COALESCE(f.remote_id, f.full_path) AS source_folder,
                    f.uidvalidity AS folder_uidvalidity
             FROM messages m JOIN accounts a ON a.id = m.account_id
             LEFT JOIN folders f ON f.id = m.folder_id WHERE m.id = ?",
        )
        .bind(message_id)
        .fetch_optional(pool)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "backend.errors.messageNotAvailableLocally".to_string())
    }

    pub fn uidvalidity(&self) -> Option<u32> {
        saved_uidvalidity(self.remote_uidvalidity)
    }

    pub fn message_ref(&self) -> Result<MessageRef, String> {
        if self.provider == "imap" {
            if self.uidvalidity().is_none() {
                return Err(self.error("backend.errors.imapUidValidityMissing"));
            }
            if self.remote_uidvalidity != self.folder_uidvalidity {
                return Err(self.changed_error());
            }
        }
        Ok(MessageRef {
            remote_id: self
                .remote_id
                .clone()
                .filter(|id| !id.is_empty())
                .ok_or_else(|| "backend.errors.messageNotAvailableRemotely".to_string())?,
            source_folder: self.source_folder.clone(),
            source_uidvalidity: self.uidvalidity(),
        })
    }

    pub async fn begin_write<'a>(
        &self,
        pool: &'a SqlitePool,
    ) -> Result<sqlx::Transaction<'a, sqlx::Sqlite>, String> {
        let mut tx = pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(|error| error.to_string())?;
        self.ensure_current(&mut tx).await?;
        Ok(tx)
    }

    /// Call inside the write transaction, before publishing fetched content.
    /// `begin_write` holds the writer lock from this check through publication.
    pub async fn ensure_current(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    ) -> Result<(), String> {
        self.validate_cached_read()?;
        let current: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM messages m LEFT JOIN folders f ON f.id = m.folder_id
             WHERE m.id = ? AND m.account_id = ? AND m.folder_id IS ?
               AND m.remote_id IS ? AND m.remote_uidvalidity IS ? AND m.created_at = ?
               AND (? != 'imap' OR (f.uidvalidity IS ?
                    AND COALESCE(f.remote_id, f.full_path) IS ?)))",
        )
        .bind(&self.message_id)
        .bind(&self.account_id)
        .bind(&self.folder_id)
        .bind(&self.remote_id)
        .bind(self.remote_uidvalidity)
        .bind(&self.created_at)
        .bind(&self.provider)
        .bind(self.folder_uidvalidity)
        .bind(&self.source_folder)
        .fetch_one(&mut **tx)
        .await
        .map_err(|error| error.to_string())?;
        if current {
            Ok(())
        } else {
            Err(self.changed_error())
        }
    }

    pub fn changed_error(&self) -> String {
        if self.provider == "imap" {
            self.error("backend.errors.imapUidValidityChanged")
        } else {
            "backend.errors.messageNotAvailableLocally".to_string()
        }
    }

    /// Local drafts have no remote identity. Remote IMAP content requires the
    /// saved generation even when its bytes already live in a cache.
    pub fn validate_cached_read(&self) -> Result<(), String> {
        if self.provider == "imap" && self.remote_id.as_deref().is_some_and(|id| !id.is_empty()) {
            self.message_ref()?;
        }
        Ok(())
    }

    fn error(&self, key: &str) -> String {
        serde_json::json!({"key": key, "params": {"folder": self.source_folder.as_deref().unwrap_or("INBOX")}}).to_string()
    }

    pub fn owns_attachment_cache(&self, path: &str, attachment_id: &str) -> bool {
        if self.provider != "imap" || self.remote_id.as_deref().is_none_or(str::is_empty) {
            return true;
        }
        use sha2::{Digest, Sha256};
        let Some(name) = std::path::Path::new(path)
            .file_name()
            .and_then(|name| name.to_str())
        else {
            return false;
        };
        let key = self.attachment_cache_id(attachment_id);
        [key.clone(), format!("{}-{key}", self.message_id)]
            .iter()
            .any(|key| {
                let prefix: String = Sha256::digest(key.as_bytes())
                    .iter()
                    .take(12)
                    .map(|byte| format!("{byte:02x}"))
                    .collect();
                name.starts_with(&format!("{prefix}-"))
            })
    }

    /// New cache files cannot collide with an earlier generation, including a
    /// legacy cache whose generation was never recorded.
    pub fn attachment_cache_id(&self, attachment_id: &str) -> String {
        if self.provider == "imap" && self.remote_id.as_deref().is_some_and(|id| !id.is_empty()) {
            format!(
                "{attachment_id}:imap:{}",
                self.remote_uidvalidity.unwrap_or(0)
            )
        } else {
            attachment_id.to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn fixture() -> SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        seed_fixture(&pool).await;
        pool
    }

    async fn seed_fixture(pool: &SqlitePool) {
        crate::db::init(pool).await.unwrap();
        sqlx::raw_sql("INSERT INTO accounts (id, email, display_name, provider, config, credentials_ref, created_at, updated_at)
                VALUES ('acc', 'me@example.test', 'Me', 'imap', '{}', '', '', '');
            INSERT INTO folders (id, account_id, name, full_path, remote_id, folder_type, uidvalidity, created_at, updated_at)
                VALUES ('inbox', 'acc', 'INBOX', 'INBOX', 'INBOX', 'inbox', 7, '', '');
            INSERT INTO threads (id, account_id, subject, last_message_date, created_at, updated_at)
                VALUES ('thread', 'acc', 'Subject', '', '', '');
            INSERT INTO messages (id, account_id, folder_id, thread_id, remote_id, remote_uidvalidity,
                from_address, to_addresses, date, body_text, created_at, updated_at)
                VALUES ('message', 'acc', 'inbox', 'thread', '42', 7, '', '[]', '', '', 'original', '');")
            .execute(pool).await.unwrap();
    }

    #[tokio::test]
    async fn content_reference_requires_the_envelopes_saved_generation() {
        let pool = fixture().await;
        let reference = ContentReference::load(&pool, "message").await.unwrap();
        assert_eq!(reference.message_ref().unwrap().source_uidvalidity, Some(7));
        for generation in [None, Some(0), Some(-1), Some(i64::from(u32::MAX) + 1)] {
            sqlx::query("UPDATE messages SET remote_uidvalidity = ?")
                .bind(generation)
                .execute(&pool)
                .await
                .unwrap();
            let unknown = ContentReference::load(&pool, "message").await.unwrap();
            assert!(unknown
                .message_ref()
                .unwrap_err()
                .contains("imapUidValidityMissing"));
            assert!(unknown
                .validate_cached_read()
                .unwrap_err()
                .contains("imapUidValidityMissing"));
        }
        sqlx::query("UPDATE messages SET remote_uidvalidity = 6")
            .execute(&pool)
            .await
            .unwrap();
        let stale = ContentReference::load(&pool, "message").await.unwrap();
        assert!(stale
            .message_ref()
            .unwrap_err()
            .contains("imapUidValidityChanged"));
        assert!(stale
            .validate_cached_read()
            .unwrap_err()
            .contains("imapUidValidityChanged"));
    }

    #[tokio::test]
    async fn late_content_writeback_cannot_poison_a_reingested_message() {
        let pool = fixture().await;
        let reference = ContentReference::load(&pool, "message").await.unwrap();
        let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
        reference.ensure_current(&mut tx).await.unwrap();
        tx.rollback().await.unwrap();
        // A reset may re-use even the local id, for example for an imported draft.
        sqlx::raw_sql(
            "UPDATE folders SET uidvalidity = 8;
            UPDATE messages SET remote_uidvalidity = 8, body_text = 'new generation';",
        )
        .execute(&pool)
        .await
        .unwrap();
        let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
        assert!(reference
            .ensure_current(&mut tx)
            .await
            .unwrap_err()
            .contains("imapUidValidityChanged"));
        tx.rollback().await.unwrap();
        let body: String = sqlx::query_scalar("SELECT body_text FROM messages")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(body, "new generation");
        let current = ContentReference::load(&pool, "message").await.unwrap();
        assert_ne!(
            reference.attachment_cache_id("part"),
            current.attachment_cache_id("part")
        );
        let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
        current.ensure_current(&mut tx).await.unwrap();
    }

    #[tokio::test]
    async fn content_publication_and_folder_reset_serialize_on_the_writer() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("generation.db");
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(2)
            .connect_with(
                sqlx::sqlite::SqliteConnectOptions::new()
                    .filename(&path)
                    .create_if_missing(true)
                    .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
                    .busy_timeout(std::time::Duration::from_secs(5)),
            )
            .await
            .unwrap();
        seed_fixture(&pool).await;
        let reference = ContentReference::load(&pool, "message").await.unwrap();
        let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
        let ready = barrier.clone();
        let writer = pool.clone();
        let publish = tokio::spawn(async move {
            ready.wait().await;
            match reference.begin_write(&writer).await {
                Ok(mut tx) => {
                    sqlx::query(
                        "UPDATE messages SET body_text = 'old fetched body' WHERE id = 'message'",
                    )
                    .execute(&mut *tx)
                    .await
                    .unwrap();
                    tx.commit().await.unwrap();
                }
                Err(error) => assert!(error.contains("imapUidValidityChanged")),
            }
        });
        barrier.wait().await;
        let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
        sqlx::raw_sql(
            "UPDATE folders SET uidvalidity = 8;
            UPDATE messages SET remote_uidvalidity = 8, body_text = 'replacement';",
        )
        .execute(&mut *tx)
        .await
        .unwrap();
        tx.commit().await.unwrap();
        publish.await.unwrap();
        let body: String = sqlx::query_scalar("SELECT body_text FROM messages")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(body, "replacement");
    }

    #[tokio::test]
    async fn moving_the_message_or_resetting_only_the_folder_invalidates_writeback() {
        let pool = fixture().await;
        let reference = ContentReference::load(&pool, "message").await.unwrap();
        sqlx::query("UPDATE folders SET uidvalidity = 8")
            .execute(&pool)
            .await
            .unwrap();
        let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
        assert!(reference.ensure_current(&mut tx).await.is_err());
        tx.rollback().await.unwrap();
        sqlx::raw_sql(
            "UPDATE folders SET uidvalidity = 7;
            UPDATE messages SET remote_id = '43';",
        )
        .execute(&pool)
        .await
        .unwrap();
        let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
        assert!(reference.ensure_current(&mut tx).await.is_err());
    }
}
