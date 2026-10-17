use serde::Deserialize;
use sqlx::SqlitePool;

use crate::commands::error::CommandResult;
use crate::AppState;

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/star/", optional_fields = nullable))]
pub struct StarThreadsInput {
    pub thread_ids: Vec<String>,
    pub starred: bool,
}

/// One transaction for the whole selection, like `set_threads_read`: the local
/// flags and the queued provider intent commit together or not at all.
#[tauri::command]
pub async fn set_threads_starred(
    pool: tauri::State<'_, SqlitePool>,
    state: tauri::State<'_, AppState>,
    input: StarThreadsInput,
) -> CommandResult<()> {
    satchel_core::mail_actions::set_flags(
        pool.inner(),
        &state.queue,
        &input.thread_ids,
        None,
        Some(input.starred),
        None,
    )
    .await?;
    Ok(())
}
