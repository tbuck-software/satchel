//! Push and pull of Done/Snooze markers (see `workflow::remote_marker`).
//!
//! Push: a workflow transition queues one `workflow_marker` mutation per
//! thread, inside the transition's transaction and before its moves, so the
//! keyword is on the source copy when MOVE carries it along. A newer marker
//! for the same thread replaces one that has not been sent yet; Done and its
//! Undo within the Undo window never reach the server.
//!
//! Pull: sync stores what the server reports per copy
//! (`messages.remote_workflow_marker`) and then calls
//! [`reconcile_remote_workflow_markers`] for the threads it touched.

use std::collections::HashSet;

use sqlx::{Executor, Sqlite, SqlitePool, Transaction};

use crate::providers::traits::MessageRef;
use crate::sync::queue::{QueuedAction, SyncQueue};
use crate::sync::remote_mutations::{
    remote_move_thread_id_from_json, stable_queue_id, RemoteMutationPayload, ACTION_REMOTE_MUTATION,
};
use crate::workflow::remote_marker::{
    pull_decision, thread_marker, MarkerCopy, PullDecision, WorkflowMarker,
};
use crate::workflow::states::WorkflowState;

/// `account_workflow_sync.mode` once a mailbox accepted Satchel's keywords.
pub const MODE_SERVER: &str = "server";
/// `account_workflow_sync.mode` when the server cannot store them.
pub const MODE_LOCAL_ONLY: &str = "local_only";

/// Providers whose server can hold the markers. Outlook would need
/// categories and is not wired yet; POP3 has no server state at all.
pub fn provider_carries_markers(provider: &str) -> bool {
    matches!(
        provider.trim().to_ascii_lowercase().as_str(),
        "imap" | "gmail"
    )
}

/// Record what the server said about keywords. Only a change is written, so
/// a sync that reports the same answer every five minutes costs one read.
pub async fn record_workflow_sync_mode<'e, E>(
    executor: E,
    account_id: &str,
    mode: &str,
) -> Result<(), String>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query(
        "INSERT INTO account_workflow_sync (account_id, mode, checked_at) VALUES (?, ?, ?)
         ON CONFLICT(account_id) DO UPDATE SET mode = excluded.mode, checked_at = excluded.checked_at
         WHERE account_workflow_sync.mode IS NOT excluded.mode",
    )
    .bind(account_id)
    .bind(mode)
    .bind(chrono::Utc::now().to_rfc3339())
    .execute(executor)
    .await
    .map_err(|err| err.to_string())?;
    Ok(())
}

/// Accounts whose server cannot store Done and Snooze, for the settings note.
pub async fn list_local_only_accounts(pool: &SqlitePool) -> Result<Vec<String>, String> {
    sqlx::query_scalar(
        "SELECT s.account_id FROM account_workflow_sync s
         JOIN accounts a ON a.id = s.account_id
         WHERE s.mode = ? ORDER BY s.account_id",
    )
    .bind(MODE_LOCAL_ONLY)
    .fetch_all(pool)
    .await
    .map_err(|err| err.to_string())
}

async fn account_is_local_only<'e, E>(executor: E, account_id: &str) -> Result<bool, String>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM account_workflow_sync WHERE account_id = ? AND mode = ?)",
    )
    .bind(account_id)
    .bind(MODE_LOCAL_ONLY)
    .fetch_one(executor)
    .await
    .map_err(|err| err.to_string())
}

#[derive(sqlx::FromRow)]
struct CopyRow {
    remote_id: Option<String>,
    source_folder: String,
    source_uidvalidity: Option<i64>,
}

