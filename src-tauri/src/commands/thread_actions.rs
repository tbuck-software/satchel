//! Additional thread-level actions: mute (silence the thread, hide from inbox
//! without archiving), mark-as-spam (move to spam folder), Not spam, and the
//! local spam filter's settings.

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

/// Not spam: train the threads as wanted mail and bring those in Spam back to
/// the Inbox. One key from the list, the reader and the Spam folder.
#[tauri::command]
pub async fn mark_threads_not_spam(
    pool: tauri::State<'_, SqlitePool>,
    state: tauri::State<'_, AppState>,
    input: SpamThreadsInput,
) -> CommandResult<satchel_core::mail_actions::NotSpamReceipt> {
    satchel_core::mail_actions::mark_threads_not_spam(pool.inner(), &state.queue, &input.thread_ids)
        .await
        .map_err(Into::into)
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/thread_actions/", optional_fields = nullable))]
pub struct UndoNotSpamInput {
    pub receipt: satchel_core::mail_actions::NotSpamReceipt,
}

/// Undo a Not spam: unlearn it and put the threads back in Spam.
#[tauri::command]
pub async fn undo_not_spam(
    pool: tauri::State<'_, SqlitePool>,
    state: tauri::State<'_, AppState>,
    input: UndoNotSpamInput,
) -> CommandResult<()> {
    satchel_core::mail_actions::undo_not_spam(pool.inner(), &state.queue, &input.receipt)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn get_spam_filter_status(
    pool: tauri::State<'_, SqlitePool>,
) -> CommandResult<satchel_core::spam::SpamFilterStatus> {
    satchel_core::spam::status(pool.inner())
        .await
        .map_err(|error| {
            tracing::warn!(%error, "spam filter status unreadable");
            crate::commands::error::CommandError::new("backend.errors.spamFilterUnavailable")
        })
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/thread_actions/", optional_fields = nullable))]
pub struct SetSpamFilterModeInput {
    pub mode: satchel_core::spam::SpamFilterMode,
}

#[tauri::command]
pub async fn set_spam_filter_mode(
    pool: tauri::State<'_, SqlitePool>,
    input: SetSpamFilterModeInput,
) -> CommandResult<()> {
    satchel_core::spam::set_mode(pool.inner(), input.mode)
        .await
        .map_err(|error| {
            tracing::warn!(%error, "spam filter mode not saved");
            crate::commands::error::CommandError::new("backend.errors.spamFilterUnavailable")
        })
}
