use base64::Engine;
use satchel_core::utils::filename::strip_filename_controls;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};
use tauri::{AppHandle, Manager};

use crate::{
    commands::error::{CommandError, CommandResult},
    db::remote_content::{attachment_cache_id, ContentReference},
    providers::provider_for_account,
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
    state: tauri::State<'_, crate::AppState>,
    pool: tauri::State<'_, crate::db::ReadPool>,
    message_ids: Vec<String>,
) -> CommandResult<Vec<AttachmentDto>> {
    attachments_for_messages(
        pool.inner().as_ref(),
        &message_ids,
        &state.shown_decrypted_parts,
    )
    .await
    .map_err(CommandError::unexpected)
}

/// The parts of decrypted content each message's latest read showed, by
/// attachment cache ID, so a part belongs to the saved generation it was
/// decrypted under.
///
/// Beside a root signature a part of decrypted content is listed only while
/// the body it came from is shown decrypted: the signature covers the
/// ciphertext, never the part. Read again without the key, the block stays
/// armored and the verdict is valid again; the row and its cached file stay
/// for the next decryption, and this set keeps them out of the listing. A
/// read that shows no decrypted part forgets the message, a restart forgets
/// every one.
#[derive(Debug, Default)]
pub struct ShownDecryptedParts(
    std::sync::Mutex<std::collections::HashMap<String, std::collections::HashSet<String>>>,
);

impl ShownDecryptedParts {
    /// What the latest read of `message_id` showed, in place of any earlier one.
    pub(crate) fn record(&self, message_id: &str, cache_ids: std::collections::HashSet<String>) {
        let mut shown = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if cache_ids.is_empty() {
            shown.remove(message_id);
        } else {
            shown.insert(message_id.to_string(), cache_ids);
        }
    }

    /// The latest read of `message_id` showed nothing of it decrypted.
    pub(crate) fn forget(&self, message_id: &str) {
        self.record(message_id, Default::default());
    }

    fn shows(&self, message_id: &str, cache_id: &str) -> bool {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(message_id)
            .is_some_and(|shown| shown.contains(cache_id))
    }
}

#[derive(FromRow)]
struct AttachmentListingRow {
    #[sqlx(flatten)]
    attachment: AttachmentDto,
    provider: String,
    remote_id: Option<String>,
    remote_uidvalidity: Option<i64>,
    decrypted_beside_root_signature: bool,
}

