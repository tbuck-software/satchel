use base64::Engine;
use satchel_core::utils::filename::strip_filename_controls;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{FromRow, SqlitePool};
use tauri::{AppHandle, Manager};

use crate::{
    commands::error::{CommandError, CommandResult},
    providers::{provider_for_account, traits::MessageRef},
};

#[derive(Debug, Clone, Serialize, FromRow)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/attachments/")
)]
pub struct AttachmentDto {
    pub id: String,
    pub message_id: String,
    pub filename: Option<String>,
    pub content_type: String,
    pub size: Option<i64>,
    pub content_id: Option<String>,
    pub is_inline: i64,
    pub cached_path: Option<String>,
}

/// Most message ids one attachment listing accepts. A thread in a topic feed
/// has a handful; the bound keeps a runaway caller from building a statement
/// with tens of thousands of placeholders.
const MAX_ATTACHMENT_LISTING_IDS: usize = 500;

/// Attachment rows for several messages in one round trip.
///
/// Replaces a per-message command. A topic feed hydrating twenty threads of
/// eight mails made 160 invokes for this alone; a thread is now one.
#[tauri::command]
pub async fn list_attachments_for_messages(
    pool: tauri::State<'_, crate::db::ReadPool>,
    message_ids: Vec<String>,
) -> CommandResult<Vec<AttachmentDto>> {
    attachments_for_messages(pool.inner().as_ref(), &message_ids)
        .await
        .map_err(CommandError::unexpected)
}

pub(super) async fn attachments_for_messages(
    pool: &SqlitePool,
    message_ids: &[String],
) -> Result<Vec<AttachmentDto>, sqlx::Error> {
    if message_ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut rows = Vec::new();
    for chunk in message_ids.chunks(MAX_ATTACHMENT_LISTING_IDS) {
        let mut query = sqlx::QueryBuilder::new(
            "SELECT id, message_id, filename, content_type, size, content_id, is_inline, cached_path
             FROM attachments WHERE message_id IN (",
        );
        let mut ids = query.separated(", ");
        for id in chunk {
            ids.push_bind(id);
        }
        // `id` breaks filename ties: the renderer resolves `cid:` references
        // and checks decryption provenance by position.
        query.push(") ORDER BY message_id, filename, id");
        rows.extend(
            query
                .build_query_as::<AttachmentDto>()
                .fetch_all(pool)
                .await?,
        );
    }
    for row in &mut rows {
        row.filename = row.filename.as_deref().map(strip_filename_controls);
        if row
            .cached_path
            .as_deref()
            .is_some_and(|path| !cache_path_is_owned_by(path, &row.id, &row.message_id))
        {
            row.cached_path = None;
        }
    }
    Ok(rows)
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/attachments/", optional_fields = nullable))]
pub struct DownloadAttachmentInput {
    pub attachment_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/attachments/", optional_fields = nullable))]
pub struct OpenAttachmentInput {
    pub attachment_id: String,
    /// Set once the user has seen the full file name — both extensions — and
    /// asked for it anyway. Absent, an executable type is refused rather than
    /// handed to the operating system.
    #[serde(default)]
    pub confirmed_executable: bool,
}

impl From<OpenAttachmentInput> for DownloadAttachmentInput {
    fn from(input: OpenAttachmentInput) -> Self {
        Self {
            attachment_id: input.attachment_id,
        }
    }
}

/// Largest attachment handed over as base64 rather than as a path.
///
/// The same ceiling the `.eml` viewer uses for inline parts. Above it the cost
/// is absurd: a 50 MB file exists at once as the freshly read `Vec<u8>`, a
/// 67 MB base64 string, serde's JSON buffer, and the JS string at twice that
/// again as UTF-16 — roughly seven times the file, for something `cached_path`
/// answers on its own.
const MAX_BASE64_PAYLOAD_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/attachments/")
)]
pub struct DownloadAttachmentResult {
    pub filename: String,
    pub mime_type: String,
    /// Base64 payload, only up to [`MAX_BASE64_PAYLOAD_BYTES`]. Above that it
    /// is `None` and the caller reads `cached_path`, which is always set.
    pub data_base64: Option<String>,
    pub cached_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/attachments/")
)]
pub struct PreparedAttachmentResult {
    pub filename: String,
    pub mime_type: String,
    pub size: i64,
    pub cached_path: String,
}

