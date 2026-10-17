use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::commands::error::{CommandError, CommandResult};
use satchel_core::db::aliases::{AliasError, AliasRepository, AliasUpsertRequest};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/aliases/"))]
pub struct AliasInput {
    pub id: Option<String>,
    pub account_id: String,
    pub email: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub is_default: bool,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/aliases/"))]
pub struct AliasDto {
    pub id: String,
    pub account_id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub is_default: i64,
    pub created_at: String,
    pub updated_at: String,
}

impl From<satchel_core::db::aliases::AliasRecord> for AliasDto {
    fn from(record: satchel_core::db::aliases::AliasRecord) -> Self {
        Self {
            id: record.id,
            account_id: record.account_id,
            email: record.email,
            display_name: record.display_name,
            is_default: record.is_default,
            created_at: record.created_at,
            updated_at: record.updated_at,
        }
    }
}

impl From<AliasInput> for AliasUpsertRequest {
    fn from(input: AliasInput) -> Self {
        Self {
            id: input.id,
            account_id: input.account_id,
            email: input.email,
            display_name: input.display_name,
            is_default: input.is_default,
        }
    }
}

pub(crate) fn alias_error_key(error: AliasError) -> String {
    match error {
        AliasError::InvalidEmail => "backend.errors.emailInvalid",
        AliasError::Persistence(error) => {
            tracing::error!(error = ?error, "alias persistence failed");
            "backend.errors.aliasPersistenceFailed"
        }
    }
    .to_owned()
}

impl From<AliasError> for CommandError {
    fn from(error: AliasError) -> Self {
        Self::new(alias_error_key(error))
    }
}

#[tauri::command]
pub async fn list_aliases(
    pool: tauri::State<'_, crate::db::ReadPool>,
    account_id: Option<String>,
) -> CommandResult<Vec<AliasDto>> {
    AliasRepository::new(pool.inner().as_ref())
        .list(account_id.as_deref())
        .await
        .map(|rows| rows.into_iter().map(Into::into).collect())
        .map_err(Into::into)
}

#[tauri::command]
pub async fn upsert_alias(
    pool: tauri::State<'_, SqlitePool>,
    input: AliasInput,
) -> CommandResult<AliasDto> {
    AliasRepository::new(pool.inner())
        .upsert(input.into())
        .await
        .map(Into::into)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn delete_alias(pool: tauri::State<'_, SqlitePool>, id: String) -> CommandResult<()> {
    AliasRepository::new(pool.inner())
        .delete(&id)
        .await
        .map_err(Into::into)
}

/// Validate that the given from_address is either the account's primary email or one of its aliases.
pub async fn assert_alias_owned(
    pool: &SqlitePool,
    account_id: &str,
    account_email: &str,
    from_address: &str,
) -> Result<(), String> {
    let normalized = from_address.trim().to_lowercase();
    if normalized == account_email.trim().to_lowercase() {
        return Ok(());
    }
    let exists = AliasRepository::new(pool)
        .contains(account_id, &normalized)
        .await
        .map_err(alias_error_key)?;
    if !exists {
        return Err(format!(
            "from-address {from_address} is not an owned alias of this account"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persistence_errors_use_a_stable_key_without_sql_details() {
        let detail = "near SELECT: secret alias SQL";
        let error = CommandError::from(AliasError::Persistence(sqlx::Error::Protocol(
            detail.into(),
        )));
        let serialized = serde_json::to_value(error).expect("command error JSON");

        assert_eq!(
            serialized,
            serde_json::json!({ "key": "backend.errors.aliasPersistenceFailed" })
        );
        assert!(!serialized.to_string().contains(detail));
    }

    #[test]
    fn invalid_email_uses_the_existing_validation_key() {
        assert_eq!(
            CommandError::from(AliasError::InvalidEmail).key,
            "backend.errors.emailInvalid"
        );
    }
}