/// Queue the server form of `marker` for a thread. Returns the queued ids;
/// none when the provider or server cannot carry markers, the thread has no
/// server copies, or the server already holds this marker as far as this
/// device knows.
pub async fn enqueue_workflow_marker_in_transaction(
    tx: &mut Transaction<'_, Sqlite>,
    queue: &SyncQueue,
    thread_id: &str,
    marker: &WorkflowMarker,
    queue_prefix: Option<&str>,
) -> Result<Vec<String>, String> {
    let Some((account_id, provider, kind, last_pushed)) =
        sqlx::query_as::<_, (String, String, String, Option<String>)>(
            "SELECT t.account_id, a.provider, a.kind, t.remote_workflow_marker
             FROM threads t JOIN accounts a ON a.id = t.account_id WHERE t.id = ?",
        )
        .bind(thread_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|err| err.to_string())?
    else {
        return Ok(Vec::new());
    };
    if kind != "email"
        || !provider_carries_markers(&provider)
        || account_is_local_only(&mut **tx, &account_id).await?
    {
        return Ok(Vec::new());
    }

    // A marker not sent yet is replaced, whatever it said: the new one
    // describes the whole target state. One already in flight stays; the new
    // row runs after it.
    let replaced = sqlx::query(
        "DELETE FROM sync_queue
         WHERE action_type = ? AND status IN ('pending', 'scheduled')
           AND CASE WHEN json_valid(payload) THEN json_extract(payload, '$.kind') END = 'workflow_marker'
           AND json_extract(payload, '$.thread_id') = ?",
    )
    .bind(ACTION_REMOTE_MUTATION)
    .bind(thread_id)
    .execute(&mut **tx)
    .await
    .map_err(|err| err.to_string())?
    .rows_affected();
    let last_pushed = last_pushed
        .as_deref()
        .and_then(WorkflowMarker::from_db_value)
        .unwrap_or(WorkflowMarker::None);
    if replaced == 0 && &last_pushed == marker {
        return Ok(Vec::new());
    }

    let refs = sqlx::query_as::<_, CopyRow>(
        "SELECT m.remote_id, COALESCE(f.remote_id, f.full_path) AS source_folder,
                f.uidvalidity AS source_uidvalidity
         FROM messages m JOIN folders f ON f.id = m.folder_id
         WHERE m.thread_id = ? AND f.folder_type != 'local'
         ORDER BY m.id",
    )
    .bind(thread_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(|err| err.to_string())?
    .into_iter()
    .filter_map(|row| {
        let remote_id = row.remote_id.filter(|id| !id.trim().is_empty())?;
        Some(MessageRef {
            remote_id,
            source_folder: Some(row.source_folder),
            source_uidvalidity: row.source_uidvalidity.and_then(|n| u32::try_from(n).ok()),
        })
    })
    .collect::<Vec<_>>();
    if refs.is_empty() {
        return Ok(Vec::new());
    }

    let mut action = QueuedAction::new(
        account_id.clone(),
        ACTION_REMOTE_MUTATION,
        serde_json::to_value(RemoteMutationPayload::WorkflowMarker {
            thread_id: thread_id.to_string(),
            refs,
            marker: marker.clone(),
        })
        .map_err(|err| err.to_string())?,
    );
    if let Some(prefix) = queue_prefix {
        action.id = stable_queue_id(prefix, "workflow-marker", &account_id);
    }
    queue.enqueue_in_transaction(tx, &action).await?;
    sqlx::query("UPDATE threads SET remote_workflow_marker = ? WHERE id = ?")
        .bind(marker.as_db_value())
        .bind(thread_id)
        .execute(&mut **tx)
        .await
        .map_err(|err| err.to_string())?;
    Ok(vec![action.id])
}

/// After a push the server refused for lack of keyword support: forget what
/// was pushed, so the server's silence is not read as someone removing it
/// once the server learns keywords.
pub async fn forget_unsupported_push(pool: &SqlitePool, thread_id: &str) -> Result<(), String> {
    sqlx::query("UPDATE threads SET remote_workflow_marker = NULL WHERE id = ?")
        .bind(thread_id)
        .execute(pool)
        .await
        .map_err(|err| err.to_string())?;
    Ok(())
}

/// Whether a workflow change of this thread is still on its way to the
/// server: a marker or a move not yet confirmed.
async fn local_change_pending(
    tx: &mut Transaction<'_, Sqlite>,
    thread_id: &str,
) -> Result<bool, String> {
    let payloads = sqlx::query_scalar::<_, String>(
        "SELECT payload FROM sync_queue
         WHERE action_type = ? AND status IN ('pending', 'scheduled', 'sending')",
    )
    .bind(ACTION_REMOTE_MUTATION)
    .fetch_all(&mut **tx)
    .await
    .map_err(|err| err.to_string())?;
    Ok(payloads.iter().any(|payload| {
        remote_move_thread_id_from_json(payload).as_deref() == Some(thread_id)
            || matches!(
                serde_json::from_str::<RemoteMutationPayload>(payload),
                Ok(RemoteMutationPayload::WorkflowMarker { thread_id: queued, .. }) if queued == thread_id
            )
    }))
}

/// Apply what the server says about these threads' workflow. Returns the
/// threads whose local state changed.
pub async fn reconcile_remote_workflow_markers(
    pool: &SqlitePool,
    thread_ids: &HashSet<String>,
) -> Result<HashSet<String>, String> {
    let mut changed = HashSet::new();
    for thread_id in thread_ids {
        if reconcile_thread(pool, thread_id).await? {
            changed.insert(thread_id.clone());
        }
    }
    Ok(changed)
}

async fn server_marker<'e, E>(
    executor: E,
    thread_id: &str,
) -> Result<Option<WorkflowMarker>, String>
where
    E: Executor<'e, Database = Sqlite>,
{
    let copies = sqlx::query_as::<_, (i64, Option<String>)>(
        "SELECT LOWER(TRIM(f.folder_type)) = 'inbox', m.remote_workflow_marker
         FROM messages m JOIN folders f ON f.id = m.folder_id
         WHERE m.thread_id = ? AND f.folder_type != 'local'",
    )
    .bind(thread_id)
    .fetch_all(executor)
    .await
    .map_err(|err| err.to_string())?
    .into_iter()
    .map(|(in_inbox, marker)| MarkerCopy {
        in_inbox: in_inbox != 0,
        marker: marker.as_deref().and_then(WorkflowMarker::from_db_value),
    })
    .collect::<Vec<_>>();
    Ok(thread_marker(&copies))
}

