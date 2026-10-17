//! Tauri-free persistence for reusable mail templates.
use sqlx::{FromRow, SqlitePool};
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct TemplateUpsertRequest {
    pub id: Option<String>,
    pub account_id: Option<String>,
    pub name: String,
    pub subject: Option<String>,
    pub body_text: String,
    pub body_html: Option<String>,
    pub shortcut: Option<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub attachments_json: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateRecord {
    pub id: String,
    pub account_id: Option<String>,
    pub name: String,
    pub subject: Option<String>,
    pub body_text: String,
    pub body_html: Option<String>,
    pub shortcut: Option<String>,
    pub cc: Option<Vec<String>>,
    pub bcc: Option<Vec<String>>,
    pub attachments_json: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Error)]
pub enum TemplateError {
    #[error("template was not found")]
    NotFound,
    #[error("template persistence failed: {0}")]
    Persistence(#[source] sqlx::Error),
}

impl From<sqlx::Error> for TemplateError {
    fn from(error: sqlx::Error) -> Self {
        Self::Persistence(error)
    }
}

#[derive(Debug, FromRow)]
struct StoredTemplate {
    id: String,
    account_id: Option<String>,
    name: String,
    subject: Option<String>,
    body_text: String,
    body_html: Option<String>,
    shortcut: Option<String>,
    cc_json: Option<String>,
    bcc_json: Option<String>,
    attachments_json: Option<String>,
    created_at: String,
    updated_at: String,
}

impl From<StoredTemplate> for TemplateRecord {
    fn from(row: StoredTemplate) -> Self {
        Self {
            id: row.id,
            account_id: row.account_id,
            name: row.name,
            subject: row.subject,
            body_text: row.body_text,
            body_html: row.body_html,
            shortcut: row.shortcut,
            cc: decode_recipients(row.cc_json),
            bcc: decode_recipients(row.bcc_json),
            attachments_json: row.attachments_json,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

fn decode_recipients(raw: Option<String>) -> Option<Vec<String>> {
    raw.map(|value| serde_json::from_str(&value).unwrap_or_default())
}

const TEMPLATE_SELECT: &str = "SELECT id, account_id, name, subject, body_text, body_html, shortcut, cc_json, bcc_json, attachments_json, created_at, updated_at FROM templates";

/// Owns durable template records while leaving rendering and Tauri command
/// concerns to the app crate.
pub struct TemplateRepository<'a> {
    pool: &'a SqlitePool,
}

impl<'a> TemplateRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn list(
        &self,
        account_id: Option<&str>,
    ) -> Result<Vec<TemplateRecord>, TemplateError> {
        let rows = match account_id {
            Some(account_id) => {
                sqlx::query_as::<_, StoredTemplate>(&format!(
                    "{TEMPLATE_SELECT} WHERE account_id IS NULL OR account_id = ? ORDER BY name ASC"
                ))
                .bind(account_id)
                .fetch_all(self.pool)
                .await?
            }
            None => {
                sqlx::query_as::<_, StoredTemplate>(&format!("{TEMPLATE_SELECT} ORDER BY name ASC"))
                    .fetch_all(self.pool)
                    .await?
            }
        };

        Ok(rows.into_iter().map(Into::into).collect())
    }

    pub async fn upsert(
        &self,
        input: TemplateUpsertRequest,
    ) -> Result<TemplateRecord, TemplateError> {
        let id = input
            .id
            .unwrap_or_else(|| format!("tpl-{}", uuid::Uuid::new_v4()));
        let now = chrono::Utc::now().to_rfc3339();
        let cc_json = serde_json::to_string(&input.cc).unwrap_or_else(|_| "[]".to_string());
        let bcc_json = serde_json::to_string(&input.bcc).unwrap_or_else(|_| "[]".to_string());

        sqlx::query_as::<_, StoredTemplate>(
            "INSERT INTO templates (id, account_id, name, subject, body_text, body_html, shortcut, cc_json, bcc_json, attachments_json, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
                account_id = excluded.account_id,
                name = excluded.name,
                subject = excluded.subject,
                body_text = excluded.body_text,
                body_html = excluded.body_html,
                shortcut = excluded.shortcut,
                cc_json = excluded.cc_json,
                bcc_json = excluded.bcc_json,
                attachments_json = excluded.attachments_json,
                updated_at = excluded.updated_at
             RETURNING id, account_id, name, subject, body_text, body_html, shortcut,
                       cc_json, bcc_json, attachments_json, created_at, updated_at",
        )
        .bind(&id)
        .bind(input.account_id.as_deref())
        .bind(&input.name)
        .bind(input.subject.as_deref())
        .bind(&input.body_text)
        .bind(input.body_html.as_deref())
        .bind(input.shortcut.as_deref())
        .bind(cc_json)
        .bind(bcc_json)
        .bind(input.attachments_json.as_deref())
        .bind(&now)
        .bind(&now)
        .fetch_one(self.pool)
        .await
        .map(Into::into)
        .map_err(Into::into)
    }

    pub async fn get(&self, id: &str) -> Result<TemplateRecord, TemplateError> {
        sqlx::query_as::<_, StoredTemplate>(&format!("{TEMPLATE_SELECT} WHERE id = ?"))
            .bind(id)
            .fetch_one(self.pool)
            .await
            .map(Into::into)
            .map_err(|error| match error {
                sqlx::Error::RowNotFound => TemplateError::NotFound,
                other => TemplateError::Persistence(other),
            })
    }

    pub async fn delete(&self, id: &str) -> Result<(), TemplateError> {
        sqlx::query("DELETE FROM templates WHERE id = ?")
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }
}
