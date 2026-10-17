//! Tauri-free thread persistence: list/detail projections and simple state updates.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{FromRow, QueryBuilder, Sqlite, SqlitePool, Transaction};
use thiserror::Error;

use crate::db::sender_preferences::{
    classification_headers, classify_message_with_provider_labels, normalize_category,
};
use crate::db::topics::TopicRepository;
use crate::db::{blocked_senders::BlockedSenderPatternType, followups::reassign_thread_tx};
use crate::providers::traits::MessageRef;
use crate::sync::queue::{QueuedAction, SyncQueue};
use crate::sync::remote_mutations::{
    canonicalize_blocked_message_ref, remote_move_thread_id_from_json, stable_blocked_move_id,
    RemoteMutationPayload,
};
use crate::utils::mime::{
    decode_html_entities, decode_mime_encoded_words, extract_email_address,
    resolve_sender_display_name,
};
use crate::workflow::states::WorkflowState;

#[derive(Debug, Error)]
pub enum ThreadError {
    #[error("thread not found")]
    NotFound,
    #[error("thread id must not be empty")]
    EmptyThreadId,
    #[error("thread category is invalid")]
    InvalidCategory,
    #[error("thread priority must be between 0 and 3")]
    InvalidPriority,
    #[error("thread board position must be finite")]
    NonFiniteBoardPosition,
    #[error("{field} contains more than {max} values")]
    TooManyMetadataValues { field: &'static str, max: usize },
    #[error("{field} contains a value longer than {max} characters")]
    MetadataValueTooLong { field: &'static str, max: usize },
    #[error("{field} contains control characters")]
    MetadataControlCharacters { field: &'static str },
    #[error("invalid smart inbox bucket: {value}")]
    InvalidSmartInboxBucket { value: String },
    #[error("thread persistence failed")]
    Persistence(#[source] sqlx::Error),
    #[error("thread mutation queue failed: {0}")]
    Queue(String),
    #[error("thread-list snapshot serialization failed")]
    SnapshotSerialization(#[source] serde_json::Error),
}

impl From<sqlx::Error> for ThreadError {
    fn from(error: sqlx::Error) -> Self {
        Self::Persistence(error)
    }
}

impl From<serde_json::Error> for ThreadError {
    fn from(error: serde_json::Error) -> Self {
        Self::SnapshotSerialization(error)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "core/db/threads/"))]
pub struct ThreadRecord {
    pub id: String,
    pub account_id: String,
    pub subject: String,
    pub snippet: String,
    pub last_message_date: String,
    /// Stored keyset date, distinct from the latest displayed message date.
    #[serde(default)]
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
    /// Same classification used by smart-folder summaries and list filters.
    pub smart_bucket: String,
    pub is_muted: bool,
    pub is_starred: bool,
    pub snooze_until: Option<String>,
    pub sender_known: bool,
    pub has_invite: bool,
    pub invite_status: Option<String>,
    pub invite_cancelled: bool,
}

#[derive(Debug, Clone)]
pub struct ThreadStateUpdateRequest {
    pub thread_id: String,
    pub next_state: WorkflowState,
    pub snooze_until: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ThreadOrganizationUpdateRequest {
    pub thread_id: String,
    pub board_position: Option<f64>,
    pub assignee: Option<String>,
    pub shared_with: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ThreadTagSetRequest {
    pub thread_ids: Vec<String>,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "core/db/threads/"))]
pub struct ThreadListRequest {
    pub account_id: Option<String>,
    pub contact_email: Option<String>,
    pub mailbox_kind: Option<String>,
    pub mailbox_value: Option<String>,
    pub workflow_state: Option<String>,
    /// Focused-list bucket. Applied server-side so opening a group can page
    /// through all of its threads instead of only the current inbox page.
    pub smart_bucket: Option<String>,
    /// Keyset cursor from a previous page. `None` starts at the newest thread.
    pub cursor: Option<ThreadCursor>,
    /// Fetch the nearest newer rows, returned in normal descending order.
    pub before_cursor: Option<ThreadCursor>,
    /// Page size. Clamped to 1..=1000; defaults to `thread_page_size()`.
    pub limit: Option<i64>,
}

/// Keyset cursor over `(threads.last_message_date DESC, threads.id DESC)`.
///
/// Offset pagination would skip or duplicate rows whenever a sync inserts a
/// thread between two page loads, which for a live mailbox is constant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "core/db/threads/"))]
pub struct ThreadCursor {
    pub last_message_date: String,
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "core/db/threads/"))]
pub struct ThreadPageRecord {
    pub threads: Vec<ThreadRecord>,
    /// Pass back as `cursor` to fetch the next page. `None` = end of list.
    pub next_cursor: Option<ThreadCursor>,
    pub has_more: bool,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "core/db/threads/"))]
pub struct SmartInboxPreviewRecord {
    pub id: String,
    pub sender_name: String,
    pub sender_email: String,
    pub subject: String,
    pub unread_count: u32,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "core/db/threads/"))]
pub struct SmartInboxBucketRecord {
    pub key: String,
    pub count: u32,
    pub unread_count: u32,
    pub preview_items: Vec<SmartInboxPreviewRecord>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "core/db/threads/"))]
pub struct SmartInboxSummaryRecord {
    pub total_count: u32,
    pub buckets: Vec<SmartInboxBucketRecord>,
}

/// Persistence boundary for thread reads and simple local state updates.
pub struct ThreadRepository<'a> {
    pool: &'a SqlitePool,
    snapshot_writer: Option<&'a SqlitePool>,
}

#[derive(Debug, FromRow)]
struct SenderProjectionTarget {
    id: String,
    category: Option<String>,
    category_source: String,
    priority: i64,
    priority_source: String,
    from_address: String,
    subject: Option<String>,
    raw_headers: Option<String>,
    provider_labels: String,
    list_unsubscribe: Option<String>,
    list_unsubscribe_post: Option<String>,
    folder_remote_id: Option<String>,
}

