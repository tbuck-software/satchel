use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/plugins/registry/"))]
pub struct RegisteredPlugin {
    pub id: String,
    pub version: String,
    pub enabled: bool,
    pub root_dir: PathBuf,
    pub component_path: Option<PathBuf>,
    pub api_version: String,
    pub capabilities: Vec<PluginCapability>,
    pub event_subscriptions: Vec<String>,
    pub ui_slots: Vec<String>,
    pub trust: PluginTrustLevel,
    pub signature: Option<String>,
    pub manifest_errors: Vec<String>,
    pub signature_verified: bool,
    /// Switched off although the manifest is fine. `enabled` already accounts for it.
    #[serde(default)]
    pub disabled_by: Option<PluginDisabledReason>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/plugins/registry/"))]
pub enum PluginDisabledReason {
    User,
    /// Stopped after [`AUTO_DISABLE_AFTER_TIMEOUTS`] timeouts in a row.
    Timeouts,
}

/// Timeouts in a row after which a plugin is switched off. Each one holds the
/// global execution lock for the full deadline, for every message it touches.
pub const AUTO_DISABLE_AFTER_TIMEOUTS: u32 = 3;

impl RegisteredPlugin {
    /// Applies the user's switch on top of what the manifest allows.
    pub fn with_disabled_by(mut self, disabled_by: Option<PluginDisabledReason>) -> Self {
        self.disabled_by = disabled_by;
        self.enabled = self.manifest_errors.is_empty() && disabled_by.is_none();
        self
    }
}

/// Consecutive timeouts per plugin; any other outcome resets the count.
#[derive(Debug, Default)]
pub struct TimeoutStrikes {
    counts: HashMap<String, u32>,
}

impl TimeoutStrikes {
    /// `true` when this timeout is the one that should switch the plugin off.
    pub fn record(&mut self, plugin_id: &str, timed_out: bool) -> bool {
        if !timed_out {
            self.counts.remove(plugin_id);
            return false;
        }
        let count = self.counts.entry(plugin_id.to_string()).or_default();
        *count += 1;
        if *count >= AUTO_DISABLE_AFTER_TIMEOUTS {
            self.counts.remove(plugin_id);
            return true;
        }
        false
    }

    pub fn reset(&mut self, plugin_id: &str) {
        self.counts.remove(plugin_id);
    }
}

#[derive(Debug, Default, Clone)]
pub struct PluginRegistry {
    plugins: HashMap<String, RegisteredPlugin>,
}

#[derive(Debug, Deserialize)]
struct CargoPackage {
    name: String,
    version: String,
    metadata: Option<CargoPackageMetadata>,
}

#[derive(Debug, Deserialize)]
struct CargoManifest {
    package: CargoPackage,
}

#[derive(Debug, Deserialize)]
struct CargoPackageMetadata {
    satchel: Option<SatchelPluginMetadata>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct SatchelPluginMetadata {
    api_version: Option<String>,
    capabilities: Option<Vec<PluginCapability>>,
    events: Option<Vec<String>>,
    ui_slots: Option<Vec<String>>,
    trust: Option<PluginTrustLevel>,
    signature: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/plugins/registry/"))]
pub enum PluginCapability {
    ReadThreads,
    WriteThreads,
    WriteTags,
    WorkflowTransition,
    SendMail,
    ReadContacts,
    NetworkAccess,
    FilesystemAccess,
}

impl PluginCapability {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReadThreads => "read_threads",
            Self::WriteThreads => "write_threads",
            Self::WriteTags => "write_tags",
            Self::WorkflowTransition => "workflow_transition",
            Self::SendMail => "send_mail",
            Self::ReadContacts => "read_contacts",
            Self::NetworkAccess => "network_access",
            Self::FilesystemAccess => "filesystem_access",
        }
    }
}

impl std::str::FromStr for PluginCapability {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim() {
            "read_threads" => Ok(Self::ReadThreads),
            "write_threads" => Ok(Self::WriteThreads),
            "write_tags" => Ok(Self::WriteTags),
            "workflow_transition" => Ok(Self::WorkflowTransition),
            "send_mail" => Ok(Self::SendMail),
            "read_contacts" => Ok(Self::ReadContacts),
            "network_access" => Ok(Self::NetworkAccess),
            "filesystem_access" => Ok(Self::FilesystemAccess),
            other => Err(format!("unknown plugin capability `{other}`")),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/plugins/registry/"))]
pub enum PluginTrustLevel {
    Verified,
    Community,
    #[default]
    LocalUnsigned,
}

impl PluginTrustLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Verified => "verified",
            Self::Community => "community",
            Self::LocalUnsigned => "local_unsigned",
        }
    }
}

