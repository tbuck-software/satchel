//! Durable mail actions shared by command and notification adapters.
//!
//! Queue intent and local presentation commit together. Confirmed provider
//! identities never change before the queued move has been reconciled.

pub mod sweep;
pub mod undo;

use std::collections::HashMap;

use sqlx::{FromRow, Sqlite, SqlitePool, Transaction};

use crate::db::threads::refresh_thread_projection_tx;
use crate::providers::{folder_mapping::provisioned_remote_folder, traits::MessageRef};
use crate::sync::{
    queue::{QueuedAction, SyncQueue},
    remote_mutations::{
        remote_move_thread_id_from_json, PreparedWorkflowMove, RemoteMutationPayload,
        ACTION_REMOTE_MUTATION, QUEUED_MOVE_THREAD_ID,
    },
};

#[derive(Debug, Clone, Default, serde::Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "core/mail_actions/"))]
pub struct TrashReceipt {
    pub undo_token: Option<String>,
    pub undo_thread_ids: Vec<String>,
    pub permanently_deleted: Vec<String>,
    /// Set when a later batch failed: the receipt covers the threads before it.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ipc-types", ts(optional))]
    pub stopped_early: Option<StoppedEarly>,
}

/// A bulk move a later batch stopped. The batches before stay applied, with
/// their queued provider moves and their Undo receipt; nothing of the rest
/// changed.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "core/mail_actions/"))]
pub struct StoppedEarly {
    /// Threads the committed batches moved, in request order.
    pub applied: Vec<String>,
    /// Threads left where they were.
    pub remaining: Vec<String>,
    /// Message key of what stopped the move.
    pub reason_key: String,
}

/// What a bulk move without an Undo receipt did.
#[derive(Debug, Clone, Default)]
pub struct MoveOutcome {
    pub queued: Vec<String>,
    pub stopped_early: Option<StoppedEarly>,
}

#[derive(Debug, Clone, Copy)]
pub enum MoveTarget<'a> {
    Folder(&'a str),
    Archive,
    Trash,
    Spam,
    /// Back to the Inbox from Spam: only messages in a Spam folder move, and
    /// the thread returns to the Inbox state.
    Inbox,
}

/// What Not spam did, and what its Undo hands back.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "core/mail_actions/"))]
pub struct NotSpamReceipt {
    /// Threads the action taught the filter about.
    pub thread_ids: Vec<String>,
    /// Threads that were in Spam and are on their way back to the Inbox.
    pub moved_thread_ids: Vec<String>,
    /// The training receipt; Undo unlearns it.
    pub undo_token: Option<String>,
    /// Receipts of the moves back to the Inbox; Undo replays them.
    pub move_receipts: Vec<String>,
    /// Threads the filter had moved, restored through its own receipt; Undo
    /// files them in Spam again.
    pub restored_thread_ids: Vec<String>,
}

#[derive(FromRow)]
struct Message {
    thread_id: String,
    id: String,
    remote_id: Option<String>,
    source_folder: String,
    source_uidvalidity: Option<i64>,
    folder_type: String,
}

impl Message {
    fn remote_ref(&self) -> Option<MessageRef> {
        self.remote_id
            .as_ref()
            .filter(|id| !id.trim().is_empty())
            .map(|id| MessageRef {
                remote_id: id.clone(),
                source_folder: Some(self.source_folder.clone()),
                source_uidvalidity: self.source_uidvalidity.and_then(|n| u32::try_from(n).ok()),
            })
    }
}

/// A pending move owns the source identity until its destination is confirmed.
/// Call under the same write reservation used to resolve and enqueue new refs.
pub async fn ensure_no_pending_move(
    tx: &mut Transaction<'_, Sqlite>,
    thread_ids: &[String],
) -> Result<(), String> {
    ensure_no_pending_move_except_receipt(tx, thread_ids, None).await
}

async fn ensure_no_pending_move_except_receipt(
    tx: &mut Transaction<'_, Sqlite>,
    thread_ids: &[String],
    ignored_receipt: Option<&str>,
) -> Result<(), String> {
    reconcile_legacy_pop3_moves(tx, Some(thread_ids)).await?;
    let ids = serde_json::to_string(thread_ids).map_err(|e| e.to_string())?;
    let requested: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM mail_action_threads t JOIN mail_action_receipts r ON r.id = t.receipt_id
         WHERE r.state = 'requested' AND (? IS NULL OR r.id != ?)
           AND t.thread_id IN (SELECT value FROM json_each(?)))",
    )
    .bind(ignored_receipt)
    .bind(ignored_receipt)
    .bind(&ids)
    .fetch_one(&mut **tx)
    .await
    .map_err(|e| e.to_string())?;
    if requested {
        return Err("backend.errors.mailMovePending".into());
    }
    // The thread index (migration 0160) narrows the queue to moves of these
    // threads; `+status` keeps the planner off the status index. The typed
    // parse stays the authority on what a payload means.
    let payloads = sqlx::query_scalar::<_, String>(&format!(
        "SELECT payload FROM sync_queue WHERE action_type = 'remote_mutation'
         AND +status IN ('pending', 'scheduled', 'sending', 'failed')
         AND {QUEUED_MOVE_THREAD_ID} IN (SELECT value FROM json_each(?))"
    ))
    .bind(&ids)
    .fetch_all(&mut **tx)
    .await
    .map_err(|e| e.to_string())?;
    if payloads
        .iter()
        .filter_map(|p| remote_move_thread_id_from_json(p))
        .any(|id| thread_ids.contains(&id))
    {
        return Err("backend.errors.mailMovePending".into());
    }
    Ok(())
}

