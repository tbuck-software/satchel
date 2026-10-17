//! Tauri-free persistence for account send-as aliases.

use sqlx::{FromRow, SqlitePool};
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct AliasUpsertRequest {
    pub id: Option<String>,
    pub account_id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub is_default: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AliasRecord {
    pub id: String,
    pub account_id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub is_default: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Error)]
pub enum AliasError {
    #[error("invalid alias email")]
    InvalidEmail,
    #[error("alias persistence failed: {0}")]
    Persistence(#[source] sqlx::Error),
}

impl From<sqlx::Error> for AliasError {
    fn from(error: sqlx::Error) -> Self {
        Self::Persistence(error)
    }
}

#[derive(Debug, FromRow)]
struct StoredAlias {
    id: String,
    account_id: String,
    email: String,
    display_name: Option<String>,
    is_default: i64,
    created_at: String,
    updated_at: String,
}

impl From<StoredAlias> for AliasRecord {
    fn from(row: StoredAlias) -> Self {
        Self {
            id: row.id,
            account_id: row.account_id,
            email: row.email,
            display_name: row.display_name,
            is_default: row.is_default,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

const ALIAS_SELECT: &str =
    "SELECT id, account_id, email, display_name, is_default, created_at, updated_at
     FROM account_aliases";

/// Owns durable aliases while leaving serialization and Tauri command concerns
/// to the app crate.
pub struct AliasRepository<'a> {
    pool: &'a SqlitePool,
}

impl<'a> AliasRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn list(&self, account_id: Option<&str>) -> Result<Vec<AliasRecord>, AliasError> {
        let rows = match account_id {
            Some(account_id) => {
                sqlx::query_as::<_, StoredAlias>(&format!(
                    "{ALIAS_SELECT} WHERE account_id = ? ORDER BY is_default DESC, email ASC"
                ))
                .bind(account_id)
                .fetch_all(self.pool)
                .await?
            }
            None => {
                sqlx::query_as::<_, StoredAlias>(&format!(
                    "{ALIAS_SELECT} ORDER BY account_id, is_default DESC, email ASC"
                ))
                .fetch_all(self.pool)
                .await?
            }
        };
        Ok(rows.into_iter().map(Into::into).collect())
    }

    pub async fn upsert(&self, input: AliasUpsertRequest) -> Result<AliasRecord, AliasError> {
        let email = input.email.trim().to_lowercase();
        if email.is_empty() || !email.contains('@') {
            return Err(AliasError::InvalidEmail);
        }

        let id = input
            .id
            .unwrap_or_else(|| format!("alias-{}", uuid::Uuid::new_v4()));
        let now = chrono::Utc::now().to_rfc3339();
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;

        if input.is_default {
            sqlx::query(
                "UPDATE account_aliases
                 SET is_default = 0, updated_at = ?
                 WHERE account_id = ?",
            )
            .bind(&now)
            .bind(&input.account_id)
            .execute(&mut *transaction)
            .await?;
        }

        let row = sqlx::query_as::<_, StoredAlias>(
            "INSERT INTO account_aliases
                (id, account_id, email, display_name, is_default, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
                email = excluded.email,
                display_name = excluded.display_name,
                is_default = excluded.is_default,
                updated_at = excluded.updated_at
             RETURNING id, account_id, email, display_name, is_default, created_at, updated_at",
        )
        .bind(&id)
        .bind(&input.account_id)
        .bind(&email)
        .bind(input.display_name.as_deref())
        .bind(if input.is_default { 1 } else { 0 })
        .bind(&now)
        .bind(&now)
        .fetch_one(&mut *transaction)
        .await?;

        transaction.commit().await?;
        Ok(row.into())
    }

    pub async fn delete(&self, id: &str) -> Result<(), AliasError> {
        sqlx::query("DELETE FROM account_aliases WHERE id = ?")
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    pub async fn contains(&self, account_id: &str, email: &str) -> Result<bool, AliasError> {
        let exists: i64 = sqlx::query_scalar(
            "SELECT COUNT(1)
             FROM account_aliases
             WHERE account_id = ? AND LOWER(TRIM(email)) = LOWER(TRIM(?))",
        )
        .bind(account_id)
        .bind(email)
        .fetch_one(self.pool)
        .await?;
        Ok(exists > 0)
    }
}
