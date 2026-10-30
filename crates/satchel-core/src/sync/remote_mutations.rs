use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{FromRow, QueryBuilder, Sqlite, SqlitePool, Transaction};

use crate::{
    providers::{
        folder_mapping::default_remote_folder,
        provider_for_account,
        traits::{FlagUpdate, MessageRef},
    },
    sync::queue::{QueuedAction, SyncQueue},
};

pub const ACTION_REMOTE_MUTATION: &str = "remote_mutation";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RemoteMutationPayload {
    UndoTrash {
        receipt_id: String,
    },
    Move {
        refs: Vec<MessageRef>,
        target_remote_folder_id: String,
    },
    /// A blocked-sender move is one immutable remote ref per queue row, so a
    /// new folder-scoped ref gets a separate durable provider operation.
    BlockedMove {
        blocked_thread_id: String,
        refs: Vec<MessageRef>,
        target_remote_folder_id: String,
        /// Persisted before the provider call so an ambiguous IMAP response
        /// can be reconciled from the source mailbox on retry.
        #[serde(default)]
        provider_move_started: bool,
        /// Set after the provider accepted the move. A retry never submits
        /// the source refs again once this bit is durable.
        #[serde(default)]
        provider_move_completed: bool,
        /// UIDVALIDITY for every IMAP source mailbox before the first MOVE.
        #[serde(default)]
        source_uidvalidity_before: Option<HashMap<String, u32>>,
    },
    /// A workflow-owned move is deliberately one thread per queue row. The
    /// stable thread id makes Undo exact even for IMAP, where the same UID can
    /// name unrelated messages in different folders.
    WorkflowMove {
        workflow_thread_id: String,
        source_folder_type: String,
        target_folder_type: String,
        refs: Vec<MessageRef>,
        target_remote_folder_id: String,
        /// Persisted before the provider call. If the process dies while the
        /// request is in flight, IMAP retries reconcile the source mailbox
        /// before deciding whether any source refs still need moving.
        #[serde(default)]
        provider_move_started: bool,
        /// UIDVALIDITY for every IMAP source mailbox before the first MOVE.
        /// A retry refuses stale UIDs if a mailbox generation changed.
        #[serde(default)]
        source_uidvalidity_before: Option<HashMap<String, u32>>,
        /// Set by the queue worker after the provider accepted the move. A
        /// subsequent retry then performs reconciliation only instead of
        /// repeating an IMAP move whose source UID no longer exists.
        #[serde(default)]
        provider_move_completed: bool,
        /// IMAP UIDs present in the target immediately before this move. New
        /// target UIDs are proven by set difference during reconciliation.
        #[serde(default)]
        target_remote_ids_before: Option<Vec<String>>,
        /// UIDVALIDITY generation of the pre-move IMAP target snapshot.
        #[serde(default)]
        target_uidvalidity_before: Option<u32>,
        /// Set atomically with deleting the old local source copies. A retry
        /// after a crash or queue-delete failure can then safely no-op.
        #[serde(default)]
        projection_completed: bool,
    },
    Delete {
        refs: Vec<MessageRef>,
    },
    Flags {
        refs: Vec<MessageRef>,
        read: Option<bool>,
        starred: Option<bool>,
    },
    DeleteDraft {
        /// Local draft whose former account owns this cleanup. When that draft
        /// returns to the account, the cleanup becomes obsolete.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        local_draft_id: Option<String>,
        /// Known provider draft id, if the local row already carried one.
        remote_draft_id: Option<String>,
        /// Otherwise the message id the draft was imported under, which the
        /// worker resolves to a draft id when it runs.
        remote_message_id: Option<String>,
        source_folder: Option<String>,
    },
    CreateFolder {
        path: String,
    },
    RenameFolder {
        from_path: String,
        to_path: String,
    },
    DeleteFolder {
        path: String,
    },
    /// File the copy of a mail that has already been delivered.
    ///
    /// Only ever appends: the message left over SMTP before this row existed,
    /// so a retry here must not go anywhere near the transport again. The raw
    /// bytes travel in the payload because they are the wire form — a rebuild
    /// would file different boundaries, and for a crypto send the plaintext.
    AppendSent {
        raw_base64: String,
        folder_hint: Option<String>,
    },
}

/// Return the thread id owned by a durable remote move, regardless of the
/// feature that created it.
pub fn remote_move_thread_id(payload: &RemoteMutationPayload) -> Option<&str> {
    match payload {
        RemoteMutationPayload::WorkflowMove {
            workflow_thread_id, ..
        } => Some(workflow_thread_id),
        RemoteMutationPayload::BlockedMove {
            blocked_thread_id, ..
        } => Some(blocked_thread_id),
        _ => None,
    }
}

