//! Labs: per-device switches for experimental features.
use serde::{Deserialize, Serialize};

use crate::commands::error::{CommandError, CommandResult};
use satchel_core::db::labs::{LabsError, LabsFlagRecord, LabsRepository};

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/labs/"))]
pub struct LabsFlagDto {
    pub id: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/labs/"))]
pub struct SetLabsFlagInput {
    pub id: String,
    pub enabled: bool,
}

impl From<LabsFlagRecord> for LabsFlagDto {
    fn from(record: LabsFlagRecord) -> Self {
        Self {
            id: record.id,
            enabled: record.enabled,
        }
    }
}

impl From<LabsError> for CommandError {
    fn from(error: LabsError) -> Self {
        match error {
            // The switches come from the same registry; an unknown id is a bug.
            LabsError::UnknownFlag => Self::new("backend.errors.unexpected"),
            LabsError::Persistence(error) => {
                tracing::error!(error = ?error, "labs persistence failed");
                Self::new("backend.errors.unexpected")
            }
        }
    }
}

#[tauri::command]
pub async fn list_labs_flags(
    pool: tauri::State<'_, crate::db::ReadPool>,
) -> CommandResult<Vec<LabsFlagDto>> {
    LabsRepository::new(pool.inner().as_ref())
        .list()
        .await
        .map(|rows| rows.into_iter().map(Into::into).collect())
        .map_err(Into::into)
}

#[tauri::command]
pub async fn set_labs_flag(
    pool: tauri::State<'_, sqlx::SqlitePool>,
    input: SetLabsFlagInput,
) -> CommandResult<LabsFlagDto> {
    LabsRepository::new(pool.inner())
        .set(&input.id, input.enabled)
        .await
        .map(Into::into)
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_use_stable_keys_without_sql_detail() {
        let unknown = serde_json::to_value(CommandError::from(LabsError::UnknownFlag)).unwrap();
        assert_eq!(
            unknown,
            serde_json::json!({ "key": "backend.errors.unexpected" })
        );
        let detail = "near SELECT: secret labs SQL";
        let failed = serde_json::to_value(CommandError::from(LabsError::Persistence(
            sqlx::Error::Protocol(detail.into()),
        )))
        .unwrap();
        assert!(!failed.to_string().contains(detail));
    }
}
