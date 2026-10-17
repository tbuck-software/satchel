//! Reading `.eml` and `.msg` files that live outside the mailbox.
//!
//! Deliberately separate from [`crate::commands::backup::import_mbox`]: that
//! path writes into the database and needs an account, which is the wrong
//! trade for someone who double-clicked a file in Finder and wants to read it.
//! Nothing here touches the pool — the payload is parsed, sanitized and handed
//! to a viewer window that dies with the file.

use base64::Engine;
use serde::Serialize;
use tauri::AppHandle;
#[cfg(desktop)]
use tauri::Manager;

use crate::commands::error::CommandResult;
use crate::email::sanitizer::sanitize_email_html;
use crate::providers::imap::{parse_raw_rfc822, ParsedRawEmail};

/// Ceiling for a file we are willing to pull into memory whole. Both parsers
/// want the complete buffer, and a viewer window is not worth an OOM.
const MAX_MESSAGE_FILE_BYTES: u64 = 64 * 1024 * 1024;

/// Largest inline part embedded straight into the preview payload. Above this
/// the image is dropped rather than pushed through the IPC boundary as base64.
const MAX_INLINE_EMBED_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/message_file/")
)]
pub struct MessageFileAttachment {
    /// Position in the parsed part list — the handle for reading the bytes back.
    pub index: usize,
    pub filename: String,
    pub content_type: String,
    pub size: i64,
    pub content_id: Option<String>,
    pub is_inline: bool,
    /// Inline parts carry their bytes so `cid:` references resolve without a
    /// second round trip. Regular attachments stay out of the payload and are
    /// fetched on demand by [`read_message_file_attachment`].
    pub data_url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/message_file/")
)]
pub struct MessageFilePreview {
    pub path: String,
    pub subject: String,
    pub from_address: String,
    pub from_name: Option<String>,
    pub to_addresses: Vec<String>,
    pub cc_addresses: Vec<String>,
    pub date_rfc3339: String,
    pub body_html: Option<String>,
    pub body_text: Option<String>,
    pub attachments: Vec<MessageFileAttachment>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/message_file/")
)]
pub struct MessageFileAttachmentData {
    pub filename: String,
    pub mime_type: String,
    pub data_base64: String,
}

/// True for paths this app claims in `fileAssociations`.
pub(crate) fn is_message_file(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("eml") || value.eq_ignore_ascii_case("msg"))
}

async fn read_message_file(path: &str) -> Result<Vec<u8>, String> {
    let meta = tokio::fs::metadata(path)
        .await
        .map_err(|err| format!("cannot read {path}: {err}"))?;
    if meta.len() > MAX_MESSAGE_FILE_BYTES {
        return Err(format!(
            "file is too large ({} MB); limit is {} MB",
            meta.len() / (1024 * 1024),
            MAX_MESSAGE_FILE_BYTES / (1024 * 1024)
        ));
    }
    tokio::fs::read(path)
        .await
        .map_err(|err| format!("cannot read {path}: {err}"))
}

fn parse_message_bytes(path: &str, bytes: &[u8]) -> Result<ParsedRawEmail, String> {
    if crate::commands::backup::is_msg_path(path) {
        return crate::commands::backup::parse_msg_bytes(bytes);
    }
    // Bytes all the way in: each MIME part declares its own charset, and a
    // lossy UTF-8 pass here would destroy every 8-bit body before the parser
    // ever reads that declaration.
    parse_raw_rfc822(bytes)
}

/// Message key for a path that is not a message file.
pub const NOT_A_MESSAGE_FILE: &str = "backend.errors.notAMessageFile";