/// Parse and extract a durable remote-move thread id. Malformed and unrelated
/// queue payloads are intentionally ignored by lifecycle guards.
pub fn remote_move_thread_id_from_json(payload: &str) -> Option<String> {
    if let Ok(payload) = serde_json::from_str::<RemoteMutationPayload>(payload) {
        return remote_move_thread_id(&payload).map(str::to_owned);
    }

    // Older queue rows may contain only the move discriminator and its thread
    // id. Keep lifecycle guards compatible with those rows while all newly
    // written operations use the typed payload above.
    let value = serde_json::from_str::<serde_json::Value>(payload).ok()?;
    match value.get("kind").and_then(serde_json::Value::as_str) {
        Some("workflow_move") => value
            .get("workflow_thread_id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        Some("blocked_move") => value
            .get("blocked_thread_id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        _ => None,
    }
}

#[derive(Debug, Clone, FromRow)]
struct RemoteMessageRow {
    account_id: String,
    provider: String,
    email: String,
    config: String,
    remote_id: Option<String>,
    folder_remote_id: Option<String>,
    folder_path: String,
    folder_uidvalidity: Option<i64>,
}

#[derive(Debug, Clone, FromRow)]
struct FolderTargetRow {
    account_id: String,
    remote_id: Option<String>,
    full_path: String,
}

#[derive(Debug, Clone)]
pub struct PreparedWorkflowMove {
    pub account_id: String,
    pub source_folder_type: String,
    pub target_folder_type: String,
    pub refs: Vec<MessageRef>,
    pub target_remote_folder_id: String,
}

impl PreparedWorkflowMove {
    pub fn into_action(
        self,
        thread_id: &str,
        scheduled_for: chrono::DateTime<chrono::Utc>,
    ) -> Result<QueuedAction, String> {
        let payload = serde_json::to_value(RemoteMutationPayload::WorkflowMove {
            workflow_thread_id: thread_id.to_string(),
            source_folder_type: self.source_folder_type,
            target_folder_type: self.target_folder_type,
            refs: self.refs,
            target_remote_folder_id: self.target_remote_folder_id,
            provider_move_started: false,
            source_uidvalidity_before: None,
            provider_move_completed: false,
            target_remote_ids_before: None,
            target_uidvalidity_before: None,
            projection_completed: false,
        })
        .map_err(|err| err.to_string())?;
        Ok(
            QueuedAction::new(self.account_id, ACTION_REMOTE_MUTATION, payload)
                .scheduled(scheduled_for),
        )
    }
}

/// Resolve only message copies in the semantic source location. In
/// particular, an Inbox -> Aside move must never carry Sent or Draft copies of
/// the same conversation.
pub async fn prepare_thread_workflow_moves(
    pool: &SqlitePool,
    thread_id: &str,
    source_folder_type: &str,
    target_folder_type: &str,
) -> Result<Vec<PreparedWorkflowMove>, String> {
    let ids = [thread_id.to_string()];
    let groups = grouped_refs_for_threads_in_kind(pool, &ids, source_folder_type).await?;
    let mut moves = Vec::new();
    for (account_id, refs) in groups {
        if refs.is_empty() {
            continue;
        }
        let target_remote_folder_id =
            remote_folder_for_kind(pool, &account_id, target_folder_type).await?;
        if target_remote_folder_id.trim().is_empty() {
            continue;
        }
        ensure_local_workflow_target_folder(
            pool,
            &account_id,
            target_folder_type,
            &target_remote_folder_id,
        )
        .await?;
        moves.push(PreparedWorkflowMove {
            account_id,
            source_folder_type: source_folder_type.to_string(),
            target_folder_type: target_folder_type.to_string(),
            refs,
            target_remote_folder_id,
        });
    }
    Ok(moves)
}

/// Re-read workflow refs while the caller holds its `BEGIN IMMEDIATE`
/// reservation. The preflight version above may provision a missing target,
/// but only this snapshot is authoritative against a simultaneous queue claim.
pub async fn prepare_thread_workflow_moves_in_transaction(
    tx: &mut Transaction<'_, Sqlite>,
    thread_id: &str,
    source_folder_type: &str,
    target_folder_type: &str,
) -> Result<Vec<PreparedWorkflowMove>, String> {
    let rows = sqlx::query_as::<_, (String, String, Option<String>, String, Option<i64>, String)>(
        "SELECT m.account_id,
                m.remote_id,
                f.remote_id,
                f.full_path,
                f.uidvalidity,
                (
                  SELECT COALESCE(NULLIF(TRIM(target.remote_id), ''), target.full_path)
                  FROM folders target
                  WHERE target.account_id = m.account_id
                    AND LOWER(TRIM(target.folder_type)) = LOWER(TRIM(?))
                  ORDER BY target.updated_at DESC LIMIT 1
                ) AS target_remote_folder_id
         FROM messages m
         JOIN accounts a ON a.id = m.account_id
         JOIN folders f ON f.id = m.folder_id
         WHERE a.kind = 'email'
           AND m.thread_id = ?
           AND m.remote_id IS NOT NULL
           AND LOWER(TRIM(f.folder_type)) = LOWER(TRIM(?))",
    )
    .bind(target_folder_type)
    .bind(thread_id)
    .bind(source_folder_type)
    .fetch_all(&mut **tx)
    .await
    .map_err(|err| err.to_string())?;

    let mut grouped = HashMap::<(String, String), Vec<MessageRef>>::new();
    for (
        account_id,
        remote_id,
        folder_remote_id,
        folder_path,
        folder_uidvalidity,
        target_remote_id,
    ) in rows
    {
        grouped
            .entry((account_id, target_remote_id))
            .or_default()
            .push(MessageRef {
                remote_id,
                source_folder: folder_remote_id.or(Some(folder_path)),
                source_uidvalidity: folder_uidvalidity.and_then(|value| u32::try_from(value).ok()),
            });
    }
    Ok(grouped
        .into_iter()
        .map(
            |((account_id, target_remote_folder_id), refs)| PreparedWorkflowMove {
                account_id,
                source_folder_type: source_folder_type.to_string(),
                target_folder_type: target_folder_type.to_string(),
                refs,
                target_remote_folder_id,
            },
        )
        .collect())
}

/// Remove the still-undoable provider half for exactly this workflow thread.
/// Returns the cancelled plans so a compound transition (for example
/// Done -> Aside) can reuse their original, provider-correct source refs.
pub async fn cancel_scheduled_workflow_moves_in_transaction(
    tx: &mut Transaction<'_, Sqlite>,
    thread_id: &str,
    target_folder_type: &str,
) -> Result<Vec<PreparedWorkflowMove>, String> {
    let rows = sqlx::query_as::<_, (String, String, String)>(
        "SELECT id, account_id, payload FROM sync_queue
         WHERE action_type = ? AND status = 'scheduled'",
    )
    .bind(ACTION_REMOTE_MUTATION)
    .fetch_all(&mut **tx)
    .await
    .map_err(|err| err.to_string())?;
    let mut cancelled = Vec::new();
    for (id, account_id, payload) in rows {
        let Ok(RemoteMutationPayload::WorkflowMove {
            workflow_thread_id,
            source_folder_type,
            target_folder_type: queued_target_type,
            refs,
            target_remote_folder_id,
            ..
        }) = serde_json::from_str::<RemoteMutationPayload>(&payload)
        else {
            continue;
        };
        if workflow_thread_id != thread_id || queued_target_type != target_folder_type {
            continue;
        }
        let removed = sqlx::query("DELETE FROM sync_queue WHERE id = ? AND status = 'scheduled'")
            .bind(id)
            .execute(&mut **tx)
            .await
            .map_err(|err| err.to_string())?
            .rows_affected();
        if removed == 1 {
            cancelled.push(PreparedWorkflowMove {
                account_id,
                source_folder_type,
                target_folder_type: queued_target_type,
                refs,
                target_remote_folder_id,
            });
        }
    }
    Ok(cancelled)
}

pub async fn enqueue_thread_flags(
    pool: &SqlitePool,
    queue: &SyncQueue,
    thread_ids: &[String],
    read: Option<bool>,
    starred: Option<bool>,
) -> Result<(), String> {
    enqueue_thread_flags_with_queue(pool, queue, thread_ids, read, starred)
        .await
        .map(|_| ())
}

pub async fn enqueue_thread_flags_with_queue(
    pool: &SqlitePool,
    queue: &SyncQueue,
    thread_ids: &[String],
    read: Option<bool>,
    starred: Option<bool>,
) -> Result<Vec<String>, String> {
    enqueue_thread_flags_with_stable_ids(pool, queue, thread_ids, read, starred, None).await
}

pub async fn enqueue_thread_flags_with_stable_ids(
    pool: &SqlitePool,
    queue: &SyncQueue,
    thread_ids: &[String],
    read: Option<bool>,
    starred: Option<bool>,
    queue_prefix: Option<&str>,
) -> Result<Vec<String>, String> {
    crate::mail_actions::set_flags(pool, queue, thread_ids, read, starred, queue_prefix).await
}

pub fn stable_queue_id(prefix: &str, purpose: &str, account_id: &str) -> String {
    let account_hash = format!("{:x}", Sha256::digest(account_id.as_bytes()));
    format!("{prefix}:{purpose}:{}", &account_hash[..16])
}

/// Return the durable id for one immutable blocked-sender provider operation.
///
/// The source folder is part of the identity because IMAP remote ids are only
/// meaningful within that folder. A changed source or target therefore gets a
/// fresh queue row and can never inherit lifecycle bits from the old operation.
pub fn stable_blocked_move_id(
    thread_id: &str,
    account_id: &str,
    message_ref: &MessageRef,
    target_remote_folder_id: &str,
) -> String {
    let source_folder = message_ref
        .source_folder
        .as_deref()
        .map(str::trim)
        .unwrap_or_default();
    let remote_id = message_ref.remote_id.trim();
    let generation = message_ref
        .source_uidvalidity
        .map_or_else(|| "-".to_string(), |value| value.to_string());
    let operation_key = format!(
        "{thread_id}\0{account_id}\0{source_folder}\0{remote_id}\0{}\0{generation}",
        target_remote_folder_id.trim()
    );
    let operation_hash = format!("{:x}", Sha256::digest(operation_key.as_bytes()));
    stable_queue_id(
        &format!("blocked-sender-move:{operation_hash}"),
        "move",
        account_id,
    )
}

pub(crate) fn canonicalize_blocked_message_ref(message_ref: MessageRef) -> Option<MessageRef> {
    let remote_id = message_ref.remote_id.trim();
    if remote_id.is_empty() {
        return None;
    }
    let source_folder = message_ref
        .source_folder
        .map(|folder| folder.trim().to_string())
        .filter(|folder| !folder.is_empty());
    Some(MessageRef {
        remote_id: remote_id.to_string(),
        source_folder,
        source_uidvalidity: message_ref.source_uidvalidity,
    })
}

pub async fn enqueue_thread_move_to_folder(
    pool: &SqlitePool,
    queue: &SyncQueue,
    thread_ids: &[String],
    target_folder_id: &str,
) -> Result<(), String> {
    crate::mail_actions::move_threads(
        pool,
        queue,
        thread_ids,
        crate::mail_actions::MoveTarget::Folder(target_folder_id),
    )
    .await
    .map(|_| ())
}

/// Prepare the provider move for one blocked thread without changing the
/// queue. The caller can commit these actions together with the local thread
/// projection.
pub async fn prepare_blocked_thread_moves_to_folder(
    pool: &SqlitePool,
    thread_id: &str,
    target_folder_id: &str,
) -> Result<Vec<QueuedAction>, String> {
    let ids = [thread_id.to_string()];
    let groups = grouped_refs_for_threads(pool, &ids).await?;
    let targets = target_remote_folders(pool, target_folder_id).await?;
    let mut actions = Vec::new();
    for (account_id, refs) in groups {
        let Some(target) = targets.get(&account_id) else {
            continue;
        };
        actions.extend(prepared_blocked_move_actions(
            thread_id, account_id, refs, target,
        )?);
    }
    Ok(actions)
}

/// Prepare the provider move for one blocked thread using a provider-specific
/// folder kind (normally `spam`) without changing the queue.
pub async fn prepare_blocked_thread_moves_to_kind(
    pool: &SqlitePool,
    thread_id: &str,
    folder_type: &str,
) -> Result<Vec<QueuedAction>, String> {
    let ids = [thread_id.to_string()];
    let groups = grouped_refs_for_threads(pool, &ids).await?;
    let mut actions = Vec::new();
    for (account_id, refs) in groups {
        let target = remote_folder_for_kind(pool, &account_id, folder_type).await?;
        actions.extend(prepared_blocked_move_actions(
            thread_id, account_id, refs, &target,
        )?);
    }
    Ok(actions)
}

pub async fn enqueue_thread_move_to_kind(
    pool: &SqlitePool,
    queue: &SyncQueue,
    thread_ids: &[String],
    folder_type: &str,
) -> Result<(), String> {
    enqueue_thread_move_to_kind_tracked(pool, queue, thread_ids, folder_type)
        .await
        .map(|_| ())
}

pub async fn enqueue_thread_move_to_kind_tracked(
    pool: &SqlitePool,
    queue: &SyncQueue,
    thread_ids: &[String],
    folder_type: &str,
) -> Result<Vec<String>, String> {
    let target = match folder_type {
        "archive" => crate::mail_actions::MoveTarget::Archive,
        "trash" => crate::mail_actions::MoveTarget::Trash,
        "spam" => crate::mail_actions::MoveTarget::Spam,
        _ => return Err("unsupported mail move target".into()),
    };
    crate::mail_actions::move_threads(pool, queue, thread_ids, target).await
}

/// Queue the provider half of discarding a draft.
///
/// Deliberately not done inline: the provider can be rate limited for the best
/// part of an hour, and a discard that fails for that reason would leave the
/// draft sitting in the conversation with an error nobody can act on. The queue
/// retries with backoff instead.
pub async fn enqueue_draft_delete(
    pool: &SqlitePool,
    queue: &SyncQueue,
    account_id: &str,
    remote_draft_id: Option<String>,
    remote_message_id: Option<String>,
    source_folder: Option<String>,
) -> Result<(), String> {
    if remote_draft_id.is_none() && remote_message_id.is_none() {
        return Ok(());
    }
    let payload = serde_json::to_value(RemoteMutationPayload::DeleteDraft {
        local_draft_id: None,
        remote_draft_id,
        remote_message_id,
        source_folder,
    })
    .map_err(|err| err.to_string())?;
    queue
        .enqueue(
            pool,
            QueuedAction::new(account_id.to_string(), ACTION_REMOTE_MUTATION, payload),
        )
        .await
}

pub async fn enqueue_thread_delete(
    pool: &SqlitePool,
    queue: &SyncQueue,
    thread_ids: &[String],
) -> Result<(), String> {
    if thread_ids.is_empty() {
        return Ok(());
    }
    let groups = grouped_refs_for_threads(pool, thread_ids).await?;
    for (account_id, refs) in groups {
        if refs.is_empty() {
            continue;
        }
        let payload = serde_json::to_value(RemoteMutationPayload::Delete { refs })
            .map_err(|err| err.to_string())?;
        queue
            .enqueue(
                pool,
                QueuedAction::new(account_id, ACTION_REMOTE_MUTATION, payload),
            )
            .await?;
    }
    Ok(())
}

pub async fn execute_remote_mutation(
    pool: &SqlitePool,
    account_id: &str,
    payload: serde_json::Value,
) -> crate::sync::errors::SyncResult<()> {
    let account = sqlx::query_as::<_, (String, String, String, String)>(
        "SELECT id, provider, email, config FROM accounts WHERE id = ? AND is_active = 1 LIMIT 1",
    )
    .bind(account_id)
    .fetch_one(pool)
    .await
    .map_err(|err| format!("account lookup failed: {err}"))?;
    let config =
        crate::utils::oauth::refresh_account_config(pool, &account.0, &account.1, &account.3)
            .await?;

    let provider = provider_for_account(&account.1, &config, &account.2, None)?;
    let mutation: RemoteMutationPayload =
        serde_json::from_value(payload).map_err(|err| format!("decode remote mutation: {err}"))?;
    match mutation {
        RemoteMutationPayload::Move {
            refs,
            target_remote_folder_id,
        } => {
            let refs = refs_requiring_move(refs, &target_remote_folder_id);
            if refs.is_empty() {
                Ok(())
            } else {
                provider
                    .move_messages(&refs, &target_remote_folder_id)
                    .await
            }
        }
        RemoteMutationPayload::BlockedMove {
            refs,
            target_remote_folder_id,
            provider_move_completed,
            ..
        } => {
            if provider_move_completed {
                return Ok(());
            }
            let refs = refs_requiring_move(refs, &target_remote_folder_id);
            if refs.is_empty() {
                Ok(())
            } else {
                provider
                    .move_messages(&refs, &target_remote_folder_id)
                    .await
            }
        }
        RemoteMutationPayload::WorkflowMove {
            refs,
            target_remote_folder_id,
            provider_move_completed,
            ..
        } => {
            if !provider_move_completed {
                provider
                    .move_messages(&refs, &target_remote_folder_id)
                    .await?;
            }
            Ok(())
        }
        RemoteMutationPayload::UndoTrash { .. } => Err("backend.errors.mailUndoUnavailable".into()),
        RemoteMutationPayload::Delete { refs } => provider.delete_messages(&refs).await,
        RemoteMutationPayload::Flags {
            refs,
            read,
            starred,
        } => {
            provider
                .set_flags(&refs, FlagUpdate { read, starred })
                .await
        }
        RemoteMutationPayload::DeleteDraft {
            remote_draft_id,
            remote_message_id,
            source_folder,
            ..
        } => {
            let resolved = match remote_draft_id {
                Some(id) => Some(id),
                None => match remote_message_id {
                    Some(message_id) => {
                        provider
                            .resolve_draft_id(&message_id, source_folder.as_deref())
                            .await?
                    }
                    None => None,
                },
            };
            match resolved {
                // Nothing on the server to remove — the local row was all of it.
                None => Ok(()),
                Some(draft_id) => provider.delete_draft(&draft_id).await,
            }
        }
        RemoteMutationPayload::CreateFolder { path } => provider.create_folder(&path).await,
        RemoteMutationPayload::RenameFolder { from_path, to_path } => {
            provider.rename_folder(&from_path, &to_path).await
        }
        RemoteMutationPayload::DeleteFolder { path } => provider.delete_folder(&path).await,
        RemoteMutationPayload::AppendSent {
            raw_base64,
            folder_hint,
        } => {
            use base64::Engine as _;
            let raw = base64::engine::general_purpose::STANDARD
                .decode(raw_base64)
                .map_err(|err| {
                    crate::sync::errors::SyncError::Permanent(format!("decode sent copy: {err}"))
                })?;
            provider.append_sent(&raw, folder_hint.as_deref()).await
        }
    }
}

/// Queue the Sent copy of a delivered mail after the direct APPEND failed.
pub async fn enqueue_append_sent(
    pool: &SqlitePool,
    queue: &SyncQueue,
    account_id: &str,
    raw_rfc822: &[u8],
    folder_hint: Option<&str>,
) -> Result<(), String> {
    use base64::Engine as _;
    let payload = serde_json::to_value(RemoteMutationPayload::AppendSent {
        raw_base64: base64::engine::general_purpose::STANDARD.encode(raw_rfc822),
        folder_hint: folder_hint.map(str::to_string),
    })
    .map_err(|err| err.to_string())?;
    queue
        .enqueue(
            pool,
            QueuedAction::new(account_id.to_string(), ACTION_REMOTE_MUTATION, payload),
        )
        .await
}

/// Move the exact local source copies after the provider accepted a workflow
/// move. This closes the interval before the next regular sync in which a
/// reverse transition would otherwise resolve no refs from the target folder.
/// IMAP additionally performs an immediate account sync in the queue worker so
/// the destination UID replaces the now-stale source UID before completion.
pub async fn apply_completed_workflow_move_projection(
    pool: &SqlitePool,
    account_id: &str,
    refs: &[MessageRef],
    target_folder_type: &str,
) -> Result<(), String> {
    let target_folder_id: Option<String> = sqlx::query_scalar(
        "SELECT id FROM folders
         WHERE account_id = ? AND folder_type = ?
         ORDER BY updated_at DESC LIMIT 1",
    )
    .bind(account_id)
    .bind(target_folder_type)
    .fetch_optional(pool)
    .await
    .map_err(|err| err.to_string())?;
    let Some(target_folder_id) = target_folder_id else {
        return Err(format!(
            "local target folder missing for workflow move: {target_folder_type}"
        ));
    };
    apply_completed_move_projection(pool, account_id, refs, &target_folder_id).await
}

pub async fn apply_completed_move_projection(
    pool: &SqlitePool,
    account_id: &str,
    refs: &[MessageRef],
    target_folder_id: &str,
) -> Result<(), String> {
    let now = chrono::Utc::now().to_rfc3339();
    let mut tx = pool.begin().await.map_err(|err| err.to_string())?;
    for message in refs {
        match message.source_folder.as_deref() {
            Some(source) => {
                sqlx::query(
                    "UPDATE messages SET folder_id = ?, pending_folder_id = NULL, updated_at = ?
                     WHERE account_id = ? AND remote_id = ?
                       AND folder_id IN (
                         SELECT id FROM folders
                         WHERE account_id = ?
                           AND (
                             LOWER(TRIM(COALESCE(remote_id, ''))) = LOWER(TRIM(?))
                             OR LOWER(TRIM(full_path)) = LOWER(TRIM(?))
                           )
                       )",
                )
                .bind(target_folder_id)
                .bind(&now)
                .bind(account_id)
                .bind(&message.remote_id)
                .bind(account_id)
                .bind(source)
                .bind(source)
                .execute(&mut *tx)
                .await
                .map_err(|err| err.to_string())?;
            }
            None => {
                sqlx::query(
                    "UPDATE messages SET folder_id = ?, pending_folder_id = NULL, updated_at = ?
                     WHERE account_id = ? AND remote_id = ?",
                )
                .bind(target_folder_id)
                .bind(&now)
                .bind(account_id)
                .bind(&message.remote_id)
                .execute(&mut *tx)
                .await
                .map_err(|err| err.to_string())?;
            }
        }
    }
    tx.commit().await.map_err(|err| err.to_string())?;
    Ok(())
}

/// A workflow move must always have a local projection target before the
/// provider call starts. In particular Gmail's synthetic archive and filtered
/// IMAP archive folders may not have appeared in a regular folder sync yet.
pub async fn ensure_local_workflow_target_folder(
    pool: &SqlitePool,
    account_id: &str,
    folder_type: &str,
    remote_folder_id: &str,
) -> Result<String, String> {
    if let Some(id) = sqlx::query_scalar::<_, String>(
        "SELECT id FROM folders
         WHERE account_id = ? AND COALESCE(remote_id, full_path) = ?
         ORDER BY updated_at DESC LIMIT 1",
    )
    .bind(account_id)
    .bind(remote_folder_id)
    .fetch_optional(pool)
    .await
    .map_err(|err| err.to_string())?
    {
        return Ok(id);
    }

    let id = crate::db::folders::resolve_provider_folder_id(pool, account_id, remote_folder_id)
        .await
        .map_err(|err| err.to_string())?;
    let display_name = match folder_type {
        "archive" => "Archiviert",
        "aside" => crate::workflow::states::ASIDE_FOLDER_NAME,
        "inbox" => "Posteingang",
        _ => remote_folder_id,
    };
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO folders
         (id, account_id, name, full_path, folder_type, remote_id,
          message_count, unread_count, sync_enabled, sort_order, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, 0, 0, 1, 0, ?, ?)
         ON CONFLICT(id) DO NOTHING",
    )
    .bind(&id)
    .bind(account_id)
    .bind(display_name)
    .bind(remote_folder_id)
    .bind(folder_type)
    .bind(remote_folder_id)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|err| err.to_string())?;
    Ok(id)
}