#[tauri::command]
pub async fn download_attachment(
    app: AppHandle,
    pool: tauri::State<'_, SqlitePool>,
    input: DownloadAttachmentInput,
) -> CommandResult<DownloadAttachmentResult> {
    download_attachment_inner(&app, pool.inner(), input)
        .await
        .map_err(CommandError::from)
}

pub async fn download_attachment_inner(
    app: &AppHandle,
    pool: &SqlitePool,
    input: DownloadAttachmentInput,
) -> Result<DownloadAttachmentResult, String> {
    let prepared = prepare_attachment_inner(app, pool, input).await?;
    let size = tokio::fs::metadata(&prepared.cached_path)
        .await
        .map(|meta| meta.len())
        .unwrap_or(u64::MAX);

    let data_base64 = if size as usize <= MAX_BASE64_PAYLOAD_BYTES {
        let bytes = tokio::fs::read(&prepared.cached_path)
            .await
            .map_err(|err| format!("read attachment cache: {err}"))?;
        // Encoding a few megabytes is CPU-bound work on an async worker.
        Some(
            tokio::task::spawn_blocking(move || {
                base64::engine::general_purpose::STANDARD.encode(bytes)
            })
            .await
            .map_err(|err| format!("attachment encoding failed: {err}"))?,
        )
    } else {
        None
    };

    Ok(DownloadAttachmentResult {
        filename: prepared.filename,
        mime_type: prepared.mime_type,
        data_base64,
        cached_path: Some(prepared.cached_path),
    })
}

pub async fn prepare_attachment_inner(
    app: &AppHandle,
    pool: &SqlitePool,
    input: DownloadAttachmentInput,
) -> Result<PreparedAttachmentResult, String> {
    let cache_dir = app
        .path()
        .resolve("satchel/attachments", tauri::path::BaseDirectory::AppData)
        .map_err(|err| err.to_string())?;
    prepare_attachment_in(&cache_dir, pool, input).await
}

