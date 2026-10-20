//! Durable mail actions shared by command and notification adapters.
//!
//! Queue intent and local presentation commit together. Confirmed provider
//! identities never change before the queued move has been reconciled.

pub mod undo;

use std::collections::HashMap;

use sqlx::{FromRow, Sqlite, SqlitePool, Transaction};

use crate::db::threads::refresh_thread_projection_tx;
use crate::providers::{folder_mapping::provisioned_remote_folder, traits::MessageRef};
use crate::sync::{
    queue::{QueuedAction, SyncQueue},
    remote_mutations::{
        remote_move_thread_id_from_json, PreparedWorkflowMove, RemoteMutationPayload,
        ACTION_REMOTE_MUTATION,
    },
};

#[derive(Debug, Clone, Default, serde::Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "core/mail_actions/"))]
pub struct TrashReceipt {
    pub undo_token: Option<String>,
    pub undo_thread_ids: Vec<String>,
    pub permanently_deleted: Vec<String>,
}

pub enum MoveTarget<'a> {
    Folder(&'a str),
    Archive,
    Trash,
    Spam,
}

#[derive(FromRow)]
struct Message {
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
    let requested: Vec<String> = sqlx::query_scalar("SELECT t.thread_id FROM mail_action_threads t JOIN mail_action_receipts r ON r.id = t.receipt_id WHERE r.state = 'requested' AND (? IS NULL OR r.id != ?)")
        .bind(ignored_receipt).bind(ignored_receipt).fetch_all(&mut **tx).await.map_err(|e| e.to_string())?;
    if requested.iter().any(|id| thread_ids.contains(id)) {
        return Err("backend.errors.mailMovePending".into());
    }
    let payloads = sqlx::query_scalar::<_, String>(
        "SELECT payload FROM sync_queue WHERE action_type = 'remote_mutation'
         AND status IN ('pending', 'scheduled', 'sending', 'failed')",
    )
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

/// Discard a failed move and its unconfirmed destination under one write lock.
pub async fn discard_failed_move(pool: &SqlitePool, id: &str) -> Result<bool, String> {
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|e| e.to_string())?;
    let row: Option<(String, String)> = sqlx::query_as(
        "DELETE FROM sync_queue WHERE id = ? AND action_type = 'remote_mutation' AND status = 'failed'
         AND json_valid(payload) AND json_extract(payload, '$.kind') IN ('workflow_move', 'blocked_move') RETURNING account_id, payload",
    ).bind(id).fetch_optional(&mut *tx).await.map_err(|e| e.to_string())?;
    let Some((account_id, payload)) = row else {
        return Ok(false);
    };
    let thread_id =
        remote_move_thread_id_from_json(&payload).ok_or("backend.errors.mailMovePending")?;
    release_move_destination(&mut tx, &account_id, &thread_id).await?;
    crate::db::threads::refresh_thread_projections_tx(&mut tx, &[thread_id])
        .await
        .map_err(|e| e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(true)
}

/// Drop failed queue rows last updated before `cutoff` (RFC 3339), as retention.
///
/// A failed move still owns its unconfirmed destination, and mailbox queries
/// list the thread under `pending_folder_id`. Deleting the row alone left the
/// thread in the target folder for good, so each purged move releases its
/// destination in the same transaction, as [`discard_failed_move`] does.
/// Failed sends are never purged: without a draft, the payload is the only
/// copy of a mail that did not go out.
pub async fn purge_failed_queue_rows(pool: &SqlitePool, cutoff: &str) -> Result<u64, String> {
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|e| e.to_string())?;
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "DELETE FROM sync_queue WHERE status = 'failed' AND updated_at < ? AND action_type <> ?
         RETURNING account_id, action_type, payload",
    )
    .bind(cutoff)
    .bind(crate::sync::queue::ACTION_SEND_MESSAGE)
    .fetch_all(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;
    let mut released = Vec::new();
    for (account_id, action_type, payload) in &rows {
        if action_type != ACTION_REMOTE_MUTATION {
            continue;
        }
        if let Some(thread_id) = remote_move_thread_id_from_json(payload) {
            release_move_destination(&mut tx, account_id, &thread_id).await?;
            released.push(thread_id);
        }
    }
    released.sort();
    released.dedup();
    crate::db::threads::refresh_thread_projections_tx(&mut tx, &released)
        .await
        .map_err(|e| e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(rows.len() as u64)
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
    let actions = sqlx::query_as::<_, (String, String)>(
        "SELECT q.id, q.payload FROM sync_queue q
         JOIN accounts a ON a.id = q.account_id
         WHERE LOWER(a.provider) = 'pop3' AND q.action_type = 'remote_mutation'
           AND q.status IN ('pending', 'scheduled', 'sending', 'failed')",
    )
    .fetch_all(&mut **tx)
    .await
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
        "SELECT m.id, CASE WHEN a.kind = 'email' THEN m.remote_id ELSE NULL END AS remote_id, COALESCE(f.remote_id, f.full_path) AS source_folder,
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
/// the folder it came from and restores the workflow state.
pub async fn mark_threads_spam(
    pool: &SqlitePool,
    queue: &SyncQueue,
    ids: &[String],
) -> Result<TrashReceipt, String> {
    move_threads_with_receipt(pool, queue, ids, MoveTarget::Spam, true)
        .await
        .map(|(_, receipt)| receipt)
}

pub async fn move_threads(
    pool: &SqlitePool,
    queue: &SyncQueue,
    ids: &[String],
    target: MoveTarget<'_>,
) -> Result<Vec<String>, String> {
    move_threads_with_receipt(pool, queue, ids, target, false)
        .await
        .map(|(queued, _)| queued)
}

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
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|e| e.to_string())?;
    ensure_no_pending_move(&mut tx, ids).await?;
    for id in ids {
        sqlx::query("DELETE FROM mail_action_receipts WHERE state = 'available' AND id IN (SELECT receipt_id FROM mail_action_threads WHERE thread_id = ?)")
            .bind(id).execute(&mut *tx).await.map_err(|e| e.to_string())?;
    }
    let now = chrono::Utc::now().to_rfc3339();
    let mut queued = Vec::new();
    let mut receipt = TrashReceipt::default();
    let receipt_id = uuid::Uuid::new_v4().to_string();
    if record_undo {
        // Keep requested reversals until processed. Available receipts only serve short-lived UI actions.
        sqlx::query(
            "DELETE FROM mail_action_receipts WHERE state != 'requested' AND created_at < ?",
        )
        .bind((chrono::Utc::now() - chrono::Duration::days(1)).to_rfc3339())
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
        sqlx::query("INSERT INTO mail_action_receipts (id, created_at) VALUES (?, ?)")
            .bind(&receipt_id)
            .bind(&now)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
    }
    let mut seen = std::collections::HashSet::new();
    for id in ids.iter().filter(|id| seen.insert(id.as_str())) {
        let (account, provider): (String, String) = sqlx::query_as(
            "SELECT t.account_id, a.provider FROM threads t JOIN accounts a ON a.id = t.account_id WHERE t.id = ?")
            .bind(id).fetch_one(&mut *tx).await.map_err(|e| e.to_string())?;
        let rows = messages(&mut tx, id).await?;
        let permanent = matches!(target, MoveTarget::Trash)
            && !rows.is_empty()
            && rows.iter().all(|m| m.folder_type == "trash");
        if permanent {
            receipt.permanently_deleted.push(id.clone());
            let refs = rows
                .iter()
                .filter_map(Message::remote_ref)
                .collect::<Vec<_>>();
            if !refs.is_empty() {
                queued.push(
                    enqueue(
                        &mut tx,
                        queue,
                        &account,
                        RemoteMutationPayload::Delete { refs },
                        None,
                    )
                    .await?,
                );
            }
            sqlx::query("DELETE FROM attachments WHERE message_id IN (SELECT id FROM messages WHERE thread_id = ?)")
                .bind(id).execute(&mut *tx).await.map_err(|e| e.to_string())?;
            sqlx::query("DELETE FROM messages WHERE thread_id = ?")
                .bind(id)
                .execute(&mut *tx)
                .await
                .map_err(|e| e.to_string())?;
            refresh_thread_projection_tx(&mut tx, id, &now)
                .await
                .map_err(|e| e.to_string())?;
            continue;
        }
        let queue_start = queued.len();
        if record_undo {
            sqlx::query("INSERT INTO mail_action_threads (receipt_id, thread_id, account_id, workflow_state, board_column, snooze_until, action_at) SELECT ?, id, account_id, workflow_state, board_column, snooze_until, ? FROM threads WHERE id = ?")
                .bind(&receipt_id).bind(&now).bind(id).execute(&mut *tx).await.map_err(|e| e.to_string())?;
            receipt.undo_thread_ids.push(id.clone());
        }
        let (folder_id, remote, kind) =
            resolve_target(&mut tx, &account, &provider, &target, &now).await?;
        let rows = rows
            .into_iter()
            .filter(|m| m.source_folder != remote)
            .filter(|m| {
                !matches!(target, MoveTarget::Archive)
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
        if matches!(target, MoveTarget::Archive | MoveTarget::Spam) {
            if !provider.eq_ignore_ascii_case("pop3") && !refs.is_empty() {
                queued.push(
                    enqueue(
                        &mut tx,
                        queue,
                        &account,
                        RemoteMutationPayload::Flags {
                            refs: refs.clone(),
                            read: Some(true),
                            starred: None,
                        },
                        None,
                    )
                    .await?,
                );
            }
            for message in &rows {
                sqlx::query("UPDATE messages SET is_read = 1 WHERE id = ?")
                    .bind(&message.id)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| e.to_string())?;
            }
        }
        if !provider.eq_ignore_ascii_case("pop3") && !refs.is_empty() {
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
            queue.enqueue_in_transaction(&mut tx, &action).await?;
            queued.push(action.id);
        }
        for message in rows {
            if record_undo {
                sqlx::query("INSERT INTO mail_action_messages (receipt_id, message_id, original_folder_id, target_folder_id) SELECT ?, id, folder_id, ? FROM messages WHERE id = ?")
                    .bind(&receipt_id).bind(&folder_id).bind(&message.id).execute(&mut *tx).await.map_err(|e| e.to_string())?;
            }
            if !provider.eq_ignore_ascii_case("pop3") && message.remote_ref().is_some() {
                sqlx::query(
                    "UPDATE messages SET pending_folder_id = ?, updated_at = ? WHERE id = ?",
                )
                .bind(&folder_id)
                .bind(&now)
                .bind(&message.id)
                .execute(&mut *tx)
                .await
                .map_err(|e| e.to_string())?;
            } else {
                sqlx::query("UPDATE messages SET folder_id = ?, pending_folder_id = NULL, updated_at = ? WHERE id = ?")
                    .bind(&folder_id).bind(&now).bind(&message.id).execute(&mut *tx).await.map_err(|e| e.to_string())?;
            }
        }
        if !matches!(target, MoveTarget::Folder(_)) {
            sqlx::query("UPDATE threads SET workflow_state = 'done', board_column = 'done', workflow_changed_at = ?, updated_at = ? WHERE id = ?")
                .bind(&now).bind(&now).bind(id).execute(&mut *tx).await.map_err(|e| e.to_string())?;
        }
        if record_undo {
            for action_id in &queued[queue_start..] {
                sqlx::query("INSERT INTO mail_action_queue (receipt_id, action_id) VALUES (?, ?)")
                    .bind(&receipt_id)
                    .bind(action_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| e.to_string())?;
            }
        }
        refresh_thread_projection_tx(&mut tx, id, &now)
            .await
            .map_err(|e| e.to_string())?;
    }
    if record_undo {
        if receipt.undo_thread_ids.is_empty() {
            sqlx::query("DELETE FROM mail_action_receipts WHERE id = ?")
                .bind(&receipt_id)
                .execute(&mut *tx)
                .await
                .map_err(|e| e.to_string())?;
        } else {
            receipt.undo_token = Some(receipt_id);
        }
    }
    tx.commit().await.map_err(|e| e.to_string())?;
    queue.wake_worker();
    Ok((queued, receipt))
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

    #[tokio::test]
    async fn purging_old_failed_rows_releases_moves_and_keeps_unsent_mail() {
        let pool = pop3_pool().await;
        let queue = SyncQueue::new();
        sqlx::query("UPDATE messages SET pending_folder_id = 'custom' WHERE id = 'message'")
            .execute(&pool)
            .await
            .expect("pending move");
        let old = "2025-01-01T00:00:00+00:00";
        let failed = |action_type: &str, payload: serde_json::Value, updated_at: &str| {
            let mut action = QueuedAction::new("acc".into(), action_type, payload);
            action.status = "failed".into();
            action.updated_at = updated_at.into();
            action
        };
        let rows = [
            failed(
                ACTION_REMOTE_MUTATION,
                serde_json::json!({"kind":"workflow_move","workflow_thread_id":"thread"}),
                old,
            ),
            failed(
                crate::sync::queue::ACTION_SEND_MESSAGE,
                serde_json::json!({"input": {"subject": "only copy"}}),
                old,
            ),
            failed(
                ACTION_REMOTE_MUTATION,
                serde_json::json!({"kind":"set_flags"}),
                &chrono::Utc::now().to_rfc3339(),
            ),
        ];
        for row in &rows {
            queue.enqueue(&pool, row.clone()).await.expect("failed row");
        }

        let cutoff = "2025-06-01T00:00:00+00:00";
        assert_eq!(purge_failed_queue_rows(&pool, cutoff).await.unwrap(), 1);

        let pending: Option<String> =
            sqlx::query_scalar("SELECT pending_folder_id FROM messages WHERE id = 'message'")
                .fetch_one(&pool)
                .await
                .expect("message");
        assert!(pending.is_none(), "the purged move kept its destination");
        let left: Vec<String> = sqlx::query_scalar("SELECT id FROM sync_queue ORDER BY id")
            .fetch_all(&pool)
            .await
            .expect("rows");
        let mut expected = vec![rows[1].id.clone(), rows[2].id.clone()];
        expected.sort();
        assert_eq!(left, expected);
    }
}