/// Queue a folder operation for the account. The local row is updated by the
/// caller; this only carries the remote half.
pub async fn enqueue_folder_mutation(
    pool: &SqlitePool,
    queue: &SyncQueue,
    account_id: &str,
    payload: RemoteMutationPayload,
) -> Result<(), String> {
    let payload = serde_json::to_value(payload).map_err(|err| err.to_string())?;
    queue
        .enqueue(
            pool,
            QueuedAction::new(account_id.to_string(), ACTION_REMOTE_MUTATION, payload),
        )
        .await
}

fn prepared_blocked_move_actions(
    thread_id: &str,
    account_id: String,
    refs: Vec<MessageRef>,
    target_remote_folder_id: &str,
) -> Result<Vec<QueuedAction>, String> {
    let target_remote_folder_id = target_remote_folder_id.trim();
    if target_remote_folder_id.is_empty() {
        return Ok(Vec::new());
    }
    let refs = refs_requiring_move(refs, target_remote_folder_id);
    refs.into_iter()
        .filter_map(canonicalize_blocked_message_ref)
        .map(|message_ref| {
            let payload = serde_json::to_value(RemoteMutationPayload::BlockedMove {
                blocked_thread_id: thread_id.to_string(),
                refs: vec![message_ref.clone()],
                target_remote_folder_id: target_remote_folder_id.to_string(),
                provider_move_started: false,
                provider_move_completed: false,
                source_uidvalidity_before: None,
            })
            .map_err(|err| err.to_string())?;
            let mut action = QueuedAction::new(account_id.clone(), ACTION_REMOTE_MUTATION, payload);
            action.id = stable_blocked_move_id(
                thread_id,
                &account_id,
                &message_ref,
                target_remote_folder_id,
            );
            Ok(action)
        })
        .collect()
}

