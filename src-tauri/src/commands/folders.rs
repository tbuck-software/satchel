use serde::Deserialize;
use sqlx::SqlitePool;

use crate::{
    commands::error::{CommandError, CommandResult},
    db::folders::{Folder, FolderError, FolderRepository},
    AppState,
};

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/folders/", optional_fields = nullable))]
pub struct CreateFolderInput {
    pub account_id: String,
    pub name: String,
    #[serde(default)]
    pub parent_path: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/folders/", optional_fields = nullable))]
pub struct RenameFolderInput {
    pub folder_id: String,
    pub new_name: String,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/folders/", optional_fields = nullable))]
pub struct DeleteFolderInput {
    pub folder_id: String,
}

impl From<FolderError> for CommandError {
    fn from(error: FolderError) -> Self {
        match error {
            FolderError::NameEmpty => Self::new("backend.errors.folderNameEmpty"),
            FolderError::NameTooLong { max } => {
                Self::new("backend.errors.folderNameTooLong").with_param("max", max)
            }
            FolderError::NameContainsSeparator => {
                Self::new("backend.errors.folderNameContainsSeparator")
            }
            FolderError::NameInvalidCharacters => {
                Self::new("backend.errors.folderNameInvalidCharacters")
            }
            FolderError::PendingMove => Self::new("backend.errors.mailMovePending"),
            FolderError::NotFound => Self::new("backend.errors.folderNotFound"),
            FolderError::SystemFolderImmutable { name } => {
                Self::new("backend.errors.systemFolderImmutable").with_param("name", name)
            }
            FolderError::AlreadyExists { path } => {
                Self::new("backend.errors.folderExists").with_param("path", path)
            }
            FolderError::ContainsMessages { name, count } => {
                Self::new("backend.errors.folderContainsMessages")
                    .with_param("name", name)
                    .with_param("count", count)
            }
            FolderError::ContainsChildren { name, count } => {
                Self::new("backend.errors.folderContainsChildren")
                    .with_param("name", name)
                    .with_param("count", count)
            }
            other @ (FolderError::Persistence(_) | FolderError::Queue(_)) => {
                Self::unexpected(other)
            }
        }
    }
}

#[tauri::command]
pub async fn create_folder(
    pool: tauri::State<'_, SqlitePool>,
    state: tauri::State<'_, AppState>,
    input: CreateFolderInput,
) -> CommandResult<Folder> {
    FolderRepository::new(pool.inner(), state.queue.as_ref())
        .create(&input.account_id, &input.name, input.parent_path.as_deref())
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn rename_folder(
    pool: tauri::State<'_, SqlitePool>,
    state: tauri::State<'_, AppState>,
    input: RenameFolderInput,
) -> CommandResult<Folder> {
    FolderRepository::new(pool.inner(), state.queue.as_ref())
        .rename(&input.folder_id, &input.new_name)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn delete_folder(
    pool: tauri::State<'_, SqlitePool>,
    state: tauri::State<'_, AppState>,
    input: DeleteFolderInput,
) -> CommandResult<()> {
    FolderRepository::new(pool.inner(), state.queue.as_ref())
        .delete(&input.folder_id)
        .await
        .map_err(Into::into)
}