/// Parse a message file for display. Never writes to the database.
#[tauri::command]
pub async fn open_message_file(path: String) -> CommandResult<MessageFilePreview> {
    // `is_message_file` existed but only the OS-open path used it. Without it
    // here, an IPC-capable caller could hand over `~/.ssh/id_ed25519`:
    // `parse_raw_rfc822` is lenient about non-MIME input and returns the whole
    // file as `body_text`, which — unlike `body_html` — is not sanitized.
    if !is_message_file(std::path::Path::new(&path)) {
        return Err(crate::commands::error::CommandError::new(
            NOT_A_MESSAGE_FILE,
        ));
    }
    let bytes = read_message_file(&path).await?;
    tokio::task::spawn_blocking(move || {
        let parsed = parse_message_bytes(&path, &bytes)?;
        Ok(build_preview(path, parsed))
    })
    .await
    .map_err(|error| crate::commands::error::CommandError::unexpected(error.to_string()))?
}

/// Shape a parsed message into the viewer payload.
///
/// Split out from the command so the sanitizing step is reachable from a test.
/// `emailCsp.ts` carries the same scar: a security layer that only exists
/// inside an untested code path is one refactor away from being gone.
fn build_preview(path: String, parsed: ParsedRawEmail) -> MessageFilePreview {
    let attachments = parsed
        .attachments
        .iter()
        .enumerate()
        .map(|(index, attachment)| {
            let content_type =
                crate::email::mime_type::normalize_mime_type(&attachment.content_type);
            let embed = attachment.is_inline
                && crate::email::mime_type::is_image_mime_type(&content_type)
                && attachment.bytes.len() <= MAX_INLINE_EMBED_BYTES;
            MessageFileAttachment {
                index,
                filename: attachment
                    .filename
                    .as_deref()
                    .map(satchel_core::utils::filename::strip_filename_controls)
                    .unwrap_or_else(|| "Anhang".to_string()),
                content_type: content_type.clone(),
                size: i64::try_from(attachment.bytes.len()).unwrap_or(i64::MAX),
                content_id: attachment.content_id.clone(),
                is_inline: attachment.is_inline,
                data_url: embed.then(|| {
                    format!(
                        "data:{};base64,{}",
                        content_type,
                        base64::engine::general_purpose::STANDARD.encode(&attachment.bytes)
                    )
                }),
            }
        })
        .collect::<Vec<_>>();

    MessageFilePreview {
        path,
        subject: parsed.subject,
        from_address: parsed.from_address,
        from_name: parsed.from_name,
        to_addresses: parsed.to_addresses,
        cc_addresses: parsed.cc_addresses,
        date_rfc3339: parsed.date_rfc3339,
        // The renderer's CSP is the load-bearing layer, but stored mail is
        // sanitized before it ever reaches a webview and a file dropped in from
        // outside deserves the same treatment.
        body_html: parsed
            .body_html
            .as_deref()
            .map(sanitize_email_html)
            .filter(|value| !value.trim().is_empty()),
        body_text: parsed.body_text.filter(|value| !value.trim().is_empty()),
        attachments,
    }
}

/// Read one part out of a message file. Re-parses rather than holding the file
/// open: these are local reads, and the alternative is per-window state that
/// has to be reaped when the viewer closes.
#[tauri::command]
pub async fn read_message_file_attachment(
    path: String,
    index: usize,
) -> CommandResult<MessageFileAttachmentData> {
    if !is_message_file(std::path::Path::new(&path)) {
        return Err(crate::commands::error::CommandError::new(
            NOT_A_MESSAGE_FILE,
        ));
    }
    let bytes = read_message_file(&path).await?;
    let parsed = parse_message_bytes(&path, &bytes)?;
    let attachment = parsed
        .attachments
        .get(index)
        .ok_or_else(|| format!("attachment {index} does not exist in this file"))?;
    Ok(MessageFileAttachmentData {
        filename: attachment
            .filename
            .as_deref()
            .map(satchel_core::utils::filename::strip_filename_controls)
            .unwrap_or_else(|| "Anhang".to_string()),
        mime_type: attachment.content_type.clone(),
        data_base64: base64::engine::general_purpose::STANDARD.encode(&attachment.bytes),
    })
}

