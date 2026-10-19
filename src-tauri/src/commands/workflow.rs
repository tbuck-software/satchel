use crate::commands::error::CommandResult;
#[cfg(test)]
use crate::workflow::states::WorkflowState;
use crate::AppState;
pub use satchel_core::workflow::transitions::{
    BatchTransitionDto, BatchTransitionEntry, BatchTransitionInput, FailedTransitionDto,
    TransitionDto, TransitionInput,
};
use sqlx::SqlitePool;

#[tauri::command]
pub async fn transition_threads(
    pool: tauri::State<'_, SqlitePool>,
    state: tauri::State<'_, AppState>,
    input: BatchTransitionInput,
) -> CommandResult<BatchTransitionDto> {
    Ok(transition_threads_inner(pool.inner(), state.inner(), input).await)
}

pub async fn transition_threads_inner(
    pool: &SqlitePool,
    state: &AppState,
    input: BatchTransitionInput,
) -> BatchTransitionDto {
    satchel_core::workflow::transitions::transition_threads_inner(pool, &state.queue, input).await
}

pub async fn transition_thread_inner(
    pool: &SqlitePool,
    state: &AppState,
    input: TransitionInput,
) -> Result<TransitionDto, String> {
    satchel_core::workflow::transitions::transition_thread_inner(pool, &state.queue, input).await
}

#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub(crate) async fn transition_thread_for_notification(
    pool: &SqlitePool,
    state: &AppState,
    input: TransitionInput,
    queue_prefix: &str,
) -> Result<Vec<String>, String> {
    satchel_core::workflow::transitions::transition_thread_for_notification(
        pool,
        &state.queue,
        input,
        queue_prefix,
    )
    .await
}

