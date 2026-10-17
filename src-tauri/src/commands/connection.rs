use serde::Serialize;
use tauri::State;

use super::error::CommandResult;

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/connection/")
)]
pub struct ConnectionStateDto {
    /// Native code does not infer internet reachability. The webview uses its OS network hint.
    pub online: Option<bool>,
    pub syncing: bool,
    pub last_sync_at: Option<String>,
    pub pending_outbox: u64,
    /// Subset of `pending_outbox` that is mail waiting to go out, not a queued
    /// archive/flag mutation. The status line needs the two apart.
    pub pending_sends: u64,
    pub failed_outbox: u64,
    pub failed_sends: u64,
    pub failed_remote_mutations: u64,
}

#[tauri::command]
pub async fn current_connection_state(
    pool: State<'_, crate::db::ReadPool>,
    state: State<'_, crate::AppState>,
) -> CommandResult<ConnectionStateDto> {
    let snapshot = crate::connection_state::database_snapshot(pool.inner().as_ref()).await;

    Ok(ConnectionStateDto {
        online: None,
        syncing: state.sync_activity.is_syncing(),
        last_sync_at: snapshot.last_sync_at,
        pending_outbox: snapshot.pending_outbox,
        pending_sends: snapshot.pending_sends,
        failed_outbox: snapshot.failed_outbox,
        failed_sends: snapshot.failed_sends,
        failed_remote_mutations: snapshot.failed_remote_mutations,
    })
}