/// Give up a failed remote mutation or draft upload. The provider keeps its
/// state. Already removed local messages are not restored; a move returns its
/// thread to the confirmed folder. A Sent copy is never discarded: its payload may be
/// the only copy of the delivered mail.
pub async fn discard_failed_sync_action(pool: &SqlitePool, id: &str) -> Result<bool, String> {
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|e| e.to_string())?;
    let row: Option<(String, String, String)> = sqlx::query_as(
        "DELETE FROM sync_queue WHERE id = ? AND status = 'failed'
           AND (action_type = 'sync_draft' OR (action_type = 'remote_mutation'
                AND CASE WHEN json_valid(payload) THEN json_extract(payload, '$.kind') END
                    IS NOT 'append_sent'))
         RETURNING account_id, action_type, payload",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;
    let Some((account_id, action_type, payload)) = row else {
        return Ok(false);
    };
    let kind = serde_json::from_str::<serde_json::Value>(&payload)
        .ok()
        .and_then(|value| value.get("kind")?.as_str().map(str::to_owned));
    if action_type == "remote_mutation"
        && matches!(kind.as_deref(), Some("workflow_move" | "blocked_move"))
    {
        let thread_id =
            remote_move_thread_id_from_json(&payload).ok_or("backend.errors.mailMovePending")?;
        release_move_destination(&mut tx, &account_id, &thread_id).await?;
        crate::db::threads::refresh_thread_projections_tx(&mut tx, &[thread_id])
            .await
            .map_err(|e| e.to_string())?;
    } else if action_type == "remote_mutation" && kind.as_deref() == Some("undo_trash") {
        if let Ok(RemoteMutationPayload::UndoTrash { receipt_id }) = serde_json::from_str(&payload)
        {
            // Abandoning the reversal must also release its durable thread guard.
            sqlx::query("UPDATE mail_action_receipts SET state = 'conflicted' WHERE id = ? AND state = 'requested' AND EXISTS (SELECT 1 FROM mail_action_threads WHERE receipt_id = mail_action_receipts.id AND account_id = ?)")
                .bind(receipt_id)
                .bind(&account_id)
                .execute(&mut *tx)
                .await
                .map_err(|e| e.to_string())?;
        }
    }
    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(true)
}

/// Return a thread whose move will not run to its confirmed provider folder.
async fn release_move_destination(
    tx: &mut Transaction<'_, Sqlite>,
    account_id: &str,
    thread_id: &str,
) -> Result<(), String> {
    sqlx::query(
        "UPDATE messages SET pending_folder_id = NULL WHERE thread_id = ? AND account_id = ?",
    )
    .bind(thread_id)
    .bind(account_id)
    .execute(&mut **tx)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Older versions queued POP3 moves that the provider cannot perform. Finish
/// their local destination and discard the impossible operation atomically.
pub async fn reconcile_all_legacy_pop3_moves(pool: &SqlitePool) -> Result<usize, String> {
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|e| e.to_string())?;
    let count = reconcile_legacy_pop3_moves(&mut tx, None).await?;
    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(count)
}

async fn reconcile_legacy_pop3_moves(
    tx: &mut Transaction<'_, Sqlite>,
    thread_ids: Option<&[String]>,
) -> Result<usize, String> {
    // Narrowed in SQLite: this runs before every move and flag change. By
    // thread through the move index when the caller names threads, else by
    // account; a scan by status would visit the whole queue each time.
    let pop3 = "q.account_id IN (SELECT id FROM accounts WHERE LOWER(provider) = 'pop3')
           AND q.action_type = 'remote_mutation'
           AND +q.status IN ('pending', 'scheduled', 'sending', 'failed')";
    let actions = match thread_ids {
        Some(ids) => {
            sqlx::query_as::<_, (String, String)>(&format!(
                "SELECT q.id, q.payload FROM sync_queue q WHERE {pop3}
                   AND {QUEUED_MOVE_THREAD_ID} IN (SELECT value FROM json_each(?))"
            ))
            .bind(serde_json::to_string(ids).map_err(|e| e.to_string())?)
            .fetch_all(&mut **tx)
            .await
        }
        None => {
            sqlx::query_as::<_, (String, String)>(&format!(
                "SELECT q.id, q.payload FROM sync_queue q WHERE {pop3}"
            ))
            .fetch_all(&mut **tx)
            .await
        }
    }
    .map_err(|e| e.to_string())?;
    let now = chrono::Utc::now().to_rfc3339();
    let mut repaired = 0;
    for (action_id, payload) in actions {
        let Some(thread_id) = remote_move_thread_id_from_json(&payload) else {
            continue;
        };
        if let Some(thread_ids) = thread_ids {
            if !thread_ids.contains(&thread_id) {
                continue;
            }
        }
        sqlx::query("UPDATE messages SET folder_id = pending_folder_id, pending_folder_id = NULL, updated_at = ? WHERE thread_id = ? AND pending_folder_id IS NOT NULL")
            .bind(&now).bind(&thread_id).execute(&mut **tx).await.map_err(|e| e.to_string())?;
        sqlx::query("DELETE FROM sync_queue WHERE id = ?")
            .bind(&action_id)
            .execute(&mut **tx)
            .await
            .map_err(|e| e.to_string())?;
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM threads WHERE id = ?)")
            .bind(&thread_id)
            .fetch_one(&mut **tx)
            .await
            .map_err(|e| e.to_string())?;
        if exists {
            refresh_thread_projection_tx(tx, &thread_id, &now)
                .await
                .map_err(|e| e.to_string())?;
        }
        repaired += 1;
    }
    Ok(repaired)
}

async fn messages(
    tx: &mut Transaction<'_, Sqlite>,
    thread_id: &str,
) -> Result<Vec<Message>, String> {
    sqlx::query_as(
        "SELECT m.thread_id, m.id, CASE WHEN a.kind = 'email' THEN m.remote_id ELSE NULL END AS remote_id, COALESCE(f.remote_id, f.full_path) AS source_folder,
        f.uidvalidity AS source_uidvalidity, f.folder_type
        FROM messages m JOIN folders f ON f.id = m.folder_id JOIN accounts a ON a.id = m.account_id WHERE m.thread_id = ?",
    )
    .bind(thread_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(|e| e.to_string())
}

async fn enqueue(
    tx: &mut Transaction<'_, Sqlite>,
    queue: &SyncQueue,
    account: &str,
    payload: RemoteMutationPayload,
    prefix: Option<&str>,
) -> Result<String, String> {
    let mut action = QueuedAction::new(
        account.into(),
        ACTION_REMOTE_MUTATION,
        serde_json::to_value(payload).map_err(|e| e.to_string())?,
    );
    if let Some(prefix) = prefix {
        action.id = crate::sync::remote_mutations::stable_queue_id(prefix, "flags", account);
    }
    queue.enqueue_in_transaction(tx, &action).await?;
    Ok(action.id)
}

/// Local flags and their durable provider intent either both commit or neither.
pub async fn set_flags(
    pool: &SqlitePool,
    queue: &SyncQueue,
    ids: &[String],
    read: Option<bool>,
    starred: Option<bool>,
    prefix: Option<&str>,
) -> Result<Vec<String>, String> {
    if ids.is_empty() || (read.is_none() && starred.is_none()) {
        return Ok(Vec::new());
    }
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|e| e.to_string())?;
    let queued = set_flags_in_transaction(&mut tx, queue, ids, read, starred, prefix).await?;
    tx.commit().await.map_err(|e| e.to_string())?;
    queue.wake_worker();
    Ok(queued)
}

