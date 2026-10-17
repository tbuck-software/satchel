//! Source scan: CPU-bound crypto must not run on an async worker.
//!
//! `list_message_bodies` already hands WASM plugin execution to
//! `spawn_blocking`, with a comment saying why — a batch of PGP messages must
//! not stall unrelated async work. The crypto next to it did not get the same
//! treatment: decryption, key generation and passphrase verification all ran
//! in-process on the Tokio worker serving the command. On a four-core device
//! that is a quarter of the runtime held for the duration; on mobile, more.
//!
//! The scan is deliberately narrow. Only calls that are slow *by design* are
//! listed: a KDF, a keypair, a CMS operation. Reading key metadata or asking
//! which engine is active is neither, and pretending otherwise would make this
//! a list nobody maintains.

use std::path::{Path, PathBuf};

/// Engine calls that must never be reached from an `async fn`.
const BLOCKING_CALLS: &[&str] = &[
    "pgp_engine::decrypt_with_private_key",
    "pgp_engine::generate_keypair",
    "pgp_engine::verify_private_key_passphrase",
    "smime_engine::decrypt",
    "smime_engine::encrypt",
    "smime_engine::extract_pkcs12",
    "smime_engine::generate_keypair",
    "smime_engine::sign",
    "smime_engine::verify",
    "smime_engine::verify_private_key_passphrase",
];

fn command_sources() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&root)
        .unwrap_or_else(|err| panic!("read {root:?}: {err}"))
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
        .collect();
    files.sort();
    files
}

/// The function a line belongs to, and whether it is `async`.
///
/// Nearest preceding `fn` header wins. A closure handed to `spawn_blocking` is
/// not a `fn` item, so lines inside one would otherwise be attributed to the
/// enclosing `async fn` — the very construct the rule asks for. Those lines are
/// marked not-async instead, by following the call's parentheses to their
/// close.
fn enclosing_functions(source: &str) -> Vec<(usize, String, bool)> {
    let mut out = Vec::new();
    let mut current: Option<(String, bool)> = None;
    let mut blocking_depth: Option<i32> = None;
    for (index, line) in source.lines().enumerate() {
        let trimmed = line.trim_start();
        // Inside `spawn_blocking(...)` the work is already off the async
        // worker, which is the whole point of the rule.
        if blocking_depth.is_none() && line.contains("spawn_blocking(") {
            blocking_depth = Some(0);
        }
        if let Some(depth) = blocking_depth.as_mut() {
            let start = if line.contains("spawn_blocking(") {
                line.find("spawn_blocking(").unwrap_or(0)
            } else {
                0
            };
            for ch in line[start..].chars() {
                match ch {
                    '(' => *depth += 1,
                    ')' => *depth -= 1,
                    _ => {}
                }
            }
            let inside = *depth > 0;
            out.push((
                index + 1,
                current
                    .clone()
                    .map(|(name, _)| name)
                    .unwrap_or_else(|| "<file>".to_string()),
                false,
            ));
            if !inside {
                blocking_depth = None;
            }
            continue;
        }
        if let Some(rest) = trimmed
            .strip_prefix("pub(crate) ")
            .or_else(|| trimmed.strip_prefix("pub "))
            .or(Some(trimmed))
        {
            let is_async = rest.starts_with("async fn ");
            if is_async || rest.starts_with("fn ") {
                let name = rest
                    .trim_start_matches("async ")
                    .trim_start_matches("fn ")
                    .split(['(', '<', ' '])
                    .next()
                    .unwrap_or("")
                    .to_string();
                current = Some((name, is_async));
            }
        }
        if let Some((name, is_async)) = current.clone() {
            out.push((index + 1, name, is_async));
        }
    }
    out
}

#[test]
fn no_blocking_crypto_call_sits_in_an_async_fn() {
    let mut offenders = Vec::new();

    for file in command_sources() {
        let source = std::fs::read_to_string(&file).expect("read source");
        let attribution = enclosing_functions(&source);
        for (line_number, line) in source.lines().enumerate() {
            // Comments describe the rule; they do not break it.
            if line.trim_start().starts_with("//") {
                continue;
            }
            let Some(call) = BLOCKING_CALLS.iter().find(|call| line.contains(**call)) else {
                continue;
            };
            let Some((_, function, is_async)) = attribution
                .iter()
                .find(|(number, _, _)| *number == line_number + 1)
            else {
                continue;
            };
            if *is_async {
                let name = file.file_name().unwrap_or_default().to_string_lossy();
                offenders.push(format!(
                    "{name}:{} — {call} inside `async fn {function}`",
                    line_number + 1
                ));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "CPU-bound crypto on an async worker. Move the call into a sync helper \
         and hand that to `tokio::task::spawn_blocking`:\n  {}",
        offenders.join("\n  ")
    );
}

/// The scan has to be able to see an offender, or it says nothing.
#[test]
fn the_scan_reports_an_async_engine_call() {
    let source = "
pub async fn unlock(key: &str) -> Result<bool, String> {
    smime_engine::verify_private_key_passphrase(key, None)
}

fn check(key: &str) -> Result<bool, String> {
    smime_engine::verify_private_key_passphrase(key, None)
}
";
    let attribution = enclosing_functions(source);
    let flagged: Vec<_> = source
        .lines()
        .enumerate()
        .filter(|(_, line)| {
            BLOCKING_CALLS.iter().any(|call| line.contains(call))
                && !line.trim_start().starts_with("//")
        })
        .filter_map(|(index, _)| {
            attribution
                .iter()
                .find(|(number, _, _)| *number == index + 1)
                .filter(|(_, _, is_async)| *is_async)
                .map(|(_, name, _)| name.clone())
        })
        .collect();

    assert_eq!(flagged, vec!["unlock".to_string()]);
}
