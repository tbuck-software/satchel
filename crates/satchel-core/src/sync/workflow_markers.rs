//! Push and pull of Done/Snooze markers (see `workflow::remote_marker`).
//!
//! Push: a workflow transition queues the thread's copies under one
//! `workflow_marker` mutation, inside the transition's transaction and before
//! its moves, so the keyword is on the source copy when MOVE carries it
//! along. Threads with the same marker share one unsent row, so archiving 300
//! threads is one provider session, not 300. A newer marker for a thread
//! takes it out of the unsent row it was in; Done plus its Undo inside the
//! Undo window never reaches the server.
//!
//! Marker writes are best effort. A server that refuses them never blocks
//! the account's queue: the folder counts as refused and the row settles.
//! `threads.remote_workflow_marker` records only what the server accepted.
//!
//! Pull: sync stores what the server reports per copy
//! (`messages.remote_workflow_marker`, `NULL` = no marker) and calls
//! [`reconcile_remote_workflow_markers`] for threads whose copies changed.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use sqlx::{Executor, Sqlite, SqlitePool, Transaction};

use crate::providers::traits::{
    MarkerTarget, MessageRef, WorkflowMarkerReport, WorkflowMarkerWrite,
};
use crate::sync::queue::{QueuedAction, SyncQueue};
use crate::sync::remote_mutations::{
    remote_move_thread_id_from_json, stable_queue_id, RemoteMutationPayload, ACTION_REMOTE_MUTATION,
};
use crate::workflow::remote_marker::{
    pull_decision, same_on_server, thread_marker, MarkerCopy, PullDecision, Snooze, WorkflowMarker,
};
use crate::workflow::states::WorkflowState;

/// `account_workflow_sync.mode` once INBOX accepted Satchel's keywords.
pub const MODE_SERVER: &str = "server";
/// `account_workflow_sync.mode` when the server cannot store them.
pub const MODE_LOCAL_ONLY: &str = "local_only";
/// INBOX's PERMANENTFLAGS lack `\*` (or the host is Gmail's IMAP). A later
/// snapshot that shows `\*` turns the account back to `server`.
pub const REASON_NO_CUSTOM_KEYWORDS: &str = "no_custom_keywords";
/// INBOX refused a keyword STORE although it lists `\*` (a full keyword
/// table, for example). Sticky: the snapshot would only say `\*` again.
pub const REASON_STORE_REFUSED: &str = "store_refused";

/// Threads per unsent marker row; keeps one payload and one STORE bounded.
const THREADS_PER_ROW: usize = 200;

/// One thread's share of a `workflow_marker` mutation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarkedThread {
    pub thread_id: String,
    pub refs: Vec<MessageRef>,
    /// Compare-and-clear: the marker this device last saw. The write only
    /// goes ahead while the thread's recorded marker and its copies on the
    /// server still show exactly this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected: Option<WorkflowMarker>,
}

impl MarkedThread {
    fn target(&self) -> MarkerTarget {
        MarkerTarget {
            refs: self.refs.clone(),
            expected: self.expected,
        }
    }
}

/// Providers whose server can hold the markers. Outlook would need
/// categories and is not wired yet; POP3 has no server state at all.
pub fn provider_carries_markers(provider: &str) -> bool {
    matches!(
        provider.trim().to_ascii_lowercase().as_str(),
        "imap" | "gmail"
    )
}

/// Record a push or snapshot verdict. Only a change is written.
pub async fn record_workflow_sync_mode<'e, E>(
    executor: E,
    account_id: &str,
    mode: &str,
    reason: Option<&str>,
) -> Result<(), String>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query(
        "INSERT INTO account_workflow_sync (account_id, mode, reason, checked_at) VALUES (?, ?, ?, ?)
         ON CONFLICT(account_id) DO UPDATE SET
             mode = excluded.mode, reason = excluded.reason, checked_at = excluded.checked_at
         WHERE account_workflow_sync.mode IS NOT excluded.mode
            OR account_workflow_sync.reason IS NOT excluded.reason",
    )
    .bind(account_id)
    .bind(mode)
    .bind(reason)
    .bind(chrono::Utc::now().to_rfc3339())
    .execute(executor)
    .await
    .map_err(|err| err.to_string())?;
    Ok(())
}

/// What INBOX's PERMANENTFLAGS say. Without `\*` the account is local-only;
/// with it, back to `server` unless a refused STORE made it local-only, which
/// the flags cannot disprove.
pub async fn record_inbox_keyword_support(
    pool: &SqlitePool,
    account_id: &str,
    allowed: bool,
) -> Result<(), String> {
    if !allowed {
        return record_workflow_sync_mode(
            pool,
            account_id,
            MODE_LOCAL_ONLY,
            Some(REASON_NO_CUSTOM_KEYWORDS),
        )
        .await;
    }
    let sticky: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM account_workflow_sync WHERE account_id = ? AND reason = ?)",
    )
    .bind(account_id)
    .bind(REASON_STORE_REFUSED)
    .fetch_one(pool)
    .await
    .map_err(|err| err.to_string())?;
    if sticky {
        return Ok(());
    }
    record_workflow_sync_mode(pool, account_id, MODE_SERVER, None).await
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

