use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use wasmtime::component::{Component, Linker};
use wasmtime::{Cache, CacheConfig, Config, Engine, Store, StoreLimits, StoreLimitsBuilder};
use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

use crate::commands::error::CommandError;
use crate::plugins::registry::{
    default_plugin_dirs, PluginCapability, PluginDisabledReason, PluginRegistry, RegisteredPlugin,
    TimeoutStrikes,
};

wasmtime::component::bindgen!({
    path: "../plugin-sdk/wit",
    world: "satchel-plugin",
    imports: { default: trappable },
});

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/plugins/runtime/"))]
pub struct PluginRuntimeLimits {
    pub fuel: u64,
    pub timeout_ms: u64,
}

fn compilation_cache(directory: PathBuf) -> Result<Cache, String> {
    let mut config = CacheConfig::new();
    config.with_directory(directory);
    config.with_files_total_size_soft_limit(256 * 1024 * 1024);
    Cache::new(config).map_err(|err| err.to_string())
}

impl Default for PluginRuntimeLimits {
    fn default() -> Self {
        Self {
            fuel: 2_000_000,
            timeout_ms: 5_000,
        }
    }
}

/// Upper bound for a single plugin command response handed to the frontend.
/// Matches the local API's request body limit; plugin outputs are UI content,
/// not data transfers.
pub const MAX_PLUGIN_OUTPUT_BYTES: usize = 256 * 1024;

fn validate_plugin_output(plugin_id: &str, output: &str) -> Result<(), String> {
    if output.len() > MAX_PLUGIN_OUTPUT_BYTES {
        return Err(format!(
            "plugin `{plugin_id}` produced a {} byte output; the limit is {MAX_PLUGIN_OUTPUT_BYTES} bytes",
            output.len()
        ));
    }
    Ok(())
}

/// A transform annotates a message; it does not tag it by the hundred.
const MAX_PLUGIN_TRANSFORM_LABELS: usize = 16;
const MAX_PLUGIN_TRANSFORM_LABEL_BYTES: usize = 64;

/// PGP states a plugin is allowed to assert.
///
/// Everything a signature *verdict* rests on is decided by the host in
/// `commands::messages` against real key material. A plugin only reports what
/// it can honestly observe — that a body is encrypted and whether a private
/// key exists — so a substituted plugin cannot put "signature valid, signed by
/// X" under an arbitrary mail.
const PLUGIN_ASSERTABLE_PGP_STATES: &[&str] = &["encrypted_no_key", "key_available_locked"];

/// The structural half of the output check: what a transform is allowed to say.
fn validate_message_transform_output(
    plugin_id: &str,
    output: &MessageTransformOutput,
) -> Result<(), String> {
    if let Some(state) = output.pgp_state.as_deref() {
        if !PLUGIN_ASSERTABLE_PGP_STATES.contains(&state) {
            return Err(format!(
                "plugin `{plugin_id}` returned pgp state `{state}`, which only the host may decide"
            ));
        }
    }
    // Signer identity comes from a verified signature, and a plugin never
    // produces one; naming a signer would be an unbacked claim about the
    // sender.
    if output.pgp_signer_name.is_some() || output.pgp_signer_fingerprint.is_some() {
        return Err(format!(
            "plugin `{plugin_id}` returned a PGP signer identity, which only the host may decide"
        ));
    }
    if output.labels.len() > MAX_PLUGIN_TRANSFORM_LABELS {
        return Err(format!(
            "plugin `{plugin_id}` returned {} labels; the limit is {MAX_PLUGIN_TRANSFORM_LABELS}",
            output.labels.len()
        ));
    }
    if let Some(label) = output
        .labels
        .iter()
        .find(|label| label.len() > MAX_PLUGIN_TRANSFORM_LABEL_BYTES)
    {
        return Err(format!(
            "plugin `{plugin_id}` returned a {} byte label; the limit is {MAX_PLUGIN_TRANSFORM_LABEL_BYTES} bytes",
            label.len()
        ));
    }
    Ok(())
}

/// Capability a plugin command needs, keyed by the command itself.
///
/// Deriving the capability here instead of letting the call site name one is
/// the point: three call sites used to pass `ReadThreads` for every command,
/// which both denied a plugin that holds a different capability and let a
/// command run under a permission that has nothing to do with it.
///
/// `render-slot:sidebar_section` maps to `None` on purpose — the host hands
/// that slot no message data, so it needs no grant. Every other command
/// carries thread content into the plugin.
const PLUGIN_COMMAND_CAPABILITIES: &[(&str, Option<PluginCapability>)] = &[
    ("transform-message", Some(PluginCapability::ReadThreads)),
    ("handle-event", Some(PluginCapability::ReadThreads)),
    (
        "render-slot:top_toolbar",
        Some(PluginCapability::ReadThreads),
    ),
    (
        "render-slot:thread_view_panel",
        Some(PluginCapability::ReadThreads),
    ),
    (
        "render-slot:assistant_context_panel",
        Some(PluginCapability::ReadThreads),
    ),
    ("render-slot:sidebar_section", None),
];

/// The capabilities the table above actually gates something on.
///
/// `plugin.wit` exports one function and imports nothing, so a plugin cannot
/// reach the network, the filesystem or the mail transport whatever its
/// manifest declares. The settings UI reads this list to say which of its
/// switches bite today instead of presenting eight equal-looking toggles.
pub fn enforced_plugin_capabilities() -> Vec<PluginCapability> {
    let mut enforced: Vec<PluginCapability> = Vec::new();
    for (_, capability) in PLUGIN_COMMAND_CAPABILITIES {
        if let Some(capability) = capability {
            if !enforced.contains(capability) {
                enforced.push(*capability);
            }
        }
    }
    enforced
}

/// Resolves a command to the capability it needs. An unlisted command is
/// refused: the host and its own UI are the only callers, so the vocabulary is
/// closed and an unknown name is an IPC caller inventing one.
fn required_capability(command: &str) -> Result<Option<PluginCapability>, String> {
    PLUGIN_COMMAND_CAPABILITIES
        .iter()
        .find(|(name, _)| *name == command)
        .map(|(_, capability)| *capability)
        .ok_or_else(|| {
            CommandError::new("backend.errors.pluginCommandUnknown")
                .with_param("command", command)
                .encoded()
        })
}

