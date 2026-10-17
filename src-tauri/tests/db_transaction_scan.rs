//! Source scan: a transaction that writes has to start with `BEGIN IMMEDIATE`.
//!
//! In WAL mode SQLite refuses to upgrade a reader snapshot to a writer once
//! another connection committed in between, and it answers that with
//! `SQLITE_BUSY_SNAPSHOT` — for which it never calls the busy handler, so the
//! pool's `busy_timeout` does not cover it. Taking the write lock up front is
//! the only thing that does.

use std::path::{Path, PathBuf};

/// Statements that make a transaction a writer. `.execute(` catches the
/// query-builder calls whose SQL never appears as a literal.
const WRITE_MARKERS: &[&str] = &[
    ".execute(",
    "INSERT ",
    "UPDATE ",
    "DELETE ",
    "REPLACE INTO",
    "CREATE ",
    "DROP ",
];

fn scanned_roots() -> Vec<PathBuf> {
    let app = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace = app.parent().expect("src-tauri has a parent").to_path_buf();
    vec![
        app.join("src/sync"),
        workspace.join("crates/satchel-core/src/db"),
    ]
}

fn rust_files(root: &Path, out: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(root).unwrap_or_else(|err| panic!("read {root:?}: {err}"));
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// Lines covered by the transaction opened on `start`.
///
/// Every transaction in these trees ends with an explicit `commit`, so the
/// commit is the cheapest end marker there is — no brace counting, and
/// therefore nothing that multi-line SQL literals can throw off. A new `fn`
/// header bounds the search in case a transaction is dropped instead.
fn transaction_body<'a>(lines: &'a [&'a str], start: usize) -> &'a [&'a str] {
    for (offset, line) in lines.iter().enumerate().skip(start + 1) {
        if line.contains(".commit()") {
            return &lines[start..=offset];
        }
        if line.trim_start().starts_with("fn ")
            || line.trim_start().starts_with("async fn ")
            || line.trim_start().starts_with("pub fn ")
            || line.trim_start().starts_with("pub async fn ")
            || line.trim_start().starts_with("pub(crate) async fn ")
            || line.trim_start().starts_with("pub(crate) fn ")
        {
            return &lines[start..offset];
        }
    }
    &lines[start..]
}

#[test]
fn read_then_write_transactions_use_begin_immediate() {
    let mut files = Vec::new();
    for root in scanned_roots() {
        rust_files(&root, &mut files);
    }
    assert!(
        files.len() > 20,
        "transaction scan found only {} files — the roots moved",
        files.len()
    );

    let mut offenders = Vec::new();
    for file in &files {
        let source = std::fs::read_to_string(file).expect("read scanned file");
        if !source.contains(".begin()") {
            continue;
        }
        let lines: Vec<&str> = source.lines().collect();
        for (index, line) in lines.iter().enumerate() {
            if !line.contains(".begin()") {
                continue;
            }
            let body = transaction_body(&lines, index);
            if let Some(write) = body
                .iter()
                .find(|line| WRITE_MARKERS.iter().any(|marker| line.contains(marker)))
            {
                offenders.push(format!(
                    "{}:{} — writes at `{}`",
                    file.display(),
                    index + 1,
                    write.trim()
                ));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "these transactions read before they write and must use \
         `begin_with(\"BEGIN IMMEDIATE\")`:\n  {}",
        offenders.join("\n  ")
    );
}
