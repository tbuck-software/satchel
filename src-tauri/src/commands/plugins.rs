use serde::Serialize;
use sqlx::SqlitePool;
use std::collections::{HashMap, HashSet};
use std::str::FromStr;

use crate::commands::error::{CommandError, CommandResult};
use crate::plugins::registry::{PluginCapability, PluginDisabledReason, PluginTrustLevel};
use crate::plugins::runtime::{enforced_plugin_capabilities, PluginAuditRecord, PluginMetrics};
use crate::AppState;
use satchel_core::db::plugin_permissions::{
    PluginPermissionError, PluginPermissionRepository, PluginPermissionUpsertRequest,
};

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/plugins/"))]
pub struct PluginDto {
    pub id: String,
    pub version: String,
    pub enabled: bool,
    pub api_version: String,
    pub capabilities: Vec<PluginCapability>,
    pub event_subscriptions: Vec<String>,
    pub ui_slots: Vec<String>,
    pub trust: PluginTrustLevel,
    pub signature: Option<String>,
    pub signature_verified: bool,
    pub manifest_errors: Vec<String>,
    /// Why a plugin with a valid manifest is off; `None` when on or broken.
    pub disabled_by: Option<PluginDisabledReason>,
    pub granted_capabilities: Vec<PluginCapability>,
    /// Capabilities the runtime actually gates a command on. The rest are
    /// manifest declarations with no enforcement behind them yet, and the
    /// settings UI has to say so rather than offer eight equal switches.
    pub enforced_capabilities: Vec<PluginCapability>,
}

#[tauri::command]
pub async fn list_plugins(
    state: tauri::State<'_, AppState>,
    pool: tauri::State<'_, crate::db::ReadPool>,
) -> CommandResult<Vec<PluginDto>> {
    let grants = load_plugin_grants(pool.inner().as_ref()).await?;
    state.plugin_runtime.set_permission_grants(grants.clone())?;
    state
        .plugin_runtime
        .set_disabled_plugins(load_disabled_plugins(pool.inner().as_ref()).await?)?;
    let enforced = enforced_plugin_capabilities();
    Ok(state
        .plugin_runtime
        .list_plugins()?
        .into_iter()
        .map(|plugin| {
            let granted_capabilities = grants
                .get(&plugin.id)
                .map(|set| set.iter().copied().collect())
                .unwrap_or_default();
            PluginDto {
                id: plugin.id,
                version: plugin.version,
                enabled: plugin.enabled,
                api_version: plugin.api_version,
                capabilities: plugin.capabilities,
                event_subscriptions: plugin.event_subscriptions,
                ui_slots: plugin.ui_slots,
                trust: plugin.trust,
                signature: plugin.signature,
                signature_verified: plugin.signature_verified,
                manifest_errors: plugin.manifest_errors,
                disabled_by: plugin.disabled_by,
                granted_capabilities,
                enforced_capabilities: enforced.clone(),
            }
        })
        .collect())
}

