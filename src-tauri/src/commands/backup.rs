use base64::Engine as _;
use serde::Serialize;
use sqlx::Row;
use sqlx::SqlitePool;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{Emitter, Manager};

use crate::commands::error::{CommandError, CommandResult};

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/backup/"))]
pub struct ExportResult {
    pub path: String,
    pub message_count: usize,
    pub bytes_written: u64,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/backup/"))]
pub struct ImportResult {
    pub path: String,
    pub message_count: usize,
    pub skipped: usize,
    /// Set when the user stopped the run; the messages counted above are in.
    pub cancelled: bool,
}

const IMPORT_PROGRESS_EVENT: &str = "import:progress";

/// Messages between two progress events. Per message would flood the bridge on
/// a multi-gigabyte archive; a hundred still moves the bar smoothly.
const IMPORT_PROGRESS_EVERY: usize = 100;

/// How far an import has got. Shared by both importers so the view has one
/// progress shape to render.
#[derive(Debug, Clone, Default, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/backup/"))]
pub struct ImportProgress {
    /// `mbox` or `platform`.
    pub kind: String,
    pub done: i64,
    pub skipped: i64,
    /// Units expected in total: tables for a platform import, and for an mbox
    /// an estimate from the bytes read per message so far. Zero until the
    /// first messages have been read.
    pub total: i64,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub running: bool,
    pub eta_seconds: Option<i64>,
    /// `completed`, `cancelled` or `error`, once the run has ended.
    pub finished_reason: Option<String>,
    pub last_error: Option<String>,
}

/// A cancel request outlives the command that set it, so it is reset when the
/// next import starts rather than when this one ends.
static IMPORT_CANCELLED: AtomicBool = AtomicBool::new(false);

/// Stop the running import after the message it is on.
///
/// Nothing half-written survives: every message is its own transaction, and
/// the platform import rolls its single transaction back.
#[tauri::command]
pub fn cancel_import() {
    IMPORT_CANCELLED.store(true, Ordering::SeqCst);
}

/// Tell the window how far the import has got, if there is a window. A caller
/// without an app handle — a test, a headless path — simply has no one to tell.
fn emit_import_progress(app: Option<&tauri::AppHandle>, progress: &ImportProgress) {
    if let Some(app) = app {
        let _ = app.emit(IMPORT_PROGRESS_EVENT, progress);
    }
}

/// Seconds left, extrapolated from the bytes read so far. `None` before there
/// is enough to extrapolate from.
fn import_eta_seconds(elapsed: std::time::Duration, done: u64, total: u64) -> Option<i64> {
    if done == 0 || total <= done {
        return None;
    }
    let rate = done as f64 / elapsed.as_secs_f64().max(0.001);
    if rate <= 0.0 {
        return None;
    }
    Some(((total - done) as f64 / rate).round() as i64)
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/backup/"))]
pub struct PlatformDataResult {
    pub path: String,
    pub table_count: usize,
    pub row_count: usize,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
struct PlatformDataExport {
    schema_version: u32,
    exported_at: String,
    redacted_sensitive: bool,
    tables: BTreeMap<String, Vec<BTreeMap<String, Option<String>>>>,
}

/// Stands in for a redacted value on the way out and is recognised again on the
/// way in. A plain marker rather than an empty string: the import has to tell
/// "the exporter withheld this" from "the value really is empty".
const REDACTED_MARKER: &str = "redacted";

struct PortableTable {
    name: &'static str,
    /// Conflict target of the import upsert. Must match the table's primary key.
    primary_key: &'static [&'static str],
    redacted_columns: &'static [&'static str],
}

/// A table without a conflict target would have to fall back to a replacing
/// insert, which is exactly what the import must never do.
const _: () = {
    let mut index = 0;
    while index < PORTABLE_TABLES.len() {
        assert!(
            !PORTABLE_TABLES[index].primary_key.is_empty(),
            "every portable table needs a primary key"
        );
        index += 1;
    }
};

/// The columns are not listed here: they are read from the schema at run time
/// (see `portable_columns`). A hand-kept list drifts away with every migration
/// that adds a column, and the loss is silent — the export simply stops
/// carrying it.
const PORTABLE_TABLES: &[PortableTable] = &[
    PortableTable {
        name: "accounts",
        primary_key: &["id"],
        // `config` holds the account secrets: IMAP/SMTP passwords and OAuth
        // refresh tokens, encrypted or — after a failed encryption — not.
        redacted_columns: &["config", "credentials_ref"],
    },
    PortableTable {
        name: "folders",
        primary_key: &["id"],
        redacted_columns: &[],
    },
    PortableTable {
        name: "account_aliases",
        primary_key: &["id"],
        redacted_columns: &[],
    },
    PortableTable {
        name: "signatures",
        primary_key: &["id"],
        redacted_columns: &[],
    },
    PortableTable {
        name: "templates",
        primary_key: &["id"],
        redacted_columns: &[],
    },
    PortableTable {
        name: "rules",
        primary_key: &["id"],
        redacted_columns: &[],
    },
    PortableTable {
        name: "blocked_senders",
        primary_key: &["id"],
        redacted_columns: &[],
    },
    PortableTable {
        name: "smart_folders",
        primary_key: &["id"],
        redacted_columns: &[],
    },
    PortableTable {
        name: "vacation_settings",
        primary_key: &["account_id"],
        redacted_columns: &[],
    },
    PortableTable {
        name: "calendar_events",
        primary_key: &["id"],
        redacted_columns: &[],
    },
    PortableTable {
        name: "provider_labels",
        primary_key: &["id"],
        redacted_columns: &[],
    },
    PortableTable {
        name: "contacts",
        primary_key: &["id"],
        redacted_columns: &[],
    },
    PortableTable {
        name: "plugin_state",
        primary_key: &["plugin_id", "key"],
        redacted_columns: &[],
    },
    PortableTable {
        name: "plugin_permissions",
        primary_key: &["plugin_id", "capability"],
        redacted_columns: &[],
    },
    PortableTable {
        name: "local_webhooks",
        primary_key: &["id"],
        redacted_columns: &["secret"],
    },
];

/// Column names of one portable table, in schema order.
///
/// `pragma_table_info` leaves out generated and hidden columns, which is what
/// the insert wants anyway.
async fn portable_columns<'e, E>(executor: E, table: &str) -> Result<Vec<String>, String>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    let columns: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_table_info(?)")
        .bind(table)
        .fetch_all(executor)
        .await
        .map_err(|err| format!("columns of {table}: {err}"))?;
    if columns.is_empty() {
        return Err(format!("portable table {table} is not in the schema"));
    }
    Ok(columns)
}

/// Message rows one export page holds.
///
/// A page is what the exporter has in memory at once, bodies included. The old
/// `fetch_all` over the whole table meant a 20-GB mailbox could not be
/// exported at all.
const EXPORT_PAGE_ROWS: i64 = 500;

/// What the mbox writer needs of a message. The database keeps parsed bodies
/// rather than complete source MIME, so complex messages are rebuilt below.
#[derive(sqlx::FromRow)]
struct MboxExportRow {
    id: String,
    date: String,
    from_address: String,
    from_name: String,
    to_addresses: Option<String>,
    cc_addresses: Option<String>,
    bcc_addresses: Option<String>,
    reply_to: Option<String>,
    subject: Option<String>,
    body_text: Option<String>,
    body_html: Option<String>,
    message_id_header: Option<String>,
    in_reply_to: Option<String>,
    references_header: Option<String>,
    draft_attachments_json: Option<String>,
    has_attachments: i64,
}

// Count export pages within the test task without observing parallel exports.
#[cfg(test)]
tokio::task_local! {
    static EXPORT_PAGE_QUERIES: std::cell::Cell<usize>;
}

/// One page of messages after `cursor`, in mbox order.
async fn fetch_export_page(
    pool: &SqlitePool,
    account_id: Option<&str>,
    cursor: Option<&(String, String)>,
    limit: i64,
) -> Result<Vec<MboxExportRow>, String> {
    #[cfg(test)]
    let _ = EXPORT_PAGE_QUERIES.try_with(|count| count.set(count.get() + 1));

    let mut query = sqlx::QueryBuilder::new(
        "SELECT id, date, from_address, COALESCE(from_name, '') AS from_name, to_addresses,
                cc_addresses, bcc_addresses, reply_to, subject, body_text, body_html,
                message_id_header, in_reply_to, references_header, draft_attachments_json,
                has_attachments
         FROM messages WHERE 1 = 1",
    );
    if let Some(account) = account_id {
        query.push(" AND account_id = ");
        query.push_bind(account);
    }
    // Keyset rather than OFFSET: a message inserted mid-export must not shift
    // the window and drop a row.
    if let Some((date, id)) = cursor {
        query.push(" AND (date > ");
        query.push_bind(date.clone());
        query.push(" OR (date = ");
        query.push_bind(date.clone());
        query.push(" AND id > ");
        query.push_bind(id.clone());
        query.push("))");
    }
    query.push(" ORDER BY date ASC, id ASC LIMIT ");
    query.push_bind(limit);

    query
        .build_query_as::<MboxExportRow>()
        .fetch_all(pool)
        .await
        .map_err(|err| err.to_string())
}

#[derive(sqlx::FromRow)]
struct MboxAttachmentRow {
    id: String,
    filename: Option<String>,
    content_type: String,
    content_id: Option<String>,
    is_inline: i64,
    path: Option<String>,
}

async fn export_attachments(
    pool: &SqlitePool,
    row: &MboxExportRow,
) -> Result<Vec<crate::providers::traits::OutgoingAttachment>, String> {
    use crate::providers::traits::OutgoingAttachment;

    if let Some(raw) = row
        .draft_attachments_json
        .as_deref()
        .filter(|raw| !raw.trim().is_empty())
    {
        let draft: Vec<crate::commands::messages::OutgoingAttachmentInput> =
            serde_json::from_str(raw).map_err(|_| {
                CommandError::new("backend.errors.exportAttachmentMetadataInvalid").encoded()
            })?;
        if !draft.is_empty() {
            return Ok(draft
                .into_iter()
                .map(|attachment| OutgoingAttachment {
                    filename: attachment.filename,
                    mime_type: attachment.mime_type,
                    content_base64: attachment.content_base64,
                    content_id: attachment.content_id,
                    inline: attachment.inline,
                })
                .collect());
        }
    }

    let rows: Vec<MboxAttachmentRow> = sqlx::query_as(
        "SELECT id, filename, content_type, content_id, is_inline,
                COALESCE(cached_path, storage_path) AS path
         FROM attachments WHERE message_id = ? ORDER BY id",
    )
    .bind(&row.id)
    .fetch_all(pool)
    .await
    .map_err(|err| err.to_string())?;
    if rows.is_empty() && row.has_attachments != 0 {
        return Err(CommandError::new("backend.errors.exportAttachmentListUnavailable").encoded());
    }
    let mut attachments = Vec::with_capacity(rows.len());
    for attachment in rows {
        let filename = attachment
            .filename
            .unwrap_or_else(|| "attachment".to_string());
        let path = attachment
            .path
            .ok_or_else(|| export_attachment_error(&filename))?;
        if !crate::commands::attachments::cache_path_is_owned_by(&path, &attachment.id, &row.id) {
            return Err(export_attachment_error(&filename));
        }
        let bytes = tokio::fs::read(&path)
            .await
            .map_err(|_| export_attachment_error(&filename))?;
        attachments.push(OutgoingAttachment {
            filename,
            mime_type: attachment.content_type,
            content_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
            content_id: attachment.content_id,
            inline: attachment.is_inline != 0,
        });
    }
    Ok(attachments)
}

fn export_attachment_error(filename: &str) -> String {
    CommandError::new("backend.errors.exportAttachmentUnavailable")
        .with_param("filename", filename)
        .encoded()
}