async fn prepare_attachment_in(
    cache_dir: &std::path::Path,
    pool: &SqlitePool,
    input: DownloadAttachmentInput,
) -> Result<PreparedAttachmentResult, String> {
    let row: Option<(
        String,
        Option<String>,
        String,
        Option<String>,
        String,
        String,
        String,
        Option<String>,
        Option<String>,
        String,
        Option<String>,
        String,
        String,
        Option<String>,
    )> = sqlx::query_as(
        "SELECT a.id, a.filename, a.content_type, a.cached_path, m.account_id, acc.provider, acc.config, COALESCE(a.remote_uid, m.remote_id), a.remote_part_id, acc.email, f.remote_id, f.full_path, m.id, a.content_base64
         FROM attachments a
         JOIN messages m ON m.id = a.message_id
         JOIN accounts acc ON acc.id = m.account_id
         JOIN folders f ON f.id = m.folder_id
         WHERE a.id = ?",
    )
    .bind(&input.attachment_id)
    .fetch_optional(pool)
    .await
    .map_err(|err| err.to_string())?;
    let Some((
        id,
        filename,
        mime,
        cached_path,
        account_id,
        provider_kind,
        config,
        remote_uid,
        remote_part_id,
        account_email,
        folder_remote_id,
        folder_path,
        message_id,
        content_base64,
    )) = row
    else {
        return Err("attachment not found".into());
    };
    let filename = strip_filename_controls(&filename.unwrap_or_else(|| "attachment".into()));
    tokio::fs::create_dir_all(cache_dir)
        .await
        .map_err(|err| format!("create attachment cache: {err}"))?;

    if let Some(path) = cached_path.as_deref() {
        if cache_path_is_owned_by(path, &id, &message_id) {
            if let Some(size) = validated_cached_file_size(cache_dir, path).await {
                return Ok(PreparedAttachmentResult {
                    filename,
                    mime_type: mime,
                    size,
                    cached_path: path.to_string(),
                });
            }
        }
    }

    let fetched = if let Some(payload) = content_base64.as_deref() {
        crate::providers::traits::FetchedAttachment {
            filename: Some(filename.clone()),
            content_type: mime.clone(),
            bytes: crate::sync::attachment_fetch::decode_attachment_base64(payload)
                .map_err(|error| format!("attachment payload decode failed: {error}"))?,
        }
    } else if satchel_core::email::decrypted::is_attachment_id(&id) {
        // The server holds this part only inside the encrypted message, and a
        // part ID from the decrypted content names another part there.
        return Err("backend.errors.decryptedAttachmentUnavailable".into());
    } else {
        let remote_uid = remote_uid.ok_or_else(|| {
            format!(
                "attachment {} has no remote message id; re-sync the message first",
                input.attachment_id
            )
        })?;
        let remote_part_id = remote_part_id.ok_or_else(|| {
            format!(
                "attachment {} has no remote part id; re-sync the message first",
                input.attachment_id
            )
        })?;
        let config =
            crate::utils::oauth::refresh_account_config(pool, &account_id, &provider_kind, &config)
                .await
                .map_err(|err| err.encoded())?;
        let provider = provider_for_account(&provider_kind, &config, &account_email, None)?;
        provider
            .fetch_attachment(
                &MessageRef {
                    remote_id: remote_uid.clone(),
                    source_folder: folder_remote_id.or(Some(folder_path)),
                    source_uidvalidity: None,
                },
                &remote_part_id,
            )
            .await
            .map_err(|error| error.to_string())?
    };

    let fetched_mime = fetched.content_type.trim();
    let mime_type = if fetched_mime.is_empty()
        || (fetched_mime.eq_ignore_ascii_case("application/octet-stream")
            && !mime.eq_ignore_ascii_case("application/octet-stream"))
    {
        mime
    } else {
        fetched.content_type
    };
    let final_filename = fetched.filename.unwrap_or(filename.clone());
    let path = attachment_cache_path(cache_dir, &id, &final_filename);
    if tokio::fs::symlink_metadata(&path)
        .await
        .is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        tokio::fs::remove_file(&path)
            .await
            .map_err(|err| format!("remove unsafe attachment cache link: {err}"))?;
    }
    tokio::fs::write(&path, &fetched.bytes)
        .await
        .map_err(|err| format!("write attachment cache: {err}"))?;
    let path_string = path.to_string_lossy().to_string();
    let size = i64::try_from(fetched.bytes.len()).unwrap_or(i64::MAX);
    set_attachment_cache(pool, &id, &path_string, size).await?;
    sqlx::query(
        "UPDATE attachments
         SET filename = ?, content_type = ?
         WHERE id = ?",
    )
    .bind(&final_filename)
    .bind(&mime_type)
    .bind(&id)
    .execute(pool)
    .await
    .map_err(|err| err.to_string())?;

    Ok(PreparedAttachmentResult {
        filename: final_filename,
        mime_type,
        size,
        cached_path: path_string,
    })
}

async fn set_attachment_cache(
    pool: &SqlitePool,
    id: &str,
    path: &str,
    size: i64,
) -> Result<(), String> {
    sqlx::query("UPDATE attachments SET cached_path = ?, size = ? WHERE id = ?")
        .bind(path)
        .bind(size)
        .bind(id)
        .execute(pool)
        .await
        .map_err(|err| err.to_string())?;
    Ok(())
}

pub(crate) fn cache_path_is_owned_by(path: &str, attachment_id: &str, message_id: &str) -> bool {
    let Some(name) = std::path::Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
    else {
        return false;
    };
    // Imports used the message id as an extra part of the cache key. Both
    // forms are owned by this row; a peer's key is not, even if its UID matches.
    [
        attachment_id.to_string(),
        format!("{message_id}-{attachment_id}"),
    ]
    .iter()
    .any(|key| name.starts_with(&format!("{}-", short_attachment_cache_key(key))))
}

/// Only paths that resolve below Satchel's attachment cache may be handed to
/// Quick Look or another application. Canonicalization also rejects symlinks
/// that point outside the cache even if the database row was tampered with.
async fn validated_cached_file_size(cache_dir: &std::path::Path, path: &str) -> Option<i64> {
    let root = tokio::fs::canonicalize(cache_dir).await.ok()?;
    let file = tokio::fs::canonicalize(path).await.ok()?;
    if !file.starts_with(root) {
        return None;
    }
    let metadata = tokio::fs::metadata(file).await.ok()?;
    metadata
        .is_file()
        .then(|| i64::try_from(metadata.len()).unwrap_or(i64::MAX))
}