pub struct PluginRuntime {
    pub limits: PluginRuntimeLimits,
    engine: Engine,
    #[cfg(test)]
    compilation_cache: Option<Cache>,
    /// Serializes component executions. The epoch deadline that enforces
    /// `timeout_ms` is engine-global, so two concurrent executions could kill
    /// each other's deadline; running one at a time keeps the timeout correct.
    execution_lock: Mutex<()>,
    registry: Mutex<Option<PluginRegistry>>,
    /// Directories the host permits plugins in. `None` until the app names
    /// them, which is the only reason `default_plugin_dirs` still exists.
    plugin_dirs: Mutex<Option<Vec<PathBuf>>>,
    component_paths: Mutex<HashMap<String, Option<PathBuf>>>,
    components: Mutex<HashMap<PathBuf, Arc<Component>>>,
    unsupported_message_transforms: Mutex<HashSet<String>>,
    unsupported_event_handlers: Mutex<HashSet<String>>,
    audit_log: Mutex<Vec<PluginAuditRecord>>,
    metrics: Mutex<HashMap<String, PluginMetrics>>,
    grants: Mutex<HashMap<String, HashSet<PluginCapability>>>,
    disabled: Mutex<HashMap<String, PluginDisabledReason>>,
    timeout_strikes: Mutex<TimeoutStrikes>,
}

/// Fuel counts instructions, not bytes: `memory.grow` is a single instruction,
/// so a plugin can bind gigabytes of RSS well inside its fuel and epoch budget
/// — on the thread that holds the global execution lock. These are the bounds
/// fuel cannot express.
const MAX_PLUGIN_MEMORY_BYTES: usize = 16 * 1024 * 1024;
const MAX_PLUGIN_TABLE_ELEMENTS: usize = 10_000;
const MAX_PLUGIN_INSTANCES: usize = 32;
const MAX_PLUGIN_WASM_STACK_BYTES: usize = 512 * 1024;

struct PluginWasiState {
    table: ResourceTable,
    wasi: WasiCtx,
    limits: StoreLimits,
}

impl WasiView for PluginWasiState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl Default for PluginRuntime {
    fn default() -> Self {
        Self::with_compilation_cache(None)
    }
}

