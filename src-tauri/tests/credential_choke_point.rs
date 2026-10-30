//! Enforces that account credentials are never read or written raw.
//!
//! `accounts.config` holds IMAP/SMTP passwords and OAuth refresh tokens and is
//! encrypted at rest. There is no second path: every read must go through
//! `decrypt_account_config` and every write through `encrypt_account_config`.
//!
//! Missing one site does not corrupt data — the value simply arrives as
//! `enc:v1:...` and fails to parse — but it breaks that feature, possibly only
//! hours later when a token refresh runs.
//!
//! The first version of this test matched the literal string
//! `config FROM accounts` and therefore missed every query that aliases or
//! joins (`acc.config`, `a.config AS config`, a multi-line SELECT list). Ten
//! sites slipped through and broke mail sync on a real mailbox. It now matches
//! any query that mentions both the table and the column.

use std::path::{Path, PathBuf};

fn source_files() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                out.push(path);
            }
        }
    }

    // Both crates: the accessor and its callers were split across
    // `satchel-core` (sync, providers, OAuth) and this crate (commands).
    let mut files = Vec::new();
    walk(Path::new("src"), &mut files);
    walk(Path::new("../crates/satchel-core/src"), &mut files);
    files
}

/// True when the source contains a query that reads accounts.config in any
/// shape: `config FROM accounts`, `acc.config`, `a.config AS config`, or a
/// SELECT list spread over several lines.
fn reads_account_config(source: &str) -> bool {
    // Collapse whitespace so a multi-line SELECT reads as one string.
    let flat = source.split_whitespace().collect::<Vec<_>>().join(" ");
    for (index, _) in flat.match_indices("accounts") {
        // Look back over the SELECT list preceding this FROM/JOIN.
        let mut start = index.saturating_sub(600);
        // 600 bytes back can land inside a multi-byte character such as '—'.
        while !flat.is_char_boundary(start) {
            start -= 1;
        }
        let window = &flat[start..index];
        if !window.contains("SELECT") {
            continue;
        }
        let after_select = window
            .rfind("SELECT")
            .map(|i| &window[i..])
            .unwrap_or(window);
        if after_select.contains("config") {
            return true;
        }
    }
    false
}

/// True when the source writes accounts.config, however the statement is
/// formatted (`UPDATE accounts SET config`, or `UPDATE accounts` and `SET
/// config` on separate lines).
fn writes_account_config(source: &str) -> bool {
    let flat = source.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.contains("INSERT INTO accounts") && flat.contains("config") {
        return true;
    }
    flat.match_indices("UPDATE accounts").any(|(index, _)| {
        let mut end = (index + 400).min(flat.len());
        while !flat.is_char_boundary(end) {
            end += 1;
        }
        flat[index..end].contains("SET config") || flat[index..end].contains("config = ?")
    })
}

/// Drop `#[cfg(test)]` blocks, which are conventionally at the end of a file.
/// Test fixtures seed throwaway databases with plaintext rows on purpose.
fn without_test_modules(source: &str) -> String {
    match source.find("#[cfg(test)]") {
        Some(index) => source[..index].to_string(),
        None => source.to_string(),
    }
}

/// Files allowed to touch the column directly.
fn is_exempt(path: &Path) -> bool {
    let name = path.to_string_lossy().replace('\\', "/");
    // The accessor itself, plus test fixtures that seed throwaway databases.
    // The accessor itself, the schema bootstrap that seeds demo data, and
    // test-only modules (gated by `#[cfg(test)] mod` in their parent).
    name.ends_with("satchel-core/src/accounts.rs")
        || name.ends_with("commands/accounts.rs")
        || name.ends_with("db/mod.rs")
        || name.ends_with("_tests.rs")
}

#[test]
fn every_config_read_goes_through_the_accessor() {
    let mut offenders = Vec::new();

    for path in source_files() {
        if is_exempt(&path) {
            continue;
        }
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        let source = without_test_modules(&raw);
        if !reads_account_config(&source) {
            continue;
        }
        // Reading is fine as long as the file also routes through the accessor,
        // or hands the value to a consumer that decrypts internally.
        let handled = source.contains("decrypt_account_config")
            || source.contains("provider_for_account")
            || source.contains("access_token_from_account_config")
            || source.contains("refresh_account_config");
        if !handled {
            offenders.push(path.to_string_lossy().to_string());
        }
    }

    assert!(
        offenders.is_empty(),
        "these files read accounts.config without decrypting it:\n  {}\n\n\
         Wrap the value in satchel_core::accounts::decrypt_account_config.",
        offenders.join("\n  ")
    );
}

#[test]
fn every_config_write_goes_through_the_accessor() {
    let mut offenders = Vec::new();

    for path in source_files() {
        if is_exempt(&path) {
            continue;
        }
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        let source = without_test_modules(&raw);
        let writes = writes_account_config(&source);
        if writes && !source.contains("encrypt_account_config") {
            offenders.push(path.to_string_lossy().to_string());
        }
    }

    assert!(
        offenders.is_empty(),
        "these files write accounts.config without encrypting it:\n  {}\n\n\
         Wrap the value in satchel_core::accounts::encrypt_account_config.",
        offenders.join("\n  ")
    );
}

#[test]
fn the_exemption_list_stays_small() {
    // Every exemption is a place where credentials could leak back into
    // plaintext. Keep it deliberate.
    let exempt = source_files().iter().filter(|p| is_exempt(p)).count();
    // 8 = accounts.rs + db/mod.rs + six `_tests.rs` fixture modules
    // (blocked_senders, followups, search, smime, topics, …).
    assert!(
        exempt <= 8,
        "credential accessor exemption list has grown to {exempt} files"
    );
}