pub const SATCHEL_PLUGIN_API_VERSION: &str = "1.0";

impl PluginRegistry {
    pub fn register(&mut self, plugin: RegisteredPlugin) {
        self.plugins.insert(plugin.id.clone(), plugin);
    }

    pub fn list(&self) -> Vec<RegisteredPlugin> {
        let mut values = self.plugins.values().cloned().collect::<Vec<_>>();
        values.sort_by(|a, b| a.id.cmp(&b.id));
        values
    }

    pub fn get(&self, plugin_id: &str) -> Option<RegisteredPlugin> {
        self.plugins.get(plugin_id).cloned()
    }

    pub fn discover_from_dir(root: &Path) -> Result<Self, String> {
        Self::discover_from_dir_with(root, allow_unsigned_plugins())
    }

    /// Release packaging gate. This never waives signatures, even in a debug build.
    /// Check the shipped resource directory, rather than a developer's source tree.
    pub fn verify_release_packages(root: &Path) -> Result<Vec<String>, String> {
        let plugins = Self::discover_from_dir_with(root, false)?.list();
        let expected = [
            "satchel-plugin-auto-unsubscribe",
            "satchel-plugin-calendar-sync",
            "satchel-plugin-github-integration",
            "satchel-plugin-pgp",
        ];
        for id in expected {
            let plugin = plugins
                .iter()
                .find(|plugin| plugin.id == id)
                .ok_or_else(|| format!("Missing bundled plugin: {id}"))?;
            if !plugin.enabled || !plugin.signature_verified {
                return Err(format!(
                    "Bundled plugin {id} failed release policy: {:?}",
                    plugin.manifest_errors
                ));
            }
        }
        if plugins.len() != expected.len() {
            return Err("Unexpected plugin in release package".to_string());
        }
        Ok(plugins.into_iter().map(|plugin| plugin.id).collect())
    }

    /// `allow_unsigned` is a parameter so a debug test build can drive the
    /// release decision without touching the process environment.
    fn discover_from_dir_with(root: &Path, allow_unsigned: bool) -> Result<Self, String> {
        let mut registry = PluginRegistry::default();
        if !root.exists() {
            return Ok(registry);
        }

        for entry in fs::read_dir(root).map_err(|err| err.to_string())? {
            let entry = entry.map_err(|err| err.to_string())?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let manifest_path = path.join("Cargo.toml");
            if !manifest_path.exists() {
                continue;
            }

            let manifest = fs::read_to_string(&manifest_path).map_err(|err| err.to_string())?;
            let parsed: CargoManifest = toml::from_str(&manifest).map_err(|err| err.to_string())?;

            let package_name = parsed.package.name;
            let package_version = parsed.package.version;
            let metadata = parsed
                .package
                .metadata
                .and_then(|metadata| metadata.satchel)
                .unwrap_or_default();
            let api_version = metadata
                .api_version
                .unwrap_or_else(|| SATCHEL_PLUGIN_API_VERSION.to_string());
            let capabilities = metadata.capabilities.unwrap_or_default();
            let event_subscriptions = metadata.events.unwrap_or_default();
            let ui_slots = metadata.ui_slots.unwrap_or_default();
            let trust = metadata.trust.unwrap_or_default();
            let signature = metadata.signature;
            let component_path = resolve_component_artifact(&path, &package_name);
            let signature_check =
                verify_plugin_signature(component_path.as_deref(), signature.as_deref());
            let signature_verified = matches!(signature_check, SignatureCheck::TrustedSignature);
            let manifest_errors = validate_plugin_manifest(
                &package_name,
                &api_version,
                &capabilities,
                &ui_slots,
                trust,
                signature.as_deref(),
                signature_check,
                allow_unsigned,
            );
            let enabled = manifest_errors.is_empty();
            registry.register(RegisteredPlugin {
                id: package_name.clone(),
                version: package_version,
                enabled,
                root_dir: path.clone(),
                component_path,
                api_version,
                capabilities,
                event_subscriptions,
                ui_slots,
                trust,
                signature,
                manifest_errors,
                signature_verified,
                disabled_by: None,
            });
        }

        Ok(registry)
    }