impl PluginRuntime {
    pub fn with_compilation_cache(directory: Option<PathBuf>) -> Self {
        let mut config = Config::new();
        let cache = directory.and_then(|directory| match compilation_cache(directory) {
            Ok(cache) => Some(cache),
            Err(err) => {
                tracing::warn!(%err, "plugin compilation cache unavailable; compiling without it");
                None
            }
        });
        config.cache(cache.clone());
        config.wasm_component_model(true);
        config.consume_fuel(true);
        config.epoch_interruption(true);
        config.max_wasm_stack(MAX_PLUGIN_WASM_STACK_BYTES);
        let engine = Engine::new(&config).expect("valid wasmtime engine config");
        Self {
            limits: PluginRuntimeLimits::default(),
            engine,
            #[cfg(test)]
            compilation_cache: cache,
            execution_lock: Mutex::new(()),
            registry: Mutex::new(None),
            plugin_dirs: Mutex::new(None),
            component_paths: Mutex::new(HashMap::new()),
            components: Mutex::new(HashMap::new()),
            unsupported_message_transforms: Mutex::new(HashSet::new()),
            unsupported_event_handlers: Mutex::new(HashSet::new()),
            audit_log: Mutex::new(Vec::new()),
            metrics: Mutex::new(HashMap::new()),
            grants: Mutex::new(HashMap::new()),
            disabled: Mutex::new(HashMap::new()),
            timeout_strikes: Mutex::new(TimeoutStrikes::default()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/plugins/runtime/"))]
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
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/plugins/runtime/"))]
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
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/plugins/runtime/"))]
pub struct MessageTransformInput {
    pub subject: String,
    pub snippet: String,
    pub body_text: String,
    pub body_html: Option<String>,
    pub has_pgp_private_key: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/plugins/runtime/"))]
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
        let disabled = self
            .disabled
            .lock()
            .map_err(|_| "plugin switch cache poisoned".to_string())?
            .clone();
        Ok(self
            .registry()?
            .list()
            .into_iter()
            .map(|plugin| {
                let disabled_by = disabled.get(&plugin.id).copied();
                plugin.with_disabled_by(disabled_by)
            })
            .collect())
    }

    /// The user's switches, as stored. Switching back on clears the strikes.
    pub fn set_disabled_plugins(
        &self,
        disabled: HashMap<String, PluginDisabledReason>,
    ) -> Result<(), String> {
        let mut strikes = self
            .timeout_strikes
            .lock()
            .map_err(|_| "plugin timeout cache poisoned".to_string())?;
        let mut current = self
            .disabled
            .lock()
            .map_err(|_| "plugin switch cache poisoned".to_string())?;
        for plugin_id in current.keys() {
            if !disabled.contains_key(plugin_id) {
                strikes.reset(plugin_id);
            }
        }
        *current = disabled;
        Ok(())
    }

    /// Counts a transform outcome; `true` when the plugin was just switched off.
    pub fn record_transform_outcome(&self, plugin_id: &str, error: Option<&str>) -> bool {
        let timed_out = error.is_some_and(is_timeout_error);
        let Ok(mut strikes) = self.timeout_strikes.lock() else {
            return false;
        };
        if !strikes.record(plugin_id, timed_out) {
            return false;
        }
        drop(strikes);
        if let Ok(mut disabled) = self.disabled.lock() {
            disabled.insert(plugin_id.to_string(), PluginDisabledReason::Timeouts);
        }
        true
    }

    pub fn audit_log(&self) -> Result<Vec<PluginAuditRecord>, String> {
        Ok(self
            .audit_log
            .lock()
            .map_err(|_| "plugin audit log poisoned".to_string())?
            .clone())
    }

    pub fn metrics(&self) -> Result<Vec<PluginMetrics>, String> {
        let mut values = self
            .metrics
            .lock()
            .map_err(|_| "plugin metrics poisoned".to_string())?
            .values()
            .cloned()
            .collect::<Vec<_>>();
        values.sort_by(|a, b| a.plugin_id.cmp(&b.plugin_id));
        Ok(values)
    }

    /// Names the directories plugins may be loaded from. The app resolves
    /// them from its own data directory and bundle resources; nothing derives
    /// them from the working directory.
    pub fn set_plugin_dirs(&self, dirs: Vec<PathBuf>) -> Result<(), String> {
        *self
            .plugin_dirs
            .lock()
            .map_err(|_| "plugin directory cache poisoned".to_string())? = Some(dirs);
        // Both caches were filled from the previous directories.
        *self
            .registry
            .lock()
            .map_err(|_| "plugin registry cache poisoned".to_string())? = None;
        self.component_paths
            .lock()
            .map_err(|_| "plugin component cache poisoned".to_string())?
            .clear();
        Ok(())
    }

    pub fn set_permission_grants(
        &self,
        grants: HashMap<String, HashSet<PluginCapability>>,
    ) -> Result<(), String> {
        *self
            .grants
            .lock()
            .map_err(|_| "plugin grant cache poisoned".to_string())? = grants;
        Ok(())
    }

    pub fn enabled_message_transform_plugins(&self) -> Result<Vec<String>, String> {
        Ok(self
            .list_plugins()?
            .into_iter()
            .filter(|plugin| plugin.enabled && is_message_transform_plugin(&plugin.id))
            .map(|plugin| plugin.id)
            .collect())
    }

    pub fn prewarm_message_transform_plugins(&self) -> Result<(), String> {
        for plugin_id in self.enabled_message_transform_plugins()? {
            if let Some(component_path) = self.resolve_component_path(&plugin_id)? {
                let _ = self.load_component(&component_path)?;
            }
        }

        Ok(())
    }

    pub fn dispatch_event(
        &self,
        event_name: &str,
        idempotency_key: &str,
        payload: serde_json::Value,
    ) -> Result<usize, String> {
        let envelope = serde_json::json!({
            "schema_version": 1,
            "event": event_name,
            "idempotency_key": idempotency_key,
            "payload": payload,
        });
        let encoded = serde_json::to_string(&envelope)
            .map_err(|err| format!("plugin event encode failed: {err}"))?;
        let mut delivered = 0usize;
        for plugin in self.list_plugins()? {
            if !plugin.enabled
                || !plugin
                    .event_subscriptions
                    .iter()
                    .any(|event| event == event_name)
            {
                continue;
            }
            let unsupported_key = format!("{}:{event_name}", plugin.id);
            if self
                .unsupported_event_handlers
                .lock()
                .map_err(|_| "plugin event cache poisoned".to_string())?
                .contains(&unsupported_key)
            {
                continue;
            }
            if self.ensure_capability(&plugin.id, "handle-event").is_err() {
                continue;
            }
            let Some(component_path) = self.resolve_component_path(&plugin.id)? else {
                continue;
            };
            match self.execute_component(&component_path, "handle-event", &encoded) {
                Ok(_) => delivered += 1,
                Err(err) if is_command_not_implemented_error(&err) => {
                    self.unsupported_event_handlers
                        .lock()
                        .map_err(|_| "plugin event cache poisoned".to_string())?
                        .insert(unsupported_key);
                }
                Err(err) => return Err(err),
            }
        }
        Ok(delivered)
    }

    pub fn execute(&self, plugin_id: &str, command: &str, payload: &str) -> Result<String, String> {
        self.ensure_capability(plugin_id, command)?;
        let Some(component_path) = self.resolve_component_path(plugin_id)? else {
            return Err(format!(
                "no Wasm component available for plugin `{plugin_id}`"
            ));
        };

        let output = self.execute_component(&component_path, command, payload)?;
        validate_plugin_output(plugin_id, &output)?;
        Ok(output)
    }

    pub fn transform_message(
        &self,
        plugin_id: &str,
        input: &MessageTransformInput,
    ) -> Result<MessageTransformOutput, String> {
        if self
            .unsupported_message_transforms
            .lock()
            .map_err(|_| "plugin transform cache poisoned".to_string())?
            .contains(plugin_id)
        {
            return Ok(MessageTransformOutput::default());
        }

        self.ensure_capability(plugin_id, "transform-message")?;

        if let Some(component_path) = self.resolve_component_path(plugin_id)? {
            let payload = serde_json::to_string(input)
                .map_err(|err| format!("plugin payload encode failed: {err}"))?;
            let output =
                match self.execute_component(&component_path, "transform-message", &payload) {
                    Ok(output) => output,
                    Err(err) if is_command_not_implemented_error(&err) => {
                        self.unsupported_message_transforms
                            .lock()
                            .map_err(|_| "plugin transform cache poisoned".to_string())?
                            .insert(plugin_id.to_string());
                        return Ok(MessageTransformOutput::default());
                    }
                    Err(err) => return Err(err),
                };
            // Same gate the `execute` path runs, plus the structural rules:
            // this output lands in the message DTO, where a forged signature
            // verdict is indistinguishable from a real one.
            validate_plugin_output(plugin_id, &output)?;
            if output.trim().is_empty() {
                return Ok(MessageTransformOutput::default());
            }
            let decoded: MessageTransformOutput = serde_json::from_str(&output)
                .map_err(|err| format!("plugin output decode failed: {err}"))?;
            validate_message_transform_output(plugin_id, &decoded)?;
            return Ok(decoded);
        }

        match plugin_id {
            "satchel-plugin-pgp" => Ok(transform_pgp_message(input)),
            _ => Ok(MessageTransformOutput::default()),
        }
    }

    fn ensure_capability(&self, plugin_id: &str, command: &str) -> Result<(), String> {
        let capability = match required_capability(command) {
            Ok(capability) => capability,
            Err(error) => {
                self.record_audit(
                    plugin_id,
                    command,
                    None,
                    "denied",
                    Some("unknown plugin command"),
                    None,
                );
                self.record_denied(plugin_id);
                return Err(error);
            }
        };

        let Some(plugin) = self.registry()?.get(plugin_id) else {
            self.record_audit(
                plugin_id,
                command,
                capability,
                "denied",
                Some("plugin not registered"),
                None,
            );
            self.record_denied(plugin_id);
            return Err(CommandError::new("backend.errors.pluginNotRegistered")
                .with_param("plugin", plugin_id)
                .encoded());
        };

        if !plugin.enabled {
            let reason = if plugin.manifest_errors.is_empty() {
                "plugin disabled".to_string()
            } else {
                plugin.manifest_errors.join("; ")
            };
            self.record_audit(
                plugin_id,
                command,
                capability,
                "denied",
                Some(&reason),
                None,
            );
            self.record_denied(plugin_id);
            return Err(CommandError::new("backend.errors.pluginDisabled")
                .with_param("plugin", plugin_id)
                .with_param("reason", reason)
                .encoded());
        }

        if let Some(capability) = capability {
            if !plugin.capabilities.contains(&capability) {
                let reason = format!("missing capability `{}`", capability.as_str());
                self.record_audit(
                    plugin_id,
                    command,
                    Some(capability),
                    "denied",
                    Some(&reason),
                    None,
                );
                self.record_denied(plugin_id);
                return Err(CommandError::new("backend.errors.pluginCapabilityMissing")
                    .with_param("capability", capability.as_str())
                    .encoded());
            }

            let granted = self
                .grants
                .lock()
                .map_err(|_| "plugin grant cache poisoned".to_string())?
                .get(plugin_id)
                .is_some_and(|grants| grants.contains(&capability));
            if !granted {
                let reason = format!("capability `{}` has not been granted", capability.as_str());
                self.record_audit(
                    plugin_id,
                    command,
                    Some(capability),
                    "denied",
                    Some(&reason),
                    None,
                );
                self.record_denied(plugin_id);
                return Err(
                    CommandError::new("backend.errors.pluginCapabilityNotGranted")
                        .with_param("capability", capability.as_str())
                        .encoded(),
                );
            }
        }

        self.record_audit(plugin_id, command, capability, "allowed", None, None);
        self.record_allowed(plugin_id);
        Ok(())
    }

    fn resolve_component_path(&self, plugin_id: &str) -> Result<Option<PathBuf>, String> {
        if let Some(cached) = self
            .component_paths
            .lock()
            .map_err(|_| "plugin component cache poisoned".to_string())?
            .get(plugin_id)
            .cloned()
        {
            return Ok(cached);
        }

        let resolved = self
            .registry()?
            .list()
            .into_iter()
            .find(|plugin| plugin.id == plugin_id)
            .and_then(|plugin| plugin.component_path);

        self.component_paths
            .lock()
            .map_err(|_| "plugin component cache poisoned".to_string())?
            .insert(plugin_id.to_string(), resolved.clone());

        Ok(resolved)
    }

    fn execute_component(
        &self,
        component_path: &PathBuf,
        command: &str,
        payload: &str,
    ) -> Result<String, String> {
        let component = self.load_component(component_path)?;
        // Serialize executions. The epoch deadline below is engine-global
        // state, so two concurrent executions would share (and corrupt) each
        // other's timeout; one at a time keeps it correct.
        let _execution_guard = self
            .execution_lock
            .lock()
            .map_err(|_| "plugin execution lock poisoned".to_string())?;

        let mut linker = Linker::new(&self.engine);
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker)
            .map_err(|err| format!("plugin wasi linker setup failed: {err}"))?;
        let mut store = Store::new(
            &self.engine,
            PluginWasiState {
                table: ResourceTable::new(),
                wasi: WasiCtxBuilder::new().build(),
                limits: StoreLimitsBuilder::new()
                    .memory_size(MAX_PLUGIN_MEMORY_BYTES)
                    .table_elements(MAX_PLUGIN_TABLE_ELEMENTS)
                    .instances(MAX_PLUGIN_INSTANCES)
                    .build(),
            },
        );
        store.limiter(|state| &mut state.limits);
        store
            .set_fuel(self.limits.fuel)
            .map_err(|err| format!("plugin fuel setup failed: {err}"))?;
        store.set_epoch_deadline(1);
        store.epoch_deadline_trap();

        // The epoch clock is engine-global: instead of a fire-and-forget sleep
        // thread (which accumulated one sleeping thread per call and could
        // fire its increment into a *later* execution), arm a cancellable
        // timer that bumps the epoch only if this execution overruns.
        let (timeout_cancel, timeout_cancelled) = std::sync::mpsc::channel::<()>();
        let engine = self.engine.clone();
        let timeout = Duration::from_millis(self.limits.timeout_ms);
        let timeout_timer =
            std::thread::spawn(move || match timeout_cancelled.recv_timeout(timeout) {
                Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => engine.increment_epoch(),
            });

        let started = Instant::now();
        // Deliberately no `?` between arming and disarming the timer: an
        // early return here would leave the epoch increment live for the next
        // execution.
        //
        // call_run_command yields a doubly wrapped result: outer layer is the
        // host/trap boundary, inner layer is the plugin's own `result<string,
        // string>` return.
        let mut run = || -> Result<Result<String, String>, String> {
            let bindings = SatchelPlugin::instantiate(&mut store, component.as_ref(), &linker)
                .map_err(|err| format!("component instantiate failed: {err}"))?;
            bindings
                .satchel_plugin_plugin()
                .call_run_command(&mut store, command, payload)
                .map_err(|err| format!("plugin command execution failed: {err}"))
        };
        let result = run();
        let duration = started.elapsed().as_millis();

        drop(timeout_cancel);
        let _ = timeout_timer.join();

        let logical_plugin_id = component_path
            .parent()
            .and_then(|path| path.parent())
            .and_then(|path| path.file_name())
            .and_then(|value| value.to_str())
            .unwrap_or("unknown");

        match result {
            Ok(Ok(value)) => {
                self.record_duration(logical_plugin_id, duration);
                Ok(value)
            }
            Ok(Err(err)) => {
                let error = format!("plugin returned error: {err}");
                self.record_error(logical_plugin_id, &error, duration);
                Err(error)
            }
            Err(err) => {
                self.record_error(logical_plugin_id, &err, duration);
                Err(err)
            }
        }
    }

    fn load_component(&self, component_path: &PathBuf) -> Result<Arc<Component>, String> {
        if let Some(component) = self
            .components
            .lock()
            .map_err(|_| "plugin component cache poisoned".to_string())?
            .get(component_path)
            .cloned()
        {
            return Ok(component);
        }

        let component = Arc::new(Component::from_file(&self.engine, component_path).map_err(
            |err| {
                format!(
                    "component load failed for {}: {err}",
                    component_path.display()
                )
            },
        )?);

        let mut cache = self
            .components
            .lock()
            .map_err(|_| "plugin component cache poisoned".to_string())?;
        let entry = cache
            .entry(component_path.clone())
            .or_insert_with(|| Arc::clone(&component));

        Ok(Arc::clone(entry))
    }

    fn registry(&self) -> Result<PluginRegistry, String> {
        let mut registry = self
            .registry
            .lock()
            .map_err(|_| "plugin registry cache poisoned".to_string())?;
        if registry.is_none() {
            let dirs = self
                .plugin_dirs
                .lock()
                .map_err(|_| "plugin directory cache poisoned".to_string())?
                .clone()
                .unwrap_or_else(default_plugin_dirs);
            *registry = Some(PluginRegistry::discover_from_dirs(&dirs)?);
        }

        registry
            .as_ref()
            .cloned()
            .ok_or_else(|| "plugin registry unavailable".to_string())
    }

    fn record_audit(
        &self,
        plugin_id: &str,
        command: &str,
        capability: Option<PluginCapability>,
        decision: &str,
        reason: Option<&str>,
        duration_ms: Option<u128>,
    ) {
        if let Ok(mut audit) = self.audit_log.lock() {
            audit.push(PluginAuditRecord {
                plugin_id: plugin_id.to_string(),
                command: command.to_string(),
                capability,
                decision: decision.to_string(),
                reason: reason.map(ToString::to_string),
                duration_ms,
                created_at: chrono::Utc::now().to_rfc3339(),
            });
            const MAX_AUDIT_RECORDS: usize = 500;
            if audit.len() > MAX_AUDIT_RECORDS {
                let excess = audit.len() - MAX_AUDIT_RECORDS;
                audit.drain(0..excess);
            }
        }
    }

    fn metric_entry<'a>(
        metrics: &'a mut HashMap<String, PluginMetrics>,
        plugin_id: &str,
    ) -> &'a mut PluginMetrics {
        metrics
            .entry(plugin_id.to_string())
            .or_insert_with(|| PluginMetrics {
                plugin_id: plugin_id.to_string(),
                ..PluginMetrics::default()
            })
    }

