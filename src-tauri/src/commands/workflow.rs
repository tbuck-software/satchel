use serde::{Deserialize, Serialize};
use sqlx::{FromRow, Sqlite, SqlitePool, Transaction};

use crate::commands::error::CommandResult;
use crate::sync::{
    queue::QueuedAction,
    remote_mutations::{
        cancel_scheduled_workflow_moves_in_transaction, prepare_thread_workflow_moves,
        prepare_thread_workflow_moves_in_transaction, remote_move_thread_id_from_json,
        PreparedWorkflowMove,
    },
};
use crate::workflow::{engine::WorkflowEngine, states::WorkflowState};
use crate::AppState;

// The UI exposes Undo for nine seconds. Six seconds of backend margin prevent
// timer jitter or a briefly suspended mobile webview from racing the worker.
const WORKFLOW_UNDO_DELAY_SECONDS: i64 = 15;

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/workflow/", optional_fields = nullable))]
pub struct TransitionInput {
    pub thread_id: String,
    pub from: WorkflowState,
    pub to: WorkflowState,
    #[serde(default)]
    pub snooze_until: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/workflow/")
)]
pub struct TransitionDto {
    pub thread_id: String,
    pub from: WorkflowState,
    pub to: WorkflowState,
    pub at: String,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/workflow/", optional_fields = nullable))]
pub struct BatchTransitionEntry {
    pub thread_id: String,
    pub from: WorkflowState,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/workflow/", optional_fields = nullable))]
pub struct BatchTransitionInput {
    pub threads: Vec<BatchTransitionEntry>,
    pub to: WorkflowState,
    #[serde(default)]
    pub snooze_until: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/workflow/")
)]
pub struct FailedTransitionDto {
    pub thread_id: String,
    pub error: String,
}

#[derive(Debug, Clone, Default, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/workflow/")
)]
pub struct BatchTransitionDto {
    pub applied: Vec<String>,
    pub failed: Vec<FailedTransitionDto>,
}

#[derive(Debug, FromRow)]
struct TransitionRow {
    thread_id: String,
    from_state: String,
    to_state: String,
    created_at: String,
}

#[derive(Default)]
struct ProviderPlans {
    inbox_aside: Vec<PreparedWorkflowMove>,
    aside_inbox: Vec<PreparedWorkflowMove>,
    inbox_archive: Vec<PreparedWorkflowMove>,
    aside_archive: Vec<PreparedWorkflowMove>,
    archive_inbox: Vec<PreparedWorkflowMove>,
    archive_aside: Vec<PreparedWorkflowMove>,
}

async fn prepare_provider_plans(
    pool: &SqlitePool,
    input: &TransitionInput,
) -> Result<ProviderPlans, String> {
    let mut plans = ProviderPlans::default();
    match (input.from, input.to) {
        (WorkflowState::Done, WorkflowState::Aside) => {
            plans.archive_aside =
                prepare_thread_workflow_moves(pool, &input.thread_id, "archive", "aside").await?;
            // If the scheduled archive can still be cancelled, its provider
            // source is usually Inbox and this is the replacement operation.
            plans.inbox_aside =
                prepare_thread_workflow_moves(pool, &input.thread_id, "inbox", "aside").await?;
        }
        (WorkflowState::Done, _) => {
            plans.archive_inbox =
                prepare_thread_workflow_moves(pool, &input.thread_id, "archive", "inbox").await?;
            // A completed thread can originate in Aside. Cancelling its
            // pending archive leaves it there, so restoring must remove that
            // marker rather than assuming Inbox.
            plans.aside_inbox =
                prepare_thread_workflow_moves(pool, &input.thread_id, "aside", "inbox").await?;
        }
        (WorkflowState::Aside, WorkflowState::Done) => {
            plans.aside_archive =
                prepare_thread_workflow_moves(pool, &input.thread_id, "aside", "archive").await?;
            // When Aside itself is still undoable, cancelling it means the
            // provider copy never left Inbox.
            plans.inbox_archive =
                prepare_thread_workflow_moves(pool, &input.thread_id, "inbox", "archive").await?;
        }
        (WorkflowState::Aside, _) => {
            plans.aside_inbox =
                prepare_thread_workflow_moves(pool, &input.thread_id, "aside", "inbox").await?;
        }
        (_, WorkflowState::Aside) => {
            plans.inbox_aside =
                prepare_thread_workflow_moves(pool, &input.thread_id, "inbox", "aside").await?;
        }
        (_, WorkflowState::Done) => {
            plans.inbox_archive =
                prepare_thread_workflow_moves(pool, &input.thread_id, "inbox", "archive").await?;
        }
        _ => {}
    }
    Ok(plans)
}

