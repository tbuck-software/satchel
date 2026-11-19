//! Explicit LAN discovery and authenticated internet delivery are independent.
use super::background_sync::PushDevice;
use super::error::{CommandError, CommandResult};
use serde::{Deserialize, Serialize};
#[cfg(any(desktop, target_os = "android"))]
use tauri::Manager;

#[cfg(desktop)]
mod desktop;
#[cfg(desktop)]
pub use desktop::Desktop;

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/relay_push/")
)]
pub enum RelayAction {
    Status,
    Start,
    Select,
    Approve,
    Cancel,
    Remove,
}

#[derive(Default, Deserialize, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/relay_push/")
)]
pub struct RelayStatus {
    pub phase: String,
    pub devices: Vec<PushDevice>,
    pub nearby: Vec<PushDevice>,
    pub code: Option<String>,
    pub request: Option<String>,
    pub name: Option<String>,
}

fn origin() -> &'static str {
    option_env!("SATCHEL_PUSH_RELAY_URL")
        .unwrap_or(include_str!("../../../../tooling/push-relay/origin.txt"))
        .trim_end_matches(['/', '\r', '\n'])
}

#[tauri::command]
pub async fn relay_push(
    app: tauri::AppHandle,
    action: RelayAction,
    id: Option<String>,
) -> CommandResult<RelayStatus> {
    if origin().is_empty() && matches!(action, RelayAction::Status) {
        return Ok(RelayStatus {
            phase: "unavailable".into(),
            ..Default::default()
        });
    }
    if origin().is_empty()
        && !matches!(
            action,
            RelayAction::Status | RelayAction::Remove | RelayAction::Cancel
        )
    {
        return Err(CommandError::new("settings.relay.unavailable"));
    }
    #[cfg(desktop)]
    return app.state::<Desktop>().apply(action, id).await;
    #[cfg(target_os = "android")]
    return app
        .state::<super::background_sync::AndroidBackgroundSync<tauri::Wry>>()
        .0
        .run_mobile_plugin_async(
            "relayPush",
            serde_json::json!({"action":action,"id":id,"origin":origin()}),
        )
        .await
        .map_err(|_| CommandError::new("settings.relay.failed"));
    #[cfg(not(any(desktop, target_os = "android")))]
    {
        let _ = (app, action, id);
        Err(CommandError::new("settings.relay.unavailable"))
    }
}