    fn record_allowed(&self, plugin_id: &str) {
        if let Ok(mut metrics) = self.metrics.lock() {
            let entry = Self::metric_entry(&mut metrics, plugin_id);
            entry.calls += 1;
            entry.allowed += 1;
            entry.last_called_at = Some(chrono::Utc::now().to_rfc3339());
        }
    }

    fn record_denied(&self, plugin_id: &str) {
        if let Ok(mut metrics) = self.metrics.lock() {
            let entry = Self::metric_entry(&mut metrics, plugin_id);
            entry.calls += 1;
            entry.denied += 1;
            entry.last_called_at = Some(chrono::Utc::now().to_rfc3339());
        }
    }

    fn record_duration(&self, plugin_id: &str, duration_ms: u128) {
        if let Ok(mut metrics) = self.metrics.lock() {
            let entry = Self::metric_entry(&mut metrics, plugin_id);
            entry.total_duration_ms += duration_ms;
            entry.last_called_at = Some(chrono::Utc::now().to_rfc3339());
        }
    }

    fn record_error(&self, plugin_id: &str, error: &str, duration_ms: u128) {
        if let Ok(mut metrics) = self.metrics.lock() {
            let entry = Self::metric_entry(&mut metrics, plugin_id);
            entry.errors += 1;
            if is_timeout_error(error) {
                entry.timeouts += 1;
            }
            entry.total_duration_ms += duration_ms;
            entry.last_error = Some(error.to_string());
            entry.last_called_at = Some(chrono::Utc::now().to_rfc3339());
        }
    }
}

