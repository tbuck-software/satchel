//! Signed desktop updates. The frontend never chooses a feed, file, or install path.

use super::error::CommandResult;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/app_update/")
)]
pub enum UpdateChannel {
    Stable,
    Nightly,
}

impl UpdateChannel {
    #[cfg(desktop)]
    fn as_str(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Nightly => "nightly",
        }
    }
}

#[derive(Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/app_update/")
)]
pub struct AppUpdatePreferences {
    pub channel: UpdateChannel,
    pub supported: bool,
}

const CHANNEL_SETTING: &str = "updates.channel";

async fn saved_channel(pool: &sqlx::SqlitePool) -> Result<UpdateChannel, sqlx::Error> {
    let saved: Option<String> = sqlx::query_scalar("SELECT value FROM app_settings WHERE key = ?")
        .bind(CHANNEL_SETTING)
        .fetch_optional(pool)
        .await?;
    Ok(
        match saved.as_deref().or(option_env!("SATCHEL_RELEASE_CHANNEL")) {
            Some("nightly") => UpdateChannel::Nightly,
            _ => UpdateChannel::Stable,
        },
    )
}

#[cfg(desktop)]
async fn save_channel(pool: &sqlx::SqlitePool, channel: UpdateChannel) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO app_settings (key, value, updated_at) VALUES (?, ?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
    )
    .bind(CHANNEL_SETTING)
    .bind(channel.as_str())
    .bind(chrono::Utc::now().to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

fn updates_supported(app: &tauri::AppHandle) -> bool {
    cfg!(desktop)
        && !cfg!(debug_assertions)
        && option_env!("SATCHEL_RELEASE_CHANNEL").is_some()
        && app.config().identifier == "de.tbuck.satchel"
}

#[tauri::command]
pub async fn get_app_update_preferences(
    app: tauri::AppHandle,
    pool: tauri::State<'_, sqlx::SqlitePool>,
) -> CommandResult<AppUpdatePreferences> {
    Ok(AppUpdatePreferences {
        channel: saved_channel(pool.inner())
            .await
            .map_err(|_| "backend.errors.updateCheckFailed")?,
        supported: updates_supported(&app),
    })
}

#[tauri::command]
pub async fn set_app_update_channel(
    app: tauri::AppHandle,
    pool: tauri::State<'_, sqlx::SqlitePool>,
    channel: UpdateChannel,
) -> CommandResult<()> {
    if !updates_supported(&app) {
        return Err("backend.errors.updateUnavailable".into());
    }
    #[cfg(desktop)]
    {
        use std::sync::atomic::Ordering::Relaxed;
        let state = app.state::<UpdateState>();
        let mut pending = state.pending.lock().await;
        if state.installing.load(Relaxed) {
            return Err("backend.errors.updateUnavailable".into());
        }
        save_channel(pool.inner(), channel)
            .await
            .map_err(|_| "backend.errors.updateCheckFailed")?;
        // Never install a downloaded package from the channel the user just left.
        *pending = None;
        state.ready.store(false, Relaxed);
        state.progress.store(0, Relaxed);
    }
    #[cfg(mobile)]
    let _ = (pool, channel);
    Ok(())
}

#[cfg(any(target_os = "macos", all(test, unix)))]
mod macos;

#[cfg(desktop)]
use std::sync::{
    atomic::{AtomicBool, AtomicU8},
    Mutex,
};
#[cfg(desktop)]
use tauri::{Emitter, Manager};

#[derive(serde::Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/app_update/")
)]
pub struct AppUpdateCheck {
    pub channel: Option<String>,
    pub version: Option<String>,
}

#[cfg(desktop)]
#[derive(Default)]
pub struct UpdateState {
    pending: tokio::sync::Mutex<Option<PendingUpdate>>,
    progress: AtomicU8,
    ready: AtomicBool,
    installing: AtomicBool,
    exit_prepared: AtomicBool,
    flush: Mutex<Option<FlushRound>>,
    notify: tokio::sync::Notify,
}

#[cfg(desktop)]
struct PendingUpdate {
    update: tauri_plugin_updater::Update,
    bytes: Option<Vec<u8>>,
    installed: bool,
    #[cfg(target_os = "macos")]
    installed_bundle: Option<std::path::PathBuf>,
}

#[cfg(desktop)]
struct FlushRound {
    id: String,
    windows: std::collections::BTreeSet<String>,
    failed: bool,
}

#[cfg(desktop)]
impl FlushRound {
    fn acknowledge(&mut self, window: &str, request_id: &str, saved: bool) -> bool {
        if self.id != request_id || !self.windows.remove(window) {
            return false;
        }
        self.failed |= !saved;
        true
    }
}