async fn prepare_provider_plans_in_transaction(
    tx: &mut Transaction<'_, Sqlite>,
    input: &TransitionInput,
) -> Result<ProviderPlans, String> {
    let mut plans = ProviderPlans::default();
    match (input.from, input.to) {
        (WorkflowState::Done, WorkflowState::Aside) => {
            plans.archive_aside = prepare_thread_workflow_moves_in_transaction(
                tx,
                &input.thread_id,
                "archive",
                "aside",
            )
            .await?;
            plans.inbox_aside = prepare_thread_workflow_moves_in_transaction(
                tx,
                &input.thread_id,
                "inbox",
                "aside",
            )
            .await?;
        }
        (WorkflowState::Done, _) => {
            plans.archive_inbox = prepare_thread_workflow_moves_in_transaction(
                tx,
                &input.thread_id,
                "archive",
                "inbox",
            )
            .await?;
            plans.aside_inbox = prepare_thread_workflow_moves_in_transaction(
                tx,
                &input.thread_id,
                "aside",
                "inbox",
            )
            .await?;
        }
        (WorkflowState::Aside, WorkflowState::Done) => {
            plans.aside_archive = prepare_thread_workflow_moves_in_transaction(
                tx,
                &input.thread_id,
                "aside",
                "archive",
            )
            .await?;
            plans.inbox_archive = prepare_thread_workflow_moves_in_transaction(
                tx,
                &input.thread_id,
                "inbox",
                "archive",
            )
            .await?;
        }
        (WorkflowState::Aside, _) => {
            plans.aside_inbox = prepare_thread_workflow_moves_in_transaction(
                tx,
                &input.thread_id,
                "aside",
                "inbox",
            )
            .await?;
        }
        (_, WorkflowState::Aside) => {
            plans.inbox_aside = prepare_thread_workflow_moves_in_transaction(
                tx,
                &input.thread_id,
                "inbox",
                "aside",
            )
            .await?;
        }
        (_, WorkflowState::Done) => {
            plans.inbox_archive = prepare_thread_workflow_moves_in_transaction(
                tx,
                &input.thread_id,
                "inbox",
                "archive",
            )
            .await?;
        }
        _ => {}
    }
    Ok(plans)
}

/// Move a batch of threads in one round trip.
///
/// Each thread carries its own `from` state: a selection can span several
/// states, and the compare-and-swap below must reject exactly the threads that
/// moved underneath the client, not the whole batch. Failures are reported per
/// thread so the caller can offer Undo only for what actually changed.
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
    let mut result = BatchTransitionDto::default();
    for entry in input.threads {
        let single = TransitionInput {
            thread_id: entry.thread_id.clone(),
            from: entry.from,
            to: input.to,
            snooze_until: input.snooze_until.clone(),
        };
        match transition_thread_inner(pool, state, single).await {
            Ok(_) => result.applied.push(entry.thread_id),
            Err(error) => result.failed.push(FailedTransitionDto {
                thread_id: entry.thread_id,
                error,
            }),
        }
    }
    result
}

pub async fn transition_thread_inner(
    pool: &SqlitePool,
    state: &AppState,
    input: TransitionInput,
) -> Result<TransitionDto, String> {
    transition_thread_configured(pool, state, input, None, WORKFLOW_UNDO_DELAY_SECONDS)
        .await
        .map(|(transition, _)| transition)
}

#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub(crate) async fn transition_thread_for_notification(
    pool: &SqlitePool,
    state: &AppState,
    input: TransitionInput,
    queue_prefix: &str,
) -> Result<Vec<String>, String> {
    transition_thread_configured(pool, state, input, Some(queue_prefix), 0)
        .await
        .map(|(_, queued_ids)| queued_ids)
}

