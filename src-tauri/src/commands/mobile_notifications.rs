//! Mail notifications on mobile: display, dismissal and tap routing.
//!
//! Nothing here talks to a server. Notifications are produced locally by the
//! in-process sync (Android additionally by `BackgroundMailSyncWorker` when the
//! main process is gone); this module only hands them to the platform and
//! carries a tap back to the frontend.

use serde::Serialize;

use crate::commands::error::CommandResult;

#[cfg(target_os = "android")]
use tauri::{
    ipc::{Channel, InvokeResponseBody},
    plugin::PluginHandle,
    Emitter, Manager, Runtime,
};

#[cfg(target_os = "android")]
struct AndroidMailNotifications<R: Runtime>(PluginHandle<R>);

#[cfg(target_os = "android")]
#[derive(Serialize)]
struct AndroidNavigationEventHandler {
    handler: Channel,
}

#[cfg(target_os = "android")]
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AndroidMailNotification<'a> {
    event_id: &'a str,
    thread_id: &'a str,
    account_id: &'a str,
    account_label: &'a str,
    sender: &'a str,
    subject: &'a str,
    preview: &'a str,
    sound: bool,
}

pub fn init<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    let builder = tauri::plugin::Builder::new("satchel-mail-notifications");
    #[cfg(target_os = "android")]
    let builder = builder.setup(|app, api| {
        let handle =
            api.register_android_plugin("de.tbuck.satchel.background", "MailNotificationPlugin")?;
        let app_handle = app.clone();
        handle.run_mobile_plugin::<()>(
            "setNavigationEventHandler",
            AndroidNavigationEventHandler {
                handler: Channel::new(move |_event: InvokeResponseBody| {
                    let _ = app_handle.emit("mobile:notification-navigation", ());
                    Ok(())
                }),
            },
        )?;
        app.manage(AndroidMailNotifications(handle));
        Ok(())
    });
    builder.build()
}

/// Hand one mail to the Android plugin and return its answer verbatim.
///
/// The plugin resolves for a notification it dropped too — `{"shown": false,
/// "reason": …}` — so the answer has to reach the caller. Only a failed
/// handover rejects. [`crate::commands::notifications::android_delivery_outcome`]
/// turns the answer into the outcome the notification outbox records.
#[cfg(target_os = "android")]
pub async fn show_android_mail_notification(
    app: &tauri::AppHandle,
    event_id: &str,
    account_id: &str,
    account_label: &str,
    sender: &str,
    subject: &str,
    preview: &str,
    thread_id: &str,
    sound: bool,
) -> Result<serde_json::Value, String> {
    let input = AndroidMailNotification {
        event_id,
        thread_id,
        account_id,
        account_label,
        sender,
        subject,
        preview,
        sound,
    };
    app.state::<AndroidMailNotifications<tauri::Wry>>()
        .0
        .run_mobile_plugin_async::<serde_json::Value>("showMailNotification", &input)
        .await
        .map_err(|error| format!("Android could not show mail notification: {error}"))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/mobile_notifications/")
)]
pub struct PendingMobileNotification {
    thread_id: Option<String>,
    action_id: Option<String>,
}

/// Take the notification tapped while the app was not running, exactly once.
#[tauri::command]
pub async fn consume_pending_mobile_notification(
    app: tauri::AppHandle,
) -> CommandResult<PendingMobileNotification> {
    #[cfg(target_os = "android")]
    {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct PendingNotification {
            thread_id: Option<String>,
            action_id: Option<String>,
        }
        let response: PendingNotification = app
            .state::<AndroidMailNotifications<tauri::Wry>>()
            .0
            .run_mobile_plugin_async("consumePendingNotification", serde_json::json!({}))
            .await
            .map_err(|error| format!("Android notification navigation failed: {error}"))?;
        return Ok(PendingMobileNotification {
            thread_id: response.thread_id.filter(|value| !value.trim().is_empty()),
            action_id: response.action_id,
        });
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        Ok(PendingMobileNotification {
            thread_id: None,
            action_id: None,
        })
    }
}