/// Window label for a path. Stable, so opening the same file twice focuses the
/// existing window instead of stacking duplicates.
#[cfg(desktop)]
fn viewer_window_label(path: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(path.as_bytes());
    let hex = digest
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("message-file-{hex}")
}

/// Open the viewer for `path`.
///
/// The path travels base64url-encoded in the query string. Percent-encoding
/// would have to survive a Rust encoder, a URL parse and `decodeURIComponent`
/// intact, and mail files arrive with spaces, umlauts and `#` in their names
/// often enough that the round trip is not worth betting on.
/// Desktop only: the viewer is a second window, which a phone does not have.
/// Mobile reaches the same parser through `open_message_file` and renders it
/// inside the single window it owns.
#[cfg(desktop)]
pub(crate) fn open_message_file_window(app: &AppHandle, path: &str) -> Result<(), String> {
    let label = viewer_window_label(path);
    if let Some(existing) = app.get_webview_window(&label) {
        let _ = existing.unminimize();
        let _ = existing.show();
        let _ = existing.set_focus();
        return Ok(());
    }

    let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(path.as_bytes());
    let title = std::path::Path::new(path)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("Nachricht");

    crate::utils::navigation::guard(
        tauri::WebviewWindowBuilder::new(
            app,
            &label,
            tauri::WebviewUrl::App(format!("viewer.html?p={encoded}").into()),
        ),
        app,
    )
    .title(title)
    .inner_size(880.0, 760.0)
    .resizable(true)
    .build()
    .map_err(|err| format!("could not open window: {err}"))?;
    Ok(())
}

