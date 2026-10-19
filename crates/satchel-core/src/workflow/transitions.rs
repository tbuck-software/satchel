use serde::{Deserialize, Serialize};
use sqlx::{FromRow, Sqlite, SqlitePool, Transaction};

use crate::sync::queue::SyncQueue;
use crate::sync::{
    queue::QueuedAction,
    remote_mutations::{
        cancel_scheduled_workflow_moves_in_transaction, prepare_thread_workflow_moves,
        prepare_thread_workflow_moves_in_transaction, remote_move_thread_id_from_json,
        PreparedWorkflowMove,
    },
};
use crate::workflow::{engine::WorkflowEngine, states::WorkflowState};

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

pub async fn transition_threads_inner(
    pool: &SqlitePool,
    queue: &SyncQueue,
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
        match transition_thread_inner(pool, queue, single).await {
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
    queue: &SyncQueue,
    input: TransitionInput,
) -> Result<TransitionDto, String> {
    transition_thread_configured(pool, queue, input, None, WORKFLOW_UNDO_DELAY_SECONDS)
        .await
        .map(|(transition, _)| transition)
}

#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub async fn transition_thread_for_notification(
    pool: &SqlitePool,
    queue: &SyncQueue,
    input: TransitionInput,
    queue_prefix: &str,
) -> Result<Vec<String>, String> {
    transition_thread_configured(pool, queue, input, Some(queue_prefix), 0)
        .await
        .map(|(_, queued_ids)| queued_ids)
}

async fn transition_thread_configured(
    pool: &SqlitePool,
    queue: &SyncQueue,
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
    let undo_requested: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM mail_action_threads t JOIN mail_action_receipts r ON r.id = t.receipt_id WHERE t.thread_id = ? AND r.state = 'requested')")
        .bind(&input.thread_id).fetch_one(&mut *tx).await.map_err(|e| e.to_string())?;
    if undo_requested {
        return Err("backend.errors.mailMovePending".into());
    }
    if workflow_move_reconciliation_in_progress(&mut tx, &input.thread_id).await? {
        return Err("the server move is being finalized; try again shortly".to_string());
    }
    // Notification Done includes Read. Both intents must commit together,
    // with source-folder flags queued before the move changes the UID.
    let mut queued_ids = if queue_prefix.is_some() && input.to == WorkflowState::Done {
        crate::mail_actions::set_flags_in_transaction(
            &mut tx,
            queue,
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
        queue.enqueue_in_transaction(&mut tx, &action).await?;
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
        queue.wake_worker();
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

pub async fn list_recent_transitions(pool: &SqlitePool) -> Result<Vec<TransitionDto>, String> {
    let rows = sqlx::query_as::<_, TransitionRow>(
        "SELECT thread_id, from_state, to_state, created_at
         FROM workflow_events
         ORDER BY created_at DESC
         LIMIT 50",
    )
    .fetch_all(pool)
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
    use sqlx::sqlite::SqlitePoolOptions;
    async fn seeded_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("pool");
        sqlx::migrate!("./src/db/migrations")
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
}