fn build_mime_mbox_entry(
    row: &MboxExportRow,
    attachments: Vec<crate::providers::traits::OutgoingAttachment>,
) -> Result<(String, String), String> {
    use crate::providers::traits::OutgoingMessage;
    let addresses = |raw: Option<&str>| -> Vec<String> {
        raw.and_then(|value| serde_json::from_str(value).ok())
            .unwrap_or_default()
    };
    let message = OutgoingMessage {
        from_address: Some(row.from_address.clone()),
        from_name: (!row.from_name.is_empty()).then(|| row.from_name.clone()),
        reply_to: row.reply_to.clone(),
        to: addresses(row.to_addresses.as_deref()),
        cc: addresses(row.cc_addresses.as_deref()),
        bcc: addresses(row.bcc_addresses.as_deref()),
        subject: row.subject.clone().unwrap_or_default(),
        body: row.body_text.clone().unwrap_or_default(),
        body_html: row.body_html.clone(),
        in_reply_to: row.in_reply_to.clone(),
        references: row
            .references_header
            .as_deref()
            .unwrap_or_default()
            .split_whitespace()
            .map(str::to_string)
            .collect(),
        attachments,
        message_id_header: row.message_id_header.clone(),
        ..Default::default()
    };
    let raw = crate::email::build::build_rfc822_with_bcc(&message, None, None)
        .map_err(|err| err.encoded())?;
    let (headers, body) = raw
        .split_once("\r\n\r\n")
        .ok_or_else(|| format!("message {} MIME headers are incomplete", row.id))?;
    let date = chrono::DateTime::parse_from_rfc3339(&row.date)
        .map(|date| date.to_rfc2822())
        .unwrap_or_else(|_| chrono::Utc::now().to_rfc2822());
    let header = headers
        .split("\r\n")
        .map(|line| {
            if line.starts_with("Date: ") {
                format!("Date: {date}")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n\n";
    Ok((header, body.replace("\r\n", "\n")))
}

/// Stream locally cached messages into an mbox file at the given path. A
/// missing attachment aborts the export, leaving the destination untouched.
/// Mbox-O format (with `>From ` line escaping and a `From <sender> <date>` line
/// per message). Compatible with Apple Mail, Thunderbird, mutt, mu, etc.
///
/// The order is `date ASC` as before, with `id` as the tiebreak the cursor
/// needs; for distinct dates the output is unchanged.
async fn write_mailbox_export(
    pool: &SqlitePool,
    path: &str,
    account_id: Option<&str>,
) -> Result<ExportResult, String> {
    let pending = format!("{path}.satchel-partial-{}", uuid::Uuid::new_v4());
    let result = write_mailbox_export_to_path(pool, &pending, account_id).await;
    let mut result = match result {
        Ok(result) => result,
        Err(error) => {
            let _ = tokio::fs::remove_file(&pending).await;
            return Err(error);
        }
    };
    if let Err(error) = tokio::fs::rename(&pending, path).await {
        let _ = tokio::fs::remove_file(&pending).await;
        return Err(format!("finish {path}: {error}"));
    }
    result.path = path.to_string();
    Ok(result)
}

async fn write_mailbox_export_to_path(
    pool: &SqlitePool,
    path: &str,
    account_id: Option<&str>,
) -> Result<ExportResult, String> {
    use tokio::io::AsyncWriteExt;

    let file = tokio::fs::File::create(path)
        .await
        .map_err(|err| format!("create {path}: {err}"))?;
    let mut out = tokio::io::BufWriter::new(file);

    let mut count = 0usize;
    let mut bytes = 0u64;
    let mut cursor: Option<(String, String)> = None;
    loop {
        let rows = fetch_export_page(pool, account_id, cursor.as_ref(), EXPORT_PAGE_ROWS).await?;
        if rows.is_empty() {
            break;
        }
        let last = rows.len() < EXPORT_PAGE_ROWS as usize;
        for row in &rows {
            let from_line = format!(
                "From {} {}\n",
                row.from_address,
                reformat_date_for_mbox(&row.date)
            );
            out.write_all(from_line.as_bytes())
                .await
                .map_err(|err| err.to_string())?;
            bytes += from_line.len() as u64;

            let attachments = export_attachments(pool, row).await?;
            let (header, body) =
                if attachments.is_empty() && row.body_html.as_deref().is_none_or(str::is_empty) {
                    (
                        build_mbox_headers(
                            &row.from_address,
                            &row.from_name,
                            row.to_addresses.as_deref(),
                            row.cc_addresses.as_deref(),
                            row.subject.as_deref(),
                            row.message_id_header.as_deref(),
                            row.in_reply_to.as_deref(),
                            row.references_header.as_deref(),
                            &row.date,
                        ),
                        row.body_text.clone().unwrap_or_default(),
                    )
                } else {
                    build_mime_mbox_entry(row, attachments)?
                };
            out.write_all(header.as_bytes())
                .await
                .map_err(|err| err.to_string())?;
            bytes += header.len() as u64;

            let escaped = escape_mbox_body(&body);
            out.write_all(escaped.as_bytes())
                .await
                .map_err(|err| err.to_string())?;
            bytes += escaped.len() as u64;
            out.write_all(b"\n\n")
                .await
                .map_err(|err| err.to_string())?;
            bytes += 2;
            count += 1;
        }
        if last {
            break;
        }
        let tail = rows.last().expect("a non-empty page has a last row");
        cursor = Some((tail.date.clone(), tail.id.clone()));
    }

    out.flush().await.map_err(|err| err.to_string())?;

    Ok(ExportResult {
        path: path.to_string(),
        message_count: count,
        bytes_written: bytes,
    })
}

#[tauri::command]
pub async fn export_mailbox(
    state: tauri::State<'_, crate::AppState>,
    pool: tauri::State<'_, SqlitePool>,
    path: String,
    account_id: Option<String>,
) -> CommandResult<ExportResult> {
    state.write_grants.take(std::path::Path::new(&path))?;
    write_mailbox_export(pool.inner(), &path, account_id.as_deref())
        .await
        .map_err(CommandError::from)
}

#[tauri::command]
pub async fn export_platform_data(
    state: tauri::State<'_, crate::AppState>,
    pool: tauri::State<'_, SqlitePool>,
    path: String,
    redact_sensitive: Option<bool>,
) -> CommandResult<PlatformDataResult> {
    state.write_grants.take(std::path::Path::new(&path))?;
    let (export, row_count) =
        collect_platform_export(pool.inner(), redact_sensitive.unwrap_or(true)).await?;
    let json = serde_json::to_string_pretty(&export).map_err(|err| err.to_string())?;
    tokio::fs::write(&path, json)
        .await
        .map_err(|err| format!("write {path}: {err}"))?;

    Ok(PlatformDataResult {
        path,
        table_count: PORTABLE_TABLES.len(),
        row_count,
    })
}

#[tauri::command]
pub async fn import_platform_data(
    app: tauri::AppHandle,
    pool: tauri::State<'_, SqlitePool>,
    path: String,
) -> CommandResult<PlatformDataResult> {
    crate::commands::file_limits::guard_import_size(std::path::Path::new(&path)).await?;
    let raw = tokio::fs::read_to_string(&path)
        .await
        .map_err(|err| format!("read {path}: {err}"))?;
    let parsed: PlatformDataExport =
        serde_json::from_str(&raw).map_err(|err| format!("parse platform export: {err}"))?;
    if parsed.schema_version != 1 {
        return Err(format!(
            "unsupported platform export schema version {}",
            parsed.schema_version
        )
        .into());
    }

    IMPORT_CANCELLED.store(false, Ordering::SeqCst);
    let row_count = apply_platform_export(Some(&app), pool.inner(), &parsed).await?;
    satchel_core::db::maintenance::request_fts_check(pool.inner())
        .await
        .map_err(|err| err.to_string())?;

    Ok(PlatformDataResult {
        path,
        table_count: parsed.tables.len(),
        row_count,
    })
}

/// Read every portable table and report the rows collected.
async fn collect_platform_export(
    pool: &SqlitePool,
    redact_sensitive: bool,
) -> Result<(PlatformDataExport, usize), String> {
    let mut tables = BTreeMap::new();
    let mut row_count = 0usize;
    let mut disclosed = 0usize;
    for table in PORTABLE_TABLES {
        let (rows, table_disclosed) = export_portable_table(pool, table, redact_sensitive).await?;
        row_count += rows.len();
        disclosed += table_disclosed;
        tables.insert(table.name.to_string(), rows);
    }

    Ok((
        PlatformDataExport {
            schema_version: 1,
            exported_at: chrono::Utc::now().to_rfc3339(),
            // Read off the file, not off the request: the promise is about what
            // the file contains, and a file that carries one secret is not a
            // redacted file no matter what was asked for.
            redacted_sensitive: disclosed == 0,
            tables,
        },
        row_count,
    ))
}

/// Write every portable table in a single transaction and report how many rows
/// were inserted or updated.
///
/// One transaction, so cancelling between tables rolls the whole thing back —
/// a partly imported set of settings would be worse than none.
async fn apply_platform_export(
    app: Option<&tauri::AppHandle>,
    pool: &SqlitePool,
    parsed: &PlatformDataExport,
) -> Result<usize, String> {
    let mut progress = ImportProgress {
        kind: "platform".into(),
        total: PORTABLE_TABLES.len() as i64,
        running: true,
        ..ImportProgress::default()
    };
    emit_import_progress(app, &progress);

    let mut tx = pool.begin().await.map_err(|err| err.to_string())?;
    let mut row_count = 0usize;
    for table in PORTABLE_TABLES {
        if IMPORT_CANCELLED.swap(false, Ordering::SeqCst) {
            drop(tx);
            progress.running = false;
            progress.finished_reason = Some("cancelled".into());
            emit_import_progress(app, &progress);
            return Err("backend.errors.importCancelled".to_string());
        }
        progress.done += 1;
        emit_import_progress(app, &progress);
        let Some(rows) = parsed.tables.get(table.name) else {
            continue;
        };
        let columns = portable_columns(&mut *tx, table.name).await?;
        for row in rows {
            row_count += import_portable_row(&mut tx, table, &columns, row).await? as usize;
        }
    }
    tx.commit().await.map_err(|err| err.to_string())?;

    progress.running = false;
    progress.finished_reason = Some("completed".into());
    emit_import_progress(app, &progress);
    Ok(row_count)
}

/// Reads one table and reports how many secret values it let through
/// unredacted.
async fn export_portable_table(
    pool: &SqlitePool,
    table: &PortableTable,
    redact_sensitive: bool,
) -> Result<(Vec<BTreeMap<String, Option<String>>>, usize), String> {
    let columns = portable_columns(pool, table.name).await?;
    let selections = columns
        .iter()
        .map(|column| format!("CAST(\"{column}\" AS TEXT) AS \"{column}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!("SELECT {selections} FROM {}", table.name);
    let rows = sqlx::query(&sql)
        .fetch_all(pool)
        .await
        .map_err(|err| err.to_string())?;
    let mut out = Vec::with_capacity(rows.len());
    let mut disclosed = 0usize;
    for row in rows {
        let mut item = BTreeMap::new();
        for column in &columns {
            let secret = table.redacted_columns.contains(&column.as_str());
            let value = if secret && redact_sensitive {
                Some(REDACTED_MARKER.to_string())
            } else {
                let stored = row
                    .try_get::<Option<String>, _>(column.as_str())
                    .map_err(|err| err.to_string())?;
                if secret && stored.is_some() {
                    disclosed += 1;
                }
                stored
            };
            item.insert(column.clone(), value);
        }
        out.push(item);
    }
    Ok((out, disclosed))
}

async fn import_portable_row(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    table: &PortableTable,
    columns: &[String],
    row: &BTreeMap<String, Option<String>>,
) -> Result<u64, String> {
    // Only what the file and the current schema agree on. Columns an older
    // export does not carry stay out of the statement: the upsert then leaves
    // the local value alone instead of overwriting it with a default. Columns a
    // newer export carries and this schema does not are ignored.
    let present = columns
        .iter()
        .filter(|column| row.contains_key(column.as_str()))
        .map(|column| column.as_str())
        .collect::<Vec<_>>();
    for key in table.primary_key {
        if !present.contains(key) {
            return Err(format!("import {}: row without {key}", table.name));
        }
    }

    // A redacted export carries a marker where the secret was. Writing it back
    // would replace a working credential with the word "redacted", so those
    // columns are left out of the update and keep whatever is here already.
    let withheld = table
        .redacted_columns
        .iter()
        .filter(|column| {
            row.get(**column).and_then(|value| value.as_deref()) == Some(REDACTED_MARKER)
        })
        .copied()
        .collect::<Vec<_>>();
    let placeholders = std::iter::repeat_n("?", present.len())
        .collect::<Vec<_>>()
        .join(", ");
    let column_list = present.join(", ");
    let conflict_target = table.primary_key.join(", ");
    let assignments = present
        .iter()
        .filter(|column| !table.primary_key.contains(*column))
        .filter(|column| !withheld.contains(*column))
        .map(|column| format!("{column} = excluded.{column}"))
        .collect::<Vec<_>>()
        .join(", ");
    // Never `INSERT OR REPLACE`: with foreign_keys=ON that deletes the
    // conflicting row and fires every ON DELETE CASCADE hanging off it, wiping
    // child tables the import does not carry. The upsert keeps the row alive.
    let conflict_action = if assignments.is_empty() {
        "DO NOTHING".to_string()
    } else {
        format!("DO UPDATE SET {assignments}")
    };
    let sql = format!(
        "INSERT INTO {} ({column_list}) VALUES ({placeholders}) ON CONFLICT({conflict_target}) {conflict_action}",
        table.name
    );
    let mut query = sqlx::query(&sql);
    for column in &present {
        // Rows that do not exist locally have nothing to keep. They get an
        // empty value rather than the marker: the account is visibly
        // unconfigured instead of carrying a password-shaped string.
        let value = if withheld.contains(column) {
            Some("")
        } else {
            row.get(*column).and_then(|value| value.as_deref())
        };
        query = query.bind(value);
    }
    let result = query
        .execute(&mut **tx)
        .await
        .map_err(|err| format!("import {}: {err}", table.name))?;
    Ok(result.rows_affected())
}

fn reformat_date_for_mbox(rfc3339: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(rfc3339)
        .ok()
        .map(|dt| dt.format("%a %b %e %H:%M:%S %Y").to_string())
        .unwrap_or_else(|| {
            chrono::Utc::now()
                .format("%a %b %e %H:%M:%S %Y")
                .to_string()
        })
}

/// One header line for the mbox writer: no injection, folded at 78 octets.
///
/// Every value here comes from a stored message, and a stored message comes
/// from the network. A `Subject` holding a newline used to end the header and
/// begin whatever the sender wrote next — `Bcc:`, a second `From:`, a body.
/// CR and LF are folded away to a single space; the rest is folded at
/// whitespace per RFC 5322 §2.2.3, with LF continuations because this file uses
/// LF throughout.
fn mbox_header(name: &str, value: &str) -> String {
    let flattened = value
        .split(['\r', '\n'])
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");

    let line = format!("{name}: {flattened}");
    if line.len() <= 78 {
        return format!("{line}\n");
    }

    let mut out = String::with_capacity(line.len() + line.len() / 78 * 2);
    let mut used = 0usize;
    for (index, word) in line.split(' ').enumerate() {
        if index > 0 {
            if used + 1 + word.len() > 78 {
                out.push_str("\n ");
                used = 1;
            } else {
                out.push(' ');
                used += 1;
            }
        }
        out.push_str(word);
        used += word.len();
    }
    out.push('\n');
    out
}

fn build_mbox_headers(
    from: &str,
    from_name: &str,
    to: Option<&str>,
    cc: Option<&str>,
    subject: Option<&str>,
    msg_id: Option<&str>,
    in_reply_to: Option<&str>,
    references: Option<&str>,
    date: &str,
) -> String {
    let mut s = String::new();
    let from_line = if from_name.is_empty() {
        mbox_header("From", from)
    } else {
        mbox_header("From", &format!("\"{from_name}\" <{from}>"))
    };
    s.push_str(&from_line);
    if let Some(to) = to.and_then(|raw| serde_json::from_str::<Vec<String>>(raw).ok()) {
        if !to.is_empty() {
            s.push_str(&mbox_header("To", &to.join(", ")));
        }
    }
    if let Some(cc) = cc.and_then(|raw| serde_json::from_str::<Vec<String>>(raw).ok()) {
        if !cc.is_empty() {
            s.push_str(&mbox_header("Cc", &cc.join(", ")));
        }
    }
    if let Some(subject) = subject {
        s.push_str(&mbox_header("Subject", subject));
    }
    if let Some(msg_id) = msg_id {
        s.push_str(&mbox_header("Message-ID", msg_id));
    }
    if let Some(reply) = in_reply_to {
        if !reply.is_empty() {
            s.push_str(&mbox_header("In-Reply-To", reply));
        }
    }
    if let Some(references) = references {
        if !references.is_empty() {
            s.push_str(&mbox_header("References", references));
        }
    }
    let formatted_date = chrono::DateTime::parse_from_rfc3339(date)
        .map(|dt| dt.to_rfc2822())
        .unwrap_or_else(|_| chrono::Utc::now().to_rfc2822());
    s.push_str(&format!("Date: {formatted_date}\n"));
    s.push_str("MIME-Version: 1.0\n");
    s.push_str("Content-Type: text/plain; charset=UTF-8\n\n");
    s
}

fn escape_mbox_body(body: &str) -> String {
    body.lines()
        .map(|line| {
            if line.starts_with("From ") {
                format!(">{line}")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

use crate::email::mbox::{strip_mbox_from_line, MboxEntries};

#[tauri::command]
pub async fn import_mbox(
    app: tauri::AppHandle,
    pool: tauri::State<'_, SqlitePool>,
    path: String,
    account_id: String,
) -> CommandResult<ImportResult> {
    let account = sqlx::query_as::<_, (String,)>("SELECT email FROM accounts WHERE id = ? LIMIT 1")
        .bind(&account_id)
        .fetch_one(pool.inner())
        .await
        .map_err(|err| err.to_string())?;
    let folder_id = ensure_import_folder(pool.inner(), &account_id).await?;

    if is_msg_path(&path) {
        // One file, one message — here the file ceiling is the message ceiling.
        crate::commands::file_limits::guard_import_size(std::path::Path::new(&path)).await?;
        let raw_bytes = tokio::fs::read(&path)
            .await
            .map_err(|err| format!("read {path}: {err}"))?;
        let parsed = parse_msg_bytes(&raw_bytes)?;
        let imported = import_parsed_message(
            &app,
            pool.inner(),
            &account_id,
            &account.0,
            &folder_id,
            parsed,
            &raw_bytes,
        )
        .await?;
        satchel_core::db::maintenance::request_fts_check(pool.inner())
            .await
            .map_err(|err| err.to_string())?;
        return Ok(ImportResult {
            path,
            message_count: usize::from(imported),
            skipped: usize::from(!imported),
            cancelled: false,
        });
    }

    let bytes_total = tokio::fs::metadata(&path)
        .await
        .map(|meta| meta.len())
        .unwrap_or(0);
    let file = tokio::fs::File::open(&path)
        .await
        .map_err(|err| format!("read {path}: {err}"))?;
    let mut entries = MboxEntries::new(tokio::io::BufReader::new(file));

    IMPORT_CANCELLED.store(false, Ordering::SeqCst);
    let mut progress = ImportProgress {
        kind: "mbox".into(),
        bytes_total,
        running: true,
        ..ImportProgress::default()
    };
    emit_import_progress(Some(&app), &progress);

    let started = std::time::Instant::now();
    let mut count = 0usize;
    let mut skipped = 0usize;
    let mut cancelled = false;
    while let Some(entry) = entries.next_entry().await? {
        match import_one_message(
            &app,
            pool.inner(),
            &account_id,
            &account.0,
            &folder_id,
            &entry,
        )
        .await
        {
            Ok(true) => count += 1,
            Ok(false) => skipped += 1,
            Err(err) => {
                skipped += 1;
                progress.last_error = Some(err);
            }
        }
        // Between messages, never inside one: a stopped import must not leave
        // a half-written thread behind.
        if IMPORT_CANCELLED.swap(false, Ordering::SeqCst) {
            cancelled = true;
            break;
        }
        if (count + skipped).is_multiple_of(IMPORT_PROGRESS_EVERY) {
            progress.done = count as i64;
            progress.skipped = (skipped + entries.dropped) as i64;
            progress.bytes_done = entries.bytes_read;
            // The file size is known, the message count is not: scale what has
            // been handled by how much of the file it took.
            progress.total = if entries.bytes_read > 0 {
                ((count + skipped) as f64 * bytes_total as f64 / entries.bytes_read as f64).round()
                    as i64
            } else {
                0
            };
            progress.eta_seconds =
                import_eta_seconds(started.elapsed(), entries.bytes_read, bytes_total);
            emit_import_progress(Some(&app), &progress);
        }
    }

    let skipped = skipped + entries.dropped;
    progress.done = count as i64;
    progress.skipped = skipped as i64;
    progress.bytes_done = entries.bytes_read;
    progress.total = (count + skipped) as i64;
    progress.eta_seconds = None;
    progress.running = false;
    progress.finished_reason = Some(if cancelled { "cancelled" } else { "completed" }.into());
    emit_import_progress(Some(&app), &progress);

    satchel_core::db::maintenance::request_fts_check(pool.inner())
        .await
        .map_err(|err| err.to_string())?;
    Ok(ImportResult {
        path,
        message_count: count,
        skipped,
        cancelled,
    })
}

async fn ensure_import_folder(pool: &SqlitePool, account_id: &str) -> Result<String, String> {
    if let Some(row) = sqlx::query_as::<_, (String,)>(
        "SELECT id FROM folders WHERE account_id = ? AND folder_type = 'archive' ORDER BY created_at ASC LIMIT 1",
    )
    .bind(account_id)
    .fetch_optional(pool)
    .await
    .map_err(|err| err.to_string())?
    {
        return Ok(row.0);
    }
    let now = chrono::Utc::now().to_rfc3339();
    let id = satchel_core::db::folders::resolve_provider_folder_id(pool, account_id, "Imported")
        .await
        .map_err(|err| err.to_string())?;
    sqlx::query(
        "INSERT INTO folders (id, account_id, name, full_path, folder_type, remote_id, uidvalidity, message_count, unread_count, sync_enabled, sort_order, created_at, updated_at)
         VALUES (?, ?, 'Imported', 'Imported', 'archive', 'Imported', NULL, 0, 0, 0, 99, ?, ?)
         ON CONFLICT DO NOTHING",
    )
    .bind(&id)
    .bind(account_id)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|err| err.to_string())?;
    Ok(id)
}

async fn import_one_message(
    app: &tauri::AppHandle,
    pool: &SqlitePool,
    account_id: &str,
    account_email: &str,
    folder_id: &str,
    raw: &str,
) -> Result<bool, String> {
    let message = strip_mbox_from_line(raw);
    let parsed = crate::providers::imap::parse_raw_rfc822(message.as_bytes())?;
    import_parsed_message(
        app,
        pool,
        account_id,
        account_email,
        folder_id,
        parsed,
        message.as_bytes(),
    )
    .await
}

async fn import_parsed_message(
    app: &tauri::AppHandle,
    pool: &SqlitePool,
    account_id: &str,
    account_email: &str,
    folder_id: &str,
    parsed: crate::providers::imap::ParsedRawEmail,
    raw_source: &[u8],
) -> Result<bool, String> {
    let cache_dir = app
        .path()
        .resolve("satchel/attachments", tauri::path::BaseDirectory::AppData)
        .map_err(|error| error.to_string())?;
    import_parsed_message_inner(
        &cache_dir,
        pool,
        account_id,
        account_email,
        folder_id,
        parsed,
        raw_source,
    )
    .await
}

async fn import_parsed_message_inner(
    cache_dir: &Path,
    pool: &SqlitePool,
    account_id: &str,
    account_email: &str,
    folder_id: &str,
    parsed: crate::providers::imap::ParsedRawEmail,
    raw_source: &[u8],
) -> Result<bool, String> {
    let raw_size = raw_source.len();
    let subject = parsed.subject;
    let from = parsed.from_address;
    let from_name = parsed.from_name;
    let to = parsed.to_addresses;
    let cc = parsed.cc_addresses;
    let date = parsed.date_rfc3339;
    let message_id_header = parsed.message_id_header;
    let body_text = parsed.body_text.unwrap_or_default();
    let original_body_html = parsed.body_html;
    let body_html =
        crate::email::sanitizer::sanitize_optional_email_html(original_body_html.clone())
            .await
            .map_err(|error| error.to_string())?;
    let has_attachments = !parsed.attachments.is_empty();

    if sqlx::query_scalar::<_, i64>(
        "SELECT 1 FROM messages WHERE account_id = ? AND message_id_header = ? LIMIT 1",
    )
    .bind(account_id)
    .bind(&message_id_header)
    .fetch_optional(pool)
    .await
    .map_err(|err| err.to_string())?
    .is_some()
    {
        return Ok(false);
    }

    let thread_id = format!("th-import-{}", uuid::Uuid::new_v4());
    let message_id = format!("m-import-{}", uuid::Uuid::new_v4());
    let remote_id = format!("import-{}", uuid::Uuid::new_v4());
    let now = chrono::Utc::now().to_rfc3339();
    let snippet_source = if body_text.trim().is_empty() {
        body_html.as_deref().unwrap_or_default()
    } else {
        body_text.as_str()
    };
    let snippet = snippet_source
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
        .chars()
        .take(220)
        .collect::<String>();
    let participants = serde_json::to_string(&vec![from.clone(), account_email.to_string()])
        .unwrap_or_else(|_| "[]".to_string());
    let cached_attachments =
        cache_import_attachments(cache_dir, &message_id, &parsed.attachments).await?;

    let mut tx = pool.begin().await.map_err(|err| err.to_string())?;
    sqlx::query(
        "INSERT INTO threads (id, account_id, subject, last_message_date, message_count, unread_count, has_attachments, participants, snippet, workflow_state, workflow_changed_at, snooze_until, due_date, priority, board_column, board_position, created_at, updated_at)
         VALUES (?, ?, ?, ?, 1, 0, ?, ?, ?, 'done', ?, NULL, NULL, 0, 'done', 1, ?, ?)",
    )
    .bind(&thread_id)
    .bind(account_id)
    .bind(&subject)
    .bind(&date)
    .bind(if has_attachments { 1 } else { 0 })
    .bind(participants)
    .bind(&snippet)
    .bind(&now)
    .bind(&now)
    .bind(&now)
    .execute(&mut *tx)
    .await
    .map_err(|err| err.to_string())?;

    sqlx::query(
        "INSERT INTO messages (id, account_id, folder_id, thread_id, message_id_header, in_reply_to, references_header, from_address, from_name, to_addresses, cc_addresses, bcc_addresses, reply_to, subject, date, body_text, body_html, original_body_html, original_source, snippet, is_read, is_starred, is_draft, is_sent, has_attachments, remote_uid, remote_id, raw_size, raw_headers, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, NULL, NULL, ?, ?, ?, ?, NULL, NULL, ?, ?, ?, ?, ?, ?, ?, 1, 0, 0, 0, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&message_id)
    .bind(account_id)
    .bind(folder_id)
    .bind(&thread_id)
    .bind(&message_id_header)
    .bind(&from)
    .bind(from_name.as_deref())
    .bind(serde_json::to_string(&to).unwrap_or_else(|_| "[]".to_string()))
    .bind(serde_json::to_string(&cc).unwrap_or_else(|_| "[]".to_string()))
    .bind(&subject)
    .bind(&date)
    .bind(body_text.trim())
    .bind(body_html.as_deref())
    .bind(original_body_html.as_deref())
    .bind(raw_source)
    .bind(&snippet)
    .bind(if has_attachments { 1 } else { 0 })
    .bind(&remote_id)
    .bind(&remote_id)
    .bind(raw_size as i64)
    .bind(&parsed.raw_headers)
    .bind(&now)
    .bind(&now)
    .execute(&mut *tx)
    .await
    .map_err(|err| err.to_string())?;

    for cached in cached_attachments {
        sqlx::query(
            "INSERT INTO attachments (id, message_id, filename, content_type, size, content_id, storage_path, is_inline, created_at, cached_path, remote_uid, remote_part_id)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&cached.id)
        .bind(&message_id)
        .bind(cached.filename.as_deref().map(satchel_core::utils::filename::strip_filename_controls))
        .bind(&cached.content_type)
        .bind(cached.size)
        .bind(cached.content_id.as_deref())
        .bind(cached.cached_path.as_deref())
        .bind(if cached.is_inline { 1 } else { 0 })
        .bind(&now)
        .bind(cached.cached_path.as_deref())
        .bind(&remote_id)
        .bind(&cached.remote_part_id)
        .execute(&mut *tx)
        .await
        .map_err(|err| err.to_string())?;
    }
    tx.commit().await.map_err(|err| err.to_string())?;
    Ok(true)
}

pub(crate) fn is_msg_path(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("msg"))
}

pub(crate) fn parse_msg_bytes(
    bytes: &[u8],
) -> Result<crate::providers::imap::ParsedRawEmail, String> {
    let outlook = msg_parser::Outlook::from_reader(std::io::Cursor::new(bytes))
        .map_err(|err| format!("parse .msg: {err}"))?;
    let subject = if outlook.subject.trim().is_empty() {
        "(no subject)".to_string()
    } else {
        outlook.subject.clone()
    };
    let from_address = if outlook.sender.email.trim().is_empty() {
        "unknown@import.local".to_string()
    } else {
        outlook.sender.email.clone()
    };
    let from_name = if outlook.sender.name.trim().is_empty() {
        None
    } else {
        Some(outlook.sender.name.clone())
    };
    let date_rfc3339 = normalize_msg_date(
        &outlook.message_delivery_time,
        &outlook.client_submit_time,
        &outlook.headers.date,
    );
    let message_id_header = if outlook.headers.message_id.trim().is_empty() {
        format!("<import-{}@satchel.local>", uuid::Uuid::new_v4())
    } else {
        outlook.headers.message_id.clone()
    };
    let body_html = if outlook.html.trim().is_empty() {
        outlook
            .html_from_rtf()
            .filter(|value| !value.trim().is_empty())
    } else {
        Some(outlook.html)
    };
    let body_text = if outlook.body.trim().is_empty() {
        None
    } else {
        Some(outlook.body.clone())
    };
    let attachments = outlook
        .attachments
        .into_iter()
        .enumerate()
        .filter_map(|(index, attachment)| {
            let filename = [
                attachment.long_file_name.as_str(),
                attachment.display_name.as_str(),
                attachment.file_name.as_str(),
            ]
            .into_iter()
            .find(|value| !value.trim().is_empty())
            .map(ToString::to_string);
            if attachment.payload_bytes.is_empty() && filename.is_none() {
                return None;
            }
            let content_type = crate::email::mime_type::normalize_mime_type(&attachment.mime_tag);
            let content_id = if attachment.content_id.trim().is_empty() {
                None
            } else {
                Some(attachment.content_id)
            };
            let is_inline =
                content_id.is_some() || content_type.to_ascii_lowercase().starts_with("image/");
            Some(crate::providers::imap::ParsedRawAttachment {
                remote_part_id: format!("msg-{index}"),
                filename,
                content_type,
                content_id,
                is_inline,
                bytes: attachment.payload_bytes,
            })
        })
        .collect();

    Ok(crate::providers::imap::ParsedRawEmail {
        subject,
        from_address,
        from_name,
        to_addresses: outlook.to.into_iter().map(|person| person.email).collect(),
        cc_addresses: outlook.cc.into_iter().map(|person| person.email).collect(),
        date_rfc3339,
        message_id_header,
        body_text,
        body_html,
        attachments,
        raw_headers: outlook.headers.raw,
    })
}

fn normalize_msg_date(delivered: &str, submitted: &str, header_date: &str) -> String {
    for candidate in [delivered, submitted, header_date] {
        let trimmed = candidate.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(trimmed) {
            return parsed.with_timezone(&chrono::Utc).to_rfc3339();
        }
        if let Ok(parsed) = chrono::DateTime::parse_from_rfc2822(trimmed) {
            return parsed.with_timezone(&chrono::Utc).to_rfc3339();
        }
    }
    chrono::Utc::now().to_rfc3339()
}

struct CachedImportAttachment {
    id: String,
    filename: Option<String>,
    content_type: String,
    content_id: Option<String>,
    is_inline: bool,
    size: i64,
    cached_path: Option<String>,
    remote_part_id: String,
}

async fn cache_import_attachments(
    cache_dir: &Path,
    message_id: &str,
    attachments: &[crate::providers::imap::ParsedRawAttachment],
) -> Result<Vec<CachedImportAttachment>, String> {
    if attachments.is_empty() {
        return Ok(Vec::new());
    }

    tokio::fs::create_dir_all(cache_dir)
        .await
        .map_err(|err| format!("create attachment cache: {err}"))?;

    let mut cached = Vec::with_capacity(attachments.len());
    for attachment in attachments {
        let id = format!("att-import-{}", uuid::Uuid::new_v4());
        let filename = attachment
            .filename
            .clone()
            .or_else(|| attachment.content_id.clone())
            .unwrap_or_else(|| "attachment".to_string());
        let path = crate::commands::attachments::attachment_cache_path(
            cache_dir,
            &format!("{message_id}-{id}"),
            &filename,
        );
        tokio::fs::write(&path, &attachment.bytes)
            .await
            .map_err(|err| format!("write imported attachment cache: {err}"))?;
        cached.push(CachedImportAttachment {
            id,
            filename: Some(filename),
            content_type: attachment.content_type.clone(),
            content_id: attachment.content_id.clone(),
            is_inline: attachment.is_inline,
            size: i64::try_from(attachment.bytes.len()).unwrap_or(i64::MAX),
            cached_path: Some(path.to_string_lossy().to_string()),
            remote_part_id: attachment.remote_part_id.clone(),
        });
    }

    Ok(cached)
}

// ---------------------------------------------------------------------------
// Profile backup, restore and credential status
// ---------------------------------------------------------------------------
//
// These used to be `satchel-cli` subcommands (ADR 0005). The CLI could work on
// a closed profile; the app cannot, so restore is staged rather than applied —
// see `restore_backup`.

/// Portable backup key files and legacy fallback locations. The active key
/// normally lives in the profile-specific OS keychain entry.
pub use satchel_core::db::MASTER_KEY_FILES;

pub use satchel_core::db::RESTORE_PENDING_SUFFIX;

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/backup/"))]
pub struct BackupEntry {
    pub path: String,
    /// Directory name, `satchel-YYYYMMDD-HHMMSS`.
    pub name: String,
    pub created_at: String,
    pub bytes: u64,
    pub account_count: i64,
    pub message_count: i64,
    /// Number of exported keys or local keychain snapshots. Zero means encrypted
    /// credentials cannot be read back.
    pub master_key_files: usize,
    /// The key travels wrapped under a passphrase, so restoring needs it.
    pub master_key_wrapped: bool,
    /// Highest migration version in the backup. Above what this build knows,
    /// `restore_backup` refuses it — knowing that before choosing helps.
    pub schema_version: i64,
    /// False when the backup's search index no longer matched its messages when
    /// it was written. `None` for a backup without a manifest: the listing does
    /// not open the database, so it cannot tell. `restore_backup` checks.
    pub search_index_ok: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/backup/"))]
pub struct RestoreStaged {
    pub source: String,
    pub pending_path: String,
    pub account_count: i64,
    pub message_count: i64,
    pub master_key_files: usize,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/backup/"))]
pub struct CredentialsStatus {
    pub total_accounts: i64,
    pub encrypted_accounts: i64,
    pub plaintext_accounts: i64,
    pub master_key_present: bool,
    pub database_bytes: u64,
    /// A restore is staged and takes effect on the next start.
    pub restore_pending: bool,
}

/// `<AppData>/satchel` — the directory holding the database and the key files.
fn profile_dir(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    app.path()
        .resolve("satchel", tauri::path::BaseDirectory::AppData)
        .map_err(|err| err.to_string())
}

/// Open a database file read-only, without touching the live pool.
async fn open_readonly(path: &Path) -> Result<SqlitePool, String> {
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(path)
        .read_only(true);
    sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .map_err(|err| format!("{}: {err}", path.display()))
}

/// What a backup database is worth knowing before it is chosen.
#[derive(Debug)]
struct DatabaseFacts {
    accounts: i64,
    messages: i64,
    schema_version: i64,
    search_index_ok: bool,
}

// How often `inspect_database` ran. The listing must never reach it.
#[cfg(test)]
tokio::task_local! {
    static INTEGRITY_CHECKS: std::cell::Cell<usize>;
}

/// Integrity check, row counts and schema version. An unverified backup is a
/// guess.
///
/// `PRAGMA integrity_check` reads every page, so this belongs where the wait is
/// expected — writing a backup and restoring one — not in the listing.
async fn inspect_database(path: &Path) -> Result<DatabaseFacts, String> {
    #[cfg(test)]
    let _ = INTEGRITY_CHECKS.try_with(|count| count.set(count.get() + 1));
    let pool = open_readonly(path).await?;
    let integrity: String = sqlx::query_scalar("PRAGMA integrity_check;")
        .fetch_one(&pool)
        .await
        .map_err(|err| err.to_string())?;
    if integrity != "ok" {
        pool.close().await;
        return Err(format!("integrity check failed: {integrity}"));
    }
    let accounts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM accounts")
        .fetch_one(&pool)
        .await
        .map_err(|err| err.to_string())?;
    let messages: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages")
        .fetch_one(&pool)
        .await
        .map_err(|err| err.to_string())?;
    let schema_version = satchel_core::backup::schema_version(&pool).await;
    // Read-only, so this is the coarse count comparison rather than FTS5's own
    // integrity-check; `stage_restore` runs the full one on its writable
    // snapshot before anything is published.
    let search_index_ok = satchel_core::db::message_index_matches_row_count(&pool).await;
    pool.close().await;
    Ok(DatabaseFacts {
        accounts,
        messages,
        schema_version: schema_version?,
        search_index_ok: search_index_ok?,
    })
}

/// Export the active key, including a keychain-only profile, into the backup.
fn copy_master_keys(from: &Path, to: &Path, passphrase: Option<&str>) -> Result<usize, String> {
    crate::utils::crypto::export_master_key(from, to, passphrase)
}

fn count_master_keys(dir: &Path) -> usize {
    crate::utils::crypto::backup_master_key_count(dir)
}

/// Consistent snapshot of the running database.
///
/// `VACUUM INTO` takes a read lock and writes one compacted file with the WAL
/// already folded in — unlike copying satchel.db, which can catch a torn write,
/// and unlike copying db+wal+shm, which is what let backups balloon to 4 GB.
#[tauri::command]
pub async fn create_backup(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::AppState>,
    pool: tauri::State<'_, SqlitePool>,
    target_dir: String,
    passphrase: Option<String>,
    acknowledge_unprotected_key: Option<bool>,
) -> CommandResult<BackupEntry> {
    let passphrase = passphrase.filter(|value| !value.is_empty());
    if let Some(passphrase) = passphrase.as_deref() {
        crate::utils::crypto::validate_backup_passphrase(passphrase)?;
    }
    // Without a passphrase the folder holds the key to every stored
    // credential; the user has to have been told that and said yes.
    if passphrase.is_none() && acknowledge_unprotected_key != Some(true) {
        return Err(CommandError::new(
            "backend.errors.backupPassphraseOrAcknowledgementRequired",
        ));
    }
    let root = std::path::PathBuf::from(&target_dir);
    if !root.is_dir() {
        return Err(format!("{target_dir} is not a directory").into());
    }
    let root = state.write_grants.take(&root)?;
    let created_at = chrono::Local::now();
    let name = format!("satchel-{}", created_at.format("%Y%m%d-%H%M%S"));
    let dest = root.join(&name);
    std::fs::create_dir_all(&dest).map_err(|err| err.to_string())?;

    let target = dest.join("satchel.db");
    let _ = std::fs::remove_file(&target);
    // VACUUM INTO takes an expression rather than a bound parameter; quote the
    // path the way SQLite expects.
    //
    // It may also renumber the rowids of a table without an explicit INTEGER
    // PRIMARY KEY, and `messages` is one — `messages_fts` and
    // `message_search_projection` both key off those numbers. Observed builds
    // preserve them; the documented guarantee is not there, which is why the
    // inspection below and `stage_restore` check the index instead of assuming.
    sqlx::query(&format!(
        "VACUUM INTO '{}'",
        target.to_string_lossy().replace('\'', "''")
    ))
    .execute(pool.inner())
    .await
    .map_err(|err| format!("Sicherung fehlgeschlagen: {err}"))?;

    // The encryption key lives outside the database. A backup without it cannot
    // restore an account whose credentials are encrypted.
    let master_key_files = copy_master_keys(&profile_dir(&app)?, &dest, passphrase.as_deref())?;
    let facts = inspect_database(&target).await?;
    let bytes = std::fs::metadata(&target)
        .map(|meta| meta.len())
        .unwrap_or(0);
    let created_at = created_at.to_utc().to_rfc3339();
    satchel_core::backup::write_backup_manifest(
        &dest,
        &satchel_core::backup::BackupManifest {
            schema_version: facts.schema_version,
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            created_at: created_at.clone(),
            account_count: facts.accounts,
            message_count: facts.messages,
            search_index_ok: Some(facts.search_index_ok),
        },
    )?;

    Ok(BackupEntry {
        path: dest.to_string_lossy().to_string(),
        name,
        created_at,
        bytes,
        account_count: facts.accounts,
        message_count: facts.messages,
        master_key_files,
        master_key_wrapped: passphrase.is_some(),
        schema_version: facts.schema_version,
        search_index_ok: Some(facts.search_index_ok),
    })
}

/// Every backup directly below `dir`, newest first.
///
/// Nothing here opens a database. The facts a backup reports about itself were
/// measured when it was written and live in its manifest; reading them back out
/// of the file meant a full-page `integrity_check` and two `COUNT(*)` per
/// listed backup, so ten backups of a 5-GB database read 50 GB before the first
/// row appeared. A backup without a manifest lists with zeroes.
#[tauri::command]
pub async fn list_backups(dir: String) -> CommandResult<Vec<BackupEntry>> {
    let root = std::path::PathBuf::from(&dir);
    let mut read_dir = match tokio::fs::read_dir(&root).await {
        Ok(entries) => entries,
        Err(_) => return Ok(Vec::new()),
    };
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    while let Some(entry) = read_dir.next_entry().await.map_err(|err| err.to_string())? {
        let path = entry.path();
        if tokio::fs::metadata(path.join("satchel.db"))
            .await
            .is_ok_and(|meta| meta.is_file())
        {
            candidates.push(path);
        }
    }
    // Timestamped names sort chronologically.
    candidates.sort();
    candidates.reverse();

    let mut entries = Vec::new();
    for path in candidates {
        let metadata = tokio::fs::metadata(path.join("satchel.db"))
            .await
            .map_err(|err| err.to_string())?;
        let manifest = satchel_core::backup::read_backup_manifest(&path).unwrap_or_default();
        entries.push(BackupEntry {
            name: path
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_default(),
            path: path.to_string_lossy().to_string(),
            // The manifest records when the backup was taken; the mtime only
            // says when the file last changed.
            created_at: if manifest.created_at.is_empty() {
                chrono::DateTime::<chrono::Utc>::from(
                    metadata
                        .modified()
                        .unwrap_or(std::time::SystemTime::UNIX_EPOCH),
                )
                .to_rfc3339()
            } else {
                manifest.created_at.clone()
            },
            bytes: metadata.len(),
            account_count: manifest.account_count,
            message_count: manifest.message_count,
            master_key_files: count_master_keys(&path),
            master_key_wrapped: crate::utils::crypto::backup_master_key_is_wrapped(&path),
            schema_version: manifest.schema_version,
            search_index_ok: manifest.search_index_ok,
        });
    }
    Ok(entries)
}

/// Stage a backup so the next start runs on it.
///
/// The CLI could overwrite the database because it refused to run while the app
/// was open. In-process there is no such moment: workers hold connections, the
/// master key is cached, and replacing the database underneath them is unsafe. So
/// a consistent snapshot is validated and re-encrypted for the active key,
/// then published with a completion manifest as `satchel.db.restore-pending` and `db::apply_pending_restore` moves it into
/// place before the pool opens.
///
/// A backup whose master key is wrapped needs its passphrase; without it, or
/// with a wrong one, nothing is staged.
#[tauri::command]
pub async fn restore_backup(
    app: tauri::AppHandle,
    path: String,
    passphrase: Option<String>,
) -> CommandResult<RestoreStaged> {
    let source = std::path::PathBuf::from(&path);
    let candidate = if source.is_dir() {
        source.clone()
    } else {
        source
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| "invalid path".to_string())?
    };
    let db = if source.is_file() {
        source.clone()
    } else {
        candidate.join("satchel.db")
    };
    if !db.is_file() {
        return Err(format!("no satchel.db under {}", candidate.display()).into());
    }

    // Refuse a corrupt backup before it can displace a working database.
    let facts = inspect_database(&db).await?;

    let profile = profile_dir(&app)?;
    std::fs::create_dir_all(&profile).map_err(|err| err.to_string())?;
    let pending = profile.join(format!("satchel.db{RESTORE_PENDING_SUFFIX}"));
    // Before `stage_restore`: an unopenable key must not leave a staged file
    // behind for the next start to apply.
    let keys = crate::utils::crypto::backup_master_keys(
        &candidate,
        passphrase.as_deref().filter(|value| !value.is_empty()),
    )?;
    let current_key = crate::utils::crypto::load_or_create_master_key(&profile).ok();
    satchel_core::backup::stage_restore(&profile, &db, &keys, current_key.as_ref()).await?;
    let master_key_files = keys.len();

    Ok(RestoreStaged {
        source: candidate.to_string_lossy().to_string(),
        pending_path: pending.to_string_lossy().to_string(),
        account_count: facts.accounts,
        message_count: facts.messages,
        master_key_files,
    })
}

/// Delete every backup below `dir` except the newest `keep`.
#[tauri::command]
pub async fn prune_backups(dir: String, keep: usize) -> CommandResult<Vec<String>> {
    let keep = keep.max(1);
    let stale: Vec<String> = list_backups(dir)
        .await?
        .into_iter()
        .skip(keep)
        .map(|entry| entry.path)
        .collect();
    for path in &stale {
        crate::utils::crypto::remove_local_backup_master_key(Path::new(path))?;
        tokio::fs::remove_dir_all(path)
            .await
            .map_err(|err| format!("{path}: {err}"))?;
    }
    Ok(stale)
}

/// Drop a staged restore before it is applied.
#[tauri::command]
pub async fn cancel_pending_restore(app: tauri::AppHandle) -> CommandResult<()> {
    let profile = profile_dir(&app)?;
    satchel_core::backup::cancel_restore(&profile)?;
    Ok(())
}

/// Whether account credentials are encrypted at rest. Never returns a secret —
/// only counts.
#[tauri::command]
pub async fn credentials_status(
    app: tauri::AppHandle,
    pool: tauri::State<'_, crate::db::ReadPool>,
) -> CommandResult<CredentialsStatus> {
    let (total_accounts, encrypted_accounts) =
        crate::commands::accounts::count_account_config_encryption(pool.inner().as_ref()).await?;

    let profile = profile_dir(&app)?;
    // Keychain-only profiles have no fallback file to inspect.
    let master_key_present = crate::utils::crypto::load_or_create_master_key(&profile).is_ok();
    let database_bytes = std::fs::metadata(profile.join("satchel.db"))
        .map(|meta| meta.len())
        .unwrap_or(0);

    Ok(CredentialsStatus {
        total_accounts,
        encrypted_accounts,
        plaintext_accounts: (total_accounts - encrypted_accounts).max(0),
        master_key_present,
        database_bytes,
        restore_pending: satchel_core::backup::restore_pending(&profile),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::email::mbox::MAX_MESSAGE_BYTES;

    /// A stored subject comes from the network. Before folding, a newline in it
    /// ended the header and started whatever the sender wrote next.
    #[test]
    fn a_subject_cannot_inject_a_header_line() {
        let headers = build_mbox_headers(
            "sender@example.test",
            "",
            None,
            None,
            Some("Rechnung\nBcc: attacker@example.test\nX-Spoof: yes"),
            None,
            None,
            None,
            "2026-01-01T00:00:00Z",
        );

        assert!(headers.contains("Subject: Rechnung Bcc: attacker@example.test X-Spoof: yes"));
        for line in headers.lines() {
            assert!(
                !line.starts_with("Bcc:") && !line.starts_with("X-Spoof:"),
                "injected header survived: {line}"
            );
        }
    }

    /// Folding must be reversible: unfolding restores the value.
    #[test]
    fn a_long_header_folds_at_whitespace_and_unfolds_again() {
        let subject = "Quartalsbericht ".repeat(12);
        let folded = mbox_header("Subject", subject.trim());

        assert!(
            folded.lines().count() > 1,
            "a 190-octet header must fold: {folded}"
        );
        for line in folded.lines().skip(1) {
            assert!(
                line.starts_with(' '),
                "continuation needs leading WSP: {line}"
            );
        }
        let unfolded = folded.replace("\n ", " ");
        assert_eq!(unfolded.trim_end(), format!("Subject: {}", subject.trim()));
    }

    /// A name with a newline must not escape the From line either.
    #[test]
    fn a_display_name_cannot_inject_a_header_line() {
        let headers = build_mbox_headers(
            "sender@example.test",
            "Buck\nTo: victim@example.test",
            None,
            None,
            None,
            None,
            None,
            None,
            "2026-01-01T00:00:00Z",
        );

        assert_eq!(
            headers
                .lines()
                .filter(|line| line.starts_with("To:"))
                .count(),
            0
        );
    }

    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use std::str::FromStr;

    /// Throwaway database with the real schema and cascades armed.
    async fn migrated_pool() -> SqlitePool {
        let options = SqliteConnectOptions::from_str("sqlite::memory:")
            .expect("sqlite options")
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .expect("in-memory pool");
        sqlx::migrate!("../crates/satchel-core/src/db/migrations")
            .run(&pool)
            .await
            .expect("migrations");
        let foreign_keys: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
            .fetch_one(&pool)
            .await
            .expect("read foreign_keys pragma");
        assert_eq!(foreign_keys, 1, "the cascade test needs foreign keys on");
        pool
    }

    async fn seed_account_with_children(pool: &SqlitePool) {
        for statement in [
            "INSERT INTO accounts
             (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
             VALUES ('account', 'Account', 'account@example.com', 'imap', '{}', 'ref', '', '')",
            "INSERT INTO threads
             (id, account_id, subject, last_message_date, workflow_state, created_at, updated_at)
             VALUES ('thread', 'account', 'Subject', '', 'inbox', '', '')",
            "INSERT INTO signatures (id, account_id, name, body_text, created_at, updated_at)
             VALUES ('signature', 'account', 'Sig', 'bye', '', '')",
            "INSERT INTO local_webhooks (id, url, created_at, updated_at)
             VALUES ('webhook', 'http://127.0.0.1/hook', '', '')",
            "INSERT INTO sender_preferences
             (account_id, sender_email, category, created_at, updated_at)
             VALUES ('account', 'boss@example.com', 'priority', '', '')",
            "INSERT INTO signature_assignments
             (account_id, identity_email, signature_id, created_at, updated_at)
             VALUES ('account', 'account@example.com', 'signature', '', '')",
            "INSERT INTO signature_suggestions
             (id, account_id, identity_email, body_text, created_at, updated_at)
             VALUES ('suggestion', 'account', 'account@example.com', 'bye', '', '')",
            "INSERT INTO signature_assets
             (id, signature_id, filename, content_type, content_id, content, created_at)
             VALUES ('asset', 'signature', 'logo.png', 'image/png', 'cid', x'00', '')",
            "INSERT INTO calendar_sources (account_id, source, remote_id, name, updated_at)
             VALUES ('account', 'caldav', 'remote', 'Work', '')",
            "INSERT INTO oauth_flow_sessions
             (state, account_id, provider, encrypted_code_verifier, created_at, expires_at)
             VALUES ('state', 'account', 'gmail', 'secret', '', '')",
            "INSERT INTO account_sync_leases
             (account_id, owner_token, lease_expires_at, updated_at)
             VALUES ('account', 'token', '', '')",
            "INSERT INTO follow_up_reminders
             (id, account_id, thread_id, due_at, created_at, updated_at)
             VALUES ('reminder', 'account', 'thread', '', '', '')",
            "INSERT INTO calendar_invites
             (id, account_id, uid, raw_ics, created_at, updated_at)
             VALUES ('invite', 'account', 'uid', 'BEGIN:VCALENDAR', '', '')",
            "INSERT INTO webhook_deliveries
             (id, webhook_id, event_name, idempotency_key, payload_json, next_attempt_at,
              created_at, updated_at)
             VALUES ('delivery', 'webhook', 'mail:new', 'key', '{}', '', '', '')",
        ] {
            sqlx::query(statement)
                .execute(pool)
                .await
                .expect("seed platform row");
        }
    }

    async fn row_count(pool: &SqlitePool, table: &str) -> i64 {
        sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(pool)
            .await
            .unwrap_or_else(|err| panic!("count {table}: {err}"))
    }

    #[tokio::test]
    async fn message_import_sanitizes_html_and_retains_original_source() {
        let pool = migrated_pool().await;
        seed_account_with_children(&pool).await;
        let folder_id = super::ensure_import_folder(&pool, "account").await.unwrap();
        let raw = b"From: sender@example.test\r\nMessage-ID: <sanitize-import>\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<p onclick=bad()>Hi<script>bad()</script></p>";
        let parsed = crate::providers::imap::parse_raw_rfc822(raw).unwrap();
        let cache_dir = tempfile::tempdir().unwrap();
        super::import_parsed_message_inner(
            cache_dir.path(),
            &pool,
            "account",
            "me@example.test",
            &folder_id,
            parsed,
            raw,
        )
        .await
        .unwrap();
        let (cleaned, original, source): (String, String, Vec<u8>) = sqlx::query_as("SELECT body_html, original_body_html, original_source FROM messages WHERE message_id_header = '<sanitize-import>'").fetch_one(&pool).await.unwrap();
        assert_eq!(cleaned, "<p>Hi</p>");
        assert!(original.contains("<script>"));
        assert_eq!(source, raw);
    }

    /// A replacing insert would delete the account row and cascade into every
    /// child table that the export does not carry.
    /// The upsert only protects the child rows while the conflict target is the
    /// real primary key of the table.
    #[tokio::test]
    async fn portable_primary_keys_match_the_schema() {
        let pool = migrated_pool().await;
        for table in PORTABLE_TABLES {
            let actual: Vec<String> = sqlx::query_scalar(
                "SELECT name FROM pragma_table_info(?) WHERE pk > 0 ORDER BY pk",
            )
            .bind(table.name)
            .fetch_all(&pool)
            .await
            .unwrap_or_else(|err| panic!("primary key of {}: {err}", table.name));
            assert_eq!(actual, table.primary_key, "primary key of {}", table.name);
        }
    }

    /// The export must carry every column the schema has. Reading the list from
    /// `pragma_table_info` is what makes that true — this test goes red the
    /// moment a hand-kept list comes back or a portable table is renamed away.
    #[tokio::test]
    async fn portable_tables_cover_every_schema_column() {
        let pool = migrated_pool().await;
        for table in PORTABLE_TABLES {
            let schema: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_table_info(?)")
                .bind(table.name)
                .fetch_all(&pool)
                .await
                .unwrap_or_else(|err| panic!("columns of {}: {err}", table.name));
            assert!(!schema.is_empty(), "{} is not in the schema", table.name);

            let exported = portable_columns(&pool, table.name)
                .await
                .unwrap_or_else(|err| panic!("{err}"));
            assert_eq!(exported, schema, "exported columns of {}", table.name);

            for column in table.redacted_columns {
                assert!(
                    schema.iter().any(|name| name == column),
                    "{}.{column} is not in the schema",
                    table.name
                );
            }
        }
    }

    /// A hand-kept column list silently drops whatever a later migration added.
    /// The upsert hides that on the machine the export came from — a column the
    /// file omits is simply not updated — so the proof runs into a *fresh*
    /// database, which is what restoring an export elsewhere actually is.
    #[tokio::test]
    async fn roundtrip_preserves_columns_added_after_0019() {
        let source = migrated_pool().await;
        seed_account_with_children(&source).await;
        for statement in [
            "UPDATE accounts SET kind = 'calendar' WHERE id = 'account'",
            "UPDATE signatures SET short_body_text = 'KURZ', short_body_html = '<p>KURZ</p>'",
            "INSERT INTO calendar_events
             (id, account_id, uid, title, starts_at, ends_at, source, remote_href, remote_etag,
              collection_href, raw_ics, created_at, updated_at)
             VALUES ('event', 'account', 'uid', 'Standup', '', '', 'caldav', '/cal/1', 'etag',
              '/cal/', 'BEGIN:VCALENDAR', '', '')",
            "INSERT INTO contacts
             (id, email, display_name, avatar_source, avatar_locked, avatar_updated_at,
              created_at, updated_at)
             VALUES ('contact', 'boss@example.com', 'Boss', 'gravatar', 1, 'yesterday', '', '')",
        ] {
            sqlx::query(statement)
                .execute(&source)
                .await
                .expect("seed a column added after 0019");
        }

        let (export, _) = collect_platform_export(&source, false)
            .await
            .expect("export platform data");
        let target = migrated_pool().await;
        apply_platform_export(None, &target, &export)
            .await
            .expect("import platform data");

        let kind: String = sqlx::query_scalar("SELECT kind FROM accounts WHERE id = 'account'")
            .fetch_one(&target)
            .await
            .expect("read account kind");
        assert_eq!(
            kind, "calendar",
            "a calendar source came back as a mail account"
        );

        let (short_text, short_html): (String, Option<String>) =
            sqlx::query_as("SELECT short_body_text, short_body_html FROM signatures")
                .fetch_one(&target)
                .await
                .expect("read signature");
        assert_eq!(short_text, "KURZ");
        assert_eq!(short_html.as_deref(), Some("<p>KURZ</p>"));

        let (href, etag, collection, ics): (
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
        ) = sqlx::query_as(
            "SELECT remote_href, remote_etag, collection_href, raw_ics FROM calendar_events",
        )
        .fetch_one(&target)
        .await
        .expect("read calendar event");
        // A NULL remote_href does not collide in idx_calendar_events_remote —
        // the next DAV sync would file the event a second time.
        assert_eq!(href.as_deref(), Some("/cal/1"));
        assert_eq!(etag.as_deref(), Some("etag"));
        assert_eq!(collection.as_deref(), Some("/cal/"));
        assert_eq!(ics.as_deref(), Some("BEGIN:VCALENDAR"));

        let (avatar_source, avatar_locked, avatar_updated): (Option<String>, i64, Option<String>) =
            sqlx::query_as("SELECT avatar_source, avatar_locked, avatar_updated_at FROM contacts")
                .fetch_one(&target)
                .await
                .expect("read contact");
        assert_eq!(avatar_source.as_deref(), Some("gravatar"));
        assert_eq!(avatar_locked, 1);
        assert_eq!(avatar_updated.as_deref(), Some("yesterday"));
    }

    /// Export files travel between versions in both directions.
    #[tokio::test]
    async fn import_tolerates_a_column_set_from_another_version() {
        let pool = migrated_pool().await;
        seed_account_with_children(&pool).await;
        sqlx::query("UPDATE accounts SET kind = 'calendar' WHERE id = 'account'")
            .execute(&pool)
            .await
            .expect("mark the account as a calendar source");

        let (mut export, _) = collect_platform_export(&pool, false)
            .await
            .expect("export platform data");
        let accounts = export
            .tables
            .get_mut("accounts")
            .expect("accounts in export");
        accounts[0].remove("kind");
        accounts[0].insert("from_a_later_release".into(), Some("value".into()));
        accounts[0].insert("display_name".into(), Some("Renamed".into()));

        apply_platform_export(None, &pool, &export)
            .await
            .expect("import an export from another version");

        let (kind, name): (String, String) =
            sqlx::query_as("SELECT kind, display_name FROM accounts WHERE id = 'account'")
                .fetch_one(&pool)
                .await
                .expect("read account");
        assert_eq!(kind, "calendar", "an omitted column keeps its local value");
        assert_eq!(name, "Renamed");
    }

    #[tokio::test]
    async fn platform_import_keeps_the_cascade_children_of_an_account() {
        let pool = migrated_pool().await;
        seed_account_with_children(&pool).await;

        let (export, _) = collect_platform_export(&pool, false)
            .await
            .expect("export platform data");
        apply_platform_export(None, &pool, &export)
            .await
            .expect("import platform data");

        for table in [
            "sender_preferences",
            "signature_assignments",
            "signature_suggestions",
            "signature_assets",
            "calendar_sources",
            "oauth_flow_sessions",
            "account_sync_leases",
            "follow_up_reminders",
            "calendar_invites",
            "webhook_deliveries",
        ] {
            assert_eq!(
                row_count(&pool, table).await,
                1,
                "import wiped {table} through a cascade"
            );
        }
        let assigned: Option<String> =
            sqlx::query_scalar("SELECT signature_id FROM signature_assignments")
                .fetch_one(&pool)
                .await
                .expect("read signature assignment");
        assert_eq!(assigned.as_deref(), Some("signature"));
    }

    #[tokio::test]
    async fn platform_import_updates_an_existing_row() {
        let pool = migrated_pool().await;
        seed_account_with_children(&pool).await;

        let (mut export, _) = collect_platform_export(&pool, false)
            .await
            .expect("export platform data");
        let accounts = export
            .tables
            .get_mut("accounts")
            .expect("accounts in export");
        accounts[0].insert("display_name".into(), Some("Renamed".into()));
        accounts[0].insert("email".into(), Some("renamed@example.com".into()));
        let written = apply_platform_export(None, &pool, &export)
            .await
            .expect("import platform data");
        assert!(written > 0, "import reported no written rows");

        let (name, email): (String, String) =
            sqlx::query_as("SELECT display_name, email FROM accounts WHERE id = 'account'")
                .fetch_one(&pool)
                .await
                .expect("read account");
        assert_eq!(name, "Renamed");
        assert_eq!(email, "renamed@example.com");
        assert_eq!(row_count(&pool, "accounts").await, 1);
    }

    async fn account_secrets(pool: &SqlitePool) -> (String, String) {
        sqlx::query_as("SELECT config, credentials_ref FROM accounts WHERE id = 'account'")
            .fetch_one(pool)
            .await
            .expect("read account secrets")
    }

    /// The account config carries the IMAP/SMTP password and the OAuth refresh
    /// token. An export that promises redaction must not contain it.
    #[tokio::test]
    async fn platform_export_redacts_the_account_config() {
        let pool = migrated_pool().await;
        seed_account_with_children(&pool).await;
        sqlx::query("UPDATE accounts SET config = 'enc:v1:the-account-secret'")
            .execute(&pool)
            .await
            .expect("store an encrypted config");

        let (export, _) = collect_platform_export(&pool, true)
            .await
            .expect("export platform data");
        let json = serde_json::to_string(&export).expect("serialize export");

        assert!(!json.contains("enc:v1:the-account-secret"));
        assert!(export.redacted_sensitive);
    }

    /// The manifest describes the file, not the request: an export that keeps
    /// the secrets must not claim to be redacted.
    #[tokio::test]
    async fn an_unredacted_export_says_so() {
        let pool = migrated_pool().await;
        seed_account_with_children(&pool).await;
        sqlx::query("UPDATE accounts SET config = 'enc:v1:the-account-secret'")
            .execute(&pool)
            .await
            .expect("store an encrypted config");

        let (export, _) = collect_platform_export(&pool, false)
            .await
            .expect("export platform data");

        assert!(!export.redacted_sensitive);
    }

    #[tokio::test]
    async fn platform_import_keeps_the_local_config_when_the_export_was_redacted() {
        let pool = migrated_pool().await;
        seed_account_with_children(&pool).await;
        let (export, _) = collect_platform_export(&pool, true)
            .await
            .expect("export platform data");
        sqlx::query("UPDATE accounts SET config = 'enc:v1:rotated-since-the-export'")
            .execute(&pool)
            .await
            .expect("change the local config");

        apply_platform_export(None, &pool, &export)
            .await
            .expect("import platform data");

        let (config, credentials_ref) = account_secrets(&pool).await;
        assert_eq!(config, "enc:v1:rotated-since-the-export");
        assert_eq!(credentials_ref, "ref");
    }

    /// The redacted file must not cost the user their login when it is read
    /// back into the database it came from.
    #[tokio::test]
    async fn a_redacted_roundtrip_leaves_the_account_loginable() {
        let pool = migrated_pool().await;
        seed_account_with_children(&pool).await;
        sqlx::query("UPDATE accounts SET config = 'enc:v1:the-account-secret'")
            .execute(&pool)
            .await
            .expect("store an encrypted config");
        let before = account_secrets(&pool).await;

        let (export, _) = collect_platform_export(&pool, true)
            .await
            .expect("export platform data");
        apply_platform_export(None, &pool, &export)
            .await
            .expect("import platform data");

        assert_eq!(account_secrets(&pool).await, before);
    }

    #[tokio::test]
    async fn mbox_entries_split_at_every_from_line() {
        let raw = concat!(
            "From alice@example.com Sat Jan  1 00:00:00 2026\n",
            "From: Alice <alice@example.com>\n",
            "Subject: One\n\n",
            "Body one\n\n",
            "From bob@example.com Sat Jan  1 00:01:00 2026\n",
            "From: Bob <bob@example.com>\n",
            "Subject: Two\n\n",
            "Body two\n"
        );

        let chunks = collect_entries(raw.as_bytes(), MAX_MESSAGE_BYTES).await.0;
        assert_eq!(chunks.len(), 2);
        assert!(chunks[0].contains("Subject: One"));
        assert!(chunks[1].contains("Subject: Two"));
    }

    #[test]
    fn eml_without_mbox_line_keeps_first_header() {
        let raw = "From: Alice <alice@example.com>\nSubject: Hello\n\nBody";
        assert_eq!(strip_mbox_from_line(raw), raw);
    }

    #[test]
    fn mbox_line_is_removed_before_header_parse() {
        let raw = "From alice@example.com Sat Jan  1 00:00:00 2026\nFrom: Alice <alice@example.com>\n\nBody";
        assert!(strip_mbox_from_line(raw).starts_with("From: Alice"));
    }

    #[test]
    fn rfc822_parser_unfolds_import_headers() {
        let raw = "Subject: Long\n folded\nFrom: A <a@example.com>\n\nBody";
        let parsed = crate::providers::imap::parse_raw_rfc822(raw.as_bytes()).expect("parse eml");
        assert_eq!(parsed.subject, "Long folded");
        assert_eq!(parsed.from_address, "a@example.com");
    }

    #[test]
    fn detects_outlook_msg_paths_case_insensitively() {
        assert!(is_msg_path("/tmp/mail.MSG"));
        assert!(!is_msg_path("/tmp/mail.eml"));
    }

    #[test]
    fn normalizes_msg_dates_from_transport_headers() {
        assert_eq!(
            normalize_msg_date("", "", "Fri, 16 Feb 2024 10:11:12 +0100"),
            "2024-02-16T09:11:12+00:00"
        );
    }

    // PERF-13: the listing must not open a database, and export and import must
    // not materialise the whole mailbox.

    /// Backup directory with a manifest that disagrees with the database next
    /// to it. Whatever the listing reports tells us which one it read.
    fn backup_dir(root: &Path, name: &str, bytes: u64, messages: i64) -> std::path::PathBuf {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).expect("backup dir");
        // Sparse: the listing only stats it, which is the point.
        std::fs::File::create(dir.join("satchel.db"))
            .and_then(|file| file.set_len(bytes))
            .expect("backup database");
        satchel_core::backup::write_backup_manifest(
            &dir,
            &satchel_core::backup::BackupManifest {
                schema_version: 42,
                app_version: "0.1.0".into(),
                created_at: "2026-09-10T12:00:00+00:00".into(),
                account_count: 2,
                message_count: messages,
                search_index_ok: Some(true),
            },
        )
        .expect("manifest");
        dir
    }

    #[tokio::test]
    async fn listing_backups_opens_no_database() {
        let root = tempfile::tempdir().expect("root");
        for index in 0..10 {
            // Invalid SQLite files of different sizes must still list using
            // their manifests, without inspecting database pages.
            let bytes = if index == 3 { 64 * 1024 * 1024 } else { 4096 };
            backup_dir(
                root.path(),
                &format!("satchel-20260910-12000{index}"),
                bytes,
                1500,
            );
        }

        let entries = INTEGRITY_CHECKS
            .scope(std::cell::Cell::new(0), async {
                let entries = list_backups(root.path().to_string_lossy().to_string())
                    .await
                    .expect("listing");
                assert_eq!(
                    INTEGRITY_CHECKS.with(|count| count.get()),
                    0,
                    "the listing ran an integrity check"
                );
                entries
            })
            .await;

        assert_eq!(entries.len(), 10);
        // Newest first, and the facts come from the manifest.
        assert_eq!(entries[0].name, "satchel-20260910-120009");
        assert_eq!(entries[0].message_count, 1500);
        assert_eq!(entries[0].account_count, 2);
        assert_eq!(entries[0].search_index_ok, Some(true));
        assert_eq!(entries[0].created_at, "2026-09-10T12:00:00+00:00");
    }

    /// A backup from before manifests reports what can be seen from outside,
    /// and says nothing about the search index rather than calling it broken.
    #[tokio::test]
    async fn a_backup_without_a_manifest_lists_without_counts() {
        let root = tempfile::tempdir().expect("root");
        let dir = backup_dir(root.path(), "satchel-20260910-120000", 4096, 1500);
        std::fs::remove_file(dir.join(satchel_core::backup::BACKUP_MANIFEST)).expect("manifest");

        let entries = list_backups(root.path().to_string_lossy().to_string())
            .await
            .expect("listing");

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].message_count, 0);
        assert_eq!(entries[0].search_index_ok, None);
        assert_eq!(entries[0].bytes, 4096);
    }

    /// The check the listing gave up has to still guard the restore.
    #[tokio::test]
    async fn a_corrupt_backup_is_refused_before_a_restore() {
        let directory = tempfile::tempdir().expect("dir");
        let db = directory.path().join("satchel.db");
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(&db)
                    .create_if_missing(true),
            )
            .await
            .expect("file pool");
        sqlx::migrate!("../crates/satchel-core/src/db/migrations")
            .run(&pool)
            .await
            .expect("migrations");
        pool.close().await;

        let mut bytes = std::fs::read(&db).expect("read backup");
        // Page 2's type byte. An unknown value makes the page unreadable.
        bytes[4096] = 0x7f;
        std::fs::write(&db, &bytes).expect("tamper");

        let error = INTEGRITY_CHECKS
            .scope(std::cell::Cell::new(0), async {
                let error = inspect_database(&db).await.expect_err("corrupt backup");
                assert!(
                    INTEGRITY_CHECKS.with(|count| count.get()) >= 1,
                    "the restore path skipped the integrity check"
                );
                error
            })
            .await;
        assert!(!error.is_empty(), "a refusal needs a reason");
    }

    async fn seed_exportable_messages(pool: &SqlitePool, count: usize) {
        sqlx::query(
            "INSERT INTO accounts
             (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
             VALUES ('account', 'Account', 'account@example.com', 'imap', '{}', 'ref', '', '')",
        )
        .execute(pool)
        .await
        .expect("account");
        sqlx::query(
            "INSERT INTO folders
             (id, account_id, name, full_path, folder_type, created_at, updated_at)
             VALUES ('folder', 'account', 'Archive', 'Archive', 'archive', '', '')",
        )
        .execute(pool)
        .await
        .expect("folder");

        let mut tx = pool.begin().await.expect("transaction");
        for index in 0..count {
            sqlx::query(
                "INSERT INTO messages
                 (id, account_id, folder_id, from_address, from_name, to_addresses, subject,
                  date, body_text, created_at, updated_at)
                 VALUES (?, 'account', 'folder', 'sender@example.test', 'Sender',
                         '[\"rcpt@example.test\"]', ?, ?, ?, '', '')",
            )
            .bind(format!("message-{index:05}"))
            .bind(format!("Subject {index}"))
            .bind(format!(
                "2026-01-01T00:{:02}:{:02}Z",
                index / 60,
                index % 60
            ))
            .bind(format!("Body {index}"))
            .execute(&mut *tx)
            .await
            .expect("message");
        }
        tx.commit().await.expect("commit");
    }

    /// 2500 messages must not be one `fetch_all`: five full pages and the
    /// closing one that comes back empty.
    #[tokio::test]
    async fn exporting_pages_the_message_table() {
        let pool = migrated_pool().await;
        seed_exportable_messages(&pool, 2500).await;
        let target = tempfile::tempdir().expect("dir");
        let path = target.path().join("export.mbox");

        let result = EXPORT_PAGE_QUERIES
            .scope(std::cell::Cell::new(0), async {
                let result = write_mailbox_export(&pool, &path.to_string_lossy(), None)
                    .await
                    .expect("export");
                assert_eq!(EXPORT_PAGE_QUERIES.with(|count| count.get()), 6);
                result
            })
            .await;

        assert_eq!(result.message_count, 2500);
        let written = std::fs::metadata(&path).expect("export file").len();
        assert_eq!(written, result.bytes_written);
    }

    /// A page boundary must not drop, repeat or reorder a message.
    #[tokio::test]
    async fn paging_covers_every_message_exactly_once() {
        let pool = migrated_pool().await;
        let total = EXPORT_PAGE_ROWS as usize * 2 + 1;
        seed_exportable_messages(&pool, total).await;
        let target = tempfile::tempdir().expect("dir");
        let path = target.path().join("export.mbox");

        write_mailbox_export(&pool, &path.to_string_lossy(), None)
            .await
            .expect("export");

        let written = std::fs::read_to_string(&path).expect("export file");
        let subjects: Vec<&str> = written
            .lines()
            .filter(|line| line.starts_with("Subject: "))
            .collect();
        assert_eq!(subjects.len(), total);
        for (index, subject) in subjects.iter().enumerate() {
            assert_eq!(
                *subject,
                format!("Subject: Subject {index}"),
                "at row {index}"
            );
        }
    }

    /// The mbox bytes are a compatibility surface: this pins them.
    #[tokio::test]
    async fn the_mbox_output_is_byte_for_byte_what_it_was() {
        let pool = migrated_pool().await;
        seed_exportable_messages(&pool, 2).await;
        sqlx::query(
            "UPDATE messages SET body_text = 'From the desk of Sender' WHERE id = 'message-00001'",
        )
        .execute(&pool)
        .await
        .expect("body with a From line");
        let target = tempfile::tempdir().expect("dir");
        let path = target.path().join("export.mbox");

        write_mailbox_export(&pool, &path.to_string_lossy(), None)
            .await
            .expect("export");

        let written = std::fs::read_to_string(&path).expect("export file");
        assert_eq!(
            written,
            concat!(
                "From sender@example.test Thu Jan  1 00:00:00 2026\n",
                "From: \"Sender\" <sender@example.test>\n",
                "To: rcpt@example.test\n",
                "Subject: Subject 0\n",
                "Date: Thu, 1 Jan 2026 00:00:00 +0000\n",
                "MIME-Version: 1.0\n",
                "Content-Type: text/plain; charset=UTF-8\n",
                "\n",
                "Body 0\n",
                "\n",
                "From sender@example.test Thu Jan  1 00:00:01 2026\n",
                "From: \"Sender\" <sender@example.test>\n",
                "To: rcpt@example.test\n",
                "Subject: Subject 1\n",
                "Date: Thu, 1 Jan 2026 00:00:01 +0000\n",
                "MIME-Version: 1.0\n",
                "Content-Type: text/plain; charset=UTF-8\n",
                "\n",
                ">From the desk of Sender\n",
                "\n",
            )
        );
    }

    #[tokio::test]
    async fn export_round_trips_html_and_binary_attachment() {
        let pool = migrated_pool().await;
        seed_exportable_messages(&pool, 1).await;
        sqlx::query("UPDATE messages SET body_html = '<p>Hello <b>HTML</b><img src=\"cid:logo\"></p>' WHERE id = 'message-00000'")
            .execute(&pool).await.expect("html");
        let target = tempfile::tempdir().expect("dir");
        let cached = crate::commands::attachments::attachment_cache_path(
            target.path(),
            "attachment",
            "report.pdf",
        );
        std::fs::write(&cached, b"%PDF-1.7\0binary").expect("cached attachment");
        let inline = crate::commands::attachments::attachment_cache_path(
            target.path(),
            "inline",
            "logo.png",
        );
        std::fs::write(&inline, b"\x89PNG\r\n").expect("cached inline image");
        sqlx::query("INSERT INTO attachments (id, message_id, filename, content_type, size, is_inline, cached_path, created_at)
            VALUES ('attachment', 'message-00000', 'report.pdf', 'application/pdf', 15, 0, ?, '')")
            .bind(cached.to_string_lossy().to_string())
            .execute(&pool).await.expect("attachment row");
        sqlx::query("INSERT INTO attachments (id, message_id, filename, content_type, size, content_id, is_inline, cached_path, created_at)
            VALUES ('inline', 'message-00000', 'logo.png', 'image/png', 6, 'logo', 1, ?, '')")
            .bind(inline.to_string_lossy().to_string())
            .execute(&pool).await.expect("inline row");
        let path = target.path().join("export.mbox");

        write_mailbox_export(&pool, &path.to_string_lossy(), None)
            .await
            .expect("export");

        let exported = std::fs::read_to_string(path).expect("exported mbox");
        let parsed =
            crate::providers::imap::parse_raw_rfc822(strip_mbox_from_line(&exported).as_bytes())
                .expect("parse exported MIME");
        assert_eq!(parsed.date_rfc3339, "2026-01-01T00:00:00+00:00");
        assert_eq!(parsed.body_text.as_deref(), Some("Body 0"));
        assert_eq!(
            parsed.body_html.as_deref(),
            Some("<p>Hello <b>HTML</b><img src=\"cid:logo\"></p>")
        );
        assert_eq!(parsed.attachments.len(), 2);
        let pdf = parsed
            .attachments
            .iter()
            .find(|part| part.filename.as_deref() == Some("report.pdf"))
            .expect("PDF part");
        assert_eq!(pdf.bytes, b"%PDF-1.7\0binary");
        let logo = parsed
            .attachments
            .iter()
            .find(|part| part.filename.as_deref() == Some("logo.png"))
            .expect("inline part");
        assert_eq!(logo.content_id.as_deref(), Some("logo"));
        assert_eq!(logo.bytes, b"\x89PNG\r\n");
    }

    #[tokio::test]
    async fn export_refuses_a_message_with_uncached_attachment_bytes() {
        let pool = migrated_pool().await;
        seed_exportable_messages(&pool, 1).await;
        sqlx::query("INSERT INTO attachments (id, message_id, filename, content_type, is_inline, created_at)
            VALUES ('attachment', 'message-00000', 'missing.pdf', 'application/pdf', 0, '')")
            .execute(&pool).await.expect("attachment row");
        let target = tempfile::tempdir().expect("dir");
        let path = target.path().join("export.mbox");
        std::fs::write(&path, b"previous complete export").expect("existing export");

        let error = write_mailbox_export(&pool, &path.to_string_lossy(), None)
            .await
            .expect_err("missing bytes must be reported");
        let error: CommandError = serde_json::from_str(&error).expect("localized error");
        assert_eq!(error.key, "backend.errors.exportAttachmentUnavailable");
        assert_eq!(error.params["filename"], serde_json::json!("missing.pdf"));
        assert_eq!(
            std::fs::read(&path).expect("existing export remains"),
            b"previous complete export",
            "an incomplete export must not replace a complete one"
        );
    }

    #[tokio::test]
    async fn html_only_mail_exports_as_html() {
        let pool = migrated_pool().await;
        seed_exportable_messages(&pool, 1).await;
        sqlx::query("UPDATE messages SET body_text = NULL, body_html = '<p>Only HTML</p>' WHERE id = 'message-00000'")
            .execute(&pool).await.expect("html body");
        let target = tempfile::tempdir().expect("dir");
        let path = target.path().join("html.mbox");

        write_mailbox_export(&pool, &path.to_string_lossy(), None)
            .await
            .expect("export");

        let exported = std::fs::read_to_string(path).expect("mbox");
        let parsed =
            crate::providers::imap::parse_raw_rfc822(strip_mbox_from_line(&exported).as_bytes())
                .expect("parsed mail");
        assert_eq!(parsed.body_html.as_deref(), Some("<p>Only HTML</p>"));
        assert_ne!(parsed.body_text.as_deref(), Some("<p>Only HTML</p>"));
    }

    #[tokio::test]
    async fn export_refuses_missing_attachment_metadata() {
        let pool = migrated_pool().await;
        seed_exportable_messages(&pool, 1).await;
        sqlx::query("UPDATE messages SET has_attachments = 1 WHERE id = 'message-00000'")
            .execute(&pool)
            .await
            .expect("attachment flag");
        let target = tempfile::tempdir().expect("dir");
        let path = target.path().join("incomplete.mbox");

        let error = write_mailbox_export(&pool, &path.to_string_lossy(), None)
            .await
            .expect_err("missing attachment metadata must be reported");
        let error: CommandError = serde_json::from_str(&error).expect("localized error");
        assert_eq!(error.key, "backend.errors.exportAttachmentListUnavailable");
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn export_includes_local_draft_attachment_payloads() {
        let pool = migrated_pool().await;
        seed_exportable_messages(&pool, 1).await;
        sqlx::query("UPDATE messages SET is_draft = 1, draft_attachments_json = ? WHERE id = 'message-00000'")
            .bind(serde_json::json!([{
                "filename": "notes.txt",
                "mime_type": "text/plain",
                "content_base64": "Tm90ZXM=",
                "inline": false
            }]).to_string())
            .execute(&pool).await.expect("draft payload");
        let target = tempfile::tempdir().expect("dir");
        let path = target.path().join("draft.mbox");

        write_mailbox_export(&pool, &path.to_string_lossy(), None)
            .await
            .expect("export");

        let exported = std::fs::read_to_string(path).expect("mbox");
        let parsed =
            crate::providers::imap::parse_raw_rfc822(strip_mbox_from_line(&exported).as_bytes())
                .expect("parsed draft");
        assert_eq!(parsed.attachments.len(), 1);
        assert_eq!(parsed.attachments[0].bytes, b"Notes");
    }

    /// Drive the reader to exhaustion: the entries, the largest one handed out
    /// and how many were dropped.
    async fn collect_entries(raw: &[u8], ceiling: usize) -> (Vec<String>, usize, usize) {
        let mut entries = MboxEntries::with_ceiling(tokio::io::BufReader::new(raw), ceiling);
        let mut out = Vec::new();
        let mut largest = 0usize;
        while let Some(entry) = entries.next_entry().await.expect("entry") {
            largest = largest.max(entry.len());
            out.push(entry);
        }
        (out, largest, entries.dropped)
    }

    /// A 64-MiB mbox must never be in memory at once — the old importer held it
    /// twice. The largest buffer is one message.
    #[tokio::test]
    async fn a_large_mbox_is_read_one_message_at_a_time() {
        let body = "x".repeat(60 * 1024);
        let mut raw = Vec::new();
        let mut expected = 0usize;
        while raw.len() < 64 * 1024 * 1024 {
            raw.extend_from_slice(
                format!(
                    "From sender@example.test Thu Jan  1 00:00:00 2026\nSubject: {expected}\n\n{body}\n\n"
                )
                .as_bytes(),
            );
            expected += 1;
        }

        let (entries, largest, dropped) = collect_entries(&raw, MAX_MESSAGE_BYTES).await;

        assert_eq!(entries.len(), expected);
        assert_eq!(dropped, 0);
        assert!(
            largest < 128 * 1024,
            "one entry should be one message, was {largest} bytes"
        );
    }

    /// An entry above the ceiling is dropped; the ones around it still arrive.
    #[tokio::test]
    async fn an_entry_over_the_ceiling_is_dropped() {
        let raw = format!(
            "From a@example.test Thu Jan  1 00:00:00 2026\nSubject: small\n\nok\n\n\
             From b@example.test Thu Jan  1 00:00:01 2026\nSubject: big\n\n{}\n\n\
             From c@example.test Thu Jan  1 00:00:02 2026\nSubject: also small\n\nok\n",
            "x".repeat(4096)
        );

        let (entries, largest, dropped) = collect_entries(raw.as_bytes(), 512).await;

        assert_eq!(entries.len(), 2);
        assert!(entries[0].contains("Subject: small"));
        assert!(entries[1].contains("Subject: also small"));
        assert_eq!(dropped, 1);
        assert!(largest <= 512);
    }

    /// A bare `.eml` has no `From ` line; it is one message, not none.
    #[tokio::test]
    async fn a_file_without_a_from_line_is_one_message() {
        let raw = "From: Alice <alice@example.test>\nSubject: Hello\n\nBody\n";

        let (entries, _, dropped) = collect_entries(raw.as_bytes(), MAX_MESSAGE_BYTES).await;

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0], raw);
        assert_eq!(dropped, 0);
    }

    /// Content before the first separator is not a message.
    #[tokio::test]
    async fn a_preamble_before_the_first_separator_is_dropped() {
        let raw = concat!(
            "Exported by something else\n\n",
            "From alice@example.test Thu Jan  1 00:00:00 2026\n",
            "Subject: Real\n\nBody\n"
        );

        let (entries, _, dropped) = collect_entries(raw.as_bytes(), MAX_MESSAGE_BYTES).await;

        assert_eq!(entries.len(), 1);
        assert!(entries[0].contains("Subject: Real"));
        assert_eq!(dropped, 0);
    }
}