    /// Merges the configured plugin directories. The first directory that
    /// carries an id wins, so the bundle's own copy outranks a plugin dropped
    /// into the writable app-data directory under the same name.
    pub fn discover_from_dirs(roots: &[PathBuf]) -> Result<Self, String> {
        let mut merged = PluginRegistry::default();
        for root in roots {
            for plugin in Self::discover_from_dir(root)?.list() {
                merged.plugins.entry(plugin.id.clone()).or_insert(plugin);
            }
        }

        Ok(merged)
    }
}

/// Where plugins may be loaded from when the host has not named a directory.
///
/// A developer build keeps reading the workspace checkout; a release build
/// loads nothing rather than trusting whatever the process happens to have as
/// its working directory.
pub fn default_plugin_dirs() -> Vec<PathBuf> {
    #[cfg(debug_assertions)]
    {
        resolve_workspace_plugins_dir().into_iter().collect()
    }
    #[cfg(not(debug_assertions))]
    {
        Vec::new()
    }
}

/// Outcome of checking a plugin's `signature` metadata against its component
/// artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignatureCheck {
    /// No signature metadata at all.
    Missing,
    /// `sha256:` digest matched the artifact. Integrity only; it never grants
    /// trust because whoever tampers with the artifact can recompute it.
    DigestMatched,
    /// `sha256:` digest did not match the artifact.
    DigestMismatched,
    /// `ed25519:` signature verified against a trusted Satchel signing key.
    TrustedSignature,
    /// `ed25519:` signature present but invalid, or the signing key is not
    /// trusted.
    UntrustedSignature,
}

/// Public Ed25519 keys (raw 32-byte hex) whose signatures mark a plugin as
/// `verified`. The entry is the public half of the repository signing key;
/// the private half lives outside version control (`.private/plugin-signing-key.pem`).
const SATCHEL_PLUGIN_TRUSTED_KEYS: &[&str] =
    &["c4cc26c1940e0f23cfea9aea1847b901e87606629709bb22bda89eaabc4343f0"];

fn verify_plugin_signature(
    component_path: Option<&Path>,
    signature: Option<&str>,
) -> SignatureCheck {
    let Some(signature) = signature.map(str::trim).filter(|value| !value.is_empty()) else {
        return SignatureCheck::Missing;
    };
    let Some(component_path) = component_path else {
        return if signature.starts_with("sha256:") {
            SignatureCheck::DigestMismatched
        } else {
            SignatureCheck::UntrustedSignature
        };
    };
    let Ok(bytes) = fs::read(component_path) else {
        return if signature.starts_with("sha256:") {
            SignatureCheck::DigestMismatched
        } else {
            SignatureCheck::UntrustedSignature
        };
    };
    if let Some(expected) = signature.strip_prefix("sha256:") {
        let actual = Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        return if actual.eq_ignore_ascii_case(expected.trim()) {
            SignatureCheck::DigestMatched
        } else {
            SignatureCheck::DigestMismatched
        };
    }
    if let Some(encoded) = signature.strip_prefix("ed25519:") {
        return if verify_ed25519_signature(&bytes, encoded.trim()) {
            SignatureCheck::TrustedSignature
        } else {
            SignatureCheck::UntrustedSignature
        };
    }
    SignatureCheck::UntrustedSignature
}

fn verify_ed25519_signature(message: &[u8], encoded_signature: &str) -> bool {
    let Some(signature) = decode_hex_64(encoded_signature) else {
        return false;
    };
    trusted_plugin_keys().iter().any(|key| {
        ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, key)
            .verify(message, &signature)
            .is_ok()
    })
}

/// Embedded release keys plus any keys added through
/// `SATCHEL_PLUGIN_TRUSTED_KEYS` (comma-separated hex), so forks and dev
/// setups can trust their own signer without patching this file.
fn trusted_plugin_keys() -> Vec<Vec<u8>> {
    let mut keys = SATCHEL_PLUGIN_TRUSTED_KEYS
        .iter()
        .filter_map(|key| decode_hex(key))
        .collect::<Vec<_>>();
    if let Some(extra) = crate::utils::environment::debug_env("SATCHEL_PLUGIN_TRUSTED_KEYS") {
        for key in extra.split(',') {
            if let Some(key) = decode_hex(key.trim()) {
                if key.len() == 32 {
                    keys.push(key);
                }
            }
        }
    }
    keys
}