/// Compose flags with another mail action under the same write reservation.
/// Queue flags before a move, while their folder-scoped UIDs still name the source.
pub async fn set_flags_in_transaction(
    tx: &mut Transaction<'_, Sqlite>,
    queue: &SyncQueue,
    ids: &[String],
    read: Option<bool>,
    starred: Option<bool>,
    prefix: Option<&str>,
) -> Result<Vec<String>, String> {
    ensure_no_pending_move(tx, ids).await?;
    let now = chrono::Utc::now().to_rfc3339();
    let mut groups = HashMap::<String, Vec<MessageRef>>::new();
    let mut seen = std::collections::HashSet::new();
    for id in ids.iter().filter(|id| seen.insert(id.as_str())) {
        let (account, provider): (String, String) = sqlx::query_as(
            "SELECT t.account_id, a.provider FROM threads t JOIN accounts a ON a.id = t.account_id WHERE t.id = ?",
        )
            .bind(id)
            .fetch_one(&mut **tx)
            .await
            .map_err(|e| e.to_string())?;
        let rows = messages(tx, id).await?;
        if !provider.eq_ignore_ascii_case("pop3") {
            groups
                .entry(account)
                .or_default()
                .extend(rows.iter().filter_map(Message::remote_ref));
        }
        sqlx::query("UPDATE messages SET is_read = COALESCE(?, is_read), is_starred = COALESCE(?, is_starred), updated_at = ? WHERE thread_id = ?")
            .bind(read).bind(starred).bind(&now).bind(id).execute(&mut **tx).await.map_err(|e| e.to_string())?;
        if rows.is_empty() {
            sqlx::query("UPDATE threads SET is_starred = COALESCE(?, is_starred), unread_count = CASE WHEN ? = 1 THEN 0 WHEN ? = 0 THEN MAX(1, unread_count) ELSE unread_count END, updated_at = ? WHERE id = ?")
                .bind(starred).bind(read).bind(read).bind(&now).bind(id).execute(&mut **tx).await.map_err(|e| e.to_string())?;
        } else {
            refresh_thread_projection_tx(tx, id, &now)
                .await
                .map_err(|e| e.to_string())?;
        }
    }
    let mut queued = Vec::new();
    for (account, refs) in groups {
        if !refs.is_empty() {
            queued.push(
                enqueue(
                    tx,
                    queue,
                    &account,
                    RemoteMutationPayload::Flags {
                        refs,
                        read,
                        starred,
                        superseded: Vec::new(),
                    },
                    prefix,
                )
                .await?,
            );
        }
    }
    Ok(queued)
}

pub async fn trash_threads(
    pool: &SqlitePool,
    queue: &SyncQueue,
    ids: &[String],
) -> Result<TrashReceipt, String> {
    move_threads_with_receipt(pool, queue, ids, MoveTarget::Trash, true)
        .await
        .map(|(_, receipt)| receipt)
}

/// Spam reuses the trash receipt, so the same Undo moves every message back to
/// the folder it came from and restores the workflow state. The filter learns
/// the messages as spam under that receipt; the Undo unlearns them.
pub async fn mark_threads_spam(
    pool: &SqlitePool,
    queue: &SyncQueue,
    ids: &[String],
) -> Result<TrashReceipt, String> {
    let receipt = move_spam_with_receipt(pool, queue, ids).await?;
    // A move a later batch stopped teaches only what it moved.
    let moved = receipt
        .stopped_early
        .as_ref()
        .map_or(ids, |stopped| stopped.applied.as_slice());
    if let Err(error) = crate::spam::train_threads(
        pool,
        moved,
        crate::spam::Label::Spam,
        crate::spam::Signal::Explicit,
        receipt.undo_token.as_deref(),
    )
    .await
    {
        // The move is the user's action and stands; a model that missed one
        // example is not worth failing it for.
        tracing::warn!(%error, "spam filter training failed after Mark as spam");
    }
    Ok(receipt)
}

/// Move threads to Spam under an undo receipt, without training. The filter's
/// own automatic move uses this: its verdict is not a new example.
pub async fn move_spam_with_receipt(
    pool: &SqlitePool,
    queue: &SyncQueue,
    ids: &[String],
) -> Result<TrashReceipt, String> {
    move_threads_with_receipt(pool, queue, ids, MoveTarget::Spam, true)
        .await
        .map(|(_, receipt)| receipt)
}

/// Not spam: teach the filter these threads are wanted, and bring whatever
/// of them sits in Spam back to the Inbox.
///
/// Every receipt of the filter's own moves is replayed through the ordinary
/// Undo — exact folder and workflow state included, however many times the
/// filter moved parts of the thread. Whatever is still in Spam after that
/// (moved by the user or by the provider) moves back to the Inbox under a
/// receipt of its own, so the Not spam itself can be undone.
pub async fn mark_threads_not_spam(
    pool: &SqlitePool,
    queue: &SyncQueue,
    ids: &[String],
) -> Result<NotSpamReceipt, String> {
    let mut seen = std::collections::HashSet::new();
    let ids: Vec<String> = ids
        .iter()
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty() && seen.insert(id.clone()))
        .collect();
    let mut outcome = NotSpamReceipt::default();
    if ids.is_empty() {
        return Ok(outcome);
    }
    for id in &ids {
        let receipts: Vec<String> = sqlx::query_scalar(
            "SELECT DISTINCT v.receipt_id FROM spam_verdicts v JOIN messages m ON m.id = v.message_id
             WHERE m.thread_id = ? AND v.receipt_id IS NOT NULL ORDER BY v.receipt_id",
        )
        .bind(id)
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;
        let mut restored = false;
        for receipt in receipts {
            restored |= undo::undo_trash(pool, queue, &receipt).await.is_ok();
        }
        sqlx::query(
            "UPDATE spam_verdicts SET receipt_id = NULL, moved_at = NULL
             WHERE message_id IN (SELECT id FROM messages WHERE thread_id = ?)",
        )
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
        let still_in_spam = !crate::spam::spam_folder_messages(pool, std::slice::from_ref(id))
            .await
            .map_err(|e| e.to_string())?
            .is_empty();
        if still_in_spam {
            match move_threads_with_receipt(
                pool,
                queue,
                std::slice::from_ref(id),
                MoveTarget::Inbox,
                true,
            )
            .await
            {
                Ok((_, receipt)) => {
                    outcome.move_receipts.extend(receipt.undo_token);
                    outcome.moved_thread_ids.push(id.clone());
                }
                // A replayed receipt whose provider move is still running
                // owns the thread; its reversal is queued and finishes it.
                Err(error) if restored && error == "backend.errors.mailMovePending" => {
                    outcome.moved_thread_ids.push(id.clone());
                }
                Err(error) => return Err(error),
            }
        } else if restored {
            outcome.moved_thread_ids.push(id.clone());
        }
        if restored {
            outcome.restored_thread_ids.push(id.clone());
        }
    }
    let token = format!("not-spam:{}", uuid::Uuid::new_v4());
    crate::spam::train_threads(
        pool,
        &ids,
        crate::spam::Label::Ham,
        crate::spam::Signal::Explicit,
        Some(&token),
    )
    .await
    .map_err(|e| e.to_string())?;
    outcome.thread_ids = ids;
    outcome.undo_token = Some(token);
    Ok(outcome)
}

