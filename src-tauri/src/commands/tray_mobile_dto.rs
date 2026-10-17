//! Mobile tray fallbacks keep their DTOs exportable on desktop.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/tray_mobile/")
)]
pub struct TrayPreferencesDto {
    pub quit_on_close: bool,
    pub start_minimized: bool,
    pub autostart_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Default)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/tray_mobile/")
)]
pub struct TraySnapshotDto {
    pub new_threads: Vec<serde_json::Value>,
    pub upcoming_events: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/tray_mobile/", optional_fields = nullable))]
pub struct TrayThreadActionInput {
    pub thread_id: String,
    pub action: String,
}
