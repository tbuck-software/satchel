use std::time::Instant;

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::commands::error::{CommandError, CommandResult};
use crate::db::message_links;
use crate::db::threads::{
    SmartInboxBucketRecord, SmartInboxPreviewRecord, SmartInboxSummaryRecord,
    ThreadCursor as CoreThreadCursor, ThreadError, ThreadListRequest,
    ThreadOrganizationUpdateRequest, ThreadPageRecord, ThreadRecord, ThreadRepository,
    ThreadTagSetRequest,
};
use crate::workflow::states::WorkflowState;
use crate::AppState;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/threads/"))]
pub struct ThreadDto {
    pub id: String,
    pub account_id: String,
    pub subject: String,
    pub normalized_subject: String,
    pub snippet: String,
    pub last_message_date: String,
    pub sort_date: Option<String>,
    pub message_count: u32,
    pub workflow_state: WorkflowState,
    pub unread_count: u32,
    pub has_attachments: bool,
    pub has_draft: bool,
    pub draft_snippet: Option<String>,
    pub priority: u8,
    pub board_position: Option<f64>,
    pub tags: Vec<String>,
    pub assignee: Option<String>,
    pub shared_with: Vec<String>,
    pub participants: String,
    pub sent_participants: String,
    pub mailbox_names: String,
    pub category: Option<String>,
    pub smart_bucket: String,
    pub is_muted: bool,
    pub is_starred: bool,
    pub snooze_until: Option<String>,
    pub sender_known: bool,
    pub has_invite: bool,
    pub invite_status: Option<String>,
    pub invite_cancelled: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/threads/"))]
pub struct ListThreadsInput {
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub contact_email: Option<String>,
    #[serde(default)]
    pub mailbox_kind: Option<String>,
    #[serde(default)]
    pub mailbox_value: Option<String>,
    #[serde(default)]
    pub workflow_state: Option<String>,
    /// Focused-list bucket. Applied server-side so opening a group can page
    /// through all of its threads instead of only the current inbox page.
    #[serde(default)]
    pub smart_bucket: Option<String>,
    /// Keyset cursor from a previous page. `None` starts at the newest thread.
    #[serde(default)]
    pub cursor: Option<ThreadCursor>,
    /// Fetch the nearest newer rows in descending order.
    pub before_cursor: Option<ThreadCursor>,
    /// Page size. Clamped to 1..=1000; defaults to the repository page size.
    #[serde(default)]
    pub limit: Option<i64>,
}

/// Keyset cursor over `(threads.last_message_date DESC, threads.id DESC)`.
///
/// Offset pagination would skip or duplicate rows whenever a sync inserts a
/// thread between two page loads, which for a live mailbox is constant.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/threads/"))]
pub struct ThreadCursor {
    pub last_message_date: String,
    pub id: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/threads/"))]
pub struct ThreadPage {
    pub threads: Vec<ThreadDto>,
    /// Pass back as `cursor` to fetch the next page. `None` = end of list.
    pub next_cursor: Option<ThreadCursor>,
    pub has_more: bool,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/threads/"))]
pub struct SmartInboxPreviewDto {
    pub id: String,
    pub sender_name: String,
    pub sender_email: String,
    pub subject: String,
    pub unread_count: u32,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/threads/"))]
pub struct SmartInboxBucketDto {
    pub key: String,
    pub count: u32,
    pub unread_count: u32,
    pub preview_items: Vec<SmartInboxPreviewDto>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/threads/"))]
pub struct SmartInboxSummaryDto {
    pub total_count: u32,
    pub buckets: Vec<SmartInboxBucketDto>,
}

impl From<ThreadCursor> for CoreThreadCursor {
    fn from(cursor: ThreadCursor) -> Self {
        Self {
            last_message_date: cursor.last_message_date,
            id: cursor.id,
        }
    }
}

impl From<CoreThreadCursor> for ThreadCursor {
    fn from(cursor: CoreThreadCursor) -> Self {
        Self {
            last_message_date: cursor.last_message_date,
            id: cursor.id,
        }
    }
}

impl From<ListThreadsInput> for ThreadListRequest {
    fn from(input: ListThreadsInput) -> Self {
        Self {
            account_id: input.account_id,
            contact_email: input.contact_email,
            mailbox_kind: input.mailbox_kind,
            mailbox_value: input.mailbox_value,
            workflow_state: input.workflow_state,
            smart_bucket: input.smart_bucket,
            cursor: input.cursor.map(Into::into),
            before_cursor: input.before_cursor.map(Into::into),
            limit: input.limit,
        }
    }
}

impl From<ThreadRecord> for ThreadDto {
    fn from(record: ThreadRecord) -> Self {
        Self {
            id: record.id,
            account_id: record.account_id,
            normalized_subject: crate::email::subject::normalized_subject(&record.subject),
            subject: record.subject,
            snippet: record.snippet,
            last_message_date: record.last_message_date,
            sort_date: record.sort_date,
            message_count: record.message_count,
            workflow_state: record.workflow_state,
            unread_count: record.unread_count,
            has_attachments: record.has_attachments,
            has_draft: record.has_draft,
            draft_snippet: record.draft_snippet,
            priority: record.priority,
            board_position: record.board_position,
            tags: record.tags,
            assignee: record.assignee,
            shared_with: record.shared_with,
            participants: record.participants,
            sent_participants: record.sent_participants,
            mailbox_names: record.mailbox_names,
            category: record.category,
            smart_bucket: record.smart_bucket,
            is_muted: record.is_muted,
            is_starred: record.is_starred,
            snooze_until: record.snooze_until,
            sender_known: record.sender_known,
            has_invite: record.has_invite,
            invite_status: record.invite_status,
            invite_cancelled: record.invite_cancelled,
        }
    }
}

impl From<ThreadPageRecord> for ThreadPage {
    fn from(page: ThreadPageRecord) -> Self {
        Self {
            threads: page.threads.into_iter().map(Into::into).collect(),
            next_cursor: page.next_cursor.map(Into::into),
            has_more: page.has_more,
        }
    }
}

impl From<SmartInboxPreviewRecord> for SmartInboxPreviewDto {
    fn from(record: SmartInboxPreviewRecord) -> Self {
        Self {
            id: record.id,
            sender_name: record.sender_name,
            sender_email: record.sender_email,
            subject: crate::email::subject::normalized_subject(&record.subject),
            unread_count: record.unread_count,
        }
    }
}

impl From<SmartInboxBucketRecord> for SmartInboxBucketDto {
    fn from(record: SmartInboxBucketRecord) -> Self {
        Self {
            key: record.key,
            count: record.count,
            unread_count: record.unread_count,
            preview_items: record.preview_items.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<SmartInboxSummaryRecord> for SmartInboxSummaryDto {
    fn from(record: SmartInboxSummaryRecord) -> Self {
        Self {
            total_count: record.total_count,
            buckets: record.buckets.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<UpdateThreadOrganizationInput> for ThreadOrganizationUpdateRequest {
    fn from(input: UpdateThreadOrganizationInput) -> Self {
        Self {
            thread_id: input.thread_id,
            board_position: input.board_position,
            assignee: input.assignee,
            shared_with: input.shared_with,
        }
    }
}

impl From<SetThreadTagsInput> for ThreadTagSetRequest {
    fn from(input: SetThreadTagsInput) -> Self {
        Self {
            thread_ids: input.thread_ids,
            tags: input.tags,
        }
    }
}

impl From<ThreadError> for CommandError {
    fn from(error: ThreadError) -> Self {
        match error {
            ThreadError::NotFound => Self::new("backend.errors.threadNotFound"),
            ThreadError::EmptyThreadId
            | ThreadError::InvalidCategory
            | ThreadError::InvalidPriority
            | ThreadError::NonFiniteBoardPosition
            | ThreadError::TooManyMetadataValues { .. }
            | ThreadError::MetadataValueTooLong { .. }
            | ThreadError::MetadataControlCharacters { .. } => {
                tracing::warn!(error = %error, "invalid thread metadata");
                Self::new("backend.errors.threadMetadataInvalid")
            }
            ThreadError::InvalidSmartInboxBucket { value } => {
                tracing::error!(bucket = %value, "invalid smart inbox bucket");
                Self::new("backend.errors.threadSmartBucketInvalid")
            }
            ThreadError::Persistence(error) => {
                tracing::error!(error = ?error, "thread persistence failed");
                Self::new("backend.errors.threadPersistenceFailed")
            }
            ThreadError::Queue(error) => {
                tracing::error!(error = %error, "thread mutation queue failed");
                Self::new("backend.errors.threadPersistenceFailed")
            }
            ThreadError::SnapshotSerialization(error) => {
                tracing::error!(error = ?error, "thread-list snapshot serialization failed");
                Self::new("backend.errors.threadPersistenceFailed")
            }
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/threads/", optional_fields = nullable))]
pub struct ThreadIdsInput {
    pub thread_ids: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/threads/", optional_fields = nullable))]
pub struct UpdateThreadsReadInput {
    pub thread_ids: Vec<String>,
    pub is_read: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/threads/", optional_fields = nullable))]
pub struct UpdateThreadOrganizationInput {
    pub thread_id: String,
    #[serde(default)]
    pub board_position: Option<f64>,
    #[serde(default)]
    pub assignee: Option<String>,
    #[serde(default)]
    pub shared_with: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/threads/", optional_fields = nullable))]
pub struct SetThreadTagsInput {
    pub thread_ids: Vec<String>,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/threads/"))]
pub struct UnreadRepairDto {
    pub messages_updated: u64,
    pub threads_updated: u64,
}

#[tauri::command]
pub async fn list_threads(
    pool: tauri::State<'_, crate::db::ReadPool>,
    write_pool: tauri::State<'_, SqlitePool>,
    input: Option<ListThreadsInput>,
) -> CommandResult<ThreadPage> {
    let input = input.unwrap_or_default();
    let page = ThreadRepository::with_pools(pool.inner().as_ref(), write_pool.inner())
        .list(input.clone().into())
        .await?;
    Ok(page.into())
}

#[tauri::command]
pub async fn list_cached_threads(
    pool: tauri::State<'_, crate::db::ReadPool>,
    write_pool: tauri::State<'_, SqlitePool>,
    input: Option<ListThreadsInput>,
) -> CommandResult<Option<ThreadPage>> {
    let page = ThreadRepository::with_pools(pool.inner().as_ref(), write_pool.inner())
        .list_cached(input.unwrap_or_default().into())
        .await?;
    Ok(page.map(Into::into))
}

#[tauri::command]
pub async fn list_smart_inbox_summary(
    pool: tauri::State<'_, crate::db::ReadPool>,
    account_id: Option<String>,
) -> CommandResult<SmartInboxSummaryDto> {
    let summary = ThreadRepository::new(pool.inner().as_ref())
        .smart_inbox_summary(account_id.as_deref())
        .await?;
    Ok(summary.into())
}

#[tauri::command]
pub async fn get_thread(
    pool: tauri::State<'_, crate::db::ReadPool>,
    thread_id: String,
) -> CommandResult<ThreadDto> {
    let record = ThreadRepository::new(pool.inner().as_ref())
        .get(&thread_id)
        .await?;
    Ok(record.into())
}

#[tauri::command]
pub async fn get_threads(
    pool: tauri::State<'_, crate::db::ReadPool>,
    thread_ids: Vec<String>,
) -> CommandResult<Vec<ThreadDto>> {
    Ok(ThreadRepository::new(pool.inner().as_ref())
        .get_many(&thread_ids)
        .await?
        .into_iter()
        .map(Into::into)
        .collect())
}

/// What a `satchel://` link to this thread is built from.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/threads/"))]
pub struct MessageLinkTargetDto {
    pub thread_id: String,
    pub subject: String,
    pub message_id: Option<String>,
}

#[tauri::command]
pub async fn message_link_target(
    pool: tauri::State<'_, crate::db::ReadPool>,
    thread_id: String,
) -> CommandResult<MessageLinkTargetDto> {
    let target = message_links::link_target(pool.inner().as_ref(), &thread_id).await?;
    Ok(MessageLinkTargetDto {
        thread_id: target.thread_id,
        subject: target.subject,
        message_id: target.message_id,
    })
}

#[tauri::command]
pub async fn resolve_message_link(
    pool: tauri::State<'_, crate::db::ReadPool>,
    message_id: String,
) -> CommandResult<Option<String>> {
    Ok(message_links::resolve_message_id(pool.inner().as_ref(), &message_id).await?)
}

#[tauri::command]
pub async fn update_thread_organization(
    pool: tauri::State<'_, SqlitePool>,
    input: UpdateThreadOrganizationInput,
) -> CommandResult<()> {
    ThreadRepository::new(pool.inner())
        .update_organization(input.into())
        .await?;
    Ok(())
}

#[tauri::command]
pub async fn set_thread_tags(
    pool: tauri::State<'_, SqlitePool>,
    input: SetThreadTagsInput,
) -> CommandResult<()> {
    ThreadRepository::new(pool.inner())
        .set_tags(input.into())
        .await?;
    Ok(())
}

/// Archiving has no IPC command: the frontend reaches it through
/// `move_threads`, and the notification quick action calls this directly.
#[cfg(any(target_os = "macos", test))]
pub(crate) async fn archive_threads_inner(
    pool: &SqlitePool,
    state: &AppState,
    input: ThreadIdsInput,
) -> CommandResult<Vec<String>> {
    satchel_core::mail_actions::move_threads(
        pool,
        &state.queue,
        &normalize_thread_ids(input.thread_ids),
        satchel_core::mail_actions::MoveTarget::Archive,
    )
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn trash_threads(
    pool: tauri::State<'_, SqlitePool>,
    state: tauri::State<'_, AppState>,
    input: ThreadIdsInput,
) -> CommandResult<satchel_core::mail_actions::TrashReceipt> {
    satchel_core::mail_actions::trash_threads(
        pool.inner(),
        &state.queue,
        &normalize_thread_ids(input.thread_ids),
    )
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn undo_trash(
    pool: tauri::State<'_, SqlitePool>,
    state: tauri::State<'_, AppState>,
    receipt_id: String,
) -> CommandResult<()> {
    satchel_core::mail_actions::undo::undo_trash(pool.inner(), &state.queue, &receipt_id).await?;
    Ok(())
}

#[tauri::command]
pub async fn set_threads_read(
    pool: tauri::State<'_, SqlitePool>,
    state: tauri::State<'_, AppState>,
    input: UpdateThreadsReadInput,
) -> CommandResult<()> {
    set_threads_read_command_inner(pool.inner(), state.inner(), input)
        .await
        .map(|_| ())
}

pub(crate) async fn set_threads_read_command_inner(
    pool: &SqlitePool,
    state: &AppState,
    input: UpdateThreadsReadInput,
) -> CommandResult<Vec<String>> {
    set_threads_read_command_inner_with_queue_prefix(pool, state, input, None).await
}

#[cfg(target_os = "android")]
pub(crate) async fn set_threads_read_inner_with_queue_prefix(
    pool: &SqlitePool,
    state: &AppState,
    input: UpdateThreadsReadInput,
    queue_prefix: Option<&str>,
) -> Result<Vec<String>, String> {
    set_threads_read_command_inner_with_queue_prefix(pool, state, input, queue_prefix)
        .await
        .map_err(|error| error.encoded())
}

async fn set_threads_read_command_inner_with_queue_prefix(
    pool: &SqlitePool,
    state: &AppState,
    input: UpdateThreadsReadInput,
    queue_prefix: Option<&str>,
) -> CommandResult<Vec<String>> {
    let started_at = Instant::now();
    let thread_ids = normalize_thread_ids(input.thread_ids);
    if thread_ids.is_empty() {
        return Ok(Vec::new());
    }

    let queued_ids = satchel_core::mail_actions::set_flags(
        pool,
        &state.queue,
        &thread_ids,
        Some(input.is_read),
        None,
        queue_prefix,
    )
    .await?;

    log_perf(
        "set_threads_read",
        started_at,
        format!("thread_ids={} is_read={}", thread_ids.len(), input.is_read),
    );
    Ok(queued_ids)
}

/// One-shot repair of a Gmail unread-state drift, run by the startup
/// maintenance worker. It short-circuits once `repair.unread_state_v2_at` is
/// set, so there is nothing a manual trigger could still do.
pub async fn repair_unread_state_inner(pool: &SqlitePool) -> Result<UnreadRepairDto, String> {
    repair_unread_state_command_inner(pool)
        .await
        .map_err(|error| error.encoded())
}

async fn repair_unread_state_command_inner(pool: &SqlitePool) -> CommandResult<UnreadRepairDto> {
    let now = chrono::Utc::now().to_rfc3339();
    let mut messages_updated = 0_u64;
    let mut conn = pool.acquire().await.map_err(|err| err.to_string())?;

    let already_repaired = sqlx::query_scalar::<_, String>(
        "SELECT value FROM app_settings WHERE key = 'repair.unread_state_v2_at'",
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(|err| err.to_string())?
    .map(|value| !value.trim().is_empty())
    .unwrap_or(false);
    if already_repaired {
        return Ok(UnreadRepairDto {
            messages_updated: 0,
            threads_updated: 0,
        });
    }

    sqlx::query(
        "CREATE TEMP TABLE IF NOT EXISTS satchel_gmail_unread_refs (
            account_id TEXT NOT NULL,
            remote_id TEXT NOT NULL,
            PRIMARY KEY (account_id, remote_id)
         )",
    )
    .execute(&mut *conn)
    .await
    .map_err(|err| err.to_string())?;
    sqlx::query("DELETE FROM satchel_gmail_unread_refs")
        .execute(&mut *conn)
        .await
        .map_err(|err| err.to_string())?;
    sqlx::query(
        "INSERT OR IGNORE INTO satchel_gmail_unread_refs (account_id, remote_id)
         SELECT DISTINCT unread_messages.account_id, TRIM(unread_messages.remote_id)
         FROM messages unread_messages
         JOIN accounts ON accounts.id = unread_messages.account_id
         JOIN folders unread_folders ON unread_folders.id = unread_messages.folder_id
         WHERE accounts.provider = 'gmail'
           AND TRIM(COALESCE(unread_messages.remote_id, '')) != ''
           AND (
             UPPER(COALESCE(unread_folders.remote_id, '')) = 'UNREAD'
             OR UPPER(COALESCE(unread_folders.name, '')) = 'UNREAD'
             OR UPPER(COALESCE(unread_folders.full_path, '')) = 'UNREAD'
           )",
    )
    .execute(&mut *conn)
    .await
    .map_err(|err| err.to_string())?;

    // The UPDATE below reads the absence of a reference as "this message is
    // read", so it is only meaningful while the UNREAD label is actually
    // synced. Gmail no longer syncs it (`gmail_label_is_state_view`), and on a
    // database where the repair never ran an empty reference table would mark
    // the entire Gmail mailbox as read in one statement.
    let unread_refs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM satchel_gmail_unread_refs")
        .fetch_one(&mut *conn)
        .await
        .map_err(|err| err.to_string())?;
    if unread_refs == 0 {
        return Ok(UnreadRepairDto {
            messages_updated: 0,
            threads_updated: 0,
        });
    }

    let gmail = sqlx::query(
        "UPDATE messages
         SET is_read = CASE
             WHEN EXISTS (
                SELECT 1 FROM satchel_gmail_unread_refs unread_refs
                WHERE unread_refs.account_id = messages.account_id
                  AND unread_refs.remote_id = TRIM(messages.remote_id)
             )
             THEN 0 ELSE 1 END,
             updated_at = ?
         WHERE account_id IN (SELECT id FROM accounts WHERE provider = 'gmail')
           AND TRIM(COALESCE(remote_id, '')) != ''
           AND is_read != CASE
             WHEN EXISTS (
                SELECT 1 FROM satchel_gmail_unread_refs unread_refs
                WHERE unread_refs.account_id = messages.account_id
                  AND unread_refs.remote_id = TRIM(messages.remote_id)
             )
             THEN 0 ELSE 1 END",
    )
    .bind(&now)
    .execute(&mut *conn)
    .await
    .map_err(|err| err.to_string())?;
    messages_updated += gmail.rows_affected();

    let done = sqlx::query(
        "UPDATE messages
         SET is_read = 1, updated_at = ?
         WHERE is_read = 0
           AND thread_id IN (SELECT id FROM threads WHERE workflow_state = 'done')",
    )
    .bind(&now)
    .execute(&mut *conn)
    .await
    .map_err(|err| err.to_string())?;
    messages_updated += done.rows_affected();

    let resolved_folders = sqlx::query(
        "UPDATE messages
         SET is_read = 1, updated_at = ?
         WHERE is_read = 0
           AND folder_id IN (
             SELECT id
             FROM folders
             WHERE folder_type IN ('archive', 'sent', 'drafts', 'trash', 'spam')
                OR LOWER(COALESCE(name, '')) LIKE '%archive%'
                OR LOWER(COALESCE(name, '')) LIKE '%archiv%'
                OR LOWER(COALESCE(name, '')) LIKE '%sent%'
                OR LOWER(COALESCE(name, '')) LIKE '%draft%'
                OR LOWER(COALESCE(name, '')) LIKE '%trash%'
                OR LOWER(COALESCE(name, '')) LIKE '%deleted%'
                OR LOWER(COALESCE(name, '')) LIKE '%spam%'
                OR LOWER(COALESCE(name, '')) LIKE '%junk%'
                OR LOWER(COALESCE(name, '')) LIKE '%blocked%'
                OR LOWER(COALESCE(full_path, '')) LIKE '%archive%'
                OR LOWER(COALESCE(full_path, '')) LIKE '%archiv%'
                OR LOWER(COALESCE(full_path, '')) LIKE '%sent%'
                OR LOWER(COALESCE(full_path, '')) LIKE '%draft%'
                OR LOWER(COALESCE(full_path, '')) LIKE '%trash%'
                OR LOWER(COALESCE(full_path, '')) LIKE '%deleted%'
                OR LOWER(COALESCE(full_path, '')) LIKE '%spam%'
                OR LOWER(COALESCE(full_path, '')) LIKE '%junk%'
                OR LOWER(COALESCE(full_path, '')) LIKE '%blocked%'
           )",
    )
    .bind(&now)
    .execute(&mut *conn)
    .await
    .map_err(|err| err.to_string())?;
    messages_updated += resolved_folders.rows_affected();

    sqlx::query("DELETE FROM satchel_gmail_unread_refs")
        .execute(&mut *conn)
        .await
        .map_err(|err| err.to_string())?;

    let thread_ids = if messages_updated > 0 {
        sqlx::query_scalar::<_, String>(
            "SELECT DISTINCT threads.id
             FROM threads
             JOIN messages ON messages.thread_id = threads.id",
        )
        .fetch_all(&mut *conn)
        .await
        .map_err(|err| err.to_string())?
    } else {
        Vec::new()
    };

    // The connection used for the repair must be released before the Core
    // repository starts its own transaction. This also keeps the projection
    // refresh's cleanup (including topic links) atomic.
    drop(conn);
    if !thread_ids.is_empty() {
        refresh_thread_projection_for_command(pool, &thread_ids).await?;
    }
    let threads_updated = thread_ids.len() as u64;

    let mut conn = pool.acquire().await.map_err(|err| err.to_string())?;

    sqlx::query(
        "INSERT INTO app_settings (key, value, updated_at)
         VALUES ('repair.unread_state_v2_at', ?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
    )
    .bind(&now)
    .bind(&now)
    .execute(&mut *conn)
    .await
    .map_err(|err| err.to_string())?;

    Ok(UnreadRepairDto {
        messages_updated,
        threads_updated,
    })
}

async fn refresh_thread_projection_for_command(
    pool: &SqlitePool,
    thread_ids: &[String],
) -> CommandResult<()> {
    ThreadRepository::new(pool)
        .refresh_projection(thread_ids)
        .await?;
    Ok(())
}

fn log_perf(operation: &str, started_at: Instant, detail: String) {
    let enabled = std::env::var("SATCHEL_PERF_LOG")
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false);
    if !enabled {
        return;
    }

    tracing::info!(
        target: "perf",
        "{} {}ms {}",
        operation,
        started_at.elapsed().as_millis(),
        detail
    );
}

fn normalize_thread_ids(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .fold(Vec::new(), |mut acc, value| {
            if !acc.iter().any(|existing| existing == &value) {
                acc.push(value);
            }
            acc
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_a_normalized_display_subject_without_changing_the_wire_subject() {
        let record = ThreadRecord {
            sort_date: None,
            id: "thread".into(),
            account_id: "account".into(),
            subject: "SV: Odp: Antw: Quarterly Plan".into(),
            snippet: String::new(),
            last_message_date: "2026-10-13".into(),
            message_count: 1,
            workflow_state: WorkflowState::Inbox,
            unread_count: 0,
            has_attachments: false,
            has_draft: false,
            draft_snippet: None,
            priority: 0,
            board_position: None,
            tags: Vec::new(),
            assignee: None,
            shared_with: Vec::new(),
            participants: "[]".into(),
            sent_participants: "[]".into(),
            mailbox_names: "[]".into(),
            category: None,
            smart_bucket: "personal".into(),
            is_muted: false,
            is_starred: false,
            snooze_until: None,
            sender_known: false,
            has_invite: false,
            invite_status: None,
            invite_cancelled: false,
        };
        let dto = serde_json::to_value(ThreadDto::from(record)).unwrap();
        assert_eq!(dto["subject"], "SV: Odp: Antw: Quarterly Plan");
        assert_eq!(dto["normalized_subject"], "Quarterly Plan");
    }

    #[test]
    fn redacts_thread_repository_persistence_details_at_ipc_boundary() {
        let detail = "near SELECT: syntax error";
        let error = CommandError::from(ThreadError::Persistence(sqlx::Error::Protocol(
            detail.into(),
        )));
        let serialized = serde_json::to_value(error).expect("command error JSON");

        assert_eq!(
            serialized,
            serde_json::json!({ "key": "backend.errors.threadPersistenceFailed" })
        );
        assert!(!serialized.to_string().contains(detail));
    }

    #[test]
    fn maps_thread_domain_errors_to_stable_ipc_keys() {
        assert_eq!(
            CommandError::from(ThreadError::NotFound).key,
            "backend.errors.threadNotFound"
        );
        assert_eq!(
            CommandError::from(ThreadError::EmptyThreadId).key,
            "backend.errors.threadMetadataInvalid"
        );
        assert_eq!(
            CommandError::from(ThreadError::InvalidCategory).key,
            "backend.errors.threadMetadataInvalid"
        );
        assert_eq!(
            CommandError::from(ThreadError::InvalidPriority).key,
            "backend.errors.threadMetadataInvalid"
        );
        assert_eq!(
            CommandError::from(ThreadError::InvalidSmartInboxBucket {
                value: "unknown".into(),
            })
            .key,
            "backend.errors.threadSmartBucketInvalid"
        );
    }

    /*
     * The notification quick action is the only caller of this path, and it
     * used to reach it through a `#[tauri::command]` wrapper that was never
     * registered — so nothing but the wrapper's signature was ever tested.
     */
    #[tokio::test]
    async fn archives_every_thread_a_notification_action_names() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("pool");
        sqlx::migrate!("../crates/satchel-core/src/db/migrations")
            .run(&pool)
            .await
            .expect("migrations");
        sqlx::raw_sql(
            "INSERT INTO accounts
             (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
             VALUES('account', 'Account', 'me@example.test', 'imap', '{}', '', '2026-01-01', '2026-01-01');
             INSERT INTO folders
             (id, account_id, name, full_path, folder_type, created_at, updated_at)
             VALUES('inbox', 'account', 'Inbox', 'INBOX', 'inbox', '2026-01-01', '2026-01-01'),
                   ('archive', 'account', 'Archiv', 'Archive', 'archive', '2026-01-01', '2026-01-01');
             INSERT INTO threads
             (id, account_id, subject, last_message_date, message_count, unread_count,
              created_at, updated_at)
             VALUES('thread-a', 'account', 'A', '2026-01-01', 1, 1, '2026-01-01', '2026-01-01'),
                   ('thread-b', 'account', 'B', '2026-01-01', 1, 1, '2026-01-01', '2026-01-01');
             INSERT INTO messages
             (id, account_id, folder_id, thread_id, from_address, to_addresses, subject,
              date, is_read, created_at, updated_at)
             VALUES('msg-a', 'account', 'inbox', 'thread-a', 'sender@example.test', '[]', 'A',
                    '2026-01-01', 0, '2026-01-01', '2026-01-01'),
                   ('msg-b', 'account', 'inbox', 'thread-b', 'sender@example.test', '[]', 'B',
                    '2026-01-01', 0, '2026-01-01', '2026-01-01');",
        )
        .execute(&pool)
        .await
        .expect("fixture");
        let state = AppState::default();

        archive_threads_inner(
            &pool,
            &state,
            ThreadIdsInput {
                thread_ids: vec!["thread-a".into(), "thread-b".into()],
            },
        )
        .await
        .expect("archive");

        let states: Vec<(String, String)> =
            sqlx::query_as("SELECT id, COALESCE(workflow_state, '') FROM threads ORDER BY id")
                .fetch_all(&pool)
                .await
                .expect("threads");
        assert_eq!(
            states,
            vec![
                ("thread-a".to_string(), "done".to_string()),
                ("thread-b".to_string(), "done".to_string()),
            ]
        );

        let folders: Vec<(String, String)> =
            sqlx::query_as("SELECT id, folder_id FROM messages ORDER BY id")
                .fetch_all(&pool)
                .await
                .expect("messages");
        assert_eq!(
            folders,
            vec![
                ("msg-a".to_string(), "archive".to_string()),
                ("msg-b".to_string(), "archive".to_string()),
            ]
        );
    }

    #[tokio::test]
    async fn projection_failure_uses_the_thread_persistence_key_without_sql_detail() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("pool");
        sqlx::migrate!("../crates/satchel-core/src/db/migrations")
            .run(&pool)
            .await
            .expect("migrations");
        sqlx::raw_sql(
            "INSERT INTO accounts
             (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
             VALUES('account', 'Account', 'me@example.test', 'imap', '{}', '', '2026-01-01', '2026-01-01');
             INSERT INTO folders
             (id, account_id, name, full_path, folder_type, created_at, updated_at)
             VALUES('inbox', 'account', 'Inbox', 'INBOX', 'inbox', '2026-01-01', '2026-01-01');
             INSERT INTO threads
             (id, account_id, subject, last_message_date, message_count, unread_count,
              created_at, updated_at)
             VALUES('thread', 'account', 'Subject', '2026-01-01', 1, 0,
                    '2026-01-01', '2026-01-01');
             INSERT INTO messages
             (id, account_id, folder_id, thread_id, from_address, to_addresses, subject,
              date, is_read, created_at, updated_at)
             VALUES('message', 'account', 'inbox', 'thread', 'sender@example.test', '[]',
                    'Subject', '2026-01-01', 1, '2026-01-01', '2026-01-01');
             CREATE TRIGGER fail_thread_projection BEFORE UPDATE ON threads
             BEGIN SELECT RAISE(ABORT, 'private SQL detail'); END;",
        )
        .execute(&pool)
        .await
        .expect("fixture");

        let error = refresh_thread_projection_for_command(&pool, &["thread".into()])
            .await
            .expect_err("projection failure");
        let serialized = serde_json::to_value(error).expect("command error JSON");

        assert_eq!(
            serialized,
            serde_json::json!({ "key": "backend.errors.threadPersistenceFailed" })
        );
        assert!(!serialized.to_string().contains("private SQL detail"));
    }
}
