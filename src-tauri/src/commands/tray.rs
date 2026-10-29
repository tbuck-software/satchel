use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex, OnceLock,
    },
};

use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};
use tauri::{
    image::Image,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, PhysicalPosition, Position, Rect, WebviewUrl,
    WebviewWindowBuilder,
};
use tauri_plugin_autostart::ManagerExt;

use crate::{
    commands::{
        error::{CommandError, CommandResult},
        workflow,
    },
    workflow::states::WorkflowState,
    AppState,
};

const TRAY_ID: &str = "satchel-tray";
/// Matched by `capabilities/tray-popover.json`.
pub(crate) const POPOVER_LABEL: &str = "tray-popover";
const PENDING_SETTING: &str = "tray.pending_new_thread_ids";
const MAX_PENDING_THREADS: usize = 20;
/// How many of them one popover has room for. Deliberately smaller than
/// `MAX_PENDING_THREADS`: the queue keeps what a later opening can still show,
/// and only what was shown is answered for.
const MAX_BRIEFING_THREADS: usize = 3;
const MAX_UPCOMING_EVENTS: usize = 3;
const POPOVER_WIDTH: f64 = 388.0;
const POPOVER_HEIGHT: f64 = 420.0;

#[derive(Debug)]
pub struct TrayPreferences {
    pub quit_on_close: AtomicBool,
    has_new_mail: AtomicBool,
    pending_thread_ids: Mutex<VecDeque<String>>,
}

impl TrayPreferences {
    fn new(pending_thread_ids: Vec<String>) -> Self {
        let pending_thread_ids: VecDeque<_> = pending_thread_ids
            .into_iter()
            .filter(|id| !id.trim().is_empty())
            .take(MAX_PENDING_THREADS)
            .collect();
        Self {
            quit_on_close: AtomicBool::new(false),
            has_new_mail: AtomicBool::new(!pending_thread_ids.is_empty()),
            pending_thread_ids: Mutex::new(pending_thread_ids),
        }
    }

    fn record_thread(&self, thread_id: &str) -> Vec<String> {
        let mut pending = self
            .pending_thread_ids
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        pending.retain(|id| id != thread_id);
        pending.push_front(thread_id.to_string());
        pending.truncate(MAX_PENDING_THREADS);
        pending.iter().cloned().collect()
    }

    fn snapshot(&self) -> Vec<String> {
        self.pending_thread_ids
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .cloned()
            .collect()
    }

    fn acknowledge(&self, acknowledged: &[String]) -> Vec<String> {
        let mut pending = self
            .pending_thread_ids
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        pending.retain(|id| !acknowledged.contains(id));
        pending.iter().cloned().collect()
    }