#[cfg(desktop)]
fn feed_url(channel: &str) -> &'static str {
    if channel == "nightly" {
        // The former Daily app keeps its old latest.json feed. Sharing that
        // feed would change its bundle identity and strand its separate profile.
        "https://github.com/tbuck-software/satchel/releases/download/nightly/satchel.json"
    } else {
        "https://github.com/tbuck-software/satchel/releases/latest/download/latest.json"
    }
}

#[cfg(desktop)]
fn permitted_release(
    channel: &str,
    version: &str,
    manifest: &serde_json::Value,
    url: &reqwest::Url,
) -> bool {
    let tag = format!("v{version}");
    manifest
        .get("app_identifier")
        .and_then(serde_json::Value::as_str)
        == Some("de.tbuck.satchel")
        && manifest.get("channel").and_then(serde_json::Value::as_str) == Some(channel)
        && manifest
            .get("release_tag")
            .and_then(serde_json::Value::as_str)
            == Some(tag.as_str())
        && url.scheme() == "https"
        && url.host_str() == Some("github.com")
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
        && url
            .path()
            .starts_with(&format!("/tbuck-software/satchel/releases/download/{tag}/"))
}

#[tauri::command]
pub async fn check_app_update(
    app: tauri::AppHandle,
    pool: tauri::State<'_, sqlx::SqlitePool>,
) -> CommandResult<AppUpdateCheck> {
    #[cfg(desktop)]
    {
        use tauri_plugin_updater::UpdaterExt;
        if !updates_supported(&app) {
            return Ok(AppUpdateCheck {
                channel: None,
                version: None,
            });
        }
        let state = app.state::<UpdateState>();
        let mut pending = state.pending.lock().await;
        // Serialize reading the choice with channel changes and offer creation.
        let channel = saved_channel(pool.inner())
            .await
            .map_err(|_| "backend.errors.updateCheckFailed")?;
        let channel = channel.as_str();
        // A periodic check must not discard a downloaded update or race installation.
        if let Some(pending) = pending.as_ref() {
            return Ok(AppUpdateCheck {
                channel: Some(channel.into()),
                version: Some(pending.update.version.clone()),
            });
        }
        let updater = app
            .updater_builder()
            .endpoints(vec![feed_url(channel).parse().expect("static update URL")])
            .map_err(|_| "backend.errors.updateCheckFailed")?
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|_| "backend.errors.updateCheckFailed")?;
        let update = updater.check().await.map_err(|error| {
            tracing::debug!(%error, "update feed unavailable");
            "backend.errors.updateCheckFailed"
        })?;
        if update.as_ref().is_some_and(|update| {
            !permitted_release(
                channel,
                &update.version,
                &update.raw_json,
                &update.download_url,
            )
        }) {
            return Err("backend.errors.updateCheckFailed".into());
        }
        let version = update.as_ref().map(|update| update.version.clone());
        *pending = update.map(|update| PendingUpdate {
            update,
            bytes: None,
            installed: false,
            #[cfg(target_os = "macos")]
            installed_bundle: None,
        });
        Ok(AppUpdateCheck {
            channel: Some(channel.into()),
            version,
        })
    }
    #[cfg(mobile)]
    {
        let _ = (app, pool);
        Ok(AppUpdateCheck {
            channel: None,
            version: None,
        })
    }
}

#[tauri::command]
pub fn app_update_progress(app: tauri::AppHandle) -> CommandResult<u8> {
    #[cfg(desktop)]
    {
        Ok(app
            .state::<UpdateState>()
            .progress
            .load(std::sync::atomic::Ordering::Relaxed))
    }
    #[cfg(mobile)]
    {
        let _ = app;
        Ok(0)
    }
}

#[tauri::command]
pub async fn download_app_update(app: tauri::AppHandle) -> CommandResult<()> {
    #[cfg(desktop)]
    {
        use std::sync::atomic::Ordering::Relaxed;
        let state = app.state::<UpdateState>();
        let mut pending = state.pending.lock().await;
        let pending = pending.as_mut().ok_or("backend.errors.updateUnavailable")?;
        if pending.bytes.is_some() || pending.installed {
            return Ok(());
        }
        state.progress.store(0, Relaxed);
        let mut downloaded = 0_u64;
        let bytes = pending
            .update
            .download(
                |chunk, total| {
                    downloaded = downloaded.saturating_add(chunk as u64);
                    if let Some(total) = total.filter(|total| *total > 0) {
                        state.progress.store(
                            ((downloaded.saturating_mul(100) / total).min(99)) as u8,
                            Relaxed,
                        );
                    }
                },
                || {},
            )
            .await
            .map_err(|error| {
                tracing::warn!(%error, "signed update download failed");
                "backend.errors.updateDownloadFailed"
            })?;
        // The plugin verifies the signature before returning any bytes.
        pending.bytes = Some(bytes);
        state.progress.store(100, Relaxed);
        state.ready.store(true, Relaxed);
        Ok(())
    }
    #[cfg(mobile)]
    {
        let _ = app;
        Err("backend.errors.updateUnavailable".into())
    }
}

