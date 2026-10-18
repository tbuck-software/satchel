//! Signed desktop updates. The frontend never chooses a feed, file, or install path.

use super::error::CommandResult;

#[cfg(any(target_os = "macos", all(test, unix)))]
mod macos;

#[cfg(desktop)]
use std::sync::{atomic::AtomicU8, Mutex};
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
        "https://github.com/tbuck-software/satchel/releases/download/nightly/latest.json"
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
    manifest.get("channel").and_then(serde_json::Value::as_str) == Some(channel)
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
pub async fn check_app_update(app: tauri::AppHandle) -> CommandResult<AppUpdateCheck> {
    #[cfg(desktop)]
    {
        use tauri_plugin_updater::UpdaterExt;
        let Some(channel) = option_env!("SATCHEL_RELEASE_CHANNEL") else {
            return Ok(AppUpdateCheck {
                channel: None,
                version: None,
            });
        };
        if cfg!(debug_assertions) {
            return Ok(AppUpdateCheck {
                channel: None,
                version: None,
            });
        }
        let state = app.state::<UpdateState>();
        let mut pending = state.pending.lock().await;
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
        let _ = app;
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

#[tauri::command]
pub async fn install_app_update(app: tauri::AppHandle) -> CommandResult<()> {
    #[cfg(desktop)]
    {
        let state = app.state::<UpdateState>();
        let mut pending = state.pending.lock().await;
        let pending = pending.as_mut().ok_or("backend.errors.updateUnavailable")?;
        flush_composers(&app, &state).await?;
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
        app.request_restart();
        Ok(())
    }
    #[cfg(mobile)]
    {
        let _ = app;
        Err("backend.errors.updateUnavailable".into())
    }
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
        let manifest = serde_json::json!({ "channel": "stable", "release_tag": "v0.1.1" });
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
    }
}