/// The epoch deadline trap is how a plugin timeout surfaces.
pub fn is_timeout_error(error: &str) -> bool {
    error.to_ascii_lowercase().contains("epoch")
}

fn is_command_not_implemented_error(error: &str) -> bool {
    error
        .to_ascii_lowercase()
        .contains("command not implemented")
}

fn is_message_transform_plugin(plugin_id: &str) -> bool {
    matches!(plugin_id, "satchel-plugin-pgp")
}

fn transform_pgp_message(input: &MessageTransformInput) -> MessageTransformOutput {
    let text = input.body_text.trim();
    if text.is_empty() {
        return MessageTransformOutput::default();
    }

    if text.contains("-----BEGIN PGP SIGNED MESSAGE-----") {
        if let Some(plain) = extract_clearsigned_content(text) {
            let snippet = first_non_empty_line(&plain).unwrap_or_else(|| input.snippet.clone());
            return MessageTransformOutput {
                snippet: Some(snippet),
                body_text: Some(plain),
                labels: vec!["pgp-signed".to_string()],
                ..MessageTransformOutput::default()
            };
        }
    }

    if text.contains("-----BEGIN PGP MESSAGE-----") {
        let state = if input.has_pgp_private_key {
            "key_available_locked"
        } else {
            "encrypted_no_key"
        };

        return MessageTransformOutput {
            labels: vec!["pgp-encrypted".to_string()],
            pgp_state: Some(state.to_string()),
            ..MessageTransformOutput::default()
        };
    }

    MessageTransformOutput::default()
}

