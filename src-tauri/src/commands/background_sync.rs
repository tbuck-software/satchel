//! Keep the app alive on Android so IMAP IDLE can hold its connection.
//!
//! Nothing here syncs. `spawn_idle_supervisors` already runs on Android and a
//! new message already reaches `show_mail_notification`; what the phone took
//! away was the process. Android froze the app a minute after it went to the
//! background and the low-memory killer removed it three minutes later, so mail
//! arriving while the app was closed went unnoticed. The foreground service
//! moves the process out of the cached bucket, and that is the entire fix.

#[cfg(target_os = "android")]
use tauri::{plugin::PluginHandle, Manager, Runtime};

#[cfg(target_os = "android")]
use crate::commands::error::CommandError;
use crate::commands::error::CommandResult;

#[cfg(target_os = "android")]
struct AndroidBackgroundSync<R: Runtime>(PluginHandle<R>);

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/background_sync/")
)]
pub struct BackgroundSyncStatus {
    pub enabled: bool,
    pub running: bool,
    #[serde(rename = "batteryOptimizationExempt")]
    pub battery_optimization_exempt: bool,
}

pub fn init<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    let builder = tauri::plugin::Builder::new("satchel-background-sync");
    #[cfg(target_os = "android")]
    let builder = builder.setup(|app, api| {
        let handle =
            api.register_android_plugin("de.tbuck.satchel.background", "BackgroundSyncPlugin")?;
        app.manage(AndroidBackgroundSync(handle));
        Ok(())
    });
    builder.build()
}

/// Turn the background connection on or off.
///
/// A no-op everywhere but Android. On the desktop the process simply keeps
/// running, and iOS forbids long-lived sockets in the background outright — its
/// answer would have to be push, not a service.
#[tauri::command]
pub async fn set_background_sync(
    #[cfg_attr(not(target_os = "android"), allow(unused_variables))] app: tauri::AppHandle,
    #[cfg_attr(not(target_os = "android"), allow(unused_variables))] enabled: bool,
) -> CommandResult<()> {
    #[cfg(target_os = "android")]
    {
        let command = if enabled { "start" } else { "stop" };
        // Logged rather than left to the caller's console: the WebView forwards
        // nothing to logcat, so a `console.warn` on the frontend side is
        // invisible on a real phone and the first failure here cost an hour of
        // guessing at which of five links in the chain had broken.
        tracing::info!(enabled, "background sync service requested");
        app.state::<AndroidBackgroundSync<tauri::Wry>>()
            .0
            .run_mobile_plugin_async::<serde_json::Value>(command, serde_json::json!({}))
            .await
            .map_err(|error| {
                tracing::warn!(%error, "background sync service refused");
                CommandError::unexpected(error)
            })?;
    }
    Ok(())
}

#[tauri::command]
pub async fn background_sync_status(
    #[cfg_attr(not(target_os = "android"), allow(unused_variables))] app: tauri::AppHandle,
) -> CommandResult<BackgroundSyncStatus> {
    #[cfg(target_os = "android")]
    {
        return app
            .state::<AndroidBackgroundSync<tauri::Wry>>()
            .0
            .run_mobile_plugin_async::<BackgroundSyncStatus>("status", serde_json::json!({}))
            .await
            .map_err(CommandError::unexpected);
    }
    #[cfg(not(target_os = "android"))]
    Ok(BackgroundSyncStatus {
        enabled: true,
        running: true,
        battery_optimization_exempt: true,
    })
}

#[tauri::command]
pub async fn open_background_sync_settings(
    #[cfg_attr(not(target_os = "android"), allow(unused_variables))] app: tauri::AppHandle,
) -> CommandResult<()> {
    #[cfg(target_os = "android")]
    app.state::<AndroidBackgroundSync<tauri::Wry>>()
        .0
        .run_mobile_plugin_async::<serde_json::Value>("openSettings", serde_json::json!({}))
        .await
        .map_err(CommandError::unexpected)?;
    Ok(())
}
