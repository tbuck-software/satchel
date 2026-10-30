use sqlx::SqlitePool;

use crate::commands::error::{CommandError, CommandResult};
use crate::sync::queue::ACTION_SYNC_DRAFT;
use crate::AppState;
pub use satchel_core::drafts::{DraftDto, DraftInput};

#[tauri::command]
pub async fn save_draft(
    app: tauri::AppHandle,
    pool: tauri::State<'_, SqlitePool>,
    state: tauri::State<'_, AppState>,
    input: DraftInput,
) -> CommandResult<DraftDto> {
    let (draft, _) = satchel_core::drafts::save(pool.inner(), &state.queue, &input)
        .await
        .map_err(CommandError::from)?;
    if draft.send_cancelled {
        use tauri::Emitter;
        let _ = app.emit(
            "outbox:queued",
            serde_json::json!({ "action_type": ACTION_SYNC_DRAFT, "draft_id": draft.id }),
        );
    }
    Ok(draft)
}

#[tauri::command]
pub async fn list_drafts(
    pool: tauri::State<'_, crate::db::ReadPool>,
) -> CommandResult<Vec<DraftDto>> {
    satchel_core::drafts::list(pool.inner().as_ref())
        .await
        .map_err(CommandError::from)
}

#[tauri::command]
pub async fn load_draft(
    pool: tauri::State<'_, crate::db::ReadPool>,
    write_pool: tauri::State<'_, SqlitePool>,
    id: String,
    state: tauri::State<'_, AppState>,
) -> CommandResult<Option<DraftDto>> {
    satchel_core::drafts::load(write_pool.inner(), pool.inner().as_ref(), &state.queue, &id)
        .await
        .map_err(CommandError::from)
}

#[tauri::command]
pub async fn delete_draft(
    pool: tauri::State<'_, SqlitePool>,
    state: tauri::State<'_, AppState>,
    id: String,
) -> CommandResult<()> {
    satchel_core::drafts::discard(pool.inner(), &state.queue, &id)
        .await
        .map_err(CommandError::from)
}
