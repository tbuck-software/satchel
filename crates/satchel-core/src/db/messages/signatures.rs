//! Message reads used by signature selection and discovery.

use sqlx::{FromRow, SqlitePool};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct SignatureHistoryMessage {
    pub thread_id: String,
    pub body_text: String,
    pub body_html: Option<String>,
}

#[derive(Debug, Error)]
pub enum SignatureMessageError {
    #[error("signature message history could not be loaded")]
    Persistence(#[source] sqlx::Error),
}

impl From<sqlx::Error> for SignatureMessageError {
    fn from(error: sqlx::Error) -> Self {
        Self::Persistence(error)
    }
}

/// Owns sent-message reads needed by the signature aggregate. Signature tables
/// remain unknown to this repository.
pub struct SignatureMessageRepository<'a> {
    pool: &'a SqlitePool,
}

impl<'a> SignatureMessageRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn sent_from_identity_in_thread(
        &self,
        account_id: &str,
        identity_email: &str,
        thread_id: Option<&str>,
    ) -> Result<bool, SignatureMessageError> {
        let Some(thread_id) = thread_id.map(str::trim).filter(|id| !id.is_empty()) else {
            return Ok(false);
        };
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(1) FROM messages
             WHERE account_id = ? AND thread_id = ? AND is_sent = 1
               AND lower(trim(from_address)) = ?",
        )
        .bind(account_id)
        .bind(thread_id)
        .bind(identity_email)
        .fetch_one(self.pool)
        .await?;
        Ok(count > 0)
    }

    pub async fn recent_sent_from_identity(
        &self,
        account_id: &str,
        identity_email: &str,
        limit: i64,
    ) -> Result<Vec<SignatureHistoryMessage>, SignatureMessageError> {
        sqlx::query_as::<_, SignatureHistoryMessage>(
            "SELECT COALESCE(thread_id, id) AS thread_id,
                    COALESCE(body_text, '') AS body_text, body_html
             FROM messages
             WHERE account_id = ? AND is_sent = 1 AND lower(trim(from_address)) = ?
             ORDER BY date DESC LIMIT ?",
        )
        .bind(account_id)
        .bind(identity_email)
        .bind(limit.max(0))
        .fetch_all(self.pool)
        .await
        .map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use sqlx::sqlite::SqlitePoolOptions;

    use super::*;

    #[tokio::test]
    async fn reads_are_account_and_identity_scoped_and_newest_first() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::db::init(&pool).await.unwrap();
        for (account_id, email) in [("a", "a@example.test"), ("b", "b@example.test")] {
            sqlx::query(
                "INSERT INTO accounts
                 (id, display_name, email, provider, config, credentials_ref, sync_state,
                  color, is_active, created_at, updated_at)
                 VALUES (?, 'Account', ?, 'imap', '{}', '', '{}', '#000', 1, 'now', 'now')",
            )
            .bind(account_id)
            .bind(email)
            .execute(&pool)
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO folders
                 (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at)
                 VALUES (?, ?, 'Sent', 'Sent', 'sent', 'Sent', 'now', 'now')",
            )
            .bind(format!("folder-{account_id}"))
            .bind(account_id)
            .execute(&pool)
            .await
            .unwrap();
        }
        for (id, account_id, thread_id, from, date) in [
            ("old", "a", "thread", " A@Example.Test ", "2026-01-01"),
            ("new", "a", "thread", "a@example.test", "2026-02-01"),
            ("other", "b", "thread", "a@example.test", "2026-03-01"),
        ] {
            sqlx::query(
                "INSERT INTO messages
                 (id, account_id, folder_id, thread_id, from_address, to_addresses, date, body_text,
                  is_sent, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, '[]', ?, ?, 1, 'now', 'now')",
            )
            .bind(id)
            .bind(account_id)
            .bind(format!("folder-{account_id}"))
            .bind(thread_id)
            .bind(from)
            .bind(date)
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
        }
        let repository = SignatureMessageRepository::new(&pool);
        assert!(repository
            .sent_from_identity_in_thread("a", "a@example.test", Some("thread"))
            .await
            .unwrap());
        assert!(!repository
            .sent_from_identity_in_thread("b", "b@example.test", Some("thread"))
            .await
            .unwrap());
        let rows = repository
            .recent_sent_from_identity("a", "a@example.test", 200)
            .await
            .unwrap();
        assert_eq!(
            rows.iter()
                .map(|row| row.body_text.as_str())
                .collect::<Vec<_>>(),
            ["new", "old"]
        );
    }
}
