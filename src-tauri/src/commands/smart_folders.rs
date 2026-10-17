use serde::{Deserialize, Serialize};

use crate::commands::error::{CommandError, CommandResult};
use satchel_core::db::smart_folders::{
    SmartFolderError, SmartFolderRepository, SmartFolderUpsertRequest,
};

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/smart_folders/", optional_fields = nullable))]
pub struct SmartFolderInput {
    pub id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub icon: Option<String>,
    /// Search query in the same syntax as `commands::messages::search_messages`
    /// (e.g. "from:boss has:attachment in:inbox").
    pub query: String,
    #[serde(default)]
    pub sort_order: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/smart_folders/")
)]
pub struct SmartFolderDto {
    pub id: String,
    pub name: String,
    pub icon: Option<String>,
    pub query: String,
    pub sort_order: i64,
    pub created_at: String,
    pub updated_at: String,
}

impl From<satchel_core::db::smart_folders::SmartFolderRecord> for SmartFolderDto {
    fn from(record: satchel_core::db::smart_folders::SmartFolderRecord) -> Self {
        Self {
            id: record.id,
            name: record.name,
            icon: record.icon,
            query: record.query,
            sort_order: record.sort_order,
            created_at: record.created_at,
            updated_at: record.updated_at,
        }
    }
}

impl From<SmartFolderError> for CommandError {
    fn from(error: SmartFolderError) -> Self {
        tracing::error!(error = ?error, "smart folder persistence failed");
        Self::new("backend.errors.smartFolderPersistenceFailed")
    }
}

impl From<SmartFolderInput> for SmartFolderUpsertRequest {
    fn from(input: SmartFolderInput) -> Self {
        Self {
            id: input.id,
            name: input.name,
            icon: input.icon,
            query: input.query,
            sort_order: input.sort_order,
        }
    }
}

#[tauri::command]
pub async fn list_smart_folders(
    pool: tauri::State<'_, crate::db::ReadPool>,
) -> CommandResult<Vec<SmartFolderDto>> {
    SmartFolderRepository::new(pool.inner().as_ref())
        .list()
        .await
        .map(|rows| rows.into_iter().map(Into::into).collect())
        .map_err(Into::into)
}

#[tauri::command]
pub async fn upsert_smart_folder(
    pool: tauri::State<'_, sqlx::SqlitePool>,
    input: SmartFolderInput,
) -> CommandResult<SmartFolderDto> {
    SmartFolderRepository::new(pool.inner())
        .upsert(input.into())
        .await
        .map(Into::into)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn delete_smart_folder(
    pool: tauri::State<'_, sqlx::SqlitePool>,
    id: String,
) -> CommandResult<()> {
    SmartFolderRepository::new(pool.inner())
        .delete(&id)
        .await
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persistence_errors_use_a_stable_key_without_sql_details() {
        let detail = "near SELECT: secret smart-folder SQL";
        let error = CommandError::from(SmartFolderError::Persistence(sqlx::Error::Protocol(
            detail.into(),
        )));
        let serialized = serde_json::to_value(error).expect("command error JSON");

        assert_eq!(
            serialized,
            serde_json::json!({ "key": "backend.errors.smartFolderPersistenceFailed" })
        );
        assert!(!serialized.to_string().contains(detail));
    }
}
