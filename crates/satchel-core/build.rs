use std::path::PathBuf;

/// Keys carried from the build environment into the binary.
///
/// Redirect URIs are included as well: mobile has no process environment, and
/// an explicitly registered loopback URI must keep its configured port instead
/// of being replaced with an unregistered ephemeral one.
/// Google's desktop client credential is public installed-app metadata, despite
/// its `client_secret` name. Confidential web-client secrets remain excluded.
/// https://developers.google.com/identity/protocols/oauth2/#installed
const BAKED_KEYS: &[&str] = &[
    "SATCHEL_GMAIL_CLIENT_ID",
    "SATCHEL_GMAIL_DESKTOP_CLIENT_SECRET",
    "SATCHEL_GMAIL_REDIRECT_URI",
    "SATCHEL_OUTLOOK_CLIENT_ID",
    "SATCHEL_OUTLOOK_REDIRECT_URI",
];

fn main() {
    // `cargo:rustc-env` only reaches the crate whose build script emitted it,
    // and `compiled_env` in `utils::oauth` is what reads these back — so the
    // baking has to happen here, not in the app's build script.
    bake_oauth_client_identity();
    declare_platform_cfgs();
}

/// `desktop` and `mobile` come from `tauri-build` in the app crate. Core has no
/// Tauri dependency, so it derives the same two aliases itself — the modules
/// that moved here (TLS roots, keyring, master key fallback) branch on them and
/// would silently take the wrong arm if the cfg were simply absent.
fn declare_platform_cfgs() {
    println!("cargo::rustc-check-cfg=cfg(desktop)");
    println!("cargo::rustc-check-cfg=cfg(mobile)");
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if matches!(target_os.as_str(), "android" | "ios") {
        println!("cargo::rustc-cfg=mobile");
    } else {
        println!("cargo::rustc-cfg=desktop");
    }
}

/// Compile the OAuth client identity in, because a phone has no environment.
///
/// On the desktop these come from the shell or from a `.env` the app resolves at
/// startup. Neither exists on Android: there is no shell to export a variable
/// in, and `load_app_support_env` points at a directory nothing ever writes a
/// `.env` to. The result was `missing SATCHEL_GMAIL_CLIENT_ID` the moment
/// anyone tried to add a Google account on a phone — not a misconfiguration,
/// but a path that could not be configured at all.
///
/// Reading the file at build time is how an installed app is meant to carry its
/// client identity. Only Google's public desktop credential is included; generic
/// Gmail and Outlook client secrets remain development-only runtime inputs.
fn bake_oauth_client_identity() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    let repo_root = manifest
        .parent()
        .and_then(|path| path.parent())
        .unwrap_or(&manifest);
    let mut env_file = repo_root.join(".env");
    // Track the local file even when absent: creating it must invalidate a
    // build that previously used the main checkout's configuration.
    println!("cargo:rerun-if-changed={}", env_file.display());
    println!(
        "cargo:rerun-if-changed={}",
        repo_root.join(".git").display()
    );
    // Linked worktrees do not inherit the main checkout's untracked .env.
    if !env_file.exists() {
        if let Some(main) = main_checkout_of_worktree(repo_root) {
            env_file = main.join(".env");
            println!("cargo:rerun-if-changed={}", env_file.display());
        }
    }

    let from_file = std::fs::read_to_string(&env_file).unwrap_or_default();

    for key in BAKED_KEYS {
        println!("cargo:rerun-if-env-changed={key}");
        // The ambient environment wins, so CI can pass configuration without a file.
        let value = std::env::var(key).ok().or_else(|| lookup(&from_file, key));
        let Some(value) = value else { continue };
        let value = value.trim();
        if value.is_empty() || value.contains('\n') {
            continue;
        }
        println!("cargo:rustc-env={key}={value}");
    }
}

/// Resolve a standard linked worktree's `<main>/.git/worktrees/<name>` pointer.
fn main_checkout_of_worktree(repo_root: &std::path::Path) -> Option<PathBuf> {
    let pointer = std::fs::read_to_string(repo_root.join(".git")).ok()?;
    let gitdir = PathBuf::from(pointer.strip_prefix("gitdir:")?.trim());
    // Git can write a path relative to the directory containing the .git file.
    let gitdir = repo_root.join(gitdir).canonicalize().ok()?;
    let worktrees = gitdir.parent()?;
    if worktrees.file_name()? != "worktrees" {
        return None;
    }
    Some(worktrees.parent()?.parent()?.to_path_buf())
}

/// Pull one `KEY=value` out of a dotenv file.
///
/// Deliberately not a dotenv parser: a build script would have to take the
/// dependency for a file format we write ourselves and read four keys from.
fn lookup(contents: &str, key: &str) -> Option<String> {
    contents.lines().find_map(|line| {
        let line = line.trim();
        if line.starts_with('#') {
            return None;
        }
        let (name, value) = line.split_once('=')?;
        if name.trim() != key {
            return None;
        }
        Some(
            value
                .trim()
                .trim_matches('"')
                .trim_matches('\'')
                .to_string(),
        )
    })
}
