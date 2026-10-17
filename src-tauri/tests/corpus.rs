//! Regression harness over a corpus of `.eml` files.
//!
//! Every message goes through `open_message_file`, which shares
//! `parse_raw_rfc822` / `parse_body_part` / `decode_bytes_with_charset` with
//! the IMAP ingest path — that is where the charset and MIME bugs live. The
//! difference is only that this door needs no pool and no account.
//!
//! Two sources: `tests/corpus` is checked in and anonymised, `.private/corpus`
//! at the repo root is local-only and skipped when absent. See the README next
//! to the fixtures.

use std::fs;
use std::path::{Path, PathBuf};

use satchel_lib::commands::error::CommandResult;
use satchel_lib::commands::message_file::{open_message_file, MessageFilePreview};
use satchel_lib::utils::mime::decode_bytes_with_declared_charset;
use serde::Deserialize;

const REPLACEMENT: char = '\u{FFFD}';

#[derive(Debug, Default, Deserialize)]
struct Expectations {
    subject: Option<String>,
    #[serde(default)]
    body_contains: Vec<String>,
    /// Strings the extracted text must *not* contain. A MIME delimiter or a
    /// part header showing up here means the splitter cut in the wrong place.
    #[serde(default)]
    body_excludes: Vec<String>,
    /// Every attachment the message has, in order. Absent means unchecked.
    attachments: Option<Vec<ExpectedAttachment>>,
    charset: Option<String>,
    #[serde(default)]
    allow_replacement: bool,
}

#[derive(Debug, Deserialize)]
struct ExpectedAttachment {
    filename: String,
    /// Decoded byte count. Zero here is what a rejected transfer encoding
    /// leaves behind, so it is the number worth pinning.
    size: i64,
}

struct Case {
    path: PathBuf,
    label: String,
    raw: Vec<u8>,
    expect: Expectations,
}

fn collect_eml(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_eml(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "eml") {
            out.push(path);
        }
    }
}

fn corpus_roots() -> Vec<PathBuf> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let checked_in = manifest.join("tests/corpus");
    // `src-tauri/..` is the repo root; the private corpus is gitignored and
    // usually absent, which is not a failure.
    let private = manifest
        .parent()
        .map(|root| root.join(".private/corpus"))
        .unwrap_or_else(|| checked_in.clone());
    vec![checked_in, private]
}

fn load_cases() -> Vec<Case> {
    let mut paths = Vec::new();
    for root in corpus_roots() {
        collect_eml(&root, &mut paths);
    }
    paths.sort();

    assert!(
        !paths.is_empty(),
        "no .eml files found under src-tauri/tests/corpus — the checked-in fixtures are missing"
    );

    paths
        .into_iter()
        .map(|path| {
            let label = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("<unnamed>")
                .to_string();
            let raw = fs::read(&path).unwrap_or_else(|err| panic!("read {label}: {err}"));
            let sidecar = path.with_extension("expect.json");
            let expect = if sidecar.exists() {
                let text = fs::read_to_string(&sidecar)
                    .unwrap_or_else(|err| panic!("read sidecar for {label}: {err}"));
                serde_json::from_str(&text)
                    .unwrap_or_else(|err| panic!("parse sidecar for {label}: {err}"))
            } else {
                Expectations::default()
            };
            Case {
                path,
                label,
                raw,
                expect,
            }
        })
        .collect()
}

async fn parse(case: &Case) -> CommandResult<MessageFilePreview> {
    open_message_file(case.path.to_string_lossy().into_owned()).await
}

/// Case-insensitive search for a header line, used only to confirm a fixture
/// declares what its sidecar claims.
fn declares_header_containing(raw: &[u8], needle: &str) -> bool {
    let head_len = raw.len().min(8192);
    String::from_utf8_lossy(&raw[..head_len])
        .to_ascii_lowercase()
        .contains(&needle.to_ascii_lowercase())
}

fn has_header(raw: &[u8], name: &str) -> bool {
    let head_len = raw.len().min(8192);
    let head = String::from_utf8_lossy(&raw[..head_len]).to_ascii_lowercase();
    let prefix = format!("{}:", name.to_ascii_lowercase());
    head.starts_with(&prefix) || head.contains(&format!("\n{prefix}"))
}

fn body_text_of(preview: &MessageFilePreview) -> String {
    preview.body_text.clone().unwrap_or_default()
}

