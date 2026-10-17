//! The app's own commands are behind an ACL, and the viewer window is not
//! allowed to reach all of them.
//!
//! Tauri checks the ACL for a plugin command, or for an app command when the
//! app defined its own manifest — nothing else (`tauri::webview`:
//! `if (plugin_command.is_some() || has_app_acl_manifest)`). Without that
//! manifest all 190-odd `#[tauri::command]` functions were callable from every
//! window, including the one that renders a stranger's `.eml`, while
//! `capabilities/message-viewer.json` described itself as narrower on purpose.
//!
//! Three ways this falls back silently, one test each: the manifest disappears
//! (a `build.rs` edit), the command list in `build.rs` drifts from
//! `generate_handler!` (a new command with no permission to grant), or the
//! viewer capability gets widened.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(relative: &str) -> String {
    let path = crate_root().join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("read {path:?}: {err}"))
}

/// Commands registered with `generate_handler!` in `src/lib.rs`.
fn registered_commands() -> BTreeSet<String> {
    let source = read("src/lib.rs");
    let start = source
        .find("generate_handler![")
        .expect("generate_handler! in lib.rs");
    let block = &source[start..];
    let end = block.find("])").expect("end of generate_handler!");
    block[..end]
        .lines()
        .filter_map(|line| line.trim().strip_prefix("commands::"))
        .filter_map(|entry| entry.trim_end_matches(',').rsplit("::").next())
        .map(str::to_string)
        .collect()
}

/// Commands declared in `build.rs`, which is what gets a permission.
fn declared_commands() -> BTreeSet<String> {
    let source = read("build.rs");
    let start = source
        .find("const COMMANDS: &[&str] = &[")
        .expect("COMMANDS in build.rs");
    let block = &source[start..];
    let end = block.find("];").expect("end of COMMANDS");
    block[..end]
        .lines()
        .filter_map(|line| line.trim().strip_prefix('"'))
        .filter_map(|line| line.split('"').next())
        .map(str::to_string)
        .collect()
}

fn capability_permissions(name: &str) -> BTreeSet<String> {
    let raw = read(&format!("capabilities/{name}.json"));
    let parsed: serde_json::Value = serde_json::from_str(&raw).expect("capability json");
    parsed["permissions"]
        .as_array()
        .expect("permissions array")
        .iter()
        .filter_map(|value| value.as_str())
        .map(str::to_string)
        .collect()
}

#[test]
fn the_app_declares_an_acl_manifest() {
    let path = crate_root().join("gen/schemas/acl-manifests.json");
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!("read {path:?}: {err} — run a build first, `build.rs` writes it")
    });
    let parsed: serde_json::Value = serde_json::from_str(&raw).expect("acl manifests");
    let app = parsed
        .get("__app-acl__")
        .expect("no app entry: without one Tauri checks no app command at all");

    let permissions = app["permissions"]
        .as_object()
        .expect("app permissions object");
    assert!(
        permissions.contains_key("allow-create-backup"),
        "the app manifest carries no per-command permissions"
    );
    assert!(
        app["permission_sets"]
            .as_object()
            .is_some_and(|sets| sets.contains_key("allow-app-commands")),
        "the set the full-client windows use is gone"
    );
}

/// A command registered but not declared has no permission and is therefore
/// callable from nowhere — which looks like a bug in the feature, not the ACL.
#[test]
fn every_registered_command_has_a_permission() {
    let registered = registered_commands();
    let declared = declared_commands();
    assert!(!registered.is_empty(), "read no commands out of lib.rs");

    let undeclared: Vec<_> = registered.difference(&declared).collect();
    let stale: Vec<_> = declared.difference(&registered).collect();
    assert!(
        undeclared.is_empty(),
        "registered but missing from COMMANDS in build.rs, so unreachable: {undeclared:?}"
    );
    assert!(
        stale.is_empty(),
        "declared in build.rs but no longer registered: {stale:?}"
    );
}

/// The window that renders a file from outside gets a list, not everything.
#[test]
fn the_viewer_window_cannot_reach_the_whole_command_surface() {
    let viewer = capability_permissions("message-viewer");

    assert!(
        !viewer.contains("allow-app-commands"),
        "the viewer took the full command set"
    );
    for forbidden in [
        "allow-create-backup",
        "allow-add-account",
        "allow-export-mailbox",
        "allow-delete-account",
        "allow-send-message",
    ] {
        assert!(
            !viewer.contains(forbidden),
            "{forbidden} is reachable from the .eml viewer window"
        );
    }
    // What it does need, so a well-meant cleanup cannot empty the list.
    for required in [
        "allow-open-message-file",
        "allow-read-message-file-attachment",
    ] {
        assert!(viewer.contains(required), "the viewer lost {required}");
    }
}

#[test]
fn the_tray_popover_stays_at_its_four_commands() {
    let tray = capability_permissions("tray-popover");
    let app_commands: BTreeSet<String> = tray
        .iter()
        .filter(|permission| !permission.contains(':'))
        .cloned()
        .collect();

    assert_eq!(
        app_commands,
        BTreeSet::from_iter(
            [
                "allow-act-on-tray-thread",
                "allow-open-tray-destination",
                "allow-open-tray-thread",
                "allow-tray-snapshot",
            ]
            .map(str::to_string)
        )
    );
}