#[tauri::command]
pub async fn list_recent_transitions(
    pool: tauri::State<'_, crate::db::ReadPool>,
) -> CommandResult<Vec<TransitionDto>> {
    satchel_core::workflow::transitions::list_recent_transitions(pool.inner().as_ref())
        .await
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::imap_workflow_target::{
        finalize_imap_workflow_target_after_uidvalidity_change,
        finalize_imap_workflow_target_projection,
        finalize_persisted_imap_workflow_target_projection,
        finalize_persisted_imap_workflow_without_target_projection,
    };
    use crate::sync::remote_mutations::RemoteMutationPayload;
    use sqlx::sqlite::SqlitePoolOptions;
    use std::collections::HashSet;

    async fn seeded_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
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
             VALUES ('account', 'Me', 'me@example.com', 'imap', '{}', '', ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("account");
        for (id, name, kind) in [
            ("inbox", "INBOX", "inbox"),
            ("sent", "Sent", "sent"),
            ("drafts", "Drafts", "drafts"),
            ("aside", "Aside", "aside"),
            ("archive", "Archive", "archive"),
        ] {
            sqlx::query(
                "INSERT INTO folders
                 (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at)
                 VALUES (?, 'account', ?, ?, ?, ?, ?, ?)",
            )
            .bind(id)
            .bind(name)
            .bind(name)
            .bind(kind)
            .bind(name)
            .bind(&now)
            .bind(&now)
            .execute(&pool)
            .await
            .expect("folder");
        }
        sqlx::query(
            "INSERT INTO threads
             (id, account_id, subject, last_message_date, workflow_state, board_column, created_at, updated_at)
             VALUES ('thread', 'account', 'Subject', ?, 'inbox', 'inbox', ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("thread");
        for (id, folder, remote) in [
            ("message-inbox", "inbox", "101"),
            ("message-sent", "sent", "202"),
            ("message-draft", "drafts", "303"),
        ] {
            sqlx::query(
                "INSERT INTO messages
                 (id, account_id, folder_id, thread_id, from_address, to_addresses, date, remote_id, created_at, updated_at)
                 VALUES (?, 'account', ?, 'thread', 'sender@example.com', '[]', ?, ?, ?, ?)",
            )
            .bind(id)
            .bind(folder)
            .bind(&now)
            .bind(remote)
            .bind(&now)
            .bind(&now)
            .execute(&pool)
            .await
            .expect("message");
        }
        pool
    }

    #[tokio::test]
    async fn trash_receipt_follows_the_exact_imap_projection_and_reverses_the_new_uid() {
        let pool = seeded_pool().await;
        let state = AppState::default();
        sqlx::query("DELETE FROM messages WHERE id != 'message-inbox'")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("UPDATE messages SET message_id_header = '<trash@example.test>' WHERE id = 'message-inbox'").execute(&pool).await.unwrap();
        let receipt =
            satchel_core::mail_actions::trash_threads(&pool, &state.queue, &["thread".into()])
                .await
                .expect("trash receipt");
        let forward = state.queue.claim_next(&pool).await.unwrap().unwrap();
        let refs = match serde_json::from_value::<RemoteMutationPayload>(forward.payload.clone())
            .unwrap()
        {
            RemoteMutationPayload::WorkflowMove { refs, .. } => refs,
            other => panic!("unexpected payload {other:?}"),
        };
        let target: String =
            sqlx::query_scalar("SELECT id FROM folders WHERE folder_type = 'trash'")
                .fetch_one(&pool)
                .await
                .unwrap();
        sqlx::query("INSERT INTO messages (id, account_id, folder_id, thread_id, from_address, to_addresses, date, remote_id, message_id_header, created_at, updated_at) SELECT 'trash-delivery', account_id, ?, thread_id, from_address, to_addresses, date, '909', message_id_header, created_at, updated_at FROM messages WHERE id = 'message-inbox'")
            .bind(&target).execute(&pool).await.unwrap();
        for _ in 0..2 {
            finalize_persisted_imap_workflow_target_projection(
                &pool,
                "account",
                "thread",
                &refs,
                &target,
                Some(&HashSet::from(["909".into()])),
                &forward.id,
                &forward.payload,
            )
            .await
            .expect("idempotent persisted reconciliation");
        }
        state
            .queue
            .mark_completed(&pool, &forward.id)
            .await
            .unwrap();
        satchel_core::mail_actions::undo::undo_trash(
            &pool,
            &state.queue,
            receipt.undo_token.as_deref().unwrap(),
        )
        .await
        .expect("undo completed IMAP trash");
        let reverse = state.queue.claim_next(&pool).await.unwrap().unwrap();
        assert_eq!(reverse.payload["refs"][0]["remote_id"], "909");
        assert_eq!(reverse.payload["refs"][0]["source_folder"], "Trash");
        assert_eq!(reverse.payload["target_remote_folder_id"], "INBOX");
        let source_copies: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE id = 'message-inbox'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(source_copies, 0);
    }

    #[tokio::test]
    async fn notification_archive_queues_read_before_move_atomically() {
        let pool = seeded_pool().await;
        let state = AppState::default();
        let ids = transition_thread_for_notification(
            &pool,
            &state,
            TransitionInput {
                thread_id: "thread".into(),
                from: WorkflowState::Inbox,
                to: WorkflowState::Done,
                snooze_until: None,
            },
            "notification:test",
        )
        .await
        .unwrap();
        let unread: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM messages WHERE thread_id = 'thread' AND is_read = 0",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            unread, 0,
            "Done must also mark the notification thread read"
        );
        let mut kinds = Vec::new();
        for id in ids {
            let raw: String = sqlx::query_scalar("SELECT payload FROM sync_queue WHERE id = ?")
                .bind(id)
                .fetch_one(&pool)
                .await
                .unwrap();
            let payload: serde_json::Value = serde_json::from_str(&raw).unwrap();
            kinds.push(payload["kind"].as_str().unwrap().to_string());
            if payload["kind"] == "flags" {
                assert!(payload["refs"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|r| r["remote_id"] == "101" && r["source_folder"] == "INBOX"));
            }
        }
        assert_eq!(kinds, vec!["flags", "workflow_move"]);
    }

    #[tokio::test]
    async fn notification_archive_rolls_back_read_when_move_cannot_be_queued() {
        let pool = seeded_pool().await;
        sqlx::query("CREATE TRIGGER reject_move BEFORE INSERT ON sync_queue WHEN json_extract(NEW.payload, '$.kind') = 'workflow_move' BEGIN SELECT RAISE(ABORT, 'queue unavailable'); END")
            .execute(&pool).await.unwrap();
        assert!(transition_thread_for_notification(
            &pool,
            &AppState::default(),
            TransitionInput {
                thread_id: "thread".into(),
                from: WorkflowState::Inbox,
                to: WorkflowState::Done,
                snooze_until: None,
            },
            "notification:test"
        )
        .await
        .is_err());
        let read: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE is_read = 1")
            .fetch_one(&pool)
            .await
            .unwrap();
        let queued: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_queue")
            .fetch_one(&pool)
            .await
            .unwrap();
        let state: String =
            sqlx::query_scalar("SELECT workflow_state FROM threads WHERE id = 'thread'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!((read, queued, state.as_str()), (0, 0, "inbox"));
    }

    #[tokio::test]
    async fn batch_transition_reports_applied_and_rejected_threads() {
        let pool = seeded_pool().await;
        let state = AppState::default();
        let now = chrono::Utc::now().to_rfc3339();
        for id in ["thread-b", "thread-c"] {
            sqlx::query(
                "INSERT INTO threads
                 (id, account_id, subject, last_message_date, workflow_state, board_column, created_at, updated_at)
                 VALUES (?, 'account', 'Subject', ?, 'inbox', 'inbox', ?, ?)",
            )
            .bind(id)
            .bind(&now)
            .bind(&now)
            .bind(&now)
            .execute(&pool)
            .await
            .expect("thread");
        }
        // Stale client belief: this row is no longer in the inbox.
        sqlx::query("UPDATE threads SET workflow_state = 'todo' WHERE id = 'thread-c'")
            .execute(&pool)
            .await
            .expect("diverge");

        let result = transition_threads_inner(
            &pool,
            &state,
            BatchTransitionInput {
                threads: ["thread", "thread-b", "thread-c"]
                    .into_iter()
                    .map(|thread_id| BatchTransitionEntry {
                        thread_id: thread_id.into(),
                        from: WorkflowState::Inbox,
                    })
                    .collect(),
                to: WorkflowState::Done,
                snooze_until: None,
            },
        )
        .await;

        assert_eq!(result.applied, vec!["thread", "thread-b"]);
        assert_eq!(result.failed.len(), 1);
        assert_eq!(result.failed[0].thread_id, "thread-c");
        assert!(
            result.failed[0].error.contains("changed concurrently"),
            "unexpected error: {}",
            result.failed[0].error
        );

        let states: Vec<(String, String)> =
            sqlx::query_as("SELECT id, workflow_state FROM threads ORDER BY id")
                .fetch_all(&pool)
                .await
                .expect("states");
        assert_eq!(
            states,
            vec![
                ("thread".to_string(), "done".to_string()),
                ("thread-b".to_string(), "done".to_string()),
                ("thread-c".to_string(), "todo".to_string()),
            ]
        );
    }

    #[tokio::test]
    async fn missing_thread_is_reported_separately_from_a_concurrent_state_change() {
        let pool = seeded_pool().await;
        let state = AppState::default();
        sqlx::query("DELETE FROM messages WHERE thread_id = 'thread'")
            .execute(&pool)
            .await
            .expect("delete messages");
        sqlx::query("DELETE FROM threads WHERE id = 'thread'")
            .execute(&pool)
            .await
            .expect("delete thread");

        let error = transition_thread_inner(
            &pool,
            &state,
            TransitionInput {
                thread_id: "thread".into(),
                from: WorkflowState::Inbox,
                to: WorkflowState::Done,
                snooze_until: None,
            },
        )
        .await
        .expect_err("missing thread must fail");

        assert_eq!(error, "thread not found");
    }

    #[tokio::test]
    async fn aside_queues_only_the_inbox_copy_and_undo_cancels_it() {
        let pool = seeded_pool().await;
        let state = AppState::default();
        transition_thread_inner(
            &pool,
            &state,
            TransitionInput {
                thread_id: "thread".into(),
                from: WorkflowState::Inbox,
                to: WorkflowState::Aside,
                snooze_until: None,
            },
        )
        .await
        .expect("set aside");

        let payload: String =
            sqlx::query_scalar("SELECT payload FROM sync_queue WHERE status = 'scheduled'")
                .fetch_one(&pool)
                .await
                .expect("queued move");
        let mutation: RemoteMutationPayload = serde_json::from_str(&payload).expect("payload");
        match mutation {
            RemoteMutationPayload::WorkflowMove {
                workflow_thread_id,
                source_folder_type,
                target_folder_type,
                refs,
                provider_move_completed,
                ..
            } => {
                assert_eq!(workflow_thread_id, "thread");
                assert_eq!(source_folder_type, "inbox");
                assert_eq!(target_folder_type, "aside");
                assert_eq!(refs.len(), 1);
                assert_eq!(refs[0].remote_id, "101");
                assert_eq!(refs[0].source_folder.as_deref(), Some("INBOX"));
                assert!(!provider_move_completed);
            }
            other => panic!("unexpected payload: {other:?}"),
        }

        transition_thread_inner(
            &pool,
            &state,
            TransitionInput {
                thread_id: "thread".into(),
                from: WorkflowState::Aside,
                to: WorkflowState::Inbox,
                snooze_until: None,
            },
        )
        .await
        .expect("undo aside");
        let queued: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_queue")
            .fetch_one(&pool)
            .await
            .expect("queue count");
        assert_eq!(queued, 0);
    }

    #[tokio::test]
    async fn done_undo_cancels_the_provider_archive() {
        let pool = seeded_pool().await;
        let state = AppState::default();
        transition_thread_inner(
            &pool,
            &state,
            TransitionInput {
                thread_id: "thread".into(),
                from: WorkflowState::Inbox,
                to: WorkflowState::Done,
                snooze_until: None,
            },
        )
        .await
        .expect("done");
        transition_thread_inner(
            &pool,
            &state,
            TransitionInput {
                thread_id: "thread".into(),
                from: WorkflowState::Done,
                to: WorkflowState::Inbox,
                snooze_until: None,
            },
        )
        .await
        .expect("undo done");
        let result: (String, i64, i64) = sqlx::query_as(
            "SELECT workflow_state,
                    (SELECT COUNT(*) FROM sync_queue),
                    (SELECT COUNT(*) FROM workflow_events)
             FROM threads WHERE id = 'thread'",
        )
        .fetch_one(&pool)
        .await
        .expect("result");
        assert_eq!(result, ("inbox".to_string(), 0, 2));
    }

    #[tokio::test]
    async fn completed_provider_move_can_be_reversed_before_a_regular_sync() {
        let pool = seeded_pool().await;
        let state = AppState::default();
        transition_thread_inner(
            &pool,
            &state,
            TransitionInput {
                thread_id: "thread".into(),
                from: WorkflowState::Inbox,
                to: WorkflowState::Aside,
                snooze_until: None,
            },
        )
        .await
        .expect("set aside");

        let payload: String = sqlx::query_scalar("SELECT payload FROM sync_queue LIMIT 1")
            .fetch_one(&pool)
            .await
            .expect("outbound move");
        let refs = match serde_json::from_str::<RemoteMutationPayload>(&payload).expect("payload") {
            RemoteMutationPayload::WorkflowMove { refs, .. } => refs,
            other => panic!("unexpected payload: {other:?}"),
        };
        crate::sync::remote_mutations::apply_completed_workflow_move_projection(
            &pool, "account", &refs, "aside",
        )
        .await
        .expect("completed projection");
        sqlx::query("DELETE FROM sync_queue")
            .execute(&pool)
            .await
            .expect("simulate completed queue row");

        transition_thread_inner(
            &pool,
            &state,
            TransitionInput {
                thread_id: "thread".into(),
                from: WorkflowState::Aside,
                to: WorkflowState::Inbox,
                snooze_until: None,
            },
        )
        .await
        .expect("return to inbox");

        let reverse_payload: String = sqlx::query_scalar("SELECT payload FROM sync_queue LIMIT 1")
            .fetch_one(&pool)
            .await
            .expect("reverse move");
        match serde_json::from_str::<RemoteMutationPayload>(&reverse_payload)
            .expect("reverse payload")
        {
            RemoteMutationPayload::WorkflowMove {
                source_folder_type,
                target_folder_type,
                refs,
                ..
            } => {
                assert_eq!(source_folder_type, "aside");
                assert_eq!(target_folder_type, "inbox");
                assert_eq!(refs.len(), 1);
                assert_eq!(refs[0].remote_id, "101");
                assert_eq!(refs[0].source_folder.as_deref(), Some("Aside"));
            }
            other => panic!("unexpected payload: {other:?}"),
        }
    }

    #[tokio::test]
    async fn reverse_transition_waits_while_provider_completion_is_reconciling() {
        let pool = seeded_pool().await;
        let state = AppState::default();
        sqlx::query(
            "UPDATE threads SET workflow_state = 'aside', board_column = 'aside'
             WHERE id = 'thread'",
        )
        .execute(&pool)
        .await
        .expect("aside projection");
        let payload = serde_json::to_string(&RemoteMutationPayload::WorkflowMove {
            workflow_thread_id: "thread".into(),
            source_folder_type: "inbox".into(),
            target_folder_type: "aside".into(),
            refs: vec![crate::providers::traits::MessageRef {
                remote_id: "101".into(),
                source_folder: Some("INBOX".into()),
                source_uidvalidity: None,
            }],
            target_remote_folder_id: "Aside".into(),
            provider_move_started: true,
            source_uidvalidity_before: Some(std::collections::HashMap::from([("INBOX".into(), 1)])),
            provider_move_completed: true,
            target_remote_ids_before: Some(Vec::new()),
            target_uidvalidity_before: Some(1),
            projection_completed: false,
        })
        .expect("payload");
        sqlx::query(
            "INSERT INTO sync_queue
             (id, account_id, action_type, payload, status, retry_count, created_at, updated_at)
             VALUES ('move', 'account', 'remote_mutation', ?, 'pending', 0, 'now', 'now')",
        )
        .bind(payload)
        .execute(&pool)
        .await
        .expect("in-flight move");

        let error = transition_thread_inner(
            &pool,
            &state,
            TransitionInput {
                thread_id: "thread".into(),
                from: WorkflowState::Aside,
                to: WorkflowState::Inbox,
                snooze_until: None,
            },
        )
        .await
        .expect_err("reverse must wait for a valid destination UID");
        assert!(error.contains("being finalized"));
        let state_value: String =
            sqlx::query_scalar("SELECT workflow_state FROM threads WHERE id = 'thread'")
                .fetch_one(&pool)
                .await
                .expect("state");
        assert_eq!(state_value, "aside");

        sqlx::query("UPDATE sync_queue SET status = 'failed' WHERE id = 'move'")
            .execute(&pool)
            .await
            .expect("permanent reconciliation failure");
        transition_thread_inner(
            &pool,
            &state,
            TransitionInput {
                thread_id: "thread".into(),
                from: WorkflowState::Aside,
                to: WorkflowState::Inbox,
                snooze_until: None,
            },
        )
        .await
        .expect_err("failed reconciliation must remain authoritative");

        sqlx::query("DELETE FROM sync_queue")
            .execute(&pool)
            .await
            .expect("workflow queue cleanup");
        for (index, status) in ["pending", "sending", "failed"].into_iter().enumerate() {
            let blocked_payload = serde_json::to_string(&RemoteMutationPayload::BlockedMove {
                blocked_thread_id: "thread".into(),
                refs: vec![crate::providers::traits::MessageRef {
                    remote_id: "101".into(),
                    source_folder: Some("INBOX".into()),
                    source_uidvalidity: None,
                }],
                target_remote_folder_id: "Spam".into(),
                provider_move_started: true,
                provider_move_completed: false,
                source_uidvalidity_before: None,
            })
            .expect("blocked move payload");
            sqlx::query(
                "INSERT INTO sync_queue
                 (id, account_id, action_type, payload, status, retry_count, created_at, updated_at)
                 VALUES (?, 'account', 'remote_mutation', ?, ?, 0, 'now', 'now')",
            )
            .bind(format!("blocked-move-{index}"))
            .bind(blocked_payload)
            .bind(status)
            .execute(&pool)
            .await
            .expect("blocked move");
            transition_thread_inner(
                &pool,
                &state,
                TransitionInput {
                    thread_id: "thread".into(),
                    from: WorkflowState::Aside,
                    to: WorkflowState::Inbox,
                    snooze_until: None,
                },
            )
            .await
            .expect_err("blocked move reconciliation must remain authoritative");
            sqlx::query("DELETE FROM sync_queue")
                .execute(&pool)
                .await
                .expect("blocked move cleanup");
        }
    }

    #[tokio::test]
    async fn done_prepares_a_local_archive_target_when_sync_has_not_created_one() {
        let pool = seeded_pool().await;
        sqlx::query("DELETE FROM folders WHERE folder_type = 'archive'")
            .execute(&pool)
            .await
            .expect("remove archive folder");
        let state = AppState::default();

        transition_thread_inner(
            &pool,
            &state,
            TransitionInput {
                thread_id: "thread".into(),
                from: WorkflowState::Inbox,
                to: WorkflowState::Done,
                snooze_until: None,
            },
        )
        .await
        .expect("done");

        let target: (String, String) = sqlx::query_as(
            "SELECT folder_type, remote_id FROM folders
             WHERE account_id = 'account' AND folder_type = 'archive'",
        )
        .fetch_one(&pool)
        .await
        .expect("local archive target");
        assert_eq!(target, ("archive".into(), "Archive".into()));
        let queued: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_queue")
            .fetch_one(&pool)
            .await
            .expect("queued move");
        assert_eq!(queued, 1);
    }

    #[tokio::test]
    async fn imap_done_reverses_with_the_destination_uid_after_exact_archive_reconciliation() {
        let pool = seeded_pool().await;
        let state = AppState::default();
        transition_thread_inner(
            &pool,
            &state,
            TransitionInput {
                thread_id: "thread".into(),
                from: WorkflowState::Inbox,
                to: WorkflowState::Done,
                snooze_until: None,
            },
        )
        .await
        .expect("done");
        let payload: String = sqlx::query_scalar("SELECT payload FROM sync_queue LIMIT 1")
            .fetch_one(&pool)
            .await
            .expect("archive move");
        let refs = match serde_json::from_str::<RemoteMutationPayload>(&payload).expect("payload") {
            RemoteMutationPayload::WorkflowMove { refs, .. } => refs,
            other => panic!("unexpected payload: {other:?}"),
        };
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "UPDATE messages
             SET message_id_header = '<moved@example.com>', subject = 'Subject', raw_size = 42
             WHERE id = 'message-inbox'",
        )
        .execute(&pool)
        .await
        .expect("source identity");
        sqlx::query(
            "INSERT INTO messages
             (id, account_id, folder_id, thread_id, from_address, to_addresses, date,
              remote_id, message_id_header, subject, raw_size, created_at, updated_at)
             VALUES ('message-archive-new', 'account', 'archive', 'thread',
                     'sender@example.com', '[]', ?, '909', '<moved@example.com>',
                     'Subject', 42, ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("destination UID");
        finalize_imap_workflow_target_projection(
            &pool,
            "account",
            "thread",
            &refs,
            "archive",
            &HashSet::from(["909".to_string()]),
        )
        .await
        .expect("exact archive reconciliation");
        sqlx::query("DELETE FROM sync_queue")
            .execute(&pool)
            .await
            .expect("completed move");

        transition_thread_inner(
            &pool,
            &state,
            TransitionInput {
                thread_id: "thread".into(),
                from: WorkflowState::Done,
                to: WorkflowState::Inbox,
                snooze_until: None,
            },
        )
        .await
        .expect("restore inbox");
        let payload: String = sqlx::query_scalar("SELECT payload FROM sync_queue LIMIT 1")
            .fetch_one(&pool)
            .await
            .expect("reverse move");
        match serde_json::from_str::<RemoteMutationPayload>(&payload).expect("payload") {
            RemoteMutationPayload::WorkflowMove { refs, .. } => {
                assert_eq!(refs.len(), 1);
                assert_eq!(refs[0].remote_id, "909");
                assert_eq!(refs[0].source_folder.as_deref(), Some("Archive"));
            }
            other => panic!("unexpected payload: {other:?}"),
        }
    }

    #[tokio::test]
    async fn imap_projection_finalize_is_idempotent_after_atomic_commit() {
        let pool = seeded_pool().await;
        let state = AppState::default();
        transition_thread_inner(
            &pool,
            &state,
            TransitionInput {
                thread_id: "thread".into(),
                from: WorkflowState::Inbox,
                to: WorkflowState::Done,
                snooze_until: None,
            },
        )
        .await
        .expect("done");
        let (action_id, payload): (String, String) =
            sqlx::query_as("SELECT id, payload FROM sync_queue LIMIT 1")
                .fetch_one(&pool)
                .await
                .expect("archive move");
        let refs = match serde_json::from_str::<RemoteMutationPayload>(&payload).expect("payload") {
            RemoteMutationPayload::WorkflowMove { refs, .. } => refs,
            other => panic!("unexpected payload: {other:?}"),
        };
        let mut completed_payload =
            serde_json::from_str::<serde_json::Value>(&payload).expect("JSON payload");
        completed_payload["provider_move_completed"] = true.into();
        sqlx::query("UPDATE sync_queue SET status = 'sending', payload = ? WHERE id = ?")
            .bind(completed_payload.to_string())
            .bind(&action_id)
            .execute(&pool)
            .await
            .expect("claim completed move");

        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "UPDATE messages
             SET message_id_header = '<moved@example.com>', subject = 'Subject', raw_size = 42
             WHERE id = 'message-inbox'",
        )
        .execute(&pool)
        .await
        .expect("source identity");
        sqlx::query(
            "INSERT INTO messages
             (id, account_id, folder_id, thread_id, from_address, to_addresses, date,
              remote_id, message_id_header, subject, raw_size, created_at, updated_at)
             VALUES ('message-archive-new', 'account', 'archive', 'thread',
                     'sender@example.com', '[]', ?, '909', '<moved@example.com>',
                     'Subject', 42, ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("destination UID");

        for _ in 0..2 {
            finalize_persisted_imap_workflow_target_projection(
                &pool,
                "account",
                "thread",
                &refs,
                "archive",
                Some(&HashSet::from(["909".to_string()])),
                &action_id,
                &completed_payload,
            )
            .await
            .expect("idempotent finalize");
        }

        let source_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE id = 'message-inbox'")
                .fetch_one(&pool)
                .await
                .expect("source count");
        assert_eq!(source_count, 0);
        let stored: String = sqlx::query_scalar("SELECT payload FROM sync_queue WHERE id = ?")
            .bind(&action_id)
            .fetch_one(&pool)
            .await
            .expect("stored payload");
        let stored: serde_json::Value = serde_json::from_str(&stored).expect("stored JSON");
        assert_eq!(stored["projection_completed"], true);
    }

    #[tokio::test]
    async fn accepted_imap_move_can_finalize_when_the_source_is_gone() {
        let pool = seeded_pool().await;
        let state = AppState::default();
        transition_thread_inner(
            &pool,
            &state,
            TransitionInput {
                thread_id: "thread".into(),
                from: WorkflowState::Inbox,
                to: WorkflowState::Done,
                snooze_until: None,
            },
        )
        .await
        .expect("done");
        let (action_id, payload): (String, String) =
            sqlx::query_as("SELECT id, payload FROM sync_queue LIMIT 1")
                .fetch_one(&pool)
                .await
                .expect("archive move");
        let refs = match serde_json::from_str::<RemoteMutationPayload>(&payload).expect("payload") {
            RemoteMutationPayload::WorkflowMove { refs, .. } => refs,
            other => panic!("unexpected payload: {other:?}"),
        };
        let mut completed_payload =
            serde_json::from_str::<serde_json::Value>(&payload).expect("JSON payload");
        completed_payload["provider_move_completed"] = true.into();
        sqlx::query("UPDATE sync_queue SET status = 'sending', payload = ? WHERE id = ?")
            .bind(completed_payload.to_string())
            .bind(&action_id)
            .execute(&pool)
            .await
            .expect("claim completed move");

        finalize_persisted_imap_workflow_without_target_projection(
            &pool,
            "account",
            "thread",
            &refs,
            &action_id,
            &completed_payload,
        )
        .await
        .expect("source-absence fallback");

        let source_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE id = 'message-inbox'")
                .fetch_one(&pool)
                .await
                .expect("source count");
        assert_eq!(source_count, 0);
        let stored: String = sqlx::query_scalar("SELECT payload FROM sync_queue WHERE id = ?")
            .bind(&action_id)
            .fetch_one(&pool)
            .await
            .expect("stored payload");
        let stored: serde_json::Value = serde_json::from_str(&stored).expect("stored JSON");
        assert_eq!(stored["projection_completed"], true);
    }

    #[tokio::test]
    async fn imap_completion_keeps_source_when_matching_target_copy_predates_move() {
        let pool = seeded_pool().await;
        sqlx::query(
            "UPDATE messages
             SET message_id_header = '<moved@example.com>', subject = 'Subject', raw_size = 42
             WHERE id = 'message-inbox'",
        )
        .execute(&pool)
        .await
        .expect("source identity");
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO messages
             (id, account_id, folder_id, thread_id, from_address, to_addresses, date,
              remote_id, message_id_header, subject, raw_size, created_at, updated_at)
             VALUES ('older-archive-copy', 'account', 'archive', 'thread',
                     'sender@example.com', '[]', ?, '808', '<moved@example.com>',
                     'Subject', 42, ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("preexisting target copy");
        let refs = vec![crate::providers::traits::MessageRef {
            remote_id: "101".into(),
            source_folder: Some("INBOX".into()),
            source_uidvalidity: None,
        }];

        let error = finalize_imap_workflow_target_projection(
            &pool,
            "account",
            "thread",
            &refs,
            "archive",
            &HashSet::new(),
        )
        .await
        .expect_err("preexisting target must not prove the move");
        assert!(error.contains("did not find moved message"));
        let source_exists: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE id = 'message-inbox'")
                .fetch_one(&pool)
                .await
                .expect("source count");
        assert_eq!(source_exists, 1);
    }

    #[tokio::test]
    async fn imap_uidvalidity_change_requires_an_unambiguous_identity_match() {
        let pool = seeded_pool().await;
        sqlx::query(
            "UPDATE messages
             SET message_id_header = '<moved@example.com>', subject = 'Subject', raw_size = 42
             WHERE id = 'message-inbox'",
        )
        .execute(&pool)
        .await
        .expect("source identity");
        let now = chrono::Utc::now().to_rfc3339();
        for (id, uid) in [("archive-old-generation", "808"), ("archive-moved", "909")] {
            sqlx::query(
                "INSERT INTO messages
                 (id, account_id, folder_id, thread_id, from_address, to_addresses, date,
                  remote_id, message_id_header, subject, raw_size, created_at, updated_at)
                 VALUES (?, 'account', 'archive', 'thread', 'sender@example.com', '[]', ?, ?,
                         '<moved@example.com>', 'Subject', 42, ?, ?)",
            )
            .bind(id)
            .bind(&now)
            .bind(uid)
            .bind(&now)
            .bind(&now)
            .execute(&pool)
            .await
            .expect("target identity copy");
        }
        let refs = vec![crate::providers::traits::MessageRef {
            remote_id: "101".into(),
            source_folder: Some("INBOX".into()),
            source_uidvalidity: None,
        }];

        let error = finalize_imap_workflow_target_after_uidvalidity_change(
            &pool, "account", "thread", &refs, "archive",
        )
        .await
        .expect_err("two generation candidates are ambiguous");
        assert!(error.contains("ambiguous"));
        sqlx::query("DELETE FROM messages WHERE id = 'archive-old-generation'")
            .execute(&pool)
            .await
            .expect("reset removes old generation");

        finalize_imap_workflow_target_after_uidvalidity_change(
            &pool, "account", "thread", &refs, "archive",
        )
        .await
        .expect("one current-generation identity is safe");
        let source_exists: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE id = 'message-inbox'")
                .fetch_one(&pool)
                .await
                .expect("source count");
        assert_eq!(source_exists, 0);
    }

    #[tokio::test]
    async fn imap_completion_falls_back_for_synthetic_ids_and_matches_distinct_copies() {
        let pool = seeded_pool().await;
        let source_date: String =
            sqlx::query_scalar("SELECT date FROM messages WHERE id = 'message-inbox'")
                .fetch_one(&pool)
                .await
                .expect("source date");
        sqlx::query(
            "UPDATE messages
             SET message_id_header = '<INBOX-101@imap.satchel.local>',
                 subject = 'No native id', raw_size = 77
             WHERE id = 'message-inbox'",
        )
        .execute(&pool)
        .await
        .expect("first source identity");
        sqlx::query(
            "INSERT INTO messages
             (id, account_id, folder_id, thread_id, from_address, to_addresses, date,
              remote_id, message_id_header, subject, raw_size, created_at, updated_at)
             VALUES ('message-inbox-2', 'account', 'inbox', 'thread',
                     'sender@example.com', '[]', ?, '102',
                     '<INBOX-102@imap.satchel.local>', 'No native id', 77, ?, ?)",
        )
        .bind(&source_date)
        .bind(&source_date)
        .bind(&source_date)
        .execute(&pool)
        .await
        .expect("second source");
        for (id, uid) in [("target-1", "909"), ("target-2", "910")] {
            sqlx::query(
                "INSERT INTO messages
                 (id, account_id, folder_id, thread_id, from_address, to_addresses, date,
                  remote_id, message_id_header, subject, raw_size, created_at, updated_at)
                 VALUES (?, 'account', 'archive', 'thread', 'sender@example.com', '[]', ?, ?,
                         '<Archive-new@imap.satchel.local>', 'No native id', 77, ?, ?)",
            )
            .bind(id)
            .bind(&source_date)
            .bind(uid)
            .bind(&source_date)
            .bind(&source_date)
            .execute(&pool)
            .await
            .expect("target copy");
        }
        let refs = vec![
            crate::providers::traits::MessageRef {
                remote_id: "101".into(),
                source_folder: Some("INBOX".into()),
                source_uidvalidity: None,
            },
            crate::providers::traits::MessageRef {
                remote_id: "102".into(),
                source_folder: Some("INBOX".into()),
                source_uidvalidity: None,
            },
        ];

        finalize_imap_workflow_target_projection(
            &pool,
            "account",
            "thread",
            &refs,
            "archive",
            &HashSet::from(["909".to_string(), "910".to_string()]),
        )
        .await
        .expect("synthetic-id fallback");
        let source_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM messages WHERE id IN ('message-inbox', 'message-inbox-2')",
        )
        .fetch_one(&pool)
        .await
        .expect("source count");
        assert_eq!(source_count, 0);
    }

    #[tokio::test]
    async fn stale_compare_and_swap_writes_neither_queue_nor_event() {
        let pool = seeded_pool().await;
        let state = AppState::default();
        let error = transition_thread_inner(
            &pool,
            &state,
            TransitionInput {
                thread_id: "thread".into(),
                from: WorkflowState::Todo,
                to: WorkflowState::Aside,
                snooze_until: None,
            },
        )
        .await
        .expect_err("stale transition must fail");
        assert!(error.contains("concurrently"));
        let result: (String, i64, i64) = sqlx::query_as(
            "SELECT workflow_state,
                    (SELECT COUNT(*) FROM sync_queue),
                    (SELECT COUNT(*) FROM workflow_events)
             FROM threads WHERE id = 'thread'",
        )
        .fetch_one(&pool)
        .await
        .expect("result");
        assert_eq!(result, ("inbox".to_string(), 0, 0));
    }
}