#[tauri::command]
pub fn acknowledge_app_update_flush(
    window: tauri::WebviewWindow,
    request_id: String,
    saved: bool,
) -> CommandResult<()> {
    #[cfg(desktop)]
    {
        let state = window.state::<UpdateState>();
        let mut flush = state.flush.lock().expect("update flush lock");
        if let Some(round) = flush.as_mut() {
            if round.acknowledge(window.label(), &request_id, saved) {
                state.notify.notify_one();
            }
        }
    }
    #[cfg(mobile)]
    let _ = (window, request_id, saved);
    Ok(())
}

#[cfg(desktop)]
async fn flush_composers(app: &tauri::AppHandle, state: &UpdateState) -> Result<(), String> {
    let windows = app
        .webview_windows()
        .into_keys()
        .filter(|label| label == "main" || label.starts_with("compose-"))
        .collect::<std::collections::BTreeSet<_>>();
    let id = uuid::Uuid::new_v4().to_string();
    *state.flush.lock().expect("update flush lock") = Some(FlushRound {
        id: id.clone(),
        windows,
        failed: false,
    });
    let result = async {
        app.emit("app-update:flush", &id)
            .map_err(|_| "backend.errors.updateDraftsUnsaved")?;
        loop {
            {
                let flush = state.flush.lock().expect("update flush lock");
                let round = flush.as_ref().expect("active flush round");
                if round.failed {
                    return Err("backend.errors.updateDraftsUnsaved");
                }
                if round.windows.is_empty() {
                    return Ok(());
                }
            }
            state.notify.notified().await;
        }
    };
    let result = tokio::time::timeout(std::time::Duration::from_secs(15), result).await;
    *state.flush.lock().expect("update flush lock") = None;
    result
        .unwrap_or(Err("backend.errors.updateDraftsUnsaved"))
        .map_err(str::to_string)
}

#[cfg(desktop)]
async fn install_pending_update(app: &tauri::AppHandle) -> CommandResult<()> {
    use std::sync::atomic::Ordering::Relaxed;
    let state = app.state::<UpdateState>();
    let mut pending = state.pending.lock().await;
    let pending = pending.as_mut().ok_or("backend.errors.updateUnavailable")?;
    flush_composers(app, &state).await?;
    if !pending.installed {
        let bytes = pending
            .bytes
            .as_ref()
            .ok_or("backend.errors.updateUnavailable")?;
        #[cfg(target_os = "macos")]
        let bundle = tauri::utils::platform::current_exe()
            .and_then(|executable| macos::own_bundle_path(&executable))
            .map_err(|error| {
                tracing::warn!(%error, "update requires a normally installed app bundle");
                "backend.errors.updateInstallLocationRequired"
            })?;
        pending.update.install(bytes).map_err(|error| {
            tracing::warn!(%error, "signed update installation failed");
            "backend.errors.updateInstallFailed"
        })?;
        pending.installed = true;
        pending.bytes = None;
        #[cfg(target_os = "macos")]
        {
            pending.installed_bundle = Some(bundle);
        }
    }
    #[cfg(target_os = "macos")]
    macos::clear_update_quarantine(
        pending
            .installed_bundle
            .as_deref()
            .expect("installed bundle"),
    )
    .map_err(|error| {
        tracing::warn!(%error, "verified update installed but quarantine cleanup failed");
        "backend.errors.updateRestartPreparationFailed"
    })?;
    state.ready.store(false, Relaxed);
    // Both restart and regular quit have already saved every draft. The
    // ordinary quit bridge must not turn a requested restart into exit(0).
    state.exit_prepared.store(true, Relaxed);
    Ok(())
}

#[tauri::command]
pub async fn install_app_update(app: tauri::AppHandle) -> CommandResult<()> {
    #[cfg(desktop)]
    {
        use std::sync::atomic::Ordering::Relaxed;
        let state = app.state::<UpdateState>();
        if state.installing.swap(true, Relaxed) {
            return Err("backend.errors.updateUnavailable".into());
        }
        let result = install_pending_update(&app).await;
        state.installing.store(false, Relaxed);
        result?;
        app.request_restart();
        Ok(())
    }
    #[cfg(mobile)]
    {
        let _ = app;
        Err("backend.errors.updateUnavailable".into())
    }
}

#[cfg(desktop)]
pub(crate) fn exit_is_prepared(app: &tauri::AppHandle) -> bool {
    app.state::<UpdateState>()
        .exit_prepared
        .load(std::sync::atomic::Ordering::Relaxed)
}