/// The windows running the full client are the ones allowed to have it.
#[test]
fn the_full_command_set_goes_only_to_full_client_windows() {
    for capability in ["default", "compose", "mobile"] {
        assert!(
            capability_permissions(capability).contains("allow-app-commands"),
            "{capability} runs the whole client but lost the command set"
        );
    }
    for capability in ["message-viewer", "tray-popover"] {
        assert!(
            !capability_permissions(capability).contains("allow-app-commands"),
            "{capability} is meant to be narrow"
        );
    }
}

/// The generated set has to cover the declared commands, or a capability that
/// asks for "all" silently gets fewer.
#[test]
fn the_generated_set_covers_every_declared_command() {
    let raw = read("permissions/allow-app-commands.toml");
    let listed: BTreeSet<String> = raw
        .lines()
        .filter_map(|line| line.trim().trim_end_matches(',').strip_prefix("\"allow-"))
        .filter_map(|line| line.strip_suffix('"'))
        .map(str::to_string)
        .collect();
    let expected: BTreeSet<String> = declared_commands()
        .iter()
        .map(|command| command.replace('_', "-"))
        .collect();

    assert_eq!(
        listed, expected,
        "permissions/allow-app-commands.toml is stale — build.rs regenerates it"
    );
}

/// Permission files exist for the declared commands.
#[test]
fn each_declared_command_has_a_generated_permission_file() {
    let directory = crate_root().join("permissions/autogenerated");
    let present: BTreeSet<String> = std::fs::read_dir(&directory)
        .unwrap_or_else(|err| panic!("read {directory:?}: {err}"))
        .flatten()
        .filter_map(|entry| {
            Path::new(&entry.file_name())
                .file_stem()
                .map(|stem| stem.to_string_lossy().to_string())
        })
        .collect();

    let missing: Vec<_> = declared_commands().difference(&present).cloned().collect();
    assert!(
        missing.is_empty(),
        "no permission file for {missing:?} — run a build"
    );
}

#[test]
fn client_filesystem_scope_excludes_credentials_and_configuration() {
    for capability in ["default", "mobile", "compose"] {
        let parsed: serde_json::Value =
            serde_json::from_str(&read(&format!("capabilities/{capability}.json"))).unwrap();
        let permissions = parsed["permissions"].as_array().unwrap();
        assert!(!permissions.iter().any(|value| value == "fs:default"));
        let scope = permissions
            .iter()
            .find(|value| value["identifier"] == "fs:scope")
            .unwrap();
        let allow = scope["allow"].as_array().unwrap();
        assert_eq!(
            allow,
            &vec![
                serde_json::json!("$APPDATA/satchel/attachments/**"),
                serde_json::json!("$APPCACHE/satchel-transfer-*")
            ]
        );
        for denied in [
            "$APPDATA/satchel/.env",
            "$APPDATA/satchel/pgp-master-key-v1",
            "$APPDATA/satchel/satchel.db*",
            "$APPDATA/satchel/plugins/**",
            "$APPCACHE/satchel/wasmtime/**",
        ] {
            assert!(
                scope["deny"]
                    .as_array()
                    .unwrap()
                    .contains(&serde_json::json!(denied)),
                "{capability} permits {denied}"
            );
        }
    }
}

/// Resolve every capability together, so an overlapping grant cannot widen a narrow window.
#[test]
fn narrow_windows_have_only_the_resolved_commands_they_need() {
    use std::collections::BTreeMap;
    use tauri::utils::{
        acl::{capability::Capability, manifest::Manifest, resolved::Resolved},
        platform::Target,
    };
    let manifests: BTreeMap<String, Manifest> =
        serde_json::from_str(&read("gen/schemas/acl-manifests.json")).unwrap();
    let capabilities: BTreeMap<String, Capability> =
        std::fs::read_dir(crate_root().join("capabilities"))
            .unwrap()
            .map(|entry| {
                let path = entry.unwrap().path();
                let capability: Capability =
                    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
                (capability.identifier.clone(), capability)
            })
            .collect();
    let viewer = BTreeSet::from_iter(
        [
            "plugin:event|listen",
            "plugin:event|unlisten",
            "plugin:window|close",
            "plugin:window|set_focus",
            "plugin:dialog|save",
            "plugin:fs|write_file",
            "plugin:fs|open",
            "plugin:fs|write",
            "open_message_file",
            "read_message_file_attachment",
            "list_accounts",
            "import_mbox",
            "open_external_url",
            "record_frontend_diagnostic",
            "show_in_folder",
        ]
        .map(str::to_string),
    );
    let tray = BTreeSet::from_iter(
        [
            "plugin:event|listen",
            "plugin:event|unlisten",
            "plugin:window|hide",
            "plugin:window|set_focus",
            "tray_snapshot",
            "open_tray_thread",
            "act_on_tray_thread",
            "open_tray_destination",
        ]
        .map(str::to_string),
    );
    for target in [Target::Linux, Target::MacOS, Target::Windows] {
        let resolved = Resolved::resolve(&manifests, capabilities.clone(), target).unwrap();
        for (label, expected) in [("message-file-fixture", &viewer), ("tray-popover", &tray)] {
            let commands = resolved
                .allowed_commands
                .iter()
                .filter(|(_, grants)| {
                    grants.iter().any(|grant| {
                        grant.windows.iter().any(|pattern| pattern.matches(label))
                            || grant.webviews.iter().any(|pattern| pattern.matches(label))
                    })
                })
                .map(|(command, _)| command.clone())
                .collect::<BTreeSet<_>>();
            assert_eq!(&commands, expected, "resolved ACL for {label} on {target}");
            assert!(!commands.contains("plugin:event|emit"));
            assert!(!commands.contains("plugin:event|emit_to"));
        }
    }
}