fn decode_hex(value: &str) -> Option<Vec<u8>> {
    let value = value.trim();
    if !value.len().is_multiple_of(2) || value.is_empty() {
        return None;
    }
    (0..value.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&value[index..index + 2], 16).ok())
        .collect()
}

fn decode_hex_64(value: &str) -> Option<[u8; 64]> {
    decode_hex(value).and_then(|bytes| <[u8; 64]>::try_from(bytes).ok())
}

/// Only debug builds accept unsigned plugins or runtime trust overrides.
fn allow_unsigned_plugins() -> bool {
    match crate::utils::environment::debug_env("SATCHEL_ALLOW_UNSIGNED_PLUGINS") {
        Some(value) => !matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off"
        ),
        None => cfg!(debug_assertions),
    }
}

/// `allow_unsigned` is passed in rather than read from the environment so the
/// release behaviour can be exercised from a debug test build.
fn validate_plugin_manifest(
    package_name: &str,
    api_version: &str,
    capabilities: &[PluginCapability],
    ui_slots: &[String],
    trust: PluginTrustLevel,
    signature: Option<&str>,
    signature_check: SignatureCheck,
    allow_unsigned: bool,
) -> Vec<String> {
    let mut errors = Vec::new();
    if !is_api_compatible(api_version) {
        errors.push(format!(
            "plugin `{package_name}` targets incompatible Satchel plugin API `{api_version}`; host supports `{SATCHEL_PLUGIN_API_VERSION}`"
        ));
    }

    let mut seen = HashSet::new();
    for capability in capabilities {
        if !seen.insert(*capability) {
            errors.push(format!(
                "plugin `{package_name}` declares duplicate capability `{}`",
                capability.as_str()
            ));
        }
    }

    for slot in ui_slots {
        if !matches!(
            slot.as_str(),
            "top_toolbar" | "thread_view_panel" | "sidebar_section" | "assistant_context_panel"
        ) {
            errors.push(format!(
                "plugin `{package_name}` declares unsupported UI slot `{slot}`"
            ));
        }
    }

    // `trust` is self-declared in the plugin's own manifest, so it must not
    // decide whether the signature is checked — `trust = "community"` would
    // otherwise gate itself out of the check entirely. `allow_unsigned` is the
    // single waiver, and it does not extend to `verified`: claiming that level
    // without a signature is a false claim, not a developer convenience.
    let waived = allow_unsigned && !matches!(trust, PluginTrustLevel::Verified);
    if signature_check != SignatureCheck::TrustedSignature && !waived {
        let level = trust.as_str();
        errors.push(match signature {
            None => {
                format!("plugin `{package_name}` is unsigned and unsigned plugins are disabled")
            }
            Some(signature) if signature.trim().starts_with("sha256:") => format!(
                "plugin `{package_name}` declares trust `{level}` but a sha256 digest is not a signature; sign with `ed25519:` via `pnpm run plugin:sign`"
            ),
            Some(_) => format!(
                "plugin `{package_name}` declares trust `{level}` but has no valid signature from a trusted Satchel signing key"
            ),
        });
    }

    errors
}

fn is_api_compatible(api_version: &str) -> bool {
    let Some((major, _rest)) = api_version.split_once('.') else {
        return false;
    };
    let Some((host_major, _host_rest)) = SATCHEL_PLUGIN_API_VERSION.split_once('.') else {
        return false;
    };
    major == host_major
}

fn resolve_component_artifact(root: &Path, package_name: &str) -> Option<PathBuf> {
    let candidates = [
        root.join("component.wasm"),
        root.join("dist").join("component.wasm"),
        root.join("dist").join(format!("{package_name}.wasm")),
        root.join("dist")
            .join(format!("{}.wasm", package_name.replace('-', "_"))),
    ];

    candidates.into_iter().find(|path| path.exists())
}