/// Install only a complete, signature-verified download. No network work is
/// started by Quit; failed saves or installation leave the app open to retry.
#[cfg(desktop)]
pub(crate) fn install_on_exit(app: &tauri::AppHandle) -> bool {
    use std::sync::atomic::Ordering::Relaxed;
    let state = app.state::<UpdateState>();
    if state.installing.load(Relaxed) {
        return true;
    }
    if !state.ready.load(Relaxed) {
        return false;
    }
    if state.installing.swap(true, Relaxed) {
        return true;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let result = install_pending_update(&app).await;
        app.state::<UpdateState>().installing.store(false, Relaxed);
        match result {
            Ok(()) => crate::exit_now(&app),
            Err(error) => {
                tracing::warn!(%error, "update on quit was stopped");
                let _ = app.emit("app-update:failed", error.encoded());
            }
        }
    });
    true
}

#[cfg(all(test, desktop))]
mod tests {
    use super::*;

    #[test]
    fn only_expected_windows_in_the_current_round_can_acknowledge() {
        let mut round = FlushRound {
            id: "current".into(),
            windows: std::collections::BTreeSet::from(["main".into(), "compose-1".into()]),
            failed: false,
        };
        assert!(!round.acknowledge("main", "old", true));
        assert!(!round.acknowledge("message-file-1", "current", false));
        assert!(round.acknowledge("main", "current", true));
        assert!(!round.acknowledge("main", "current", false));
        assert!(!round.failed);
        assert!(round.acknowledge("compose-1", "current", false));
        assert!(round.failed);
        assert!(round.windows.is_empty());
    }

    #[test]
    fn refuses_another_channel_or_version_even_at_the_trusted_host() {
        let manifest = serde_json::json!({ "channel": "stable", "release_tag": "v0.1.1", "app_identifier": "de.tbuck.satchel" });
        let url = reqwest::Url::parse(
            "https://github.com/tbuck-software/satchel/releases/download/v0.1.1/app.tar.gz",
        )
        .unwrap();
        assert!(permitted_release("stable", "0.1.1", &manifest, &url));
        assert!(!permitted_release("nightly", "0.1.1", &manifest, &url));
        assert!(!permitted_release("stable", "0.1.2", &manifest, &url));
        let mut foreign = url.clone();
        foreign
            .set_host(Some("github.com.attacker.example"))
            .unwrap();
        assert!(!permitted_release("stable", "0.1.1", &manifest, &foreign));
        foreign = url.clone();
        foreign.set_query(Some("token=fixture"));
        assert!(!permitted_release("stable", "0.1.1", &manifest, &foreign));
    }

    #[test]
    fn channels_have_separate_feeds() {
        assert!(feed_url("stable").contains("/releases/latest/download/"));
        assert!(feed_url("nightly").contains("/releases/download/nightly/"));
        assert!(feed_url("nightly").ends_with("/satchel.json"));
    }

    #[tokio::test]
    async fn channel_choice_survives_a_new_connection_without_changing_mail() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let url = format!("sqlite:{}", file.path().display());
        let pool = sqlx::SqlitePool::connect(&url).await.unwrap();
        sqlx::query(
            "CREATE TABLE app_settings (key TEXT PRIMARY KEY, value TEXT, updated_at TEXT)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO app_settings VALUES ('fixture.mail', 'unchanged', '')")
            .execute(&pool)
            .await
            .unwrap();
        save_channel(&pool, UpdateChannel::Nightly).await.unwrap();
        pool.close().await;
        let pool = sqlx::SqlitePool::connect(&url).await.unwrap();
        assert_eq!(saved_channel(&pool).await.unwrap(), UpdateChannel::Nightly);
        save_channel(&pool, UpdateChannel::Stable).await.unwrap();
        assert_eq!(saved_channel(&pool).await.unwrap(), UpdateChannel::Stable);
        let value: String =
            sqlx::query_scalar("SELECT value FROM app_settings WHERE key = 'fixture.mail'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(value, "unchanged");
        pool.close().await;
    }

    #[test]
    fn a_daily_bundle_cannot_be_offered_to_the_regular_app() {
        let url = reqwest::Url::parse("https://github.com/tbuck-software/satchel/releases/download/v0.1.1-nightly.42/app.tar.gz").unwrap();
        let mut manifest = serde_json::json!({"channel": "nightly", "release_tag": "v0.1.1-nightly.42", "app_identifier": "de.tbuck.satchel.daily"});
        assert!(!permitted_release(
            "nightly",
            "0.1.1-nightly.42",
            &manifest,
            &url
        ));
        manifest["app_identifier"] = "de.tbuck.satchel".into();
        assert!(permitted_release(
            "nightly",
            "0.1.1-nightly.42",
            &manifest,
            &url
        ));
    }
}
