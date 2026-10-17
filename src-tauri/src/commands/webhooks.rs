use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::commands::error::{CommandError, CommandResult};
use satchel_core::db::webhooks::{
    WebhookError, WebhookRecord, WebhookRepository, WebhookUpsertRequest,
};

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/webhooks/")
)]
pub struct WebhookDto {
    pub id: String,
    pub url: String,
    pub events_json: String,
    pub enabled: i64,
    pub created_at: String,
    pub updated_at: String,
}

impl From<WebhookRecord> for WebhookDto {
    fn from(record: WebhookRecord) -> Self {
        Self {
            id: record.id,
            url: record.url,
            events_json: record.events_json,
            enabled: record.enabled,
            created_at: record.created_at,
            updated_at: record.updated_at,
        }
    }
}

#[derive(Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/webhooks/", optional_fields = nullable))]
pub struct WebhookInput {
    pub id: Option<String>,
    pub url: String,
    pub events: Vec<String>,
    pub enabled: bool,
    pub secret: Option<String>,
}

fn validate_webhook_url(url: &str) -> CommandResult<()> {
    let valid = reqwest::Url::parse(url).is_ok_and(|url| match url.scheme() {
        "https" => true,
        "http" => matches!(url.host_str(), Some("localhost" | "127.0.0.1")),
        _ => false,
    });
    if !valid {
        return Err(CommandError::new("backend.errors.webhookUrlInvalid"));
    }
    Ok(())
}

pub(crate) fn webhook_error_key(error: WebhookError) -> String {
    match error {
        WebhookError::PayloadSerialization(error) => {
            tracing::error!(error = ?error, "webhook payload serialization failed");
        }
        WebhookError::Persistence(error) => {
            tracing::error!(error = ?error, "webhook persistence failed");
        }
    }
    "backend.errors.webhookPersistenceFailed".to_owned()
}

impl From<WebhookError> for CommandError {
    fn from(error: WebhookError) -> Self {
        Self::new(webhook_error_key(error))
    }
}

#[tauri::command]
pub async fn list_webhooks(
    pool: tauri::State<'_, crate::db::ReadPool>,
) -> CommandResult<Vec<WebhookDto>> {
    WebhookRepository::new(pool.inner().as_ref())
        .list()
        .await
        .map(|rows| rows.into_iter().map(Into::into).collect())
        .map_err(Into::into)
}

#[tauri::command]
pub async fn upsert_webhook(
    pool: tauri::State<'_, SqlitePool>,
    input: WebhookInput,
) -> CommandResult<WebhookDto> {
    validate_webhook_url(&input.url)?;

    WebhookRepository::new(pool.inner())
        .upsert(WebhookUpsertRequest {
            id: input.id,
            url: input.url,
            events: input.events,
            enabled: input.enabled,
            secret: input.secret,
        })
        .await
        .map(Into::into)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn delete_webhook(pool: tauri::State<'_, SqlitePool>, id: String) -> CommandResult<()> {
    WebhookRepository::new(pool.inner())
        .delete(&id)
        .await
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persistence_errors_use_a_stable_key_without_sql_details() {
        let detail = "near SELECT: secret webhook SQL";
        let error = CommandError::from(WebhookError::Persistence(sqlx::Error::Protocol(
            detail.into(),
        )));
        let serialized = serde_json::to_value(error).expect("command error JSON");

        assert_eq!(
            serialized,
            serde_json::json!({ "key": "backend.errors.webhookPersistenceFailed" })
        );
        assert!(!serialized.to_string().contains(detail));
    }

    #[test]
    fn serializable_webhook_dto_does_not_contain_a_secret() {
        let serialized = serde_json::to_value(WebhookDto {
            id: "wh-1".into(),
            url: "https://example.test/hook".into(),
            events_json: "[\"message.created\"]".into(),
            enabled: 1,
            created_at: "2026-01-01".into(),
            updated_at: "2026-01-01".into(),
        })
        .expect("webhook DTO JSON");

        assert!(!serialized.to_string().contains("secret"));
    }

    #[test]
    fn invalid_url_uses_a_stable_key() {
        let error = validate_webhook_url("ftp://example.test").expect_err("invalid URL");
        assert_eq!(error.key, "backend.errors.webhookUrlInvalid");
    }

    #[test]
    fn accepts_https_and_exact_loopback_http_hosts() {
        assert!(validate_webhook_url("https://hooks.example.test/events").is_ok());
        assert!(validate_webhook_url("http://localhost:8799/events").is_ok());
        assert!(validate_webhook_url("http://127.0.0.1:8799/events").is_ok());
    }

    #[test]
    fn rejects_http_host_prefix_confusion() {
        for url in [
            "http://localhost.attacker.example/events",
            "http://127.0.0.1.evil/events",
            "http://localhost@attacker.example/events",
        ] {
            let error = validate_webhook_url(url).expect_err("remote HTTP URL");
            assert_eq!(error.key, "backend.errors.webhookUrlInvalid");
        }
    }
}
