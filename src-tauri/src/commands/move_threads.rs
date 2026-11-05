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
///
/// Moving mail out of Spam into a folder that is not Trash tells the spam
/// filter that mail was wanted.
#[tauri::command]
pub async fn move_threads_to_folder(
    pool: tauri::State<'_, SqlitePool>,
    state: tauri::State<'_, AppState>,
    input: MoveThreadsInput,
) -> CommandResult<()> {
    move_threads_to_folder_inner(pool.inner(), state.inner(), input).await
}

pub(crate) async fn move_threads_to_folder_inner(
    pool: &SqlitePool,
    state: &AppState,
    input: MoveThreadsInput,
) -> CommandResult<()> {
    let rescued = rescued_from_spam(pool, &input).await;
    satchel_core::mail_actions::move_threads(
        pool,
        &state.queue,
        &input.thread_ids,
        satchel_core::mail_actions::MoveTarget::Folder(&input.target_folder_id),
    )
    .await?;
    if !rescued.is_empty() {
        if let Err(error) = satchel_core::spam::train_messages(
            pool,
            &rescued,
            satchel_core::spam::Label::Ham,
            satchel_core::spam::Signal::Explicit,
            None,
        )
        .await
        {
            tracing::warn!(%error, "spam filter training failed after a move out of Spam");
        }
    }
    Ok(())
}

/// The messages this move takes out of Spam, if its target is a folder where
/// wanted mail lives.
async fn rescued_from_spam(pool: &SqlitePool, input: &MoveThreadsInput) -> Vec<String> {
    let target: Option<String> =
        sqlx::query_scalar("SELECT LOWER(COALESCE(folder_type, '')) FROM folders WHERE id = ?")
            .bind(&input.target_folder_id)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
    if target
        .as_deref()
        .is_none_or(|kind| matches!(kind, "spam" | "trash"))
    {
        return Vec::new();
    }
    satchel_core::spam::spam_folder_messages(pool, &input.thread_ids)
        .await
        .unwrap_or_default()
}
