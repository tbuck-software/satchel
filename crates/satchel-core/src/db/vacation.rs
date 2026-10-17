//! Tauri-free persistence for per-account vacation settings.

use sqlx::{FromRow, SqlitePool};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VacationSettingsUpdate {
    pub account_id: String,
    pub enabled: bool,
    pub subject: Option<String>,
    pub body: String,
    pub starts_at: Option<String>,
    pub ends_at: Option<String>,
    pub only_contacts: bool,
}

/// The persisted representation intentionally keeps SQLite's integer flags.
/// The app command maps this directly to its existing JSON DTO shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VacationSettingsRecord {
    pub account_id: String,
    pub enabled: i64,
    pub subject: Option<String>,
    pub body: String,
    pub starts_at: Option<String>,
    pub ends_at: Option<String>,
    pub only_contacts: i64,
    pub updated_at: String,
}

#[derive(Debug, Error)]
pub enum VacationError {
    #[error("vacation settings persistence failed: {0}")]
    Persistence(#[source] sqlx::Error),
}

impl From<sqlx::Error> for VacationError {
    fn from(error: sqlx::Error) -> Self {
        Self::Persistence(error)
    }
}

#[derive(Debug, FromRow)]
struct StoredVacationSettings {
    account_id: String,
    enabled: i64,
    subject: Option<String>,
    body: String,
    starts_at: Option<String>,
    ends_at: Option<String>,
    only_contacts: i64,
    updated_at: String,
}

impl From<StoredVacationSettings> for VacationSettingsRecord {
    fn from(row: StoredVacationSettings) -> Self {
        Self {
            account_id: row.account_id,
            enabled: row.enabled,
            subject: row.subject,
            body: row.body,
            starts_at: row.starts_at,
            ends_at: row.ends_at,
            only_contacts: row.only_contacts,
            updated_at: row.updated_at,
        }
    }
}

const VACATION_SELECT: &str =
    "SELECT account_id, enabled, subject, body, starts_at, ends_at, only_contacts, updated_at
     FROM vacation_settings";

/// Owns the durable vacation-settings record while leaving provider APIs and
/// Tauri serialization to the app crate.
pub struct VacationRepository<'a> {
    pool: &'a SqlitePool,
}

impl<'a> VacationRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn get(
        &self,
        account_id: &str,
    ) -> Result<Option<VacationSettingsRecord>, VacationError> {
        sqlx::query_as::<_, StoredVacationSettings>(&format!(
            "{VACATION_SELECT} WHERE account_id = ?"
        ))
        .bind(account_id)
        .fetch_optional(self.pool)
        .await
        .map(|row| row.map(Into::into))
        .map_err(Into::into)
    }

    pub async fn save(
        &self,
        update: VacationSettingsUpdate,
    ) -> Result<VacationSettingsRecord, VacationError> {
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query_as::<_, StoredVacationSettings>(
            "INSERT INTO vacation_settings
                (account_id, enabled, subject, body, starts_at, ends_at, only_contacts, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(account_id) DO UPDATE SET
                enabled = excluded.enabled,
                subject = excluded.subject,
                body = excluded.body,
                starts_at = excluded.starts_at,
                ends_at = excluded.ends_at,
                only_contacts = excluded.only_contacts,
                updated_at = excluded.updated_at
             RETURNING account_id, enabled, subject, body, starts_at, ends_at, only_contacts, updated_at",
        )
        .bind(&update.account_id)
        .bind(if update.enabled { 1 } else { 0 })
        .bind(update.subject.as_deref())
        .bind(&update.body)
        .bind(update.starts_at.as_deref())
        .bind(update.ends_at.as_deref())
        .bind(if update.only_contacts { 1 } else { 0 })
        .bind(&now)
        .fetch_one(self.pool)
        .await
        .map(Into::into)
        .map_err(Into::into)
    }
}