/// Debug-only: the working directory is an attacker-influenced input (shell
/// alias, `.desktop` entry with `Path=`), so a shipped build never derives a
/// code-loading path from it.
#[cfg(debug_assertions)]
pub fn resolve_workspace_plugins_dir() -> Result<PathBuf, String> {
    let cwd = std::env::current_dir().map_err(|err| err.to_string())?;
    let plugins_dir = if cwd.ends_with("src-tauri") {
        cwd.parent()
            .map(|path| path.join("plugins"))
            .unwrap_or_else(|| cwd.join("plugins"))
    } else {
        cwd.join("plugins")
    };

    Ok(plugins_dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::signature::KeyPair;

    fn write_component(content: &[u8]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(dir.path().join("component.wasm"), content).expect("write artifact");
        dir
    }

    fn plugin(manifest_errors: Vec<String>) -> RegisteredPlugin {
        RegisteredPlugin {
            id: "satchel-plugin-example".into(),
            version: "1.0.0".into(),
            enabled: manifest_errors.is_empty(),
            root_dir: PathBuf::from("/plugins/example"),
            component_path: None,
            api_version: SATCHEL_PLUGIN_API_VERSION.into(),
            capabilities: Vec::new(),
            event_subscriptions: Vec::new(),
            ui_slots: Vec::new(),
            trust: PluginTrustLevel::Verified,
            signature: None,
            manifest_errors,
            signature_verified: true,
            disabled_by: None,
        }
    }

    #[test]
    fn a_switched_off_plugin_is_not_enabled_and_says_why() {
        let by_user = plugin(Vec::new()).with_disabled_by(Some(PluginDisabledReason::User));
        assert!(!by_user.enabled);
        assert_eq!(by_user.disabled_by, Some(PluginDisabledReason::User));
        assert!(by_user.manifest_errors.is_empty());

        let broken = plugin(vec!["bad api version".into()]);
        assert!(!broken.enabled);
        assert_eq!(broken.disabled_by, None);

        // The switch cannot turn on what the manifest rules out.
        assert!(!broken.clone().with_disabled_by(None).enabled);
        assert!(plugin(Vec::new()).with_disabled_by(None).enabled);
    }

    #[test]
    fn timeouts_in_a_row_switch_a_plugin_off_and_a_success_resets_them() {
        let mut strikes = TimeoutStrikes::default();
        for _ in 1..AUTO_DISABLE_AFTER_TIMEOUTS {
            assert!(!strikes.record("slow", true));
        }
        assert!(!strikes.record("slow", false), "a success in between");
        for _ in 1..AUTO_DISABLE_AFTER_TIMEOUTS {
            assert!(!strikes.record("slow", true));
        }
        assert!(strikes.record("slow", true));
        assert!(!strikes.record("other", true), "counted per plugin");
    }

    #[test]
    fn missing_signature_is_unsigned() {
        let dir = write_component(b"artifact");
        assert_eq!(
            verify_plugin_signature(Some(&dir.path().join("component.wasm")), None),
            SignatureCheck::Missing
        );
    }

    #[test]
    fn sha256_digest_is_integrity_only() {
        let dir = write_component(b"artifact");
        let path = dir.path().join("component.wasm");
        let digest = Sha256::digest(b"artifact")
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();

        assert_eq!(
            verify_plugin_signature(Some(&path), Some(&format!("sha256:{digest}"))),
            SignatureCheck::DigestMatched
        );
        // A digest never grants cryptographic trust, even when it matches.
        assert_ne!(
            verify_plugin_signature(Some(&path), Some(&format!("sha256:{digest}"))),
            SignatureCheck::TrustedSignature
        );
        assert_eq!(
            verify_plugin_signature(Some(&path), Some("sha256:deadbeef")),
            SignatureCheck::DigestMismatched
        );
    }

    #[test]
    fn tampered_artifact_fails_ed25519_verification() {
        let rng = ring::rand::SystemRandom::new();
        let pkcs8 = ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).expect("generate key");
        let key_pair =
            ring::signature::Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).expect("key pair");
        let public_hex = hex_encode(key_pair.public_key().as_ref());

        let artifact = b"artifact";
        let signature = key_pair.sign(artifact);
        let dir = write_component(artifact);
        let path = dir.path().join("component.wasm");

        std::env::set_var("SATCHEL_PLUGIN_TRUSTED_KEYS", &public_hex);
        let valid = verify_plugin_signature(
            Some(&path),
            Some(&format!("ed25519:{}", hex_encode(signature.as_ref()))),
        );
        std::env::remove_var("SATCHEL_PLUGIN_TRUSTED_KEYS");
        assert_eq!(
            valid,
            if cfg!(debug_assertions) {
                SignatureCheck::TrustedSignature
            } else {
                SignatureCheck::UntrustedSignature
            }
        );

        // Same signature over a modified artifact must not verify.
        let tampered_dir = write_component(b"artifacT");
        std::env::set_var("SATCHEL_PLUGIN_TRUSTED_KEYS", &public_hex);
        let tampered = verify_plugin_signature(
            Some(&tampered_dir.path().join("component.wasm")),
            Some(&format!("ed25519:{}", hex_encode(signature.as_ref()))),
        );
        std::env::remove_var("SATCHEL_PLUGIN_TRUSTED_KEYS");
        assert_eq!(tampered, SignatureCheck::UntrustedSignature);
    }

    #[test]
    fn untrusted_signing_key_is_rejected() {
        let rng = ring::rand::SystemRandom::new();
        let pkcs8 = ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).expect("generate key");
        let key_pair =
            ring::signature::Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).expect("key pair");
        let dir = write_component(b"artifact");

        let check = verify_plugin_signature(
            Some(&dir.path().join("component.wasm")),
            Some(&format!(
                "ed25519:{}",
                hex_encode(key_pair.sign(b"artifact").as_ref())
            )),
        );

        assert_eq!(check, SignatureCheck::UntrustedSignature);
    }

    #[test]
    fn malformed_signature_encoding_is_untrusted() {
        let dir = write_component(b"artifact");
        assert_eq!(
            verify_plugin_signature(
                Some(&dir.path().join("component.wasm")),
                Some("ed25519:not-hex"),
            ),
            SignatureCheck::UntrustedSignature
        );
        assert_eq!(
            verify_plugin_signature(
                Some(&dir.path().join("component.wasm")),
                Some("pgp:whatever"),
            ),
            SignatureCheck::UntrustedSignature
        );
    }

    fn manifest_errors_for(
        trust: PluginTrustLevel,
        signature: Option<&str>,
        signature_check: SignatureCheck,
        allow_unsigned: bool,
    ) -> Vec<String> {
        validate_plugin_manifest(
            "plugin",
            SATCHEL_PLUGIN_API_VERSION,
            &[],
            &[],
            trust,
            signature,
            signature_check,
            allow_unsigned,
        )
    }

    #[test]
    fn verified_trust_requires_trusted_signature() {
        let errors = manifest_errors_for(
            PluginTrustLevel::Verified,
            Some("sha256:abc"),
            SignatureCheck::DigestMatched,
            true,
        );
        assert!(errors
            .iter()
            .any(|error| error.contains("digest is not a signature")));

        // The unsigned waiver never reaches `verified`: a plugin that claims
        // the level without a signature is making a false claim, even in a
        // developer build.
        let errors = manifest_errors_for(
            PluginTrustLevel::Verified,
            None,
            SignatureCheck::Missing,
            true,
        );
        assert!(!errors.is_empty());

        let errors = manifest_errors_for(
            PluginTrustLevel::Verified,
            Some("ed25519:00"),
            SignatureCheck::TrustedSignature,
            true,
        );
        assert!(errors.is_empty());
    }

    #[test]
    fn no_trust_level_can_waive_the_signature_in_a_release_build() {
        // `trust` is written by the plugin itself, so none of its values may
        // decide whether the signature is checked.
        for trust in [
            PluginTrustLevel::Verified,
            PluginTrustLevel::Community,
            PluginTrustLevel::LocalUnsigned,
        ] {
            let errors = manifest_errors_for(trust, None, SignatureCheck::Missing, false);
            assert!(
                errors.iter().any(|error| error.contains("is unsigned")),
                "{} without a signature must be rejected, got: {errors:?}",
                trust.as_str()
            );

            // A matching sha256 digest is integrity, not provenance.
            let errors = manifest_errors_for(
                trust,
                Some("sha256:abc"),
                SignatureCheck::DigestMatched,
                false,
            );
            assert!(
                !errors.is_empty(),
                "{} with a digest instead of a signature must be rejected",
                trust.as_str()
            );

            assert!(manifest_errors_for(
                trust,
                Some("ed25519:00"),
                SignatureCheck::TrustedSignature,
                false
            )
            .is_empty());
        }
    }

    #[test]
    fn community_manifest_without_signature_stays_disabled() {
        let root = tempfile::tempdir().expect("temp dir");
        let plugin_dir = root.path().join("impostor");
        std::fs::create_dir_all(plugin_dir.join("dist")).expect("plugin dir");
        std::fs::write(plugin_dir.join("dist").join("component.wasm"), b"wasm")
            .expect("write artifact");
        std::fs::write(
            plugin_dir.join("Cargo.toml"),
            "[package]\nname = \"satchel-plugin-pgp\"\nversion = \"0.1.0\"\n\n\
             [package.metadata.satchel]\napi_version = \"1.0\"\ntrust = \"community\"\n",
        )
        .expect("write manifest");

        let registry = PluginRegistry::discover_from_dir_with(root.path(), false)
            .expect("discovery should succeed");
        let plugin = registry
            .get("satchel-plugin-pgp")
            .expect("the manifest is still registered, just not enabled");

        assert_eq!(plugin.trust, PluginTrustLevel::Community);
        assert!(!plugin.enabled);
        assert!(plugin
            .manifest_errors
            .iter()
            .any(|error| error.contains("is unsigned")));
    }

    #[test]
    fn release_packaging_rejects_unsigned_builtin_plugins_even_in_debug_builds() {
        let root = tempfile::tempdir().expect("plugin packages");
        for name in [
            "auto-unsubscribe",
            "calendar-sync",
            "github-integration",
            "pgp",
        ] {
            let folder = root.path().join(name);
            fs::create_dir_all(folder.join("dist")).expect("plugin folder");
            fs::write(folder.join("dist/component.wasm"), b"component").expect("component");
            fs::write(
                folder.join("Cargo.toml"),
                format!(
                    "[package]\nname = \"satchel-plugin-{name}\"\nversion = \"0.1.0\"\n\n\
                     [package.metadata.satchel]\ntrust = \"local_unsigned\"\n"
                ),
            )
            .expect("manifest");
        }
        let error =
            PluginRegistry::verify_release_packages(root.path()).expect_err("unsigned release");
        assert!(error.contains("is unsigned"), "{error}");
    }

    #[test]
    fn release_packaging_rejects_missing_plugins() {
        let root = tempfile::tempdir().expect("plugin packages");
        assert!(PluginRegistry::verify_release_packages(root.path())
            .expect_err("incomplete release")
            .contains("Missing bundled plugin"));
    }

    #[test]
    fn discovery_reads_only_the_directories_it_is_given() {
        let empty = tempfile::tempdir().expect("temp dir");
        let registry =
            PluginRegistry::discover_from_dirs(&[empty.path().to_path_buf()]).expect("discovery");

        assert!(registry.list().is_empty());
    }

    #[test]
    fn the_first_configured_directory_owns_an_id() {
        let bundled = tempfile::tempdir().expect("temp dir");
        let writable = tempfile::tempdir().expect("temp dir");
        for (root, version) in [(bundled.path(), "1.0.0"), (writable.path(), "9.9.9")] {
            let plugin_dir = root.join("pgp");
            std::fs::create_dir_all(&plugin_dir).expect("plugin dir");
            std::fs::write(
                plugin_dir.join("Cargo.toml"),
                format!("[package]\nname = \"satchel-plugin-pgp\"\nversion = \"{version}\"\n"),
            )
            .expect("write manifest");
        }

        let registry = PluginRegistry::discover_from_dirs(&[
            bundled.path().to_path_buf(),
            writable.path().to_path_buf(),
        ])
        .expect("discovery");

        assert_eq!(
            registry.get("satchel-plugin-pgp").map(|p| p.version),
            Some("1.0.0".to_string())
        );
    }

    #[test]
    fn unsigned_plugin_overrides_are_debug_only() {
        for denied in ["0", "false", "no", "off"] {
            std::env::set_var("SATCHEL_ALLOW_UNSIGNED_PLUGINS", denied);
            assert!(
                !allow_unsigned_plugins(),
                "{denied} must deny unsigned plugins"
            );
        }
        for allowed in ["1", "true", "yes", "on"] {
            std::env::set_var("SATCHEL_ALLOW_UNSIGNED_PLUGINS", allowed);
            assert_eq!(
                allow_unsigned_plugins(),
                cfg!(debug_assertions),
                "{allowed} may allow unsigned plugins only in debug builds"
            );
        }
        std::env::remove_var("SATCHEL_ALLOW_UNSIGNED_PLUGINS");
    }

    fn hex_encode(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}