/// Undo a Not spam: return the threads it moved to Spam, file the ones the
/// filter had moved there again, and only then unlearn it. A replay that
/// fails stops before the filter forgets anything, so a retry finds the
/// lesson and the receipts where they were.
///
/// A refile a later batch stops is such a failure: the threads it filed stay
/// in Spam, the reason key is the error, and the lesson stays until a retry
/// files the rest. There is no partial outcome to report — this Undo is
/// itself the user's way back, and the retry is the same call.
pub async fn undo_not_spam(
    pool: &SqlitePool,
    queue: &SyncQueue,
    receipt: &NotSpamReceipt,
) -> Result<(), String> {
    for move_receipt in &receipt.move_receipts {
        undo::undo_trash(pool, queue, move_receipt).await?;
    }
    let mut seen = std::collections::HashSet::new();
    let refile: Vec<String> = receipt
        .restored_thread_ids
        .iter()
        .filter(|id| seen.insert(id.as_str()))
        .cloned()
        .collect();
    let mut moved_stopped = None;
    if !refile.is_empty() {
        let moved = move_spam_with_receipt(pool, queue, &refile).await?;
        let refiled = moved
            .stopped_early
            .as_ref()
            .map_or(refile.as_slice(), |stopped| stopped.applied.as_slice());
        for id in refiled {
            sqlx::query(
                "UPDATE spam_verdicts SET receipt_id = ?, moved_at = ?
                 WHERE verdict = 'spam' AND message_id IN (SELECT id FROM messages WHERE thread_id = ?)",
            )
            .bind(moved.undo_token.as_deref())
            .bind(chrono::Utc::now().to_rfc3339())
            .bind(id)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
        }
        moved_stopped = moved.stopped_early;
    }
    if let Some(stopped) = moved_stopped {
        // The filter keeps its lesson until every thread is back in Spam.
        return Err(stopped.reason_key);
    }
    if let Some(token) = receipt.undo_token.as_deref() {
        crate::spam::untrain_receipt(pool, token)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Move threads, all or nothing as far as the caller can tell: a move a
/// later batch stopped is an error carrying its reason key. The batches
/// before it stay applied; [`move_threads_reporting`] says which they were.
pub async fn move_threads(
    pool: &SqlitePool,
    queue: &SyncQueue,
    ids: &[String],
    target: MoveTarget<'_>,
) -> Result<Vec<String>, String> {
    let outcome = move_threads_reporting(pool, queue, ids, target).await?;
    match outcome.stopped_early {
        Some(stopped) => Err(stopped.reason_key),
        None => Ok(outcome.queued),
    }
}

/// Move threads and say what a later batch left undone, for callers that
/// report to a person or retry the rest.
pub async fn move_threads_reporting(
    pool: &SqlitePool,
    queue: &SyncQueue,
    ids: &[String],
    target: MoveTarget<'_>,
) -> Result<MoveOutcome, String> {
    move_threads_with_receipt(pool, queue, ids, target, false)
        .await
        .map(|(queued, receipt)| MoveOutcome {
            queued,
            stopped_early: receipt.stopped_early,
        })
}

/// Threads per write transaction of a bulk move. One batch of this size
/// holds the write lock for well under a second on a loaded machine.
const MOVE_BATCH_THREADS: usize = 200;

/// Pause between batches. Waiting writers (sync, the UI) are woken by
/// SQLite's busy handler, which polls; a bare yield would take the lock
/// again before any of them looks.
const MOVE_BATCH_PAUSE: std::time::Duration = std::time::Duration::from_millis(5);

/// The key shown when a later batch failed for a reason without one, such
/// as a busy database.
const MOVE_INTERRUPTED: &str = "backend.errors.mailMoveInterrupted";

#[cfg(any(test, feature = "test-hooks"))]
thread_local! {
    static MOVE_BATCH_OVERRIDE: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

/// Move in batches of `threads` on this thread, so a small fixture crosses
/// batch boundaries. `None` restores the default.
#[cfg(any(test, feature = "test-hooks"))]
pub fn set_move_batch_threads_for_tests(threads: Option<usize>) {
    MOVE_BATCH_OVERRIDE.with(|batch| batch.set(threads.map(|n| n.max(1))));
}

#[cfg(any(test, feature = "test-hooks"))]
fn move_batch_threads() -> usize {
    MOVE_BATCH_OVERRIDE
        .with(std::cell::Cell::get)
        .unwrap_or(MOVE_BATCH_THREADS)
}

#[cfg(not(any(test, feature = "test-hooks")))]
fn move_batch_threads() -> usize {
    MOVE_BATCH_THREADS
}

/// Queue the provider moves and project them locally, in batches of
/// [`MOVE_BATCH_THREADS`] threads, one write transaction each.
///
/// The whole request is checked in the first batch — a pending move, an
/// unknown thread or a folder of another account refuses it before anything
/// changes, and any failure of the first batch is an error. A later batch
/// can still fail: a busy database, a target folder sync removed, a move
/// another action queued in between. The batches before stay applied, and
/// the receipt says so in `stopped_early`, covering exactly those threads.
async fn move_threads_with_receipt(
    pool: &SqlitePool,
    queue: &SyncQueue,
    ids: &[String],
    target: MoveTarget<'_>,
    record_undo: bool,
) -> Result<(Vec<String>, TrashReceipt), String> {
    if ids.is_empty() {
        return Ok((Vec::new(), TrashReceipt::default()));
    }
    let mut seen = std::collections::HashSet::new();
    let unique: Vec<String> = ids
        .iter()
        .filter(|id| seen.insert(id.as_str()))
        .cloned()
        .collect();
    let mut run = BulkMove::new(target, record_undo, MoveScope::Thread);
    let batches: Vec<&[String]> = unique.chunks(move_batch_threads()).collect();
    let mut applied = 0;
    for (position, batch) in batches.iter().enumerate() {
        let last = position + 1 == batches.len();
        // A failed batch rolls back; so does what it noted for the caller.
        let noted = (
            run.queued.len(),
            run.receipt.undo_thread_ids.len(),
            run.receipt.permanently_deleted.len(),
        );
        match run.batch(pool, queue, &unique, batch, position, last).await {
            Ok(()) => applied += batch.len(),
            Err(error) if position == 0 => return Err(error),
            Err(error) => {
                run.queued.truncate(noted.0);
                run.receipt.undo_thread_ids.truncate(noted.1);
                run.receipt.permanently_deleted.truncate(noted.2);
                tracing::warn!(
                    applied,
                    remaining = unique.len() - applied,
                    %error,
                    "a bulk move stopped after its first batches"
                );
                let reason_key = if error.starts_with("backend.errors.") {
                    error
                } else {
                    MOVE_INTERRUPTED.to_string()
                };
                run.receipt.stopped_early = Some(StoppedEarly {
                    applied: unique[..applied].to_vec(),
                    remaining: unique[applied..].to_vec(),
                    reason_key,
                });
                break;
            }
        }
        queue.wake_worker();
        if !last {
            tokio::time::sleep(MOVE_BATCH_PAUSE).await;
        }
    }
    if record_undo && !run.receipt.undo_thread_ids.is_empty() {
        run.receipt.undo_token = Some(run.receipt_id);
    } else if record_undo && run.receipt.stopped_early.is_some() {
        // The batch that would have dropped an empty receipt never ran.
        if let Err(error) = sqlx::query("DELETE FROM mail_action_receipts WHERE id = ?")
            .bind(&run.receipt_id)
            .execute(pool)
            .await
        {
            tracing::warn!(%error, "an empty Undo receipt of a stopped move stays behind");
        }
    }
    Ok((run.queued, run.receipt))
}

/// Which messages of a thread a move takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MoveScope {
    /// Every message, as a move the person asks for.
    Thread,
    /// Only those in the Inbox: what a rule run on new mail saw. The
    /// person's sent replies and filed messages stay where they are.
    Inbox,
}

/// A rule's move of a thread's Inbox messages into one of its account's
/// folders, inside the rule run's transaction. Queued like any move: the
/// remote move goes in first, the message keeps its folder until the
/// provider confirms (`pending_folder_id`). Flags must be queued before.
pub async fn move_inbox_messages_in_transaction(
    tx: &mut Transaction<'_, Sqlite>,
    queue: &SyncQueue,
    thread_id: &str,
    folder_id: &str,
) -> Result<Vec<String>, String> {
    let ids = [thread_id.to_string()];
    let mut run = BulkMove::new(MoveTarget::Folder(folder_id), false, MoveScope::Inbox);
    ensure_no_pending_move(tx, &ids).await?;
    run.check(tx, &ids).await?;
    run.open_receipt(tx, &ids).await?;
    run.apply(tx, queue, &ids).await?;
    Ok(run.queued)
}

/// One bulk move across its batches.
struct BulkMove<'a> {
    target: MoveTarget<'a>,
    record_undo: bool,
    scope: MoveScope,
    /// One timestamp for the whole move: Undo compares it with the thread's
    /// `workflow_changed_at`.
    now: String,
    receipt_id: String,
    /// The resolved target folder per account: id, remote id, kind.
    targets: HashMap<String, (String, String, String)>,
    queued: Vec<String>,
    receipt: TrashReceipt,
}

impl<'a> BulkMove<'a> {
    fn new(target: MoveTarget<'a>, record_undo: bool, scope: MoveScope) -> Self {
        Self {
            target,
            record_undo,
            scope,
            now: chrono::Utc::now().to_rfc3339(),
            receipt_id: uuid::Uuid::new_v4().to_string(),
            targets: HashMap::new(),
            queued: Vec::new(),
            receipt: TrashReceipt::default(),
        }
    }

    /// One write transaction: the whole request is checked in the first.
    async fn batch(
        &mut self,
        pool: &SqlitePool,
        queue: &SyncQueue,
        all: &[String],
        batch: &[String],
        position: usize,
        last: bool,
    ) -> Result<(), String> {
        let mut tx = pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(|e| e.to_string())?;
        if position == 0 {
            ensure_no_pending_move(&mut tx, all).await?;
            self.check(&mut tx, all).await?;
            self.open_receipt(&mut tx, all).await?;
        } else {
            // Another action may have queued a move in between.
            ensure_no_pending_move(&mut tx, batch).await?;
        }
        self.queue_markers(&mut tx, queue, batch).await?;
        self.apply(&mut tx, queue, batch).await?;
        if last && self.record_undo && self.receipt.undo_thread_ids.is_empty() {
            sqlx::query("DELETE FROM mail_action_receipts WHERE id = ?")
                .bind(&self.receipt_id)
                .execute(&mut *tx)
                .await
                .map_err(|e| e.to_string())?;
        }
        tx.commit().await.map_err(|e| e.to_string())
    }

    /// Refuse the request before it changes anything, in the order one
    /// thread after the other would have found the problem.
    async fn check(
        &mut self,
        tx: &mut Transaction<'_, Sqlite>,
        ids: &[String],
    ) -> Result<(), String> {
        let threads = thread_accounts(tx, ids).await?;
        for id in ids {
            let Some((account, provider)) = threads.get(id) else {
                return Err(sqlx::Error::RowNotFound.to_string());
            };
            if matches!(self.target, MoveTarget::Folder(_)) {
                self.target_for(tx, account, provider).await?;
            }
        }
        Ok(())
    }

    /// Forget older Undo receipts of these threads and open this move's.
    async fn open_receipt(
        &self,
        tx: &mut Transaction<'_, Sqlite>,
        ids: &[String],
    ) -> Result<(), String> {
        sqlx::query(
            "DELETE FROM mail_action_receipts WHERE state = 'available' AND id IN (
                 SELECT receipt_id FROM mail_action_threads
                 WHERE thread_id IN (SELECT value FROM json_each(?)))",
        )
        .bind(json_ids(ids)?)
        .execute(&mut **tx)
        .await
        .map_err(|e| e.to_string())?;
        if self.record_undo {
            // Keep requested reversals until processed. Available receipts only serve short-lived UI actions.
            sqlx::query(
                "DELETE FROM mail_action_receipts WHERE state != 'requested' AND created_at < ?",
            )
            .bind((chrono::Utc::now() - chrono::Duration::days(1)).to_rfc3339())
            .execute(&mut **tx)
            .await
            .map_err(|e| e.to_string())?;
            sqlx::query("INSERT INTO mail_action_receipts (id, created_at) VALUES (?, ?)")
                .bind(&self.receipt_id)
                .bind(&self.now)
                .execute(&mut **tx)
                .await
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// Rules and the notification's Done archive here; the server learns
    /// Done as from a workflow transition. Queued per batch, ahead of the
    /// batch's moves, so MOVE carries the keyword along and a thread a later
    /// batch never reached gets none. Later batches fill the marker row an
    /// earlier one left waiting. A thread already outside the Inbox gets
    /// Done too, which it is.
    async fn queue_markers(
        &mut self,
        tx: &mut Transaction<'_, Sqlite>,
        queue: &SyncQueue,
        ids: &[String],
    ) -> Result<(), String> {
        if matches!(self.target, MoveTarget::Archive) {
            let markers = crate::sync::workflow_markers::enqueue_workflow_markers_in_transaction(
                tx,
                queue,
                ids,
                &crate::workflow::remote_marker::WorkflowMarker::Done,
                None,
            )
            .await?;
            for id in markers {
                // A row filled by an earlier batch is reported once.
                if !self.queued.contains(&id) {
                    self.queued.push(id);
                }
            }
        }
        Ok(())
    }

    async fn target_for(
        &mut self,
        tx: &mut Transaction<'_, Sqlite>,
        account: &str,
        provider: &str,
    ) -> Result<(String, String, String), String> {
        if let Some(target) = self.targets.get(account) {
            return Ok(target.clone());
        }
        let resolved = resolve_target(tx, account, provider, &self.target, &self.now).await?;
        self.targets.insert(account.to_string(), resolved.clone());
        Ok(resolved)
    }

    /// Move one batch: provider intent first, while the folder-scoped UIDs
    /// still name the source, then the local projection. A constant number
    /// of statements per batch, whatever its size.
    async fn apply(
        &mut self,
        tx: &mut Transaction<'_, Sqlite>,
        queue: &SyncQueue,
        ids: &[String],
    ) -> Result<(), String> {
        let threads = thread_accounts(tx, ids).await?;
        let mut messages = thread_messages(tx, ids).await?;
        let mut actions = Vec::new();
        let mut receipt_actions = Vec::new();
        let mut removed = Vec::new();
        let mut snapshots = Vec::new();
        let mut read = Vec::new();
        let mut originals = Vec::new();
        let mut pending: Vec<(String, Vec<String>)> = Vec::new();
        let mut local: Vec<(String, Vec<String>)> = Vec::new();
        let mut to_inbox = Vec::new();
        let mut to_done = Vec::new();
        let mut refresh = Vec::new();
        for id in ids {
            // A thread that vanished since the first batch has nothing left to move.
            let Some((account, provider)) = threads.get(id) else {
                continue;
            };
            refresh.push(id.clone());
            let rows = messages.remove(id).unwrap_or_default();
            let pop3 = provider.eq_ignore_ascii_case("pop3");
            let permanent = matches!(self.target, MoveTarget::Trash)
                && !rows.is_empty()
                && rows.iter().all(|m| m.folder_type == "trash");
            if permanent {
                self.receipt.permanently_deleted.push(id.clone());
                let refs = rows
                    .iter()
                    .filter_map(Message::remote_ref)
                    .collect::<Vec<_>>();
                if !refs.is_empty() {
                    let action = remote_mutation(account, RemoteMutationPayload::Delete { refs })?;
                    self.queued.push(action.id.clone());
                    actions.push(action);
                }
                removed.push(id.clone());
                continue;
            }
            let queue_start = actions.len();
            if self.record_undo {
                snapshots.push(id.clone());
                self.receipt.undo_thread_ids.push(id.clone());
            }
            let (folder_id, remote, kind) = self.target_for(tx, account, provider).await?;
            let rows = rows
                .into_iter()
                .filter(|m| m.source_folder != remote)
                .filter(|m| self.scope == MoveScope::Thread || m.folder_type == "inbox")
                .filter(|m| !matches!(self.target, MoveTarget::Inbox) || m.folder_type == "spam")
                .filter(|m| {
                    !matches!(self.target, MoveTarget::Archive)
                        || !matches!(
                            m.folder_type.as_str(),
                            "sent" | "drafts" | "trash" | "archive"
                        )
                })
                .collect::<Vec<_>>();
            let refs = rows
                .iter()
                .filter_map(Message::remote_ref)
                .collect::<Vec<_>>();
            if matches!(self.target, MoveTarget::Archive | MoveTarget::Spam) {
                if !pop3 && !refs.is_empty() {
                    actions.push(remote_mutation(
                        account,
                        RemoteMutationPayload::Flags {
                            refs: refs.clone(),
                            read: Some(true),
                            starred: None,
                            superseded: Vec::new(),
                        },
                    )?);
                }
                read.extend(rows.iter().map(|m| m.id.clone()));
            }
            if !pop3 && !refs.is_empty() {
                // Reuse the existing crash recovery and exact IMAP target reconciliation.
                let mut action = PreparedWorkflowMove {
                    account_id: account.clone(),
                    source_folder_type: "folder".into(),
                    target_folder_type: kind,
                    refs,
                    target_remote_folder_id: remote,
                }
                .into_action(id, chrono::Utc::now())?;
                action.status = "pending".into();
                action.scheduled_for = None;
                actions.push(action);
            }
            for action in &actions[queue_start..] {
                self.queued.push(action.id.clone());
                if self.record_undo {
                    receipt_actions.push(action.id.clone());
                }
            }
            let moved_any = !rows.is_empty();
            for message in rows {
                if self.record_undo {
                    originals.push((message.id.clone(), folder_id.clone()));
                }
                let group = if !pop3 && message.remote_ref().is_some() {
                    &mut pending
                } else {
                    &mut local
                };
                match group.iter_mut().find(|(folder, _)| *folder == folder_id) {
                    Some((_, ids)) => ids.push(message.id),
                    None => group.push((folder_id.clone(), vec![message.id])),
                }
            }
            if matches!(self.target, MoveTarget::Inbox) {
                if moved_any {
                    to_inbox.push(id.clone());
                }
            } else if !matches!(self.target, MoveTarget::Folder(_)) {
                to_done.push(id.clone());
            }
        }

        let now = self.now.as_str();
        if !snapshots.is_empty() {
            sqlx::query(
                "INSERT INTO mail_action_threads (receipt_id, thread_id, account_id, workflow_state, board_column, snooze_until, action_at)
                 SELECT ?, t.id, t.account_id, t.workflow_state, t.board_column, t.snooze_until, ?
                 FROM json_each(?) j JOIN threads t ON t.id = j.value ORDER BY j.key",
            )
            .bind(&self.receipt_id)
            .bind(now)
            .bind(json_ids(&snapshots)?)
            .execute(&mut **tx)
            .await
            .map_err(|e| e.to_string())?;
        }
        queue.enqueue_batch_in_transaction(tx, &actions).await?;
        if !removed.is_empty() {
            let removed = json_ids(&removed)?;
            sqlx::query(
                "DELETE FROM attachments WHERE message_id IN (
                     SELECT id FROM messages WHERE thread_id IN (SELECT value FROM json_each(?)))",
            )
            .bind(&removed)
            .execute(&mut **tx)
            .await
            .map_err(|e| e.to_string())?;
            sqlx::query("DELETE FROM messages WHERE thread_id IN (SELECT value FROM json_each(?))")
                .bind(&removed)
                .execute(&mut **tx)
                .await
                .map_err(|e| e.to_string())?;
        }
        if !read.is_empty() {
            sqlx::query(
                "UPDATE messages SET is_read = 1 WHERE id IN (SELECT value FROM json_each(?))",
            )
            .bind(json_ids(&read)?)
            .execute(&mut **tx)
            .await
            .map_err(|e| e.to_string())?;
        }
        if !originals.is_empty() {
            sqlx::query(
                "INSERT INTO mail_action_messages (receipt_id, message_id, original_folder_id, target_folder_id)
                 SELECT ?, m.id, m.folder_id, json_extract(j.value, '$[1]')
                 FROM json_each(?) j JOIN messages m ON m.id = json_extract(j.value, '$[0]')
                 ORDER BY j.key",
            )
            .bind(&self.receipt_id)
            .bind(serde_json::to_string(&originals).map_err(|e| e.to_string())?)
            .execute(&mut **tx)
            .await
            .map_err(|e| e.to_string())?;
        }
        for (folder_id, ids) in &pending {
            sqlx::query(
                "UPDATE messages SET pending_folder_id = ?, updated_at = ?
                 WHERE id IN (SELECT value FROM json_each(?))",
            )
            .bind(folder_id)
            .bind(now)
            .bind(json_ids(ids)?)
            .execute(&mut **tx)
            .await
            .map_err(|e| e.to_string())?;
        }
        for (folder_id, ids) in &local {
            sqlx::query(
                "UPDATE messages SET folder_id = ?, pending_folder_id = NULL, updated_at = ?
                 WHERE id IN (SELECT value FROM json_each(?))",
            )
            .bind(folder_id)
            .bind(now)
            .bind(json_ids(ids)?)
            .execute(&mut **tx)
            .await
            .map_err(|e| e.to_string())?;
        }
        if !to_inbox.is_empty() {
            sqlx::query("UPDATE threads SET workflow_state = 'inbox', board_column = 'inbox', snooze_until = NULL, workflow_changed_at = ?, updated_at = ? WHERE id IN (SELECT value FROM json_each(?))")
                .bind(now).bind(now).bind(json_ids(&to_inbox)?).execute(&mut **tx).await.map_err(|e| e.to_string())?;
        }
        if !to_done.is_empty() {
            sqlx::query("UPDATE threads SET workflow_state = 'done', board_column = 'done', workflow_changed_at = ?, updated_at = ? WHERE id IN (SELECT value FROM json_each(?))")
                .bind(now).bind(now).bind(json_ids(&to_done)?).execute(&mut **tx).await.map_err(|e| e.to_string())?;
        }
        if !receipt_actions.is_empty() {
            sqlx::query(
                "INSERT INTO mail_action_queue (receipt_id, action_id)
                 SELECT ?, value FROM json_each(?) ORDER BY key",
            )
            .bind(&self.receipt_id)
            .bind(json_ids(&receipt_actions)?)
            .execute(&mut **tx)
            .await
            .map_err(|e| e.to_string())?;
        }
        crate::db::threads::refresh_thread_projections_batch_tx(tx, &refresh, now)
            .await
            .map_err(|e| e.to_string())
    }
}

fn json_ids(ids: &[String]) -> Result<String, String> {
    serde_json::to_string(ids).map_err(|e| e.to_string())
}

fn remote_mutation(account: &str, payload: RemoteMutationPayload) -> Result<QueuedAction, String> {
    Ok(QueuedAction::new(
        account.into(),
        ACTION_REMOTE_MUTATION,
        serde_json::to_value(payload).map_err(|e| e.to_string())?,
    ))
}

/// Account and provider of each thread that exists.
async fn thread_accounts(
    tx: &mut Transaction<'_, Sqlite>,
    ids: &[String],
) -> Result<HashMap<String, (String, String)>, String> {
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT t.id, t.account_id, a.provider FROM threads t JOIN accounts a ON a.id = t.account_id
         WHERE t.id IN (SELECT value FROM json_each(?))",
    )
    .bind(json_ids(ids)?)
    .fetch_all(&mut **tx)
    .await
    .map_err(|e| e.to_string())?;
    Ok(rows
        .into_iter()
        .map(|(id, account, provider)| (id, (account, provider)))
        .collect())
}