#[derive(sqlx::FromRow)]
struct ThreadRow {
    account_id: String,
    provider: String,
    workflow_state: Option<String>,
    snooze_until: Option<String>,
    remote_workflow_marker: Option<String>,
}

async fn reconcile_thread(pool: &SqlitePool, thread_id: &str) -> Result<bool, String> {
    // Read first without the write lock: almost every call finds nothing new.
    let Some(server) = server_marker(pool, thread_id).await? else {
        return Ok(false);
    };
    let seen: Option<Option<String>> =
        sqlx::query_scalar("SELECT remote_workflow_marker FROM threads WHERE id = ?")
            .bind(thread_id)
            .fetch_optional(pool)
            .await
            .map_err(|err| err.to_string())?;
    let Some(seen) = seen else {
        return Ok(false);
    };
    let seen = seen.as_deref().and_then(WorkflowMarker::from_db_value);
    if seen.as_ref().unwrap_or(&WorkflowMarker::None) == &server {
        if seen.is_none() {
            // First reading of a thread the server says nothing about.
            sqlx::query(
                "UPDATE threads SET remote_workflow_marker = ? WHERE id = ? AND remote_workflow_marker IS NULL",
            )
            .bind(server.as_db_value())
            .bind(thread_id)
            .execute(pool)
            .await
            .map_err(|err| err.to_string())?;
        }
        return Ok(false);
    }

    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|err| err.to_string())?;
    let Some(server) = server_marker(&mut *tx, thread_id).await? else {
        return Ok(false);
    };
    let Some(row) = sqlx::query_as::<_, ThreadRow>(
        "SELECT t.account_id, a.provider, t.workflow_state, t.snooze_until, t.remote_workflow_marker
         FROM threads t JOIN accounts a ON a.id = t.account_id WHERE t.id = ?",
    )
    .bind(thread_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|err| err.to_string())?
    else {
        return Ok(false);
    };
    // A server that cannot store keywords says nothing on purpose; its
    // silence must not wake a local snooze.
    if !provider_carries_markers(&row.provider)
        || account_is_local_only(&mut *tx, &row.account_id).await?
    {
        return Ok(false);
    }
    // Never fight the person: a change of theirs that has not reached the
    // server yet wins over what the server still shows.
    if local_change_pending(&mut tx, thread_id).await? {
        return Ok(false);
    }
    let seen = row
        .remote_workflow_marker
        .as_deref()
        .and_then(WorkflowMarker::from_db_value);
    let local_state = row
        .workflow_state
        .as_deref()
        .and_then(WorkflowState::from_db_value)
        .unwrap_or(WorkflowState::Inbox);
    let now = chrono::Utc::now();
    let decision = pull_decision(
        seen.as_ref(),
        &server,
        local_state,
        row.snooze_until.as_deref(),
        now,
    );
    let mut applied = false;
    if let PullDecision::Apply {
        state,
        snooze_until,
    } = decision
    {
        let now = now.to_rfc3339();
        sqlx::query(
            "UPDATE threads
             SET workflow_state = ?, board_column = ?, snooze_until = ?,
                 workflow_changed_at = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(state.as_db_value())
        .bind(state.as_db_value())
        .bind(snooze_until.map(|at| at.to_rfc3339()))
        .bind(&now)
        .bind(&now)
        .bind(thread_id)
        .execute(&mut *tx)
        .await
        .map_err(|err| err.to_string())?;
        sqlx::query(
            "INSERT INTO workflow_events (id, thread_id, from_state, to_state, reason, created_at)
             VALUES (?, ?, ?, ?, 'server', ?)",
        )
        .bind(format!("wf-{}", uuid::Uuid::new_v4()))
        .bind(thread_id)
        .bind(local_state.as_db_value())
        .bind(state.as_db_value())
        .bind(&now)
        .execute(&mut *tx)
        .await
        .map_err(|err| err.to_string())?;
        applied = true;
    }
    sqlx::query("UPDATE threads SET remote_workflow_marker = ? WHERE id = ?")
        .bind(server.as_db_value())
        .bind(thread_id)
        .execute(&mut *tx)
        .await
        .map_err(|err| err.to_string())?;
    tx.commit().await.map_err(|err| err.to_string())?;
    Ok(applied)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};
    use sqlx::sqlite::SqlitePoolOptions;

    async fn pool(provider: &str) -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("pool");
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .expect("migrations");
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
             VALUES ('acc', 'Me', 'me@example.com', ?, '{}', '', ?, ?)",
        )
        .bind(provider)
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("account");
        for (id, kind) in [("INBOX", "inbox"), ("Archive", "archive")] {
            sqlx::query(
                "INSERT INTO folders (id, account_id, name, full_path, folder_type, remote_id, uidvalidity, created_at, updated_at)
                 VALUES (?, 'acc', ?, ?, ?, ?, 7, ?, ?)",
            )
            .bind(id)
            .bind(id)
            .bind(id)
            .bind(kind)
            .bind(id)
            .bind(&now)
            .bind(&now)
            .execute(&pool)
            .await
            .expect("folder");
        }
        sqlx::query(
            "INSERT INTO threads (id, account_id, subject, last_message_date, workflow_state, board_column, created_at, updated_at)
             VALUES ('t', 'acc', 'Subject', ?, 'inbox', 'inbox', ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("thread");
        message(&pool, "m1", "INBOX", "41", None).await;
        pool
    }

    async fn message(pool: &SqlitePool, id: &str, folder: &str, uid: &str, marker: Option<&str>) {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO messages (id, account_id, folder_id, thread_id, from_address, to_addresses, date, remote_id, remote_workflow_marker, created_at, updated_at)
             VALUES (?, 'acc', ?, 't', 'a@example.com', '[]', ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(folder)
        .bind(&now)
        .bind(uid)
        .bind(marker)
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await
        .expect("message");
    }

    async fn queued_markers(pool: &SqlitePool) -> Vec<(String, WorkflowMarker)> {
        sqlx::query_scalar::<_, String>(
            "SELECT payload FROM sync_queue WHERE json_extract(payload, '$.kind') = 'workflow_marker' ORDER BY rowid",
        )
        .fetch_all(pool)
        .await
        .expect("queue")
        .into_iter()
        .map(|payload| match serde_json::from_str(&payload).expect("payload") {
            RemoteMutationPayload::WorkflowMarker { thread_id, marker, .. } => (thread_id, marker),
            other => panic!("unexpected payload {other:?}"),
        })
        .collect()
    }

    async fn push(pool: &SqlitePool, marker: WorkflowMarker) -> Vec<String> {
        let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.expect("tx");
        let ids =
            enqueue_workflow_marker_in_transaction(&mut tx, &SyncQueue::new(), "t", &marker, None)
                .await
                .expect("enqueue");
        tx.commit().await.expect("commit");
        ids
    }

    async fn state(pool: &SqlitePool) -> (String, Option<String>, Option<String>) {
        sqlx::query_as(
            "SELECT workflow_state, snooze_until, remote_workflow_marker FROM threads WHERE id = 't'",
        )
        .fetch_one(pool)
        .await
        .expect("thread")
    }

    async fn set_server(pool: &SqlitePool, marker: &WorkflowMarker) {
        sqlx::query("UPDATE messages SET remote_workflow_marker = ? WHERE thread_id = 't'")
            .bind(marker.as_db_value())
            .execute(pool)
            .await
            .expect("server marker");
    }

    async fn pull(pool: &SqlitePool) -> bool {
        !reconcile_remote_workflow_markers(pool, &HashSet::from(["t".to_string()]))
            .await
            .expect("reconcile")
            .is_empty()
    }

    #[tokio::test]
    async fn a_marker_push_carries_every_server_copy_of_the_thread() {
        let pool = pool("imap").await;
        message(&pool, "m2", "Archive", "9", None).await;
        let ids = push(&pool, WorkflowMarker::Done).await;
        assert_eq!(ids.len(), 1);
        let payload: String = sqlx::query_scalar("SELECT payload FROM sync_queue")
            .fetch_one(&pool)
            .await
            .unwrap();
        let RemoteMutationPayload::WorkflowMarker { refs, .. } =
            serde_json::from_str(&payload).unwrap()
        else {
            panic!("marker payload");
        };
        let refs = refs
            .iter()
            .map(|r| {
                (
                    r.source_folder.clone().unwrap(),
                    r.remote_id.clone(),
                    r.source_uidvalidity,
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            refs,
            vec![
                ("INBOX".into(), "41".into(), Some(7)),
                ("Archive".into(), "9".into(), Some(7))
            ]
        );
        assert_eq!(state(&pool).await.2.as_deref(), Some("done"));
    }

    #[tokio::test]
    async fn a_rapid_toggle_leaves_only_the_last_marker() {
        let pool = pool("gmail").await;
        let wake = Utc::now() + Duration::hours(3);
        push(&pool, WorkflowMarker::Done).await;
        push(&pool, WorkflowMarker::Snoozed(wake)).await;
        push(&pool, WorkflowMarker::None).await;
        assert_eq!(
            queued_markers(&pool).await,
            vec![("t".into(), WorkflowMarker::None)]
        );
        // Nothing new to say: the server was last told None.
        sqlx::query("DELETE FROM sync_queue")
            .execute(&pool)
            .await
            .unwrap();
        assert!(push(&pool, WorkflowMarker::None).await.is_empty());
    }

    #[tokio::test]
    async fn a_marker_in_flight_is_kept_and_the_new_one_follows() {
        let pool = pool("imap").await;
        push(&pool, WorkflowMarker::Done).await;
        sqlx::query("UPDATE sync_queue SET status = 'sending'")
            .execute(&pool)
            .await
            .unwrap();
        push(&pool, WorkflowMarker::None).await;
        assert_eq!(
            queued_markers(&pool).await,
            vec![
                ("t".into(), WorkflowMarker::Done),
                ("t".into(), WorkflowMarker::None)
            ]
        );
    }

    #[tokio::test]
    async fn local_only_accounts_and_other_providers_queue_nothing() {
        let pool = pool("imap").await;
        record_workflow_sync_mode(&pool, "acc", MODE_LOCAL_ONLY)
            .await
            .unwrap();
        assert!(push(&pool, WorkflowMarker::Done).await.is_empty());
        assert_eq!(
            list_local_only_accounts(&pool).await.unwrap(),
            vec!["acc".to_string()]
        );
        record_workflow_sync_mode(&pool, "acc", MODE_SERVER)
            .await
            .unwrap();
        assert!(list_local_only_accounts(&pool).await.unwrap().is_empty());

        let outlook = pool_with_provider("outlook").await;
        assert!(push(&outlook, WorkflowMarker::Done).await.is_empty());
    }

    async fn pool_with_provider(provider: &str) -> SqlitePool {
        pool(provider).await
    }

    #[tokio::test]
    async fn a_snooze_set_on_another_device_is_adopted_and_wakes_here() {
        let pool = pool("imap").await;
        let wake =
            chrono::DateTime::from_timestamp((Utc::now() + Duration::hours(2)).timestamp(), 0)
                .unwrap();
        set_server(&pool, &WorkflowMarker::Snoozed(wake)).await;
        assert!(pull(&pool).await);
        let (state_now, until, seen) = state(&pool).await;
        assert_eq!(state_now, "snoozed");
        assert_eq!(until, Some(wake.to_rfc3339()));
        assert_eq!(seen, Some(WorkflowMarker::Snoozed(wake).as_db_value()));
        // The local snooze worker compares this column as text against
        // `Utc::now().to_rfc3339()`; the stored form must sort the same way.
        assert!(until.unwrap() > Utc::now().to_rfc3339());
        // Seen once, nothing more to do.
        assert!(!pull(&pool).await);

        // Woken elsewhere: the keywords are gone.
        set_server(&pool, &WorkflowMarker::None).await;
        assert!(pull(&pool).await);
        assert_eq!(state(&pool).await.0, "inbox");
    }

    #[tokio::test]
    async fn the_servers_silence_does_not_reset_a_local_state() {
        let pool = pool("imap").await;
        sqlx::query("UPDATE threads SET workflow_state = 'todo'")
            .execute(&pool)
            .await
            .unwrap();
        set_server(&pool, &WorkflowMarker::None).await;
        assert!(!pull(&pool).await);
        assert_eq!(
            state(&pool).await,
            ("todo".into(), None, Some(String::new()))
        );
    }

    #[tokio::test]
    async fn an_unsent_local_change_wins_over_the_server() {
        let pool = pool("imap").await;
        // As the UI sends it, with milliseconds; the marker keeps seconds.
        let wake = Utc::now() + Duration::hours(2);
        let marker = WorkflowMarker::for_state(WorkflowState::Snoozed, Some(&wake.to_rfc3339()));
        // Snoozed here, not pushed yet; the server still shows nothing.
        sqlx::query("UPDATE threads SET workflow_state = 'snoozed', snooze_until = ?")
            .bind(wake.to_rfc3339())
            .execute(&pool)
            .await
            .unwrap();
        push(&pool, marker.clone()).await;
        set_server(&pool, &WorkflowMarker::None).await;
        assert!(!pull(&pool).await);
        assert_eq!(state(&pool).await.0, "snoozed");

        // Pushed and echoed back: still nothing to do.
        sqlx::query("DELETE FROM sync_queue")
            .execute(&pool)
            .await
            .unwrap();
        set_server(&pool, &marker).await;
        assert!(!pull(&pool).await);
        assert_eq!(state(&pool).await.0, "snoozed");
    }

    #[tokio::test]
    async fn a_refused_push_does_not_read_as_a_wake_later() {
        let pool = pool("imap").await;
        let wake = Utc::now() + Duration::hours(2);
        sqlx::query("UPDATE threads SET workflow_state = 'snoozed', snooze_until = ?")
            .bind(wake.to_rfc3339())
            .execute(&pool)
            .await
            .unwrap();
        push(&pool, WorkflowMarker::Snoozed(wake)).await;
        sqlx::query("DELETE FROM sync_queue")
            .execute(&pool)
            .await
            .unwrap();
        forget_unsupported_push(&pool, "t").await.unwrap();
        set_server(&pool, &WorkflowMarker::None).await;
        assert!(!pull(&pool).await);
        assert_eq!(state(&pool).await.0, "snoozed");
    }

    #[tokio::test]
    async fn local_only_accounts_ignore_the_server() {
        let pool = pool("imap").await;
        record_workflow_sync_mode(&pool, "acc", MODE_LOCAL_ONLY)
            .await
            .unwrap();
        set_server(
            &pool,
            &WorkflowMarker::Snoozed(Utc::now() + Duration::hours(2)),
        )
        .await;
        assert!(!pull(&pool).await);
        assert_eq!(state(&pool).await.0, "inbox");
    }

    #[tokio::test]
    async fn done_elsewhere_is_applied_once_the_mail_left_the_inbox() {
        let pool = pool("imap").await;
        sqlx::query("UPDATE messages SET folder_id = 'Archive', remote_workflow_marker = 'done'")
            .execute(&pool)
            .await
            .unwrap();
        assert!(pull(&pool).await);
        assert_eq!(state(&pool).await.0, "done");
        let reason: String =
            sqlx::query_scalar("SELECT reason FROM workflow_events WHERE thread_id = 't'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(reason, "server");
    }
}
