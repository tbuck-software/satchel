use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::commands::error::{CommandError, CommandResult};
use crate::utils::oauth::{access_token_from_account_config, OAuthProvider};
use satchel_core::db::accounts::{AccountProviderConfig, AccountRepository};
use satchel_core::db::vacation::{
    VacationError, VacationRepository, VacationSettingsRecord, VacationSettingsUpdate,
};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/vacation/")
)]
pub struct VacationInput {
    pub account_id: String,
    pub enabled: bool,
    #[serde(default)]
    pub subject: Option<String>,
    pub body: String,
    /// RFC3339; vacation reply only fires while now() is in [starts_at, ends_at).
    #[serde(default)]
    pub starts_at: Option<String>,
    #[serde(default)]
    pub ends_at: Option<String>,
    /// If true: only reply to known contacts (Gmail-specific).
    #[serde(default)]
    pub only_contacts: bool,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/vacation/")
)]
pub struct VacationDto {
    pub account_id: String,
    pub enabled: i64,
    pub subject: Option<String>,
    pub body: String,
    pub starts_at: Option<String>,
    pub ends_at: Option<String>,
    pub only_contacts: i64,
    pub updated_at: String,
}

impl From<VacationSettingsRecord> for VacationDto {
    fn from(record: VacationSettingsRecord) -> Self {
        Self {
            account_id: record.account_id,
            enabled: record.enabled,
            subject: record.subject,
            body: record.body,
            starts_at: record.starts_at,
            ends_at: record.ends_at,
            only_contacts: record.only_contacts,
            updated_at: record.updated_at,
        }
    }
}

#[tauri::command]
pub async fn get_vacation(
    pool: tauri::State<'_, crate::db::ReadPool>,
    account_id: String,
) -> CommandResult<Option<VacationDto>> {
    VacationRepository::new(pool.inner().as_ref())
        .get(&account_id)
        .await
        .map(|record| record.map(Into::into))
        .map_err(CommandError::from)
}

#[tauri::command]
pub async fn set_vacation(
    pool: tauri::State<'_, SqlitePool>,
    input: VacationInput,
) -> CommandResult<VacationDto> {
    let record = VacationRepository::new(pool.inner())
        .save(VacationSettingsUpdate {
            account_id: input.account_id.clone(),
            enabled: input.enabled,
            subject: input.subject.clone(),
            body: input.body.clone(),
            starts_at: input.starts_at.clone(),
            ends_at: input.ends_at.clone(),
            only_contacts: input.only_contacts,
        })
        .await
        .map_err(CommandError::from)?;

    let account = AccountRepository::new(pool.inner())
        .load_provider_config(&input.account_id)
        .await
        .map_err(CommandError::from)?;
    if let Some(account) = account {
        push_vacation_to_remote(&account, &input)
            .await
            .map_err(map_remote_update_error)?;
    }

    Ok(record.into())
}

fn map_remote_update_error(error: impl ToString) -> CommandError {
    let detail = error.to_string();
    tracing::error!(%detail, "vacation remote update failed");
    CommandError::new("backend.errors.vacationRemoteUpdateFailed")
}

async fn push_vacation_to_remote(
    account: &AccountProviderConfig,
    input: &VacationInput,
) -> Result<(), String> {
    match account.provider.as_str() {
        "gmail" => push_gmail_vacation(&account.config, input).await,
        "outlook" => push_outlook_vacation(&account.config, input).await,
        // IMAP-only accounts: no native vacation API. Rely on server-side Sieve
        // or document for the user that they need to set it via webmail.
        _ => Ok(()),
    }
}

async fn push_gmail_vacation(
    config_json: &crate::commands::accounts::AccountConfig,
    input: &VacationInput,
) -> Result<(), String> {
    let token = access_token_from_account_config(OAuthProvider::Gmail, config_json).await?;
    let starts_ms = input
        .starts_at
        .as_deref()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.timestamp_millis());
    let ends_ms = input
        .ends_at
        .as_deref()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.timestamp_millis());
    let mut payload = serde_json::json!({
        "enableAutoReply": input.enabled,
        "responseSubject": input.subject.clone().unwrap_or_else(|| "Out of Office".into()),
        "responseBodyPlainText": input.body,
        "restrictToContacts": input.only_contacts,
    });
    if let Some(start) = starts_ms {
        payload["startTime"] = start.into();
    }
    if let Some(end) = ends_ms {
        payload["endTime"] = end.into();
    }
    let client = satchel_core::utils::http::client();
    let response = client
        .put("https://gmail.googleapis.com/gmail/v1/users/me/settings/vacation")
        .bearer_auth(token)
        .json(&payload)
        .send()
        .await
        .map_err(|err| format!("gmail vacation push failed: {err}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!(
            "gmail vacation push failed with status {status}: {body}"
        ));
    }
    Ok(())
}

impl From<VacationError> for CommandError {
    fn from(error: VacationError) -> Self {
        tracing::error!(error = ?error, "vacation settings persistence failed");
        Self::new("backend.errors.vacationPersistenceFailed")
    }
}

async fn push_outlook_vacation(
    config_json: &crate::commands::accounts::AccountConfig,
    input: &VacationInput,
) -> Result<(), String> {
    let token = access_token_from_account_config(OAuthProvider::Outlook, config_json).await?;
    let status = if input.enabled {
        if input.starts_at.is_some() && input.ends_at.is_some() {
            "scheduled"
        } else {
            "alwaysEnabled"
        }
    } else {
        "disabled"
    };
    let mut auto_reply = serde_json::json!({
        "status": status,
        "internalReplyMessage": input.body,
        "externalReplyMessage": input.body,
        "externalAudience": if input.only_contacts { "contactsOnly" } else { "all" },
    });
    if status == "scheduled" {
        auto_reply["scheduledStartDateTime"] = serde_json::json!({
            "dateTime": input.starts_at,
            "timeZone": "UTC",
        });
        auto_reply["scheduledEndDateTime"] = serde_json::json!({
            "dateTime": input.ends_at,
            "timeZone": "UTC",
        });
    }
    let payload = serde_json::json!({ "automaticRepliesSetting": auto_reply });
    let client = satchel_core::utils::http::client();
    let response = client
        .patch("https://graph.microsoft.com/v1.0/me/mailboxSettings")
        .bearer_auth(token)
        .json(&payload)
        .send()
        .await
        .map_err(|err| format!("outlook vacation push failed: {err}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!(
            "outlook vacation push failed with status {status}: {body}"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persistence_errors_use_a_stable_redacted_key() {
        let detail = "near SELECT: secret vacation SQL";
        let error = CommandError::from(VacationError::Persistence(sqlx::Error::Protocol(
            detail.into(),
        )));
        let serialized = serde_json::to_value(error).expect("command error JSON");
        assert_eq!(
            serialized,
            serde_json::json!({ "key": "backend.errors.vacationPersistenceFailed" })
        );
        assert!(!serialized.to_string().contains(detail));
    }

    #[test]
    fn remote_errors_use_a_stable_redacted_key() {
        let detail = "provider returned secret response details";
        let error = map_remote_update_error(detail);
        let serialized = serde_json::to_value(error).expect("command error JSON");
        assert_eq!(
            serialized,
            serde_json::json!({ "key": "backend.errors.vacationRemoteUpdateFailed" })
        );
        assert!(!serialized.to_string().contains(detail));
    }
}