#[derive(Debug, Clone, serde::Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/plugins/", optional_fields = nullable))]
pub struct PluginPermissionInput {
    pub plugin_id: String,
    pub capability: PluginCapability,
    pub granted: bool,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/plugins/", optional_fields = nullable))]
pub struct PluginCommandInput {
    pub plugin_id: String,
    pub command: String,
    pub payload: String,
}

#[tauri::command]
pub async fn execute_plugin_command(
    state: tauri::State<'_, AppState>,
    input: PluginCommandInput,
) -> CommandResult<String> {
    // WASM execution blocks (wasmtime instantiate + run up to the plugin
    // timeout); keep it off the tokio worker threads.
    let runtime = state.plugin_runtime.clone();
    tokio::task::spawn_blocking(move || {
        runtime.execute(&input.plugin_id, &input.command, &input.payload)
    })
    .await
    .map_err(|err| format!("plugin command task failed: {err}"))?
    .map_err(Into::into)
}

#[tauri::command]
pub async fn set_plugin_permission(
    state: tauri::State<'_, AppState>,
    pool: tauri::State<'_, SqlitePool>,
    input: PluginPermissionInput,
) -> CommandResult<()> {
    PluginPermissionRepository::new(pool.inner())
        .upsert(PluginPermissionUpsertRequest {
            plugin_id: input.plugin_id,
            capability: input.capability.as_str().to_owned(),
            granted: input.granted,
        })
        .await
        .map_err(CommandError::from)?;
    let grants = load_plugin_grants(pool.inner()).await?;
    state.plugin_runtime.set_permission_grants(grants)?;
    Ok(())
}

#[derive(Debug, Clone, serde::Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/plugins/", optional_fields = nullable))]
pub struct PluginEnabledInput {
    pub plugin_id: String,
    pub enabled: bool,
}

/// The user's on/off switch. A plugin with manifest errors stays off either way.
#[tauri::command]
pub async fn set_plugin_enabled(
    state: tauri::State<'_, AppState>,
    pool: tauri::State<'_, SqlitePool>,
    input: PluginEnabledInput,
) -> CommandResult<()> {
    let reason = (!input.enabled).then_some(PluginDisabledReason::User);
    store_plugin_disabled(pool.inner(), &input.plugin_id, reason).await?;
    state
        .plugin_runtime
        .set_disabled_plugins(load_disabled_plugins(pool.inner()).await?)?;
    Ok(())
}

const DISABLED_PLUGINS_KEY: &str = "plugins.disabled";

/// `{"<plugin id>": "user" | "timeouts"}` in `app_settings`.
pub async fn load_disabled_plugins(
    pool: &SqlitePool,
) -> Result<HashMap<String, PluginDisabledReason>, CommandError> {
    let stored: Option<String> = sqlx::query_scalar("SELECT value FROM app_settings WHERE key = ?")
        .bind(DISABLED_PLUGINS_KEY)
        .fetch_optional(pool)
        .await
        .map_err(|err| CommandError::from(err.to_string()))?;
    // An unreadable value switches nothing off rather than failing the list.
    Ok(stored
        .and_then(|value| serde_json::from_str(&value).ok())
        .unwrap_or_default())
}

/// Sets or clears one plugin's switch.
pub async fn store_plugin_disabled(
    pool: &SqlitePool,
    plugin_id: &str,
    reason: Option<PluginDisabledReason>,
) -> Result<(), CommandError> {
    let mut disabled = load_disabled_plugins(pool).await?;
    match reason {
        Some(reason) => disabled.insert(plugin_id.to_string(), reason),
        None => disabled.remove(plugin_id),
    };
    let value =
        serde_json::to_string(&disabled).map_err(|err| CommandError::from(err.to_string()))?;
    sqlx::query(
        "INSERT INTO app_settings (key, value, updated_at) VALUES (?, ?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
    )
    .bind(DISABLED_PLUGINS_KEY)
    .bind(value)
    .bind(chrono::Utc::now().to_rfc3339())
    .execute(pool)
    .await
    .map_err(|err| CommandError::from(err.to_string()))?;
    Ok(())
}

#[tauri::command]
pub async fn list_plugin_audit_log(
    state: tauri::State<'_, AppState>,
) -> CommandResult<Vec<PluginAuditRecord>> {
    state.plugin_runtime.audit_log().map_err(Into::into)
}

#[tauri::command]
pub async fn list_plugin_metrics(
    state: tauri::State<'_, AppState>,
) -> CommandResult<Vec<PluginMetrics>> {
    state.plugin_runtime.metrics().map_err(Into::into)
}

pub async fn load_plugin_grants(
    pool: &SqlitePool,
) -> Result<HashMap<String, HashSet<PluginCapability>>, CommandError> {
    let grouped = PluginPermissionRepository::new(pool)
        .list_granted()
        .await
        .map_err(CommandError::from)?;
    let mut grants: HashMap<String, HashSet<PluginCapability>> = HashMap::new();
    for (plugin_id, capabilities) in grouped {
        for capability in capabilities {
            if let Ok(parsed) = PluginCapability::from_str(&capability) {
                grants.entry(plugin_id.clone()).or_default().insert(parsed);
            }
        }
    }
    Ok(grants)
}

impl From<PluginPermissionError> for CommandError {
    fn from(error: PluginPermissionError) -> Self {
        tracing::error!(error = ?error, "plugin permission persistence failed");
        Self::new("backend.errors.pluginPermissionPersistenceFailed")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persistence_errors_use_a_stable_key_without_sql_details() {
        let detail = "near SELECT: secret plugin permission SQL";
        let error = CommandError::from(PluginPermissionError::Persistence(sqlx::Error::Protocol(
            detail.into(),
        )));
        let serialized = serde_json::to_value(error).expect("command error JSON");

        assert_eq!(
            serialized,
            serde_json::json!({
                "key": "backend.errors.pluginPermissionPersistenceFailed"
            })
        );
        assert!(!serialized.to_string().contains(detail));
    }
}
