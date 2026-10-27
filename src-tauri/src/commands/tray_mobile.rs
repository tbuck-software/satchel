use std::sync::atomic::AtomicBool;

pub use super::tray_mobile_dto::{TrayPreferencesDto, TraySnapshotDto, TrayThreadActionInput};
use sqlx::SqlitePool;
use tauri::{AppHandle, Manager};

use crate::commands::error::{CommandError, CommandResult};

#[derive(Debug)]
pub struct TrayPreferences {
    pub quit_on_close: AtomicBool,
}

impl Default for TrayPreferences {
    fn default() -> Self {
        Self {
            quit_on_close: AtomicBool::new(true),
        }
    }
}

pub fn install_tray(app: &AppHandle, _pending_thread_ids: Vec<String>) -> Result<(), String> {
    app.manage(TrayPreferences::default());
    Ok(())
}

pub(crate) fn show_main(_app: &AppHandle) {}

pub async fn open_unread_count(pool: &SqlitePool) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        &format!("SELECT COUNT(1)
         FROM threads
         WHERE workflow_state = 'inbox'
           AND unread_count > 0
           AND COALESCE(is_muted, 0) = 0
           AND {}
           AND EXISTS (
             SELECT 1
             FROM messages inbox_messages
             JOIN folders inbox_folders ON inbox_folders.id = inbox_messages.folder_id
             WHERE inbox_messages.thread_id = threads.id
               AND inbox_messages.is_read = 0
               AND (
                 LOWER(TRIM(inbox_folders.name)) = 'inbox'
                 OR (
                   LOWER(inbox_folders.folder_type) = 'inbox'
                   AND LOWER(TRIM(COALESCE(inbox_folders.full_path, inbox_folders.name))) NOT LIKE 'inbox.%'
                   AND LOWER(TRIM(COALESCE(inbox_folders.full_path, inbox_folders.name))) NOT LIKE 'inbox/%'
                 )
               )
           )", crate::db::account_visibility::visible_account_sql("threads.account_id")),
    )
    .fetch_one(pool)
    .await
}

pub async fn refresh_account_visibility(_app: &AppHandle, _pool: &SqlitePool) {}

pub async fn load_pending_thread_ids(_pool: &SqlitePool) -> Vec<String> {
    Vec::new()
}

pub async fn mark_new_mail(_app: &AppHandle, _pool: &SqlitePool, _thread_id: &str) {}

#[tauri::command]
pub async fn tray_preferences() -> CommandResult<TrayPreferencesDto> {
    Ok(TrayPreferencesDto {
        quit_on_close: true,
        start_minimized: false,
        autostart_enabled: false,
    })
}

#[tauri::command]
pub async fn set_quit_on_close(_enabled: bool) -> CommandResult<()> {
    Err(CommandError::unexpected(
        "quit-on-close is a desktop-only setting",
    ))
}

#[tauri::command]
pub async fn set_start_minimized(_enabled: bool) -> CommandResult<()> {
    Err(CommandError::unexpected(
        "start minimized is a desktop-only setting",
    ))
}

#[tauri::command]
pub async fn set_autostart(_enabled: bool) -> CommandResult<()> {
    Err(CommandError::unexpected(
        "autostart is managed by the mobile operating system",
    ))
}

#[tauri::command]
pub async fn tray_snapshot() -> CommandResult<TraySnapshotDto> {
    Ok(TraySnapshotDto::default())
}

#[tauri::command]
pub async fn open_tray_thread(_thread_id: String) -> CommandResult<()> {
    Err(CommandError::unexpected(
        "the tray is only available on desktop",
    ))
}

#[tauri::command]
pub async fn open_tray_destination(_destination: String) -> CommandResult<()> {
    Err(CommandError::unexpected(
        "the tray is only available on desktop",
    ))
}

#[tauri::command]
pub async fn act_on_tray_thread(_input: TrayThreadActionInput) -> CommandResult<()> {
    Err(CommandError::unexpected(
        "the tray is only available on desktop",
    ))
}