fn extract_clearsigned_content(text: &str) -> Option<String> {
    let start_marker = "-----BEGIN PGP SIGNED MESSAGE-----";
    let sig_marker = "-----BEGIN PGP SIGNATURE-----";
    let start = text.find(start_marker)?;
    let after_start = &text[start + start_marker.len()..];

    let body_start = if let Some(pos) = after_start.find("\n\n") {
        pos + 2
    } else {
        // CRLF text never matches the bare "\n\n" above, so this branch is what
        // handles it; a message carrying neither separator has no body at all.
        after_start.find("\r\n\r\n")? + 4
    };

    let content_and_sig = &after_start[body_start..];
    let sig_pos = content_and_sig.find(sig_marker)?;
    let signed_text = &content_and_sig[..sig_pos];

    let normalized = signed_text
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .lines()
        .map(|line| line.strip_prefix("- ").unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string();

    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

fn first_non_empty_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Path to a built plugin component, or None when plugins were never built.
    fn component_artifact(plugin_dir: &str) -> Option<PathBuf> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()?
            .join("plugins")
            .join(plugin_dir)
            .join("dist")
            .join("component.wasm");
        path.is_file().then_some(path)
    }

    /// CI sets this so a missing artifact is a hard failure rather than a
    /// silent skip — a test that never runs is as useless as no test.
    fn artifacts_required() -> bool {
        std::env::var("SATCHEL_REQUIRE_PLUGIN_ARTIFACTS")
            .map(|value| matches!(value.trim(), "1" | "true" | "yes"))
            .unwrap_or(false)
    }

    fn require_or_skip(plugin_dir: &str) -> Option<PathBuf> {
        match component_artifact(plugin_dir) {
            Some(path) => Some(path),
            None if artifacts_required() => panic!(
                "plugins/{plugin_dir}/dist/component.wasm is missing. \
                 Run `npm run plugins:build` before the test suite."
            ),
            None => {
                eprintln!(
                    "[skip] plugins/{plugin_dir}/dist/component.wasm not built \
                     — run `npm run plugins:build` to exercise the WASM path"
                );
                None
            }
        }
    }

    fn sample_input(body_text: &str, has_key: bool) -> MessageTransformInput {
        MessageTransformInput {
            subject: "Vertrag".into(),
            snippet: "…".into(),
            body_text: body_text.into(),
            body_html: None,
            has_pgp_private_key: has_key,
        }
    }

    const CLEARSIGNED: &str = "-----BEGIN PGP SIGNED MESSAGE-----\nHash: SHA256\n\n\
Hallo Alex,\n- - nicht einrücken\nbis bald\n\
-----BEGIN PGP SIGNATURE-----\nabc\n-----END PGP SIGNATURE-----";

    const ENCRYPTED: &str = "-----BEGIN PGP MESSAGE-----\nhQIMA…\n-----END PGP MESSAGE-----";

    // ---- The WASM path: does the plugin system actually work end to end? ----

    #[test]
    fn a_third_timeout_in_a_row_switches_the_plugin_off_until_switched_back_on() {
        let runtime = PluginRuntime::default();
        let timeout = "plugin returned error: epoch deadline reached";

        assert!(!runtime.record_transform_outcome("slow", Some(timeout)));
        assert!(!runtime.record_transform_outcome("slow", Some(timeout)));
        assert!(!runtime.record_transform_outcome("slow", Some("decode failed")));
        assert!(!runtime.record_transform_outcome("slow", Some(timeout)));
        assert!(!runtime.record_transform_outcome("slow", Some(timeout)));
        assert!(runtime.record_transform_outcome("slow", Some(timeout)));
        assert_eq!(
            runtime.disabled.lock().unwrap().get("slow"),
            Some(&PluginDisabledReason::Timeouts)
        );

        runtime.set_disabled_plugins(HashMap::new()).unwrap();
        assert!(runtime.disabled.lock().unwrap().is_empty());
    }

    #[test]
    fn loads_and_runs_a_real_component_through_wasmtime() {
        let Some(path) = require_or_skip("pgp-support") else {
            return;
        };
        let runtime = PluginRuntime::default();
        let payload = serde_json::to_string(&sample_input(CLEARSIGNED, false)).unwrap();

        let raw = runtime
            .execute_component(&path, "transform-message", &payload)
            .expect("component should execute");
        let output: MessageTransformOutput =
            serde_json::from_str(&raw).expect("component output should decode");

        let body = output
            .body_text
            .expect("clearsigned body should be unwrapped");
        assert!(body.contains("Hallo Alex"));
        // Dash-escaping per RFC 4880 must be undone.
        assert!(body.contains("- nicht einrücken"));
        assert!(!body.contains("BEGIN PGP SIGNATURE"));
    }

    #[test]
    fn concurrent_component_executions_keep_their_own_timeouts() {
        let Some(path) = require_or_skip("pgp-support") else {
            return;
        };
        let runtime = Arc::new(PluginRuntime::default());
        let payload = serde_json::to_string(&sample_input(CLEARSIGNED, false)).unwrap();

        // The epoch deadline is engine-global; before executions were
        // serialized, overlapping runs could fire each other's timeout trap.
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let runtime = Arc::clone(&runtime);
                let path = path.clone();
                let payload = payload.clone();
                std::thread::spawn(move || {
                    runtime.execute_component(&path, "transform-message", &payload)
                })
            })
            .collect();
        for handle in handles {
            let raw = handle
                .join()
                .expect("execution thread should not panic")
                .expect("concurrent execution must succeed with its own full timeout window");
            let output: MessageTransformOutput = serde_json::from_str(&raw).unwrap();
            assert!(
                output
                    .body_text
                    .as_deref()
                    .is_some_and(|body| body.contains("Hallo Alex")),
                "concurrent execution must produce the unwrapped clearsigned body"
            );
        }
    }

    #[test]
    fn component_reports_encrypted_state_from_host_supplied_key_availability() {
        let Some(path) = require_or_skip("pgp-support") else {
            return;
        };
        let runtime = PluginRuntime::default();

        for (has_key, expected) in [(false, "encrypted_no_key"), (true, "key_available_locked")] {
            let payload = serde_json::to_string(&sample_input(ENCRYPTED, has_key)).unwrap();
            let raw = runtime
                .execute_component(&path, "transform-message", &payload)
                .expect("component should execute");
            let output: MessageTransformOutput = serde_json::from_str(&raw).unwrap();
            assert_eq!(output.pgp_state.as_deref(), Some(expected));
            assert!(output.labels.iter().any(|label| label == "pgp-encrypted"));
        }
    }

    #[test]
    fn component_rejects_unknown_commands_recognisably() {
        let Some(path) = require_or_skip("pgp-support") else {
            return;
        };
        let runtime = PluginRuntime::default();

        let err = runtime
            .execute_component(&path, "does-not-exist", "{}")
            .expect_err("unknown command must fail");
        // The caller relies on this shape to cache "plugin can't do this".
        assert!(
            is_command_not_implemented_error(&err),
            "error should be recognised as not-implemented, got: {err}"
        );
    }

    #[test]
    fn component_errors_are_recorded_in_metrics() {
        let Some(path) = require_or_skip("pgp-support") else {
            return;
        };
        let runtime = PluginRuntime::default();
        let _ = runtime.execute_component(&path, "does-not-exist", "{}");

        let metrics = runtime.metrics().expect("metrics readable");
        assert!(
            metrics.iter().any(|entry| entry.errors > 0),
            "a failing plugin call should be visible in metrics"
        );
    }

    #[test]
    fn every_shipped_plugin_component_loads() {
        // Guards against a plugin that builds but produces an invalid component.
        for plugin_dir in [
            "pgp-support",
            "auto-unsubscribe",
            "calendar-sync",
            "github-integration",
        ] {
            let Some(path) = require_or_skip(plugin_dir) else {
                continue;
            };
            let runtime = PluginRuntime::default();
            runtime
                .load_component(&path)
                .unwrap_or_else(|err| panic!("{plugin_dir} failed to load: {err}"));
        }
    }

    // ---- The native fallback used when no component is built ----

    #[test]
    fn native_fallback_matches_the_component_for_clearsigned_mail() {
        let output = transform_pgp_message(&sample_input(CLEARSIGNED, false));
        let body = output
            .body_text
            .expect("clearsigned body should be unwrapped");
        assert!(body.contains("Hallo Alex"));
        assert!(body.contains("- nicht einrücken"));
    }

    #[test]
    fn native_fallback_matches_the_component_for_encrypted_mail() {
        assert_eq!(
            transform_pgp_message(&sample_input(ENCRYPTED, false))
                .pgp_state
                .as_deref(),
            Some("encrypted_no_key")
        );
        assert_eq!(
            transform_pgp_message(&sample_input(ENCRYPTED, true))
                .pgp_state
                .as_deref(),
            Some("key_available_locked")
        );
    }

    #[test]
    fn native_fallback_leaves_ordinary_mail_untouched() {
        let output = transform_pgp_message(&sample_input("Ganz normale Mail.", false));
        assert!(output.body_text.is_none());
        assert!(output.pgp_state.is_none());
        assert!(output.labels.is_empty());
    }

    #[test]
    fn registry_discovers_the_workspace_plugins() {
        let runtime = PluginRuntime::default();
        let plugins = runtime.list_plugins().expect("registry should be readable");
        assert!(
            plugins
                .iter()
                .any(|plugin| plugin.id == "satchel-plugin-pgp"),
            "expected satchel-plugin-pgp in registry, got: {:?}",
            plugins.iter().map(|p| &p.id).collect::<Vec<_>>()
        );
    }

    #[test]
    fn plugins_are_not_discovered_from_the_working_directory() {
        // Unconfigured, a debug build falls back to the workspace checkout
        // resolved from the process' working directory — the path a release
        // build must never have. The test above shows that fallback finding
        // the shipped plugins; naming a directory has to take it out of play.
        let elsewhere = tempfile::tempdir().expect("temp dir");
        let runtime = PluginRuntime::default();
        runtime
            .set_plugin_dirs(vec![elsewhere.path().to_path_buf()])
            .expect("plugin directories should be configurable");

        assert!(runtime
            .list_plugins()
            .expect("registry should be readable")
            .is_empty());
    }

    #[test]
    fn every_command_the_host_dispatches_has_a_capability_entry() {
        for command in [
            "transform-message",
            "handle-event",
            "render-slot:top_toolbar",
            "render-slot:thread_view_panel",
            "render-slot:sidebar_section",
            "render-slot:assistant_context_panel",
        ] {
            required_capability(command)
                .unwrap_or_else(|_| panic!("{command} must be in the capability table"));
        }
    }

    #[test]
    fn an_unlisted_command_is_refused() {
        let error = required_capability("drop-database").expect_err("unlisted command");
        assert!(
            error.contains("backend.errors.pluginCommandUnknown"),
            "{error}"
        );
        assert!(error.contains("drop-database"), "{error}");
    }

    #[test]
    fn only_enforced_capabilities_are_reported_as_enforced() {
        assert_eq!(
            enforced_plugin_capabilities(),
            vec![PluginCapability::ReadThreads]
        );
    }

    #[test]
    fn commands_require_their_own_capability() {
        let runtime = PluginRuntime::default();
        let calendar = "satchel-plugin-calendar-sync";
        let github = "satchel-plugin-github-integration";
        // The calendar plugin renders a slot the host hands no message data,
        // so it needs no grant; before the table it was denied for lacking
        // `read_threads`.
        runtime
            .set_permission_grants(HashMap::new())
            .expect("grants should be settable");
        runtime
            .ensure_capability(calendar, "render-slot:sidebar_section")
            .expect("a data-free slot needs no grant");
        let denied = runtime
            .ensure_capability(calendar, "transform-message")
            .expect_err("message content needs read_threads");
        assert!(
            denied.contains("backend.errors.pluginCapabilityNotGranted"),
            "{denied}"
        );
        assert!(denied.contains("read_threads"), "{denied}");

        // A plugin that declares `network_access` but was never granted
        // `read_threads` still may not see message content.
        runtime
            .set_permission_grants(HashMap::from([(
                github.to_string(),
                HashSet::from([PluginCapability::NetworkAccess]),
            )]))
            .expect("grants should be settable");
        let denied = runtime
            .ensure_capability(github, "render-slot:thread_view_panel")
            .expect_err("a thread panel needs read_threads");
        assert!(
            denied.contains("backend.errors.pluginCapabilityNotGranted"),
            "{denied}"
        );

        runtime
            .set_permission_grants(HashMap::from([(
                github.to_string(),
                HashSet::from([PluginCapability::ReadThreads]),
            )]))
            .expect("grants should be settable");
        runtime
            .ensure_capability(github, "render-slot:thread_view_panel")
            .expect("granted read_threads opens the thread panel");
    }

    #[test]
    fn a_missing_declaration_is_named_in_the_message_key() {
        let runtime = PluginRuntime::default();
        // The calendar plugin declares only `read_threads`; strip the grant
        // check by granting it, so the declaration check is what answers.
        runtime
            .set_permission_grants(HashMap::new())
            .expect("grants should be settable");
        let error = runtime
            .ensure_capability("satchel-plugin-calendar-sync", "handle-event")
            .expect_err("no grant, no dispatch");
        assert!(error.contains("read_threads"), "{error}");
    }

    #[test]
    fn an_unregistered_plugin_is_refused_before_anything_else() {
        let runtime = PluginRuntime::default();
        let error = runtime
            .ensure_capability("not-a-plugin", "handle-event")
            .expect_err("unregistered plugin");
        assert!(
            error.contains("backend.errors.pluginNotRegistered"),
            "{error}"
        );
    }

    #[test]
    fn a_transform_output_passes_the_same_gate_as_execute() {
        let Some(_path) = require_or_skip("pgp-support") else {
            return;
        };
        let runtime = PluginRuntime::default();
        runtime
            .set_permission_grants(HashMap::from([(
                "satchel-plugin-pgp".to_string(),
                HashSet::from([PluginCapability::ReadThreads]),
            )]))
            .expect("grants should be settable");

        let output = runtime
            .transform_message("satchel-plugin-pgp", &sample_input(ENCRYPTED, false))
            .expect("the shipped plugin stays within what a plugin may assert");

        assert_eq!(output.pgp_state.as_deref(), Some("encrypted_no_key"));
    }
}