fn report(failures: Vec<String>) {
    if failures.is_empty() {
        return;
    }
    panic!(
        "{} corpus expectation(s) failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[tokio::test]
async fn every_corpus_message_parses() {
    let mut failures = Vec::new();
    for case in load_cases() {
        match parse(&case).await {
            Ok(_) => {}
            Err(err) => failures.push(format!("  {}: parse failed: {err}", case.label)),
        }
    }
    report(failures);
}

#[tokio::test]
async fn corpus_headers_survive_parsing() {
    let mut failures = Vec::new();
    for case in load_cases() {
        let Ok(preview) = parse(&case).await else {
            continue; // reported by `every_corpus_message_parses`
        };

        if has_header(&case.raw, "subject") && preview.subject.trim().is_empty() {
            failures.push(format!(
                "  {}: Subject header present but subject is empty",
                case.label
            ));
        }
        if has_header(&case.raw, "from") && preview.from_address.trim().is_empty() {
            failures.push(format!(
                "  {}: From header present but from_address is empty",
                case.label
            ));
        }
        if has_header(&case.raw, "date") && preview.date_rfc3339.trim().is_empty() {
            failures.push(format!(
                "  {}: Date header present but date_rfc3339 is empty",
                case.label
            ));
        }

        if let Some(expected) = case.expect.subject.as_ref() {
            if &preview.subject != expected {
                failures.push(format!(
                    "  {}: subject mismatch\n      expected: {expected:?}\n      actual:   {:?}",
                    case.label, preview.subject
                ));
            }
        }
    }
    report(failures);
}

/// Attachments and the parts around them, as the sidecar names them.
///
/// Deliberately not `#[ignore]`d: a rejected transfer encoding and a
/// misplaced boundary both show up here as a size of zero or a missing part,
/// and both are silent everywhere else.
#[tokio::test]
async fn corpus_attachments_and_part_boundaries_hold() {
    let mut failures = Vec::new();
    for case in load_cases() {
        let Ok(preview) = parse(&case).await else {
            continue;
        };

        if let Some(expected) = case.expect.attachments.as_ref() {
            let actual = preview
                .attachments
                .iter()
                .map(|attachment| (attachment.filename.clone(), attachment.size))
                .collect::<Vec<_>>();
            let wanted = expected
                .iter()
                .map(|attachment| (attachment.filename.clone(), attachment.size))
                .collect::<Vec<_>>();
            if actual != wanted {
                failures.push(format!(
                    "  {}: attachment mismatch\n      expected: {wanted:?}\n      actual:   {actual:?}",
                    case.label
                ));
            }
        }

        let text = body_text_of(&preview);
        let html = preview.body_html.clone().unwrap_or_default();
        for needle in &case.expect.body_excludes {
            if text.contains(needle) || html.contains(needle) {
                failures.push(format!(
                    "  {}: body must not contain {needle:?}",
                    case.label
                ));
            }
        }
    }
    report(failures);
}

#[tokio::test]
async fn corpus_bodies_are_extracted() {
    let mut failures = Vec::new();
    for case in load_cases() {
        let Ok(preview) = parse(&case).await else {
            continue;
        };

        let has_part = declares_header_containing(&case.raw, "text/plain")
            || declares_header_containing(&case.raw, "text/html");
        let text = body_text_of(&preview);
        if has_part && text.trim().is_empty() && preview.body_html.is_none() {
            failures.push(format!(
                "  {}: message has a text or html part but neither body survived",
                case.label
            ));
        }
    }
    report(failures);
}

// BUG: 8-bit bodies in any non-UTF-8 charset are destroyed before the charset
// decoder ever sees them. `commands::message_file::parse_message_bytes` runs
// `String::from_utf8_lossy` over the whole message, and
// `providers::imap::parse_message_body` does the same to the IMAP body bytes,
// so every byte >0x7F becomes U+FFFD and `decode_bytes_with_charset` then
// decodes the replacement character's own UTF-8 bytes — "für" arrives as
// "fï¿½r". Quoted-printable and base64 bodies survive because they are ASCII on
// the wire; only `Content-Transfer-Encoding: 8bit` (and absent/binary) is hit.
// Affects fixtures 01 (ISO-8859-1) and 04 (windows-1252 html part).
#[tokio::test]
async fn corpus_bodies_match_expected_content() {
    let mut failures = Vec::new();
    for case in load_cases() {
        let Ok(preview) = parse(&case).await else {
            continue;
        };
        let text = body_text_of(&preview);

        for needle in &case.expect.body_contains {
            let in_text = text.contains(needle);
            let in_html = preview
                .body_html
                .as_deref()
                .is_some_and(|html| html.contains(needle));
            if !in_text && !in_html {
                failures.push(format!(
                    "  {}: body is missing {needle:?}\n      body_text: {:?}",
                    case.label,
                    text.chars().take(200).collect::<String>()
                ));
            }
        }
    }
    report(failures);
}

/// A U+FFFD in the extracted text means a decoder gave up on bytes it should
/// have understood. Only a fixture whose sidecar says `allow_replacement` is
/// permitted one, because only then is the source itself genuinely broken.
///
// BUG: same root cause as `corpus_bodies_match_expected_content` — the lossy
// UTF-8 conversion in `parse_message_bytes` / `parse_message_body`. Fixture 05
// (8-bit German text, no charset parameter) keeps the U+FFFD literally, because
// the missing charset defaults to UTF-8 and no second decode hides it.
#[tokio::test]
async fn corpus_bodies_have_no_replacement_characters() {
    let mut failures = Vec::new();
    for case in load_cases() {
        if case.expect.allow_replacement {
            continue;
        }
        let Ok(preview) = parse(&case).await else {
            continue;
        };

        let text = body_text_of(&preview);
        if let Some(offset) = text.find(REPLACEMENT) {
            let window: String = text[offset.saturating_sub(40)..].chars().take(80).collect();
            failures.push(format!(
                "  {}: body_text contains U+FFFD near {window:?}",
                case.label
            ));
        }
    }
    report(failures);
}

/// The `charset` sidecar key pins two things at once: the fixture really
/// declares that label, and the app's decoder resolves it. An unrecognised
/// label is not an error at runtime — it falls back to UTF-8 and mangles the
/// body silently — so the recognition has to be asserted here.
#[tokio::test]
async fn corpus_declared_charsets_are_recognised() {
    let mut failures = Vec::new();
    for case in load_cases() {
        let Some(charset) = case.expect.charset.as_ref() else {
            continue;
        };

        if !declares_header_containing(&case.raw, &format!("charset={charset}"))
            && !declares_header_containing(&case.raw, &format!("charset=\"{charset}\""))
        {
            failures.push(format!(
                "  {}: sidecar claims charset {charset:?} but no header declares it",
                case.label
            ));
        }

        if let Err(err) = decode_bytes_with_declared_charset(b"probe", charset) {
            failures.push(format!(
                "  {}: charset {charset:?} is not resolvable by the decoder: {err}",
                case.label
            ));
        }
    }
    report(failures);
}
