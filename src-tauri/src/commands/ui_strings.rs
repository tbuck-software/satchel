use std::sync::{OnceLock, RwLock};

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::commands::error::CommandResult;

#[path = "native_ui_defaults.rs"]
mod native_ui_defaults;

/// Strings rendered by Rust or by operating-system UI that cannot use
/// vue-i18n directly. Generated English catalogue defaults cover the short
/// interval before the frontend sends its active catalogue.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/ui_strings/")
)]
pub struct UiStrings {
    pub lang: String,
    pub follow_system_locale: bool,
    pub tray_show: String,
    pub tray_hide: String,
    pub tray_quit: String,
    pub tray_briefing: String,
    pub dock_compose: String,
    pub dock_calendar: String,
    pub dock_sync: String,
    pub dock_settings: String,
    pub dock_close_other_windows: String,
    pub notification_archive: String,
    pub notification_read: String,
    pub notification_trash: String,
    pub notification_send_failed: String,
    pub notification_sync_failed: String,
    pub notification_follow_up_due: String,
    pub notification_no_subject: String,
    pub notification_channel_name: String,
    pub notification_channel_description: String,
    pub notification_new_message: String,
    pub notification_syncing: String,
    pub notification_reply: String,
    pub notification_summary: String,
    pub notification_background_sync_channel: String,
    pub notification_background_sync_description: String,
    pub notification_background_sync_text: String,
    pub notification_action_failed_title: String,
    pub notification_action_failed_message: String,
    /// Desktop quick actions fail before anything is saved, unlike Android's.
    pub notification_action_not_applied_message: String,
    pub compose_window_title: String,
    pub oauth_invalid_title: String,
    pub oauth_invalid_callback: String,
    pub oauth_invalid_state: String,
    pub oauth_cancelled_title: String,
    pub oauth_cancelled_message: String,
    pub oauth_missing_code: String,
    pub oauth_connected_title: String,
    pub oauth_connected_message: String,
    pub oauth_failed_title: String,
    pub oauth_failed_message: String,
    pub oauth_close_hint: String,
    pub oauth_provider_fallback: String,
}

impl Default for UiStrings {
    fn default() -> Self {
        native_ui_defaults::for_language(&system_language())
    }
}

#[cfg(target_os = "macos")]
fn system_language() -> String {
    objc2_foundation::NSLocale::currentLocale()
        .languageCode()
        .to_string()
}

#[cfg(target_os = "windows")]
fn system_language() -> String {
    use std::ptr::null_mut;
    use windows_sys::Win32::Globalization::{GetUserPreferredUILanguages, MUI_LANGUAGE_NAME};

    let mut language_count = 0;
    let mut buffer_length = 0;
    let measured = unsafe {
        GetUserPreferredUILanguages(
            MUI_LANGUAGE_NAME,
            &mut language_count,
            null_mut(),
            &mut buffer_length,
        )
    };
    if measured == 0 || buffer_length == 0 {
        return "en".to_string();
    }

    let mut languages = vec![0_u16; buffer_length as usize];
    let loaded = unsafe {
        GetUserPreferredUILanguages(
            MUI_LANGUAGE_NAME,
            &mut language_count,
            languages.as_mut_ptr(),
            &mut buffer_length,
        )
    };
    if loaded == 0 {
        return "en".to_string();
    }

    let first_language_length = languages
        .iter()
        .position(|code_unit| *code_unit == 0)
        .unwrap_or(languages.len());
    String::from_utf16_lossy(&languages[..first_language_length])
}

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
fn system_language() -> String {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .find_map(|name| std::env::var(name).ok().filter(|value| !value.is_empty()))
        .unwrap_or_else(|| "en".to_string())
}

static UI_STRINGS: OnceLock<RwLock<UiStrings>> = OnceLock::new();

fn shared() -> &'static RwLock<UiStrings> {
    UI_STRINGS.get_or_init(|| RwLock::new(UiStrings::default()))
}

pub fn current() -> UiStrings {
    shared()
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

fn apply_current_native_ui(_app: &AppHandle) -> CommandResult<()> {
    #[cfg(target_os = "android")]
    persist_android(&current())?;

    #[cfg(desktop)]
    crate::commands::tray::refresh_native_ui(_app)?;
    #[cfg(target_os = "macos")]
    crate::utils::mac_notify::refresh_category();

    Ok(())
}

#[tauri::command]
pub fn set_ui_strings(_app: AppHandle, mut strings: UiStrings) -> CommandResult<()> {
    strings.lang = match strings.lang.as_str() {
        "de" => "de",
        _ => "en",
    }
    .into();
    let previous = current();
    *shared()
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = strings;

    if let Err(error) = apply_current_native_ui(&_app) {
        *shared()
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = previous;
        let _ = apply_current_native_ui(&_app);
        return Err(error);
    }

    Ok(())
}

#[cfg(target_os = "android")]
fn persist_android(strings: &UiStrings) -> Result<(), String> {
    use std::mem::ManuallyDrop;

    use jni::{objects::JObject, objects::JValue, JavaVM};

    let android = ndk_context::android_context();
    let vm = unsafe { JavaVM::from_raw(android.vm().cast()) }
        .map_err(|error| format!("Android UI strings VM: {error}"))?;
    let mut env = vm
        .attach_current_thread()
        .map_err(|error| format!("Android UI strings thread: {error}"))?;
    // ndk-context owns this global Activity reference for the process lifetime.
    let context = ManuallyDrop::new(unsafe { JObject::from_raw(android.context().cast()) });
    let preference_name = JObject::from(
        env.new_string("satchel.ui-strings")
            .map_err(|error| format!("Android UI strings preference name: {error}"))?,
    );
    let preferences = env
        .call_method(
            &*context,
            "getSharedPreferences",
            "(Ljava/lang/String;I)Landroid/content/SharedPreferences;",
            &[JValue::Object(&preference_name), JValue::Int(0)],
        )
        .and_then(|value| value.l())
        .map_err(|error| format!("Android UI strings preferences: {error}"))?;
    let editor = env
        .call_method(
            preferences,
            "edit",
            "()Landroid/content/SharedPreferences$Editor;",
            &[],
        )
        .and_then(|value| value.l())
        .map_err(|error| format!("Android UI strings editor: {error}"))?;
    let key = JObject::from(
        env.new_string("strings")
            .map_err(|error| format!("Android UI strings key: {error}"))?,
    );
    let json = JObject::from(
        env.new_string(serde_json::to_string(strings).map_err(|error| error.to_string())?)
            .map_err(|error| format!("Android UI strings JSON: {error}"))?,
    );
    let editor = env
        .call_method(
            editor,
            "putString",
            "(Ljava/lang/String;Ljava/lang/String;)Landroid/content/SharedPreferences$Editor;",
            &[JValue::Object(&key), JValue::Object(&json)],
        )
        .and_then(|value| value.l())
        .map_err(|error| format!("Android UI strings write: {error}"))?;
    let committed = env
        .call_method(editor, "commit", "()Z", &[])
        .and_then(|value| value.z())
        .map_err(|error| format!("Android UI strings commit: {error}"))?;
    if !committed {
        return Err("Android UI strings could not be persisted".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::native_ui_defaults;

    #[test]
    fn generated_defaults_follow_the_system_language() {
        let english = native_ui_defaults::for_language("en-US");
        assert_eq!(english.lang, "en");
        assert_eq!(english.tray_hide, "Hide");

        let german = native_ui_defaults::for_language("de-DE");
        assert_eq!(german.lang, "de");
        assert_eq!(german.tray_hide, "Ausblenden");
        assert!(german.follow_system_locale);
    }
}
