use serde::Deserialize;
use sqlx::SqlitePool;

use crate::commands::error::CommandResult;
use crate::AppState;

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/move_threads/", optional_fields = nullable))]
pub struct MoveThreadsInput {
    pub thread_ids: Vec<String>,
    pub target_folder_id: String,
}

/// Queue a durable move without changing confirmed provider identities.
#[tauri::command]
pub async fn move_threads_to_folder(
    pool: tauri::State<'_, SqlitePool>,
    state: tauri::State<'_, AppState>,
    input: MoveThreadsInput,
) -> CommandResult<()> {
    satchel_core::mail_actions::move_threads(
        pool.inner(),
        &state.queue,
        &input.thread_ids,
        satchel_core::mail_actions::MoveTarget::Folder(&input.target_folder_id),
    )
    .await?;
    Ok(())
}