#[tauri::command]
pub async fn dismiss_mobile_notifications(
    app: tauri::AppHandle,
    thread_ids: Vec<String>,
) -> CommandResult<()> {
    dismiss_mail_notifications(&app, thread_ids).await?;
    Ok(())
}

pub async fn dismiss_mail_notifications(
    app: &tauri::AppHandle,
    thread_ids: Vec<String>,
) -> Result<(), String> {
    #[cfg(target_os = "android")]
    {
        app.state::<AndroidMailNotifications<tauri::Wry>>()
            .0
            .run_mobile_plugin_async::<serde_json::Value>(
                "dismissNotifications",
                serde_json::json!({ "threadIds": thread_ids }),
            )
            .await
            .map_err(|error| format!("Android notification dismissal failed: {error}"))?;
    }
    #[cfg(target_os = "macos")]
    {
        let _ = app;
        crate::utils::mac_notify::dismiss_threads(thread_ids);
    }
    #[cfg(not(any(target_os = "android", target_os = "macos")))]
    {
        let _ = (app, thread_ids);
    }
    Ok(())
}

/// Release routes queued by iOS before a listener existed.
///
/// A notification can be tapped while the process is cold; the Swift side holds
/// the route until the frontend says it is listening.
#[tauri::command]
pub fn mobile_notifications_frontend_ready() -> CommandResult<()> {
    #[cfg(target_os = "ios")]
    {
        let ready: unsafe extern "C" fn() =
            unsafe { ios_native_symbol(c"satchel_mobile_frontend_ready")? };
        unsafe { ready() };
    }
    Ok(())
}

#[cfg(target_os = "ios")]
unsafe fn ios_native_symbol<T: Copy>(name: &std::ffi::CStr) -> Result<T, String> {
    let symbol = libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr());
    if symbol.is_null() {
        return Err(format!(
            "iOS native bridge symbol `{}` is unavailable",
            name.to_string_lossy()
        ));
    }
    if std::mem::size_of::<T>() != std::mem::size_of_val(&symbol) {
        return Err("iOS native bridge function pointer has an unexpected size".to_string());
    }
    Ok(std::mem::transmute_copy(&symbol))
}

#[cfg(target_os = "ios")]
mod ios_events {
    use std::{
        collections::VecDeque,
        ffi::{c_char, CStr},
        sync::{Mutex, OnceLock},
    };

    use tauri::Emitter;

    static APP: OnceLock<tauri::AppHandle> = OnceLock::new();
    static PENDING: Mutex<VecDeque<serde_json::Value>> = Mutex::new(VecDeque::new());

    pub fn initialize(app: &tauri::AppHandle) {
        let _ = APP.set(app.clone());
        if let Ok(mut pending) = PENDING.lock() {
            while let Some(event) = pending.pop_front() {
                let _ = app.emit("mobile:notification-route", event);
            }
        }
    }

    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn satchel_mobile_event(json: *const c_char) {
        if json.is_null() {
            return;
        }
        let Ok(text) = CStr::from_ptr(json).to_str() else {
            return;
        };
        if text.len() > 1_048_576 {
            return;
        }
        let Ok(event) = serde_json::from_str::<serde_json::Value>(text) else {
            return;
        };
        if let Some(app) = APP.get() {
            let _ = app.emit("mobile:notification-route", event);
        } else if let Ok(mut pending) = PENDING.lock() {
            if pending.len() == 500 {
                pending.pop_front();
            }
            pending.push_back(event);
        }
    }
}

pub fn initialize_mobile_notification_bridge(app: &tauri::AppHandle) {
    #[cfg(target_os = "ios")]
    ios_events::initialize(app);
    #[cfg(not(target_os = "ios"))]
    let _ = app;
}
