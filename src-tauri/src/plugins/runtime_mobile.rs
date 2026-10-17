use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::plugins::registry::{PluginCapability, PluginDisabledReason, RegisteredPlugin};

/// Mobile has no executable plugin runtime, so no command capabilities apply.
pub fn enforced_plugin_capabilities() -> Vec<PluginCapability> {
    Vec::new()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/plugins/runtime_mobile/")
)]
pub struct PluginRuntimeLimits {
    pub fuel: u64,
    pub timeout_ms: u64,
}

#[derive(Default)]
pub struct PluginRuntime {
    pub limits: PluginRuntimeLimits,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/plugins/runtime_mobile/")
)]
pub struct PluginAuditRecord {
    pub plugin_id: String,
    pub command: String,
    pub capability: Option<PluginCapability>,
    pub decision: String,
    pub reason: Option<String>,
    pub duration_ms: Option<u128>,
    pub created_at: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/plugins/runtime_mobile/")
)]
pub struct PluginMetrics {
    pub plugin_id: String,
    pub calls: u64,
    pub allowed: u64,
    pub denied: u64,
    pub errors: u64,
    pub timeouts: u64,
    pub total_duration_ms: u128,
    pub last_error: Option<String>,
    pub last_called_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/plugins/runtime_mobile/")
)]
pub struct MessageTransformInput {
    pub subject: String,
    pub snippet: String,
    pub body_text: String,
    pub body_html: Option<String>,
    pub has_pgp_private_key: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/plugins/runtime_mobile/")
)]
pub struct MessageTransformOutput {
    pub subject: Option<String>,
    pub snippet: Option<String>,
    pub body_text: Option<String>,
    pub body_html: Option<String>,
    pub labels: Vec<String>,
    pub pgp_state: Option<String>,
    pub pgp_error_detail: Option<String>,
    pub pgp_signer_name: Option<String>,
    pub pgp_signer_fingerprint: Option<String>,
}

impl PluginRuntime {
    pub fn list_plugins(&self) -> Result<Vec<RegisteredPlugin>, String> {
        Ok(Vec::new())
    }

    pub fn audit_log(&self) -> Result<Vec<PluginAuditRecord>, String> {
        Ok(Vec::new())
    }

    pub fn metrics(&self) -> Result<Vec<PluginMetrics>, String> {
        Ok(Vec::new())
    }

    pub fn set_plugin_dirs(&self, _dirs: Vec<std::path::PathBuf>) -> Result<(), String> {
        Ok(())
    }

    pub fn set_permission_grants(
        &self,
        _grants: HashMap<String, HashSet<PluginCapability>>,
    ) -> Result<(), String> {
        Ok(())
    }

    pub fn set_disabled_plugins(
        &self,
        _disabled: HashMap<String, PluginDisabledReason>,
    ) -> Result<(), String> {
        Ok(())
    }

    pub fn record_transform_outcome(&self, _plugin_id: &str, _error: Option<&str>) -> bool {
        false
    }

    pub fn enabled_message_transform_plugins(&self) -> Result<Vec<String>, String> {
        Ok(Vec::new())
    }

    pub fn prewarm_message_transform_plugins(&self) -> Result<(), String> {
        Ok(())
    }

    pub fn dispatch_event(
        &self,
        _event_name: &str,
        _idempotency_key: &str,
        _payload: serde_json::Value,
    ) -> Result<usize, String> {
        Ok(0)
    }

    pub fn execute(
        &self,
        _plugin_id: &str,
        _command: &str,
        _payload: &str,
    ) -> Result<String, String> {
        Err("executable plugins are not available on mobile".to_string())
    }

    pub fn transform_message(
        &self,
        _plugin_id: &str,
        _input: &MessageTransformInput,
    ) -> Result<MessageTransformOutput, String> {
        Ok(MessageTransformOutput::default())
    }
}