/// [`messages`] for many threads in one query, each thread's rows in the
/// order the single query returns them.
async fn thread_messages(
    tx: &mut Transaction<'_, Sqlite>,
    ids: &[String],
) -> Result<HashMap<String, Vec<Message>>, String> {
    let rows: Vec<Message> = sqlx::query_as(
        "SELECT m.thread_id, m.id, CASE WHEN a.kind = 'email' THEN m.remote_id ELSE NULL END AS remote_id, COALESCE(f.remote_id, f.full_path) AS source_folder,
        f.uidvalidity AS source_uidvalidity, f.folder_type
        FROM json_each(?) j JOIN messages m ON m.thread_id = j.value
        JOIN folders f ON f.id = m.folder_id JOIN accounts a ON a.id = m.account_id
        ORDER BY j.key, COALESCE(m.pending_folder_id, m.folder_id), m.is_read, m.rowid",
    )
    .bind(json_ids(ids)?)
    .fetch_all(&mut **tx)
    .await
    .map_err(|e| e.to_string())?;
    let mut grouped: HashMap<String, Vec<Message>> = HashMap::new();
    for row in rows {
        grouped.entry(row.thread_id.clone()).or_default().push(row);
    }
    Ok(grouped)
}

async fn resolve_target(
    tx: &mut Transaction<'_, Sqlite>,
    account: &str,
    provider: &str,
    target: &MoveTarget<'_>,
    now: &str,
) -> Result<(String, String, String), String> {
    if let MoveTarget::Folder(id) = target {
        return sqlx::query_as("SELECT id, COALESCE(remote_id, full_path), folder_type FROM folders WHERE id = ? AND account_id = ?")
            .bind(id).bind(account).fetch_optional(&mut **tx).await.map_err(|e| e.to_string())?
            .ok_or_else(|| "backend.errors.folderAccountMismatch".into());
    }
    let kind = match target {
        MoveTarget::Archive => "archive",
        MoveTarget::Trash => "trash",
        MoveTarget::Spam => "spam",
        MoveTarget::Inbox => "inbox",
        MoveTarget::Folder(_) => unreachable!(),
    };
    if let Some(row) = sqlx::query_as("SELECT id, COALESCE(remote_id, full_path), folder_type FROM folders WHERE account_id = ? AND folder_type = ? LIMIT 1")
        .bind(account).bind(kind).fetch_optional(&mut **tx).await.map_err(|e| e.to_string())? {
        return Ok(row);
    }
    let remote = provisioned_remote_folder(provider, kind).to_string();
    let id = crate::db::folders::resolve_provider_folder_id(&mut **tx, account, &remote)
        .await
        .map_err(|err| err.to_string())?;
    let (name, path) = match (provider, kind) {
        (_, "archive") => ("Archiviert", "Archive"),
        ("outlook", "trash") => ("Papierkorb", "Deleted Items"),
        (_, "trash") => ("Papierkorb", "Trash"),
        _ => (remote.as_str(), remote.as_str()),
    };
    sqlx::query("INSERT INTO folders (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT DO NOTHING")
        .bind(&id).bind(account).bind(name).bind(path).bind(kind).bind(&remote).bind(now).bind(now)
        .execute(&mut **tx).await.map_err(|e| e.to_string())?;
    // A row for the remote folder may already exist under another type.
    let id: String = sqlx::query_scalar(
        "SELECT id FROM folders WHERE account_id = ? AND COALESCE(remote_id, full_path) = ?",
    )
    .bind(account)
    .bind(&remote)
    .fetch_one(&mut **tx)
    .await
    .map_err(|e| e.to_string())?;
    Ok((id, remote, kind.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn pop3_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory pool");
        crate::db::init(&pool).await.expect("migrations");
        sqlx::query("INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, created_at, updated_at) VALUES ('acc', 'POP3', 'me@example.test', 'pop3', '{}', '', '2026-01-01', '2026-01-01')")
            .execute(&pool).await.expect("account");
        sqlx::query("INSERT INTO folders (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at) VALUES ('inbox', 'acc', 'Inbox', 'INBOX', 'inbox', 'INBOX', '2026-01-01', '2026-01-01')")
            .execute(&pool).await.expect("folder");
        sqlx::query("INSERT INTO folders (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at) VALUES ('custom', 'acc', 'Local', 'Local', 'custom', 'Local', '2026-01-01', '2026-01-01')")
            .execute(&pool).await.expect("custom folder");
        sqlx::query("INSERT INTO threads (id, account_id, subject, last_message_date, created_at, updated_at) VALUES ('thread', 'acc', 'Subject', '2026-01-01', '2026-01-01', '2026-01-01')")
            .execute(&pool).await.expect("thread");
        sqlx::query("INSERT INTO messages (id, account_id, folder_id, thread_id, from_address, to_addresses, date, remote_id, created_at, updated_at) VALUES ('message', 'acc', 'inbox', 'thread', 'sender@example.test', '[]', '2026-01-01', 'UIDL-1', '2026-01-01', '2026-01-01')")
            .execute(&pool).await.expect("message");
        pool
    }

    #[tokio::test]
    async fn pop3_archive_and_trash_complete_locally_without_remote_moves() {
        let pool = pop3_pool().await;
        let queue = SyncQueue::new();
        let ids = vec!["thread".to_string()];

        assert!(
            move_threads(&pool, &queue, &ids, MoveTarget::Folder("custom"))
                .await
                .expect("local folder move")
                .is_empty()
        );
        let folder_id: String =
            sqlx::query_scalar("SELECT folder_id FROM messages WHERE id = 'message'")
                .fetch_one(&pool)
                .await
                .expect("moved message");
        assert_eq!(folder_id, "custom");

        assert!(move_threads(&pool, &queue, &ids, MoveTarget::Archive)
            .await
            .expect("archive")
            .is_empty());
        let (folder_type, pending): (String, Option<String>) = sqlx::query_as("SELECT f.folder_type, m.pending_folder_id FROM messages m JOIN folders f ON f.id = m.folder_id WHERE m.id = 'message'")
            .fetch_one(&pool).await.expect("archived message");
        assert_eq!(folder_type, "archive");
        assert!(pending.is_none());

        set_flags(&pool, &queue, &ids, Some(true), None, None)
            .await
            .expect("follow-up flags");
        assert!(move_threads(&pool, &queue, &ids, MoveTarget::Trash)
            .await
            .expect("trash")
            .is_empty());
        let (folder_type, pending): (String, Option<String>) = sqlx::query_as("SELECT f.folder_type, m.pending_folder_id FROM messages m JOIN folders f ON f.id = m.folder_id WHERE m.id = 'message'")
            .fetch_one(&pool).await.expect("trashed message");
        assert_eq!(folder_type, "trash");
        assert!(pending.is_none());
        let moves: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_queue WHERE json_extract(payload, '$.kind') = 'workflow_move'")
            .fetch_one(&pool).await.expect("move count");
        assert_eq!(moves, 0);
    }

    #[tokio::test]
    async fn failed_legacy_pop3_move_is_reconciled_on_startup() {
        let pool = pop3_pool().await;
        let queue = SyncQueue::new();
        let ids = vec!["thread".to_string()];
        let now = "2026-01-01";
        let (archive, _, _) = {
            let mut tx = pool.begin().await.expect("transaction");
            let target = resolve_target(&mut tx, "acc", "pop3", &MoveTarget::Archive, now)
                .await
                .expect("archive target");
            tx.commit().await.expect("target commit");
            target
        };
        sqlx::query("UPDATE messages SET pending_folder_id = ? WHERE id = 'message'")
            .bind(&archive)
            .execute(&pool)
            .await
            .expect("pending move");
        let mut stale = QueuedAction::new(
            "acc".into(),
            ACTION_REMOTE_MUTATION,
            serde_json::json!({"kind":"workflow_move","workflow_thread_id":"thread"}),
        );
        stale.status = "failed".into();
        queue.enqueue(&pool, stale).await.expect("stale move");

        assert_eq!(
            reconcile_all_legacy_pop3_moves(&pool)
                .await
                .expect("recovery"),
            1
        );
        set_flags(&pool, &queue, &ids, Some(true), None, None)
            .await
            .expect("flag after obsolete POP3 move");
        let (folder_id, pending): (String, Option<String>) = sqlx::query_as(
            "SELECT folder_id, pending_folder_id FROM messages WHERE id = 'message'",
        )
        .fetch_one(&pool)
        .await
        .expect("message state");
        assert_eq!(folder_id, archive);
        assert!(pending.is_none());
        let moves: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_queue WHERE json_extract(payload, '$.kind') = 'workflow_move'")
            .fetch_one(&pool).await.expect("move count");
        assert_eq!(moves, 0);
    }
}