struct MarkerThreadRow {
    account_id: String,
    provider: String,
    recorded: WorkflowMarker,
}

/// The thread's account if it can carry markers at all.
async fn marker_thread(
    tx: &mut Transaction<'_, Sqlite>,
    thread_id: &str,
) -> Result<Option<MarkerThreadRow>, String> {
    let Some((account_id, provider, kind, recorded)) =
        sqlx::query_as::<_, (String, String, String, Option<String>)>(
            "SELECT t.account_id, a.provider, a.kind, t.remote_workflow_marker
             FROM threads t JOIN accounts a ON a.id = t.account_id WHERE t.id = ?",
        )
        .bind(thread_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|err| err.to_string())?
    else {
        return Ok(None);
    };
    if kind != "email"
        || !provider_carries_markers(&provider)
        || account_is_local_only(&mut **tx, &account_id).await?
    {
        return Ok(None);
    }
    Ok(Some(MarkerThreadRow {
        account_id,
        provider,
        recorded: WorkflowMarker::from_column(recorded.as_deref()),
    }))
}

type UnsentRow = (String, String, i64, Option<String>, RemoteMutationPayload);

async fn unsent_marker_rows(
    tx: &mut Transaction<'_, Sqlite>,
    account_id: &str,
) -> Result<Vec<UnsentRow>, String> {
    let rows = sqlx::query_as::<_, (String, String, i64, Option<String>, String)>(
        "SELECT id, status, retry_count, next_retry_at, payload FROM sync_queue
         WHERE account_id = ? AND action_type = ? AND status IN ('pending', 'scheduled')
           AND CASE WHEN json_valid(payload) THEN json_extract(payload, '$.kind') END = 'workflow_marker'
         ORDER BY rowid",
    )
    .bind(account_id)
    .bind(ACTION_REMOTE_MUTATION)
    .fetch_all(&mut **tx)
    .await
    .map_err(|err| err.to_string())?;
    Ok(rows
        .into_iter()
        .filter_map(|(id, status, retries, next_retry_at, payload)| {
            serde_json::from_str(&payload)
                .ok()
                .map(|payload| (id, status, retries, next_retry_at, payload))
        })
        .collect())
}

async fn store_payload(
    tx: &mut Transaction<'_, Sqlite>,
    id: &str,
    payload: &RemoteMutationPayload,
) -> Result<(), String> {
    sqlx::query("UPDATE sync_queue SET payload = ? WHERE id = ?")
        .bind(serde_json::to_string(payload).map_err(|err| err.to_string())?)
        .bind(id)
        .execute(&mut **tx)
        .await
        .map_err(|err| err.to_string())?;
    Ok(())
}

/// Take a thread out of every marker row not sent yet. Returns whether it
/// was in one.
async fn withdraw_unsent(
    tx: &mut Transaction<'_, Sqlite>,
    account_id: &str,
    thread_id: &str,
) -> Result<bool, String> {
    let mut withdrawn = false;
    for (id, _, _, _, mut payload) in unsent_marker_rows(tx, account_id).await? {
        let RemoteMutationPayload::WorkflowMarker { threads, .. } = &mut payload else {
            continue;
        };
        let before = threads.len();
        threads.retain(|thread| thread.thread_id != thread_id);
        if threads.len() == before {
            continue;
        }
        withdrawn = true;
        if threads.is_empty() {
            sqlx::query("DELETE FROM sync_queue WHERE id = ?")
                .bind(&id)
                .execute(&mut **tx)
                .await
                .map_err(|err| err.to_string())?;
        } else {
            store_payload(tx, &id, &payload).await?;
        }
    }
    Ok(withdrawn)
}

async fn marker_in_flight(
    tx: &mut Transaction<'_, Sqlite>,
    thread_id: &str,
) -> Result<bool, String> {
    let payloads = sqlx::query_scalar::<_, String>(
        "SELECT payload FROM sync_queue
         WHERE action_type = ? AND status = 'sending'
           AND CASE WHEN json_valid(payload) THEN json_extract(payload, '$.kind') END = 'workflow_marker'",
    )
    .bind(ACTION_REMOTE_MUTATION)
    .fetch_all(&mut **tx)
    .await
    .map_err(|err| err.to_string())?;
    Ok(payloads.iter().any(|payload| {
        matches!(
            serde_json::from_str::<RemoteMutationPayload>(payload),
            Ok(RemoteMutationPayload::WorkflowMarker { threads, .. })
                if threads.iter().any(|queued| queued.thread_id == thread_id)
        )
    }))
}