impl<'a> ThreadRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self {
            pool,
            snapshot_writer: None,
        }
    }

    pub fn with_pools(read: &'a SqlitePool, write: &'a SqlitePool) -> Self {
        Self {
            pool: read,
            snapshot_writer: Some(write),
        }
    }

    pub async fn list(&self, request: ThreadListRequest) -> Result<ThreadPageRecord, ThreadError> {
        let observed_at = chrono::Utc::now().to_rfc3339();
        let epoch = if request.cursor.is_none() && request.before_cursor.is_none() {
            sqlx::query_scalar::<_, i64>(
                "SELECT CAST(COALESCE((SELECT value FROM app_settings
                WHERE key = 'thread_list_snapshot_epoch'), '0') AS INTEGER)",
            )
            .fetch_one(self.pool)
            .await?
        } else {
            0
        };
        let page = list_threads_inner(self.pool, request.clone()).await?;
        if request.cursor.is_none() && request.before_cursor.is_none() {
            let pool = self.snapshot_writer.unwrap_or(self.pool).clone();
            let snapshot = page.clone();
            tokio::spawn(async move {
                let _permit = SNAPSHOT_WRITER
                    .acquire()
                    .await
                    .expect("snapshot semaphore open");
                if let Err(error) =
                    store_thread_list_snapshot(&pool, &request, &snapshot, &observed_at, epoch)
                        .await
                {
                    tracing::warn!(error = ?error, "could not update thread-list snapshot");
                }
            });
        }
        Ok(page)
    }

    pub async fn list_cached(
        &self,
        request: ThreadListRequest,
    ) -> Result<Option<ThreadPageRecord>, ThreadError> {
        load_thread_list_snapshot(self.pool, self.snapshot_writer, &request).await
    }

    pub async fn get(&self, id: &str) -> Result<ThreadRecord, ThreadError> {
        fetch_thread_record(self.pool, id).await
    }

    /// Fetch list projections in bounded SQL batches; missing IDs are omitted.
    pub async fn get_many(&self, ids: &[String]) -> Result<Vec<ThreadRecord>, ThreadError> {
        let ids: std::collections::BTreeSet<_> = ids.iter().filter(|id| !id.is_empty()).collect();
        let ids: Vec<_> = ids.into_iter().collect();
        let sql = thread_record_sql();
        let prefix = sql
            .strip_suffix("WHERE threads.id = ?\n         LIMIT 1")
            .expect("single-thread query predicate");
        let mut records = Vec::new();
        for batch in ids.chunks(500) {
            let mut query = sqlx::QueryBuilder::<sqlx::Sqlite>::new(prefix);
            query.push("WHERE threads.id IN (");
            let mut separated = query.separated(", ");
            for id in batch {
                separated.push_bind(*id);
            }
            separated.push_unseparated(")");
            let rows = query
                .build_query_as::<ThreadRow>()
                .fetch_all(self.pool)
                .await?;
            records.extend(rows.into_iter().map(thread_record_from_row));
        }
        Ok(records)
    }

    /// Set a thread-specific category. A manual choice is protected from
    /// sender preferences and message ingest until the caller changes it.
    pub async fn set_category(
        &self,
        thread_id: &str,
        category: Option<&str>,
    ) -> Result<(), ThreadError> {
        let category = normalize_category(category).map_err(|_| ThreadError::InvalidCategory)?;
        let thread_id = thread_id.trim();
        if thread_id.is_empty() {
            return Err(ThreadError::EmptyThreadId);
        }
        let changed = sqlx::query(
            "UPDATE threads
             SET category = ?, category_source = 'manual_thread', updated_at = ?
             WHERE id = ?",
        )
        .bind(category)
        .bind(chrono::Utc::now().to_rfc3339())
        .bind(thread_id)
        .execute(self.pool)
        .await?
        .rows_affected();
        if changed == 0 {
            return Err(ThreadError::NotFound);
        }
        Ok(())
    }

    /// Set a thread-specific priority. Manual values are protected from
    /// sender preferences and message ingest until the caller changes them.
    pub async fn set_priority(&self, thread_id: &str, priority: u8) -> Result<(), ThreadError> {
        let thread_id = thread_id.trim();
        if thread_id.is_empty() {
            return Err(ThreadError::EmptyThreadId);
        }
        if priority > 3 {
            return Err(ThreadError::InvalidPriority);
        }
        let changed = sqlx::query(
            "UPDATE threads
             SET priority = ?, priority_source = 'manual_thread', updated_at = ?
             WHERE id = ?",
        )
        .bind(priority as i64)
        .bind(chrono::Utc::now().to_rfc3339())
        .bind(thread_id)
        .execute(self.pool)
        .await?
        .rows_affected();
        if changed == 0 {
            return Err(ThreadError::NotFound);
        }
        Ok(())
    }

    /// Project one effective sender preference inside a caller-owned
    /// transaction. The category and priority projections share each target
    /// thread update, which lets sender-preference upserts commit both fields
    /// together.
    pub async fn project_sender_preference_in_transaction(
        &self,
        transaction: &mut Transaction<'_, Sqlite>,
        sender_email: &str,
        category: Option<Option<&str>>,
        is_priority: Option<bool>,
    ) -> Result<(), ThreadError> {
        if category.is_none() && is_priority.is_none() {
            return Ok(());
        }

        let project_category = category.is_some();
        let project_priority = is_priority.is_some();
        let mut target_query = String::from(
            "SELECT threads.id,
                    threads.category,
                    threads.category_source,
                    threads.priority,
                    threads.priority_source,
                    latest.from_address,
                    latest.subject,
                    latest.raw_headers,
                    latest.provider_labels,
                    latest.list_unsubscribe,
                    latest.list_unsubscribe_post,
                    folders.remote_id AS folder_remote_id
             FROM threads
             JOIN messages AS latest ON latest.id = (
                 SELECT candidate.id
                 FROM messages AS candidate
                 WHERE candidate.thread_id = threads.id
                 ORDER BY candidate.is_draft ASC, candidate.date DESC, candidate.id DESC
                 LIMIT 1
             )
             LEFT JOIN folders ON folders.id = latest.folder_id
             WHERE LOWER(TRIM(latest.from_address)) = ?
               AND (",
        );
        if project_category {
            target_query.push_str("threads.category_source != 'manual_thread'");
        }
        if project_category && project_priority {
            target_query.push_str(" OR ");
        }
        if project_priority {
            target_query.push_str("threads.priority_source != 'manual_thread'");
        }
        target_query.push(')');
        let targets = sqlx::query_as::<_, SenderProjectionTarget>(&target_query)
            .bind(sender_email)
            .fetch_all(&mut **transaction)
            .await?;

        let now = chrono::Utc::now().to_rfc3339();
        for target in targets {
            let update_category = project_category && target.category_source != "manual_thread";
            let update_priority = project_priority && target.priority_source != "manual_thread";
            if !update_category && !update_priority {
                continue;
            }

            let (next_category, next_category_source) = if update_category {
                match category.expect("category projection is present") {
                    Some(category) => (Some(category.to_owned()), "manual_sender"),
                    None => {
                        let labels = serde_json::from_str::<Vec<String>>(&target.provider_labels)
                            .unwrap_or_default();
                        let headers = classification_headers(
                            target.list_unsubscribe.as_deref(),
                            target.list_unsubscribe_post.as_deref(),
                            target.folder_remote_id.as_deref(),
                        );
                        let headers = if let Some(raw_headers) = target.raw_headers.as_deref() {
                            format!("{raw_headers}\n{headers}")
                        } else {
                            headers
                        };
                        let category = classify_message_with_provider_labels(
                            &labels,
                            Some(&headers),
                            &target.from_address,
                            target.subject.as_deref(),
                        );
                        let source = if crate::db::sender_preferences::provider_category(&labels)
                            .is_some()
                        {
                            "provider"
                        } else {
                            "heuristic"
                        };
                        (Some(category.to_owned()), source)
                    }
                }
            } else {
                (target.category.clone(), target.category_source.as_str())
            };

            let (next_priority, next_priority_source) = if update_priority {
                let labels = serde_json::from_str::<Vec<String>>(&target.provider_labels)
                    .unwrap_or_default();
                let provider_is_important = labels
                    .iter()
                    .any(|label| label.trim().eq_ignore_ascii_case("IMPORTANT"));
                if is_priority.expect("priority projection is present") {
                    (2, "manual_sender")
                } else if provider_is_important {
                    (2, "provider")
                } else {
                    (1, "heuristic")
                }
            } else {
                (target.priority, target.priority_source.as_str())
            };

            let mut update = QueryBuilder::<Sqlite>::new("UPDATE threads SET ");
            let mut has_assignment = false;
            if update_category {
                update
                    .push("category = ")
                    .push_bind(next_category)
                    .push(", category_source = ")
                    .push_bind(next_category_source);
                has_assignment = true;
            }
            if update_priority {
                if has_assignment {
                    update.push(", ");
                }
                update
                    .push("priority = ")
                    .push_bind(next_priority)
                    .push(", priority_source = ")
                    .push_bind(next_priority_source);
                has_assignment = true;
            }
            debug_assert!(has_assignment);
            update
                .push(", updated_at = ")
                .push_bind(&now)
                .push(" WHERE id = ")
                .push_bind(target.id)
                .build()
                .execute(&mut **transaction)
                .await?;
        }
        Ok(())
    }

    pub async fn update_state(
        &self,
        request: ThreadStateUpdateRequest,
    ) -> Result<ThreadRecord, ThreadError> {
        update_thread_state_inner(self.pool, request).await
    }

    /// Update board and collaboration metadata for one thread.
    ///
    /// Normalization belongs here so every caller observes the same limits,
    /// deduplication, and not-found behavior.
    pub async fn update_organization(
        &self,
        request: ThreadOrganizationUpdateRequest,
    ) -> Result<(), ThreadError> {
        let thread_id = request.thread_id.trim();
        if thread_id.is_empty() {
            return Err(ThreadError::EmptyThreadId);
        }
        if request
            .board_position
            .is_some_and(|value| !value.is_finite())
        {
            return Err(ThreadError::NonFiniteBoardPosition);
        }

        let assignee = normalize_optional_metadata(request.assignee, 320, "assignee")?;
        let shared_with = normalize_metadata_list(request.shared_with, 100, 320, "shared_with")?;
        let shared_with_json = serde_json::to_string(&shared_with)?;
        let changed = sqlx::query(
            "UPDATE threads
             SET board_position = ?, assignee = ?, shared_with = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(request.board_position)
        .bind(assignee)
        .bind(shared_with_json)
        .bind(chrono::Utc::now().to_rfc3339())
        .bind(thread_id)
        .execute(self.pool)
        .await?
        .rows_affected();
        if changed == 0 {
            return Err(ThreadError::NotFound);
        }
        Ok(())
    }

    /// Replace tags for every requested thread in one transaction.
    ///
    /// Thread existence is checked before any link changes. A missing thread
    /// therefore cannot leave earlier threads or newly created tags behind.
    pub async fn set_tags(&self, request: ThreadTagSetRequest) -> Result<(), ThreadError> {
        let thread_ids = normalize_metadata_list(request.thread_ids, 500, 320, "thread_ids")?;
        if thread_ids.is_empty() {
            return Ok(());
        }
        let tags = normalize_metadata_list(request.tags, 50, 80, "tags")?
            .into_iter()
            .map(|tag| tag.to_lowercase())
            .collect::<Vec<_>>();

        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        for thread_id in &thread_ids {
            let exists = sqlx::query_scalar::<_, i64>("SELECT 1 FROM threads WHERE id = ?")
                .bind(thread_id)
                .fetch_optional(&mut *tx)
                .await?
                .is_some();
            if !exists {
                return Err(ThreadError::NotFound);
            }
        }

        let mut tag_ids = Vec::with_capacity(tags.len());
        for tag in &tags {
            let tag_id = sqlx::query_scalar::<_, String>(
                "SELECT id FROM tags WHERE LOWER(name) = LOWER(?) ORDER BY id LIMIT 1",
            )
            .bind(tag)
            .fetch_optional(&mut *tx)
            .await?
            .unwrap_or_else(|| format!("tag-{}", uuid::Uuid::new_v4()));
            sqlx::query(
                "INSERT INTO tags (id, name, color, icon, is_system, sort_order)
                 VALUES (?, ?, NULL, NULL, 0, 0)
                 ON CONFLICT(id) DO NOTHING",
            )
            .bind(&tag_id)
            .bind(tag)
            .execute(&mut *tx)
            .await?;
            tag_ids.push(tag_id);
        }

        for thread_id in &thread_ids {
            sqlx::query("DELETE FROM thread_tags WHERE thread_id = ?")
                .bind(thread_id)
                .execute(&mut *tx)
                .await?;
            for tag_id in &tag_ids {
                sqlx::query(
                    "INSERT INTO thread_tags (thread_id, tag_id) VALUES (?, ?)
                     ON CONFLICT(thread_id, tag_id) DO NOTHING",
                )
                .bind(thread_id)
                .bind(tag_id)
                .execute(&mut *tx)
                .await?;
            }
        }
        tx.commit().await?;
        Ok(())
    }

    /// Refresh the aggregate projection for the supplied threads.
    ///
    /// Message rows are the source of truth. A single transaction covers all
    /// requested ids so an empty-thread cleanup cannot expose a half-deleted
    /// thread, tags, workflow events, or topic links to another reader.
    pub async fn refresh_projection(&self, thread_ids: &[String]) -> Result<(), ThreadError> {
        if thread_ids.is_empty() {
            return Ok(());
        }

        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        refresh_thread_projections_tx(&mut tx, thread_ids).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Find threads whose message sender matches one blocked-sender rule.
    ///
    /// Message selection belongs to the thread aggregate rather than the
    /// blocked-sender rule store. Address and domain matching are exact and
    /// case-insensitive; the caller has already normalized the pattern.
    pub async fn find_matching_sender_threads(
        &self,
        account_id: Option<&str>,
        pattern: &str,
        pattern_type: BlockedSenderPatternType,
    ) -> Result<Vec<String>, ThreadError> {
        let mut query = QueryBuilder::<Sqlite>::new(
            "SELECT DISTINCT messages.thread_id
             FROM messages
             JOIN accounts ON accounts.id = messages.account_id
             WHERE accounts.kind = 'email' AND ",
        );
        if let Some(account_id) = account_id {
            query
                .push("account_id = ")
                .push_bind(account_id)
                .push(" AND ");
        }
        match pattern_type {
            BlockedSenderPatternType::Address => {
                query
                    .push("LOWER(TRIM(from_address)) = ")
                    .push_bind(pattern);
            }
            BlockedSenderPatternType::Domain => {
                let bare = pattern.trim_start_matches('*').trim_start_matches('@');
                query.push(
                    "INSTR(TRIM(from_address), '@') > 0
                     AND LOWER(SUBSTR(TRIM(from_address), INSTR(TRIM(from_address), '@') + 1)) = ",
                );
                query.push_bind(bare);
            }
        }
        let rows = query
            .build_query_scalar::<String>()
            .fetch_all(self.pool)
            .await?;
        Ok(rows)
    }

    /// Whether a pre-existing thread still needs the blocked-mail projection.
    /// Reads `workflow_state` only: `board_column` is a write-only mirror.
    pub async fn blocked_projection_pending(&self, thread_id: &str) -> Result<bool, ThreadError> {
        let state = sqlx::query_scalar::<_, i64>(
            "SELECT CASE WHEN COALESCE(workflow_state, '') = 'done' THEN 0 ELSE 1 END
             FROM threads
             WHERE id = ?",
        )
        .bind(thread_id)
        .fetch_optional(self.pool)
        .await?;
        Ok(state.unwrap_or_default() != 0)
    }

    /// Commit blocked-sender provider moves and their local thread projection
    /// as one durable transaction. A failed queue insert or projection update
    /// therefore leaves the provider move and thread state retryable. Message
    /// folder rows remain in their source folder until remote sync confirms the
    /// provider move and supplies its new remote identity.
    pub async fn commit_blocked_moves(
        &self,
        queue: &SyncQueue,
        thread_ids: &[String],
        actions: &[QueuedAction],
    ) -> Result<(), ThreadError> {
        if thread_ids.is_empty() {
            return Ok(());
        }
        let now = chrono::Utc::now().to_rfc3339();
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let mut queued_action_count = 0;
        for action in actions {
            for action in refresh_blocked_move_refs_in_transaction(&mut tx, action).await? {
                queue
                    .enqueue_in_transaction(&mut tx, &action)
                    .await
                    .map_err(ThreadError::Queue)?;
                queued_action_count += 1;
            }
        }
        for thread_id in thread_ids {
            let changed = sqlx::query(
                "UPDATE threads
                 SET workflow_state = 'done', board_column = 'done',
                     workflow_changed_at = ?, updated_at = ?
                 WHERE id = ?",
            )
            .bind(&now)
            .bind(&now)
            .bind(thread_id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
            if changed == 0 {
                return Err(ThreadError::NotFound);
            }
            // The messages are the truth about the count. Writing
            // `unread_count = 0` here and leaving `is_read = 0` behind meant
            // the next projection refresh — the ingest runs one before it gets
            // to the blocked check — counted them straight back up.
            sqlx::query("UPDATE messages SET is_read = 1, updated_at = ? WHERE thread_id = ?")
                .bind(&now)
                .bind(thread_id)
                .execute(&mut *tx)
                .await?;
            refresh_thread_projection_tx(&mut tx, thread_id, &now).await?;
        }
        tx.commit().await?;
        if queued_action_count > 0 {
            queue.wake_worker();
        }
        Ok(())
    }

    /// Set the local message read state and refresh its thread projections.
    ///
    /// The message flags and aggregate counts commit together. Queue ordering
    /// stays with the caller because this repository has no remote provider.
    pub async fn set_read_state(
        &self,
        thread_ids: &[String],
        is_read: bool,
    ) -> Result<(), ThreadError> {
        if thread_ids.is_empty() {
            return Ok(());
        }

        let now = chrono::Utc::now().to_rfc3339();
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let mut query = QueryBuilder::<Sqlite>::new("UPDATE messages SET is_read = ");
        query.push_bind(if is_read { 1 } else { 0 });
        query.push(", updated_at = ");
        query.push_bind(&now);
        query.push(" WHERE thread_id IN (");
        {
            let mut separated = query.separated(", ");
            for thread_id in thread_ids {
                separated.push_bind(thread_id);
            }
        }
        query.push(")");
        query.build().execute(&mut *tx).await?;

        for thread_id in thread_ids {
            refresh_thread_projection_tx(&mut tx, thread_id, &now).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Permanently remove eligible trash threads and their local dependents.
    ///
    /// Selection and deletion share one transaction so a maintenance pass
    /// cannot expose a half-removed thread aggregate. A thread qualifies only
    /// when it is done, older than `cutoff`, has at least one real trash
    /// message, and has no message outside a real trash folder. Missing folder
    /// rows deliberately count as non-trash. Durable workflow moves defer the
    /// deletion because failed moves may be retried later.
    pub async fn purge_trash_before(&self, cutoff: &str) -> Result<u64, ThreadError> {
        let mut after = String::new();
        let mut removed = 0;
        loop {
            let batch = super::trash::purge_batch(self.pool, cutoff, &after).await?;
            removed += batch.removed_threads;
            let Some(cursor) = batch.cursor else {
                return Ok(removed);
            };
            after = cursor;
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }

    /// Merge a split child thread into its canonical conversation.
    ///
    /// One transaction moves messages, topic links, tags and workflow
    /// dependents together. It also refreshes the canonical projection and
    /// removes the child before commit. The canonical thread's scalar workflow
    /// state is never overwritten. A source or target that disappeared during
    /// candidate discovery is an idempotent no-op. A queued remote move keeps
    /// the child id in its durable payload, so the merge is deferred until
    /// that row is removed. Failed rows count because they can be requeued.
    pub async fn merge_into(
        &self,
        child_thread_id: &str,
        canonical_thread_id: &str,
    ) -> Result<u64, ThreadError> {
        if child_thread_id == canonical_thread_id {
            return Ok(0);
        }

        let now = chrono::Utc::now().to_rfc3339();
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;

        let thread_accounts = sqlx::query_as::<_, (String, String)>(
            "SELECT id, account_id FROM threads WHERE id IN (?, ?)",
        )
        .bind(child_thread_id)
        .bind(canonical_thread_id)
        .fetch_all(&mut *tx)
        .await?;
        if thread_accounts.len() != 2
            || thread_accounts
                .iter()
                .any(|(_, account_id)| account_id != &thread_accounts[0].1)
        {
            return Ok(0);
        }
        let account_id = &thread_accounts[0].1;

        let has_cross_account_message = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(
                SELECT 1 FROM messages
                WHERE thread_id = ? AND account_id != ?
             )",
        )
        .bind(child_thread_id)
        .bind(account_id)
        .fetch_one(&mut *tx)
        .await?;
        if has_cross_account_message {
            return Ok(0);
        }

        if remote_move_queued_for_thread(&mut tx, account_id, child_thread_id).await? {
            return Ok(0);
        }

        let moved = sqlx::query(
            "UPDATE messages SET thread_id = ?, updated_at = ?
             WHERE thread_id = ? AND account_id = ?",
        )
        .bind(canonical_thread_id)
        .bind(&now)
        .bind(child_thread_id)
        .bind(account_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if moved == 0 {
            return Ok(0);
        }

        // Existing destination links/tags win; source-only memberships are
        // inserted before the source rows are removed.
        TopicRepository::transfer_thread_links_tx(&mut tx, child_thread_id, canonical_thread_id)
            .await?;
        sqlx::query(
            "INSERT OR IGNORE INTO thread_tags (thread_id, tag_id)
             SELECT ?, tag_id FROM thread_tags WHERE thread_id = ?",
        )
        .bind(canonical_thread_id)
        .bind(child_thread_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM thread_tags WHERE thread_id = ?")
            .bind(child_thread_id)
            .execute(&mut *tx)
            .await?;

        sqlx::query("UPDATE workflow_events SET thread_id = ? WHERE thread_id = ?")
            .bind(canonical_thread_id)
            .bind(child_thread_id)
            .execute(&mut *tx)
            .await?;
        reassign_thread_tx(&mut tx, child_thread_id, canonical_thread_id).await?;
        sqlx::query("UPDATE ingest_effects SET thread_id = ? WHERE thread_id = ?")
            .bind(canonical_thread_id)
            .bind(child_thread_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "DELETE FROM threads
             WHERE id = ? AND NOT EXISTS (SELECT 1 FROM messages WHERE thread_id = ?)",
        )
        .bind(child_thread_id)
        .bind(child_thread_id)
        .execute(&mut *tx)
        .await?;

        refresh_thread_projection_tx(&mut tx, canonical_thread_id, &now).await?;
        clear_thread_list_snapshots_tx(&mut tx).await?;
        tx.commit().await?;
        Ok(moved)
    }

    pub async fn smart_inbox_summary(
        &self,
        account_id: Option<&str>,
    ) -> Result<SmartInboxSummaryRecord, ThreadError> {
        list_smart_inbox_summary_inner(self.pool, account_id).await
    }
}

/// Re-read blocked-move refs while the commit transaction owns the write lock.
/// A folder sync can move a message between preflight and commit; queueing the
/// old folder-scoped ref would then address the wrong remote copy on retry.
///
/// Every returned action owns exactly one canonical remote ref. If a ref moved
/// folders, its new action receives a new id and fresh lifecycle state; the old
/// pending/failed row is removed so its stale folder-scoped id cannot be sent.
async fn refresh_blocked_move_refs_in_transaction(
    tx: &mut Transaction<'_, Sqlite>,
    action: &QueuedAction,
) -> Result<Vec<QueuedAction>, ThreadError> {
    let Some("blocked_move") = action.payload.get("kind").and_then(Value::as_str) else {
        return Ok(vec![action.clone()]);
    };
    let Some(thread_id) = action
        .payload
        .get("blocked_thread_id")
        .and_then(Value::as_str)
    else {
        return Ok(vec![action.clone()]);
    };
    let Some(target_remote_folder_id) = action
        .payload
        .get("target_remote_folder_id")
        .and_then(Value::as_str)
    else {
        return Ok(vec![action.clone()]);
    };
    let target_remote_folder_id = target_remote_folder_id.trim();
    if target_remote_folder_id.is_empty() {
        return Ok(vec![action.clone()]);
    }

    let old_refs = action
        .payload
        .get("refs")
        .cloned()
        .map(|refs| {
            serde_json::from_value::<Vec<MessageRef>>(refs).map(|refs| {
                refs.into_iter()
                    .filter_map(canonicalize_blocked_message_ref)
                    .collect::<Vec<_>>()
            })
        })
        .transpose()?;
    let rows = sqlx::query_as::<_, (String, Option<String>, String, Option<i64>)>(
        "SELECT m.remote_id, f.remote_id, f.full_path, f.uidvalidity
         FROM messages m
         JOIN accounts a ON a.id = m.account_id
         JOIN folders f ON f.id = m.folder_id
         WHERE a.kind = 'email'
           AND m.account_id = ?
           AND m.thread_id = ?
           AND m.remote_id IS NOT NULL",
    )
    .bind(&action.account_id)
    .bind(thread_id)
    .fetch_all(&mut **tx)
    .await?;

    let refs = rows
        .into_iter()
        .filter_map(
            |(remote_id, folder_remote_id, folder_path, folder_uidvalidity)| {
                let source_folder = folder_remote_id.or(Some(folder_path));
                source_folder
                    .as_deref()
                    .is_none_or(|source| source.trim() != target_remote_folder_id)
                    .then_some(MessageRef {
                        remote_id,
                        source_folder,
                        source_uidvalidity: folder_uidvalidity
                            .and_then(|value| u32::try_from(value).ok()),
                    })
                    .and_then(canonicalize_blocked_message_ref)
            },
        )
        .collect::<Vec<_>>();

    let original_ref = old_refs
        .as_ref()
        .and_then(|refs| (refs.len() == 1).then(|| &refs[0]));
    let mut desired_ids = HashSet::new();
    let mut refreshed = Vec::with_capacity(refs.len());
    for message_ref in refs {
        let id = stable_blocked_move_id(
            thread_id,
            &action.account_id,
            &message_ref,
            target_remote_folder_id,
        );
        desired_ids.insert(id.clone());
        let same_immutable_ref =
            original_ref.is_some_and(|original| same_message_ref(original, &message_ref));
        let mut refreshed_action = if same_immutable_ref {
            let mut refreshed = action.clone();
            let mut payload = action.payload.clone();
            payload["refs"] = serde_json::to_value([message_ref.clone()])?;
            payload["target_remote_folder_id"] = Value::String(target_remote_folder_id.to_string());
            if let (Some(uidvalidity), Some(source_folder)) = (
                message_ref.source_uidvalidity,
                message_ref.source_folder.as_deref(),
            ) {
                payload["source_uidvalidity_before"] = serde_json::to_value(HashMap::from([(
                    source_folder.to_string(),
                    uidvalidity,
                )]))?;
            }
            refreshed.payload = payload;
            refreshed
        } else {
            let source_uidvalidity_before = message_ref
                .source_uidvalidity
                .zip(message_ref.source_folder.clone())
                .map(|(uidvalidity, source_folder)| HashMap::from([(source_folder, uidvalidity)]));
            let payload = serde_json::to_value(RemoteMutationPayload::BlockedMove {
                blocked_thread_id: thread_id.to_string(),
                refs: vec![message_ref.clone()],
                target_remote_folder_id: target_remote_folder_id.to_string(),
                provider_move_started: false,
                provider_move_completed: false,
                source_uidvalidity_before,
            })?;
            QueuedAction::new(
                action.account_id.clone(),
                action.action_type.clone(),
                payload,
            )
        };
        refreshed_action.id = id;
        refreshed.push(refreshed_action);
    }
    if !desired_ids.contains(&action.id) {
        sqlx::query(
            "DELETE FROM sync_queue
             WHERE id = ? AND status IN ('pending', 'scheduled', 'failed')",
        )
        .bind(&action.id)
        .execute(&mut **tx)
        .await?;
    }
    Ok(refreshed)
}

fn normalized_folder(folder: Option<&str>) -> Option<&str> {
    folder.map(str::trim).filter(|folder| !folder.is_empty())
}

fn same_message_ref(left: &MessageRef, right: &MessageRef) -> bool {
    left.remote_id == right.remote_id
        && normalized_folder(left.source_folder.as_deref())
            == normalized_folder(right.source_folder.as_deref())
        && left.source_uidvalidity == right.source_uidvalidity
}

fn normalize_optional_metadata(
    value: Option<String>,
    max_len: usize,
    field: &'static str,
) -> Result<Option<String>, ThreadError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    validate_metadata_value(trimmed, max_len, field)?;
    Ok(Some(trimmed.to_string()))
}

fn normalize_metadata_list(
    values: Vec<String>,
    max_items: usize,
    max_len: usize,
    field: &'static str,
) -> Result<Vec<String>, ThreadError> {
    if values.len() > max_items {
        return Err(ThreadError::TooManyMetadataValues {
            field,
            max: max_items,
        });
    }
    let mut normalized = Vec::new();
    for value in values {
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        validate_metadata_value(value, max_len, field)?;
        if !normalized
            .iter()
            .any(|existing: &String| existing.eq_ignore_ascii_case(value))
        {
            normalized.push(value.to_string());
        }
    }
    Ok(normalized)
}

fn validate_metadata_value(
    value: &str,
    max_len: usize,
    field: &'static str,
) -> Result<(), ThreadError> {
    if value.chars().count() > max_len {
        return Err(ThreadError::MetadataValueTooLong {
            field,
            max: max_len,
        });
    }
    if value.chars().any(char::is_control) {
        return Err(ThreadError::MetadataControlCharacters { field });
    }
    Ok(())
}

/// Return whether a durable remote move still names this thread.
///
/// The typed extractor keeps this repository Tauri-free while preserving the
/// queue's retry safety. Invalid or unrelated payloads are ignored.
pub(super) async fn remote_move_queued_for_thread(
    tx: &mut Transaction<'_, Sqlite>,
    account_id: &str,
    thread_id: &str,
) -> Result<bool, ThreadError> {
    let payloads = sqlx::query_scalar::<_, String>(
        "SELECT payload FROM sync_queue
         WHERE account_id = ?
           AND action_type = 'remote_mutation'
           AND status IN ('pending', 'sending', 'scheduled', 'failed')",
    )
    .bind(account_id)
    .fetch_all(&mut **tx)
    .await?;

    Ok(payloads
        .into_iter()
        .filter_map(|payload| remote_move_thread_id_from_json(&payload))
        .any(|queued_thread_id| queued_thread_id == thread_id))
}

/// Refresh the aggregate projection for the supplied threads inside a
/// transaction the caller owns.
///
/// Ingest writes the message rows and their projection under one write lock,
/// so it cannot borrow the pool for a second transaction of its own.
pub async fn refresh_thread_projections_tx(
    tx: &mut Transaction<'_, Sqlite>,
    thread_ids: &[String],
) -> Result<(), ThreadError> {
    let now = chrono::Utc::now().to_rfc3339();
    for thread_id in thread_ids {
        refresh_thread_projection_tx(tx, thread_id, &now).await?;
    }
    Ok(())
}

pub(crate) async fn refresh_thread_projection_tx(
    tx: &mut Transaction<'_, Sqlite>,
    thread_id: &str,
    now: &str,
) -> Result<(), ThreadError> {
    let message_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(DISTINCT CASE
            WHEN id LIKE 'm-imap-%' THEN COALESCE(
                NULLIF(TRIM(CASE
                    WHEN LOWER(COALESCE(message_id_header, '')) LIKE '%@imap.satchel.local>' OR LOWER(COALESCE(message_id_header, '')) LIKE '%@imap.flowmail.local>' THEN ''
                    ELSE COALESCE(message_id_header, '')
                END), ''),
                COALESCE(NULLIF(TRIM(folder_id), ''), '') || ':' || COALESCE(NULLIF(TRIM(remote_id), ''), id)
            )
            ELSE COALESCE(NULLIF(TRIM(message_id_header), ''), NULLIF(TRIM(remote_id), ''), id)
         END)
         FROM messages
         WHERE thread_id = ?",
    )
    .bind(thread_id)
    .fetch_one(&mut **tx)
    .await?;

    if message_count <= 0 {
        sqlx::query("DELETE FROM thread_tags WHERE thread_id = ?")
            .bind(thread_id)
            .execute(&mut **tx)
            .await?;
        sqlx::query("DELETE FROM workflow_events WHERE thread_id = ?")
            .bind(thread_id)
            .execute(&mut **tx)
            .await?;
        TopicRepository::remove_thread_links_tx(tx, thread_id).await?;
        sqlx::query("DELETE FROM threads WHERE id = ?")
            .bind(thread_id)
            .execute(&mut **tx)
            .await?;
        return Ok(());
    }

    let unread_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*)
         FROM (
                SELECT CASE
                    WHEN id LIKE 'm-imap-%' THEN COALESCE(
                        NULLIF(TRIM(CASE
                            WHEN LOWER(COALESCE(message_id_header, '')) LIKE '%@imap.satchel.local>' OR LOWER(COALESCE(message_id_header, '')) LIKE '%@imap.flowmail.local>' THEN ''
                            ELSE COALESCE(message_id_header, '')
                        END), ''),
                        COALESCE(NULLIF(TRIM(folder_id), ''), '') || ':' || COALESCE(NULLIF(TRIM(remote_id), ''), id)
                    )
                    ELSE COALESCE(NULLIF(TRIM(message_id_header), ''), NULLIF(TRIM(remote_id), ''), id)
                END AS dedupe_key
                FROM messages
                WHERE thread_id = ? AND is_read = 0
            GROUP BY dedupe_key
         )",
    )
    .bind(thread_id)
    .fetch_one(&mut **tx)
    .await?;
    let is_starred = sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(MAX(is_starred), 0) FROM messages WHERE thread_id = ?",
    )
    .bind(thread_id)
    .fetch_one(&mut **tx)
    .await?;
    let has_attachments = sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(MAX(has_attachments), 0) FROM messages WHERE thread_id = ?",
    )
    .bind(thread_id)
    .fetch_one(&mut **tx)
    .await?;
    let latest = sqlx::query_as::<_, (Option<String>, Option<String>, Option<String>)>(
        "SELECT subject, snippet, date
         FROM messages
         WHERE thread_id = ?
         ORDER BY is_draft ASC, date DESC, id DESC
         LIMIT 1",
    )
    .bind(thread_id)
    .fetch_one(&mut **tx)
    .await?;

    sqlx::query(
        "UPDATE threads
         SET message_count = ?, unread_count = ?, is_starred = ?, has_attachments = ?,
             subject = COALESCE(?, subject), snippet = COALESCE(?, snippet),
             last_message_date = COALESCE(?, last_message_date), updated_at = ?
         WHERE id = ?",
    )
    .bind(message_count)
    .bind(unread_count)
    .bind(is_starred)
    .bind(has_attachments)
    .bind(latest.0)
    .bind(latest.1)
    .bind(latest.2)
    .bind(now)
    .bind(thread_id)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

#[derive(Debug, FromRow)]
struct SmartBucketAggregateRow {
    bucket: String,
    count: i64,
    unread_count: i64,
}

#[derive(Debug, FromRow)]
struct SmartBucketPreviewRow {
    bucket: String,
    id: String,
    sender_name: String,
    sender_email: String,
    subject: String,
    unread_count: i64,
}

#[derive(Debug, FromRow)]
struct ThreadRow {
    smart_bucket: String,
    id: String,
    account_id: String,
    subject: Option<String>,
    snippet: Option<String>,
    last_message_date: Option<String>,
    sort_date: String,
    message_count: i64,
    workflow_state: String,
    unread_count: i64,
    has_attachments: i64,
    has_draft: i64,
    draft_snippet: Option<String>,
    priority: i64,
    board_position: Option<f64>,
    tags: Option<String>,
    assignee: Option<String>,
    shared_with: Option<String>,
    participants: Option<String>,
    message_participants: Option<String>,
    sent_participants: Option<String>,
    mailbox_names: Option<String>,
    latest_from_address: Option<String>,
    latest_from_name: Option<String>,
    category: Option<String>,
    is_muted: i64,
    is_starred: i64,
    snooze_until: Option<String>,
    sender_known: i64,
    has_invite: i64,
    invite_status: Option<String>,
    invite_cancelled: i64,
}

/// Default page size. Small enough that the first paint is fast on a large
/// mailbox, large enough that scrolling rarely waits.
/// Read once per process: this used to be `std::env::var` on every list request.
static THREAD_PAGE_SIZE: std::sync::LazyLock<i64> = std::sync::LazyLock::new(|| {
    thread_page_size_from(std::env::var("SATCHEL_THREADS_PAGE_SIZE").ok().as_deref())
});

fn thread_page_size() -> i64 {
    *THREAD_PAGE_SIZE
}

fn thread_page_size_from(raw: Option<&str>) -> i64 {
    raw.and_then(|raw| raw.parse::<i64>().ok())
        .filter(|value| *value > 0)
        .map(|value| value.min(1_000))
        .unwrap_or(200)
}

fn parse_string_array(raw: Option<String>) -> Vec<String> {
    raw.as_deref()
        .and_then(|value| serde_json::from_str::<Vec<String>>(value).ok())
        .unwrap_or_default()
}

const THREAD_LIST_SNAPSHOT_VERSION: u8 = 1;
const THREAD_LIST_SNAPSHOT_LIMIT: i64 = 24;
/// A snapshot is a first-paint cache, not a store of record. Rows it names can
/// be gone by now — purged trash, a merged thread — and opening one of those
/// ends in `ThreadError::NotFound`. The write paths that remove threads clear
/// the table; this bounds everything they cannot see.
const THREAD_LIST_SNAPSHOT_MAX_AGE_HOURS: i64 = 24;

fn thread_list_snapshot_key(request: &ThreadListRequest) -> Result<String, ThreadError> {
    Ok(format!(
        "v{THREAD_LIST_SNAPSHOT_VERSION}:{}",
        serde_json::to_string(request)?
    ))
}

/// Drop the whole first-paint cache from inside a write transaction.
///
/// The cache key is the serialized request, so there is no way to tell which
/// of at most 24 pages named a thread that just disappeared. Rebuilding them
/// costs one `list()` each; serving a thread that no longer exists costs a
/// `ThreadError::NotFound` on the next click.
pub(super) async fn clear_thread_list_snapshots_tx(
    tx: &mut Transaction<'_, Sqlite>,
) -> Result<(), ThreadError> {
    sqlx::query(
        "INSERT INTO app_settings(key, value, updated_at)
        VALUES('thread_list_snapshot_epoch', '1', '')
        ON CONFLICT(key) DO UPDATE SET value = CAST(value AS INTEGER) + 1",
    )
    .execute(&mut **tx)
    .await?;
    sqlx::query("DELETE FROM thread_list_snapshots")
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// An unparsable `updated_at` counts as stale: a snapshot whose age cannot be
/// established is exactly the one not to trust.
fn snapshot_is_stale(updated_at: &str) -> bool {
    match chrono::DateTime::parse_from_rfc3339(updated_at) {
        Ok(stamp) => {
            chrono::Utc::now().signed_duration_since(stamp.with_timezone(&chrono::Utc))
                >= chrono::Duration::hours(THREAD_LIST_SNAPSHOT_MAX_AGE_HOURS)
        }
        Err(_) => true,
    }
}

async fn load_thread_list_snapshot(
    pool: &SqlitePool,
    writer: Option<&SqlitePool>,
    request: &ThreadListRequest,
) -> Result<Option<ThreadPageRecord>, ThreadError> {
    if request.cursor.is_some() || request.before_cursor.is_some() {
        return Ok(None);
    }

    let cache_key = thread_list_snapshot_key(request)?;
    let row = sqlx::query_as::<_, (String, String)>(
        "SELECT page_json, updated_at FROM thread_list_snapshots WHERE cache_key = ?",
    )
    .bind(&cache_key)
    .fetch_optional(pool)
    .await?;

    let Some((page_json, updated_at)) = row else {
        return Ok(None);
    };
    if snapshot_is_stale(&updated_at) {
        tracing::debug!(%updated_at, "discarding aged thread-list snapshot");
        discard_thread_snapshot(pool, writer, cache_key, page_json).await;
        return Ok(None);
    }
    match serde_json::from_str::<ThreadPageRecord>(&page_json) {
        Ok(page) => {
            tracing::debug!(rows = page.threads.len(), "loaded thread-list snapshot");
            Ok(Some(page))
        }
        Err(error) => {
            tracing::warn!(%error, "discarding invalid thread-list snapshot");
            discard_thread_snapshot(pool, writer, cache_key, page_json).await;
            Ok(None)
        }
    }
}

async fn discard_thread_snapshot(
    read: &SqlitePool,
    writer: Option<&SqlitePool>,
    key: String,
    observed: String,
) {
    async fn discard(pool: &SqlitePool, key: String, observed: String) {
        let _ =
            sqlx::query("DELETE FROM thread_list_snapshots WHERE cache_key = ? AND page_json = ?")
                .bind(key)
                .bind(observed)
                .execute(pool)
                .await;
    }
    if let Some(write) = writer {
        let write = write.clone();
        tokio::spawn(async move {
            discard(&write, key, observed).await;
        });
    } else {
        discard(read, key, observed).await;
    }
}

// Cache writers share one connection at a time, leaving readers room in the pool.
static SNAPSHOT_WRITER: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);

async fn store_thread_list_snapshot(
    pool: &SqlitePool,
    request: &ThreadListRequest,
    page: &ThreadPageRecord,
    observed_at: &str,
    epoch: i64,
) -> Result<(), ThreadError> {
    if request.cursor.is_some() || request.before_cursor.is_some() {
        return Ok(());
    }

    let cache_key = thread_list_snapshot_key(request)?;
    let page_json = serde_json::to_string(page)?;
    let existing: Option<(String, String)> = sqlx::query_as(
        "SELECT page_json, updated_at FROM thread_list_snapshots WHERE cache_key = ?",
    )
    .bind(&cache_key)
    .fetch_optional(pool)
    .await?;
    if existing.is_some_and(|(json, updated_at)| {
        updated_at.as_str() > observed_at || (json == page_json && !snapshot_is_stale(&updated_at))
    }) {
        return Ok(());
    }

    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    // A purge may have committed while this optional snapshot waited for a writer.
    if !page.threads.is_empty() {
        let mut query = QueryBuilder::<Sqlite>::new("SELECT COUNT(*) FROM threads WHERE id IN (");
        let mut ids = query.separated(", ");
        for thread in &page.threads {
            ids.push_bind(&thread.id);
        }
        ids.push_unseparated(")");
        let present: i64 = query.build_query_scalar().fetch_one(&mut *tx).await?;
        if present != page.threads.len() as i64 {
            tx.rollback().await?;
            return Ok(());
        }
    }
    let changed = sqlx::query(
        "INSERT INTO thread_list_snapshots (cache_key, page_json, updated_at)
         SELECT ?, ?, ? WHERE CAST(COALESCE((SELECT value FROM app_settings
             WHERE key = 'thread_list_snapshot_epoch'), '0') AS INTEGER) = ?
         ON CONFLICT(cache_key) DO UPDATE SET
           page_json = excluded.page_json,
           updated_at = excluded.updated_at
         WHERE excluded.updated_at >= thread_list_snapshots.updated_at
           AND (excluded.page_json <> thread_list_snapshots.page_json
                OR thread_list_snapshots.updated_at < ?)",
    )
    .bind(cache_key)
    .bind(page_json)
    .bind(observed_at)
    .bind(epoch)
    .bind(
        (chrono::Utc::now() - chrono::Duration::hours(THREAD_LIST_SNAPSHOT_MAX_AGE_HOURS))
            .to_rfc3339(),
    )
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if changed == 0 {
        tx.rollback().await?;
        return Ok(());
    }
    sqlx::query(
        "DELETE FROM thread_list_snapshots
         WHERE cache_key IN (
           SELECT cache_key
           FROM thread_list_snapshots
           ORDER BY updated_at DESC
           LIMIT -1 OFFSET ?
         )",
    )
    .bind(THREAD_LIST_SNAPSHOT_LIMIT)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::sync::remote_mutations::RemoteMutationPayload;
    use serde_json::Value;
    use sqlx::sqlite::SqlitePoolOptions;

    use super::*;

    async fn migrated_pool() -> sqlx::SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory pool");
        crate::db::init(&pool).await.expect("migrations");
        pool
    }

    #[tokio::test]
    async fn get_many_matches_single_projection_and_omits_missing_ids() {
        let pool = migrated_pool().await;
        seed_thread(&pool).await;
        let repository = ThreadRepository::new(&pool);
        let single = repository.get("thread-1").await.unwrap();
        let rows = repository
            .get_many(&["thread-1".into(), "missing".into(), "thread-1".into()])
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, single.id);
        assert_eq!(rows[0].participants, single.participants);
        assert_eq!(rows[0].mailbox_names, single.mailbox_names);
        assert!(repository.get_many(&[]).await.unwrap().is_empty());
        let ids: Vec<_> = (0..1001).map(|i| format!("batch-{i}")).collect();
        for id in &ids {
            insert_thread(&pool, id, "acc").await;
        }
        assert_eq!(repository.get_many(&ids).await.unwrap().len(), 1001);
    }

    async fn seed_thread(pool: &sqlx::SqlitePool) {
        sqlx::query(
            "INSERT INTO accounts(id, display_name, email, provider, config, credentials_ref, is_active, created_at, updated_at)
             VALUES('acc', 'A', 'a@x.test', 'imap', '{}', '', 1, '2026-01-01', '2026-01-01')",
        )
        .execute(pool)
        .await
        .expect("account");
        sqlx::query(
            "INSERT INTO threads(id, account_id, subject, last_message_date, message_count, unread_count, created_at, updated_at)
             VALUES('thread-1', 'acc', 'Subject', '2026-08-01T00:00:00Z', 1, 0, '2026-01-01', '2026-01-01')",
        )
        .execute(pool)
        .await
        .expect("thread");
        for (id, folder_type) in [
            ("inbox", "inbox"),
            ("archive", "archive"),
            ("drafts", "drafts"),
        ] {
            sqlx::query(
                "INSERT INTO folders(id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at)
                 VALUES(?, 'acc', ?, ?, ?, ?, '2026-01-01', '2026-01-01')",
            )
            .bind(id)
            .bind(folder_type)
            .bind(folder_type)
            .bind(folder_type)
            .bind(folder_type)
            .execute(pool)
            .await
            .expect("folder");
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn insert_message(
        pool: &sqlx::SqlitePool,
        id: &str,
        thread_id: &str,
        folder_id: &str,
        message_id_header: Option<&str>,
        remote_id: &str,
        date: &str,
        is_read: bool,
        is_starred: bool,
        is_draft: bool,
        has_attachments: bool,
        subject: &str,
        snippet: &str,
    ) {
        sqlx::query(
            "INSERT INTO messages(
                id, account_id, folder_id, thread_id, message_id_header,
                from_address, to_addresses, cc_addresses, bcc_addresses,
                subject, date, snippet, is_read, is_starred, is_draft,
                has_attachments, remote_id, created_at, updated_at
             ) VALUES(?, 'acc', ?, ?, ?, 'sender@example.com', '[]', '[]', '[]',
                      ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(folder_id)
        .bind(thread_id)
        .bind(message_id_header)
        .bind(subject)
        .bind(date)
        .bind(snippet)
        .bind(is_read as i64)
        .bind(is_starred as i64)
        .bind(is_draft as i64)
        .bind(has_attachments as i64)
        .bind(remote_id)
        .bind(date)
        .bind(date)
        .execute(pool)
        .await
        .expect("message");
    }

    async fn insert_thread(pool: &sqlx::SqlitePool, id: &str, account_id: &str) {
        sqlx::query(
            "INSERT INTO threads(id, account_id, subject, last_message_date, message_count,
                                  unread_count, created_at, updated_at)
             VALUES(?, ?, 'Thread', '2026-08-01T00:00:00Z', 1, 0, '2026-01-01', '2026-01-01')",
        )
        .bind(id)
        .bind(account_id)
        .execute(pool)
        .await
        .expect("thread");
    }

    async fn insert_trash_folder(pool: &sqlx::SqlitePool, id: &str, folder_type: &str) {
        sqlx::query(
            "INSERT INTO folders(id, account_id, name, full_path, folder_type, remote_id,
                                 created_at, updated_at)
             VALUES(?, 'acc', ?, ?, ?, ?, '2026-01-01', '2026-01-01')",
        )
        .bind(id)
        .bind(folder_type)
        .bind(folder_type)
        .bind(folder_type)
        .bind(id)
        .execute(pool)
        .await
        .expect("trash folder");
    }

    async fn mark_old_done(pool: &sqlx::SqlitePool, thread_id: &str) {
        sqlx::query(
            "UPDATE threads
             SET workflow_state = 'done',
                 workflow_changed_at = '2026-01-01T00:00:00Z',
                 updated_at = '2026-01-01T00:00:00Z'
             WHERE id = ?",
        )
        .bind(thread_id)
        .execute(pool)
        .await
        .expect("old done thread");
    }

    #[tokio::test]
    async fn dense_folder_pages_are_limited_before_the_full_projection() {
        use sqlx::Execute;
        let pool = migrated_pool().await;
        let mut query = thread_list_query(
            &ThreadListRequest {
                mailbox_kind: Some("section".into()),
                mailbox_value: Some("inbox".into()),
                ..Default::default()
            },
            true,
        )
        .unwrap();
        let sql = query.sql();
        assert!(sql.starts_with("WITH listed_thread_ids AS MATERIALIZED"));
        let limit = sql.find("LIMIT ?").unwrap();
        assert!(limit < sql.find(" AS smart_bucket").unwrap());
        assert!(limit < sql.find("LEFT JOIN messages latest_message").unwrap());
        let mut built = query.build();
        let sql = format!("EXPLAIN QUERY PLAN {}", built.sql());
        let args = built.take_arguments().unwrap().unwrap();
        let plan = sqlx::query_with(&sql, args).fetch_all(&pool).await.unwrap();
        let details = plan
            .iter()
            .map(|r| sqlx::Row::get::<String, _>(r, "detail"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            details.contains("COVERING INDEX idx_threads_inbox_date_id"),
            "{details}"
        );
        assert!(
            !details.contains("idx_messages_effective_folder_thread_read"),
            "{details}"
        );
    }

    #[tokio::test]
    async fn sparse_list_plans_start_from_the_filter() {
        use sqlx::Execute;
        let pool = migrated_pool().await;
        for request in [
            ThreadListRequest {
                mailbox_kind: Some("folder".into()),
                mailbox_value: Some("Sparse".into()),
                ..Default::default()
            },
            ThreadListRequest {
                mailbox_kind: Some("section".into()),
                mailbox_value: Some("drafts".into()),
                ..Default::default()
            },
            ThreadListRequest {
                contact_email: Some("contact@test.local".into()),
                ..Default::default()
            },
        ] {
            let mut builder = thread_list_query(&request, true).unwrap();
            let mut query = builder.build();
            let sql = format!("EXPLAIN QUERY PLAN {}", query.sql());
            let args = query.take_arguments().unwrap().unwrap();
            let plan = sqlx::query_with(&sql, args).fetch_all(&pool).await.unwrap();
            let details = plan
                .iter()
                .map(|r| sqlx::Row::get::<String, _>(r, "detail"))
                .collect::<Vec<_>>()
                .join("\n");
            assert!(!details.contains("SCAN threads"), "{details}");
            assert!(
                details.contains("SEARCH threads USING INDEX sqlite_autoindex_threads_1 (id=?)"),
                "{details}"
            );
            if request.contact_email.is_some() {
                assert!(
                    details.contains("COVERING INDEX idx_message_addresses_email_thread (email=?)"),
                    "{details}"
                );
                assert!(!details.contains("SCAN contact_messages"), "{details}");
            } else {
                assert!(
                    details.contains("idx_messages_effective_folder_thread_read (<expr>=?)"),
                    "{details}"
                );
            }
        }
    }

    #[tokio::test]
    async fn unchanged_snapshots_skip_writes_and_older_tasks_cannot_overwrite() {
        let pool = migrated_pool().await;
        let request = ThreadListRequest::default();
        let page = ThreadPageRecord {
            threads: Vec::new(),
            next_cursor: None,
            has_more: false,
        };
        let first = chrono::Utc::now().to_rfc3339();
        store_thread_list_snapshot(&pool, &request, &page, &first, 0)
            .await
            .unwrap();
        sqlx::raw_sql("CREATE TRIGGER reject_snapshot_update BEFORE UPDATE ON thread_list_snapshots BEGIN SELECT RAISE(ABORT, 'unexpected write'); END;
            CREATE TRIGGER reject_snapshot_delete BEFORE DELETE ON thread_list_snapshots BEGIN SELECT RAISE(ABORT, 'unexpected prune'); END;").execute(&pool).await.unwrap();
        store_thread_list_snapshot(&pool, &request, &page, &chrono::Utc::now().to_rfc3339(), 0)
            .await
            .unwrap();
        let older_page = ThreadPageRecord {
            has_more: true,
            ..page
        };
        store_thread_list_snapshot(&pool, &request, &older_page, "2025-01-01T00:00:00Z", 0)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn invalidated_snapshots_cannot_be_restored_by_an_in_flight_task() {
        let pool = migrated_pool().await;
        let mut tx = pool.begin().await.unwrap();
        clear_thread_list_snapshots_tx(&mut tx).await.unwrap();
        tx.commit().await.unwrap();
        let request = ThreadListRequest::default();
        let page = ThreadPageRecord {
            threads: Vec::new(),
            next_cursor: None,
            has_more: false,
        };
        store_thread_list_snapshot(&pool, &request, &page, &chrono::Utc::now().to_rfc3339(), 0)
            .await
            .unwrap();
        assert!(ThreadRepository::new(&pool)
            .list_cached(request)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn list_returns_while_a_writer_blocks_the_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let url = format!("sqlite://{}?mode=rwc", dir.path().join("mail.db").display());
        let pool = SqlitePoolOptions::new()
            .max_connections(3)
            .connect(&url)
            .await
            .unwrap();
        crate::db::init(&pool).await.unwrap();
        let writer = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
        let repository = ThreadRepository::new(&pool);
        let page = tokio::time::timeout(
            std::time::Duration::from_millis(250),
            repository.list(ThreadListRequest::default()),
        )
        .await
        .expect("list must not wait for a writer")
        .unwrap();
        assert!(page.threads.is_empty());
        writer.rollback().await.unwrap();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
        while repository
            .list_cached(ThreadListRequest::default())
            .await
            .unwrap()
            .is_none()
        {
            assert!(tokio::time::Instant::now() < deadline);
            tokio::task::yield_now().await;
        }
    }

    #[tokio::test]
    async fn repository_returns_typed_not_found_for_missing_thread() {
        let pool = migrated_pool().await;
        let error = ThreadRepository::new(&pool)
            .get("missing-thread")
            .await
            .expect_err("missing thread should fail");

        assert!(matches!(error, ThreadError::NotFound));
    }

    #[tokio::test]
    async fn repository_normalizes_thread_organization_and_tags() {
        let pool = migrated_pool().await;
        seed_thread(&pool).await;
        let repository = ThreadRepository::new(&pool);

        repository
            .update_organization(ThreadOrganizationUpdateRequest {
                thread_id: "  thread-1  ".into(),
                board_position: Some(12.5),
                assignee: Some("   ".into()),
                shared_with: vec![
                    " First@Example.com ".into(),
                    "first@example.com".into(),
                    "second@example.com".into(),
                ],
            })
            .await
            .expect("organization update");
        repository
            .set_tags(ThreadTagSetRequest {
                thread_ids: vec![" thread-1 ".into(), "THREAD-1".into()],
                tags: vec![" Invoice ".into(), "invoice".into()],
            })
            .await
            .expect("tag update");

        let organization: (Option<f64>, Option<String>, String, String) = sqlx::query_as(
            "SELECT board_position, assignee, shared_with, updated_at
             FROM threads WHERE id = 'thread-1'",
        )
        .fetch_one(&pool)
        .await
        .expect("organization row");
        assert_eq!(organization.0, Some(12.5));
        assert_eq!(organization.1, None);
        assert_eq!(
            serde_json::from_str::<Vec<String>>(&organization.2).expect("shared_with JSON"),
            vec!["First@Example.com", "second@example.com"]
        );
        assert_ne!(organization.3, "2026-01-01");
        let tags = sqlx::query_scalar::<_, String>(
            "SELECT tags.name FROM thread_tags
             JOIN tags ON tags.id = thread_tags.tag_id
             WHERE thread_tags.thread_id = 'thread-1'",
        )
        .fetch_all(&pool)
        .await
        .expect("tag rows");
        assert_eq!(tags, vec!["invoice"]);
    }

    #[tokio::test]
    async fn blocked_sender_selection_is_exact_and_projection_retry_state_is_local() {
        let pool = migrated_pool().await;
        seed_thread(&pool).await;
        insert_thread(&pool, "thread-2", "acc").await;
        insert_message(
            &pool,
            "message-1",
            "thread-1",
            "inbox",
            None,
            "remote-1",
            "2026-08-01T00:00:00Z",
            false,
            false,
            false,
            false,
            "Subject",
            "Snippet",
        )
        .await;
        insert_message(
            &pool,
            "message-2",
            "thread-2",
            "inbox",
            None,
            "remote-2",
            "2026-08-01T00:00:00Z",
            false,
            false,
            false,
            false,
            "Subject",
            "Snippet",
        )
        .await;
        sqlx::query("UPDATE messages SET from_address = ? WHERE id = ?")
            .bind("Alice@Example.com")
            .bind("message-1")
            .execute(&pool)
            .await
            .expect("sender");
        sqlx::query("UPDATE messages SET from_address = ? WHERE id = ?")
            .bind("alice@notexample.com")
            .bind("message-2")
            .execute(&pool)
            .await
            .expect("sender");

        let repository = ThreadRepository::new(&pool);
        assert_eq!(
            repository
                .find_matching_sender_threads(
                    Some("acc"),
                    "alice@example.com",
                    BlockedSenderPatternType::Address,
                )
                .await
                .expect("address selection"),
            vec!["thread-1"]
        );
        assert_eq!(
            repository
                .find_matching_sender_threads(
                    Some("acc"),
                    "example.com",
                    BlockedSenderPatternType::Domain,
                )
                .await
                .expect("domain selection"),
            vec!["thread-1"]
        );
        assert!(
            repository
                .find_matching_sender_threads(
                    Some("acc"),
                    "alice_%",
                    BlockedSenderPatternType::Address,
                )
                .await
                .expect("underscore address selection")
                .is_empty()
        );
        assert!(repository
            .find_matching_sender_threads(
                Some("acc"),
                "alice%@example.com",
                BlockedSenderPatternType::Address,
            )
            .await
            .expect("percent address selection")
            .is_empty());
        sqlx::query("UPDATE accounts SET kind = 'feed' WHERE id = 'acc'")
            .execute(&pool)
            .await
            .expect("non-mail account");
        assert!(repository
            .find_matching_sender_threads(
                Some("acc"),
                "alice@example.com",
                BlockedSenderPatternType::Address,
            )
            .await
            .expect("non-mail sender selection")
            .is_empty());
        assert!(repository
            .blocked_projection_pending("thread-1")
            .await
            .expect("pending blocked projection"));

        repository
            .commit_blocked_moves(&SyncQueue::new(), &["thread-1".to_string()], &[])
            .await
            .expect("blocked projection");
        let state: (String, i64) = sqlx::query_as(
            "SELECT workflow_state, unread_count FROM threads WHERE id = 'thread-1'",
        )
        .fetch_one(&pool)
        .await
        .expect("thread projection");
        assert_eq!(state, ("done".into(), 0));
        assert!(!repository
            .blocked_projection_pending("thread-1")
            .await
            .expect("completed blocked projection"));

        // A path that sets the state without the mirror column is still done.
        sqlx::query("UPDATE threads SET board_column = '' WHERE id = 'thread-1'")
            .execute(&pool)
            .await
            .expect("clear mirror");
        assert!(!repository
            .blocked_projection_pending("thread-1")
            .await
            .expect("done without board column"));
    }

    #[tokio::test]
    async fn blocked_thread_converges_to_zero_unread_after_a_second_message() {
        let pool = migrated_pool().await;
        seed_thread(&pool).await;
        let repository = ThreadRepository::new(&pool);
        let queue = SyncQueue::new();

        // The ingest refreshes the projection before it reaches the blocked
        // check, so replay that order for both messages.
        for (index, id) in ["message-1", "message-2"].into_iter().enumerate() {
            insert_message(
                &pool,
                id,
                "thread-1",
                "inbox",
                None,
                &format!("remote-{index}"),
                "2026-08-01T00:00:00Z",
                false,
                false,
                false,
                false,
                "Subject",
                "Snippet",
            )
            .await;
            repository
                .refresh_projection(&["thread-1".to_string()])
                .await
                .expect("ingest projection refresh");
            repository
                .commit_blocked_moves(&queue, &["thread-1".to_string()], &[])
                .await
                .expect("blocked commit");
        }

        // One more refresh: the count has to survive being derived again.
        repository
            .refresh_projection(&["thread-1".to_string()])
            .await
            .expect("later projection refresh");

        let state: (String, i64) = sqlx::query_as(
            "SELECT workflow_state, unread_count FROM threads WHERE id = 'thread-1'",
        )
        .fetch_one(&pool)
        .await
        .expect("thread projection");
        assert_eq!(state, ("done".into(), 0));

        let unread_messages: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM messages WHERE thread_id = 'thread-1' AND is_read = 0",
        )
        .fetch_one(&pool)
        .await
        .expect("message read state");
        assert_eq!(unread_messages, 0);
    }

    #[tokio::test]
    async fn blocked_move_commit_rolls_back_queue_when_thread_projection_fails() {
        let pool = migrated_pool().await;
        seed_thread(&pool).await;
        sqlx::query(
            "CREATE TRIGGER fail_blocked_thread_projection
             BEFORE UPDATE OF workflow_state ON threads
             BEGIN SELECT RAISE(ABORT, 'blocked thread projection failure'); END",
        )
        .execute(&pool)
        .await
        .expect("projection failure trigger");
        let action = QueuedAction::new(
            "acc".into(),
            "remote_mutation",
            serde_json::json!({
                "kind": "blocked_move",
                "blocked_thread_id": "thread-1",
                "refs": [{"remote_id": "remote-1", "source_folder": "inbox"}],
                "target_remote_folder_id": "spam",
                "provider_move_started": false,
                "provider_move_completed": false,
                "source_uidvalidity_before": null
            }),
        );
        let error = ThreadRepository::new(&pool)
            .commit_blocked_moves(&SyncQueue::new(), &["thread-1".into()], &[action])
            .await
            .expect_err("projection failure should roll back");
        assert!(matches!(error, ThreadError::Persistence(_)));
        let queued: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_queue")
            .fetch_one(&pool)
            .await
            .expect("queue count");
        assert_eq!(queued, 0);
        let state: String = sqlx::query_scalar(
            "SELECT COALESCE(workflow_state, '') FROM threads WHERE id = 'thread-1'",
        )
        .fetch_one(&pool)
        .await
        .expect("thread state");
        assert_ne!(state, "done");
    }

    #[tokio::test]
    async fn blocked_move_commit_rolls_back_thread_projection_when_queue_fails() {
        let pool = migrated_pool().await;
        seed_thread(&pool).await;
        sqlx::query(
            "CREATE TRIGGER fail_blocked_queue
             BEFORE INSERT ON sync_queue
             BEGIN SELECT RAISE(ABORT, 'blocked queue failure'); END",
        )
        .execute(&pool)
        .await
        .expect("queue failure trigger");
        let action = QueuedAction::new(
            "acc".into(),
            "remote_mutation",
            serde_json::json!({ "kind": "blocked_move" }),
        );
        let error = ThreadRepository::new(&pool)
            .commit_blocked_moves(&SyncQueue::new(), &["thread-1".into()], &[action])
            .await
            .expect_err("queue failure should roll back");
        assert!(matches!(error, ThreadError::Queue(_)));
        let queued: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_queue")
            .fetch_one(&pool)
            .await
            .expect("queue count");
        assert_eq!(queued, 0);
        let state: String = sqlx::query_scalar(
            "SELECT COALESCE(workflow_state, '') FROM threads WHERE id = 'thread-1'",
        )
        .fetch_one(&pool)
        .await
        .expect("thread state");
        assert_ne!(state, "done");
    }

    #[tokio::test]
    async fn blocked_move_commit_reloads_folder_scoped_refs_under_its_write_lock() {
        let pool = migrated_pool().await;
        seed_thread(&pool).await;
        insert_message(
            &pool,
            "message-1",
            "thread-1",
            "inbox",
            None,
            "remote-1",
            "2026-08-01T00:00:00Z",
            false,
            false,
            false,
            false,
            "Subject",
            "Snippet",
        )
        .await;

        // This is the state a folder sync can create after the caller's
        // preflight selected the Inbox copy. The commit must not persist the
        // stale Inbox-scoped UID.
        sqlx::query("UPDATE messages SET folder_id = 'archive' WHERE id = 'message-1'")
            .execute(&pool)
            .await
            .expect("move local copy before commit");
        let action = QueuedAction::new(
            "acc".into(),
            "remote_mutation",
            serde_json::json!({
                "kind": "blocked_move",
                "blocked_thread_id": "thread-1",
                "refs": [{"remote_id": "remote-1", "source_folder": "inbox"}],
                "target_remote_folder_id": "spam",
                "provider_move_started": false,
                "provider_move_completed": false,
                "source_uidvalidity_before": null
            }),
        );
        ThreadRepository::new(&pool)
            .commit_blocked_moves(&SyncQueue::new(), &["thread-1".into()], &[action])
            .await
            .expect("commit refreshed refs");

        let payload: String = sqlx::query_scalar(
            "SELECT payload FROM sync_queue WHERE action_type = 'remote_mutation'",
        )
        .fetch_one(&pool)
        .await
        .expect("queued blocked move");
        let payload: serde_json::Value = serde_json::from_str(&payload).expect("payload JSON");
        assert_eq!(payload["refs"][0]["remote_id"], "remote-1");
        assert_eq!(payload["refs"][0]["source_folder"], "archive");
    }

    #[tokio::test]
    async fn blocked_move_commit_carries_imap_generation_into_identity_and_payload() {
        let pool = migrated_pool().await;
        seed_thread(&pool).await;
        sqlx::query("UPDATE folders SET uidvalidity = 41 WHERE id = 'inbox'")
            .execute(&pool)
            .await
            .expect("source generation");
        insert_message(
            &pool,
            "message-1",
            "thread-1",
            "inbox",
            None,
            "remote-1",
            "2026-08-01T00:00:00Z",
            false,
            false,
            false,
            false,
            "Subject",
            "Snippet",
        )
        .await;
        let action = QueuedAction::new(
            "acc".into(),
            "remote_mutation",
            serde_json::json!({
                "kind": "blocked_move",
                "blocked_thread_id": "thread-1",
                "refs": [{"remote_id": "remote-1", "source_folder": "inbox"}],
                "target_remote_folder_id": "spam",
                "provider_move_started": false,
                "provider_move_completed": false,
                "source_uidvalidity_before": null
            }),
        );
        ThreadRepository::new(&pool)
            .commit_blocked_moves(&SyncQueue::new(), &["thread-1".into()], &[action])
            .await
            .expect("generation-aware blocked move");
        let (id, payload): (String, String) = sqlx::query_as(
            "SELECT id, payload FROM sync_queue WHERE action_type = 'remote_mutation'",
        )
        .fetch_one(&pool)
        .await
        .expect("blocked move");
        let payload: Value = serde_json::from_str(&payload).expect("payload JSON");
        let message_ref: MessageRef =
            serde_json::from_value(payload["refs"][0].clone()).expect("message ref");
        assert_eq!(message_ref.source_uidvalidity, Some(41));
        assert_eq!(payload["source_uidvalidity_before"]["inbox"], 41);
        assert_eq!(
            id,
            crate::sync::remote_mutations::stable_blocked_move_id(
                "thread-1",
                "acc",
                &message_ref,
                "spam",
            )
        );
    }

    #[tokio::test]
    async fn repository_validates_organization_and_tag_limits() {
        let pool = migrated_pool().await;
        seed_thread(&pool).await;
        let repository = ThreadRepository::new(&pool);

        let error = repository
            .update_organization(ThreadOrganizationUpdateRequest {
                thread_id: "   ".into(),
                board_position: None,
                assignee: None,
                shared_with: Vec::new(),
            })
            .await
            .expect_err("empty thread id");
        assert!(matches!(error, ThreadError::EmptyThreadId));

        let error = repository
            .update_organization(ThreadOrganizationUpdateRequest {
                thread_id: "thread-1".into(),
                board_position: Some(f64::NAN),
                assignee: None,
                shared_with: Vec::new(),
            })
            .await
            .expect_err("non-finite board position");
        assert!(matches!(error, ThreadError::NonFiniteBoardPosition));

        let error = repository
            .update_organization(ThreadOrganizationUpdateRequest {
                thread_id: "missing".into(),
                board_position: None,
                assignee: None,
                shared_with: Vec::new(),
            })
            .await
            .expect_err("missing organization thread");
        assert!(matches!(error, ThreadError::NotFound));

        let error = repository
            .set_tags(ThreadTagSetRequest {
                thread_ids: vec!["thread-1".into()],
                tags: vec!["line\nbreak".into()],
            })
            .await
            .expect_err("control character");
        assert!(matches!(
            error,
            ThreadError::MetadataControlCharacters { field: "tags" }
        ));

        let error = repository
            .set_tags(ThreadTagSetRequest {
                thread_ids: vec!["thread-1".into()],
                tags: vec![String::new(); 51],
            })
            .await
            .expect_err("raw tag limit");
        assert!(matches!(
            error,
            ThreadError::TooManyMetadataValues {
                field: "tags",
                max: 50
            }
        ));

        let error = repository
            .set_tags(ThreadTagSetRequest {
                thread_ids: vec!["thread-1".into()],
                tags: vec!["x".repeat(81)],
            })
            .await
            .expect_err("tag length limit");
        assert!(matches!(
            error,
            ThreadError::MetadataValueTooLong {
                field: "tags",
                max: 80
            }
        ));

        let error = repository
            .set_tags(ThreadTagSetRequest {
                thread_ids: vec![String::new(); 501],
                tags: Vec::new(),
            })
            .await
            .expect_err("raw thread id limit");
        assert!(matches!(
            error,
            ThreadError::TooManyMetadataValues {
                field: "thread_ids",
                max: 500
            }
        ));

        let error = repository
            .update_organization(ThreadOrganizationUpdateRequest {
                thread_id: "thread-1".into(),
                board_position: None,
                assignee: None,
                shared_with: vec![String::new(); 101],
            })
            .await
            .expect_err("raw collaborator limit");
        assert!(matches!(
            error,
            ThreadError::TooManyMetadataValues {
                field: "shared_with",
                max: 100
            }
        ));

        repository
            .set_tags(ThreadTagSetRequest {
                thread_ids: Vec::new(),
                tags: vec!["x".repeat(81)],
            })
            .await
            .expect("empty thread selection bypasses tag validation");
    }

    #[tokio::test]
    async fn repository_set_tags_rolls_back_when_any_thread_is_missing() {
        let pool = migrated_pool().await;
        seed_thread(&pool).await;
        sqlx::query("INSERT INTO tags(id, name) VALUES('old-tag', 'old')")
            .execute(&pool)
            .await
            .expect("old tag");
        sqlx::query("INSERT INTO thread_tags(thread_id, tag_id) VALUES('thread-1', 'old-tag')")
            .execute(&pool)
            .await
            .expect("old thread tag");

        let error = ThreadRepository::new(&pool)
            .set_tags(ThreadTagSetRequest {
                thread_ids: vec!["thread-1".into(), "missing".into()],
                tags: vec!["new".into()],
            })
            .await
            .expect_err("missing thread");
        assert!(matches!(error, ThreadError::NotFound));

        let links: Vec<String> = sqlx::query_scalar(
            "SELECT tag_id FROM thread_tags WHERE thread_id = 'thread-1' ORDER BY tag_id",
        )
        .fetch_all(&pool)
        .await
        .expect("thread tags");
        assert_eq!(links, vec!["old-tag"]);
        let new_tags: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tags WHERE name = 'new'")
            .fetch_one(&pool)
            .await
            .expect("new tag count");
        assert_eq!(new_tags, 0);
    }

    #[tokio::test]
    async fn repository_set_tags_rolls_back_changes_and_new_tags_on_write_failure() {
        let pool = migrated_pool().await;
        seed_thread(&pool).await;
        insert_thread(&pool, "thread-2", "acc").await;
        sqlx::query("INSERT INTO tags(id, name) VALUES('old-tag', 'old')")
            .execute(&pool)
            .await
            .expect("old tag");
        sqlx::query(
            "INSERT INTO thread_tags(thread_id, tag_id)
             VALUES('thread-1', 'old-tag'), ('thread-2', 'old-tag')",
        )
        .execute(&pool)
        .await
        .expect("old thread tags");
        sqlx::query(
            "CREATE TRIGGER fail_second_thread_tag
             BEFORE INSERT ON thread_tags
             WHEN NEW.thread_id = 'thread-2' AND NEW.tag_id != 'old-tag'
             BEGIN SELECT RAISE(ABORT, 'private tag rollback detail'); END",
        )
        .execute(&pool)
        .await
        .expect("failure trigger");

        let error = ThreadRepository::new(&pool)
            .set_tags(ThreadTagSetRequest {
                thread_ids: vec!["thread-1".into(), "thread-2".into()],
                tags: vec!["new".into()],
            })
            .await
            .expect_err("second thread tag insert should fail");
        assert!(matches!(error, ThreadError::Persistence(_)));

        let links: Vec<(String, String)> =
            sqlx::query_as("SELECT thread_id, tag_id FROM thread_tags ORDER BY thread_id, tag_id")
                .fetch_all(&pool)
                .await
                .expect("thread tags after rollback");
        assert_eq!(
            links,
            vec![
                ("thread-1".into(), "old-tag".into()),
                ("thread-2".into(), "old-tag".into())
            ]
        );
        let new_tags: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tags WHERE name = 'new'")
            .fetch_one(&pool)
            .await
            .expect("new tag count after rollback");
        assert_eq!(new_tags, 0);
    }

    #[tokio::test]
    async fn repository_set_tags_reuses_the_lowest_case_insensitive_tag_id_and_clears_links() {
        let pool = migrated_pool().await;
        seed_thread(&pool).await;
        sqlx::query("INSERT INTO tags(id, name) VALUES('z-tag', 'Invoice'), ('a-tag', 'INVOICE')")
            .execute(&pool)
            .await
            .expect("existing tags");
        let repository = ThreadRepository::new(&pool);

        repository
            .set_tags(ThreadTagSetRequest {
                thread_ids: vec!["thread-1".into()],
                tags: vec!["invoice".into()],
            })
            .await
            .expect("reuse tag");
        let tag_id: String =
            sqlx::query_scalar("SELECT tag_id FROM thread_tags WHERE thread_id = 'thread-1'")
                .fetch_one(&pool)
                .await
                .expect("tag id");
        assert_eq!(tag_id, "a-tag");

        repository
            .set_tags(ThreadTagSetRequest {
                thread_ids: vec!["thread-1".into()],
                tags: Vec::new(),
            })
            .await
            .expect("clear tags");
        let links: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM thread_tags WHERE thread_id = 'thread-1'")
                .fetch_one(&pool)
                .await
                .expect("tag link count");
        assert_eq!(links, 0);
    }

    #[tokio::test]
    async fn repository_update_state_persists_workflow_board_and_snooze_semantics() {
        let pool = migrated_pool().await;
        seed_thread(&pool).await;
        let repository = ThreadRepository::new(&pool);

        repository
            .update_state(ThreadStateUpdateRequest {
                thread_id: "thread-1".into(),
                next_state: WorkflowState::Waiting,
                snooze_until: Some("2026-08-02T00:00:00Z".into()),
            })
            .await
            .expect("supplied snooze");
        let supplied = sqlx::query_as::<_, (String, String, Option<String>)>(
            "SELECT workflow_state, board_column, snooze_until FROM threads WHERE id = 'thread-1'",
        )
        .fetch_one(&pool)
        .await
        .expect("supplied state");
        assert_eq!(supplied, ("waiting".into(), "waiting".into(), None,));

        repository
            .update_state(ThreadStateUpdateRequest {
                thread_id: "thread-1".into(),
                next_state: WorkflowState::Snoozed,
                snooze_until: Some("2026-08-03T00:00:00Z".into()),
            })
            .await
            .expect("explicit snooze");
        let explicit_snooze = sqlx::query_scalar::<_, Option<String>>(
            "SELECT snooze_until FROM threads WHERE id = 'thread-1'",
        )
        .fetch_one(&pool)
        .await
        .expect("explicit snooze value");
        assert_eq!(explicit_snooze.as_deref(), Some("2026-08-03T00:00:00Z"));

        repository
            .update_state(ThreadStateUpdateRequest {
                thread_id: "thread-1".into(),
                next_state: WorkflowState::Snoozed,
                snooze_until: None,
            })
            .await
            .expect("default snooze");
        let default_snooze = sqlx::query_scalar::<_, Option<String>>(
            "SELECT snooze_until FROM threads WHERE id = 'thread-1'",
        )
        .fetch_one(&pool)
        .await
        .expect("default snooze value")
        .expect("default snooze should be set");
        let parsed_default = chrono::DateTime::parse_from_rfc3339(&default_snooze)
            .expect("default snooze timestamp");
        let now = chrono::Utc::now();
        let parsed_default = parsed_default.with_timezone(&chrono::Utc);
        assert!(parsed_default > now);
        assert!(parsed_default < now + chrono::Duration::hours(2) + chrono::Duration::minutes(1));

        repository
            .update_state(ThreadStateUpdateRequest {
                thread_id: "thread-1".into(),
                next_state: WorkflowState::Done,
                snooze_until: None,
            })
            .await
            .expect("clear snooze");
        let cleared = sqlx::query_as::<_, (String, String, Option<String>)>(
            "SELECT workflow_state, board_column, snooze_until FROM threads WHERE id = 'thread-1'",
        )
        .fetch_one(&pool)
        .await
        .expect("cleared state");
        assert_eq!(cleared.0, "done");
        assert_eq!(cleared.1, "done");
        assert_eq!(cleared.2, None);
    }

    #[tokio::test]
    async fn repository_refresh_projection_deduplicates_and_tracks_read_state() {
        let pool = migrated_pool().await;
        seed_thread(&pool).await;

        // Two IMAP copies with the same real Message-ID are one logical
        // message. Synthetic IMAP ids remain folder/remote scoped.
        insert_message(
            &pool,
            "m-imap-inbox-1",
            "thread-1",
            "inbox",
            Some("<same@example.com>"),
            "uid-1",
            "2026-08-01T00:00:00Z",
            false,
            false,
            false,
            false,
            "Old",
            "old snippet",
        )
        .await;
        insert_message(
            &pool,
            "m-imap-archive-1",
            "thread-1",
            "archive",
            Some("<same@example.com>"),
            "uid-2",
            "2026-08-02T00:00:00Z",
            false,
            true,
            false,
            true,
            "Copied",
            "copied snippet",
        )
        .await;
        insert_message(
            &pool,
            "m-imap-inbox-2",
            "thread-1",
            "inbox",
            Some("<synthetic@imap.satchel.local>"),
            "uid-3",
            "2026-08-03T00:00:00Z",
            false,
            false,
            false,
            false,
            "Latest",
            "latest snippet",
        )
        .await;
        // A newer draft must not replace the latest received message.
        insert_message(
            &pool,
            "m-imap-draft",
            "thread-1",
            "drafts",
            Some("<draft@imap.satchel.local>"),
            "draft-1",
            "2026-08-04T00:00:00Z",
            false,
            false,
            true,
            false,
            "Draft subject",
            "Draft snippet",
        )
        .await;

        ThreadRepository::new(&pool)
            .refresh_projection(&["thread-1".into()])
            .await
            .expect("refresh projection");

        let projection = sqlx::query_as::<_, (i64, i64, i64, i64, String, String, String)>(
            "SELECT message_count, unread_count, is_starred, has_attachments,
                    subject, snippet, last_message_date
             FROM threads WHERE id = 'thread-1'",
        )
        .fetch_one(&pool)
        .await
        .expect("projection");
        assert_eq!(projection.0, 3);
        assert_eq!(projection.1, 3);
        assert_eq!(projection.2, 1);
        assert_eq!(projection.3, 1);
        assert_eq!(projection.4, "Latest");
        assert_eq!(projection.5, "latest snippet");
        assert_eq!(projection.6, "2026-08-03T00:00:00Z");

        ThreadRepository::new(&pool)
            .set_read_state(&["thread-1".into()], true)
            .await
            .expect("mark read");
        let read_state = sqlx::query_as::<_, (i64, i64)>(
            "SELECT (SELECT COUNT(*) FROM messages WHERE is_read = 0), unread_count
             FROM threads WHERE id = 'thread-1'",
        )
        .fetch_one(&pool)
        .await
        .expect("read state");
        assert_eq!(read_state, (0, 0));
    }

    #[tokio::test]
    async fn repository_set_read_state_rolls_back_messages_when_projection_fails() {
        let pool = migrated_pool().await;
        seed_thread(&pool).await;
        insert_message(
            &pool,
            "m-1",
            "thread-1",
            "inbox",
            Some("<first@example.com>"),
            "remote-1",
            "2026-08-05T00:00:00Z",
            false,
            false,
            false,
            false,
            "First message",
            "First snippet",
        )
        .await;
        sqlx::query(
            "CREATE TRIGGER fail_thread_projection
             BEFORE UPDATE ON threads
             BEGIN
               SELECT RAISE(ABORT, 'projection failed');
             END",
        )
        .execute(&pool)
        .await
        .expect("failure trigger");

        let error = ThreadRepository::new(&pool)
            .set_read_state(&["thread-1".into()], true)
            .await
            .expect_err("projection must fail");
        assert!(matches!(error, ThreadError::Persistence(_)));

        let is_read: i64 = sqlx::query_scalar("SELECT is_read FROM messages WHERE id = 'm-1'")
            .fetch_one(&pool)
            .await
            .expect("message state");
        assert_eq!(is_read, 0);
    }

    #[tokio::test]
    async fn repository_refresh_projection_handles_multiple_and_missing_ids() {
        let pool = migrated_pool().await;
        seed_thread(&pool).await;
        insert_message(
            &pool,
            "m-1",
            "thread-1",
            "inbox",
            Some("<first@example.com>"),
            "remote-1",
            "2026-08-05T00:00:00Z",
            true,
            false,
            false,
            false,
            "First message",
            "First snippet",
        )
        .await;
        sqlx::query(
            "INSERT INTO threads(id, account_id, subject, last_message_date, created_at, updated_at)
             VALUES('thread-2', 'acc', 'Second', '2026-08-01T00:00:00Z', '2026-01-01', '2026-01-01')",
        )
        .execute(&pool)
        .await
        .expect("second thread");
        insert_message(
            &pool,
            "m-2",
            "thread-2",
            "inbox",
            Some("<second@example.com>"),
            "remote-2",
            "2026-08-05T00:00:00Z",
            true,
            false,
            false,
            false,
            "Second message",
            "Second snippet",
        )
        .await;

        ThreadRepository::new(&pool)
            .refresh_projection(&["thread-1".into(), "thread-2".into(), "missing".into()])
            .await
            .expect("multi-id refresh");

        let counts = sqlx::query_as::<_, (i64, i64)>(
            "SELECT message_count, unread_count FROM threads
             WHERE id IN ('thread-1', 'thread-2') ORDER BY id",
        )
        .fetch_all(&pool)
        .await
        .expect("counts");
        assert_eq!(counts, vec![(1, 0), (1, 0)]);
    }

    #[tokio::test]
    async fn repository_purge_trash_removes_the_full_thread_aggregate() {
        let pool = migrated_pool().await;
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .expect("foreign keys");
        seed_thread(&pool).await;
        insert_trash_folder(&pool, "trash", "  TrAsH  ").await;
        insert_thread(&pool, "purge-thread", "acc").await;
        mark_old_done(&pool, "purge-thread").await;
        insert_message(
            &pool,
            "purge-message",
            "purge-thread",
            "trash",
            Some("<purge@example.com>"),
            "purge-remote",
            "2026-01-01T00:00:00Z",
            false,
            false,
            false,
            true,
            "Purgeable message",
            "purgeable snippet",
        )
        .await;

        sqlx::query(
            "INSERT INTO attachments
             (id, message_id, filename, content_type, size, cached_path, created_at)
             VALUES('purge-attachment', 'purge-message', 'file.txt', 'text/plain', 4,
                    '/tmp/purge-file', '2026-01-01')",
        )
        .execute(&pool)
        .await
        .expect("attachment");
        sqlx::query("INSERT INTO tags(id, name) VALUES('purge-tag', 'Purge')")
            .execute(&pool)
            .await
            .expect("tag");
        sqlx::query(
            "INSERT INTO thread_tags(thread_id, tag_id)
             VALUES('purge-thread', 'purge-tag')",
        )
        .execute(&pool)
        .await
        .expect("thread tag");
        sqlx::query(
            "INSERT INTO workflow_events
             (id, thread_id, from_state, to_state, reason, created_at)
             VALUES('purge-event', 'purge-thread', 'inbox', 'done', 'test', '2026-01-01')",
        )
        .execute(&pool)
        .await
        .expect("workflow event");
        sqlx::query(
            "INSERT INTO topics(id, name, created_at, updated_at)
             VALUES('purge-topic', 'Purge', '2026-01-01', '2026-01-01')",
        )
        .execute(&pool)
        .await
        .expect("topic");
        sqlx::query(
            "INSERT INTO topic_items(topic_id, item_type, item_id, added_at)
             VALUES('purge-topic', 'thread', 'purge-thread', '2026-01-01'),
                   ('purge-topic', 'message', 'purge-message', '2026-01-01')",
        )
        .execute(&pool)
        .await
        .expect("topic links");
        sqlx::query(
            "INSERT INTO notification_outbox(message_id, state, created_at)
             VALUES('purge-message', 'pending', '2026-01-01')",
        )
        .execute(&pool)
        .await
        .expect("notification outbox");
        sqlx::query(
            "INSERT INTO follow_up_reminders
             (id, account_id, thread_id, message_id, due_at, created_at, updated_at)
             VALUES('purge-followup', 'acc', 'purge-thread', 'purge-message',
                    '2026-01-02', '2026-01-01', '2026-01-01')",
        )
        .execute(&pool)
        .await
        .expect("follow-up");
        sqlx::query(
            "INSERT INTO calendar_invites
             (id, message_id, account_id, uid, raw_ics, created_at, updated_at)
             VALUES('purge-invite', 'purge-message', 'acc', 'purge-uid',
                    'BEGIN:VCALENDAR', '2026-01-01', '2026-01-01')",
        )
        .execute(&pool)
        .await
        .expect("calendar invite");

        let fts_before: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM messages_fts WHERE messages_fts MATCH 'Purgeable'",
        )
        .fetch_one(&pool)
        .await
        .expect("fts before purge");
        assert_eq!(fts_before, 1);
        let search_projection_before: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM message_search_projection
             WHERE message_id = 'purge-message'",
        )
        .fetch_one(&pool)
        .await
        .expect("search projection before purge");
        assert_eq!(search_projection_before, 1);

        let removed = ThreadRepository::new(&pool)
            .purge_trash_before("2026-08-01T00:00:00Z")
            .await
            .expect("purge");
        assert_eq!(removed, 1);

        for (table, column, value) in [
            ("threads", "id", "purge-thread"),
            ("messages", "id", "purge-message"),
            ("attachments", "id", "purge-attachment"),
            ("thread_tags", "thread_id", "purge-thread"),
            ("workflow_events", "thread_id", "purge-thread"),
            ("notification_outbox", "message_id", "purge-message"),
            ("follow_up_reminders", "thread_id", "purge-thread"),
        ] {
            let count: i64 = sqlx::query_scalar(&format!(
                "SELECT COUNT(*) FROM {table} WHERE {column} = '{value}'",
            ))
            .fetch_one(&pool)
            .await
            .expect("deleted aggregate rows");
            assert_eq!(count, 0, "{table} still references purged aggregate");
        }
        let topic_links: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM topic_items WHERE item_id IN ('purge-thread', 'purge-message')",
        )
        .fetch_one(&pool)
        .await
        .expect("topic links");
        assert_eq!(topic_links, 0);
        let invite_message_id: Option<String> =
            sqlx::query_scalar("SELECT message_id FROM calendar_invites WHERE id = 'purge-invite'")
                .fetch_one(&pool)
                .await
                .expect("calendar invite");
        assert_eq!(invite_message_id, None);
        let fts_after: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM messages_fts WHERE messages_fts MATCH 'Purgeable'",
        )
        .fetch_one(&pool)
        .await
        .expect("fts after purge");
        assert_eq!(fts_after, 0);
        let search_projection_after: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM message_search_projection
             WHERE message_id = 'purge-message'",
        )
        .fetch_one(&pool)
        .await
        .expect("search projection after purge");
        assert_eq!(search_projection_after, 0);
    }

    #[tokio::test]
    async fn repository_purge_trash_requires_only_old_done_all_trash_threads() {
        let pool = migrated_pool().await;
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .expect("foreign keys");
        seed_thread(&pool).await;
        insert_trash_folder(&pool, "trash", "trash").await;
        insert_trash_folder(&pool, "orphan-folder", "trash").await;

        for id in [
            "eligible",
            "mixed",
            "missing-folder",
            "not-done",
            "recent",
            "no-trash",
            "pending-move",
            "failed-move",
        ] {
            insert_thread(&pool, id, "acc").await;
        }
        for id in [
            "eligible",
            "mixed",
            "missing-folder",
            "not-done",
            "recent",
            "no-trash",
            "pending-move",
            "failed-move",
        ] {
            mark_old_done(&pool, id).await;
        }
        sqlx::query("UPDATE threads SET workflow_changed_at = NULL WHERE id = 'eligible'")
            .execute(&pool)
            .await
            .expect("updated_at fallback");
        sqlx::query(
            "UPDATE threads
             SET workflow_state = 'inbox' WHERE id = 'not-done'",
        )
        .execute(&pool)
        .await
        .expect("not done");
        sqlx::query(
            "UPDATE threads
             SET workflow_changed_at = '2026-08-15T00:00:00Z' WHERE id = 'recent'",
        )
        .execute(&pool)
        .await
        .expect("recent");

        insert_message(
            &pool,
            "eligible-message",
            "eligible",
            "trash",
            Some("<eligible@example.com>"),
            "eligible-remote",
            "2026-01-01T00:00:00Z",
            true,
            false,
            false,
            false,
            "Eligible",
            "Eligible",
        )
        .await;
        for (thread, message, folder) in [
            ("mixed", "mixed-trash", "trash"),
            ("mixed", "mixed-inbox", "inbox"),
            ("missing-folder", "missing-message", "orphan-folder"),
            ("not-done", "not-done-message", "trash"),
            ("recent", "recent-message", "trash"),
            ("no-trash", "archive-message", "archive"),
            ("pending-move", "pending-message", "trash"),
            ("failed-move", "failed-message", "trash"),
        ] {
            insert_message(
                &pool,
                message,
                thread,
                folder,
                Some("<candidate@example.com>"),
                message,
                "2026-01-01T00:00:00Z",
                true,
                false,
                false,
                false,
                thread,
                thread,
            )
            .await;
        }

        // Build an intentionally broken fixture while keeping foreign-key
        // enforcement enabled for the repository operation itself.
        sqlx::query("PRAGMA foreign_keys = OFF")
            .execute(&pool)
            .await
            .expect("disable foreign keys for fixture");
        sqlx::query("DELETE FROM folders WHERE id = 'orphan-folder'")
            .execute(&pool)
            .await
            .expect("orphan folder");
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .expect("re-enable foreign keys");

        for (id, status) in [("pending-move", "pending"), ("failed-move", "failed")] {
            sqlx::query(
                "INSERT INTO sync_queue
                 (id, account_id, action_type, payload, status, created_at, updated_at)
                 VALUES(?, 'acc', 'remote_mutation', ?, ?, '2026-01-01', '2026-01-01')",
            )
            .bind(format!("queue-{id}"))
            .bind(format!(
                "{{\"kind\":\"workflow_move\",\"workflow_thread_id\":\"{id}\"}}"
            ))
            .bind(status)
            .execute(&pool)
            .await
            .expect("workflow move");
        }

        let removed = ThreadRepository::new(&pool)
            .purge_trash_before("2026-08-01T00:00:00Z")
            .await
            .expect("purge candidates");
        assert_eq!(removed, 1);
        for id in [
            "eligible",
            "mixed",
            "missing-folder",
            "not-done",
            "recent",
            "no-trash",
            "pending-move",
            "failed-move",
        ] {
            let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM threads WHERE id = ?")
                .bind(id)
                .fetch_one(&pool)
                .await
                .expect("candidate thread");
            let expected = if id == "eligible" { 0 } else { 1 };
            assert_eq!(count, expected, "unexpected {id} result");
        }
    }

    #[tokio::test]
    async fn repository_purge_trash_rolls_back_all_deletes_on_error() {
        let pool = migrated_pool().await;
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .expect("foreign keys");
        seed_thread(&pool).await;
        insert_trash_folder(&pool, "trash", "trash").await;
        insert_thread(&pool, "rollback-thread", "acc").await;
        mark_old_done(&pool, "rollback-thread").await;
        insert_message(
            &pool,
            "rollback-message",
            "rollback-thread",
            "trash",
            Some("<rollback@example.com>"),
            "rollback-remote",
            "2026-01-01T00:00:00Z",
            true,
            false,
            false,
            false,
            "Rollback",
            "Rollback",
        )
        .await;
        sqlx::query(
            "INSERT INTO attachments
             (id, message_id, filename, content_type, size, created_at)
             VALUES('rollback-attachment', 'rollback-message', 'file.txt', 'text/plain', 4,
                    '2026-01-01')",
        )
        .execute(&pool)
        .await
        .expect("attachment");
        sqlx::query("INSERT INTO tags(id, name) VALUES('rollback-tag', 'Rollback')")
            .execute(&pool)
            .await
            .expect("tag");
        sqlx::query(
            "INSERT INTO thread_tags(thread_id, tag_id)
             VALUES('rollback-thread', 'rollback-tag')",
        )
        .execute(&pool)
        .await
        .expect("thread tag");
        sqlx::query(
            "INSERT INTO workflow_events
             (id, thread_id, from_state, to_state, reason, created_at)
             VALUES('rollback-event', 'rollback-thread', 'inbox', 'done', 'test', '2026-01-01')",
        )
        .execute(&pool)
        .await
        .expect("workflow event");
        sqlx::query(
            "INSERT INTO topics(id, name, created_at, updated_at)
             VALUES('rollback-topic', 'Rollback', '2026-01-01', '2026-01-01')",
        )
        .execute(&pool)
        .await
        .expect("topic");
        sqlx::query(
            "INSERT INTO topic_items(topic_id, item_type, item_id, added_at)
             VALUES('rollback-topic', 'thread', 'rollback-thread', '2026-01-01'),
                   ('rollback-topic', 'message', 'rollback-message', '2026-01-01')",
        )
        .execute(&pool)
        .await
        .expect("topic links");
        sqlx::query(
            "CREATE TRIGGER fail_trash_purge
             BEFORE DELETE ON threads
             WHEN old.id = 'rollback-thread'
             BEGIN SELECT RAISE(ABORT, 'purge failure'); END",
        )
        .execute(&pool)
        .await
        .expect("failure trigger");

        let error = ThreadRepository::new(&pool)
            .purge_trash_before("2026-08-01T00:00:00Z")
            .await
            .expect_err("purge should fail");
        assert!(matches!(error, ThreadError::Persistence(_)));
        let remaining: (i64, i64, i64, i64, i64, i64) = sqlx::query_as(
            "SELECT
                (SELECT COUNT(*) FROM threads WHERE id = 'rollback-thread'),
                (SELECT COUNT(*) FROM messages WHERE id = 'rollback-message'),
                (SELECT COUNT(*) FROM attachments WHERE id = 'rollback-attachment'),
                (SELECT COUNT(*) FROM thread_tags WHERE thread_id = 'rollback-thread'),
                (SELECT COUNT(*) FROM workflow_events WHERE id = 'rollback-event'),
                (SELECT COUNT(*) FROM topic_items WHERE item_id IN ('rollback-thread', 'rollback-message'))",
        )
        .fetch_one(&pool)
        .await
        .expect("rollback state");
        assert_eq!(remaining, (1, 1, 1, 1, 1, 2));
    }

    #[tokio::test]
    async fn repository_refresh_projection_removes_empty_thread_dependents_atomically() {
        let pool = migrated_pool().await;
        seed_thread(&pool).await;
        sqlx::query("INSERT INTO tags(id, name) VALUES('tag-1', 'Tag')")
            .execute(&pool)
            .await
            .expect("tag");
        sqlx::query("INSERT INTO thread_tags(thread_id, tag_id) VALUES('thread-1', 'tag-1')")
            .execute(&pool)
            .await
            .expect("thread tag");
        sqlx::query(
            "INSERT INTO workflow_events(id, thread_id, from_state, to_state, reason, created_at)
             VALUES('event-1', 'thread-1', 'inbox', 'done', 'test', '2026-01-01')",
        )
        .execute(&pool)
        .await
        .expect("workflow event");
        sqlx::query(
            "INSERT INTO topics(id, name, created_at, updated_at)
             VALUES('topic-1', 'Topic', '2026-01-01', '2026-01-01')",
        )
        .execute(&pool)
        .await
        .expect("topic");
        sqlx::query(
            "INSERT INTO topic_items(topic_id, item_type, item_id, added_at)
             VALUES('topic-1', 'thread', 'thread-1', '2026-01-01')",
        )
        .execute(&pool)
        .await
        .expect("topic link");

        ThreadRepository::new(&pool)
            .refresh_projection(&["thread-1".into()])
            .await
            .expect("empty cleanup");

        for table in ["threads", "thread_tags", "workflow_events", "topic_items"] {
            let count: i64 = sqlx::query_scalar(&format!(
                "SELECT COUNT(*) FROM {table} WHERE {} = 'thread-1'",
                if table == "threads" {
                    "id"
                } else if table == "thread_tags" || table == "workflow_events" {
                    "thread_id"
                } else {
                    "item_id"
                }
            ))
            .fetch_one(&pool)
            .await
            .expect("dependent count");
            assert_eq!(count, 0, "{table} still references deleted thread");
        }
    }

    #[tokio::test]
    async fn repository_merge_transfers_everything_and_preserves_canonical_state() {
        let pool = migrated_pool().await;
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .expect("foreign keys");
        seed_thread(&pool).await;
        insert_thread(&pool, "thread-2", "acc").await;
        sqlx::query(
            "UPDATE threads SET workflow_state = 'done', priority = 7, board_column = 'done'
             WHERE id = 'thread-2'",
        )
        .execute(&pool)
        .await
        .expect("canonical state");
        insert_message(
            &pool,
            "child-message",
            "thread-1",
            "inbox",
            Some("<child@example.com>"),
            "child-remote",
            "2026-08-02T00:00:00Z",
            false,
            false,
            false,
            false,
            "Child",
            "Child snippet",
        )
        .await;
        insert_message(
            &pool,
            "canonical-message",
            "thread-2",
            "inbox",
            Some("<canonical@example.com>"),
            "canonical-remote",
            "2026-08-03T00:00:00Z",
            true,
            false,
            false,
            false,
            "Canonical",
            "Canonical snippet",
        )
        .await;
        sqlx::query(
            "INSERT INTO tags(id, name) VALUES('tag-child', 'Child'), ('tag-shared', 'Shared')",
        )
        .execute(&pool)
        .await
        .expect("tags");
        sqlx::query(
            "INSERT INTO thread_tags(thread_id, tag_id)
             VALUES('thread-1', 'tag-child'), ('thread-1', 'tag-shared'),
                   ('thread-2', 'tag-shared')",
        )
        .execute(&pool)
        .await
        .expect("thread tags");
        sqlx::query(
            "INSERT INTO topics(id, name, created_at, updated_at)
             VALUES('topic-shared', 'Shared', '2026-01-01', '2026-01-01'),
                   ('topic-child', 'Child', '2026-01-01', '2026-01-01')",
        )
        .execute(&pool)
        .await
        .expect("topics");
        sqlx::query(
            "INSERT INTO topic_items(topic_id, item_type, item_id, added_at, added_by)
             VALUES('topic-shared', 'thread', 'thread-1', '2026-02-01', 'rule'),
                   ('topic-shared', 'thread', 'thread-2', '2026-01-01', 'user'),
                   ('topic-child', 'thread', 'thread-1', '2026-02-01', 'rule')",
        )
        .execute(&pool)
        .await
        .expect("topic links");
        sqlx::query(
            "INSERT INTO workflow_events(id, thread_id, from_state, to_state, reason, created_at)
             VALUES('event-child', 'thread-1', 'inbox', 'waiting', 'test', '2026-02-01')",
        )
        .execute(&pool)
        .await
        .expect("workflow event");
        sqlx::query(
            "INSERT INTO follow_up_reminders
             (id, account_id, thread_id, message_id, due_at, created_at, updated_at)
             VALUES('followup-child', 'acc', 'thread-1', 'child-message',
                    '2026-09-01', '2026-02-01', '2026-02-01')",
        )
        .execute(&pool)
        .await
        .expect("follow-up");

        let moved = ThreadRepository::new(&pool)
            .merge_into("thread-1", "thread-2")
            .await
            .expect("merge");
        assert_eq!(moved, 1);

        let source_exists: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM threads WHERE id = 'thread-1'")
                .fetch_one(&pool)
                .await
                .expect("source thread");
        assert_eq!(source_exists, 0);
        let message_threads: Vec<String> =
            sqlx::query_scalar("SELECT thread_id FROM messages ORDER BY id")
                .fetch_all(&pool)
                .await
                .expect("message threads");
        assert_eq!(message_threads, vec!["thread-2", "thread-2"]);

        let canonical: (String, i64, String, i64) = sqlx::query_as(
            "SELECT workflow_state, priority, board_column, message_count
             FROM threads WHERE id = 'thread-2'",
        )
        .fetch_one(&pool)
        .await
        .expect("canonical thread");
        assert_eq!(canonical, ("done".into(), 7, "done".into(), 2));

        let tag_ids: Vec<String> = sqlx::query_scalar(
            "SELECT tag_id FROM thread_tags WHERE thread_id = 'thread-2' ORDER BY tag_id",
        )
        .fetch_all(&pool)
        .await
        .expect("merged tags");
        assert_eq!(tag_ids, vec!["tag-child", "tag-shared"]);
        let topics: Vec<(String, String, String)> = sqlx::query_as(
            "SELECT topic_id, added_at, added_by FROM topic_items
             WHERE item_type = 'thread' AND item_id = 'thread-2' ORDER BY topic_id",
        )
        .fetch_all(&pool)
        .await
        .expect("merged topics");
        assert_eq!(
            topics,
            vec![
                ("topic-child".into(), "2026-02-01".into(), "rule".into()),
                ("topic-shared".into(), "2026-01-01".into(), "user".into()),
            ]
        );
        let dependents: (String, String) = sqlx::query_as(
            "SELECT
                (SELECT thread_id FROM workflow_events WHERE id = 'event-child'),
                (SELECT thread_id FROM follow_up_reminders WHERE id = 'followup-child')",
        )
        .fetch_one(&pool)
        .await
        .expect("merged dependents");
        assert_eq!(dependents, ("thread-2".into(), "thread-2".into()));
    }

    #[tokio::test]
    async fn repository_merge_defers_for_active_workflow_moves_including_failed() {
        let pool = migrated_pool().await;
        seed_thread(&pool).await;
        insert_thread(&pool, "thread-2", "acc").await;
        insert_message(
            &pool,
            "child-message",
            "thread-1",
            "inbox",
            Some("<child@example.com>"),
            "child-remote",
            "2026-08-02T00:00:00Z",
            false,
            false,
            false,
            false,
            "Child",
            "Child snippet",
        )
        .await;

        insert_thread(&pool, "malformed-child", "acc").await;
        insert_thread(&pool, "malformed-target", "acc").await;
        insert_message(
            &pool,
            "malformed-child-message",
            "malformed-child",
            "inbox",
            Some("<malformed@example.com>"),
            "malformed-remote",
            "2026-08-02T00:00:00Z",
            false,
            false,
            false,
            false,
            "Malformed",
            "Malformed snippet",
        )
        .await;
        sqlx::query(
            "INSERT INTO sync_queue
             (id, account_id, action_type, payload, status, created_at, updated_at)
             VALUES('unrelated', 'acc', 'remote_mutation', '{\"kind\":\"other\"}',
                    'pending', '2026-01-01', '2026-01-01'),
                   ('malformed', 'acc', 'remote_mutation', '{not-json',
                    'pending', '2026-01-01', '2026-01-01')",
        )
        .execute(&pool)
        .await
        .expect("non-blocking queue rows");
        assert_eq!(
            ThreadRepository::new(&pool)
                .merge_into("malformed-child", "malformed-target")
                .await
                .expect("unrelated payload must not block"),
            1
        );
        sqlx::query("DELETE FROM sync_queue")
            .execute(&pool)
            .await
            .expect("queue cleanup");

        for (index, status) in ["pending", "sending", "scheduled", "failed"]
            .into_iter()
            .enumerate()
        {
            let payload =
                "{\"kind\":\"workflow_move\",\"workflow_thread_id\":\"thread-1\"}".to_string();
            sqlx::query(
                "INSERT INTO sync_queue
                 (id, account_id, action_type, payload, status, created_at, updated_at)
                 VALUES(?, 'acc', 'remote_mutation', ?, ?, '2026-01-01', '2026-01-01')",
            )
            .bind(format!("move-{index}"))
            .bind(payload)
            .bind(status)
            .execute(&pool)
            .await
            .expect("queue row");

            let moved = ThreadRepository::new(&pool)
                .merge_into("thread-1", "thread-2")
                .await
                .expect("deferred merge");
            assert_eq!(moved, 0, "{status} workflow move must defer merge");
            let still_child: String =
                sqlx::query_scalar("SELECT thread_id FROM messages WHERE id = 'child-message'")
                    .fetch_one(&pool)
                    .await
                    .expect("child message");
            assert_eq!(still_child, "thread-1");
            sqlx::query("DELETE FROM sync_queue")
                .execute(&pool)
                .await
                .expect("queue cleanup");
        }

        for (index, status) in ["pending", "sending", "failed"].into_iter().enumerate() {
            let payload = serde_json::to_string(&RemoteMutationPayload::BlockedMove {
                blocked_thread_id: "thread-1".into(),
                refs: Vec::new(),
                target_remote_folder_id: "Spam".into(),
                provider_move_started: true,
                provider_move_completed: false,
                source_uidvalidity_before: None,
            })
            .expect("blocked move payload");
            sqlx::query(
                "INSERT INTO sync_queue
                 (id, account_id, action_type, payload, status, created_at, updated_at)
                 VALUES(?, 'acc', 'remote_mutation', ?, ?, '2026-01-01', '2026-01-01')",
            )
            .bind(format!("blocked-move-{index}"))
            .bind(payload)
            .bind(status)
            .execute(&pool)
            .await
            .expect("blocked queue row");

            let moved = ThreadRepository::new(&pool)
                .merge_into("thread-1", "thread-2")
                .await
                .expect("deferred blocked merge");
            assert_eq!(moved, 0, "{status} blocked move must defer merge");
            let still_child: String =
                sqlx::query_scalar("SELECT thread_id FROM messages WHERE id = 'child-message'")
                    .fetch_one(&pool)
                    .await
                    .expect("child message");
            assert_eq!(still_child, "thread-1");
            sqlx::query("DELETE FROM sync_queue")
                .execute(&pool)
                .await
                .expect("queue cleanup");
        }

        let moved = ThreadRepository::new(&pool)
            .merge_into("thread-1", "thread-2")
            .await
            .expect("merge after queue settles");
        assert_eq!(moved, 1);
    }

    #[tokio::test]
    async fn repository_merge_rejects_missing_and_cross_account_data() {
        let pool = migrated_pool().await;
        seed_thread(&pool).await;
        insert_thread(&pool, "thread-2", "acc").await;
        insert_message(
            &pool,
            "child-message",
            "thread-1",
            "inbox",
            Some("<child@example.com>"),
            "child-remote",
            "2026-08-02T00:00:00Z",
            false,
            false,
            false,
            false,
            "Child",
            "Child snippet",
        )
        .await;

        assert_eq!(
            ThreadRepository::new(&pool)
                .merge_into("thread-1", "thread-1")
                .await
                .expect("same thread no-op"),
            0
        );
        assert_eq!(
            ThreadRepository::new(&pool)
                .merge_into("missing", "thread-2")
                .await
                .expect("missing source no-op"),
            0
        );
        assert_eq!(
            ThreadRepository::new(&pool)
                .merge_into("thread-1", "missing")
                .await
                .expect("missing target no-op"),
            0
        );

        sqlx::query(
            "INSERT INTO accounts(id, display_name, email, provider, config, credentials_ref,
                                  is_active, created_at, updated_at)
             VALUES('acc-2', 'B', 'b@x.test', 'imap', '{}', '', 1, '2026-01-01', '2026-01-01')",
        )
        .execute(&pool)
        .await
        .expect("second account");
        insert_thread(&pool, "thread-other-account", "acc-2").await;
        assert_eq!(
            ThreadRepository::new(&pool)
                .merge_into("thread-1", "thread-other-account")
                .await
                .expect("cross-account no-op"),
            0
        );
        let still_child: String =
            sqlx::query_scalar("SELECT thread_id FROM messages WHERE id = 'child-message'")
                .fetch_one(&pool)
                .await
                .expect("child message");
        assert_eq!(still_child, "thread-1");

        insert_message(
            &pool,
            "foreign-message",
            "thread-1",
            "inbox",
            Some("<foreign@example.com>"),
            "foreign-remote",
            "2026-08-03T00:00:00Z",
            true,
            false,
            false,
            false,
            "Foreign",
            "Foreign snippet",
        )
        .await;
        sqlx::query("UPDATE messages SET account_id = 'acc-2' WHERE id = 'foreign-message'")
            .execute(&pool)
            .await
            .expect("corrupt message account");
        assert_eq!(
            ThreadRepository::new(&pool)
                .merge_into("thread-1", "thread-2")
                .await
                .expect("cross-account message no-op"),
            0
        );
        let remaining: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE thread_id = 'thread-1'")
                .fetch_one(&pool)
                .await
                .expect("child messages");
        assert_eq!(remaining, 2);
    }

    #[tokio::test]
    async fn repository_merge_rolls_back_all_transfers_when_projection_fails() {
        let pool = migrated_pool().await;
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .expect("foreign keys");
        seed_thread(&pool).await;
        insert_thread(&pool, "thread-2", "acc").await;
        insert_message(
            &pool,
            "child-message",
            "thread-1",
            "inbox",
            Some("<child@example.com>"),
            "child-remote",
            "2026-08-02T00:00:00Z",
            false,
            false,
            false,
            false,
            "Child",
            "Child snippet",
        )
        .await;
        sqlx::query("INSERT INTO tags(id, name) VALUES('tag-child', 'Child')")
            .execute(&pool)
            .await
            .expect("tag");
        sqlx::query("INSERT INTO thread_tags(thread_id, tag_id) VALUES('thread-1', 'tag-child')")
            .execute(&pool)
            .await
            .expect("thread tag");
        sqlx::query(
            "INSERT INTO topics(id, name, created_at, updated_at)
             VALUES('topic-child', 'Child', '2026-01-01', '2026-01-01')",
        )
        .execute(&pool)
        .await
        .expect("topic");
        sqlx::query(
            "INSERT INTO topic_items(topic_id, item_type, item_id, added_at)
             VALUES('topic-child', 'thread', 'thread-1', '2026-01-01')",
        )
        .execute(&pool)
        .await
        .expect("topic link");
        sqlx::query(
            "INSERT INTO workflow_events(id, thread_id, from_state, to_state, reason, created_at)
             VALUES('event-child', 'thread-1', 'inbox', 'done', 'test', '2026-01-01')",
        )
        .execute(&pool)
        .await
        .expect("workflow event");
        sqlx::query(
            "INSERT INTO follow_up_reminders
             (id, account_id, thread_id, message_id, due_at, created_at, updated_at)
             VALUES('followup-child', 'acc', 'thread-1', 'child-message',
                    '2026-09-01', '2026-01-01', '2026-01-01')",
        )
        .execute(&pool)
        .await
        .expect("follow-up");
        sqlx::query(
            "CREATE TRIGGER fail_merge_projection
             BEFORE UPDATE OF message_count ON threads
             WHEN NEW.id = 'thread-2'
             BEGIN
               SELECT RAISE(ABORT, 'merge projection failed');
             END",
        )
        .execute(&pool)
        .await
        .expect("failure trigger");

        let error = ThreadRepository::new(&pool)
            .merge_into("thread-1", "thread-2")
            .await
            .expect_err("projection must fail");
        assert!(matches!(error, ThreadError::Persistence(_)));

        let source_state: (i64, i64, i64, i64, i64, i64, i64, i64, i64, i64) = sqlx::query_as(
            "SELECT
                (SELECT COUNT(*) FROM threads WHERE id = 'thread-1'),
                (SELECT COUNT(*) FROM messages WHERE thread_id = 'thread-1'),
                (SELECT COUNT(*) FROM thread_tags WHERE thread_id = 'thread-1'),
                (SELECT COUNT(*) FROM topic_items WHERE item_id = 'thread-1'),
                (SELECT COUNT(*) FROM follow_up_reminders WHERE thread_id = 'thread-1'),
                (SELECT COUNT(*) FROM messages WHERE thread_id = 'thread-2'),
                (SELECT COUNT(*) FROM thread_tags WHERE thread_id = 'thread-2'),
                (SELECT COUNT(*) FROM topic_items WHERE item_id = 'thread-2'),
                (SELECT COUNT(*) FROM workflow_events WHERE thread_id = 'thread-2'),
                (SELECT COUNT(*) FROM follow_up_reminders WHERE thread_id = 'thread-2')",
        )
        .fetch_one(&pool)
        .await
        .expect("rollback state");
        assert_eq!(source_state, (1, 1, 1, 1, 1, 0, 0, 0, 0, 0));
        let workflow_thread: String =
            sqlx::query_scalar("SELECT thread_id FROM workflow_events WHERE id = 'event-child'")
                .fetch_one(&pool)
                .await
                .expect("workflow rollback");
        assert_eq!(workflow_thread, "thread-1");
    }

    #[tokio::test]
    async fn repository_rejects_unknown_smart_inbox_bucket() {
        let pool = migrated_pool().await;
        let error = ThreadRepository::new(&pool)
            .list(ThreadListRequest {
                smart_bucket: Some("unknown".into()),
                ..Default::default()
            })
            .await
            .expect_err("unknown bucket should fail");

        assert!(matches!(
            error,
            ThreadError::InvalidSmartInboxBucket { value } if value == "unknown"
        ));
    }

    #[tokio::test]
    async fn repository_snapshot_round_trip_uses_exact_key_and_skips_cursors() {
        let pool = migrated_pool().await;
        let repository = ThreadRepository::new(&pool);
        let request = ThreadListRequest {
            mailbox_kind: Some("section".into()),
            mailbox_value: Some("inbox".into()),
            limit: Some(40),
            ..Default::default()
        };

        let page = repository
            .list(request.clone())
            .await
            .expect("list and store snapshot");
        assert!(page.threads.is_empty());

        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
        while repository
            .list_cached(request.clone())
            .await
            .expect("cache lookup")
            .is_none()
        {
            assert!(
                tokio::time::Instant::now() < deadline,
                "snapshot task completed"
            );
            tokio::task::yield_now().await;
        }
        let cached = repository
            .list_cached(request.clone())
            .await
            .expect("load snapshot")
            .expect("cached page");
        assert_eq!(cached, page);
        assert!(repository
            .list_cached(ThreadListRequest {
                limit: Some(20),
                ..request.clone()
            })
            .await
            .expect("different exact-key lookup")
            .is_none());

        let cursor_request = ThreadListRequest {
            cursor: Some(ThreadCursor {
                last_message_date: "2026-08-10T00:00:00Z".into(),
                id: "thread-40".into(),
            }),
            ..request
        };
        repository
            .list(cursor_request.clone())
            .await
            .expect("cursor page");
        assert!(repository
            .list_cached(cursor_request)
            .await
            .expect("cursor cache lookup")
            .is_none());
    }

    #[tokio::test]
    async fn repository_discards_invalid_snapshot_json() {
        let pool = migrated_pool().await;
        let repository = ThreadRepository::new(&pool);
        let request = ThreadListRequest {
            account_id: Some("acc".into()),
            limit: Some(12),
            ..Default::default()
        };
        let key = thread_list_snapshot_key(&request).expect("snapshot key");
        // Fresh, so the age check cannot be what rejects it.
        sqlx::query(
            "INSERT INTO thread_list_snapshots(cache_key, page_json, updated_at)
             VALUES(?, 'not json', ?)",
        )
        .bind(&key)
        .bind(chrono::Utc::now().to_rfc3339())
        .execute(&pool)
        .await
        .expect("invalid snapshot");

        assert!(repository
            .list_cached(request)
            .await
            .expect("invalid snapshot is a cache miss")
            .is_none());
        let remaining = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM thread_list_snapshots WHERE cache_key = ?",
        )
        .bind(key)
        .fetch_one(&pool)
        .await
        .expect("snapshot cleanup");
        assert_eq!(remaining, 0);
    }

    #[tokio::test]
    async fn snapshot_older_than_the_ttl_is_discarded() {
        let pool = migrated_pool().await;
        let repository = ThreadRepository::new(&pool);
        let request = ThreadListRequest {
            account_id: Some("acc".into()),
            limit: Some(12),
            ..Default::default()
        };
        let key = thread_list_snapshot_key(&request).expect("snapshot key");
        let aged = (chrono::Utc::now()
            - chrono::Duration::hours(THREAD_LIST_SNAPSHOT_MAX_AGE_HOURS + 1))
        .to_rfc3339();
        sqlx::query(
            "INSERT INTO thread_list_snapshots(cache_key, page_json, updated_at)
             VALUES(?, '{\"threads\":[],\"next_cursor\":null}', ?)",
        )
        .bind(&key)
        .bind(&aged)
        .execute(&pool)
        .await
        .expect("aged snapshot");

        assert!(repository
            .list_cached(request)
            .await
            .expect("aged snapshot is a cache miss")
            .is_none());
        let remaining = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM thread_list_snapshots WHERE cache_key = ?",
        )
        .bind(key)
        .fetch_one(&pool)
        .await
        .expect("snapshot cleanup");
        assert_eq!(remaining, 0);
    }

    #[tokio::test]
    async fn purging_trash_clears_the_snapshot_cache() {
        let pool = migrated_pool().await;
        seed_thread(&pool).await;
        insert_trash_folder(&pool, "trash", "trash").await;
        insert_thread(&pool, "purge-thread", "acc").await;
        mark_old_done(&pool, "purge-thread").await;
        insert_message(
            &pool,
            "purge-message",
            "purge-thread",
            "trash",
            Some("<purge@example.com>"),
            "purge-remote",
            "2026-01-01T00:00:00Z",
            false,
            false,
            false,
            true,
            "Purgeable message",
            "purgeable snippet",
        )
        .await;

        let repository = ThreadRepository::new(&pool);
        let request = ThreadListRequest {
            account_id: Some("acc".into()),
            limit: Some(40),
            ..Default::default()
        };
        let listed = repository.list(request.clone()).await.expect("list");
        assert!(listed
            .threads
            .iter()
            .any(|thread| thread.id == "purge-thread"));
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
        while repository
            .list_cached(request.clone())
            .await
            .unwrap()
            .is_none()
        {
            assert!(tokio::time::Instant::now() < deadline);
            tokio::task::yield_now().await;
        }
        assert!(repository
            .list_cached(request.clone())
            .await
            .expect("snapshot lookup")
            .is_some());

        let removed = repository
            .purge_trash_before("2027-01-01T00:00:00Z")
            .await
            .expect("purge");
        assert_eq!(removed, 1);

        assert!(
            repository
                .list_cached(request)
                .await
                .expect("snapshot lookup after purge")
                .is_none(),
            "a purged thread was still reachable through the first-paint cache"
        );
    }

    #[test]
    fn normalize_participants_merges_message_addresses() {
        let participants = normalize_participants_json(
            Some(r#"["sample.user@example.com"]"#.to_string()),
            Some(
                r#"[
                  {"name":"sample.user@example.com","email":"sample.user@example.com"},
                  {"name":"Mara Vogel | Beispielfirma","email":"mara@example.com"}
                ]"#
                .to_string(),
            ),
            Some("sample.user@example.com".to_string()),
            Some("sample.user@example.com".to_string()),
        );

        assert!(participants.contains("sample.user@example.com"));
        assert!(participants.contains("mara@example.com"));
        assert!(participants.contains("Mara Vogel | Beispielfirma"));
    }

    #[test]
    fn normalize_participants_puts_latest_sender_first() {
        let participants = normalize_participants_json(
            Some(r#"["sample.user@example.com"]"#.to_string()),
            Some(
                r#"[
                  {"name":"Sample User","email":"sample.user@example.com"},
                  {"name":"SHOP APOTHEKE","email":"info@shop-apotheke.com"}
                ]"#
                .to_string(),
            ),
            Some("SHOP APOTHEKE".to_string()),
            Some("info@shop-apotheke.com".to_string()),
        );

        let participants = serde_json::from_str::<Vec<Value>>(&participants)
            .expect("participants should be valid JSON");

        assert_eq!(participants[0]["name"], "SHOP APOTHEKE");
        assert_eq!(participants[0]["email"], "info@shop-apotheke.com");
    }

    #[test]
    fn normalize_participants_prefers_latest_sender_name_over_longer_history() {
        let participants = normalize_participants_json(
            Some(
                r#"[{"name":"Training Lehmann","email":"training-lehmann@example.org"}]"#
                    .to_string(),
            ),
            Some(
                r#"[{"name":"Training Lehmann","email":"training-lehmann@example.org"}]"#
                    .to_string(),
            ),
            Some("Lena Lehmann".to_string()),
            Some("training-lehmann@example.org".to_string()),
        );

        let participants = serde_json::from_str::<Vec<Value>>(&participants)
            .expect("participants should be valid JSON");

        assert_eq!(participants[0]["name"], "Lena Lehmann");
        assert_eq!(participants[0]["email"], "training-lehmann@example.org");
    }
    #[tokio::test]
    async fn paginates_without_gaps_or_duplicates() {
        use sqlx::sqlite::SqlitePoolOptions;

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory pool");
        crate::db::init(&pool).await.expect("migrations");

        sqlx::query(
            "INSERT INTO accounts(id, display_name, email, provider, config, credentials_ref, is_active, created_at, updated_at)
             VALUES('acc', 'A', 'a@x.test', 'imap', '{}', '', 1, '2026-01-01', '2026-01-01')",
        )
        .execute(&pool)
        .await
        .expect("account");

        sqlx::query(
            "INSERT INTO folders(id, account_id, name, full_path, folder_type, created_at, updated_at)
             VALUES('f', 'acc', 'INBOX', 'INBOX', 'inbox', '2026-01-01', '2026-01-01')",
        )
        .execute(&pool)
        .await
        .expect("folder");

        // Deliberately give two threads the same timestamp: the tie is exactly
        // where an ORDER BY on date alone loses or repeats rows.
        for (id, date) in [
            ("t1", "2026-07-01T10:00:00Z"),
            ("t2", "2026-07-01T09:00:00Z"),
            ("t3", "2026-07-01T09:00:00Z"),
            ("t4", "2026-07-01T08:00:00Z"),
            ("t5", "2026-07-01T07:00:00Z"),
        ] {
            sqlx::query(
                "INSERT INTO threads(id, account_id, subject, last_message_date, message_count, unread_count, created_at, updated_at)
                 VALUES(?, 'acc', 'S', ?, 1, 0, '2026-01-01', '2026-01-01')",
            )
            .bind(id)
            .bind(date)
            .execute(&pool)
            .await
            .expect("thread");

            // The list query only returns threads that actually have messages.
            sqlx::query(
                "INSERT INTO messages(id, account_id, folder_id, thread_id, from_address, to_addresses, date, created_at, updated_at)
                 VALUES(?, 'acc', 'f', ?, 'a@x.test', '[]', ?, '2026-01-01', '2026-01-01')",
            )
            .bind(format!("m-{id}"))
            .bind(id)
            .bind(date)
            .execute(&pool)
            .await
            .expect("message");
        }

        // Walk the whole list two rows at a time.
        let mut seen: Vec<String> = Vec::new();
        let mut cursor: Option<ThreadCursor> = None;
        for _ in 0..10 {
            let page = ThreadRepository::new(&pool)
                .list(ThreadListRequest {
                    limit: Some(2),
                    cursor: cursor.clone(),
                    ..Default::default()
                })
                .await
                .expect("page");

            seen.extend(page.threads.iter().map(|thread| thread.id.clone()));
            cursor = page.next_cursor.clone();
            if !page.has_more {
                break;
            }
        }

        // Order is (last_message_date DESC, id DESC): t2 and t3 share a
        // timestamp, so the id tiebreak puts t3 first. The point is that the
        // order is total and stable — without the tiebreak, paging across the
        // tie would drop or repeat one of them.
        assert_eq!(
            seen,
            vec!["t1", "t3", "t2", "t4", "t5"],
            "pagination must return every thread exactly once, newest first"
        );

        // A displayed message timestamp can differ from the stored thread sort date.
        sqlx::query("UPDATE messages SET date = '2026-07-02T08:00:00Z' WHERE thread_id = 't4'")
            .execute(&pool)
            .await
            .expect("display date");
        // Reverse paging picks the nearest newer rows, not the mailbox head.
        let newer = ThreadRepository::new(&pool)
            .list(ThreadListRequest {
                limit: Some(2),
                before_cursor: Some(ThreadCursor {
                    id: "t4".into(),
                    last_message_date: "2026-07-01T08:00:00Z".into(),
                }),
                ..Default::default()
            })
            .await
            .expect("previous page");
        assert_eq!(
            newer
                .threads
                .iter()
                .map(|row| row.id.as_str())
                .collect::<Vec<_>>(),
            vec!["t3", "t2"]
        );
        assert!(newer.has_more);
        let newest = ThreadRepository::new(&pool)
            .list(ThreadListRequest {
                limit: Some(2),
                before_cursor: newer.next_cursor,
                ..Default::default()
            })
            .await
            .expect("first page");
        assert_eq!(newest.threads[0].id, "t1");
        assert!(!newest.has_more);

        // New mail may move the cursor thread after a page was received.
        // The next page must still use the captured position.
        sqlx::query(
            "UPDATE threads SET last_message_date = '2026-07-02T12:00:00Z' WHERE id = 't2'",
        )
        .execute(&pool)
        .await
        .expect("new mail");
        let older = ThreadRepository::new(&pool)
            .list(ThreadListRequest {
                limit: Some(2),
                cursor: Some(ThreadCursor {
                    id: "t2".into(),
                    last_message_date: "2026-07-01T09:00:00Z".into(),
                }),
                ..Default::default()
            })
            .await
            .expect("immutable cursor page");
        assert_eq!(
            older
                .threads
                .iter()
                .map(|row| row.id.as_str())
                .collect::<Vec<_>>(),
            vec!["t4", "t5"]
        );
        assert_eq!(
            older.threads[0].sort_date.as_deref(),
            Some("2026-07-01T08:00:00Z")
        );
        assert_eq!(older.threads[0].last_message_date, "2026-07-02T08:00:00Z");

        let mut unique = seen.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), seen.len(), "no duplicates across pages");
    }

    #[tokio::test]
    async fn reports_no_further_page_when_the_list_fits() {
        use sqlx::sqlite::SqlitePoolOptions;

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory pool");
        crate::db::init(&pool).await.expect("migrations");

        let page = ThreadRepository::new(&pool)
            .list(ThreadListRequest::default())
            .await
            .expect("page");

        assert!(page.threads.is_empty());
        assert!(!page.has_more);
        assert!(page.next_cursor.is_none());
    }

    #[tokio::test]
    async fn a_draft_reply_does_not_take_over_the_thread_identity() {
        use sqlx::sqlite::SqlitePoolOptions;

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory pool");
        crate::db::init(&pool).await.expect("migrations");

        sqlx::query(
            "INSERT INTO accounts(id, display_name, email, provider, config, credentials_ref, is_active, created_at, updated_at)
             VALUES('acc', 'A', 'me@x.test', 'imap', '{}', '', 1, '2026-01-01', '2026-01-01')",
        )
        .execute(&pool)
        .await
        .expect("account");

        for (id, name, folder_type) in [("inbox", "INBOX", "inbox"), ("drafts", "Drafts", "drafts")]
        {
            sqlx::query(
                "INSERT INTO folders(id, account_id, name, full_path, folder_type, created_at, updated_at)
                 VALUES(?, 'acc', ?, ?, ?, '2026-01-01', '2026-01-01')",
            )
            .bind(id)
            .bind(name)
            .bind(name)
            .bind(folder_type)
            .execute(&pool)
            .await
            .expect("folder");
        }

        sqlx::query(
            "INSERT INTO threads(id, account_id, subject, last_message_date, message_count, unread_count, created_at, updated_at)
             VALUES('t', 'acc', 'Dein Steuerbescheid ist da!', '2026-07-21T03:54:42Z', 2, 1, '2026-01-01', '2026-01-01')",
        )
        .execute(&pool)
        .await
        .expect("thread");

        sqlx::query(
            "INSERT INTO messages(id, account_id, folder_id, thread_id, from_address, from_name, to_addresses, subject, snippet, date, is_draft, created_at, updated_at)
             VALUES('m', 'acc', 'inbox', 't', 'steuer@buhl.test', 'steuerWelt', '[\"me@x.test\"]', 'Dein Steuerbescheid ist da!', 'Dein Bescheid liegt bereit', '2026-07-21T03:54:42Z', 0, '2026-01-01', '2026-01-01')",
        )
        .execute(&pool)
        .await
        .expect("inbound message");

        // The draft is newer than the message it answers — that is the normal
        // case, not an edge case.
        sqlx::query(
            "INSERT INTO messages(id, account_id, folder_id, thread_id, from_address, from_name, to_addresses, subject, snippet, date, is_draft, created_at, updated_at)
             VALUES('d', 'acc', 'drafts', 't', 'me@x.test', 'Me', '[\"steuer@buhl.test\"]', 'Re: Dein Steuerbescheid ist da!', 'das ist ein test', '2026-07-28T16:41:18Z', 1, '2026-01-01', '2026-01-01')",
        )
        .execute(&pool)
        .await
        .expect("draft");

        let page = ThreadRepository::new(&pool)
            .list(ThreadListRequest::default())
            .await
            .expect("page");
        let thread = page.threads.first().expect("thread row");

        assert_eq!(thread.subject, "Dein Steuerbescheid ist da!");
        assert_eq!(thread.snippet, "Dein Bescheid liegt bereit");
        assert_eq!(thread.last_message_date, "2026-07-21T03:54:42Z");
        assert!(thread.has_draft);
        assert_eq!(thread.draft_snippet.as_deref(), Some("das ist ein test"));

        let participants =
            serde_json::from_str::<Vec<Value>>(&thread.participants).expect("participants JSON");
        assert_eq!(participants[0]["email"], "steuer@buhl.test");
    }

    #[test]
    fn the_page_size_override_is_parsed_and_clamped() {
        assert_eq!(thread_page_size_from(None), 200);
        assert_eq!(thread_page_size_from(Some("50")), 50);
        assert_eq!(thread_page_size_from(Some("5000")), 1_000);
        assert_eq!(thread_page_size_from(Some("0")), 200);
        assert_eq!(thread_page_size_from(Some("many")), 200);
    }

    /*
     * PERF-15. Subject, snippet, date, sender address and sender name each ran
     * their own correlated lookup of the latest message, with the same order.
     * One seek now feeds all five.
     */
    #[tokio::test]
    async fn the_thread_projection_looks_up_the_latest_message_once() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory pool");
        crate::db::init(&pool).await.expect("migrations");

        let record_sql = thread_record_sql();
        let plan: Vec<String> = sqlx::query_scalar(&format!(
            "SELECT detail FROM (EXPLAIN QUERY PLAN {record_sql})"
        ))
        .bind("t")
        .fetch_all(&pool)
        .await
        .unwrap_or_default();
        let plan = if plan.is_empty() {
            // Older SQLite builds do not allow EXPLAIN inside a subquery.
            sqlx::query(&format!("EXPLAIN QUERY PLAN {record_sql}"))
                .bind("t")
                .fetch_all(&pool)
                .await
                .expect("plan")
                .iter()
                .map(|row| sqlx::Row::get::<String, _>(row, "detail"))
                .collect()
        } else {
            plan
        };

        // The draft lookups use the same index with `is_draft=?`; only the
        // unfiltered latest-message seeks count here.
        let latest_seeks = plan
            .iter()
            .filter(|step| {
                step.starts_with("SEARCH messages ")
                    && step.contains("idx_messages_thread_latest (thread_id=?)")
            })
            .count();
        assert_eq!(latest_seeks, 1, "plan: {plan:#?}");
    }

    /// Same picks as the five separate lookups: drafts last, the id breaks a
    /// date tie, and a missing subject still falls back to the thread's own.
    #[tokio::test]
    async fn the_joined_latest_message_picks_what_each_lookup_picked() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory pool");
        crate::db::init(&pool).await.expect("migrations");
        for statement in [
            "INSERT INTO accounts(id, display_name, email, provider, config, credentials_ref, is_active, created_at, updated_at)
             VALUES('acc', 'A', 'me@x.test', 'imap', '{}', '', 1, '2026-01-01', '2026-01-01')",
            "INSERT INTO folders(id, account_id, name, full_path, folder_type, created_at, updated_at)
             VALUES('f', 'acc', 'INBOX', 'INBOX', 'inbox', '2026-01-01', '2026-01-01')",
            "INSERT INTO threads(id, account_id, subject, snippet, last_message_date, message_count, unread_count, created_at, updated_at)
             VALUES('t', 'acc', 'Thread subject', 'Thread snippet', '2026-07-02T10:00:00Z', 3, 0, '2026-01-01', '2026-01-01')",
            // Two messages share the newest date; `m-b` wins on id. Its subject
            // is NULL, so the thread's own subject shows.
            "INSERT INTO messages(id, account_id, folder_id, thread_id, from_address, from_name, to_addresses, subject, snippet, date, is_draft, created_at, updated_at)
             VALUES('m-a', 'acc', 'f', 't', 'a@x.test', 'A', '[]', 'From A', 'Snippet A', '2026-07-02T10:00:00Z', 0, '2026-01-01', '2026-01-01')",
            "INSERT INTO messages(id, account_id, folder_id, thread_id, from_address, from_name, to_addresses, subject, snippet, date, is_draft, created_at, updated_at)
             VALUES('m-b', 'acc', 'f', 't', 'b@x.test', 'B', '[]', NULL, 'Snippet B', '2026-07-02T10:00:00Z', 0, '2026-01-01', '2026-01-01')",
            "INSERT INTO messages(id, account_id, folder_id, thread_id, from_address, from_name, to_addresses, subject, snippet, date, is_draft, created_at, updated_at)
             VALUES('m-draft', 'acc', 'f', 't', 'me@x.test', 'Me', '[]', 'Draft', 'Draft snippet', '2026-07-09T10:00:00Z', 1, '2026-01-01', '2026-01-01')",
        ] {
            sqlx::query(statement).execute(&pool).await.expect("seed");
        }

        let repository = ThreadRepository::new(&pool);
        let listed = repository
            .list(ThreadListRequest::default())
            .await
            .expect("page")
            .threads
            .remove(0);
        let fetched = repository.get("t").await.expect("thread");

        for thread in [&listed, &fetched] {
            assert_eq!(thread.subject, "Thread subject");
            assert_eq!(thread.snippet, "Snippet B");
            assert_eq!(thread.last_message_date, "2026-07-02T10:00:00Z");
        }
        let participants = serde_json::from_str::<Vec<Value>>(&listed.participants).expect("json");
        assert_eq!(participants[0]["email"], "b@x.test", "latest sender leads");
    }

    #[tokio::test]
    async fn smart_inbox_summary_and_filter_keep_priority_and_pinning_independent() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory pool");
        crate::db::init(&pool).await.expect("migrations");

        sqlx::query(
            "INSERT INTO accounts(id, display_name, email, provider, config, credentials_ref, is_active, created_at, updated_at)
             VALUES('acc', 'A', 'a@x.test', 'gmail', '{}', '', 1, '2026-01-01', '2026-01-01')",
        )
        .execute(&pool)
        .await
        .expect("account");
        sqlx::query(
            "INSERT INTO folders(id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at)
             VALUES('inbox', 'acc', 'Inbox', 'INBOX', 'inbox', 'INBOX', '2026-01-01', '2026-01-01')",
        )
        .execute(&pool)
        .await
        .expect("folder");

        for (index, id, category, priority, starred) in [
            (1, "priority", "promotions", 2, 1),
            (2, "pinned", "updates", 1, 1),
            (3, "invite", "promotions", 1, 0),
            (4, "social", "social", 1, 0),
            (5, "newsletter", "promotions", 1, 0),
            (6, "personal", "personal", 1, 0),
            (7, "machine", "personal", 1, 0),
        ] {
            let date = format!("2026-07-{index:02}T10:00:00Z");
            sqlx::query(
                "INSERT INTO threads
                 (id, account_id, subject, last_message_date, message_count, unread_count,
                  workflow_state, priority, category, is_starred, created_at, updated_at)
                 VALUES (?, 'acc', ?, ?, 1, ?, 'inbox', ?, ?, ?, '2026-01-01', '2026-01-01')",
            )
            .bind(id)
            .bind(id)
            .bind(&date)
            .bind(index)
            .bind(priority)
            .bind(category)
            .bind(starred)
            .execute(&pool)
            .await
            .expect("thread");
            sqlx::query(
                "INSERT INTO messages
                 (id, account_id, folder_id, thread_id, from_address, from_name, to_addresses,
                  subject, date, is_read, created_at, updated_at)
                 VALUES (?, 'acc', 'inbox', ?, ?, ?, '[]', ?, ?, 0, '2026-01-01', '2026-01-01')",
            )
            .bind(format!("message-{id}"))
            .bind(id)
            .bind(format!("{id}@example.com"))
            .bind(id)
            .bind(id)
            .bind(&date)
            .execute(&pool)
            .await
            .expect("message");
        }
        sqlx::query(
            "UPDATE messages SET from_address = 'googleplay-noreply@google.com'
             WHERE id = 'message-machine'",
        )
        .execute(&pool)
        .await
        .expect("machine sender");
        sqlx::query(
            "INSERT INTO calendar_invites
             (id, message_id, account_id, uid, raw_ics, created_at, updated_at)
             VALUES ('calendar', 'message-invite', 'acc', 'uid', 'BEGIN:VCALENDAR',
                     '2026-01-01', '2026-01-01')",
        )
        .execute(&pool)
        .await
        .expect("invite");

        let summary = ThreadRepository::new(&pool)
            .smart_inbox_summary(Some("acc"))
            .await
            .expect("summary");
        assert_eq!(summary.total_count, 7);
        assert_eq!(
            summary
                .buckets
                .iter()
                .map(|bucket| (bucket.key.as_str(), bucket.count))
                .collect::<Vec<_>>(),
            vec![
                ("priority", 1),
                ("pinned", 2),
                ("invitations", 1),
                ("notifications", 2),
                ("newsletter", 1),
            ]
        );

        let pinned = ThreadRepository::new(&pool)
            .list(ThreadListRequest {
                account_id: Some("acc".into()),
                mailbox_kind: Some("section".into()),
                mailbox_value: Some("inbox".into()),
                workflow_state: Some("inbox".into()),
                smart_bucket: Some("pinned".into()),
                ..Default::default()
            })
            .await
            .expect("pinned page");
        assert_eq!(
            pinned
                .threads
                .iter()
                .map(|thread| thread.id.as_str())
                .collect::<Vec<_>>(),
            vec!["pinned", "priority"]
        );

        let notifications = ThreadRepository::new(&pool)
            .list(ThreadListRequest {
                account_id: Some("acc".into()),
                mailbox_kind: Some("section".into()),
                mailbox_value: Some("inbox".into()),
                workflow_state: Some("inbox".into()),
                smart_bucket: Some("notifications".into()),
                ..Default::default()
            })
            .await
            .expect("notification page");
        assert_eq!(
            notifications
                .threads
                .iter()
                .map(|thread| thread.id.as_str())
                .collect::<Vec<_>>(),
            vec!["machine", "social"]
        );
    }

    #[tokio::test]
    async fn code_notifications_keep_the_same_bucket_in_summary_list_and_state_updates() {
        let pool = migrated_pool().await;
        seed_thread(&pool).await;
        for (id, category, source) in [
            ("pipeline", "personal", "heuristic"),
            ("merge-request", "promotions", "heuristic"),
            ("manual-personal", "personal", "manual_thread"),
            ("manual-newsletter", "promotions", "manual_sender"),
            ("provider-personal", "personal", "heuristic"),
        ] {
            insert_thread(&pool, id, "acc").await;
            insert_message(
                &pool,
                &format!("message-{id}"),
                id,
                "inbox",
                None,
                id,
                "2026-10-07T10:00:00Z",
                false,
                false,
                false,
                false,
                id,
                "",
            )
            .await;
            sqlx::query("UPDATE threads SET category = ?, category_source = ? WHERE id = ?")
                .bind(category)
                .bind(source)
                .bind(id)
                .execute(&pool)
                .await
                .unwrap();
            sqlx::query(
                "UPDATE messages SET from_address = 'gitlab@code.example' WHERE thread_id = ?",
            )
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
        }
        sqlx::query("UPDATE messages SET provider_labels = '[\"CATEGORY_PERSONAL\"]' WHERE thread_id = 'provider-personal'")
            .execute(&pool).await.unwrap();
        let repository = ThreadRepository::new(&pool);
        let summary = repository.smart_inbox_summary(Some("acc")).await.unwrap();
        assert_eq!(
            summary
                .buckets
                .iter()
                .find(|bucket| bucket.key == "notifications")
                .map(|bucket| bucket.count),
            Some(2)
        );
        let page = repository
            .list(ThreadListRequest {
                smart_bucket: Some("notifications".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(page.threads.len(), 2);
        assert!(page
            .threads
            .iter()
            .all(|thread| thread.smart_bucket == "notifications"));
        for id in ["pipeline", "merge-request"] {
            let updated = repository
                .update_state(ThreadStateUpdateRequest {
                    thread_id: id.into(),
                    next_state: WorkflowState::Done,
                    snooze_until: None,
                })
                .await
                .unwrap();
            assert_eq!(updated.smart_bucket, "notifications");
        }
        let personal = repository
            .list(ThreadListRequest {
                smart_bucket: Some("personal".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(personal
            .threads
            .iter()
            .any(|thread| thread.id == "manual-personal"));
        assert!(personal
            .threads
            .iter()
            .any(|thread| thread.id == "provider-personal"));
        assert!(!personal
            .threads
            .iter()
            .any(|thread| thread.id == "pipeline"));
        let newsletters = repository
            .list(ThreadListRequest {
                smart_bucket: Some("newsletter".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(
            newsletters
                .threads
                .iter()
                .map(|thread| thread.id.as_str())
                .collect::<Vec<_>>(),
            vec!["manual-newsletter"]
        );
    }

    /// Spellings the front-end heuristic reads as machine senders. Missing one
    /// here builds a bucket row out of local rows that the `smart_bucket` query
    /// then answers with an empty page.
    #[tokio::test]
    async fn smart_inbox_notifications_cover_separatorless_and_subdomain_senders() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory pool");
        crate::db::init(&pool).await.expect("migrations");

        sqlx::query(
            "INSERT INTO accounts(id, display_name, email, provider, config, credentials_ref, is_active, created_at, updated_at)
             VALUES('acc', 'A', 'a@x.test', 'gmail', '{}', '', 1, '2026-01-01', '2026-01-01')",
        )
        .execute(&pool)
        .await
        .expect("account");
        sqlx::query(
            "INSERT INTO folders(id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at)
             VALUES('inbox', 'acc', 'Inbox', 'INBOX', 'inbox', 'INBOX', '2026-01-01', '2026-01-01')",
        )
        .execute(&pool)
        .await
        .expect("folder");

        for (index, id, from_address) in [
            (1, "netcup", "donotreply@netcup.test"),
            (2, "polar", "whispernotes@notifications.polar.test"),
            (3, "human", "mike@agency.test"),
        ] {
            let date = format!("2026-08-{index:02}T10:00:00Z");
            sqlx::query(
                "INSERT INTO threads
                 (id, account_id, subject, last_message_date, message_count, unread_count,
                  workflow_state, priority, category, is_starred, created_at, updated_at)
                 VALUES (?, 'acc', ?, ?, 1, 1, 'inbox', 1, 'personal', 0, '2026-01-01', '2026-01-01')",
            )
            .bind(id)
            .bind(id)
            .bind(&date)
            .execute(&pool)
            .await
            .expect("thread");
            sqlx::query(
                "INSERT INTO messages
                 (id, account_id, folder_id, thread_id, from_address, from_name, to_addresses,
                  subject, date, is_read, created_at, updated_at)
                 VALUES (?, 'acc', 'inbox', ?, ?, ?, '[]', ?, ?, 0, '2026-01-01', '2026-01-01')",
            )
            .bind(format!("message-{id}"))
            .bind(id)
            .bind(from_address)
            .bind(id)
            .bind(id)
            .bind(&date)
            .execute(&pool)
            .await
            .expect("message");
        }

        let summary = ThreadRepository::new(&pool)
            .smart_inbox_summary(Some("acc"))
            .await
            .expect("summary");
        assert_eq!(
            summary
                .buckets
                .iter()
                .map(|bucket| (bucket.key.as_str(), bucket.count))
                .collect::<Vec<_>>(),
            vec![("notifications", 2)]
        );

        let notifications = ThreadRepository::new(&pool)
            .list(ThreadListRequest {
                account_id: Some("acc".into()),
                mailbox_kind: Some("section".into()),
                mailbox_value: Some("inbox".into()),
                workflow_state: Some("inbox".into()),
                smart_bucket: Some("notifications".into()),
                ..Default::default()
            })
            .await
            .expect("notification page");
        assert_eq!(
            notifications
                .threads
                .iter()
                .map(|thread| thread.id.as_str())
                .collect::<Vec<_>>(),
            vec!["polar", "netcup"]
        );
    }
}
#[derive(Debug, Serialize)]
struct ParticipantProjection {
    name: String,
    email: String,
}

fn normalize_participants_json(
    raw: Option<String>,
    message_participants: Option<String>,
    latest_from_name: Option<String>,
    latest_from_address: Option<String>,
) -> String {
    let mut normalized = Vec::<ParticipantProjection>::new();

    for raw_json in [
        message_participants.unwrap_or_else(|| "[]".to_string()),
        raw.unwrap_or_else(|| "[]".to_string()),
    ] {
        let parsed = serde_json::from_str::<Vec<Value>>(&raw_json).unwrap_or_default();
        for participant in parsed.into_iter().filter_map(participant_from_json) {
            upsert_participant(&mut normalized, participant);
        }
    }

    if let Some(latest_email) = normalize_participant_email(latest_from_address.as_deref()) {
        let latest_name = resolve_sender_display_name(latest_from_name.as_deref(), &latest_email);
        if let Some(index) = normalized
            .iter()
            .position(|participant| participant.email.eq_ignore_ascii_case(&latest_email))
        {
            normalized.remove(index);
        }

        normalized.insert(
            0,
            ParticipantProjection {
                name: latest_name,
                email: latest_email,
            },
        );
    }

    serde_json::to_string(&normalized).unwrap_or_else(|_| "[]".to_string())
}

fn upsert_participant(participants: &mut Vec<ParticipantProjection>, next: ParticipantProjection) {
    if let Some(existing) = participants
        .iter_mut()
        .find(|participant| participant.email.eq_ignore_ascii_case(&next.email))
    {
        if existing.name.eq_ignore_ascii_case(&existing.email)
            || (!next.name.eq_ignore_ascii_case(&next.email)
                && next.name.len() > existing.name.len())
        {
            existing.name = next.name;
        }
        return;
    }

    participants.push(next);
}

fn normalize_mailbox_names_json(raw: Option<String>) -> String {
    let raw = raw.unwrap_or_else(|| "[]".to_string());
    let parsed = serde_json::from_str::<Vec<String>>(&raw);
    let Ok(values) = parsed else {
        return "[]".to_string();
    };

    let normalized = values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();

    serde_json::to_string(&normalized).unwrap_or_else(|_| "[]".to_string())
}

fn participant_from_json(value: Value) -> Option<ParticipantProjection> {
    match value {
        Value::String(raw) => participant_from_string(&raw),
        Value::Object(object) => {
            let email = object
                .get("email")
                .and_then(Value::as_str)
                .and_then(|value| normalize_participant_email(Some(value)))?;
            let raw_name = object.get("name").and_then(Value::as_str);
            Some(ParticipantProjection {
                name: resolve_sender_display_name(raw_name, &email),
                email,
            })
        }
        _ => None,
    }
}

async fn update_thread_state_inner(
    pool: &SqlitePool,
    input: ThreadStateUpdateRequest,
) -> Result<ThreadRecord, ThreadError> {
    let now = chrono::Utc::now().to_rfc3339();

    // Snoozing without a timestamp must never persist NULL: the wake worker
    // only looks at rows with snooze_until set, so the thread would vanish
    // from the inbox forever. Default to the +2h the UI also assumes.
    let default_snooze;
    let snooze_until = if input.next_state == WorkflowState::Snoozed {
        match input
            .snooze_until
            .as_deref()
            .filter(|s| !s.trim().is_empty())
        {
            Some(value) => Some(value),
            None => {
                default_snooze = (chrono::Utc::now() + chrono::Duration::hours(2)).to_rfc3339();
                Some(default_snooze.as_str())
            }
        }
    } else {
        None
    };

    sqlx::query(
        "UPDATE threads
         SET workflow_state = ?, board_column = ?, workflow_changed_at = ?, snooze_until = ?, updated_at = ?
         WHERE id = ?",
    )
    .bind(input.next_state.as_db_value())
    .bind(input.next_state.as_db_value())
    .bind(&now)
    .bind(snooze_until)
    .bind(&now)
    .bind(&input.thread_id)
    .execute(pool)
    .await?;

    fetch_thread_record(pool, &input.thread_id).await
}

/// One thread with its display projection. A const so a test can read the plan.
const THREAD_RECORD_SQL: &str = "
            threads.id,
            threads.account_id,
            COALESCE(latest_message.subject, threads.subject) AS subject,
            COALESCE(latest_message.snippet, threads.snippet) AS snippet,
            COALESCE(latest_message.date, threads.last_message_date) AS last_message_date,
            threads.last_message_date AS sort_date,
            threads.message_count,
            threads.workflow_state,
            threads.unread_count,
            threads.has_attachments,
            EXISTS (
              SELECT 1
              FROM messages draft_messages
              JOIN folders draft_folders ON draft_folders.id = COALESCE(draft_messages.pending_folder_id, draft_messages.folder_id)
              WHERE draft_messages.thread_id = threads.id
                AND draft_messages.is_draft = 1
                AND LOWER(TRIM(draft_folders.folder_type)) <> 'trash'
            ) AS has_draft,
            (
              SELECT NULLIF(TRIM(draft_messages.snippet), '')
              FROM messages draft_messages
              JOIN folders draft_folders ON draft_folders.id = COALESCE(draft_messages.pending_folder_id, draft_messages.folder_id)
              WHERE draft_messages.thread_id = threads.id
                AND draft_messages.is_draft = 1
                AND LOWER(TRIM(draft_folders.folder_type)) <> 'trash'
              ORDER BY draft_messages.date DESC, draft_messages.id DESC
              LIMIT 1
            ) AS draft_snippet,
            threads.priority,
            threads.board_position,
            threads.assignee,
            threads.shared_with,
            (
              SELECT COALESCE(json_group_array(name), '[]')
              FROM (
                SELECT tags.name
                FROM thread_tags
                JOIN tags ON tags.id = thread_tags.tag_id
                WHERE thread_tags.thread_id = threads.id
                ORDER BY tags.name COLLATE NOCASE
              )
            ) AS tags,
            threads.participants,
            (
              SELECT COALESCE(json_group_array(json_object('name', name, 'email', email)), '[]')
              FROM (
                SELECT
                  email,
                  COALESCE(NULLIF(MAX(NULLIF(name, '')), ''), email) AS name
                FROM (
                  SELECT json_each.value AS email, '' AS name, messages.date AS participant_date
                  FROM messages
                  JOIN folders sent_folders ON sent_folders.id = COALESCE(messages.pending_folder_id, messages.folder_id),
                       json_each(COALESCE(messages.to_addresses, '[]'))
                  WHERE messages.thread_id = threads.id
                    AND (messages.is_sent = 1 OR LOWER(sent_folders.folder_type) = 'sent' OR LOWER(sent_folders.name) LIKE '%sent%')
                  UNION ALL
                  SELECT json_each.value AS email, '' AS name, messages.date AS participant_date
                  FROM messages
                  JOIN folders sent_folders ON sent_folders.id = COALESCE(messages.pending_folder_id, messages.folder_id),
                       json_each(COALESCE(messages.cc_addresses, '[]'))
                  WHERE messages.thread_id = threads.id
                    AND (messages.is_sent = 1 OR LOWER(sent_folders.folder_type) = 'sent' OR LOWER(sent_folders.name) LIKE '%sent%')
                )
                WHERE TRIM(email) <> ''
                GROUP BY LOWER(TRIM(email))
                ORDER BY MAX(participant_date) DESC
              )
            ) AS sent_participants,
            (
              SELECT COALESCE(json_group_array(json_object('name', name, 'email', email)), '[]')
              FROM (
                SELECT
                  email,
                  COALESCE(NULLIF(MAX(NULLIF(name, '')), ''), email) AS name
                FROM (
                  SELECT messages.from_address AS email, COALESCE(messages.from_name, '') AS name, messages.date AS participant_date
                  FROM messages
                  WHERE messages.thread_id = threads.id
                  UNION ALL
                  SELECT json_each.value AS email, '' AS name, messages.date AS participant_date
                  FROM messages, json_each(COALESCE(messages.to_addresses, '[]'))
                  WHERE messages.thread_id = threads.id
                  UNION ALL
                  SELECT json_each.value AS email, '' AS name, messages.date AS participant_date
                  FROM messages, json_each(COALESCE(messages.cc_addresses, '[]'))
                  WHERE messages.thread_id = threads.id
                  UNION ALL
                  SELECT json_each.value AS email, '' AS name, messages.date AS participant_date
                  FROM messages, json_each(COALESCE(messages.bcc_addresses, '[]'))
                  WHERE messages.thread_id = threads.id
                )
                WHERE TRIM(email) <> ''
                GROUP BY LOWER(TRIM(email))
                ORDER BY MAX(participant_date) DESC
              )
            ) AS message_participants,
            threads.category,
            threads.is_muted,
            threads.is_starred,
            threads.snooze_until,
            EXISTS (
              SELECT 1 FROM calendar_invites
              JOIN messages invite_messages ON invite_messages.id = calendar_invites.message_id
              WHERE invite_messages.thread_id = threads.id
            ) AS has_invite,
            (
              SELECT calendar_invites.rsvp_status
              FROM calendar_invites
              JOIN messages invite_messages ON invite_messages.id = calendar_invites.message_id
              WHERE invite_messages.thread_id = threads.id
              ORDER BY invite_messages.date DESC, calendar_invites.updated_at DESC
              LIMIT 1
            ) AS invite_status,
            EXISTS (
              SELECT 1 FROM calendar_invites
              JOIN messages invite_messages ON invite_messages.id = calendar_invites.message_id
              WHERE invite_messages.thread_id = threads.id AND calendar_invites.cancelled = 1
            ) AS invite_cancelled,
            EXISTS (
              SELECT 1
              FROM messages sent
              JOIN folders sent_folders ON sent_folders.id = COALESCE(sent.pending_folder_id, sent.folder_id)
              WHERE sent.thread_id = threads.id
                AND (sent.is_sent = 1 OR LOWER(sent_folders.folder_type) = 'sent' OR LOWER(sent_folders.name) LIKE '%sent%')
            ) AS sender_known,
            -- Drafts sort last: a reply you started must not turn the thread's
            -- correspondent into your own address. The id tiebreak keeps this
            -- pick identical to the subject/snippet pick when dates collide.
            latest_message.from_address AS latest_from_address,
            latest_message.from_name AS latest_from_name,
            (
              SELECT COALESCE(json_group_array(name), '[]')
              FROM (
                SELECT DISTINCT folders.name AS name
                FROM messages
                JOIN folders ON folders.id = COALESCE(messages.pending_folder_id, messages.folder_id)
                WHERE messages.thread_id = threads.id
                ORDER BY folders.name
              )
            ) AS mailbox_names
         FROM threads
         -- One index seek per thread for the latest message, instead of one per
         -- projected column. Drafts sort last and the id breaks date ties, the
         -- same pick every column used to make on its own.
         LEFT JOIN messages latest_message ON latest_message.id = (
           SELECT messages.id
           FROM messages
           WHERE messages.thread_id = threads.id
           ORDER BY messages.is_draft ASC, messages.date DESC, messages.id DESC
           LIMIT 1
         )
         WHERE threads.id = ?
         LIMIT 1";

/// One thread in the exact shape of a `list_threads` row. Shared by the state
/// transition (returning the refreshed thread) and `get_thread`, which the
/// search detail pane uses for hits outside the loaded list.
async fn fetch_thread_record(
    pool: &SqlitePool,
    thread_id: &str,
) -> Result<ThreadRecord, ThreadError> {
    let sql = thread_record_sql();
    let row = sqlx::query_as::<_, ThreadRow>(&sql)
        .bind(thread_id)
        .fetch_one(pool)
        .await
        .map_err(|error| match error {
            sqlx::Error::RowNotFound => ThreadError::NotFound,
            other => ThreadError::Persistence(other),
        })?;

    Ok(thread_record_from_row(row))
}

fn thread_record_from_row(row: ThreadRow) -> ThreadRecord {
    ThreadRecord {
        sort_date: Some(row.sort_date),
        id: row.id,
        account_id: row.account_id,
        subject: decode_mime_encoded_words(&row.subject.unwrap_or_else(|| "(no subject)".into())),
        snippet: decode_html_entities(&decode_mime_encoded_words(&row.snippet.unwrap_or_default())),
        last_message_date: row
            .last_message_date
            .unwrap_or_else(|| chrono::Utc::now().to_rfc3339()),
        message_count: row.message_count.max(0) as u32,
        workflow_state: WorkflowState::from_db_value(&row.workflow_state)
            .unwrap_or(WorkflowState::Inbox),
        unread_count: row.unread_count.max(0) as u32,
        has_attachments: row.has_attachments > 0,
        has_draft: row.has_draft > 0,
        draft_snippet: row
            .draft_snippet
            .map(|snippet| decode_html_entities(&decode_mime_encoded_words(&snippet))),
        priority: row.priority.clamp(0, 3) as u8,
        board_position: row.board_position,
        tags: parse_string_array(row.tags),
        assignee: row.assignee,
        shared_with: parse_string_array(row.shared_with),
        sent_participants: normalize_participants_json(None, row.sent_participants, None, None),
        participants: normalize_participants_json(
            row.participants,
            row.message_participants,
            row.latest_from_name,
            row.latest_from_address,
        ),
        mailbox_names: normalize_mailbox_names_json(row.mailbox_names),
        category: row.category,
        smart_bucket: row.smart_bucket,
        is_muted: row.is_muted > 0,
        is_starred: row.is_starred > 0,
        snooze_until: row.snooze_until,
        sender_known: row.sender_known > 0,
        has_invite: row.has_invite > 0,
        invite_status: row.invite_status,
        invite_cancelled: row.invite_cancelled > 0,
    }
}

fn thread_record_sql() -> String {
    format!(
        "SELECT {} AS smart_bucket, {THREAD_RECORD_SQL}",
        smart_bucket_case_sql()
    )
}

fn participant_from_string(raw: &str) -> Option<ParticipantProjection> {
    let decoded = decode_mime_encoded_words(raw.trim());
    let email = normalize_participant_email(Some(&decoded))?;
    let name = decoded
        .find('<')
        .map(|index| decoded[..index].trim().trim_matches('"').trim().to_string())
        .filter(|value| !value.is_empty());

    Some(ParticipantProjection {
        name: resolve_sender_display_name(name.as_deref(), &email),
        email,
    })
}

fn normalize_participant_email(raw: Option<&str>) -> Option<String> {
    let value = raw?.trim();
    if value.is_empty() {
        return None;
    }

    let decoded = decode_mime_encoded_words(value);
    let normalized = extract_email_address(&decoded)
        .unwrap_or(decoded)
        .trim()
        .to_string();
    (!normalized.is_empty()).then_some(normalized)
}

async fn list_smart_inbox_summary_inner(
    pool: &SqlitePool,
    account_id: Option<&str>,
) -> Result<SmartInboxSummaryRecord, ThreadError> {
    let account_id = account_id.map(str::trim).filter(|value| !value.is_empty());
    let total_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(1)
         FROM threads
         WHERE workflow_state IN ('inbox', 'todo')
           AND is_muted = 0
           AND (? IS NULL OR account_id = ?)
           AND EXISTS (
             SELECT 1 FROM accounts
             WHERE accounts.id = threads.account_id AND accounts.is_active = 1
           )
           AND EXISTS (
             SELECT 1 FROM messages
             JOIN folders ON folders.id = COALESCE(messages.pending_folder_id, messages.folder_id)
             WHERE messages.thread_id = threads.id
               AND LOWER(TRIM(COALESCE(folders.remote_id, folders.name))) = 'inbox'
           )",
    )
    .bind(account_id)
    .bind(account_id)
    .fetch_one(pool)
    .await?;

    let bucket_case = smart_bucket_case_sql();
    let mut aggregate_query = QueryBuilder::<Sqlite>::new("WITH scoped AS (SELECT threads.*, ");
    aggregate_query.push(bucket_case.as_str());
    aggregate_query.push(
        " AS bucket FROM threads
         LEFT JOIN messages latest_message ON latest_message.id = (
           SELECT id FROM messages WHERE thread_id = threads.id
           ORDER BY is_draft ASC, date DESC, id DESC LIMIT 1
         )
         WHERE workflow_state IN ('inbox', 'todo')
           AND is_muted = 0
           AND EXISTS (
             SELECT 1 FROM accounts
             WHERE accounts.id = threads.account_id AND accounts.is_active = 1
           ) AND (",
    );
    if let Some(account_id) = account_id {
        aggregate_query
            .push("threads.account_id = ")
            .push_bind(account_id.to_string());
    } else {
        aggregate_query.push("1 = 1");
    }
    aggregate_query.push(
        ") AND EXISTS (
           SELECT 1 FROM messages
           JOIN folders ON folders.id = COALESCE(messages.pending_folder_id, messages.folder_id)
           WHERE messages.thread_id = threads.id
             AND LOWER(TRIM(COALESCE(folders.remote_id, folders.name))) = 'inbox'
         )), memberships AS (
           SELECT id, unread_count, bucket FROM scoped
           UNION ALL
           SELECT id, unread_count, 'pinned' AS bucket
           FROM scoped WHERE is_starred = 1 AND bucket != 'pinned'
         )
         SELECT bucket, COUNT(1) AS count, COALESCE(SUM(unread_count), 0) AS unread_count
         FROM memberships WHERE bucket != 'personal'
         GROUP BY bucket",
    );
    let aggregates = aggregate_query
        .build_query_as::<SmartBucketAggregateRow>()
        .fetch_all(pool)
        .await?;

    let mut preview_query = QueryBuilder::<Sqlite>::new(
        "WITH scoped AS (
           SELECT threads.id, threads.subject, threads.unread_count, threads.last_message_date,
                  threads.is_starred,
                  COALESCE(NULLIF(TRIM(latest_message.from_name), ''),
                           latest_message.from_address, threads.subject) AS sender_name,
                  COALESCE(latest_message.from_address, '') AS sender_email, ",
    );
    preview_query.push(bucket_case.as_str());
    preview_query.push(
        " AS bucket
           FROM threads
           LEFT JOIN messages latest_message ON latest_message.id = (
             SELECT id FROM messages WHERE thread_id = threads.id
             ORDER BY is_draft ASC, date DESC, id DESC LIMIT 1
           )
           WHERE workflow_state IN ('inbox', 'todo')
             AND is_muted = 0
             AND EXISTS (
               SELECT 1 FROM accounts
               WHERE accounts.id = threads.account_id AND accounts.is_active = 1
             ) AND (",
    );
    if let Some(account_id) = account_id {
        preview_query
            .push("threads.account_id = ")
            .push_bind(account_id.to_string());
    } else {
        preview_query.push("1 = 1");
    }
    preview_query.push(
        ") AND EXISTS (
          SELECT 1 FROM messages
          JOIN folders ON folders.id = COALESCE(messages.pending_folder_id, messages.folder_id)
          WHERE messages.thread_id = threads.id
            AND LOWER(TRIM(COALESCE(folders.remote_id, folders.name))) = 'inbox'
        )), memberships AS (
          SELECT id, subject, unread_count, last_message_date, sender_name, sender_email, bucket
          FROM scoped
          UNION ALL
          SELECT id, subject, unread_count, last_message_date, sender_name, sender_email, 'pinned' AS bucket
          FROM scoped WHERE is_starred = 1 AND bucket != 'pinned'
        ), ranked AS (
          SELECT *, ROW_NUMBER() OVER (PARTITION BY bucket ORDER BY last_message_date DESC, id DESC) AS rank
          FROM memberships WHERE bucket != 'personal'
        )
        SELECT bucket, id, sender_name, sender_email, COALESCE(subject, '(kein Betreff)') AS subject,
               unread_count
        FROM ranked WHERE rank <= 4 ORDER BY bucket, rank",
    );
    let preview_rows = preview_query
        .build_query_as::<SmartBucketPreviewRow>()
        .fetch_all(pool)
        .await?;

    let mut previews = HashMap::<String, Vec<SmartInboxPreviewRecord>>::new();
    for row in preview_rows {
        previews
            .entry(row.bucket)
            .or_default()
            .push(SmartInboxPreviewRecord {
                id: row.id,
                sender_name: row.sender_name,
                sender_email: row.sender_email,
                subject: decode_mime_encoded_words(&row.subject),
                unread_count: row.unread_count.max(0) as u32,
            });
    }
    let order = [
        "priority",
        "pinned",
        "invitations",
        "notifications",
        "newsletter",
    ];
    let aggregates = aggregates
        .into_iter()
        .map(|row| (row.bucket.clone(), row))
        .collect::<HashMap<_, _>>();
    let buckets = order
        .into_iter()
        .filter_map(|key| aggregates.get(key).map(|row| (key, row)))
        .map(|(key, row)| SmartInboxBucketRecord {
            key: key.to_string(),
            count: row.count.max(0) as u32,
            unread_count: row.unread_count.max(0) as u32,
            preview_items: previews.remove(key).unwrap_or_default(),
        })
        .collect();

    Ok(SmartInboxSummaryRecord {
        total_count: total_count.max(0) as u32,
        buckets,
    })
}

/// Every projection joins the latest message once. Reuse it for classification
/// as well, so returning a bucket does not add per-thread message lookups.
const MACHINE_NOTIFICATION_SQL: &str = "(
  (threads.category IS NULL OR threads.category = 'personal')
  AND (
      LOWER(TRIM(latest_message.from_address)) GLOB 'no-reply*'
      OR LOWER(TRIM(latest_message.from_address)) GLOB 'noreply*'
      OR LOWER(TRIM(latest_message.from_address)) GLOB '*[-._]no-reply@*'
      OR LOWER(TRIM(latest_message.from_address)) GLOB '*[-._]noreply@*'
      OR LOWER(TRIM(latest_message.from_address)) GLOB 'do-not-reply*'
      OR LOWER(TRIM(latest_message.from_address)) GLOB 'donotreply*'
      OR LOWER(TRIM(latest_message.from_address)) GLOB '*[-._]donotreply@*'
      OR LOWER(TRIM(latest_message.from_address)) GLOB '*@no-reply.*'
      OR LOWER(TRIM(latest_message.from_address)) GLOB '*@noreply.*'
      OR LOWER(TRIM(latest_message.from_address)) GLOB '*@notifications.*'
      OR LOWER(TRIM(latest_message.from_address)) GLOB '*@notification.*'
      OR LOWER(TRIM(latest_message.from_address)) GLOB 'notifications@*'
      OR LOWER(TRIM(latest_message.from_address)) GLOB 'notification@*'
      OR LOWER(TRIM(latest_message.from_address)) GLOB 'automated@*'
      OR LOWER(TRIM(latest_message.from_address)) GLOB 'robot@*'
      OR LOWER(TRIM(latest_message.from_address)) GLOB 'bot@*'
      OR LOWER(TRIM(latest_message.from_address)) GLOB 'mailer-daemon@*'
      OR LOWER(TRIM(latest_message.from_address)) GLOB 'alerts@*'
      OR LOWER(TRIM(latest_message.from_address)) GLOB 'alert@*'
      OR LOWER(TRIM(latest_message.from_address)) GLOB 'billing@*'
      OR LOWER(TRIM(latest_message.from_address)) GLOB 'invoice@*'
      OR LOWER(TRIM(latest_message.from_address)) GLOB 'receipts@*'
      OR LOWER(TRIM(latest_message.from_address)) GLOB 'delivery@*'
      OR LOWER(TRIM(latest_message.from_address)) GLOB 'orders@*'
      OR LOWER(TRIM(latest_message.from_address)) GLOB 'order@*'
      OR LOWER(TRIM(latest_message.from_address)) GLOB 'newsletter@*'
      OR LOWER(TRIM(latest_message.from_address)) GLOB 'info@*'
      OR LOWER(TRIM(latest_message.from_address)) GLOB 'hello@*'
      OR LOWER(TRIM(latest_message.from_address)) GLOB 'support@*'
      OR LOWER(TRIM(latest_message.from_address)) GLOB 'service@*'
      OR LOWER(TRIM(latest_message.from_address)) GLOB 'contact@*'
      OR LOWER(TRIM(latest_message.from_address)) GLOB 'booking@*'
      OR LOWER(TRIM(latest_message.from_address)) GLOB 'bookings@*'
      OR LOWER(TRIM(latest_message.from_address)) GLOB 'reservation@*'
      OR LOWER(TRIM(latest_message.from_address)) GLOB 'reservations@*'
      OR LOWER(TRIM(latest_message.from_address)) GLOB 'mail@*'
      OR LOWER(TRIM(latest_message.from_address)) GLOB 'post@*'
  )
)";

// Repair the projection of old heuristic categories without rewriting mail.
// Explicit sender/thread choices still take precedence over address heuristics.
const CODE_NOTIFICATION_SQL: &str = "(
  threads.category_source = 'heuristic'
  AND LOWER(TRIM(latest_message.from_address)) GLOB 'gitlab@*'
  AND NOT EXISTS (
    SELECT 1 FROM json_each(COALESCE(latest_message.provider_labels, '[]'))
    WHERE UPPER(TRIM(value)) IN ('CATEGORY_PERSONAL', 'CATEGORY_PROMOTIONS')
  )
)";

fn smart_bucket_case_sql() -> String {
    format!(
        "CASE
       WHEN threads.priority >= 2 THEN 'priority'
       WHEN threads.is_starred = 1 THEN 'pinned'
       WHEN EXISTS (
         SELECT 1 FROM calendar_invites
         JOIN messages bucket_invite_messages ON bucket_invite_messages.id = calendar_invites.message_id
         WHERE bucket_invite_messages.thread_id = threads.id
       ) THEN 'invitations'
       WHEN threads.category IN ('updates', 'social') OR {MACHINE_NOTIFICATION_SQL} OR {CODE_NOTIFICATION_SQL} THEN 'notifications'
       WHEN threads.category = 'promotions' THEN 'newsletter'
       ELSE 'personal'
     END"
    )
}

fn push_folder_candidates(query: &mut QueryBuilder<'_, Sqlite>) {
    // Remove RHS affinity so SQLite can seek the COALESCE expression index.
    query.push("SELECT mailbox_messages.thread_id FROM folders mailbox_folders
        CROSS JOIN messages mailbox_messages INDEXED BY idx_messages_effective_folder_thread_read
        WHERE COALESCE(mailbox_messages.pending_folder_id, mailbox_messages.folder_id) = +mailbox_folders.id");
}

const THREAD_LIST_LATEST_JOIN: &str = " LEFT JOIN messages latest_message ON latest_message.id = (
           SELECT messages.id
           FROM messages
           WHERE messages.thread_id = threads.id
           ORDER BY messages.is_draft ASC, messages.date DESC, messages.id DESC
           LIMIT 1
         ) ";

fn thread_list_query(
    input: &ThreadListRequest,
    addresses_ready: bool,
) -> Result<QueryBuilder<'static, Sqlite>, ThreadError> {
    let requested = input.limit.unwrap_or_else(thread_page_size).clamp(1, 1_000);
    // Fetch one extra row to learn whether another page exists without a
    // second COUNT query over the same predicate.
    let limit = requested + 1;
    let mut query = QueryBuilder::<Sqlite>::new(
        "WITH listed_thread_ids AS MATERIALIZED (SELECT threads.id FROM threads",
    );
    let dense_inbox = input.mailbox_kind.as_deref().map(str::trim) == Some("section")
        && input.mailbox_value.as_deref().map(str::trim) == Some("inbox")
        && input
            .contact_email
            .as_deref()
            .is_none_or(|email| email.trim().is_empty())
        && input
            .smart_bucket
            .as_deref()
            .is_none_or(|bucket| bucket.trim().is_empty());
    if dense_inbox {
        query.push(" INDEXED BY idx_threads_inbox_date_id");
    }
    if input
        .smart_bucket
        .as_deref()
        .map(str::trim)
        .is_some_and(|bucket| !bucket.is_empty() && bucket != "pinned")
    {
        query.push(THREAD_LIST_LATEST_JOIN);
    }
    query.push(
        " WHERE EXISTS (SELECT 1 FROM messages WHERE messages.thread_id = threads.id)
           AND EXISTS (
             SELECT 1 FROM accounts
             WHERE accounts.id = threads.account_id AND accounts.is_active = 1
           ) ",
    );

    if let Some(account_id) = input
        .account_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        query.push(" AND threads.account_id = ");
        query.push_bind(account_id.to_string());
    }

    if let Some(workflow_state) = input
        .workflow_state
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "all")
    {
        query.push(" AND threads.workflow_state = ");
        query.push_bind(workflow_state.to_string());
    }

    if let Some(contact_email) = input
        .contact_email
        .as_deref()
        .and_then(|value| normalize_participant_email(Some(value)))
    {
        push_contact_filter(&mut query, &contact_email, addresses_ready);
    }

    match (
        input.mailbox_kind.as_deref().map(str::trim),
        input.mailbox_value.as_deref().map(str::trim),
    ) {
        (Some("folder"), Some(folder)) if !folder.is_empty() => {
            query.push(" AND threads.id IN (");
            push_folder_candidates(&mut query);
            query
                .push(" AND (mailbox_folders.name = ")
                .push_bind(folder.to_string());
            query
                .push(" OR mailbox_folders.full_path = ")
                .push_bind(folder.to_string());
            query.push("))");
        }
        (Some("section"), Some(section)) => {
            if let Some(predicate) = mailbox_section_predicate(section) {
                if dense_inbox {
                    query.push(" AND EXISTS (SELECT 1 FROM messages mailbox_messages
                        INDEXED BY idx_messages_thread_effective_folder_read
                        JOIN folders mailbox_folders ON mailbox_folders.id = COALESCE(mailbox_messages.pending_folder_id, mailbox_messages.folder_id)
                        WHERE mailbox_messages.thread_id = threads.id AND ");
                    query.push(predicate).push(")");
                } else {
                    query.push(" AND threads.id IN (");
                    push_folder_candidates(&mut query);
                    query.push(" AND ").push(predicate).push(")");
                }
            }
            if section == "inbox" {
                query.push(" AND threads.workflow_state IN ('inbox', 'todo')");
            }
        }
        _ => {}
    }

    if let Some(bucket) = input
        .smart_bucket
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        push_smart_bucket_filter(&mut query, bucket)?;
    }

    // Both directions keep the immutable sort position captured by the page.
    let backwards = input.before_cursor.is_some();
    if let Some(cursor) = input.before_cursor.as_ref().or(input.cursor.as_ref()) {
        let comparison = if backwards { " > " } else { " < " };
        query
            .push(" AND (threads.last_message_date")
            .push(comparison)
            .push_bind(cursor.last_message_date.clone())
            .push(" OR (threads.last_message_date = ")
            .push_bind(cursor.last_message_date.clone())
            .push(" AND threads.id")
            .push(comparison)
            .push_bind(cursor.id.clone())
            .push("))");
    }

    query.push(if backwards {
        " ORDER BY threads.last_message_date ASC, threads.id ASC LIMIT "
    } else {
        " ORDER BY threads.last_message_date DESC, threads.id DESC LIMIT "
    });
    query.push_bind(limit);

    query.push(") SELECT ");
    query.push(smart_bucket_case_sql());
    query.push(
        " AS smart_bucket,
            threads.id,
            threads.account_id,
            COALESCE(latest_message.subject, threads.subject) AS subject,
            COALESCE(latest_message.snippet, threads.snippet) AS snippet,
            COALESCE(latest_message.date, threads.last_message_date) AS last_message_date,
            threads.last_message_date AS sort_date,
            threads.message_count,
            threads.workflow_state,
            threads.unread_count,
            threads.has_attachments,
            EXISTS (
              SELECT 1
              FROM messages draft_messages
              JOIN folders draft_folders ON draft_folders.id = COALESCE(draft_messages.pending_folder_id, draft_messages.folder_id)
              WHERE draft_messages.thread_id = threads.id
                AND draft_messages.is_draft = 1
                AND LOWER(TRIM(draft_folders.folder_type)) <> 'trash'
            ) AS has_draft,
            (
              SELECT NULLIF(TRIM(draft_messages.snippet), '')
              FROM messages draft_messages
              JOIN folders draft_folders ON draft_folders.id = COALESCE(draft_messages.pending_folder_id, draft_messages.folder_id)
              WHERE draft_messages.thread_id = threads.id
                AND draft_messages.is_draft = 1
                AND LOWER(TRIM(draft_folders.folder_type)) <> 'trash'
              ORDER BY draft_messages.date DESC, draft_messages.id DESC
              LIMIT 1
            ) AS draft_snippet,
            threads.priority,
            threads.board_position,
            threads.assignee,
            threads.shared_with,
            (
              SELECT COALESCE(json_group_array(name), '[]')
              FROM (
                SELECT tags.name
                FROM thread_tags
                JOIN tags ON tags.id = thread_tags.tag_id
                WHERE thread_tags.thread_id = threads.id
                ORDER BY tags.name COLLATE NOCASE
              )
            ) AS tags,
            threads.participants,
            (
              SELECT COALESCE(json_group_array(json_object('name', name, 'email', email)), '[]')
              FROM (
                SELECT
                  email,
                  COALESCE(NULLIF(MAX(NULLIF(name, '')), ''), email) AS name
                FROM (
                  SELECT json_each.value AS email, '' AS name, messages.date AS participant_date
                  FROM messages
                  JOIN folders sent_folders ON sent_folders.id = COALESCE(messages.pending_folder_id, messages.folder_id),
                       json_each(COALESCE(messages.to_addresses, '[]'))
                  WHERE messages.thread_id = threads.id
                    AND (messages.is_sent = 1 OR LOWER(sent_folders.folder_type) = 'sent' OR LOWER(sent_folders.name) LIKE '%sent%')
                  UNION ALL
                  SELECT json_each.value AS email, '' AS name, messages.date AS participant_date
                  FROM messages
                  JOIN folders sent_folders ON sent_folders.id = COALESCE(messages.pending_folder_id, messages.folder_id),
                       json_each(COALESCE(messages.cc_addresses, '[]'))
                  WHERE messages.thread_id = threads.id
                    AND (messages.is_sent = 1 OR LOWER(sent_folders.folder_type) = 'sent' OR LOWER(sent_folders.name) LIKE '%sent%')
                )
                WHERE TRIM(email) <> ''
                GROUP BY LOWER(TRIM(email))
                ORDER BY MAX(participant_date) DESC
              )
            ) AS sent_participants,
            (
              SELECT COALESCE(json_group_array(json_object('name', name, 'email', email)), '[]')
              FROM (
                SELECT
                  email,
                  COALESCE(NULLIF(MAX(NULLIF(name, '')), ''), email) AS name
                FROM (
                  SELECT messages.from_address AS email, COALESCE(messages.from_name, '') AS name, messages.date AS participant_date
                  FROM messages
                  WHERE messages.thread_id = threads.id
                  UNION ALL
                  SELECT json_each.value AS email, '' AS name, messages.date AS participant_date
                  FROM messages, json_each(COALESCE(messages.to_addresses, '[]'))
                  WHERE messages.thread_id = threads.id
                  UNION ALL
                  SELECT json_each.value AS email, '' AS name, messages.date AS participant_date
                  FROM messages, json_each(COALESCE(messages.cc_addresses, '[]'))
                  WHERE messages.thread_id = threads.id
                  UNION ALL
                  SELECT json_each.value AS email, '' AS name, messages.date AS participant_date
                  FROM messages, json_each(COALESCE(messages.bcc_addresses, '[]'))
                  WHERE messages.thread_id = threads.id
                )
                WHERE TRIM(email) <> ''
                GROUP BY LOWER(TRIM(email))
                ORDER BY MAX(participant_date) DESC
              )
            ) AS message_participants,
            threads.category,
            threads.is_muted,
            threads.is_starred,
            threads.snooze_until,
            EXISTS (
              SELECT 1 FROM calendar_invites
              JOIN messages invite_messages ON invite_messages.id = calendar_invites.message_id
              WHERE invite_messages.thread_id = threads.id
            ) AS has_invite,
            (
              SELECT calendar_invites.rsvp_status
              FROM calendar_invites
              JOIN messages invite_messages ON invite_messages.id = calendar_invites.message_id
              WHERE invite_messages.thread_id = threads.id
              ORDER BY invite_messages.date DESC, calendar_invites.updated_at DESC
              LIMIT 1
            ) AS invite_status,
            EXISTS (
              SELECT 1 FROM calendar_invites
              JOIN messages invite_messages ON invite_messages.id = calendar_invites.message_id
              WHERE invite_messages.thread_id = threads.id AND calendar_invites.cancelled = 1
            ) AS invite_cancelled,
            EXISTS (
              SELECT 1
              FROM messages sent
              JOIN folders sent_folders ON sent_folders.id = COALESCE(sent.pending_folder_id, sent.folder_id)
              WHERE sent.thread_id = threads.id
                AND (sent.is_sent = 1 OR LOWER(sent_folders.folder_type) = 'sent' OR LOWER(sent_folders.name) LIKE '%sent%')
            ) AS sender_known,
            -- Drafts sort last: a reply you started must not turn the thread's
            -- correspondent into your own address. The id tiebreak keeps this
            -- pick identical to the subject/snippet pick when dates collide.
            latest_message.from_address AS latest_from_address,
            latest_message.from_name AS latest_from_name,
            (
              SELECT COALESCE(json_group_array(name), '[]')
              FROM (
                SELECT DISTINCT folders.name AS name
                FROM messages
                JOIN folders ON folders.id = COALESCE(messages.pending_folder_id, messages.folder_id)
                WHERE messages.thread_id = threads.id
                ORDER BY folders.name
              )
            ) AS mailbox_names
         FROM listed_thread_ids
         CROSS JOIN threads ON threads.id = listed_thread_ids.id",
    );
    query.push(THREAD_LIST_LATEST_JOIN);
    query.push(if input.before_cursor.is_some() {
        " ORDER BY threads.last_message_date ASC, threads.id ASC"
    } else {
        " ORDER BY threads.last_message_date DESC, threads.id DESC"
    });
    Ok(query)
}

