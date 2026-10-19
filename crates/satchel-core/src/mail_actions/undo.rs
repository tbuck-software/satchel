//! A receipt owns only the deliveries changed by its action. Queue claims and
//! cancellation share a write reservation; started deliveries finish first.
use super::{ensure_no_pending_move_except_receipt, messages};
use crate::db::threads::refresh_thread_projection_tx;
use crate::providers::traits::MessageRef;
use crate::sync::{
    queue::{QueuedAction, SyncQueue},
    remote_mutations::{PreparedWorkflowMove, RemoteMutationPayload, ACTION_REMOTE_MUTATION},
};
use sqlx::{FromRow, Sqlite, SqlitePool, Transaction};
use std::collections::HashMap;

#[derive(FromRow)]
struct ThreadSnapshot {
    thread_id: String,
    account_id: String,
    workflow_state: String,
    board_column: String,
    snooze_until: Option<String>,
    action_at: String,
}

#[derive(FromRow)]
struct MessageSnapshot {
    message_id: String,
    original_folder_id: String,
    target_folder_id: String,
}

/// Accept Undo durably even if the provider has already claimed the delivery.
/// The worker completes reconciliation before resolving the reverse refs.
pub async fn undo_trash(
    pool: &SqlitePool,
    queue: &SyncQueue,
    receipt_id: &str,
) -> Result<(), String> {
    apply(pool, queue, receipt_id, false).await
}

pub async fn execute_queued_undo(
    pool: &SqlitePool,
    queue: &SyncQueue,
    receipt_id: &str,
) -> Result<(), String> {
    apply(pool, queue, receipt_id, true).await
}

