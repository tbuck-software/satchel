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
    let now = chrono::Utc::now().to_rfc3339();
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|err| err.to_string())?;
    sqlx::query("UPDATE threads SET is_muted = ?, updated_at = ? WHERE id = ?")
        .bind(input.muted)
        .bind(&now)
        .bind(&input.thread_id)
        .execute(&mut *tx)
        .await
        .map_err(|err| err.to_string())?;
    if input.muted {
        // Muting silences the thread, so its messages are read. Zeroing
        // `threads.unread_count` on its own only held until the next
        // projection refresh derived it from `messages` again.
        sqlx::query("UPDATE messages SET is_read = 1, updated_at = ? WHERE thread_id = ?")
            .bind(&now)
            .bind(&input.thread_id)
            .execute(&mut *tx)
            .await
            .map_err(|err| err.to_string())?;
        satchel_core::db::threads::refresh_thread_projections_tx(
            &mut tx,
            std::slice::from_ref(&input.thread_id),
        )
        .await
        .map_err(|err| err.to_string())?;
    }
    tx.commit().await.map_err(|err| err.to_string())?;
    Ok(())
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
