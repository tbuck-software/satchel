//! Platform glue that stays in the app: the macOS AppKit bridges and the
//! tracing setup. The portable helpers live in `satchel_core::utils`.
pub use satchel_core::utils::*;

pub mod build_info;

#[cfg(target_os = "macos")]
pub mod mac_dock_menu;
#[cfg(target_os = "macos")]
pub mod mac_notify;
#[cfg(target_os = "macos")]
pub mod mac_quick_look;
pub mod redact;
pub mod telemetry;

/// `<AppData>/satchel` — the profile directory holding the database and the
/// fallback master key. Core takes the path; only this crate can ask Tauri for
/// it.
pub fn profile_dir(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    use tauri::Manager;
    app.path()
        .resolve("satchel", tauri::path::BaseDirectory::AppData)
        .map_err(|err| err.to_string())
}
pub mod navigation;