#[tauri::command]
pub async fn open_attachment_default(
    app: AppHandle,
    pool: tauri::State<'_, SqlitePool>,
    input: OpenAttachmentInput,
) -> CommandResult<()> {
    use tauri_plugin_opener::OpenerExt;

    let confirmed = input.confirmed_executable;
    let prepared = prepare_attachment_inner(&app, pool.inner(), input.into()).await?;
    let cache_filename = std::path::Path::new(&prepared.cached_path)
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| CommandError::unexpected("invalid cached attachment filename"))?;
    let risk = attachment_open_risk(cache_filename);
    if risk.executable && !confirmed {
        // A second extension is the part the user cannot see in a chip that
        // shows an icon and a truncated name, so it gets its own sentence.
        let key = if risk.double_extension {
            "backend.errors.attachmentExecutableDisguisedNotConfirmed"
        } else {
            "backend.errors.attachmentExecutableNotConfirmed"
        };
        return Err(CommandError::new(key)
            .with_param("filename", prepared.filename)
            .with_param("extension", risk.extension.unwrap_or_default()));
    }
    quarantine_for_external_open(std::path::Path::new(&prepared.cached_path))?;
    app.opener()
        .open_path(&prepared.cached_path, None::<&str>)
        .map_err(|err| CommandError::unexpected(format!("open attachment: {err}")))
}

/// Types the operating system runs rather than displays. The list is the
/// hand-off gate, not a malware verdict: it decides what needs a confirmed
/// click before `open_path` sees it.
const EXECUTABLE_ATTACHMENT_EXTENSIONS: &[&str] = &[
    "action",
    "apk",
    "app",
    "appimage",
    "bat",
    "cmd",
    "com",
    "command",
    "cpl",
    "desktop",
    "dmg",
    "exe",
    "gadget",
    "hta",
    "inf",
    "ins",
    "jar",
    "js",
    "jse",
    "lnk",
    "msc",
    "msi",
    "msp",
    "pif",
    "pkg",
    "ps1",
    "psm1",
    "reg",
    "run",
    "scf",
    "scr",
    "sct",
    "vb",
    "vbe",
    "vbs",
    "workflow",
    "ws",
    "wsf",
    "wsh",
    "terminal",
    "tool",
    "fileloc",
    "inetloc",
    "webloc",
    "shortcut",
    "mobileconfig",
    "html",
    "htm",
    "svg",
    "url",
    "iso",
    "img",
    "vhd",
    "vhdx",
    "chm",
    "msix",
    "appx",
    "appinstaller",
    "xll",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentOpenRisk {
    pub executable: bool,
    /// Lowercased final extension, when the name has one.
    pub extension: Option<String>,
    /// `rechnung.pdf.exe` — the name reads as a document until the last dot.
    pub double_extension: bool,
}

/// Pure so it can be exercised without a Tauri context or a cache directory.
pub fn attachment_open_risk(filename: &str) -> AttachmentOpenRisk {
    let filename = strip_filename_controls(filename);
    let segments: Vec<&str> = filename.rsplit('.').collect();
    let extension = segments
        .first()
        .filter(|_| segments.len() > 1)
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty());
    let executable = extension
        .as_deref()
        .is_some_and(|value| EXECUTABLE_ATTACHMENT_EXTENSIONS.contains(&value));
    let double_extension = segments.len() > 2
        && segments
            .get(1)
            .is_some_and(|inner| looks_like_an_extension(inner));
    AttachmentOpenRisk {
        executable,
        extension,
        double_extension,
    }
}

fn looks_like_an_extension(value: &str) -> bool {
    let value = value.trim();
    (1..=5).contains(&value.len()) && value.chars().all(|ch| ch.is_ascii_alphanumeric())
}

/// Mark a cache file as foreign before another application touches it: the
/// per-platform equivalents of "this came from the network".
fn quarantine_for_external_open(path: &std::path::Path) -> Result<(), String> {
    strip_executable_bits(path)?;
    mark_as_downloaded(path)
}