#[cfg(test)]
mod output_validation_tests {
    use super::*;

    #[test]
    fn rejects_oversized_plugin_output() {
        let oversized = "x".repeat(MAX_PLUGIN_OUTPUT_BYTES + 1);
        let error = validate_plugin_output("some-plugin", &oversized)
            .expect_err("oversized output must be rejected");
        assert!(error.contains("the limit is"), "error: {error}");
        assert!(error.contains("some-plugin"), "error: {error}");
    }

    #[test]
    fn accepts_output_at_the_size_limit() {
        let at_limit = "x".repeat(MAX_PLUGIN_OUTPUT_BYTES);
        validate_plugin_output("some-plugin", &at_limit).expect("limit-sized output is fine");
    }

    fn transform_output(state: Option<&str>) -> MessageTransformOutput {
        MessageTransformOutput {
            pgp_state: state.map(ToString::to_string),
            ..MessageTransformOutput::default()
        }
    }

    #[test]
    fn rejects_a_signature_verdict_a_plugin_cannot_have_established() {
        // A substituted plugin that could write these would put "signed by X"
        // under an arbitrary mail; the host decides them against real keys.
        for forged in [
            "signature_valid",
            "signature_unchained",
            "signature_identity_mismatch",
            "signature_invalid",
            "signature_unknown_key",
            "decrypted",
        ] {
            let error =
                validate_message_transform_output("some-plugin", &transform_output(Some(forged)))
                    .expect_err("a host-owned verdict must not come from a plugin");
            assert!(error.contains(forged), "error: {error}");
        }
    }