async fn apply(
    pool: &SqlitePool,
    queue: &SyncQueue,
    receipt_id: &str,
    from_worker: bool,
) -> Result<(), String> {
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|e| e.to_string())?;
    let state: Option<String> =
        sqlx::query_scalar("SELECT state FROM mail_action_receipts WHERE id = ?")
            .bind(receipt_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
    match state.as_deref() {
        Some("consumed") => return Ok(()),
        Some("available" | "requested") => {}
        _ => return Err("backend.errors.mailUndoUnavailable".into()),
    }
    // Conflict handling may follow partial reverse planning. Roll it back before
    // committing the terminal receipt state and releasing its thread guard.
    sqlx::query("SAVEPOINT undo_apply")
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    let threads: Vec<ThreadSnapshot> = sqlx::query_as("SELECT thread_id, account_id, workflow_state, board_column, snooze_until, action_at FROM mail_action_threads WHERE receipt_id = ?")
        .bind(receipt_id).fetch_all(&mut *tx).await.map_err(|e| e.to_string())?;
    // A later user operation owns its own state. Never overwrite it with an old receipt.
    for snapshot in &threads {
        let unchanged: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM threads WHERE id = ? AND workflow_state = 'done' AND workflow_changed_at = ?)")
            .bind(&snapshot.thread_id).bind(&snapshot.action_at).fetch_one(&mut *tx).await.map_err(|e| e.to_string())?;
        if !unchanged {
            return invalidate(tx, receipt_id).await;
        }
    }
    let deliveries: Vec<(String, String, String)> = sqlx::query_as("SELECT q.id, q.status, q.payload FROM sync_queue q JOIN mail_action_queue a ON a.action_id = q.id WHERE a.receipt_id = ?")
        .bind(receipt_id).fetch_all(&mut *tx).await.map_err(|e| e.to_string())?;
    let in_flight = deliveries.iter().any(|(_, status, payload)| {
        status == "sending"
            || serde_json::from_str::<serde_json::Value>(payload)
                .map(|p| {
                    p["provider_move_started"].as_bool().unwrap_or(false)
                        || p["provider_move_completed"].as_bool().unwrap_or(false)
                })
                .unwrap_or(true)
    });
    if in_flight {
        if from_worker {
            return Err("backend.errors.mailMovePending".into());
        }
        let account = threads
            .first()
            .ok_or("backend.errors.mailUndoUnavailable")?
            .account_id
            .clone();
        let mut action = QueuedAction::new(
            account,
            ACTION_REMOTE_MUTATION,
            serde_json::to_value(RemoteMutationPayload::UndoTrash {
                receipt_id: receipt_id.into(),
            })
            .map_err(|e| e.to_string())?,
        );
        action.id = format!("undo-trash:{receipt_id}");
        queue.enqueue_in_transaction(&mut tx, &action).await?;
        for (dependency, _, _) in &deliveries {
            sqlx::query("INSERT OR IGNORE INTO sync_queue_dependencies (action_id, depends_on) VALUES (?, ?)")
                .bind(&action.id).bind(dependency).execute(&mut *tx).await.map_err(|e| e.to_string())?;
        }
        sqlx::query("UPDATE mail_action_receipts SET state = 'requested' WHERE id = ?")
            .bind(receipt_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        tx.commit().await.map_err(|e| e.to_string())?;
        queue.wake_worker();
        return Ok(());
    }
    for (action_id, _, _) in deliveries {
        sqlx::query("DELETE FROM sync_queue WHERE id = ?")
            .bind(action_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
    }
    let ids = threads
        .iter()
        .map(|thread| thread.thread_id.clone())
        .collect::<Vec<_>>();
    ensure_no_pending_move_except_receipt(&mut tx, &ids, Some(receipt_id)).await?;
    let snapshots: Vec<MessageSnapshot> = sqlx::query_as("SELECT message_id, original_folder_id, target_folder_id FROM mail_action_messages WHERE receipt_id = ?")
        .bind(receipt_id).fetch_all(&mut *tx).await.map_err(|e| e.to_string())?;
    let mut reverse = HashMap::<(String, String, String), Vec<MessageRef>>::new();
    let mut current_refs = HashMap::new();
    let now = chrono::Utc::now().to_rfc3339();
    for snapshot in snapshots {
        let current: Option<(String, String, String, String)> = sqlx::query_as("SELECT m.account_id, m.thread_id, a.provider, m.folder_id FROM messages m JOIN accounts a ON a.id = m.account_id WHERE m.id = ?")
            .bind(&snapshot.message_id).fetch_optional(&mut *tx).await.map_err(|e| e.to_string())?;
        let Some((account, thread, provider, current_folder)) = current else {
            return invalidate(tx, receipt_id).await;
        };
        if current_folder != snapshot.original_folder_id
            && current_folder != snapshot.target_folder_id
        {
            return invalidate(tx, receipt_id).await;
        }
        if current_folder == snapshot.original_folder_id || provider.eq_ignore_ascii_case("pop3") {
            sqlx::query("UPDATE messages SET folder_id = ?, pending_folder_id = NULL, updated_at = ? WHERE id = ?")
                .bind(&snapshot.original_folder_id).bind(&now).bind(&snapshot.message_id).execute(&mut *tx).await.map_err(|e| e.to_string())?;
            continue;
        }
        if let std::collections::hash_map::Entry::Vacant(entry) = current_refs.entry(thread.clone())
        {
            entry.insert(
                messages(&mut tx, &thread)
                    .await?
                    .into_iter()
                    .map(|message| (message.id.clone(), message.remote_ref()))
                    .collect::<HashMap<_, _>>(),
            );
        }
        let Some(reference) = current_refs
            .get(&thread)
            .and_then(|refs| refs.get(&snapshot.message_id))
            .cloned()
        else {
            return invalidate(tx, receipt_id).await;
        };
        if let Some(reference) = reference {
            reverse
                .entry((account, thread, snapshot.original_folder_id.clone()))
                .or_default()
                .push(reference);
            sqlx::query("UPDATE messages SET pending_folder_id = ?, updated_at = ? WHERE id = ?")
                .bind(&snapshot.original_folder_id)
                .bind(&now)
                .bind(&snapshot.message_id)
                .execute(&mut *tx)
                .await
                .map_err(|e| e.to_string())?;
        } else {
            sqlx::query("UPDATE messages SET folder_id = ?, pending_folder_id = NULL, updated_at = ? WHERE id = ?")
                .bind(&snapshot.original_folder_id).bind(&now).bind(&snapshot.message_id).execute(&mut *tx).await.map_err(|e| e.to_string())?;
        }
    }
    for ((account, thread, original_folder), refs) in reverse {
        let folder: Option<(String, String)> = sqlx::query_as("SELECT COALESCE(remote_id, full_path), folder_type FROM folders WHERE id = ? AND account_id = ?")
            .bind(&original_folder).bind(&account).fetch_optional(&mut *tx).await.map_err(|e| e.to_string())?;
        let Some((remote, kind)) = folder else {
            return invalidate(tx, receipt_id).await;
        };
        let mut action = PreparedWorkflowMove {
            account_id: account,
            source_folder_type: "trash".into(),
            target_folder_type: kind,
            refs,
            target_remote_folder_id: remote,
        }
        .into_action(&thread, chrono::Utc::now())?;
        action.status = "pending".into();
        action.scheduled_for = None;
        queue.enqueue_in_transaction(&mut tx, &action).await?;
    }
    for snapshot in threads {
        sqlx::query("UPDATE threads SET workflow_state = ?, board_column = ?, snooze_until = ?, workflow_changed_at = ?, updated_at = ? WHERE id = ?")
            .bind(snapshot.workflow_state).bind(snapshot.board_column).bind(snapshot.snooze_until).bind(&now).bind(&now).bind(&snapshot.thread_id).execute(&mut *tx).await.map_err(|e| e.to_string())?;
        refresh_thread_projection_tx(&mut tx, &snapshot.thread_id, &now)
            .await
            .map_err(|e| e.to_string())?;
    }
    sqlx::query("UPDATE mail_action_receipts SET state = 'consumed' WHERE id = ?")
        .bind(receipt_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())?;
    queue.wake_worker();
    Ok(())
}

async fn invalidate(mut tx: Transaction<'_, Sqlite>, receipt_id: &str) -> Result<(), String> {
    sqlx::query("ROLLBACK TO SAVEPOINT undo_apply")
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    sqlx::query("UPDATE mail_action_receipts SET state = 'conflicted' WHERE id = ?")
        .bind(receipt_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())?;
    Err("backend.errors.mailUndoUnavailable".into())
}

/// The IMAP target proof is the authority for the replacement local ID. Record
/// it in the same transaction that removes the old delivery projection.
pub async fn remap_message_in_transaction(
    tx: &mut Transaction<'_, Sqlite>,
    source_id: &str,
    target_id: &str,
) -> Result<(), String> {
    sqlx::query("UPDATE mail_action_messages SET message_id = ? WHERE message_id = ? AND receipt_id IN (SELECT id FROM mail_action_receipts WHERE state IN ('available', 'requested'))")
        .bind(target_id).bind(source_id).execute(&mut **tx).await.map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};

    async fn seed(pool: &SqlitePool, provider: &str) {
        crate::db::init(pool).await.expect("migrations");
        sqlx::query("INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, created_at, updated_at) VALUES ('acc', 'Mail', 'me@example.test', ?, '{}', '', '2026-01-01', '2026-01-01')")
            .bind(provider).execute(pool).await.expect("account");
        for (id, kind, remote, uidvalidity) in [
            ("inbox", "inbox", "INBOX", 41),
            ("trash", "trash", "Trash", 99),
        ] {
            sqlx::query("INSERT INTO folders (id, account_id, name, full_path, folder_type, remote_id, uidvalidity, created_at, updated_at) VALUES (?, 'acc', ?, ?, ?, ?, ?, '2026-01-01', '2026-01-01')")
                .bind(id).bind(remote).bind(remote).bind(kind).bind(remote).bind(uidvalidity).execute(pool).await.expect("folder");
        }
        sqlx::query("INSERT INTO threads (id, account_id, subject, workflow_state, board_column, snooze_until, last_message_date, created_at, updated_at) VALUES ('thread', 'acc', 'Subject', 'snoozed', 'snoozed', '2030-01-01', '2026-01-01', '2026-01-01', '2026-01-01')")
            .execute(pool).await.expect("thread");
        sqlx::query("INSERT INTO messages (id, account_id, folder_id, thread_id, from_address, to_addresses, date, remote_id, created_at, updated_at) VALUES ('message', 'acc', 'inbox', 'thread', 'sender@example.test', '[]', '2026-01-01', '42', '2026-01-01', '2026-01-01')")
            .execute(pool).await.expect("message");
    }
    async fn pool(provider: &str) -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("pool");
        seed(&pool, provider).await;
        pool
    }
    async fn trash(pool: &SqlitePool, queue: &SyncQueue) -> String {
        super::super::trash_threads(pool, queue, &["thread".into()])
            .await
            .expect("trash")
            .undo_token
            .expect("undo receipt")
    }
    async fn assert_restored(pool: &SqlitePool) {
        let state: (String, String, Option<String>) = sqlx::query_as(
            "SELECT workflow_state, board_column, snooze_until FROM threads WHERE id = 'thread'",
        )
        .fetch_one(pool)
        .await
        .expect("thread state");
        assert_eq!(
            state,
            (
                "snoozed".into(),
                "snoozed".into(),
                Some("2030-01-01".into())
            )
        );
    }
    #[tokio::test]
    async fn offline_undo_cancels_unclaimed_delivery_and_restores_exact_source() {
        let pool = pool("imap").await;
        let queue = SyncQueue::new();
        let token = trash(&pool, &queue).await;
        undo_trash(&pool, &queue, &token)
            .await
            .expect("offline undo");
        undo_trash(&pool, &queue, &token)
            .await
            .expect("idempotent undo");
        let message: (String, Option<String>) = sqlx::query_as(
            "SELECT folder_id, pending_folder_id FROM messages WHERE id = 'message'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(message, ("inbox".into(), None));
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_queue")
                .fetch_one(&pool)
                .await
                .unwrap(),
            0
        );
        assert_restored(&pool).await;
    }
    #[tokio::test]
    async fn completed_move_undo_uses_reconciled_destination_identity() {
        let pool = pool("imap").await;
        let queue = SyncQueue::new();
        let token = trash(&pool, &queue).await;
        let forward = queue.claim_next(&pool).await.unwrap().unwrap();
        // This is the same atomic source replacement used by the IMAP finalizer.
        let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
        sqlx::query("INSERT INTO messages (id, account_id, folder_id, thread_id, from_address, to_addresses, date, remote_id, created_at, updated_at) VALUES ('target-message', 'acc', 'trash', 'thread', 'sender@example.test', '[]', '2026-01-01', '700', '2026-01-01', '2026-01-01')").execute(&mut *tx).await.unwrap();
        remap_message_in_transaction(&mut tx, "message", "target-message")
            .await
            .unwrap();
        sqlx::query("DELETE FROM messages WHERE id = 'message'")
            .execute(&mut *tx)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        queue.mark_completed(&pool, &forward.id).await.unwrap();
        undo_trash(&pool, &queue, &token)
            .await
            .expect("completed undo");
        let reverse = queue.claim_next(&pool).await.unwrap().unwrap();
        assert_eq!(reverse.payload["refs"][0]["remote_id"], "700");
        assert_eq!(reverse.payload["refs"][0]["source_folder"], "Trash");
        assert_eq!(reverse.payload["refs"][0]["source_uidvalidity"], 99);
        assert_eq!(reverse.payload["target_remote_folder_id"], "INBOX");
        assert_restored(&pool).await;
    }
    #[tokio::test]
    async fn claimed_delivery_accepts_durable_undo_then_reverses_after_completion() {
        let pool = pool("gmail").await;
        let queue = SyncQueue::new();
        let token = trash(&pool, &queue).await;
        let forward = queue.claim_next(&pool).await.unwrap().unwrap();
        undo_trash(&pool, &queue, &token)
            .await
            .expect("durable undo acceptance");
        assert_eq!(
            sqlx::query_scalar::<_, String>("SELECT status FROM sync_queue WHERE id = ?")
                .bind(&forward.id)
                .fetch_one(&pool)
                .await
                .unwrap(),
            "sending"
        );
        assert!(execute_queued_undo(&pool, &queue, &token).await.is_err());
        assert!(queue
            .claim_ready(&pool, &format!("undo-trash:{token}"))
            .await
            .unwrap()
            .is_none());
        assert!(queue.ready_account_heads(&pool).await.unwrap().is_empty());
        crate::sync::remote_mutations::apply_completed_move_projection(
            &pool,
            "acc",
            &[MessageRef {
                remote_id: "42".into(),
                source_folder: Some("INBOX".into()),
                source_uidvalidity: Some(41),
            }],
            "trash",
        )
        .await
        .unwrap();
        queue.mark_completed(&pool, &forward.id).await.unwrap();
        assert!(
            super::super::set_flags(&pool, &queue, &["thread".into()], Some(true), None, None)
                .await
                .is_err()
        );
        let undo = queue.claim_next(&pool).await.unwrap().unwrap();
        assert_eq!(undo.payload["kind"], "undo_trash");
        execute_queued_undo(&pool, &queue, &token)
            .await
            .expect("reverse after completion");
        queue.mark_completed(&pool, &undo.id).await.unwrap();
        let reverse = queue.claim_next(&pool).await.unwrap().unwrap();
        assert_eq!(reverse.payload["refs"][0]["source_folder"], "Trash");
        assert_eq!(reverse.payload["refs"][0]["remote_id"], "42");
        assert_restored(&pool).await;
    }
    #[tokio::test]
    async fn crash_recovered_started_delivery_is_never_cancelled_or_resubmitted_by_undo() {
        let pool = pool("imap").await;
        let queue = SyncQueue::new();
        let token = trash(&pool, &queue).await;
        sqlx::query("UPDATE sync_queue SET payload = json_set(payload, '$.provider_move_started', json('true')), status = 'pending'").execute(&pool).await.unwrap();
        undo_trash(&pool, &queue, &token)
            .await
            .expect("durable undo");
        assert_eq!(sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_queue WHERE json_extract(payload, '$.kind') = 'workflow_move'").fetch_one(&pool).await.unwrap(), 1);
        assert_eq!(sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_queue WHERE json_extract(payload, '$.kind') = 'undo_trash'").fetch_one(&pool).await.unwrap(), 1);
        let folder: String =
            sqlx::query_scalar("SELECT folder_id FROM messages WHERE id = 'message'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(folder, "inbox");
    }
    #[tokio::test]
    async fn permanent_delete_returns_no_undo_capability() {
        let pool = pool("imap").await;
        let queue = SyncQueue::new();
        sqlx::query("UPDATE messages SET folder_id = 'trash'")
            .execute(&pool)
            .await
            .unwrap();
        let receipt = super::super::trash_threads(&pool, &queue, &["thread".into()])
            .await
            .expect("permanent delete");
        assert!(receipt.undo_token.is_none());
        assert!(receipt.undo_thread_ids.is_empty());
        assert_eq!(receipt.permanently_deleted, ["thread"]);
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM messages")
                .fetch_one(&pool)
                .await
                .unwrap(),
            0
        );
    }
    #[tokio::test]
    async fn mixed_batch_undo_never_cancels_a_permanent_provider_delete() {
        let pool = pool("imap").await;
        let queue = SyncQueue::new();
        sqlx::query("INSERT INTO threads (id, account_id, subject, last_message_date, created_at, updated_at) VALUES ('permanent', 'acc', 'Old', '2026-01-01', '2026-01-01', '2026-01-01')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO messages (id, account_id, folder_id, thread_id, from_address, to_addresses, date, remote_id, created_at, updated_at) VALUES ('permanent-message', 'acc', 'trash', 'permanent', 'sender@example.test', '[]', '2026-01-01', '84', '2026-01-01', '2026-01-01')").execute(&pool).await.unwrap();
        let receipt =
            super::super::trash_threads(&pool, &queue, &["thread".into(), "permanent".into()])
                .await
                .expect("mixed trash");
        assert_eq!(receipt.undo_thread_ids, ["thread"]);
        assert_eq!(receipt.permanently_deleted, ["permanent"]);
        undo_trash(&pool, &queue, receipt.undo_token.as_deref().unwrap())
            .await
            .expect("partial undo");
        let remaining = queue.claim_next(&pool).await.unwrap().unwrap();
        assert_eq!(remaining.payload["kind"], "delete");
        assert_eq!(remaining.payload["refs"][0]["remote_id"], "84");
        assert_restored(&pool).await;
    }

    #[tokio::test]
    async fn local_pop3_undo_restores_without_queuing_provider_moves() {
        let pool = pool("pop3").await;
        let queue = SyncQueue::new();
        let token = trash(&pool, &queue).await;
        undo_trash(&pool, &queue, &token).await.expect("local undo");
        let folder: String =
            sqlx::query_scalar("SELECT folder_id FROM messages WHERE id = 'message'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(folder, "inbox");
        assert!(queue.claim_next(&pool).await.unwrap().is_none());
        assert_restored(&pool).await;
    }

    #[tokio::test]
    async fn receipt_failure_rolls_back_provider_intent_and_local_trash() {
        let pool = pool("imap").await;
        let queue = SyncQueue::new();
        sqlx::query("CREATE TRIGGER reject_receipt BEFORE INSERT ON mail_action_messages BEGIN SELECT RAISE(ABORT, 'receipt rejected'); END").execute(&pool).await.unwrap();
        assert!(
            super::super::trash_threads(&pool, &queue, &["thread".into()])
                .await
                .is_err()
        );
        let message: (String, Option<String>) = sqlx::query_as(
            "SELECT folder_id, pending_folder_id FROM messages WHERE id = 'message'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(message, ("inbox".into(), None));
        assert!(queue.claim_next(&pool).await.unwrap().is_none());
        assert_restored(&pool).await;
    }

    #[tokio::test]
    async fn incoming_workflow_change_releases_a_conflicted_requested_undo() {
        let pool = pool("gmail").await;
        let queue = SyncQueue::new();
        let token = trash(&pool, &queue).await;
        let forward = queue.claim_next(&pool).await.unwrap().unwrap();
        undo_trash(&pool, &queue, &token)
            .await
            .expect("durable undo request");
        crate::sync::remote_mutations::apply_completed_move_projection(
            &pool,
            "acc",
            &[MessageRef {
                remote_id: "42".into(),
                source_folder: Some("INBOX".into()),
                source_uidvalidity: Some(41),
            }],
            "trash",
        )
        .await
        .unwrap();
        queue.mark_completed(&pool, &forward.id).await.unwrap();
        sqlx::query("UPDATE threads SET workflow_state = 'inbox', board_column = 'inbox', workflow_changed_at = '2031-01-01' WHERE id = 'thread'").execute(&pool).await.unwrap();
        let error = execute_queued_undo(&pool, &queue, &token)
            .await
            .expect_err("newer workflow state wins");
        assert_eq!(error, "backend.errors.mailUndoUnavailable");
        let state: String =
            sqlx::query_scalar("SELECT state FROM mail_action_receipts WHERE id = ?")
                .bind(&token)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(state, "conflicted");
        super::super::move_threads(
            &pool,
            &queue,
            &["thread".into()],
            super::super::MoveTarget::Folder("inbox"),
        )
        .await
        .expect("subsequent user move is not locked");
        let workflow: String =
            sqlx::query_scalar("SELECT workflow_state FROM threads WHERE id = 'thread'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(workflow, "inbox");
    }

    #[tokio::test]
    async fn missing_delivery_releases_requested_undo_without_partial_reverse() {
        let pool = pool("gmail").await;
        let queue = SyncQueue::new();
        sqlx::query("INSERT INTO messages (id, account_id, folder_id, thread_id, from_address, to_addresses, date, remote_id, created_at, updated_at) VALUES ('missing-message', 'acc', 'inbox', 'thread', 'sender@example.test', '[]', '2026-01-01', '43', '2026-01-01', '2026-01-01')").execute(&pool).await.unwrap();
        let token = trash(&pool, &queue).await;
        let forward = queue.claim_next(&pool).await.unwrap().unwrap();
        undo_trash(&pool, &queue, &token).await.unwrap();
        let refs = ["42", "43"].map(|id| MessageRef {
            remote_id: id.into(),
            source_folder: Some("INBOX".into()),
            source_uidvalidity: Some(41),
        });
        crate::sync::remote_mutations::apply_completed_move_projection(
            &pool, "acc", &refs, "trash",
        )
        .await
        .unwrap();
        queue.mark_completed(&pool, &forward.id).await.unwrap();
        sqlx::query("DELETE FROM messages WHERE id = 'missing-message'")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            execute_queued_undo(&pool, &queue, &token)
                .await
                .unwrap_err(),
            "backend.errors.mailUndoUnavailable"
        );
        let current: (String, Option<String>) = sqlx::query_as(
            "SELECT folder_id, pending_folder_id FROM messages WHERE id = 'message'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(current, ("trash".into(), None));
        super::super::set_flags(&pool, &queue, &["thread".into()], Some(true), None, None)
            .await
            .expect("terminal receipt releases thread guard");
    }

    #[tokio::test]
    async fn worker_claim_and_undo_cannot_both_own_the_source() {
        let directory = tempfile::tempdir().unwrap();
        let options = SqliteConnectOptions::new()
            .filename(directory.path().join("undo.sqlite"))
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(std::time::Duration::from_secs(3));
        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect_with(options)
            .await
            .unwrap();
        seed(&pool, "imap").await;
        let queue = SyncQueue::new();
        let token = trash(&pool, &queue).await;
        let (claimed, undone) =
            tokio::join!(queue.claim_next(&pool), undo_trash(&pool, &queue, &token));
        undone.expect("undo accepted");
        match claimed {
            Ok(Some(action)) => {
                let status: String =
                    sqlx::query_scalar("SELECT status FROM sync_queue WHERE id = ?")
                        .bind(action.id)
                        .fetch_one(&pool)
                        .await
                        .unwrap();
                assert_eq!(status, "sending");
                assert_eq!(
                    sqlx::query_scalar::<_, String>(
                        "SELECT state FROM mail_action_receipts WHERE id = ?"
                    )
                    .bind(token)
                    .fetch_one(&pool)
                    .await
                    .unwrap(),
                    "requested"
                );
            }
            Ok(None) => assert_restored(&pool).await,
            Err(error) if error.contains("database is locked") => assert_restored(&pool).await,
            other => panic!("unexpected claim result: {other:?}"),
        }
    }
}