#[cfg(unix)]
fn strip_executable_bits(path: &std::path::Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    let metadata = std::fs::metadata(path).map_err(|err| format!("stat attachment: {err}"))?;
    let mode = metadata.permissions().mode();
    if mode & 0o111 == 0 {
        return Ok(());
    }
    // GNOME and KDE only launch a `.desktop` file that carries the bit.
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode & !0o111))
        .map_err(|err| format!("clear attachment execute bit: {err}"))
}

#[cfg(not(unix))]
fn strip_executable_bits(_path: &std::path::Path) -> Result<(), String> {
    Ok(())
}

#[cfg(target_os = "macos")]
fn mark_as_downloaded(path: &std::path::Path) -> Result<(), String> {
    use std::ffi::{c_char, c_int, c_void, CString};
    use std::os::unix::ffi::OsStrExt;
    use std::time::{SystemTime, UNIX_EPOCH};

    unsafe extern "C" {
        fn setxattr(
            path: *const c_char,
            name: *const c_char,
            value: *const c_void,
            size: usize,
            position: u32,
            options: c_int,
        ) -> c_int;
    }

    let path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| "attachment cache path contains a null byte".to_string())?;
    let name = c"com.apple.quarantine";
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| format!("read system time for quarantine: {err}"))?
        .as_secs();
    // 0x83 matches a downloaded item opened after an explicit user action.
    let value = format!("0083;{timestamp:x};Satchel;");
    // SAFETY: all pointers stay valid for the call and macOS copies the value.
    let result = unsafe {
        setxattr(
            path.as_ptr(),
            name.as_ptr(),
            value.as_ptr().cast(),
            value.len(),
            0,
            0,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(format!(
            "mark attachment as downloaded: {}",
            std::io::Error::last_os_error()
        ))
    }
}