/// A conversation can contain local copies from several folders. Moving it to
/// Junk, for example, must not ask IMAP to move an already-junk copy back into
/// the same mailbox. Besides preventing new no-op jobs, this is used at
/// execution time so existing failed queue rows become retryable after an
/// update.
fn refs_requiring_move(refs: Vec<MessageRef>, target_remote_folder_id: &str) -> Vec<MessageRef> {
    let target = target_remote_folder_id.trim();
    refs.into_iter()
        .filter(|message| {
            message
                .source_folder
                .as_deref()
                .is_none_or(|source| source.trim() != target)
        })
        .collect()
}

async fn grouped_refs_for_threads(
    pool: &SqlitePool,
    thread_ids: &[String],
) -> Result<HashMap<String, Vec<MessageRef>>, String> {
    grouped_refs_for_threads_with_kind(pool, thread_ids, None).await
}

async fn grouped_refs_for_threads_in_kind(
    pool: &SqlitePool,
    thread_ids: &[String],
    folder_type: &str,
) -> Result<HashMap<String, Vec<MessageRef>>, String> {
    grouped_refs_for_threads_with_kind(pool, thread_ids, Some(folder_type)).await
}

async fn grouped_refs_for_threads_with_kind(
    pool: &SqlitePool,
    thread_ids: &[String],
    folder_type: Option<&str>,
) -> Result<HashMap<String, Vec<MessageRef>>, String> {
    let mut query = QueryBuilder::<Sqlite>::new(
        // Alias every column: `messages` and `folders` both carry `remote_id`,
        // and `FromRow` matches by name. Without the aliases the row decode
        // fails with "no column found for name: folder_remote_id" — which took
        // archive, trash and every flag update down before they reached the
        // queue, silently, because the callers swallow the error.
        "SELECT m.account_id AS account_id,
                a.provider AS provider,
                a.email AS email,
                a.config AS config,
                m.remote_id AS remote_id,
                f.remote_id AS folder_remote_id,
                f.full_path AS folder_path,
                f.uidvalidity AS folder_uidvalidity
         FROM messages m
         JOIN accounts a ON a.id = m.account_id
         JOIN folders f ON f.id = m.folder_id
         WHERE a.kind = 'email' AND m.remote_id IS NOT NULL AND m.thread_id IN (",
    );
    let mut separated = query.separated(", ");
    for id in thread_ids {
        separated.push_bind(id);
    }
    query.push(")");
    if let Some(folder_type) = folder_type {
        query.push(" AND LOWER(TRIM(f.folder_type)) = LOWER(TRIM(");
        query.push_bind(folder_type);
        query.push("))");
    }

    let rows = query
        .build_query_as::<RemoteMessageRow>()
        .fetch_all(pool)
        .await
        .map_err(|err| err.to_string())?;

    let mut grouped = HashMap::<String, Vec<MessageRef>>::new();
    for row in rows {
        let Some(remote_id) = row.remote_id else {
            continue;
        };
        let _provider_context = (&row.provider, &row.email, &row.config);
        grouped.entry(row.account_id).or_default().push(MessageRef {
            remote_id,
            source_folder: row.folder_remote_id.or(Some(row.folder_path)),
            source_uidvalidity: row
                .folder_uidvalidity
                .and_then(|value| u32::try_from(value).ok()),
        });
    }
    Ok(grouped)
}

async fn target_remote_folders(
    pool: &SqlitePool,
    target_folder_id: &str,
) -> Result<HashMap<String, String>, String> {
    let rows = sqlx::query_as::<_, FolderTargetRow>(
        "SELECT account_id, remote_id, full_path
         FROM folders WHERE id = ?",
    )
    .bind(target_folder_id)
    .fetch_all(pool)
    .await
    .map_err(|err| err.to_string())?;
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            let remote = row.remote_id.or(Some(row.full_path));
            remote.map(|value| (row.account_id, value))
        })
        .collect())
}