async fn transition_thread_configured(
    pool: &SqlitePool,
    state: &AppState,
    input: TransitionInput,
    queue_prefix: Option<&str>,
    undo_delay_seconds: i64,
) -> Result<(TransitionDto, Vec<String>), String> {
    let engine = WorkflowEngine;
    engine
        .validate_transition(input.from, input.to)
        .map_err(|err| err.to_string())?;

    // Provider planning may provision a missing local projection target (for
    // example Gmail's synthetic archive). The actual state CAS, queue mutation
    // and workflow event are still committed atomically below.
    // Preflight provisions any missing local target. Refs are deliberately
    // discarded and read again under the write reservation below.
    prepare_provider_plans(pool, &input).await?;
    let now = chrono::Utc::now().to_rfc3339();
    let scheduled_for = chrono::Utc::now() + chrono::Duration::seconds(undo_delay_seconds);
    let default_snooze;
    let snooze_until = if input.to == WorkflowState::Snoozed {
        match input
            .snooze_until
            .as_deref()
            .filter(|value| !value.trim().is_empty())
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

    // IMMEDIATE serializes this decision against the worker's queue claim.
    // Whichever side reserves the write lock first wins: Undo deletes a still
    // scheduled row, or it observes the already-claimed in-flight move.
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|err| err.to_string())?;
    if workflow_move_reconciliation_in_progress(&mut tx, &input.thread_id).await? {
        return Err("the server move is being finalized; try again shortly".to_string());
    }
    // Notification Done includes Read. Both intents must commit together,
    // with source-folder flags queued before the move changes the UID.
    let mut queued_ids = if queue_prefix.is_some() && input.to == WorkflowState::Done {
        satchel_core::mail_actions::set_flags_in_transaction(
            &mut tx,
            &state.queue,
            std::slice::from_ref(&input.thread_id),
            Some(true),
            None,
            queue_prefix,
        )
        .await?
    } else {
        Vec::new()
    };
    let plans = prepare_provider_plans_in_transaction(&mut tx, &input).await?;
    let changed = sqlx::query(
        "UPDATE threads
         SET workflow_state = ?, board_column = ?, workflow_changed_at = ?, snooze_until = ?, updated_at = ?
         WHERE id = ? AND workflow_state = ?",
    )
    .bind(input.to.as_db_value())
    .bind(input.to.as_db_value())
    .bind(&now)
    .bind(snooze_until)
    .bind(&now)
    .bind(&input.thread_id)
    .bind(input.from.as_db_value())
    .execute(&mut *tx)
    .await
    .map_err(|err| err.to_string())?
    .rows_affected();
    if changed != 1 {
        let current =
            sqlx::query_scalar::<_, String>("SELECT workflow_state FROM threads WHERE id = ?")
                .bind(&input.thread_id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(|err| err.to_string())?;
        return match current {
            Some(current) => Err(format!(
                "workflow state changed concurrently (expected {}, found {current})",
                input.from.as_db_value(),
            )),
            None => Err("thread not found".to_string()),
        };
    }

    let cancelled_aside = if input.from == WorkflowState::Aside {
        cancel_scheduled_workflow_moves_in_transaction(&mut tx, &input.thread_id, "aside").await?
    } else {
        Vec::new()
    };
    let cancelled_archive = if input.from == WorkflowState::Done {
        cancel_scheduled_workflow_moves_in_transaction(&mut tx, &input.thread_id, "archive").await?
    } else {
        Vec::new()
    };

    let selected = match (input.from, input.to) {
        (WorkflowState::Done, WorkflowState::Aside) if !cancelled_archive.is_empty() => {
            // If Done originated in Aside, cancelling Archive leaves the mail
            // exactly where the requested target already is.
            if cancelled_archive
                .iter()
                .any(|plan| plan.source_folder_type == "aside")
            {
                Vec::new()
            } else {
                plans.inbox_aside
            }
        }
        (WorkflowState::Done, WorkflowState::Aside) => plans.archive_aside,
        (WorkflowState::Done, _) if !cancelled_archive.is_empty() => {
            if cancelled_archive
                .iter()
                .any(|plan| plan.source_folder_type == "aside")
            {
                plans.aside_inbox
            } else {
                Vec::new()
            }
        }
        (WorkflowState::Done, _) => plans.archive_inbox,
        (WorkflowState::Aside, WorkflowState::Done) if !cancelled_aside.is_empty() => {
            plans.inbox_archive
        }
        (WorkflowState::Aside, WorkflowState::Done) => plans.aside_archive,
        (WorkflowState::Aside, _) if !cancelled_aside.is_empty() => Vec::new(),
        (WorkflowState::Aside, _) => plans.aside_inbox,
        (_, WorkflowState::Aside) => plans.inbox_aside,
        (_, WorkflowState::Done) => plans.inbox_archive,
        _ => Vec::new(),
    };

    for plan in selected {
        let account_id = plan.account_id.clone();
        let mut action: QueuedAction = plan.into_action(&input.thread_id, scheduled_for)?;
        if let Some(prefix) = queue_prefix {
            action.id = crate::sync::remote_mutations::stable_queue_id(prefix, "move", &account_id);
        }
        queued_ids.push(action.id.clone());
        state.queue.enqueue_in_transaction(&mut tx, &action).await?;
    }

    sqlx::query(
        "INSERT INTO workflow_events (id, thread_id, from_state, to_state, reason, created_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(queue_prefix.map_or_else(
        || format!("wf-{}", uuid::Uuid::new_v4()),
        |prefix| format!("{prefix}:workflow-event"),
    ))
    .bind(&input.thread_id)
    .bind(input.from.as_db_value())
    .bind(input.to.as_db_value())
    .bind("manual")
    .bind(&now)
    .execute(&mut *tx)
    .await
    .map_err(|err| err.to_string())?;

    tx.commit().await.map_err(|err| err.to_string())?;
    if !queued_ids.is_empty() {
        state.queue.wake_worker();
    }

    Ok((
        TransitionDto {
            thread_id: input.thread_id,
            from: input.from,
            to: input.to,
            at: now,
        },
        queued_ids,
    ))
}

