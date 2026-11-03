//! Additional thread-level actions: mute (silence the thread, hide from inbox
//! without archiving) and mark-as-spam (move to spam folder).

use serde::Deserialize;
use sqlx::SqlitePool;

use crate::commands::error::CommandResult;
use crate::AppState;

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/thread_actions/", optional_fields = nullable))]
pub struct MuteThreadInput {
    pub thread_id: String,
    pub muted: bool,
}

#[tauri::command]
pub async fn set_thread_muted(
    pool: tauri::State<'_, SqlitePool>,
    input: MuteThreadInput,
) -> CommandResult<()> {
    satchel_core::db::threads::ThreadRepository::new(pool.inner())
        .set_muted(&input.thread_id, input.muted)
        .await
        .map_err(Into::into)
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/thread_actions/", optional_fields = nullable))]
pub struct SpamThreadsInput {
    pub thread_ids: Vec<String>,
}

#[tauri::command]
pub async fn mark_threads_spam(
    pool: tauri::State<'_, SqlitePool>,
    state: tauri::State<'_, AppState>,
    input: SpamThreadsInput,
) -> CommandResult<satchel_core::mail_actions::TrashReceipt> {
    satchel_core::mail_actions::mark_threads_spam(pool.inner(), &state.queue, &input.thread_ids)
        .await
        .map_err(Into::into)
}