/// Windows has no quarantine xattr; the Attachment Manager reads the
/// `Zone.Identifier` alternate data stream instead, and zone 3 is "internet".
/// Not covered by CI — the platform is neither built nor released here.
#[cfg(target_os = "windows")]
fn mark_as_downloaded(path: &std::path::Path) -> Result<(), String> {
    let mut stream = path.as_os_str().to_os_string();
    stream.push(":Zone.Identifier");
    std::fs::write(
        std::path::Path::new(&stream),
        "[ZoneTransfer]\r\nZoneId=3\r\n",
    )
    .map_err(|err| format!("mark attachment as downloaded: {err}"))
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn mark_as_downloaded(_path: &std::path::Path) -> Result<(), String> {
    Ok(())
}

#[tauri::command]
pub async fn toggle_attachment_quick_look(
    app: AppHandle,
    pool: tauri::State<'_, SqlitePool>,
    input: DownloadAttachmentInput,
) -> CommandResult<bool> {
    let prepared = prepare_attachment_inner(&app, pool.inner(), input).await?;

    #[cfg(target_os = "macos")]
    {
        let path = prepared.cached_path;
        let (sender, receiver) = tokio::sync::oneshot::channel();
        app.run_on_main_thread(move || {
            let _ = sender.send(crate::utils::mac_quick_look::toggle(&path));
        })
        .map_err(|err| format!("schedule Quick Look: {err}"))?;
        receiver
            .await
            .map_err(|_| "Quick Look did not answer".to_string())??;
        Ok(true)
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = prepared;
        Ok(false)
    }
}

/// Cache file name for an attachment: a digest of the attachment id plus a
/// trimmed version of the original name.
///
/// The id must never reach the file name. Gmail part ids run past 500
/// characters, so `{id}-{filename}` blows the 255-byte name limit of every
/// common file system — and the sync path used to build exactly that. The write
/// failed with `ENAMETOOLONG`, the error was discarded, and every freshly
/// downloaded inline image was thrown away: 9 700 parts fetched from Gmail for
/// nothing, then fetched again on every open until the account hit a 429.
/// Regression guard: `cache_path_stays_within_file_name_limits`.
pub(crate) fn attachment_cache_path(
    cache_dir: &std::path::Path,
    attachment_id: &str,
    filename: &str,
) -> std::path::PathBuf {
    cache_dir.join(format!(
        "{}-{}",
        short_attachment_cache_key(attachment_id),
        safe_filename(filename)
    ))
}

/// Longest original name kept in a cache file name. The digest prefix already
/// carries uniqueness, so this only has to stay recognisable in a file listing.
const MAX_CACHE_NAME_BYTES: usize = 120;

fn short_attachment_cache_key(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    digest
        .iter()
        .take(12)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn safe_filename(value: &str) -> String {
    // Every branch yields ASCII, so byte and char lengths agree below.
    let sanitized = strip_filename_controls(value)
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    if sanitized.trim_matches('_').is_empty() {
        return "attachment".to_string();
    }
    if sanitized.len() <= MAX_CACHE_NAME_BYTES {
        return sanitized;
    }
    // Keep the extension: it is what makes the cache dir readable, and Finder
    // and `open` both key off it.
    match sanitized.rsplit_once('.') {
        Some((stem, ext)) if !ext.is_empty() && ext.len() < 16 => {
            let budget = MAX_CACHE_NAME_BYTES - ext.len() - 1;
            format!("{}.{ext}", &stem[..budget.min(stem.len())])
        }
        _ => sanitized[..MAX_CACHE_NAME_BYTES].to_string(),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_final_cache_name_is_the_opening_gate() {
        let original = format!("{}.jar{}", "A".repeat(116), "X".repeat(16));
        assert!(!super::attachment_open_risk(&original).executable);
        let path = super::attachment_cache_path(std::path::Path::new("/cache"), "id", &original);
        assert!(
            super::attachment_open_risk(path.file_name().unwrap().to_str().unwrap()).executable
        );
    }

    #[test]
    fn active_documents_installers_and_shortcuts_require_confirmation() {
        for extension in [
            "terminal",
            "tool",
            "fileloc",
            "inetloc",
            "webloc",
            "shortcut",
            "mobileconfig",
            "html",
            "svg",
            "url",
            "iso",
            "img",
            "vhd",
            "vhdx",
            "chm",
            "msix",
            "appx",
            "appinstaller",
            "xll",
        ] {
            assert!(super::attachment_open_risk(&format!("invoice.{extension}")).executable);
        }
        assert_eq!(
            super::safe_filename("Rechnung\u{202e}fdp.html"),
            "Rechnungfdp.html"
        );
    }

    /// The ceiling, stated where someone changing it will see it.
    ///
    /// A 50 MB attachment used to exist four times over on the way to the
    /// window: the read buffer, the base64 string a third larger, serde's JSON
    /// buffer, and the JS string at twice that again as UTF-16.
    #[test]
    fn the_base64_ceiling_matches_the_viewer() {
        assert_eq!(
            super::MAX_BASE64_PAYLOAD_BYTES,
            4 * 1024 * 1024,
            "the .eml viewer drops inline parts at the same size"
        );
    }

    /// One round trip for a whole thread, rows grouped by message and none
    /// borrowed from a message nobody asked about.
    #[tokio::test]
    async fn lists_the_attachments_of_several_messages_at_once() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("pool");
        sqlx::query(
            "CREATE TABLE attachments (id TEXT PRIMARY KEY, message_id TEXT NOT NULL,
             filename TEXT, content_type TEXT NOT NULL, size INTEGER, content_id TEXT,
             is_inline INTEGER NOT NULL DEFAULT 0, cached_path TEXT)",
        )
        .execute(&pool)
        .await
        .expect("schema");
        // `z9` is inserted first but shares `a2`'s name: a tie the order has
        // to break the same way on every call, because the renderer resolves
        // `cid:` references by position and checks provenance by position.
        for (id, message, name) in [
            ("z9", "m1", "a.png"),
            ("a1", "m1", "b.pdf"),
            ("a2", "m1", "a.png"),
            ("a3", "m2", "c\u{202e}.txt"),
            ("a4", "m3", "other.doc"),
        ] {
            sqlx::query(
                "INSERT INTO attachments (id, message_id, filename, content_type)
                 VALUES (?, ?, ?, 'application/octet-stream')",
            )
            .bind(id)
            .bind(message)
            .bind(name)
            .execute(&pool)
            .await
            .expect("row");
        }
        let shared = super::attachment_cache_path(std::path::Path::new("/cache"), "a2", "a.png");
        sqlx::query("UPDATE attachments SET cached_path = ? WHERE id IN ('a1', 'a2')")
            .bind(shared.to_string_lossy().to_string())
            .execute(&pool)
            .await
            .expect("legacy shared cache path");

        let rows = super::attachments_for_messages(&pool, &["m1".into(), "m2".into()])
            .await
            .expect("listing");

        let ids: Vec<&str> = rows.iter().map(|row| row.id.as_str()).collect();
        assert_eq!(ids, ["a2", "z9", "a1", "a3"]);
        assert_eq!(rows[0].cached_path.as_deref(), shared.to_str());
        assert_eq!(rows[3].filename.as_deref(), Some("c.txt"));
        assert_eq!(rows[2].cached_path, None, "a1 must not expose a2's bytes");
        assert!(super::attachments_for_messages(&pool, &[])
            .await
            .expect("empty")
            .is_empty());
    }

    use super::{attachment_cache_path, attachment_open_risk, safe_filename, MAX_CACHE_NAME_BYTES};

    /// A real Gmail attachment id, shortened only in the middle.
    fn gmail_attachment_id() -> String {
        format!(
            "att-gmail-m-gmail-acc-69a874a9-18a9-4459-a74e-2fa36a723d5f-ARCHIVE-19f75b00cd2efc9d-{}",
            "ANGjdJ".repeat(70)
        )
    }

    #[test]
    fn cache_path_stays_within_file_name_limits() {
        let id = gmail_attachment_id();
        assert!(id.len() > 255, "test id no longer exercises the limit");

        let path = attachment_cache_path(std::path::Path::new("/cache"), &id, "image001.jpg");
        let name = path.file_name().unwrap().to_string_lossy();

        assert!(name.len() < 255, "cache file name too long: {}", name.len());
        assert!(name.ends_with("-image001.jpg"));
    }

    #[test]
    fn cache_path_separates_attachments_that_share_a_file_name() {
        let dir = std::path::Path::new("/cache");
        let first = attachment_cache_path(dir, "att-a", "image001.jpg");
        let second = attachment_cache_path(dir, "att-b", "image001.jpg");

        assert_ne!(first, second);
        // Stable across calls: the cached_path in the database has to stay valid.
        assert_eq!(first, attachment_cache_path(dir, "att-a", "image001.jpg"));
    }

    #[test]
    fn a_shared_cache_path_is_not_owned_by_the_other_attachment() {
        let dir = std::path::Path::new("/cache");
        let first = attachment_cache_path(dir, "att-inbox", "same.pdf");
        let imported = attachment_cache_path(dir, "message-att-import", "same.pdf");

        assert!(super::cache_path_is_owned_by(
            first.to_str().unwrap(),
            "att-inbox",
            "message"
        ));
        assert!(!super::cache_path_is_owned_by(
            first.to_str().unwrap(),
            "att-archive",
            "message"
        ));
        assert!(super::cache_path_is_owned_by(
            imported.to_str().unwrap(),
            "att-import",
            "message"
        ));
    }

    #[tokio::test]
    async fn caching_one_part_never_changes_another_row_with_the_same_remote_uid() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("pool");
        sqlx::query("CREATE TABLE attachments (id TEXT PRIMARY KEY, remote_uid TEXT, remote_part_id TEXT, cached_path TEXT, size INTEGER)")
            .execute(&pool).await.expect("schema");
        sqlx::query("INSERT INTO attachments (id, remote_uid, remote_part_id) VALUES ('inbox', '42', '1'), ('archive', '42', '1')")
            .execute(&pool).await.expect("rows");

        super::set_attachment_cache(&pool, "inbox", "/cache/inbox.pdf", 4)
            .await
            .expect("cache inbox");

        let other: Option<String> =
            sqlx::query_scalar("SELECT cached_path FROM attachments WHERE id = 'archive'")
                .fetch_one(&pool)
                .await
                .expect("archive path");
        assert_eq!(other, None);
    }

    /// A part of decrypted content whose cache file is gone, released by the
    /// rolling body window or deleted by maintenance. Its part ID names a part
    /// of the decrypted plaintext; on the server the same ID is a part of the
    /// encrypted message, so it is never fetched under this row.
    #[tokio::test]
    async fn a_decrypted_part_without_its_cache_file_is_not_fetched_from_the_server() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("pool");
        sqlx::migrate!("../crates/satchel-core/src/db/migrations")
            .run(&pool)
            .await
            .expect("migrations");
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO accounts
             (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
             VALUES ('acc', 'Me', 'me@example.com', 'imap', '{}', '', ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("account");
        sqlx::query(
            "INSERT INTO folders
             (id, account_id, name, full_path, remote_id, folder_type, created_at, updated_at)
             VALUES ('fld', 'acc', 'INBOX', 'INBOX', 'INBOX', 'inbox', ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("folder");
        sqlx::query(
            "INSERT INTO messages
             (id, account_id, folder_id, from_address, to_addresses, date, is_draft, remote_id,
              created_at, updated_at)
             VALUES ('msg', 'acc', 'fld', 'a@b.c', '[]', ?, 0, '12', ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("message");

        let cache = tempfile::tempdir().expect("cache dir");
        let id = satchel_core::email::decrypted::attachment_id("msg", "mime-1");
        let cached = attachment_cache_path(cache.path(), &id, "agenda.pdf");
        std::fs::write(&cached, b"%PDF").expect("cache file");
        // Rows written before decrypted parts lost their remote part ID.
        sqlx::query(
            "INSERT INTO attachments
             (id, message_id, filename, content_type, size, is_inline, cached_path,
              remote_part_id, created_at)
             VALUES (?, 'msg', 'agenda.pdf', 'application/pdf', 4, 0, ?, 'mime-1', ?)",
        )
        .bind(&id)
        .bind(cached.to_string_lossy().to_string())
        .bind(&now)
        .execute(&pool)
        .await
        .expect("attachment");
        let prepare = || {
            super::prepare_attachment_in(
                cache.path(),
                &pool,
                super::DownloadAttachmentInput {
                    attachment_id: id.clone(),
                },
            )
        };
        assert!(prepare().await.is_ok(), "the cached copy is served");

        std::fs::remove_file(&cached).expect("cache file removed");
        let missing_file = prepare().await;
        sqlx::query("UPDATE attachments SET cached_path = NULL WHERE id = ?")
            .bind(&id)
            .execute(&pool)
            .await
            .expect("released");
        let released = prepare().await;
        for outcome in [missing_file, released] {
            assert_eq!(
                outcome.err().as_deref(),
                Some("backend.errors.decryptedAttachmentUnavailable")
            );
        }
        let row: (Option<String>, String) =
            sqlx::query_as("SELECT filename, content_type FROM attachments WHERE id = ?")
                .bind(&id)
                .fetch_one(&pool)
                .await
                .expect("row");
        assert_eq!(row, (Some("agenda.pdf".into()), "application/pdf".into()));
    }

    #[test]
    fn long_sender_file_names_are_trimmed_but_keep_their_extension() {
        let name = safe_filename(&format!("{}.pdf", "Rechnung ".repeat(40)));

        assert!(name.len() <= MAX_CACHE_NAME_BYTES);
        assert!(name.ends_with(".pdf"));
    }

    #[test]
    fn unnamed_attachments_fall_back_to_a_usable_name() {
        assert_eq!(safe_filename("   "), "attachment");
        assert_eq!(safe_filename(""), "attachment");
    }

    #[test]
    fn executable_attachment_names_are_flagged() {
        let cases = [
            ("rechnung.pdf", false, false),
            ("rechnung.pdf.exe", true, true),
            ("setup.desktop", true, false),
            ("bild.png", false, false),
            ("Rechnung.PDF.ExE", true, true),
            ("archive.tar.gz", false, true),
            ("noextension", false, false),
        ];

        for (name, executable, double) in cases {
            let risk = attachment_open_risk(name);
            assert_eq!(risk.executable, executable, "{name}");
            assert_eq!(risk.double_extension, double, "{name}");
        }

        assert_eq!(
            attachment_open_risk("rechnung.pdf.exe")
                .extension
                .as_deref(),
            Some("exe")
        );
    }

    #[cfg(unix)]
    #[test]
    fn preparing_an_attachment_leaves_no_execute_bit() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("setup.desktop");
        std::fs::write(&path, b"[Desktop Entry]").expect("write cache file");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("set permissions");

        super::quarantine_for_external_open(&path).expect("hardening a cache file");

        let mode = std::fs::metadata(&path).expect("stat").permissions().mode();
        assert_eq!(mode & 0o111, 0, "execute bits survived: {mode:o}");
    }
}