async fn list_threads_inner(
    pool: &SqlitePool,
    input: ThreadListRequest,
) -> Result<ThreadPageRecord, ThreadError> {
    let requested = input.limit.unwrap_or_else(thread_page_size).clamp(1, 1_000);
    let addresses_ready = input.contact_email.is_none()
        || !crate::db::data_repairs::is_pending(pool, &crate::db::data_repairs::MESSAGE_ADDRESSES)
            .await?;
    let mut query = thread_list_query(&input, addresses_ready)?;
    let mut rows = query.build_query_as::<ThreadRow>().fetch_all(pool).await?;

    let has_more = rows.len() as i64 > requested;
    if has_more {
        rows.truncate(requested as usize);
    }
    // The cursor must use the stored sort columns, not the display values the
    // SELECT computes — otherwise the next page skips or repeats rows.
    let next_cursor = has_more
        .then(|| {
            rows.last().map(|row| ThreadCursor {
                last_message_date: row.sort_date.clone(),
                id: row.id.clone(),
            })
        })
        .flatten();

    if input.before_cursor.is_some() {
        rows.reverse();
    }

    tracing::info!(
        rows = rows.len(),
        has_more,
        account_id = input.account_id.as_deref().unwrap_or(""),
        contact_email = input.contact_email.as_deref().unwrap_or(""),
        mailbox_kind = input.mailbox_kind.as_deref().unwrap_or(""),
        mailbox_value = input.mailbox_value.as_deref().unwrap_or(""),
        workflow_state = input.workflow_state.as_deref().unwrap_or(""),
        smart_bucket = input.smart_bucket.as_deref().unwrap_or(""),
        "listed threads"
    );

    let threads = rows
        .into_iter()
        .map(|row| ThreadRecord {
            sort_date: Some(row.sort_date),
            id: row.id,
            account_id: row.account_id,
            subject: decode_mime_encoded_words(
                &row.subject.unwrap_or_else(|| "(no subject)".into()),
            ),
            snippet: decode_html_entities(&decode_mime_encoded_words(
                &row.snippet.unwrap_or_default(),
            )),
            last_message_date: row
                .last_message_date
                .unwrap_or_else(|| chrono::Utc::now().to_rfc3339()),
            message_count: row.message_count.max(0) as u32,
            workflow_state: WorkflowState::from_db_value(&row.workflow_state)
                .unwrap_or(WorkflowState::Inbox),
            unread_count: row.unread_count.max(0) as u32,
            has_attachments: row.has_attachments > 0,
            has_draft: row.has_draft > 0,
            draft_snippet: row
                .draft_snippet
                .map(|snippet| decode_html_entities(&decode_mime_encoded_words(&snippet))),
            priority: row.priority.clamp(0, 3) as u8,
            board_position: row.board_position,
            tags: parse_string_array(row.tags),
            assignee: row.assignee,
            shared_with: parse_string_array(row.shared_with),
            sent_participants: normalize_participants_json(None, row.sent_participants, None, None),
            participants: normalize_participants_json(
                row.participants,
                row.message_participants,
                row.latest_from_name,
                row.latest_from_address,
            ),
            mailbox_names: normalize_mailbox_names_json(row.mailbox_names),
            category: row.category,
            smart_bucket: row.smart_bucket,
            is_muted: row.is_muted > 0,
            is_starred: row.is_starred > 0,
            snooze_until: row.snooze_until,
            sender_known: row.sender_known > 0,
            has_invite: row.has_invite > 0,
            invite_status: row.invite_status,
            invite_cancelled: row.invite_cancelled > 0,
        })
        .collect();

    Ok(ThreadPageRecord {
        threads,
        next_cursor,
        has_more,
    })
}