pub(super) async fn attachments_for_messages(
    pool: &SqlitePool,
    message_ids: &[String],
    shown_decrypted: &ShownDecryptedParts,
) -> Result<Vec<AttachmentDto>, sqlx::Error> {
    if message_ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut rows = Vec::new();
    for chunk in message_ids.chunks(MAX_ATTACHMENT_LISTING_IDS) {
        let mut query = sqlx::QueryBuilder::new(format!(
            "SELECT a.id, a.message_id, a.filename, a.content_type, a.size, a.content_id, a.is_inline, a.cached_path,
                    acc.provider, m.remote_id, m.remote_uidvalidity,
                    {} AS decrypted_beside_root_signature
             FROM attachments a JOIN messages m ON m.id = a.message_id
             JOIN accounts acc ON acc.id = m.account_id WHERE a.message_id IN (",
            satchel_core::db::signed_attachments::DECRYPTED_BESIDE_ROOT_SIGNATURE
        ));
        let mut ids = query.separated(", ");
        for id in chunk {
            ids.push_bind(id);
        }
        // Beside a root signature, rows from older listing rules may name
        // parts from outside the signed entity; they wait for their rebuild.
        // `id` breaks filename ties: the renderer resolves `cid:` references
        // and checks decryption provenance by position.
        query.push(format!(
            ") AND NOT {} ORDER BY a.message_id, a.filename, a.id",
            satchel_core::db::signed_attachments::HIDDEN_BESIDE_ROOT_SIGNATURE
        ));
        rows.extend(
            query
                .build_query_as::<AttachmentListingRow>()
                .fetch_all(pool)
                .await?,
        );
    }
    let rows = rows
        .into_iter()
        .filter_map(|listing| {
            let mut row = listing.attachment;
            let cache_id = attachment_cache_id(
                &listing.provider,
                listing.remote_id.as_deref(),
                listing.remote_uidvalidity,
                &row.id,
            );
            // Beside a root signature, a decrypted part only with its body.
            if listing.decrypted_beside_root_signature
                && !shown_decrypted.shows(&row.message_id, &cache_id)
            {
                return None;
            }
            row.filename = row.filename.as_deref().map(strip_filename_controls);
            if row
                .cached_path
                .as_deref()
                .is_some_and(|path| !cache_path_is_owned_by(path, &cache_id, &row.message_id))
            {
                row.cached_path = None;
            }
            Some(row)
        })
        .collect();
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
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ipc-types", ts(optional = nullable))]
    pub remote_uidvalidity: Option<u32>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ipc-types", ts(optional = nullable))]
    pub remote_uidvalidity: Option<u32>,
    pub filename: String,
    pub mime_type: String,
    pub size: i64,
    pub cached_path: String,
}

/// Keep the saved identity beside the file until the last asynchronous read.
struct ReferencedAttachment {
    result: PreparedAttachmentResult,
    reference: ContentReference,
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
    let cache_dir = attachment_cache_directory(app)?;
    let prepared = prepare_referenced_attachment(&cache_dir, pool, input).await?;
    download_prepared_attachment(pool, prepared).await
}

async fn download_prepared_attachment(
    pool: &SqlitePool,
    prepared: ReferencedAttachment,
) -> Result<DownloadAttachmentResult, String> {
    let ReferencedAttachment {
        result: prepared,
        reference,
    } = prepared;
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

    // Metadata, cache reads and encoding all yield. Validate the captured
    // identity after them for both inline bytes and the large-file path.
    let mut tx = pool.begin().await.map_err(|error| error.to_string())?;
    reference.ensure_current(&mut tx).await?;
    Ok(DownloadAttachmentResult {
        remote_uidvalidity: prepared.remote_uidvalidity,
        filename: prepared.filename,
        mime_type: prepared.mime_type,
        data_base64,
        cached_path: Some(prepared.cached_path),
    })
}

#[derive(FromRow)]
struct AttachmentTarget {
    id: String,
    filename: Option<String>,
    mime: String,
    cached_path: Option<String>,
    account_id: String,
    provider_kind: String,
    config: String,
    remote_part_id: Option<String>,
    account_email: String,
    message_id: String,
    content_base64: Option<String>,
}

pub async fn prepare_attachment_inner(
    app: &AppHandle,
    pool: &SqlitePool,
    input: DownloadAttachmentInput,
) -> Result<PreparedAttachmentResult, String> {
    let cache_dir = attachment_cache_directory(app)?;
    prepare_attachment_in_dir(&cache_dir, pool, input).await
}

fn attachment_cache_directory(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    app.path()
        .resolve("satchel/attachments", tauri::path::BaseDirectory::AppData)
        .map_err(|err| err.to_string())
}

pub(super) async fn prepare_attachment_in_dir(
    cache_dir: &std::path::Path,
    pool: &SqlitePool,
    input: DownloadAttachmentInput,
) -> Result<PreparedAttachmentResult, String> {
    Ok(prepare_referenced_attachment(cache_dir, pool, input)
        .await?
        .result)
}

async fn prepare_referenced_attachment(
    cache_dir: &std::path::Path,
    pool: &SqlitePool,
    input: DownloadAttachmentInput,
) -> Result<ReferencedAttachment, String> {
    let message_id: String = sqlx::query_scalar("SELECT message_id FROM attachments WHERE id = ?")
        .bind(&input.attachment_id)
        .fetch_optional(pool)
        .await
        .map_err(|err| err.to_string())?
        .ok_or_else(|| "backend.errors.messageNotAvailableLocally".to_string())?;
    let reference = ContentReference::load(pool, &message_id).await?;
    reference.validate_cached_read()?;
    let row: Option<AttachmentTarget> = sqlx::query_as(
        "SELECT a.id, a.filename, a.content_type AS mime, a.cached_path, m.account_id,
                acc.provider AS provider_kind, acc.config, a.remote_part_id,
                acc.email AS account_email, m.id AS message_id, a.content_base64
         FROM attachments a
         JOIN messages m ON m.id = a.message_id
         JOIN accounts acc ON acc.id = m.account_id
         WHERE a.id = ?",
    )
    .bind(&input.attachment_id)
    .fetch_optional(pool)
    .await
    .map_err(|err| err.to_string())?;
    let Some(AttachmentTarget {
        id,
        filename,
        mime,
        cached_path,
        account_id,
        provider_kind,
        config,
        remote_part_id,
        account_email,
        message_id,
        content_base64,
    }) = row
    else {
        return Err("backend.errors.messageNotAvailableLocally".into());
    };
    if message_id != reference.message_id {
        return Err(reference.changed_error());
    }
    let filename = strip_filename_controls(&filename.unwrap_or_else(|| "attachment".into()));
    tokio::fs::create_dir_all(cache_dir)
        .await
        .map_err(|err| format!("create attachment cache: {err}"))?;

    let cache_id = reference.attachment_cache_id(&id);
    if let Some(path) = cached_path.as_deref() {
        if cache_path_is_owned_by(path, &cache_id, &message_id) {
            if let Some(size) = validated_cached_file_size(cache_dir, path).await {
                let mut tx = pool.begin().await.map_err(|err| err.to_string())?;
                reference.ensure_current(&mut tx).await?;
                return Ok(ReferencedAttachment {
                    result: PreparedAttachmentResult {
                        remote_uidvalidity: reference.uidvalidity(),
                        filename,
                        mime_type: mime,
                        size,
                        cached_path: path.to_string(),
                    },
                    reference,
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
        // part ID from the decrypted content names another part there. Older
        // decrypted rows still carry such an ID; it is never sent.
        return Err("backend.errors.decryptedAttachmentUnavailable".into());
    } else {
        let remote_part_id = remote_part_id
            .ok_or_else(|| "backend.errors.messageNotAvailableLocally".to_string())?;
        let config =
            crate::utils::oauth::refresh_account_config(pool, &account_id, &provider_kind, &config)
                .await
                .map_err(|err| err.encoded())?;
        let provider = provider_for_account(&provider_kind, &config, &account_email, None)?;
        provider
            .fetch_attachment(&reference.message_ref()?, &remote_part_id)
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
    // Each request owns its file. A late request must never overwrite a file
    // already published by a replacement row or another fetch.
    let path = attachment_cache_path(
        cache_dir,
        &cache_id,
        &format!("{}-{final_filename}", uuid::Uuid::new_v4()),
    );
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
    let mut tx = reference.begin_write(pool).await?;
    set_attachment_cache(&mut tx, &id, &message_id, &path_string, size).await?;
    sqlx::query(
        "UPDATE attachments
         SET filename = ?, content_type = ?
         WHERE id = ?",
    )
    .bind(&final_filename)
    .bind(&mime_type)
    .bind(&id)
    .execute(&mut *tx)
    .await
    .map_err(|err| err.to_string())?;

    tx.commit().await.map_err(|err| err.to_string())?;

    Ok(ReferencedAttachment {
        result: PreparedAttachmentResult {
            remote_uidvalidity: reference.uidvalidity(),
            filename: final_filename,
            mime_type,
            size,
            cached_path: path_string,
        },
        reference,
    })
}

pub(crate) async fn set_attachment_cache(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    id: &str,
    message_id: &str,
    path: &str,
    size: i64,
) -> Result<(), String> {
    let result = sqlx::query(
        "UPDATE attachments SET cached_path = ?, size = ? WHERE id = ? AND message_id = ?",
    )
    .bind(path)
    .bind(size)
    .bind(id)
    .bind(message_id)
    .execute(&mut **tx)
    .await
    .map_err(|err| err.to_string())?;
    if result.rows_affected() != 1 {
        return Err("backend.errors.messageNotAvailableLocally".into());
    }
    Ok(())
}

pub(crate) use crate::db::remote_content::cache_path_is_owned_by;

/// Only paths that resolve below Satchel's attachment cache may be handed to
/// Quick Look or another application. Canonicalization also rejects symlinks
/// that point outside the cache even if the database row was tampered with.
pub(super) async fn validated_cached_file_size(
    cache_dir: &std::path::Path,
    path: &str,
) -> Option<i64> {
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

pub(crate) use crate::db::remote_content::attachment_cache_path;
#[cfg(test)]
use crate::db::remote_content::{safe_filename, MAX_CACHE_NAME_BYTES};

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn attachment_download_revalidates_after_preparation() {
        use super::{
            download_prepared_attachment, prepare_referenced_attachment, DownloadAttachmentInput,
        };
        use crate::db::remote_content::ContentReference;
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::db::init(&pool).await.unwrap();
        sqlx::raw_sql("INSERT INTO accounts (id,display_name,email,provider,config,credentials_ref,created_at,updated_at)
            VALUES ('a','A','me@example.test','imap','{}','','','');
            INSERT INTO folders (id,account_id,name,full_path,remote_id,folder_type,uidvalidity,created_at,updated_at)
            VALUES ('f','a','Inbox','INBOX','INBOX','inbox',7,'','');
            INSERT INTO threads (id,account_id,subject,last_message_date,created_at,updated_at) VALUES ('t','a','Subject','','','');
            INSERT INTO messages (id,account_id,folder_id,thread_id,remote_id,remote_uidvalidity,from_address,to_addresses,date,created_at,updated_at)
            VALUES ('m','a','f','t','42',7,'sender@example.test','[]','','','');
            INSERT INTO attachments (id,message_id,filename,content_type,created_at) VALUES ('att','m','file.bin','application/octet-stream','');")
            .execute(&pool).await.unwrap();
        let dir = tempfile::tempdir().unwrap();
        for size in [4, super::MAX_BASE64_PAYLOAD_BYTES + 1] {
            for scenario in ["current", "reset", "replaced", "local-reset"] {
                sqlx::query("UPDATE folders SET uidvalidity = 7")
                    .execute(&pool)
                    .await
                    .unwrap();
                sqlx::query("UPDATE messages SET remote_id = ?, remote_uidvalidity = 7")
                    .bind((scenario != "local-reset").then_some("42"))
                    .execute(&pool)
                    .await
                    .unwrap();
                let reference = ContentReference::load(&pool, "m").await.unwrap();
                let path = super::attachment_cache_path(
                    dir.path(),
                    &reference.attachment_cache_id("att"),
                    "file.bin",
                );
                std::fs::write(&path, vec![b'x'; size]).unwrap();
                sqlx::query("UPDATE attachments SET cached_path = ?")
                    .bind(path.to_str().unwrap())
                    .execute(&pool)
                    .await
                    .unwrap();
                let prepared = prepare_referenced_attachment(
                    dir.path(),
                    &pool,
                    DownloadAttachmentInput {
                        attachment_id: "att".into(),
                    },
                )
                .await
                .unwrap();
                match scenario {
                    "reset" | "local-reset" => {
                        sqlx::query("UPDATE folders SET uidvalidity = 8")
                            .execute(&pool)
                            .await
                            .unwrap();
                    }
                    "replaced" => {
                        sqlx::query("UPDATE messages SET remote_id = '43'")
                            .execute(&pool)
                            .await
                            .unwrap();
                    }
                    _ => {}
                }
                let result = download_prepared_attachment(&pool, prepared).await;
                if matches!(scenario, "reset" | "replaced") {
                    assert!(
                        result.unwrap_err().contains("imapUidValidityChanged"),
                        "{scenario}, {size}"
                    );
                } else {
                    let result = result.unwrap();
                    assert_eq!(result.cached_path.as_deref(), path.to_str());
                    assert_eq!(
                        result.data_base64.is_some(),
                        size <= super::MAX_BASE64_PAYLOAD_BYTES
                    );
                    if size == 4 {
                        assert_eq!(result.data_base64.as_deref(), Some("eHh4eA=="));
                    }
                }
            }
        }
    }

    /// The parts a strict signed entity lists are served from the generation
    /// they were synced in, the signature part not at all.
    #[tokio::test]
    async fn an_attachment_under_a_signature_is_served_only_within_its_generation() {
        use super::{
            download_prepared_attachment, prepare_referenced_attachment, DownloadAttachmentInput,
        };
        use crate::db::remote_content::ContentReference;
        let parsed = crate::providers::imap::parse_raw_message_body(include_bytes!(
            "../../../crates/satchel-core/tests/corpus/pgp-mime/thunderbird-mixed-attachment.eml"
        ));
        assert!(parsed.trust.root_crypto && parsed.trust.signed_binding);
        let [figures] = parsed.attachments.as_slice() else {
            panic!(
                "expected only the signed attachment: {:?}",
                parsed.attachments
            );
        };
        assert_eq!(figures.filename.as_deref(), Some("figures.csv"));
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::db::init(&pool).await.unwrap();
        sqlx::raw_sql("INSERT INTO accounts (id,display_name,email,provider,config,credentials_ref,created_at,updated_at)
            VALUES ('a','A','me@example.test','imap','{}','','','');
            INSERT INTO folders (id,account_id,name,full_path,remote_id,folder_type,uidvalidity,created_at,updated_at)
            VALUES ('f','a','Inbox','INBOX','INBOX','inbox',7,'','');")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO messages (id,account_id,folder_id,remote_id,remote_uidvalidity,from_address,to_addresses,date,pgp_signed,trust_metadata,created_at,updated_at)
            VALUES ('m','a','f','42',7,'sender@example.test','[]','',?,?,'','')")
            .bind(parsed.pgp_signed.as_deref())
            .bind(serde_json::to_string(&parsed.trust).unwrap())
            .execute(&pool)
            .await
            .unwrap();
        // The part as sync lists it today, and the signature part as an older
        // sync stored it, under a name its unsigned headers chose.
        sqlx::query("INSERT INTO attachments (id,message_id,filename,content_type,remote_part_id,listing_rules,created_at)
            VALUES ('att', 'm', ?, ?, ?, 1, ''), ('phantom', 'm', 'Invoice.pdf', 'application/pgp-signature', '3', 0, '')")
            .bind(figures.filename.as_deref())
            .bind(&figures.content_type)
            .bind(&figures.remote_part_id)
            .execute(&pool)
            .await
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let reference = ContentReference::load(&pool, "m").await.unwrap();
        let path = super::attachment_cache_path(
            dir.path(),
            &reference.attachment_cache_id("att"),
            "figures.csv",
        );
        std::fs::write(&path, b"q1,q2").unwrap();
        sqlx::query("UPDATE attachments SET cached_path = ? WHERE id = 'att'")
            .bind(path.to_str().unwrap())
            .execute(&pool)
            .await
            .unwrap();
        let input = || DownloadAttachmentInput {
            attachment_id: "att".into(),
        };

        let listed = super::attachments_for_messages(&pool, &["m".into()], &Default::default())
            .await
            .unwrap();
        assert_eq!(listed.len(), 1, "the signature part is not listed");
        assert_eq!(listed[0].cached_path.as_deref(), path.to_str());
        let served = download_prepared_attachment(
            &pool,
            prepare_referenced_attachment(dir.path(), &pool, input())
                .await
                .unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(served.data_base64.as_deref(), Some("cTEscTI="));

        // A reset after preparation refuses the bytes already read.
        let prepared = prepare_referenced_attachment(dir.path(), &pool, input())
            .await
            .unwrap();
        sqlx::query("UPDATE folders SET uidvalidity = 8")
            .execute(&pool)
            .await
            .unwrap();
        assert!(download_prepared_attachment(&pool, prepared)
            .await
            .unwrap_err()
            .contains("imapUidValidityChanged"));
        assert!(prepare_referenced_attachment(dir.path(), &pool, input())
            .await
            .err()
            .unwrap()
            .contains("imapUidValidityChanged"));

        // Synced again in the new generation, the earlier file is no longer
        // this attachment's, and the signature part stays hidden.
        sqlx::query("UPDATE messages SET remote_uidvalidity = 8")
            .execute(&pool)
            .await
            .unwrap();
        let listed = super::attachments_for_messages(&pool, &["m".into()], &Default::default())
            .await
            .unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "att");
        assert_eq!(listed[0].cached_path, None);
    }

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
        attachment_schema(&pool).await;
        sqlx::query("INSERT INTO messages (id, account_id) VALUES ('m1', 'acc'), ('m2', 'acc'), ('m3', 'acc')")
            .execute(&pool)
            .await
            .unwrap();
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

        let rows = super::attachments_for_messages(
            &pool,
            &["m1".into(), "m2".into()],
            &Default::default(),
        )
        .await
        .expect("listing");

        let ids: Vec<&str> = rows.iter().map(|row| row.id.as_str()).collect();
        assert_eq!(ids, ["a2", "z9", "a1", "a3"]);
        assert_eq!(rows[0].cached_path.as_deref(), shared.to_str());
        assert_eq!(rows[3].filename.as_deref(), Some("c.txt"));
        assert_eq!(rows[2].cached_path, None, "a1 must not expose a2's bytes");
        sqlx::raw_sql(
            "UPDATE accounts SET provider = 'imap';
            UPDATE messages SET remote_id = '42', remote_uidvalidity = 7;",
        )
        .execute(&pool)
        .await
        .unwrap();
        let rows = super::attachments_for_messages(&pool, &["m1".into()], &Default::default())
            .await
            .unwrap();
        assert_eq!(
            rows[0].cached_path, None,
            "a legacy cache has no verified generation"
        );
        let verified =
            super::attachment_cache_path(std::path::Path::new("/cache"), "a2:imap:7", "a.png");
        sqlx::query("UPDATE attachments SET cached_path = ? WHERE id = 'a2'")
            .bind(verified.to_str().unwrap())
            .execute(&pool)
            .await
            .unwrap();
        let rows = super::attachments_for_messages(&pool, &["m1".into()], &Default::default())
            .await
            .unwrap();
        assert_eq!(rows[0].cached_path.as_deref(), verified.to_str());
        sqlx::query("UPDATE messages SET remote_uidvalidity = 8")
            .execute(&pool)
            .await
            .unwrap();
        let rows = super::attachments_for_messages(&pool, &["m1".into()], &Default::default())
            .await
            .unwrap();
        assert_eq!(
            rows[0].cached_path, None,
            "a reused UID cannot expose the previous generation's file"
        );

        assert!(
            super::attachments_for_messages(&pool, &[], &Default::default())
                .await
                .expect("empty")
                .is_empty()
        );
    }

    async fn attachment_schema(pool: &sqlx::SqlitePool) {
        for sql in [
            "CREATE TABLE attachments (id TEXT PRIMARY KEY, message_id TEXT NOT NULL,
             filename TEXT, content_type TEXT NOT NULL, size INTEGER, content_id TEXT,
             is_inline INTEGER NOT NULL DEFAULT 0, cached_path TEXT,
             listing_rules INTEGER NOT NULL DEFAULT 0)",
            "CREATE TABLE accounts (id TEXT PRIMARY KEY, provider TEXT)",
            "CREATE TABLE messages (id TEXT PRIMARY KEY, account_id TEXT, remote_id TEXT,
             remote_uidvalidity INTEGER, trust_metadata TEXT, pgp_signed BLOB, smime_signed BLOB)",
            "INSERT INTO accounts VALUES ('acc', 'gmail')",
        ] {
            sqlx::query(sql).execute(pool).await.expect("schema");
        }
    }

    const SIGNED_WITH_ATTACHMENT: &[u8] = include_bytes!(
        "../../../crates/satchel-core/tests/corpus/pgp-mime/thunderbird-mixed-attachment.eml"
    );

    /// An older sync could list parts from outside the signed entity next to
    /// a root signature: a signature part under a name its unsigned headers
    /// chose, or an outer part whose Content-ID fills the signed HTML. Those
    /// rows stay hidden, so no `cid:` resolves from them, until the repair has
    /// rebuilt them from the bundle. Rows written under the current rules stay
    /// listed, also once a refetched body has cleared the signed binding.
    #[tokio::test]
    async fn legacy_rows_beside_a_root_signature_stay_hidden_until_rebuilt() {
        let bundle = crate::providers::imap::parse_raw_message_body(SIGNED_WITH_ATTACHMENT)
            .pgp_signed
            .unwrap();
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::db::init(&pool).await.unwrap();
        sqlx::raw_sql("INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
                VALUES ('a', 'Account', 'me@example.test', 'gmail', '{}', '', '', '');
            INSERT INTO folders (id, account_id, name, full_path, folder_type, created_at, updated_at)
                VALUES ('f', 'a', 'Inbox', 'INBOX', 'inbox', '', '');")
            .execute(&pool)
            .await
            .unwrap();
        for (id, trust, signed) in [
            ("legacy", Some(r#"{"root_crypto":true}"#), true),
            (
                "refetched",
                Some(r#"{"root_crypto":true,"signed_binding":false}"#),
                true,
            ),
            ("nested", Some(r#"{"root_crypto":false}"#), true),
            ("plain", None, false),
        ] {
            sqlx::query("INSERT INTO messages (id, account_id, folder_id, from_address, to_addresses, date, trust_metadata, pgp_signed, created_at, updated_at)
                VALUES (?, 'a', 'f', 'sender@example.test', '[]', '', ?, ?, '', '')")
                .bind(id)
                .bind(trust)
                .bind(signed.then_some(bundle.as_slice()))
                .execute(&pool)
                .await
                .unwrap();
        }
        sqlx::raw_sql("INSERT INTO attachments (id, message_id, filename, content_type, content_id, is_inline, listing_rules, created_at) VALUES
                ('phantom', 'legacy', 'Invoice.pdf', 'application/pgp-signature', NULL, 0, 0, ''),
                ('outer-logo', 'legacy', 'logo.png', 'image/png', 'logo@example.test', 1, 0, ''),
                ('current', 'refetched', 'figures.csv', 'text/csv', NULL, 0, 1, ''),
                ('nested-old', 'nested', 'outer.pdf', 'application/pdf', NULL, 0, 0, ''),
                ('plain-old', 'plain', 'notes.txt', 'text/plain', NULL, 0, 0, '');")
            .execute(&pool)
            .await
            .unwrap();
        let ids = || async {
            super::attachments_for_messages(
                &pool,
                &[
                    "legacy".into(),
                    "refetched".into(),
                    "nested".into(),
                    "plain".into(),
                ],
                &Default::default(),
            )
            .await
            .unwrap()
            .into_iter()
            .map(|row| (row.message_id, row.filename.unwrap_or_default()))
            .collect::<Vec<_>>()
        };
        let pair = |message: &str, name: &str| (message.to_string(), name.to_string());
        assert_eq!(
            ids().await,
            [
                pair("nested", "outer.pdf"),
                pair("plain", "notes.txt"),
                pair("refetched", "figures.csv"),
            ]
        );

        let repair = satchel_core::db::signed_attachments::SIGNED_ENTITY_ATTACHMENTS;
        satchel_core::db::data_repairs::run_repair(&pool, &repair, 100, None, |_| {})
            .await
            .unwrap();
        assert_eq!(
            ids().await,
            [
                pair("legacy", "figures.csv"),
                pair("nested", "outer.pdf"),
                pair("plain", "notes.txt"),
                pair("refetched", "figures.csv"),
            ]
        );
        // The rebuilt row carries the bytes of the signed entity.
        let (payload, remote_part): (Option<String>, Option<String>) = sqlx::query_as(
            "SELECT content_base64, remote_part_id FROM attachments WHERE message_id = 'legacy'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let bytes =
            crate::sync::attachment_fetch::decode_attachment_base64(&payload.unwrap()).unwrap();
        assert!(String::from_utf8_lossy(&bytes).contains(','), "{bytes:?}");
        assert_eq!(remote_part, None);
        let current: String =
            sqlx::query_scalar("SELECT id FROM attachments WHERE message_id = 'refetched'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(current, "current");
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
        sqlx::query("CREATE TABLE attachments (id TEXT PRIMARY KEY, message_id TEXT, remote_uid TEXT, remote_part_id TEXT, cached_path TEXT, size INTEGER)")
            .execute(&pool).await.expect("schema");
        sqlx::query("INSERT INTO attachments (id, message_id, remote_uid, remote_part_id) VALUES ('inbox', 'message-inbox', '42', '1'), ('archive', 'message-archive', '42', '1')")
            .execute(&pool).await.expect("rows");

        let mut tx = pool.begin().await.unwrap();
        assert!(super::set_attachment_cache(
            &mut tx,
            "inbox",
            "replacement-message",
            "/cache/stale.pdf",
            9
        )
        .await
        .is_err());
        super::set_attachment_cache(&mut tx, "inbox", "message-inbox", "/cache/inbox.pdf", 4)
            .await
            .expect("cache inbox");
        tx.commit().await.unwrap();

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
             (id, account_id, name, full_path, remote_id, folder_type, uidvalidity, created_at,
              updated_at)
             VALUES ('fld', 'acc', 'INBOX', 'INBOX', 'INBOX', 'inbox', 7, ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("folder");
        sqlx::query(
            "INSERT INTO messages
             (id, account_id, folder_id, from_address, to_addresses, date, is_draft, remote_id,
              remote_uidvalidity, created_at, updated_at)
             VALUES ('msg', 'acc', 'fld', 'a@b.c', '[]', ?, 0, '12', 7, ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("message");

        let cache = tempfile::tempdir().expect("cache dir");
        let id = satchel_core::email::decrypted::attachment_id("msg", "mime-1");
        let reference = crate::db::remote_content::ContentReference::load(&pool, "msg")
            .await
            .expect("reference");
        let cached = attachment_cache_path(
            cache.path(),
            &reference.attachment_cache_id(&id),
            "agenda.pdf",
        );
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
            super::prepare_attachment_in_dir(
                cache.path(),
                &pool,
                super::DownloadAttachmentInput {
                    attachment_id: id.clone(),
                },
            )
        };
        let served = prepare().await;
        assert!(served.is_ok(), "the cached copy is served: {served:?}");

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