async fn workflow_move_reconciliation_in_progress(
    tx: &mut Transaction<'_, Sqlite>,
    thread_id: &str,
) -> Result<bool, String> {
    let payloads = sqlx::query_scalar::<_, String>(
        "SELECT payload FROM sync_queue
         WHERE action_type = 'remote_mutation'
           AND status IN ('pending', 'sending', 'failed')",
    )
    .fetch_all(&mut **tx)
    .await
    .map_err(|err| err.to_string())?;
    Ok(payloads
        .into_iter()
        .filter_map(|payload| remote_move_thread_id_from_json(&payload))
        .any(|queued_thread_id| queued_thread_id == thread_id))
}

#[tauri::command]
pub async fn list_recent_transitions(
    pool: tauri::State<'_, crate::db::ReadPool>,
) -> CommandResult<Vec<TransitionDto>> {
    let rows = sqlx::query_as::<_, TransitionRow>(
        "SELECT thread_id, from_state, to_state, created_at
         FROM workflow_events
         ORDER BY created_at DESC
         LIMIT 50",
    )
    .fetch_all(pool.inner().as_ref())
    .await
    .map_err(|err| err.to_string())?;

    Ok(rows
        .into_iter()
        .map(|row| TransitionDto {
            thread_id: row.thread_id,
            from: WorkflowState::from_db_value(&row.from_state).unwrap_or(WorkflowState::Inbox),
            to: WorkflowState::from_db_value(&row.to_state).unwrap_or(WorkflowState::Inbox),
            at: row.created_at,
        })
        .collect())
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
    async fn reserved_plan_discards_refs_read_before_the_worker_finished() {
        let pool = seeded_pool().await;
        let input = TransitionInput {
            thread_id: "thread".into(),
            from: WorkflowState::Aside,
            to: WorkflowState::Inbox,
            snooze_until: None,
        };
        let stale = prepare_provider_plans(&pool, &input)
            .await
            .expect("preflight plan");
        assert!(stale.aside_inbox.is_empty());

        // This is the worker finishing Inbox -> Aside in the preflight window.
        sqlx::query("UPDATE messages SET folder_id = 'aside' WHERE id = 'message-inbox'")
            .execute(&pool)
            .await
            .expect("worker projection");
        let mut tx = pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .expect("transition reservation");
        let fresh = prepare_provider_plans_in_transaction(&mut tx, &input)
            .await
            .expect("reserved plan");
        assert_eq!(fresh.aside_inbox.len(), 1);
        assert_eq!(fresh.aside_inbox[0].refs[0].remote_id, "101");
        assert_eq!(
            fresh.aside_inbox[0].refs[0].source_folder.as_deref(),
            Some("Aside")
        );
        tx.rollback().await.expect("rollback test reservation");
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