fn push_smart_bucket_filter(
    query: &mut QueryBuilder<'_, Sqlite>,
    bucket: &str,
) -> Result<(), ThreadError> {
    if bucket == "pinned" {
        // A priority thread can also belong to the independent pinned folder.
        query.push(" AND threads.is_starred = 1");
    } else if [
        "priority",
        "invitations",
        "notifications",
        "newsletter",
        "personal",
    ]
    .contains(&bucket)
    {
        query.push(" AND (");
        query.push(smart_bucket_case_sql());
        query.push(") = ").push_bind(bucket.to_owned());
    } else {
        return Err(ThreadError::InvalidSmartInboxBucket {
            value: bucket.to_owned(),
        });
    }
    Ok(())
}

fn push_contact_filter(
    query: &mut QueryBuilder<'_, Sqlite>,
    contact_email: &str,
    addresses_ready: bool,
) {
    if addresses_ready {
        query
            .push(
                " AND threads.id IN (SELECT thread_id FROM message_addresses
            WHERE email = LOWER(TRIM(",
            )
            .push_bind(contact_email.to_string())
            .push(")))");
        return;
    }
    // The fallback runs only until the resumable address repair finishes.
    query.push(
        " AND threads.id IN (
            SELECT contact_messages.thread_id
            FROM messages contact_messages
            WHERE (
                LOWER(TRIM(contact_messages.from_address)) = LOWER(TRIM(",
    );
    query.push_bind(contact_email.to_string());
    query.push(
        "))
                OR EXISTS (
                  SELECT 1 FROM json_each(contact_messages.to_addresses)
                  WHERE LOWER(TRIM(json_each.value)) = LOWER(TRIM(",
    );
    query.push_bind(contact_email.to_string());
    query.push(
        "))
                )
                OR EXISTS (
                  SELECT 1 FROM json_each(COALESCE(contact_messages.cc_addresses, '[]'))
                  WHERE LOWER(TRIM(json_each.value)) = LOWER(TRIM(",
    );
    query.push_bind(contact_email.to_string());
    query.push(
        "))
                )
                OR EXISTS (
                  SELECT 1 FROM json_each(COALESCE(contact_messages.bcc_addresses, '[]'))
                  WHERE LOWER(TRIM(json_each.value)) = LOWER(TRIM(",
    );
    query.push_bind(contact_email.to_string());
    query.push(
        "))
                )
              )
        )",
    );
}

fn mailbox_section_predicate(section: &str) -> Option<&'static str> {
    match section {
        "archive" => Some(
            "(LOWER(mailbox_folders.folder_type) = 'archive'
              OR LOWER(mailbox_folders.name) LIKE '%archive%'
              OR LOWER(mailbox_folders.name) LIKE '%archiv%')",
        ),
        "sent" => Some(
            "(LOWER(mailbox_folders.folder_type) = 'sent'
              OR LOWER(mailbox_folders.name) LIKE '%sent%')",
        ),
        "drafts" => Some(
            "(LOWER(mailbox_folders.folder_type) = 'drafts'
              OR LOWER(mailbox_folders.name) LIKE '%draft%')",
        ),
        "trash" => Some(
            "(LOWER(mailbox_folders.folder_type) = 'trash'
              OR LOWER(mailbox_folders.name) LIKE '%trash%'
              OR LOWER(mailbox_folders.name) LIKE '%deleted%')",
        ),
        "spam" => Some(
            "(LOWER(mailbox_folders.folder_type) = 'spam'
              OR LOWER(mailbox_folders.name) LIKE '%spam%'
              OR LOWER(mailbox_folders.name) LIKE '%junk%')",
        ),
        "inbox" => Some(
            "(LOWER(TRIM(mailbox_folders.name)) = 'inbox'
              OR (
                LOWER(mailbox_folders.folder_type) = 'inbox'
                AND LOWER(TRIM(COALESCE(mailbox_folders.full_path, mailbox_folders.name))) NOT LIKE 'inbox.%'
                AND LOWER(TRIM(COALESCE(mailbox_folders.full_path, mailbox_folders.name))) NOT LIKE 'inbox/%'
              ))",
        ),
        "notifications" => Some(
            "(LOWER(mailbox_folders.name) LIKE '%notif%'
              OR LOWER(mailbox_folders.name) LIKE '%updates%'
              OR LOWER(mailbox_folders.name) LIKE '%forum%')",
        ),
        "blocked" => Some("LOWER(mailbox_folders.name) LIKE '%blocked%'"),
        _ => None,
    }
}
