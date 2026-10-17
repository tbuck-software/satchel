use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/crash/"))]
pub struct CrashEntry {
    pub raw: String,
}

/// Returns any panics captured since the previous successful start, then clears them.
/// The frontend shows a one-time "Satchel crashed last time — copy report?" toast.
#[tauri::command]
pub fn pending_crash_reports() -> Vec<CrashEntry> {
    crate::utils::telemetry::drain_pending_crashes()
        .into_iter()
        .map(|raw| CrashEntry { raw })
        .collect()
}