    fn clear(&self) -> bool {
        let mut pending = self
            .pending_thread_ids
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let changed = !pending.is_empty();
        pending.clear();
        changed
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/tray/"))]
pub struct TrayPreferencesDto {
    pub quit_on_close: bool,
    pub start_minimized: bool,
    pub autostart_enabled: bool,
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/tray/"))]
pub struct TrayThreadDto {
    pub id: String,
    pub sender_name: String,
    pub sender_address: String,
    pub subject: String,
    pub received_at: String,
    pub is_unread: bool,
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/tray/"))]
pub struct TrayCalendarEventDto {
    pub title: String,
    pub starts_at: String,
    pub ends_at: String,
    pub all_day: bool,
    pub location: Option<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/tray/"))]
pub struct TraySnapshotDto {
    pub new_threads: Vec<TrayThreadDto>,
    pub upcoming_events: Vec<TrayCalendarEventDto>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/tray/"))]
pub enum TrayThreadAction {
    Done,
    Waiting,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/tray/", optional_fields = nullable))]
pub struct TrayThreadActionInput {
    pub thread_id: String,
    pub action: TrayThreadAction,
}

pub fn install_tray(app: &AppHandle, pending_thread_ids: Vec<String>) -> Result<(), String> {
    let has_pending = !pending_thread_ids.is_empty();
    let prefs = TrayPreferences::new(pending_thread_ids);
    app.manage(prefs);

    create_tray_popover(app)?;

    let menu = native_menu(app)?;

    let app_for_handler = app.clone();
    let initial_icon = if has_pending {
        attention_tray_icon()
    } else {
        default_tray_icon()
    };
    let _tray = TrayIconBuilder::with_id(TRAY_ID)
        .icon(initial_icon.unwrap_or_else(|| Image::new_owned(vec![0; 4], 1, 1)))
        // The compact glyph should adapt to the menu-bar appearance. The
        // attention capsule deliberately carries its own white/brand colours,
        // matching the treatment used by apps such as Spark.
        .icon_as_template(!has_pending)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| match event.id.as_ref() {
            "show" => show_main(app),
            "hide" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                }
            }
            "quit" => {
                if let Some(prefs) = app.try_state::<TrayPreferences>() {
                    prefs.quit_on_close.store(true, Ordering::Relaxed);
                }
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(move |_tray, event| {
            if let TrayIconEvent::Click {
                rect,
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                toggle_tray_popover(&app_for_handler, rect);
            }
        })
        .build(app)
        .map_err(err_str)?;

    if let Some(window) = app.get_webview_window("main") {
        let _ = window.set_badge_count(None);
    }
    Ok(())
}

pub(crate) fn show_main(app: &AppHandle) {
    clear_new_mail(app);
    hide_tray_popover(app);
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn create_tray_popover(app: &AppHandle) -> Result<(), String> {
    if app.get_webview_window(POPOVER_LABEL).is_some() {
        return Ok(());
    }

    let strings = crate::commands::ui_strings::current();
    crate::utils::navigation::guard(
        WebviewWindowBuilder::new(app, POPOVER_LABEL, WebviewUrl::App("tray.html".into())),
        app,
    )
    .title(&strings.tray_briefing)
    .inner_size(POPOVER_WIDTH, POPOVER_HEIGHT)
    .resizable(false)
    .decorations(false)
    .always_on_top(true)
    .visible_on_all_workspaces(true)
    .skip_taskbar(true)
    // AppKit draws a rectangular native shadow around transparent,
    // decorationless windows. The CSS shell owns the rounded shadow.
    .shadow(false)
    .transparent(true)
    .visible(false)
    .focused(false)
    .build()
    .map_err(err_str)?;
    Ok(())
}

fn native_menu(app: &AppHandle) -> Result<Menu<tauri::Wry>, String> {
    let strings = crate::commands::ui_strings::current();
    let show_item =
        MenuItem::with_id(app, "show", &strings.tray_show, true, None::<&str>).map_err(err_str)?;
    let hide_item =
        MenuItem::with_id(app, "hide", &strings.tray_hide, true, None::<&str>).map_err(err_str)?;
    let quit_item =
        MenuItem::with_id(app, "quit", &strings.tray_quit, true, None::<&str>).map_err(err_str)?;
    Menu::with_items(app, &[&show_item, &hide_item, &quit_item]).map_err(err_str)
}

/// Rebuild native menu items after the frontend changes locale. IDs stay
/// stable, so the tray handler installed at startup needs no replacement.
pub fn refresh_native_ui(app: &AppHandle) -> Result<(), String> {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        tray.set_menu(Some(native_menu(app)?)).map_err(err_str)?;
    }
    if let Some(window) = app.get_webview_window(POPOVER_LABEL) {
        window
            .set_title(&crate::commands::ui_strings::current().tray_briefing)
            .map_err(err_str)?;
    }
    Ok(())
}

fn toggle_tray_popover(app: &AppHandle, rect: Rect) {
    let Some(window) = app.get_webview_window(POPOVER_LABEL) else {
        return;
    };
    if window.is_visible().unwrap_or(false) {
        let _ = window.hide();
        return;
    }

    position_tray_popover(app, &window, rect);
    let _ = window.show();
    let _ = window.set_focus();
}

fn position_tray_popover(app: &AppHandle, window: &tauri::WebviewWindow, rect: Rect) {
    // tray-icon already reports a physical rectangle on macOS. Passing its
    // physical click coordinates to `monitor_from_point` is incorrect there:
    // Tao compares them against CoreGraphics' unscaled display coordinates,
    // which can select another display in mixed-DPI setups.
    let anchor = rect.position.to_physical::<i32>(1.0);
    let anchor_size = rect.size.to_physical::<u32>(1.0);
    let anchor_x = anchor.x as i64 + anchor_size.width as i64 / 2;
    let anchor_y = anchor.y as i64 + anchor_size.height as i64 / 2;
    let Ok(monitors) = app.available_monitors() else {
        return;
    };
    let Some(monitor) = monitors
        .into_iter()
        .min_by_key(|monitor| distance_to_monitor_squared(anchor_x, anchor_y, monitor))
    else {
        return;
    };
    let scale = monitor.scale_factor();
    // Do not read the hidden window's current physical size: it may still be
    // attached to a different monitor and therefore use the wrong scale.
    let window_size = tauri::PhysicalSize::new(
        (POPOVER_WIDTH * scale).round() as u32,
        (POPOVER_HEIGHT * scale).round() as u32,
    );
    let work = monitor.work_area();
    let gap = (6.0 * scale).round() as i32;
    let margin = (8.0 * scale).round() as i32;

    let desired_x = anchor.x + anchor_size.width as i32 / 2 - window_size.width as i32 / 2;
    let min_x = work.position.x + margin;
    let max_x = work.position.x + work.size.width as i32 - window_size.width as i32 - margin;
    let x = desired_x.clamp(min_x, max_x.max(min_x));

    let below = anchor.y + anchor_size.height as i32 + gap;
    let above = anchor.y - window_size.height as i32 - gap;
    let work_bottom = work.position.y + work.size.height as i32;
    let y = if below + window_size.height as i32 <= work_bottom {
        below
    } else {
        above.max(work.position.y + margin)
    };

    let _ = window.set_position(Position::Physical(PhysicalPosition::new(x, y)));
}

fn distance_to_monitor_squared(x: i64, y: i64, monitor: &tauri::Monitor) -> i128 {
    let position = monitor.position();
    let size = monitor.size();
    let left = position.x as i64;
    let top = position.y as i64;
    let right = left + size.width as i64;
    let bottom = top + size.height as i64;
    let dx = if x < left {
        left - x
    } else if x > right {
        x - right
    } else {
        0
    };
    let dy = if y < top {
        top - y
    } else if y > bottom {
        y - bottom
    } else {
        0
    };
    dx as i128 * dx as i128 + dy as i128 * dy as i128
}

fn hide_tray_popover(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(POPOVER_LABEL) {
        let _ = window.hide();
    }
}

fn default_tray_icon() -> Option<Image<'static>> {
    let bytes = include_bytes!("../../icons/trayTemplate.png");
    Image::from_bytes(bytes).ok()
}

/// A wider white capsule with the regular Satchel glyph punched out of it.
///
/// The transparent glyph lets the current menu-bar colour show through while
/// the non-template white surround preserves the attention treatment.
fn attention_tray_icon() -> Option<Image<'static>> {
    let glyph = default_tray_icon()?;
    // tray-icon renders every macOS status image at 18 pt high and preserves
    // its aspect ratio. Keep a 2x raster here: the 32 px source glyph is 16 pt,
    // while 8 pt of horizontal padding on either side makes the capsule 32 pt
    // wide without hard-coding its final width.
    const RASTER_SCALE: u32 = 2;
    const DISPLAY_HEIGHT: u32 = 18;
    const HORIZONTAL_PADDING: u32 = 8 * RASTER_SCALE;
    const CORNER_RADIUS: f32 = 4.0 * RASTER_SCALE as f32;
    const HEIGHT: u32 = DISPLAY_HEIGHT * RASTER_SCALE;
    let width = glyph.width() + 2 * HORIZONTAL_PADDING;
    let glyph_x = HORIZONTAL_PADDING;
    let glyph_y = (HEIGHT.saturating_sub(glyph.height())) / 2;
    let mut rgba = vec![0_u8; (width * HEIGHT * 4) as usize];

    for y in 0..HEIGHT {
        for x in 0..width {
            let index = ((y * width + x) * 4) as usize;
            let pill_alpha = rounded_capsule_alpha(x, y, width, HEIGHT, CORNER_RADIUS);
            let glyph_alpha = if (glyph_x..glyph_x + glyph.width()).contains(&x)
                && (glyph_y..glyph_y + glyph.height()).contains(&y)
            {
                let glyph_index =
                    (((y - glyph_y) * glyph.width() + (x - glyph_x)) * 4 + 3) as usize;
                glyph.rgba()[glyph_index]
            } else {
                0
            };
            rgba[index..index + 3].fill(255);
            rgba[index + 3] = pill_alpha.saturating_sub(glyph_alpha);
        }
    }
    Some(Image::new_owned(rgba, width, HEIGHT))
}

fn rounded_capsule_alpha(x: u32, y: u32, width: u32, height: u32, radius: f32) -> u8 {
    const SAMPLES: [(f32, f32); 4] = [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)];
    let left = 0.0_f32;
    let top = 0.0_f32;
    let right = width as f32;
    let bottom = height as f32;
    let mut inside = 0_u8;

    for (offset_x, offset_y) in SAMPLES {
        let px = x as f32 + offset_x;
        let py = y as f32 + offset_y;
        let nearest_x = px.clamp(left + radius, right - radius);
        let nearest_y = py.clamp(top + radius, bottom - radius);
        let dx = px - nearest_x;
        let dy = py - nearest_y;
        if px >= left
            && px <= right
            && py >= top
            && py <= bottom
            && dx * dx + dy * dy <= radius * radius
        {
            inside += 1;
        }
    }
    ((inside as u16 * 255) / SAMPLES.len() as u16) as u8
}

fn err_str<E: std::fmt::Display>(err: E) -> String {
    err.to_string()
}

/// Count actionable unread conversations in the real primary inbox.
///
/// The stored thread unread count intentionally survives `done`, but a done
/// conversation must not contribute to the app badge. Likewise, IMAP folders
/// below `INBOX.*` are regular folders, not the primary inbox.
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

/// Restore the unseen thread ids before the tray is installed. The list is
/// intentionally tiny: it is an attention cursor, not another inbox index.
pub async fn load_pending_thread_ids(pool: &SqlitePool) -> Vec<String> {
    let raw = sqlx::query_scalar::<_, String>("SELECT value FROM app_settings WHERE key = ?")
        .bind(PENDING_SETTING)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
    let ids = raw
        .and_then(|value| serde_json::from_str::<Vec<String>>(&value).ok())
        .unwrap_or_default()
        .into_iter()
        .filter(|id| !id.trim().is_empty() && id.len() <= 512)
        .take(MAX_PENDING_THREADS)
        .collect::<Vec<_>>();
    visible_pending_thread_ids(pool, &ids)
        .await
        .unwrap_or_default()
}

/// Record genuinely new mail, but stay quiet while the user is already looking
/// at Satchel. Unlike unread state this survives filing and read-flag changes.
pub async fn mark_new_mail(app: &AppHandle, pool: &SqlitePool, thread_id: &str) {
    if thread_id.trim().is_empty() || thread_id.len() > 512 || user_is_viewing_satchel(app) {
        return;
    }
    let visible: bool = sqlx::query_scalar(&format!(
        "SELECT EXISTS (SELECT 1 FROM threads WHERE id = ? AND {})",
        crate::db::account_visibility::visible_account_sql("threads.account_id")
    ))
    .bind(thread_id)
    .fetch_one(pool)
    .await
    .unwrap_or(false);
    if !visible {
        return;
    }
    let Some(state) = app.try_state::<TrayPreferences>() else {
        return;
    };
    state.record_thread(thread_id);
    set_attention(app, true);
    if let Err(err) = persist_current_pending_thread_ids(app, pool).await {
        tracing::warn!(%err, "failed to persist tray attention state");
    }
}

async fn visible_pending_thread_ids(
    pool: &SqlitePool,
    ids: &[String],
) -> Result<Vec<String>, String> {
    sqlx::query_scalar(&format!(
        "SELECT threads.id FROM json_each(?) pending JOIN threads ON threads.id = pending.value
         WHERE {} ORDER BY CAST(pending.key AS INTEGER)",
        crate::db::account_visibility::visible_account_sql("threads.account_id")
    ))
    .bind(serde_json::to_string(ids).map_err(|error| error.to_string())?)
    .fetch_all(pool)
    .await
    .map_err(|error| error.to_string())
}

pub async fn refresh_account_visibility(app: &AppHandle, pool: &SqlitePool) {
    let ids = app
        .try_state::<TrayPreferences>()
        .map(|state| state.snapshot())
        .unwrap_or_default();
    match visible_pending_thread_ids(pool, &ids).await {
        Ok(ids) => set_attention(app, !ids.is_empty()),
        Err(error) => tracing::warn!(%error, "could not refresh visible tray attention"),
    }
}

fn user_is_viewing_satchel(app: &AppHandle) -> bool {
    ["main", POPOVER_LABEL].into_iter().any(|label| {
        app.get_webview_window(label).is_some_and(|window| {
            window.is_visible().unwrap_or(false) && window.is_focused().unwrap_or(false)
        })
    })
}

pub fn clear_new_mail(app: &AppHandle) {
    let Some(state) = app.try_state::<TrayPreferences>() else {
        return;
    };
    let changed = state.clear();
    set_attention(app, false);
    if changed {
        let app = app.clone();
        let pool = app.state::<SqlitePool>().inner().clone();
        tauri::async_runtime::spawn(async move {
            if let Err(err) = persist_current_pending_thread_ids(&app, &pool).await {
                tracing::warn!(%err, "failed to clear persisted tray attention state");
            }
        });
    }
}

fn set_attention(app: &AppHandle, enabled: bool) {
    let Some(state) = app.try_state::<TrayPreferences>() else {
        return;
    };
    if state.has_new_mail.swap(enabled, Ordering::Relaxed) == enabled {
        return;
    }
    let icon = if enabled {
        attention_tray_icon()
    } else {
        default_tray_icon()
    };
    if let (Some(tray), Some(icon)) = (app.tray_by_id(TRAY_ID), icon) {
        let _ = tray.set_title(None::<String>);
        let _ = tray.set_icon(Some(icon));
        let _ = tray.set_icon_as_template(!enabled);
    }
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.set_badge_count(None);
    }
}

async fn persist_pending_thread_ids(pool: &SqlitePool, ids: &[String]) -> Result<(), String> {
    let now = chrono::Utc::now().to_rfc3339();
    let value = serde_json::to_string(ids).map_err(|err| err.to_string())?;
    write_setting(pool, PENDING_SETTING, &value, &now).await
}

fn tray_persistence_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

async fn persist_current_pending_thread_ids(
    app: &AppHandle,
    pool: &SqlitePool,
) -> Result<(), String> {
    let _guard = tray_persistence_lock().lock().await;
    let ids = app
        .try_state::<TrayPreferences>()
        .map(|state| state.snapshot())
        .unwrap_or_default();
    persist_pending_thread_ids(pool, &ids).await
}

/// The prefix of the pending queue one popover opening shows and answers for.
///
/// A thread whose row has meanwhile been deleted still counts as briefed —
/// otherwise it would sit at the head of the queue forever and push everything
/// behind it out of reach.
fn briefing_slice(pending: &[String]) -> &[String] {
    &pending[..pending.len().min(MAX_BRIEFING_THREADS)]
}

async fn load_tray_threads(
    pool: &SqlitePool,
    thread_ids: &[String],
) -> Result<Vec<TrayThreadDto>, String> {
    let mut rows = Vec::new();
    for thread_id in thread_ids {
        let row = sqlx::query_as::<_, TrayThreadDto>(
            &format!("SELECT
                threads.id,
                COALESCE(NULLIF(TRIM(latest.from_name), ''), NULLIF(TRIM(latest.from_address), ''), '') AS sender_name,
                COALESCE(latest.from_address, '') AS sender_address,
                COALESCE(NULLIF(TRIM(threads.subject), ''), '') AS subject,
                threads.last_message_date AS received_at,
                threads.unread_count > 0 AS is_unread
             FROM threads
             LEFT JOIN messages latest ON latest.id = (
                 SELECT messages.id
                 FROM messages
                 WHERE messages.thread_id = threads.id
                 ORDER BY messages.date DESC, messages.id DESC
                 LIMIT 1
             )
             WHERE threads.id = ? AND {}", crate::db::account_visibility::visible_account_sql("threads.account_id")),
        )
        .bind(thread_id)
        .fetch_optional(pool)
        .await
        .map_err(|err| err.to_string())?;
        if let Some(row) = row {
            rows.push(row);
        }
    }
    Ok(rows)
}

async fn load_upcoming_calendar_events(
    pool: &SqlitePool,
) -> Result<Vec<TrayCalendarEventDto>, crate::db::calendar::CalendarError> {
    load_upcoming_calendar_events_at(pool, chrono::Utc::now()).await
}

async fn load_upcoming_calendar_events_at(
    pool: &SqlitePool,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<Vec<TrayCalendarEventDto>, crate::db::calendar::CalendarError> {
    let now_text = now.to_rfc3339();
    let rows = crate::db::calendar::CalendarRepository::new(pool)
        .upcoming(&now_text)
        .await?;

    let mut events = Vec::new();
    for row in rows {
        // DATE recurrences use nominal calendar boundaries, so compare against
        // the viewer's local date/time rather than the current UTC date/time.
        let recurrence_now = if row.all_day {
            now.with_timezone(&chrono::Local).naive_local().and_utc()
        } else {
            now
        };
        let recurrences = row
            .raw_ics
            .as_deref()
            .map(|raw| {
                crate::email::calendar::upcoming_recurrence_occurrences(
                    raw,
                    recurrence_now,
                    MAX_UPCOMING_EVENTS,
                )
            })
            .unwrap_or_default();
        if !recurrences.is_empty() {
            events.extend(
                recurrences
                    .into_iter()
                    .map(|occurrence| TrayCalendarEventDto {
                        title: row.title.clone(),
                        starts_at: occurrence.starts_at,
                        ends_at: occurrence.ends_at,
                        all_day: row.all_day,
                        location: row.location.clone(),
                    }),
            );
            continue;
        }

        let Some(start) = parse_calendar_timestamp(&row.starts_at, row.all_day) else {
            continue;
        };
        let Some(end) = parse_calendar_timestamp(&row.ends_at, row.all_day) else {
            continue;
        };
        let duration = if row.all_day {
            end.with_timezone(&chrono::Local).date_naive()
                - start.with_timezone(&chrono::Local).date_naive()
        } else {
            end - start
        };
        if end < now || end < start || duration > chrono::Duration::days(7) {
            continue;
        }
        events.push(TrayCalendarEventDto {
            title: row.title,
            starts_at: row.starts_at,
            ends_at: row.ends_at,
            all_day: row.all_day,
            location: row.location,
        });
    }

    // Keep DATE and legacy mirrors equivalent at the tray boundary. Sorting
    // their raw strings would compare nominal dates against timed UTC values.
    for event in &mut events {
        if event.all_day {
            event.starts_at.truncate(10);
            event.ends_at.truncate(10);
        }
    }
    events.sort_by_cached_key(|event| parse_calendar_timestamp(&event.starts_at, event.all_day));
    events.dedup_by(|left, right| {
        left.title == right.title
            && left.starts_at == right.starts_at
            && left.ends_at == right.ends_at
    });
    events.truncate(MAX_UPCOMING_EVENTS);
    Ok(events)
}

fn parse_calendar_timestamp(value: &str, all_day: bool) -> Option<chrono::DateTime<chrono::Utc>> {
    use chrono::TimeZone;
    if all_day {
        let midnight = chrono::NaiveDate::parse_from_str(value.get(..10)?, "%Y-%m-%d")
            .ok()?
            .and_hms_opt(0, 0, 0)?;
        // Some zones skip midnight. Use the first valid local minute of the
        // date rather than dropping an otherwise valid all-day event.
        return (0..24 * 60).find_map(|minute| {
            let local = midnight.checked_add_signed(chrono::Duration::minutes(minute))?;
            chrono::Local
                .from_local_datetime(&local)
                .earliest()
                .map(|date| date.with_timezone(&chrono::Utc))
        });
    }
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|value| value.with_timezone(&chrono::Utc))
}

#[tauri::command]
pub async fn tray_snapshot(
    app: AppHandle,
    pool: tauri::State<'_, crate::db::ReadPool>,
    write_pool: tauri::State<'_, SqlitePool>,
) -> CommandResult<TraySnapshotDto> {
    let Some(state) = app.try_state::<TrayPreferences>() else {
        return Ok(TraySnapshotDto::default());
    };
    let pending = visible_pending_thread_ids(pool.inner().as_ref(), &state.snapshot()).await?;
    let briefed = briefing_slice(&pending);
    let new_threads = load_tray_threads(pool.inner().as_ref(), briefed).await?;
    let upcoming_events = load_upcoming_calendar_events(pool.inner().as_ref()).await?;

    // Only what the briefing had room for. Acknowledging the whole queue meant
    // that twelve new mails turned into three senders, once, and the
    // attention dot went out over the nine nobody ever saw.
    state.acknowledge(briefed);
    persist_current_pending_thread_ids(&app, write_pool.inner()).await?;
    refresh_account_visibility(&app, write_pool.inner()).await;

    Ok(TraySnapshotDto {
        new_threads,
        upcoming_events,
    })
}

#[tauri::command]
pub async fn open_tray_thread(app: AppHandle, thread_id: String) -> CommandResult<()> {
    let thread_id = thread_id.trim();
    if thread_id.is_empty() || thread_id.len() > 512 {
        return Err(CommandError::new("backend.errors.trayThreadInvalid"));
    }
    hide_tray_popover(&app);
    show_main(&app);
    app.emit(
        "mail:focus-thread",
        serde_json::json!({ "thread_id": thread_id }),
    )
    .map_err(CommandError::unexpected)
}

#[tauri::command]
pub async fn open_tray_destination(app: AppHandle, destination: String) -> CommandResult<()> {
    if !matches!(
        destination.as_str(),
        "inbox" | "calendar" | "compose" | "settings" | "calendar_settings"
    ) {
        return Err(CommandError::new("backend.errors.trayDestinationInvalid"));
    }
    hide_tray_popover(&app);
    show_main(&app);
    app.emit(
        "tray:navigate",
        serde_json::json!({ "destination": destination }),
    )
    .map_err(CommandError::unexpected)
}

#[tauri::command]
pub async fn act_on_tray_thread(app: AppHandle, input: TrayThreadActionInput) -> CommandResult<()> {
    let thread_id = input.thread_id.trim();
    if thread_id.is_empty() || thread_id.len() > 512 {
        return Err(CommandError::new("backend.errors.trayThreadInvalid"));
    }

    let pool = app.state::<SqlitePool>();
    let current: Option<String> =
        sqlx::query_scalar("SELECT workflow_state FROM threads WHERE id = ?")
            .bind(thread_id)
            .fetch_optional(pool.inner())
            .await
            .map_err(CommandError::unexpected)?;
    let from = current
        .as_deref()
        .and_then(WorkflowState::from_db_value)
        .ok_or_else(|| CommandError::new("backend.errors.trayMessageNotFound"))?;
    let to = match input.action {
        TrayThreadAction::Done => WorkflowState::Done,
        TrayThreadAction::Waiting => WorkflowState::Waiting,
    };

    if from == to {
        return Ok(());
    }

    workflow::transition_thread_inner(
        pool.inner(),
        app.state::<AppState>().inner(),
        workflow::TransitionInput {
            thread_id: thread_id.to_string(),
            from,
            to,
            snooze_until: None,
        },
    )
    .await
    .map_err(CommandError::from)?;
    app.emit(
        "thread:updated",
        serde_json::json!({ "thread_id": thread_id }),
    )
    .map_err(CommandError::unexpected)
}

/// The launch agent decides; `app_settings` only remembers what we last set.
///
/// Removing Satchel from the login items outside the app left the switch on,
/// and turning an already-on switch on again does nothing — so the only way
/// back was off and on. An agent that cannot be queried falls back to the
/// stored value, which is still better than claiming "off".
pub(crate) fn resolve_autostart(agent: Option<bool>, stored: bool) -> bool {
    agent.unwrap_or(stored)
}

#[tauri::command]
pub async fn tray_preferences(
    app: tauri::AppHandle,
    pool: tauri::State<'_, crate::db::ReadPool>,
    write_pool: tauri::State<'_, SqlitePool>,
) -> CommandResult<TrayPreferencesDto> {
    let quit_on_close = read_bool(pool.inner().as_ref(), "tray.quit_on_close")
        .await
        .unwrap_or(false);
    let start_minimized = read_bool(pool.inner().as_ref(), "tray.start_minimized")
        .await
        .unwrap_or(false);
    let stored_autostart = read_bool(pool.inner().as_ref(), "autostart.enabled")
        .await
        .unwrap_or(false);
    let agent_autostart = app.autolaunch().is_enabled().ok();
    let autostart_enabled = resolve_autostart(agent_autostart, stored_autostart);
    if autostart_enabled != stored_autostart {
        let now = chrono::Utc::now().to_rfc3339();
        write_setting(
            write_pool.inner(),
            "autostart.enabled",
            if autostart_enabled { "true" } else { "false" },
            &now,
        )
        .await
        .map_err(CommandError::from)?;
    }
    Ok(TrayPreferencesDto {
        quit_on_close,
        start_minimized,
        autostart_enabled,
    })
}

#[tauri::command]
pub async fn set_quit_on_close(
    app: tauri::AppHandle,
    pool: tauri::State<'_, SqlitePool>,
    enabled: bool,
) -> CommandResult<()> {
    if let Some(state) = app.try_state::<TrayPreferences>() {
        state.quit_on_close.store(enabled, Ordering::Relaxed);
    }
    let now = chrono::Utc::now().to_rfc3339();
    write_setting(
        pool.inner(),
        "tray.quit_on_close",
        if enabled { "true" } else { "false" },
        &now,
    )
    .await
    .map_err(CommandError::from)
}

#[tauri::command]
pub async fn set_autostart(
    app: tauri::AppHandle,
    pool: tauri::State<'_, SqlitePool>,
    enabled: bool,
) -> CommandResult<()> {
    let manager = app.autolaunch();
    if enabled {
        manager.enable().map_err(CommandError::unexpected)?;
    } else {
        manager.disable().map_err(CommandError::unexpected)?;
    }
    let now = chrono::Utc::now().to_rfc3339();
    write_setting(
        pool.inner(),
        "autostart.enabled",
        if enabled { "true" } else { "false" },
        &now,
    )
    .await
    .map_err(CommandError::from)
}

/// Passed by the launch agent, so a login start can be told from a user start.
pub const AUTOSTART_ARG: &str = "--autostarted";

pub fn launched_by_autostart(args: impl IntoIterator<Item = String>) -> bool {
    args.into_iter().any(|arg| arg == AUTOSTART_ARG)
}

/// Whether a login start should leave the window in the tray.
pub async fn start_minimized_enabled(pool: &SqlitePool) -> bool {
    read_bool(pool, "tray.start_minimized")
        .await
        .unwrap_or(false)
}

#[tauri::command]
pub async fn set_start_minimized(
    app: tauri::AppHandle,
    pool: tauri::State<'_, SqlitePool>,
    enabled: bool,
) -> CommandResult<()> {
    let now = chrono::Utc::now().to_rfc3339();
    write_setting(
        pool.inner(),
        "tray.start_minimized",
        if enabled { "true" } else { "false" },
        &now,
    )
    .await
    .map_err(CommandError::from)?;
    // A launch agent registered before AUTOSTART_ARG existed starts without
    // it; registering again writes the current arguments.
    let manager = app.autolaunch();
    if enabled && manager.is_enabled().unwrap_or(false) {
        manager.enable().map_err(CommandError::unexpected)?;
    }
    Ok(())
}

async fn read_bool(pool: &SqlitePool, key: &str) -> Option<bool> {
    sqlx::query_scalar::<_, String>("SELECT value FROM app_settings WHERE key = ?")
        .bind(key)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .map(|raw| {
            matches!(
                raw.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes"
            )
        })
}

async fn write_setting(pool: &SqlitePool, key: &str, value: &str, now: &str) -> Result<(), String> {
    sqlx::query(
        "INSERT INTO app_settings (key, value, updated_at) VALUES (?, ?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
    )
    .bind(key)
    .bind(value)
    .bind(now)
    .execute(pool)
    .await
    .map_err(|err| err.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use sqlx::sqlite::SqlitePoolOptions;

    use super::{
        attention_tray_icon, briefing_slice, load_tray_threads, load_upcoming_calendar_events,
        load_upcoming_calendar_events_at, open_unread_count, resolve_autostart, TrayPreferences,
        MAX_BRIEFING_THREADS, MAX_PENDING_THREADS,
    };
    use crate::{
        commands::workflow, sync::remote_mutations::RemoteMutationPayload,
        workflow::states::WorkflowState, AppState,
    };

    #[test]
    fn only_a_launch_agent_start_counts_as_an_autostart() {
        let args = |list: &[&str]| list.iter().map(|arg| arg.to_string()).collect::<Vec<_>>();

        assert!(super::launched_by_autostart(args(&[
            "satchel",
            "--autostarted"
        ])));
        assert!(!super::launched_by_autostart(args(&["satchel"])));
        assert!(!super::launched_by_autostart(args(&[
            "satchel",
            "/tmp/invite.ics"
        ])));
    }

    #[tokio::test]
    async fn start_minimized_is_read_from_the_stored_setting() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::db::init(&pool).await.unwrap();
        assert!(!super::start_minimized_enabled(&pool).await);

        super::write_setting(&pool, "tray.start_minimized", "true", "2026-01-01")
            .await
            .unwrap();

        assert!(super::start_minimized_enabled(&pool).await);
    }

    #[test]
    fn attention_state_keeps_newest_threads_and_acknowledges_only_seen_ids() {
        let state = TrayPreferences::new(vec!["older".into()]);

        assert_eq!(state.record_thread("new"), vec!["new", "older"]);
        assert_eq!(state.record_thread("older"), vec!["older", "new"]);
        assert_eq!(state.acknowledge(&["older".into()]), vec!["new"]);
        assert!(state.clear());
        assert!(state.snapshot().is_empty());
    }

    /// Twelve mails over lunch used to become three senders, once: the popover
    /// showed three and acknowledged all twelve, so the attention mark went out
    /// over nine the user never saw.
    #[test]
    fn opening_the_popover_answers_only_for_the_threads_it_showed() {
        let queued: Vec<String> = (0..MAX_PENDING_THREADS).map(|i| format!("t{i}")).collect();
        let state = TrayPreferences::new(queued.clone());

        let pending = state.snapshot();
        let briefed = briefing_slice(&pending);
        assert_eq!(briefed, &queued[..MAX_BRIEFING_THREADS]);

        let remaining = state.acknowledge(briefed);
        assert_eq!(remaining.len(), MAX_PENDING_THREADS - MAX_BRIEFING_THREADS);
        assert!(!remaining.is_empty(), "the icon keeps its attention mark");
        assert_eq!(remaining[0], queued[MAX_BRIEFING_THREADS]);
    }

    #[test]
    fn a_queue_that_fits_in_one_briefing_is_emptied_by_it() {
        let state = TrayPreferences::new(vec!["a".into(), "b".into(), "c".into()]);

        let pending = state.snapshot();
        assert!(state.acknowledge(briefing_slice(&pending)).is_empty());
    }

    /// The launch agent is the truth; the row only remembers what we set.
    #[test]
    fn autostart_follows_the_launch_agent_and_falls_back_to_the_stored_row() {
        assert!(
            !resolve_autostart(Some(false), true),
            "removed outside the app"
        );
        assert!(
            resolve_autostart(Some(true), false),
            "added outside the app"
        );
        assert!(resolve_autostart(None, true), "agent unreachable");
        assert!(!resolve_autostart(None, false));
    }

    #[test]
    fn attention_icon_is_a_padded_white_capsule_with_a_transparent_glyph() {
        let icon = attention_tray_icon().expect("attention icon");
        // 2x raster: 18 pt high, with a 16 pt glyph canvas and 8 pt padding
        // on both sides. tray-icon therefore renders this as 32 x 18 pt.
        assert_eq!((icon.width(), icon.height()), (64, 36));
        let glyph = ((12 * icon.width() + 32) * 4) as usize;
        let fill = ((18 * icon.width() + 8) * 4) as usize;
        assert_eq!(&icon.rgba()[fill..fill + 4], &[255, 255, 255, 255]);
        assert_eq!(icon.rgba()[glyph + 3], 0);
    }

    #[tokio::test]
    async fn badge_counts_only_unread_open_primary_inbox_threads() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite");
        sqlx::query(
            "CREATE TABLE app_settings (key TEXT PRIMARY KEY, value TEXT);
             CREATE TABLE threads (id TEXT PRIMARY KEY, account_id TEXT DEFAULT 'acc', workflow_state TEXT, unread_count INTEGER, is_muted INTEGER);
             CREATE TABLE folders (id TEXT PRIMARY KEY, name TEXT, full_path TEXT, folder_type TEXT);
             CREATE TABLE messages (id TEXT PRIMARY KEY, thread_id TEXT, folder_id TEXT, is_read INTEGER);",
        )
        .execute(&pool)
        .await
        .expect("schema");

        for (id, name, path, kind) in [
            ("root", "INBOX", "INBOX", "inbox"),
            ("localized", "Posteingang", "Posteingang", "inbox"),
            ("nested", "INBOX.Archive", "INBOX.Archive", "inbox"),
            ("archive", "Archive", "Archive", "archive"),
        ] {
            sqlx::query(
                "INSERT INTO folders (id, name, full_path, folder_type) VALUES (?, ?, ?, ?)",
            )
            .bind(id)
            .bind(name)
            .bind(path)
            .bind(kind)
            .execute(&pool)
            .await
            .expect("folder");
        }

        for (id, state, unread, muted, folder, is_read) in [
            ("open", "inbox", 3, 0, "root", 0),
            ("localized", "inbox", 1, 0, "localized", 0),
            ("done", "done", 1, 0, "root", 0),
            ("read", "inbox", 0, 0, "root", 1),
            ("muted", "inbox", 1, 1, "root", 0),
            ("nested", "inbox", 1, 0, "nested", 0),
            ("archived", "inbox", 1, 0, "archive", 0),
        ] {
            sqlx::query("INSERT INTO threads (id, workflow_state, unread_count, is_muted) VALUES (?, ?, ?, ?)")
                .bind(id)
                .bind(state)
                .bind(unread)
                .bind(muted)
                .execute(&pool)
                .await
                .expect("thread");
            sqlx::query(
                "INSERT INTO messages (id, thread_id, folder_id, is_read) VALUES (?, ?, ?, ?)",
            )
            .bind(format!("message-{id}"))
            .bind(id)
            .bind(folder)
            .bind(is_read)
            .execute(&pool)
            .await
            .expect("message");
        }

        assert_eq!(open_unread_count(&pool).await.expect("badge count"), 2);
        sqlx::query(
            r#"INSERT INTO app_settings VALUES ('account.visibility.acc', '{"mode":"manual"}')"#,
        )
        .execute(&pool)
        .await
        .unwrap();
        assert_eq!(open_unread_count(&pool).await.unwrap(), 0);
        assert!(
            super::visible_pending_thread_ids(&pool, &["open".into(), "localized".into()])
                .await
                .unwrap()
                .is_empty()
        );
        sqlx::query("DELETE FROM app_settings")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(open_unread_count(&pool).await.unwrap(), 2);
        assert_eq!(
            super::visible_pending_thread_ids(&pool, &["localized".into(), "open".into()])
                .await
                .unwrap(),
            vec!["localized", "open"]
        );
    }

    #[tokio::test]
    async fn tray_done_marks_the_thread_read_and_queues_the_provider_flag() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite");
        sqlx::migrate!("../crates/satchel-core/src/db/migrations")
            .run(&pool)
            .await
            .expect("migrations");
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO accounts
             (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
             VALUES ('account', 'Me', 'me@example.com', 'imap', '{}', '', ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("account");
        sqlx::query(
            "INSERT INTO folders
             (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at)
             VALUES ('inbox', 'account', 'INBOX', 'INBOX', 'inbox', 'INBOX', ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("folder");
        sqlx::query(
            "INSERT INTO threads
             (id, account_id, subject, last_message_date, unread_count, workflow_state, created_at, updated_at)
             VALUES ('thread', 'account', 'Subject', ?, 1, 'inbox', ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("thread");
        sqlx::query(
            "INSERT INTO messages
             (id, account_id, folder_id, thread_id, from_address, to_addresses, date, is_read, remote_id, created_at, updated_at)
             VALUES ('message', 'account', 'inbox', 'thread', 'sender@example.com', '[]', ?, 0, '101', ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("message");

        workflow::transition_thread_inner(
            &pool,
            &AppState::default(),
            workflow::TransitionInput {
                thread_id: "thread".into(),
                from: WorkflowState::Inbox,
                to: WorkflowState::Done,
                snooze_until: None,
            },
        )
        .await
        .expect("done");

        let projection: (i64, i64) = sqlx::query_as(
            "SELECT unread_count,
                    (SELECT is_read FROM messages WHERE id = 'message')
             FROM threads WHERE id = 'thread'",
        )
        .fetch_one(&pool)
        .await
        .expect("read projection");
        assert_eq!(projection, (0, 1));

        let payload: String = sqlx::query_scalar(
            "SELECT payload FROM sync_queue WHERE action_type = 'remote_mutation'
             AND json_extract(payload, '$.kind') = 'flags'",
        )
        .fetch_one(&pool)
        .await
        .expect("queued flag");
        match serde_json::from_str::<RemoteMutationPayload>(&payload).expect("flag payload") {
            RemoteMutationPayload::Flags {
                refs,
                read,
                starred,
            } => {
                assert_eq!(read, Some(true));
                assert_eq!(starred, None);
                assert_eq!(refs.len(), 1);
                assert_eq!(refs[0].remote_id, "101");
                assert_eq!(refs[0].source_folder.as_deref(), Some("INBOX"));
            }
            other => panic!("unexpected payload: {other:?}"),
        }
    }

    #[tokio::test]
    async fn briefing_loads_exact_pending_threads_and_upcoming_events() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite");
        sqlx::query(
            "CREATE TABLE app_settings (key TEXT PRIMARY KEY, value TEXT);
             CREATE TABLE accounts (id TEXT PRIMARY KEY, kind TEXT);
             CREATE TABLE threads (
               id TEXT PRIMARY KEY, account_id TEXT, subject TEXT,
               last_message_date TEXT, unread_count INTEGER
             );
             CREATE TABLE messages (
               id TEXT PRIMARY KEY, thread_id TEXT, from_name TEXT,
               from_address TEXT, date TEXT
             );
             CREATE TABLE calendar_events (
               id TEXT PRIMARY KEY, account_id TEXT, uid TEXT, title TEXT,
               starts_at TEXT, ends_at TEXT, all_day INTEGER, location TEXT,
               raw_ics TEXT
             );
             CREATE TABLE calendar_invites (
               id TEXT PRIMARY KEY, account_id TEXT, uid TEXT, summary TEXT,
               starts_at TEXT, ends_at TEXT, all_day INTEGER, location TEXT,
               cancelled INTEGER, rsvp_status TEXT
             );",
        )
        .execute(&pool)
        .await
        .expect("schema");
        sqlx::query("INSERT INTO accounts (id, kind) VALUES ('account', 'email')")
            .execute(&pool)
            .await
            .expect("account");
        sqlx::query(
            "INSERT INTO threads (id, account_id, subject, last_message_date, unread_count)
             VALUES ('thread', 'account', 'Freigabe', '2026-08-04T10:00:00Z', 1)",
        )
        .execute(&pool)
        .await
        .expect("thread");
        sqlx::query(
            "INSERT INTO messages (id, thread_id, from_name, from_address, date)
             VALUES ('message', 'thread', 'Mara Hoffmann', 'mara@example.com', '2026-08-04T10:00:00Z')",
        )
        .execute(&pool)
        .await
        .expect("message");
        let starts_at = (chrono::Utc::now() + chrono::Duration::minutes(20)).to_rfc3339();
        let ends_at = (chrono::Utc::now() + chrono::Duration::minutes(80)).to_rfc3339();
        sqlx::query(
            "INSERT INTO calendar_events
             (id, account_id, uid, title, starts_at, ends_at, all_day, location)
             VALUES ('event', 'account', 'event-uid', 'Projektbesprechung', ?, ?, 0, 'Meet')",
        )
        .bind(&starts_at)
        .bind(&ends_at)
        .execute(&pool)
        .await
        .expect("event");
        let later_starts_at = (chrono::Utc::now() + chrono::Duration::hours(3)).to_rfc3339();
        let later_ends_at = (chrono::Utc::now() + chrono::Duration::hours(4)).to_rfc3339();
        sqlx::query(
            "INSERT INTO calendar_events
             (id, account_id, uid, title, starts_at, ends_at, all_day, location)
             VALUES ('later-event', 'account', 'later-event-uid', 'Späterer Termin', ?, ?, 0, NULL)",
        )
        .bind(&later_starts_at)
        .bind(&later_ends_at)
        .execute(&pool)
        .await
        .expect("later event");
        let recurring_start =
            chrono::Utc::now() - chrono::Duration::days(14) + chrono::Duration::hours(1);
        let recurring_end = recurring_start + chrono::Duration::hours(1);
        let recurring_ics = format!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:recurring-uid\r\nSUMMARY:Regeltermin\r\nDTSTART:{}\r\nDTEND:{}\r\nRRULE:FREQ=WEEKLY;INTERVAL=2\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
            recurring_start.format("%Y%m%dT%H%M%SZ"),
            recurring_end.format("%Y%m%dT%H%M%SZ"),
        );
        sqlx::query(
            "INSERT INTO calendar_events
             (id, account_id, uid, title, starts_at, ends_at, all_day, location, raw_ics)
             VALUES ('recurring', 'account', 'recurring-uid', 'Regeltermin', ?, ?, 0, NULL, ?)",
        )
        .bind(recurring_start.to_rfc3339())
        .bind(recurring_end.to_rfc3339())
        .bind(recurring_ics)
        .execute(&pool)
        .await
        .expect("recurring event");
        sqlx::query(
            "INSERT INTO calendar_events
             (id, account_id, uid, title, starts_at, ends_at, all_day, location)
             VALUES ('broken', 'account', 'broken-uid', 'Defekter Termin',
                     '2000-10-24T22:00:00Z', '2099-08-19T13:30:00Z', 0, NULL)",
        )
        .execute(&pool)
        .await
        .expect("broken event");

        let threads = load_tray_threads(&pool, &["thread".into()])
            .await
            .expect("tray threads");
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].sender_name, "Mara Hoffmann");
        assert!(threads[0].is_unread);

        let events = load_upcoming_calendar_events(&pool)
            .await
            .expect("calendar query");
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].title, "Projektbesprechung");
        assert_eq!(events[0].location.as_deref(), Some("Meet"));
        assert_eq!(events[1].title, "Regeltermin");
        assert_eq!(events[2].title, "Späterer Termin");

        // DATE and legacy UTC-midnight rows share the local calendar-day end.
        // Run this test under UTC, America/Los_Angeles and Europe/Berlin.
        use chrono::TimeZone;
        sqlx::query("DELETE FROM calendar_events")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM calendar_invites")
            .execute(&pool)
            .await
            .unwrap();

        let midnight = chrono::Local
            .with_ymd_and_hms(2026, 3, 29, 0, 0, 0)
            .earliest()
            .unwrap()
            .with_timezone(&chrono::Utc);
        for (id, title, start, end, all_day) in [
            (
                "date",
                "Holiday",
                "2026-03-29".to_string(),
                "2026-03-30".to_string(),
                true,
            ),
            (
                "legacy",
                "Holiday",
                "2026-03-29T00:00:00Z".to_string(),
                "2026-03-30T00:00:00Z".to_string(),
                true,
            ),
            (
                "before",
                "Before",
                (midnight - chrono::Duration::minutes(30)).to_rfc3339(),
                (midnight - chrono::Duration::minutes(15)).to_rfc3339(),
                false,
            ),
            (
                "after",
                "After",
                (midnight + chrono::Duration::minutes(30)).to_rfc3339(),
                (midnight + chrono::Duration::minutes(45)).to_rfc3339(),
                false,
            ),
        ] {
            sqlx::query("INSERT INTO calendar_events (id, account_id, uid, title, starts_at, ends_at, all_day) VALUES (?, 'account', ?, ?, ?, ?, ?)")
                .bind(id).bind(id).bind(title).bind(start).bind(end).bind(all_day)
                .execute(&pool).await.unwrap();
        }
        let ordered =
            load_upcoming_calendar_events_at(&pool, midnight - chrono::Duration::hours(1))
                .await
                .unwrap();
        assert_eq!(
            ordered
                .iter()
                .map(|event| event.title.as_str())
                .collect::<Vec<_>>(),
            ["Before", "Holiday", "After"]
        );
        assert_eq!(ordered[1].starts_at, "2026-03-29");
        assert_eq!(ordered[1].ends_at, "2026-03-30");
        sqlx::query("DELETE FROM calendar_events")
            .execute(&pool)
            .await
            .unwrap();

        for (year, month, day) in [(2026, 3, 9), (2026, 3, 30), (2026, 10, 26), (2026, 11, 2)] {
            let end_date = chrono::NaiveDate::from_ymd_opt(year, month, day).unwrap();
            let end_utc = chrono::Local
                .with_ymd_and_hms(year, month, day, 0, 0, 0)
                .earliest()
                .unwrap()
                .with_timezone(&chrono::Utc);
            for span_days in [1, 7] {
                let current_start = end_date - chrono::Duration::days(span_days);
                for legacy in [false, true] {
                    for recurring in [false, true] {
                        // The recurring anchor is already past; the final occurrence
                        // must survive until its own local end, without a fallback.
                        let start = if recurring {
                            current_start.pred_opt().unwrap()
                        } else {
                            current_start
                        };
                        let end = start + chrono::Duration::days(span_days);
                        let start_text = if legacy {
                            format!("{start}T00:00:00Z")
                        } else {
                            start.to_string()
                        };
                        let end_text = if legacy {
                            format!("{end}T00:00:00Z")
                        } else {
                            end.to_string()
                        };
                        let raw = recurring.then(|| format!(
                            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:date-boundary\r\nSUMMARY:All day\r\nDTSTART;VALUE=DATE:{}\r\nDTEND;VALUE=DATE:{}\r\nRRULE:FREQ=DAILY;COUNT=2\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
                            start.format("%Y%m%d"), end.format("%Y%m%d"),
                        ));
                        sqlx::query("INSERT INTO calendar_events (id, account_id, uid, title, starts_at, ends_at, all_day, raw_ics) VALUES ('date-boundary', 'account', 'date-boundary', 'All day', ?, ?, 1, ?) ON CONFLICT(id) DO UPDATE SET starts_at = excluded.starts_at, ends_at = excluded.ends_at, raw_ics = excluded.raw_ics")
                            .bind(start_text).bind(end_text).bind(raw).execute(&pool).await.unwrap();
                        let before = load_upcoming_calendar_events_at(
                            &pool,
                            end_utc - chrono::Duration::hours(1),
                        )
                        .await
                        .unwrap();
                        assert_eq!(before.len(), 1, "legacy={legacy}, recurring={recurring}, days={span_days}, end={end_date}");
                        assert_eq!(before[0].starts_at, current_start.to_string());
                        let after = load_upcoming_calendar_events_at(
                            &pool,
                            end_utc + chrono::Duration::hours(1),
                        )
                        .await
                        .unwrap();
                        assert!(after.is_empty(), "legacy={legacy}, recurring={recurring}, days={span_days}, end={end_date}");
                    }
                }
            }
        }
    }
}