/// Open a viewer window from the frontend — used for `.eml`/`.msg` parts that
/// arrived as attachments and are already sitting in the attachment cache.
#[tauri::command]
pub async fn open_message_file_in_window(app: AppHandle, path: String) -> CommandResult<()> {
    if !tokio::fs::try_exists(&path).await.unwrap_or(false) {
        return Err(format!("{path} does not exist").into());
    }
    #[cfg(desktop)]
    return open_message_file_window(&app, &path).map_err(Into::into);
    #[cfg(mobile)]
    {
        let _ = app;
        Err("a separate viewer window is only available on desktop".into())
    }
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn attachment_reads_reject_non_message_paths_before_reading() {
        let error = super::read_message_file_attachment("/missing/secret.txt".into(), 0)
            .await
            .unwrap_err();
        assert_eq!(error.key, super::NOT_A_MESSAGE_FILE);
    }

    /// The extension gate, which existed but was not on this door.
    #[tokio::test]
    async fn opening_a_non_message_file_is_refused_without_reading_it() {
        let directory = tempfile::tempdir().expect("temp dir");
        let key = directory.path().join("id_ed25519");
        std::fs::write(&key, b"-----BEGIN OPENSSH PRIVATE KEY-----\nsecret\n").expect("key file");

        let error = super::open_message_file(key.to_string_lossy().to_string())
            .await
            .expect_err("a private key is not a message");
        assert_eq!(error.key, super::NOT_A_MESSAGE_FILE);

        // And the ones that are allowed still get through the gate.
        for name in ["mail.eml", "mail.EML", "mail.msg"] {
            assert!(super::is_message_file(std::path::Path::new(name)), "{name}");
        }
        assert!(!super::is_message_file(std::path::Path::new(
            "mail.eml.exe"
        )));
    }
    #[cfg(desktop)]
    use super::viewer_window_label;
    use super::{build_preview, is_message_file, parse_message_bytes};
    use std::path::Path;

    /// A multipart message carrying a script in the HTML body, one inline image
    /// referenced by `cid:` and one real attachment.
    fn sample_eml() -> String {
        [
            "From: Jagdgenossenschaft <jagd@example.org>",
            "To: empfang@example.org",
            "Cc: revier@example.org",
            "Subject: Abschussstatistik",
            "Date: Tue, 29 Jul 2026 09:15:00 +0200",
            "Message-ID: <abschuss-2026@example.org>",
            "MIME-Version: 1.0",
            "Content-Type: multipart/mixed; boundary=\"grenze\"",
            "",
            "--grenze",
            "Content-Type: text/html; charset=utf-8",
            "",
            "<p>Hallo</p><script>fetch('https://tracker.example/x')</script>",
            "<img src=\"cid:logo\">",
            "",
            "--grenze",
            "Content-Type: image/png",
            "Content-Transfer-Encoding: base64",
            "Content-ID: <logo>",
            "Content-Disposition: inline; filename=\"logo.png\"",
            "",
            "iVBORw0KGgo=",
            "",
            "--grenze",
            "Content-Type: application/pdf",
            "Content-Transfer-Encoding: base64",
            "Content-Disposition: attachment; filename=\"statistik.pdf\"",
            "",
            "JVBERi0xLjQK",
            "",
            "--grenze--",
            "",
        ]
        .join("\r\n")
    }

    #[test]
    fn viewer_does_not_embed_the_issue_91_mime_payload() {
        let raw = b"Content-Type: multipart/related; boundary=x\r\n\r\n--x\r\nContent-Type: text/html\r\n\r\n<img src=\"cid:x\">\r\n--x\r\nContent-Type: image/png\"><link rel=preconnect href=https://u1.t.example>\r\nContent-ID: <x>\r\nContent-Disposition: inline; filename=logo.png\r\n\r\nAAAA\r\n--x--\r\n";
        let preview = super::build_preview(
            "test.eml".into(),
            crate::providers::imap::parse_raw_rfc822(raw).unwrap(),
        );
        assert_eq!(
            preview.attachments[0].content_type,
            "application/octet-stream"
        );
        assert!(preview.attachments[0].data_url.is_none());
        assert!(!preview.body_html.unwrap().contains("<link"));
    }

    #[test]
    fn reads_headers_and_splits_inline_parts_from_attachments() {
        let raw = sample_eml();
        let parsed = parse_message_bytes("/tmp/abschuss.eml", raw.as_bytes()).unwrap();
        let preview = build_preview("/tmp/abschuss.eml".to_string(), parsed);

        assert_eq!(preview.subject, "Abschussstatistik");
        assert_eq!(preview.from_address, "jagd@example.org");
        assert_eq!(
            preview.to_addresses,
            vec!["empfang@example.org".to_string()]
        );
        assert_eq!(preview.cc_addresses, vec!["revier@example.org".to_string()]);

        let inline = preview
            .attachments
            .iter()
            .find(|attachment| attachment.is_inline)
            .expect("inline image survived parsing");
        // The renderer resolves `cid:` against this, so it has to ship embedded.
        assert!(inline
            .data_url
            .as_deref()
            .is_some_and(|value| value.starts_with("data:image/png;base64,")));

        let attached = preview
            .attachments
            .iter()
            .find(|attachment| !attachment.is_inline)
            .expect("pdf attachment survived parsing");
        assert_eq!(attached.filename, "statistik.pdf");
        // Kept out of the payload: it is fetched on demand when saving.
        assert!(attached.data_url.is_none());
    }

    #[test]
    fn strips_scripts_from_the_body_before_it_reaches_a_window() {
        let raw = sample_eml();
        let parsed = parse_message_bytes("/tmp/abschuss.eml", raw.as_bytes()).unwrap();
        let preview = build_preview("/tmp/abschuss.eml".to_string(), parsed);

        let html = preview.body_html.expect("html body");
        assert!(html.contains("Hallo"));
        assert!(
            !html.to_ascii_lowercase().contains("<script"),
            "unsanitized body reached the viewer: {html}"
        );
        assert!(!html.contains("tracker.example"));
    }

    #[test]
    fn claims_only_the_associated_extensions() {
        assert!(is_message_file(Path::new("/tmp/Angebot.eml")));
        assert!(is_message_file(Path::new("/tmp/Angebot.MSG")));
        assert!(!is_message_file(Path::new("/tmp/Angebot.pdf")));
        assert!(!is_message_file(Path::new("/tmp/Angebot")));
    }

    #[test]
    fn window_labels_are_stable_per_path_and_tauri_safe() {
        let first = viewer_window_label("/Users/x/Mail Ordner/Anfrage Müller #3.eml");
        assert_eq!(
            first,
            viewer_window_label("/Users/x/Mail Ordner/Anfrage Müller #3.eml")
        );
        assert_ne!(first, viewer_window_label("/Users/x/other.eml"));
        // Tauri rejects labels outside this alphabet, and mail file names carry
        // spaces and umlauts as a matter of course.
        assert!(first
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '/'));
    }
}