async fn remote_folder_for_kind(
    pool: &SqlitePool,
    account_id: &str,
    folder_type: &str,
) -> Result<String, String> {
    if let Some(row) = sqlx::query_as::<_, FolderTargetRow>(
        "SELECT f.account_id, f.remote_id, f.full_path
         FROM folders f
         WHERE f.account_id = ? AND f.folder_type = ?
         LIMIT 1",
    )
    .bind(account_id)
    .bind(folder_type)
    .fetch_optional(pool)
    .await
    .map_err(|err| err.to_string())?
    {
        return Ok(row.remote_id.unwrap_or(row.full_path));
    }

    let provider: String = sqlx::query_scalar("SELECT provider FROM accounts WHERE id = ?")
        .bind(account_id)
        .fetch_one(pool)
        .await
        .map_err(|err| err.to_string())?;
    if folder_type == "aside" {
        return Err("backend.errors.asideFolderMissing".to_string());
    }
    Ok(default_remote_folder(&provider, folder_type).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    #[test]
    fn remote_move_thread_id_extractor_covers_workflow_and_blocked_moves() {
        let workflow = serde_json::to_string(&RemoteMutationPayload::WorkflowMove {
            workflow_thread_id: "workflow-thread".into(),
            source_folder_type: "inbox".into(),
            target_folder_type: "archive".into(),
            refs: Vec::new(),
            target_remote_folder_id: "Archive".into(),
            provider_move_started: false,
            source_uidvalidity_before: None,
            provider_move_completed: false,
            target_remote_ids_before: None,
            target_uidvalidity_before: None,
            projection_completed: false,
        })
        .expect("workflow payload");
        let blocked = serde_json::to_string(&RemoteMutationPayload::BlockedMove {
            blocked_thread_id: "blocked-thread".into(),
            refs: Vec::new(),
            target_remote_folder_id: "Spam".into(),
            provider_move_started: false,
            provider_move_completed: false,
            source_uidvalidity_before: None,
        })
        .expect("blocked payload");

        assert_eq!(
            remote_move_thread_id_from_json(&workflow).as_deref(),
            Some("workflow-thread")
        );
        assert_eq!(
            remote_move_thread_id_from_json(&blocked).as_deref(),
            Some("blocked-thread")
        );
        assert_eq!(remote_move_thread_id_from_json("{not-json"), None);
        assert_eq!(
            remote_move_thread_id_from_json(r#"{"kind":"flags","refs":[]}"#),
            None
        );
    }

    async fn seed_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .unwrap();

        sqlx::query(
            "INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, sync_state, color, is_active, created_at, updated_at, kind)
             VALUES ('acc-1', 'A', 'a@example.com', 'imap', '{}', '', '{}', '#000', 1, '2026-01-01', '2026-01-01', 'email')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO folders (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at)
             VALUES ('fld-inbox', 'acc-1', 'INBOX', 'INBOX', 'inbox', 'INBOX', '2026-01-01', '2026-01-01')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO threads (id, account_id, subject, participants, message_count, unread_count, has_attachments, workflow_state, board_column, priority, last_message_date, created_at, updated_at)
             VALUES ('th-1', 'acc-1', 'Betreff', '[]', 1, 0, 0, 'inbox', 'inbox', 0, '2026-01-01', '2026-01-01', '2026-01-01')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO messages (id, account_id, folder_id, thread_id, from_address, to_addresses, date, remote_id, created_at, updated_at)
             VALUES ('msg-1', 'acc-1', 'fld-inbox', 'th-1', 'a@example.com', '[]', '2026-01-01', '42', '2026-01-01', '2026-01-01')",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    /// MAIL-10. A failed Sent APPEND used to end in a `tracing::warn!`: the mail
    /// was delivered, and on every other device it did not exist. The copy now
    /// gets its own row — and that row must never be a second send.
    #[tokio::test]
    async fn a_failed_sent_copy_is_queued_as_an_append_and_never_as_a_send() {
        use base64::Engine as _;
        let pool = seed_pool().await;
        let queue = SyncQueue::new();
        let raw = b"From: a@example.com\r\nSubject: Status\r\n\r\nBody\r\n";

        enqueue_append_sent(&pool, &queue, "acc-1", raw, Some("INBOX.Gesendet"))
            .await
            .expect("queue the copy");

        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT action_type, payload FROM sync_queue")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(rows.len(), 1, "exactly one row, and not a send");
        assert_eq!(rows[0].0, ACTION_REMOTE_MUTATION);
        let payload: RemoteMutationPayload = serde_json::from_str(&rows[0].1).unwrap();
        let RemoteMutationPayload::AppendSent {
            raw_base64,
            folder_hint,
        } = payload
        else {
            panic!("queued something other than an append: {}", rows[0].1);
        };
        assert_eq!(folder_hint.as_deref(), Some("INBOX.Gesendet"));
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(raw_base64)
                .unwrap(),
            raw,
            "the queued copy has to be the bytes that went out"
        );
    }

    /// Regression: the SELECT joins two tables that both have a `remote_id`
    /// column. Without explicit aliases `FromRow` cannot find
    /// `folder_remote_id`/`folder_path`, and every archive, trash and flag
    /// mutation fails before it ever reaches the queue.
    #[tokio::test]
    async fn collects_message_refs_for_a_thread() {
        let pool = seed_pool().await;

        let grouped = grouped_refs_for_threads(&pool, &["th-1".to_string()])
            .await
            .expect("refs resolve");

        let refs = grouped.get("acc-1").expect("account group");
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].remote_id, "42");
        assert_eq!(refs[0].source_folder.as_deref(), Some("INBOX"));
    }

    #[tokio::test]
    async fn blocked_move_preparation_is_stable_and_thread_identifiable() {
        let pool = seed_pool().await;
        let first = prepare_blocked_thread_moves_to_kind(&pool, "th-1", "spam")
            .await
            .expect("first preparation");
        let second = prepare_blocked_thread_moves_to_kind(&pool, "th-1", "spam")
            .await
            .expect("second preparation");

        assert_eq!(first.len(), 1);
        assert_eq!(second.len(), 1);
        assert_eq!(first[0].id, second[0].id);
        assert_eq!(first[0].account_id, "acc-1");
        assert_eq!(
            first[0]
                .payload
                .get("kind")
                .and_then(|value| value.as_str()),
            Some("blocked_move")
        );
        assert_eq!(
            first[0]
                .payload
                .get("blocked_thread_id")
                .and_then(|value| value.as_str()),
            Some("th-1")
        );
    }

    #[test]
    fn blocked_move_operation_ids_are_per_immutable_remote_ref() {
        let inbox = MessageRef {
            remote_id: "42".into(),
            source_folder: Some("INBOX".into()),
            source_uidvalidity: None,
        };
        let other_message = MessageRef {
            remote_id: "43".into(),
            source_folder: Some("INBOX".into()),
            source_uidvalidity: None,
        };
        let other_folder = MessageRef {
            remote_id: "42".into(),
            source_folder: Some("Archive".into()),
            source_uidvalidity: None,
        };
        let first = stable_blocked_move_id("th-1", "acc-1", &inbox, "Spam");
        assert_ne!(
            first,
            stable_blocked_move_id("th-1", "acc-1", &other_message, "Spam")
        );
        assert_ne!(
            first,
            stable_blocked_move_id("th-1", "acc-1", &other_folder, "Spam")
        );
        assert_eq!(
            first,
            stable_blocked_move_id("th-1", "acc-1", &inbox, " Spam ")
        );
        let actions = prepared_blocked_move_actions(
            "th-1",
            "acc-1".into(),
            vec![inbox, other_message],
            "Spam",
        )
        .expect("prepare immutable actions");
        assert_eq!(actions.len(), 2);
        assert_ne!(actions[0].id, actions[1].id);
        assert_eq!(actions[0].payload["refs"].as_array().unwrap().len(), 1);
        assert_eq!(actions[1].payload["refs"].as_array().unwrap().len(), 1);
        let canonical = prepared_blocked_move_actions(
            "th-1",
            "acc-1".into(),
            vec![MessageRef {
                remote_id: " 42 ".into(),
                source_folder: Some(" INBOX ".into()),
                source_uidvalidity: None,
            }],
            " Spam ",
        )
        .expect("canonical prepare");
        assert_eq!(canonical[0].payload["refs"][0]["remote_id"], "42");
        assert_eq!(canonical[0].payload["refs"][0]["source_folder"], "INBOX");
        assert_eq!(canonical[0].payload["target_remote_folder_id"], "Spam");
        assert_eq!(canonical[0].id, first);

        let generation_one = MessageRef {
            remote_id: "42".into(),
            source_folder: Some("INBOX".into()),
            source_uidvalidity: Some(1),
        };
        let generation_two = MessageRef {
            source_uidvalidity: Some(2),
            ..generation_one.clone()
        };
        assert_ne!(
            stable_blocked_move_id("th-1", "acc-1", &generation_one, "Spam"),
            stable_blocked_move_id("th-1", "acc-1", &generation_two, "Spam")
        );
        let generation_action =
            prepared_blocked_move_actions("th-1", "acc-1".into(), vec![generation_one], "Spam")
                .expect("generation prepare");
        assert_eq!(
            generation_action[0].payload["refs"][0]["source_uidvalidity"],
            1
        );
    }

    #[test]
    fn skips_message_refs_that_are_already_in_the_move_target() {
        let refs = vec![
            MessageRef {
                remote_id: "1".into(),
                source_folder: Some("Junkmail".into()),
                source_uidvalidity: None,
            },
            MessageRef {
                remote_id: "127".into(),
                source_folder: Some("INBOX".into()),
                source_uidvalidity: None,
            },
        ];

        let remaining = refs_requiring_move(refs, "Junkmail");

        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].remote_id, "127");
    }

    #[test]
    fn treats_an_entirely_same_folder_move_as_completed() {
        let refs = vec![MessageRef {
            remote_id: "1".into(),
            source_folder: Some(" Junkmail ".into()),
            source_uidvalidity: None,
        }];

        assert!(refs_requiring_move(refs, "Junkmail").is_empty());
    }
}
