//! Keep development configuration out of shipped credential and trust paths.
use std::path::PathBuf;

fn read(relative: &str) -> String {
    std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative)).unwrap()
}

#[test]
fn dotenv_entry_points_are_debug_only() {
    let source = read("src/lib.rs");
    for guarded in [
        "#[cfg(debug_assertions)]\n    let _ = dotenvy::dotenv();",
        "#[cfg(debug_assertions)]\n            load_app_support_env(app);",
        "#[cfg(debug_assertions)]\nfn load_app_support_env",
    ] {
        assert!(
            source.contains(guarded),
            "unguarded dotenv entry point: {guarded}"
        );
    }
}

#[test]
fn sensitive_runtime_overrides_use_the_debug_policy() {
    for (file, keys) in [
        (
            "../crates/satchel-core/src/providers/gmail.rs",
            vec!["SATCHEL_GMAIL_API_BASE"],
        ),
        (
            "../crates/satchel-core/src/providers/outlook.rs",
            vec!["SATCHEL_GRAPH_API_BASE"],
        ),
        (
            "../crates/satchel-core/src/utils/pgp_engine.rs",
            vec!["SATCHEL_GPG_BIN", "SATCHEL_GPGCONF_BIN"],
        ),
        (
            "../crates/satchel-core/src/utils/smime_engine.rs",
            vec!["SATCHEL_OPENSSL_BIN"],
        ),
        (
            "src/plugins/registry.rs",
            vec![
                "SATCHEL_ALLOW_UNSIGNED_PLUGINS",
                "SATCHEL_PLUGIN_TRUSTED_KEYS",
            ],
        ),
    ] {
        let source = read(file);
        for key in keys {
            assert!(
                source.contains(&format!("environment::debug_env(\"{key}\")")),
                "{file} bypasses the debug policy for {key}"
            );
            assert!(
                !source.contains(&format!("std::env::var(\"{key}\")")),
                "{file} reads {key} in release"
            );
        }
    }
    assert!(
        read("../crates/satchel-core/src/utils/oauth.rs").contains("environment::debug_env(key)")
    );
}