/// Join a waiting row with the same marker instead of opening another. Only
/// a fresh `pending` row qualifies: one waiting for a retry would hold the
/// new thread back, and notification rows have their own lease.
async fn join_unsent(
    tx: &mut Transaction<'_, Sqlite>,
    account_id: &str,
    marker: &WorkflowMarker,
    entry: &MarkedThread,
) -> Result<Option<String>, String> {
    for (id, status, retries, next_retry_at, mut payload) in
        unsent_marker_rows(tx, account_id).await?
    {
        if status != "pending"
            || retries != 0
            || next_retry_at.is_some()
            || id.starts_with("notification:")
        {
            continue;
        }
        let RemoteMutationPayload::WorkflowMarker {
            threads,
            marker: queued,
        } = &mut payload
        else {
            continue;
        };
        if queued != marker || threads.len() >= THREADS_PER_ROW {
            continue;
        }
        threads.push(entry.clone());
        store_payload(tx, &id, &payload).await?;
        return Ok(Some(id));
    }
    Ok(None)
}

async fn thread_refs(
    tx: &mut Transaction<'_, Sqlite>,
    thread_id: &str,
) -> Result<Vec<MessageRef>, String> {
    Ok(sqlx::query_as::<_, CopyRow>(
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
    .collect())
}

async fn enqueue_marker(
    tx: &mut Transaction<'_, Sqlite>,
    queue: &SyncQueue,
    thread_id: &str,
    account_id: &str,
    marker: WorkflowMarker,
    expected: Option<WorkflowMarker>,
    queue_prefix: Option<&str>,
) -> Result<Vec<String>, String> {
    let refs = thread_refs(tx, thread_id).await?;
    if refs.is_empty() {
        return Ok(Vec::new());
    }
    let entry = MarkedThread {
        thread_id: thread_id.to_string(),
        refs,
        expected,
    };
    if queue_prefix.is_none() {
        if let Some(id) = join_unsent(tx, account_id, &marker, &entry).await? {
            return Ok(vec![id]);
        }
    }
    let mut action = QueuedAction::new(
        account_id.to_string(),
        ACTION_REMOTE_MUTATION,
        serde_json::to_value(RemoteMutationPayload::WorkflowMarker {
            threads: vec![entry],
            marker,
        })
        .map_err(|err| err.to_string())?,
    );
    if let Some(prefix) = queue_prefix {
        action.id = stable_queue_id(prefix, "workflow-marker", account_id);
    }
    queue.enqueue_in_transaction(tx, &action).await?;
    Ok(vec![action.id])
}

/// Queue the server form of `marker` for a thread. Returns the queued ids;
/// none when the provider or server cannot carry markers, the thread has no
/// server copies, or the server already holds this marker.
pub async fn enqueue_workflow_marker_in_transaction(
    tx: &mut Transaction<'_, Sqlite>,
    queue: &SyncQueue,
    thread_id: &str,
    marker: &WorkflowMarker,
    queue_prefix: Option<&str>,
) -> Result<Vec<String>, String> {
    let Some(thread) = marker_thread(tx, thread_id).await? else {
        return Ok(Vec::new());
    };
    let marker = if thread.provider.eq_ignore_ascii_case("imap") {
        marker.after(&thread.recorded)
    } else {
        *marker
    };
    withdraw_unsent(tx, &thread.account_id, thread_id).await?;
    // The server holds what was recorded, unless a row is on its way there
    // right now; then the new marker has to follow it.
    if thread.recorded == marker && !marker_in_flight(tx, thread_id).await? {
        return Ok(Vec::new());
    }
    enqueue_marker(
        tx,
        queue,
        thread_id,
        &thread.account_id,
        marker,
        None,
        queue_prefix,
    )
    .await
}

/// The snooze worker woke this thread. Cancel a snooze that never left, and
/// clear the one on the server only if it is the snooze this device set
/// (it knows its time) and nobody replaced it since: compare-and-clear, so a
/// newer snooze from another device survives a device that woke late.
///
/// IMAP only. Gmail labels carry the time, every device resolves a passed
/// wake label to Inbox by itself, and expired labels are cleaned up with
/// their messages (`GmailProvider::delete_expired_wake_labels`).
pub async fn enqueue_wake_clear_in_transaction(
    tx: &mut Transaction<'_, Sqlite>,
    queue: &SyncQueue,
    thread_id: &str,
) -> Result<Vec<String>, String> {
    let Some(thread) = marker_thread(tx, thread_id).await? else {
        return Ok(Vec::new());
    };
    withdraw_unsent(tx, &thread.account_id, thread_id).await?;
    let own_snooze = matches!(
        thread.recorded,
        WorkflowMarker::Snoozed(Snooze { wake: Some(_), .. })
    );
    if !thread.provider.eq_ignore_ascii_case("imap") || !own_snooze {
        return Ok(Vec::new());
    }
    enqueue_marker(
        tx,
        queue,
        thread_id,
        &thread.account_id,
        WorkflowMarker::None,
        Some(thread.recorded),
        None,
    )
    .await
}

/// Drop compare-and-clear entries whose thread moved on after they were
/// queued: its recorded marker is no longer the one the clear expects.
pub async fn retain_current_entries(
    pool: &SqlitePool,
    threads: Vec<MarkedThread>,
) -> Result<Vec<MarkedThread>, String> {
    let mut current = Vec::with_capacity(threads.len());
    for thread in threads {
        if let Some(expected) = thread.expected {
            let recorded: Option<Option<String>> =
                sqlx::query_scalar("SELECT remote_workflow_marker FROM threads WHERE id = ?")
                    .bind(&thread.thread_id)
                    .fetch_optional(pool)
                    .await
                    .map_err(|err| err.to_string())?;
            let Some(recorded) = recorded else {
                continue;
            };
            if WorkflowMarker::from_column(recorded.as_deref()) != expected {
                continue;
            }
        }
        current.push(thread);
    }
    Ok(current)
}

/// The provider call of a `workflow_marker` row, and its bookkeeping.
pub async fn execute_marker_write(
    pool: &SqlitePool,
    account_id: &str,
    provider: &dyn crate::providers::traits::MailProvider,
    threads: &[MarkedThread],
    marker: &WorkflowMarker,
) -> crate::sync::errors::SyncResult<()> {
    let targets = threads.iter().map(MarkedThread::target).collect::<Vec<_>>();
    let report = provider.set_workflow_marker(&targets, marker).await?;
    apply_marker_report(pool, account_id, threads, marker, &report).await?;
    Ok(())
}

/// Record what the server accepted. A refused or skipped thread keeps its
/// last recorded marker, so the server's silence later reads as no change.
pub async fn apply_marker_report(
    pool: &SqlitePool,
    account_id: &str,
    threads: &[MarkedThread],
    marker: &WorkflowMarker,
    report: &WorkflowMarkerReport,
) -> Result<(), String> {
    let mut applied = false;
    for (thread, outcome) in threads.iter().zip(&report.targets) {
        if *outcome != WorkflowMarkerWrite::Applied {
            continue;
        }
        applied = true;
        sqlx::query("UPDATE threads SET remote_workflow_marker = ? WHERE id = ?")
            .bind(marker.as_column())
            .bind(&thread.thread_id)
            .execute(pool)
            .await
            .map_err(|err| err.to_string())?;
    }
    if report.refused_in_inbox {
        record_workflow_sync_mode(
            pool,
            account_id,
            MODE_LOCAL_ONLY,
            Some(REASON_STORE_REFUSED),
        )
        .await?;
    } else if applied {
        record_inbox_keyword_support(pool, account_id, true).await?;
    }
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
                Ok(RemoteMutationPayload::WorkflowMarker { threads, .. })
                    if threads.iter().any(|queued| queued.thread_id == thread_id)
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

async fn server_marker<'e, E>(executor: E, thread_id: &str) -> Result<WorkflowMarker, String>
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
        marker: WorkflowMarker::from_column(marker.as_deref()),
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
    let server = server_marker(pool, thread_id).await?;
    let recorded: Option<Option<String>> =
        sqlx::query_scalar("SELECT remote_workflow_marker FROM threads WHERE id = ?")
            .bind(thread_id)
            .fetch_optional(pool)
            .await
            .map_err(|err| err.to_string())?;
    let Some(recorded) = recorded else {
        return Ok(false);
    };
    if same_on_server(&WorkflowMarker::from_column(recorded.as_deref()), &server) {
        return Ok(false);
    }

    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|err| err.to_string())?;
    let server = server_marker(&mut *tx, thread_id).await?;
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
    let seen = WorkflowMarker::from_column(row.remote_workflow_marker.as_deref());
    let local_state = row
        .workflow_state
        .as_deref()
        .and_then(WorkflowState::from_db_value)
        .unwrap_or(WorkflowState::Inbox);
    let now = chrono::Utc::now();
    let decision = pull_decision(
        &seen,
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
        .bind(server.as_column())
        .bind(thread_id)
        .execute(&mut *tx)
        .await
        .map_err(|err| err.to_string())?;
    tx.commit().await.map_err(|err| err.to_string())?;
    Ok(applied)
}

#[cfg(test)]
mod tests;