    #[test]
    fn accepts_the_states_the_shipped_plugin_reports() {
        for state in ["encrypted_no_key", "key_available_locked"] {
            validate_message_transform_output("some-plugin", &transform_output(Some(state)))
                .unwrap_or_else(|error| panic!("{state} must stay allowed: {error}"));
        }
        validate_message_transform_output("some-plugin", &transform_output(None))
            .expect("a transform that says nothing about PGP is fine");
    }

    #[test]
    fn rejects_a_plugin_supplied_signer_identity() {
        let named = MessageTransformOutput {
            pgp_signer_name: Some("Alex Morgan".into()),
            ..MessageTransformOutput::default()
        };
        assert!(validate_message_transform_output("some-plugin", &named).is_err());

        let fingerprinted = MessageTransformOutput {
            pgp_signer_fingerprint: Some("DEADBEEF".into()),
            ..MessageTransformOutput::default()
        };
        assert!(validate_message_transform_output("some-plugin", &fingerprinted).is_err());
    }

    #[test]
    fn rejects_structurally_oversized_labels() {
        let many = MessageTransformOutput {
            labels: vec!["tag".to_string(); MAX_PLUGIN_TRANSFORM_LABELS + 1],
            ..MessageTransformOutput::default()
        };
        assert!(validate_message_transform_output("some-plugin", &many).is_err());

        let long = MessageTransformOutput {
            labels: vec!["x".repeat(MAX_PLUGIN_TRANSFORM_LABEL_BYTES + 1)],
            ..MessageTransformOutput::default()
        };
        assert!(validate_message_transform_output("some-plugin", &long).is_err());
    }
}

#[cfg(test)]
mod compilation_cache_tests {
    use super::*;

    #[test]
    fn unchanged_components_hit_the_disk_cache_across_engines() {
        let directory = tempfile::tempdir().unwrap();
        let component = b"(component (core module (func (export \"run\"))))";
        let first = PluginRuntime::with_compilation_cache(Some(directory.path().to_path_buf()));
        Component::new(&first.engine, component).unwrap();
        assert_eq!(first.compilation_cache.as_ref().unwrap().cache_misses(), 1);
        drop(first);
        let second = PluginRuntime::with_compilation_cache(Some(directory.path().to_path_buf()));
        Component::new(&second.engine, component).unwrap();
        assert_eq!(second.compilation_cache.as_ref().unwrap().cache_hits(), 1);
        Component::new(
            &second.engine,
            b"(component (core module (func (export \"changed\"))))",
        )
        .unwrap();
        assert_eq!(second.compilation_cache.as_ref().unwrap().cache_misses(), 1);
    }
}