/// Tauri hands out permissions per window label. A window whose label matches
/// no capability runs with *none* of them, and nothing reports it: the window
/// opens, looks right, and every `invoke`, event listener and dialog call
/// silently fails. That is how the composer popout lost its close guard.
///
/// These tests keep the label producers and `capabilities/*.json` in step.
#[cfg(all(test, desktop))]
mod window_capabilities {
    use std::path::{Path, PathBuf};

    /// Files that build a window. A new entry is a new window family and needs
    /// both a capability and a label in [`runtime_labels`].
    const WINDOW_BUILDER_SITES: [&str; 4] = [
        "commands/message_file.rs",
        "commands/notifications.rs",
        "commands/tray.rs",
        "lib.rs",
    ];

    /// Matches `WebviewWindowBuilder::new(` too — it ends in the same suffix.
    const WINDOW_BUILDER_CALL: &str = "WindowBuilder::new(";

    fn crate_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    }

    struct Capability {
        file: String,
        windows: Vec<String>,
        permissions: Vec<String>,
        desktop: bool,
    }

    fn strings(value: &serde_json::Value, key: &str) -> Vec<String> {
        value[key]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn capabilities() -> Vec<Capability> {
        let dir = crate_dir().join("capabilities");
        let mut parsed = Vec::new();
        for entry in std::fs::read_dir(&dir)
            .expect("capabilities directory")
            .flatten()
        {
            let path = entry.path();
            if !path.extension().is_some_and(|ext| ext == "json") {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("read capability");
            let value: serde_json::Value = serde_json::from_str(&text)
                .unwrap_or_else(|err| panic!("{} is not valid JSON: {err}", path.display()));
            let platforms = strings(&value, "platforms");
            let mut windows = strings(&value, "windows");
            // Tauri reads an omitted list as "every window".
            if windows.is_empty() {
                windows.push("*".to_string());
            }
            parsed.push(Capability {
                file: path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                windows,
                permissions: strings(&value, "permissions"),
                // An omitted platform list means all of them.
                desktop: platforms.is_empty()
                    || platforms
                        .iter()
                        .any(|p| matches!(p.as_str(), "linux" | "macOS" | "windows")),
            });
        }
        assert!(
            !parsed.is_empty(),
            "no capability files in {}",
            dir.display()
        );
        parsed
    }

    /// Labels the desktop app actually opens: the ones declared in
    /// `tauri.conf.json` plus the ones built at runtime.
    fn runtime_labels() -> Vec<String> {
        let text = std::fs::read_to_string(crate_dir().join("tauri.conf.json"))
            .expect("read tauri.conf.json");
        let conf: serde_json::Value = serde_json::from_str(&text).expect("parse tauri.conf.json");
        let mut labels: Vec<String> = conf["app"]["windows"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            // Tauri labels an unlabelled config window `main`.
            .map(|window| window["label"].as_str().unwrap_or("main").to_string())
            .collect();
        labels.push(super::viewer_window_label(
            "/Users/x/Mail Ordner/Anfrage Müller #3.eml",
        ));
        labels.push(crate::commands::notifications::compose_window_label());
        labels.push(crate::commands::tray::POPOVER_LABEL.to_string());
        labels
    }

    /// Glob match with `*` and `?`, the way Tauri resolves a `windows` entry.
    fn glob_matches(pattern: &str, label: &str) -> bool {
        let pattern: Vec<char> = pattern.chars().collect();
        let label: Vec<char> = label.chars().collect();
        let (mut p, mut l) = (0, 0);
        let (mut star, mut resume) = (None, 0);
        while l < label.len() {
            if p < pattern.len() && (pattern[p] == '?' || pattern[p] == label[l]) {
                p += 1;
                l += 1;
            } else if p < pattern.len() && pattern[p] == '*' {
                star = Some(p);
                resume = l;
                p += 1;
            } else if let Some(at) = star {
                p = at + 1;
                resume += 1;
                l = resume;
            } else {
                return false;
            }
        }
        pattern[p..].iter().all(|ch| *ch == '*')
    }

    fn covering(label: &str) -> Vec<Capability> {
        capabilities()
            .into_iter()
            .filter(|cap| {
                cap.desktop
                    && cap
                        .windows
                        .iter()
                        .any(|pattern| glob_matches(pattern, label))
            })
            .collect()
    }

    #[test]
    fn glob_matching_follows_the_capability_patterns() {
        assert!(glob_matches("compose-*", "compose-1f2e"));
        assert!(glob_matches("main", "main"));
        assert!(!glob_matches("main", "main-2"));
        assert!(!glob_matches("compose-*", "composer"));
        assert!(!glob_matches("message-file-*", "compose-1f2e"));
    }

    #[test]
    fn every_window_label_is_covered_by_a_capability() {
        for label in runtime_labels() {
            let covering = covering(&label);
            assert!(
                !covering.is_empty(),
                "window `{label}` matches no `windows` entry in \
                 src-tauri/capabilities/*.json and would open without a single \
                 permission — no events, no dialogs, no close guard"
            );
        }
    }

    #[test]
    fn compose_popouts_may_listen_and_destroy() {
        let label = crate::commands::notifications::compose_window_label();
        let covering = covering(&label);
        // Name the files in the failure: which capability was meant to grant
        // this is the first thing you need and the last thing you can guess.
        let sources: Vec<&str> = covering.iter().map(|cap| cap.file.as_str()).collect();
        let granted: Vec<String> = covering
            .iter()
            .flat_map(|cap| cap.permissions.clone())
            .collect();
        // Without these two the close guard breaks in the one way nobody sees:
        // `onCloseRequested` never registers, and the save before `destroy()`
        // never runs.
        for permission in ["core:event:default", "core:window:allow-destroy"] {
            assert!(
                granted.iter().any(|held| held == permission),
                "`{permission}` is not granted to `{label}` by {sources:?}; \
                 granted: {granted:?}"
            );
        }
    }

    #[test]
    fn no_window_family_escapes_the_label_list() {
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

        let src = crate_dir().join("src");
        let mut files = Vec::new();
        walk(&src, &mut files);
        let mut sites: Vec<String> = files
            .iter()
            .filter(|path| {
                let source = std::fs::read_to_string(path).unwrap_or_default();
                let production = source.split("#[cfg(test)]").next().unwrap_or_default();
                production.contains(WINDOW_BUILDER_CALL)
                    || production.contains("WindowBuilder::from_config(")
            })
            .map(|path| {
                path.strip_prefix(&src)
                    .unwrap_or(path)
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();
        sites.sort();
        assert_eq!(
            sites, WINDOW_BUILDER_SITES,
            "a window is built somewhere new. Give the family a capability in \
             src-tauri/capabilities/, add its label to `runtime_labels`, then \
             list the file here"
        );
    }
}
