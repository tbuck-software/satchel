//! Push and pull of Done/Snooze markers (see `workflow::remote_marker`).
//!
//! Push: a workflow transition queues the thread's copies under one
//! `workflow_marker` mutation, inside the transition's transaction and before
//! its moves, so the keyword is on the source copy when MOVE carries it
//! along. Threads with the same marker share one unsent row, so archiving 300
//! threads is a few provider sessions, not 300. A newer marker for a thread
//! takes it out of the unsent row it was in; Done plus its Undo inside the
//! Undo window never reaches the server.
//!
//! Queue lookups are done by SQLite over the payload (`json_each`), so the
//! cost of queueing one thread does not grow with the threads already
//! waiting, and a batch ([`enqueue_workflow_markers_in_transaction`]) writes
//! each row once.
//!
//! Marker writes are best effort. A server that refuses them never blocks
//! the account's queue: the folder counts as refused and the row settles.
//! `threads.remote_workflow_marker` records only what the server accepted.
//!
//! Pull: sync stores what the server reports per copy
//! (`messages.remote_workflow_marker`, `NULL` = no marker) and calls
//! [`reconcile_remote_workflow_markers`] for threads whose copies changed.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Serialize};
use sqlx::{Executor, Sqlite, SqlitePool, Transaction};

use crate::providers::traits::{
    KeywordSupport, MarkerTarget, MessageRef, WorkflowMarkerReport, WorkflowMarkerWrite,
};
use crate::sync::queue::{QueuedAction, SyncQueue};
use crate::sync::remote_mutations::{
    remote_move_thread_id_from_json, stable_queue_id, RemoteMutationPayload,
    ACTION_REMOTE_MUTATION, QUEUED_MOVE_THREAD_ID,
};
use crate::workflow::remote_marker::{
    pull_decision, same_on_server, thread_marker, MarkerCopy, PullDecision, Snooze, WorkflowMarker,
};
use crate::workflow::states::WorkflowState;

/// `account_workflow_sync.mode` once INBOX accepted Satchel's keywords.
pub const MODE_SERVER: &str = "server";
/// `account_workflow_sync.mode` when the server cannot store them.
pub const MODE_LOCAL_ONLY: &str = "local_only";
/// INBOX's PERMANENTFLAGS lack `\*`. A later snapshot that shows `\*` lifts it.
pub const REASON_NO_CUSTOM_KEYWORDS: &str = "no_custom_keywords";
/// The account is on Gmail's IMAP host, where keywords become Gmail labels.
pub const REASON_GMAIL_IMAP: &str = "gmail_imap";
/// INBOX answered a keyword STORE with a definite `NO` or `BAD`. Not
/// permanent: it holds for [`STORE_REFUSED_RETRY_AFTER`] and until the app
/// restarts, then the next push tries again, and an accepted INBOX store
/// lifts it.
pub const REASON_STORE_REFUSED: &str = "store_refused";
/// How long a refused STORE keeps the account local-only.
pub const STORE_REFUSED_RETRY_AFTER: chrono::Duration = chrono::Duration::hours(6);

/// Threads per unsent marker row; keeps one payload and one STORE bounded.
const THREADS_PER_ROW: usize = 100;

/// When this process started: a refused STORE recorded before it is retried.
fn process_started() -> &'static str {
    static STARTED: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    STARTED.get_or_init(|| chrono::Utc::now().to_rfc3339())
}

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

/// Record a push or snapshot verdict. A refused STORE always moves
/// `checked_at`, which is when its retry clock starts; anything else is
/// written only when it changes.
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
            OR account_workflow_sync.reason IS NOT excluded.reason
            OR excluded.reason = 'store_refused'",
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

/// SQL condition (on `account_workflow_sync s`) for a local-only verdict that
/// still holds. Binds: process start, retry cutoff.
const EFFECTIVE_LOCAL_ONLY: &str = "s.mode = 'local_only'
     AND (s.reason IS NOT 'store_refused' OR (s.checked_at >= ? AND s.checked_at > ?))";

fn retry_cutoff() -> String {
    (chrono::Utc::now() - STORE_REFUSED_RETRY_AFTER).to_rfc3339()
}

/// What INBOX's PERMANENTFLAGS (or the host) say. A refused STORE that still
/// holds is not lifted by flags that only say `\*` again.
pub async fn record_inbox_keyword_support(
    pool: &SqlitePool,
    account_id: &str,
    support: KeywordSupport,
) -> Result<(), String> {
    match support {
        KeywordSupport::NotAllowed => {
            record_workflow_sync_mode(
                pool,
                account_id,
                MODE_LOCAL_ONLY,
                Some(REASON_NO_CUSTOM_KEYWORDS),
            )
            .await
        }
        KeywordSupport::GmailHost => {
            record_workflow_sync_mode(pool, account_id, MODE_LOCAL_ONLY, Some(REASON_GMAIL_IMAP))
                .await
        }
        KeywordSupport::Allowed => {
            let refused: bool = sqlx::query_scalar(&format!(
                "SELECT EXISTS(SELECT 1 FROM account_workflow_sync s
                 WHERE s.account_id = ? AND s.reason = 'store_refused' AND {EFFECTIVE_LOCAL_ONLY})"
            ))
            .bind(account_id)
            .bind(process_started())
            .bind(retry_cutoff())
            .fetch_one(pool)
            .await
            .map_err(|err| err.to_string())?;
            if refused {
                return Ok(());
            }
            record_workflow_sync_mode(pool, account_id, MODE_SERVER, None).await
        }
    }
}

/// An account that keeps Done and Snooze on this device, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalOnlyAccount {
    pub account_id: String,
    pub reason: String,
}

/// Accounts whose verdict currently keeps Done and Snooze local.
pub async fn list_local_only_accounts(pool: &SqlitePool) -> Result<Vec<LocalOnlyAccount>, String> {
    Ok(sqlx::query_as::<_, (String, Option<String>)>(&format!(
        "SELECT s.account_id, s.reason FROM account_workflow_sync s
         JOIN accounts a ON a.id = s.account_id
         WHERE {EFFECTIVE_LOCAL_ONLY} ORDER BY s.account_id"
    ))
    .bind(process_started())
    .bind(retry_cutoff())
    .fetch_all(pool)
    .await
    .map_err(|err| err.to_string())?
    .into_iter()
    .map(|(account_id, reason)| LocalOnlyAccount {
        account_id,
        reason: reason.unwrap_or_else(|| REASON_NO_CUSTOM_KEYWORDS.into()),
    })
    .collect())
}

async fn account_is_local_only<'e, E>(executor: E, account_id: &str) -> Result<bool, String>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query_scalar(&format!(
        "SELECT EXISTS(SELECT 1 FROM account_workflow_sync s WHERE s.account_id = ? AND {EFFECTIVE_LOCAL_ONLY})"
    ))
    .bind(account_id)
    .bind(process_started())
    .bind(retry_cutoff())
    .fetch_one(executor)
    .await
    .map_err(|err| err.to_string())
}

/// After the first completed INBOX snapshot on this device. Before it, an
/// IMAP snooze set elsewhere is recorded but not adopted: a new device would
/// otherwise hide every thread a lost device left `$Satchel-Snoozed` on.
pub async fn mark_snapshot_baseline(pool: &SqlitePool, account_id: &str) -> Result<(), String> {
    sqlx::query(
        "INSERT INTO account_workflow_sync (account_id, mode, reason, checked_at, baseline_at)
         VALUES (?, 'server', NULL, ?, ?)
         ON CONFLICT(account_id) DO UPDATE SET baseline_at = excluded.baseline_at
         WHERE account_workflow_sync.baseline_at IS NULL",
    )
    .bind(account_id)
    .bind(chrono::Utc::now().to_rfc3339())
    .bind(chrono::Utc::now().to_rfc3339())
    .execute(pool)
    .await
    .map_err(|err| err.to_string())?;
    Ok(())
}

async fn has_snapshot_baseline<'e, E>(executor: E, account_id: &str) -> Result<bool, String>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM account_workflow_sync WHERE account_id = ? AND baseline_at IS NOT NULL)",
    )
    .bind(account_id)
    .fetch_one(executor)
    .await
    .map_err(|err| err.to_string())
}

#[derive(Debug, Clone)]
struct MarkerThreadRow {
    account_id: String,
    provider: String,
    recorded: WorkflowMarker,
}

/// The thread's account if it can carry markers at all. Account verdicts
/// are looked up once per batch.
async fn marker_thread(
    tx: &mut Transaction<'_, Sqlite>,
    thread_id: &str,
    local_only: &mut HashMap<String, bool>,
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
    if kind != "email" || !provider_carries_markers(&provider) {
        return Ok(None);
    }
    let excluded = match local_only.get(&account_id) {
        Some(excluded) => *excluded,
        None => {
            let excluded = account_is_local_only(&mut **tx, &account_id).await?;
            local_only.insert(account_id.clone(), excluded);
            excluded
        }
    };
    if excluded {
        return Ok(None);
    }
    Ok(Some(MarkerThreadRow {
        account_id,
        provider,
        recorded: WorkflowMarker::from_column(recorded.as_deref()),
    }))
}

/// [`marker_thread`] for many threads in one query, in the given order.
async fn marker_threads(
    tx: &mut Transaction<'_, Sqlite>,
    thread_ids: &[String],
    local_only: &mut HashMap<String, bool>,
) -> Result<Vec<(String, MarkerThreadRow)>, String> {
    let rows = sqlx::query_as::<_, (String, String, String, String, Option<String>)>(
        "SELECT t.id, t.account_id, a.provider, a.kind, t.remote_workflow_marker
         FROM json_each(?) j JOIN threads t ON t.id = j.value
         JOIN accounts a ON a.id = t.account_id
         ORDER BY j.key",
    )
    .bind(serde_json::to_string(thread_ids).map_err(|err| err.to_string())?)
    .fetch_all(&mut **tx)
    .await
    .map_err(|err| err.to_string())?;
    let mut threads = Vec::with_capacity(rows.len());
    for (thread_id, account_id, provider, kind, recorded) in rows {
        if kind != "email" || !provider_carries_markers(&provider) {
            continue;
        }
        let excluded = match local_only.get(&account_id) {
            Some(excluded) => *excluded,
            None => {
                let excluded = account_is_local_only(&mut **tx, &account_id).await?;
                local_only.insert(account_id.clone(), excluded);
                excluded
            }
        };
        if !excluded {
            threads.push((
                thread_id,
                MarkerThreadRow {
                    account_id,
                    provider,
                    recorded: WorkflowMarker::from_column(recorded.as_deref()),
                },
            ));
        }
    }
    Ok(threads)
}

/// Marker rows in these states that list any of `thread_ids`, found by
/// SQLite without parsing the others.
async fn marker_rows_with_threads(
    tx: &mut Transaction<'_, Sqlite>,
    statuses: &str,
    thread_ids: &[String],
) -> Result<Vec<(String, RemoteMutationPayload)>, String> {
    if thread_ids.is_empty() {
        return Ok(Vec::new());
    }
    // One thread, the common case: a byte search for its serialized key
    // skips every other row before any JSON is read, so the lookup costs the
    // same however many threads are already waiting.
    let needle = match thread_ids {
        [only] => Some(format!(
            "\"thread_id\":{}",
            serde_json::to_string(only).map_err(|err| err.to_string())?
        )),
        _ => None,
    };
    let wanted = serde_json::to_string(thread_ids).map_err(|err| err.to_string())?;
    let rows = sqlx::query_as::<_, (String, String)>(&format!(
        "SELECT id, payload FROM sync_queue
         WHERE action_type = 'remote_mutation' AND +status IN ({statuses})
           AND (? IS NULL OR instr(payload, ?) > 0)
           AND CASE WHEN json_valid(payload) THEN json_extract(payload, '$.kind') END = 'workflow_marker'
           AND EXISTS (
               SELECT 1 FROM json_each(CASE WHEN json_valid(payload) THEN payload ELSE '{{}}' END, '$.threads') queued
               WHERE json_extract(queued.value, '$.thread_id') IN (SELECT value FROM json_each(?))
           )
         ORDER BY rowid"
    ))
    .bind(needle.as_deref())
    .bind(needle.as_deref())
    .bind(wanted)
    .fetch_all(&mut **tx)
    .await
    .map_err(|err| err.to_string())?;
    Ok(rows
        .into_iter()
        .filter_map(|(id, payload)| serde_json::from_str(&payload).ok().map(|p| (id, p)))
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

/// Take these threads out of every marker row not sent yet; each touched
/// row is written once.
async fn withdraw_unsent(
    tx: &mut Transaction<'_, Sqlite>,
    thread_ids: &[String],
) -> Result<(), String> {
    let withdrawn = thread_ids
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    for (id, mut payload) in
        marker_rows_with_threads(tx, "'pending', 'scheduled'", thread_ids).await?
    {
        let RemoteMutationPayload::WorkflowMarker { threads, .. } = &mut payload else {
            continue;
        };
        threads.retain(|thread| !withdrawn.contains(thread.thread_id.as_str()));
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
    Ok(())
}

/// The marker of a row on its way to the server right now, per thread.
async fn markers_in_flight(
    tx: &mut Transaction<'_, Sqlite>,
    thread_ids: &[String],
) -> Result<HashMap<String, WorkflowMarker>, String> {
    let mut in_flight = HashMap::new();
    for (_, payload) in marker_rows_with_threads(tx, "'sending'", thread_ids).await? {
        if let RemoteMutationPayload::WorkflowMarker { threads, marker } = payload {
            for thread in threads {
                in_flight.insert(thread.thread_id, marker);
            }
        }
    }
    Ok(in_flight)
}

/// A fresh `pending` row with this marker that still has room. A row waiting
/// for a retry would hold new threads back, and notification rows have their
/// own lease.
async fn joinable_row(
    tx: &mut Transaction<'_, Sqlite>,
    account_id: &str,
    marker: &WorkflowMarker,
) -> Result<Option<(String, RemoteMutationPayload)>, String> {
    let row = sqlx::query_as::<_, (String, String)>(
        "SELECT id, payload FROM sync_queue
         WHERE account_id = ? AND action_type = 'remote_mutation' AND +status = 'pending'
           AND retry_count = 0 AND next_retry_at IS NULL AND id NOT LIKE 'notification:%'
           AND CASE WHEN json_valid(payload) THEN json_extract(payload, '$.kind') END = 'workflow_marker'
           AND json_extract(payload, '$.marker') = ?
           AND json_array_length(payload, '$.threads') < ?
         ORDER BY rowid DESC LIMIT 1",
    )
    .bind(account_id)
    .bind(marker.as_db_value())
    .bind(THREADS_PER_ROW as i64)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|err| err.to_string())?;
    Ok(row.and_then(|(id, payload)| serde_json::from_str(&payload).ok().map(|p| (id, p))))
}

/// The server copies of each thread, in message order. Local-only folders
/// have none.
async fn threads_refs(
    tx: &mut Transaction<'_, Sqlite>,
    thread_ids: &[&str],
) -> Result<HashMap<String, Vec<MessageRef>>, String> {
    let rows = sqlx::query_as::<_, (String, Option<String>, String, Option<i64>)>(
        "SELECT m.thread_id, m.remote_id, COALESCE(f.remote_id, f.full_path) AS source_folder,
                f.uidvalidity AS source_uidvalidity
         FROM json_each(?) j JOIN messages m ON m.thread_id = j.value
         JOIN folders f ON f.id = m.folder_id
         WHERE f.folder_type != 'local'
         ORDER BY j.key, m.id",
    )
    .bind(serde_json::to_string(thread_ids).map_err(|err| err.to_string())?)
    .fetch_all(&mut **tx)
    .await
    .map_err(|err| err.to_string())?;
    let mut refs = HashMap::<String, Vec<MessageRef>>::new();
    for (thread_id, remote_id, source_folder, source_uidvalidity) in rows {
        let Some(remote_id) = remote_id.filter(|id| !id.trim().is_empty()) else {
            continue;
        };
        refs.entry(thread_id).or_default().push(MessageRef {
            remote_id,
            source_folder: Some(source_folder),
            source_uidvalidity: source_uidvalidity.and_then(|n| u32::try_from(n).ok()),
        });
    }
    Ok(refs)
}

/// One thread to queue: its account, the marker and an optional guard.
struct Planned {
    account_id: String,
    thread_id: String,
    marker: WorkflowMarker,
    expected: Option<WorkflowMarker>,
}

/// Write planned entries, filling a waiting row of the same marker first,
/// then new rows of [`THREADS_PER_ROW`]. Every row is written once.
async fn write_planned(
    tx: &mut Transaction<'_, Sqlite>,
    queue: &SyncQueue,
    planned: Vec<Planned>,
    queue_prefix: Option<&str>,
) -> Result<Vec<String>, String> {
    // Grouped in first-seen order, so rows keep the callers' order.
    let mut groups = BTreeMap::<(usize, String, String), Vec<MarkedThread>>::new();
    let mut order = HashMap::<(String, String), usize>::new();
    let mut markers = HashMap::<String, WorkflowMarker>::new();
    let mut thread_ids: Vec<&str> = planned.iter().map(|plan| plan.thread_id.as_str()).collect();
    thread_ids.sort_unstable();
    thread_ids.dedup();
    let refs = threads_refs(tx, &thread_ids).await?;
    for plan in planned {
        let refs = refs.get(&plan.thread_id).cloned().unwrap_or_default();
        if refs.is_empty() {
            continue;
        }
        let key = (plan.account_id.clone(), plan.marker.as_db_value());
        let next = order.len();
        let position = *order.entry(key.clone()).or_insert(next);
        markers.insert(key.1.clone(), plan.marker);
        groups
            .entry((position, key.0, key.1))
            .or_default()
            .push(MarkedThread {
                thread_id: plan.thread_id,
                refs,
                expected: plan.expected,
            });
    }

    let mut ids = Vec::new();
    for ((_, account_id, marker_key), mut entries) in groups {
        let marker = markers[&marker_key];
        if let Some(prefix) = queue_prefix {
            // Notification rows keep a stable id and never share.
            let mut action = QueuedAction::new(
                account_id.clone(),
                ACTION_REMOTE_MUTATION,
                serde_json::to_value(RemoteMutationPayload::WorkflowMarker {
                    threads: entries,
                    marker,
                })
                .map_err(|err| err.to_string())?,
            );
            action.id = stable_queue_id(prefix, "workflow-marker", &account_id);
            queue.enqueue_in_transaction(tx, &action).await?;
            ids.push(action.id);
            continue;
        }
        if let Some((id, mut payload)) = joinable_row(tx, &account_id, &marker).await? {
            if let RemoteMutationPayload::WorkflowMarker { threads, .. } = &mut payload {
                let room = THREADS_PER_ROW
                    .saturating_sub(threads.len())
                    .min(entries.len());
                threads.extend(entries.drain(..room));
                store_payload(tx, &id, &payload).await?;
                ids.push(id);
            }
        }
        while !entries.is_empty() {
            let take = entries.len().min(THREADS_PER_ROW);
            let action = QueuedAction::new(
                account_id.clone(),
                ACTION_REMOTE_MUTATION,
                serde_json::to_value(RemoteMutationPayload::WorkflowMarker {
                    threads: entries.drain(..take).collect(),
                    marker,
                })
                .map_err(|err| err.to_string())?,
            );
            queue.enqueue_in_transaction(tx, &action).await?;
            ids.push(action.id);
        }
    }
    Ok(ids)
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
    enqueue_workflow_markers_in_transaction(
        tx,
        queue,
        std::slice::from_ref(&thread_id.to_string()),
        marker,
        queue_prefix,
    )
    .await
}

/// Queue the same marker for many threads in one pass: each account verdict
/// is read once and each row written once. Archiving from a rule or sweep
/// uses this ahead of the moves, inside the same transaction.
pub async fn enqueue_workflow_markers_in_transaction(
    tx: &mut Transaction<'_, Sqlite>,
    queue: &SyncQueue,
    thread_ids: &[String],
    marker: &WorkflowMarker,
    queue_prefix: Option<&str>,
) -> Result<Vec<String>, String> {
    let mut seen = HashSet::new();
    let thread_ids = thread_ids
        .iter()
        .filter(|id| seen.insert(id.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    let threads = marker_threads(tx, &thread_ids, &mut HashMap::new()).await?;
    let ids = threads.iter().map(|(id, _)| id.clone()).collect::<Vec<_>>();
    withdraw_unsent(tx, &ids).await?;
    let in_flight = markers_in_flight(tx, &ids).await?;
    let planned = threads
        .into_iter()
        .filter_map(|(thread_id, row)| {
            let marker = if row.provider.eq_ignore_ascii_case("imap") {
                marker.after(&row.recorded)
            } else {
                *marker
            };
            // The server holds what was recorded, unless a row is on its way
            // there right now; then the new marker has to follow it.
            (row.recorded != marker || in_flight.contains_key(&thread_id)).then_some(Planned {
                account_id: row.account_id,
                thread_id,
                marker,
                expected: None,
            })
        })
        .collect();
    write_planned(tx, queue, planned, queue_prefix).await
}

/// The snooze worker woke this thread. Cancel a snooze that never left, and
/// clear the one on the server only if it is the snooze this device set
/// (it knows its time) and nobody replaced it since: compare-and-clear, so a
/// newer snooze from another device survives a device that woke late. A
/// snooze push still in flight is the one to clear once it lands.
///
/// IMAP only. Gmail labels carry the time, every device resolves a passed
/// wake label to Inbox by itself, and expired labels are cleaned up with
/// their messages (`GmailProvider::delete_expired_wake_labels`).
pub async fn enqueue_wake_clear_in_transaction(
    tx: &mut Transaction<'_, Sqlite>,
    queue: &SyncQueue,
    thread_id: &str,
) -> Result<Vec<String>, String> {
    let Some(row) = marker_thread(tx, thread_id, &mut HashMap::new()).await? else {
        return Ok(Vec::new());
    };
    let ids = vec![thread_id.to_string()];
    withdraw_unsent(tx, &ids).await?;
    if !row.provider.eq_ignore_ascii_case("imap") {
        return Ok(Vec::new());
    }
    let own = |marker: &WorkflowMarker| {
        matches!(
            marker,
            WorkflowMarker::Snoozed(Snooze { wake: Some(_), .. })
        )
    };
    let expected = match markers_in_flight(tx, &ids).await?.remove(thread_id) {
        Some(sending) if own(&sending) => sending,
        _ if own(&row.recorded) => row.recorded,
        _ => return Ok(Vec::new()),
    };
    write_planned(
        tx,
        queue,
        vec![Planned {
            account_id: row.account_id,
            thread_id: thread_id.to_string(),
            marker: WorkflowMarker::None,
            expected: Some(expected),
        }],
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
/// Only a definite refusal in INBOX turns the account local-only, and an
/// accepted INBOX store turns it back.
pub async fn apply_marker_report(
    pool: &SqlitePool,
    account_id: &str,
    threads: &[MarkedThread],
    marker: &WorkflowMarker,
    report: &WorkflowMarkerReport,
) -> Result<(), String> {
    for (thread, outcome) in threads.iter().zip(&report.targets) {
        if *outcome != WorkflowMarkerWrite::Applied {
            continue;
        }
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
    } else if report.applied_in_inbox {
        record_workflow_sync_mode(pool, account_id, MODE_SERVER, None).await?;
    }
    Ok(())
}

/// Whether a workflow change of this thread is still on its way to the
/// server: a marker or a move not yet confirmed.
async fn local_change_pending(
    tx: &mut Transaction<'_, Sqlite>,
    thread_id: &str,
) -> Result<bool, String> {
    let ids = vec![thread_id.to_string()];
    if !marker_rows_with_threads(tx, "'pending', 'scheduled', 'sending'", &ids)
        .await?
        .is_empty()
    {
        return Ok(true);
    }
    // The thread index (migration 0160) finds its moves without reading
    // any other payload.
    let payloads = sqlx::query_scalar::<_, String>(&format!(
        "SELECT payload FROM sync_queue
         WHERE action_type = 'remote_mutation' AND +status IN ('pending', 'scheduled', 'sending')
           AND {QUEUED_MOVE_THREAD_ID} = ?"
    ))
    .bind(thread_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(|err| err.to_string())?;
    Ok(payloads
        .iter()
        .any(|payload| remote_move_thread_id_from_json(payload).as_deref() == Some(thread_id)))
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
    let foreign_imap_snooze = matches!(server, WorkflowMarker::Snoozed(Snooze { wake: None, .. }));
    let decision =
        if foreign_imap_snooze && !has_snapshot_baseline(&mut *tx, &row.account_id).await? {
            // First look at this mailbox from this device: an IMAP snooze it
            // finds may be years old. Seen, not adopted.
            PullDecision::Keep
        } else {
            pull_decision(
                &seen,
                &server,
                local_state,
                row.snooze_until.as_deref(),
                now,
            )
        };
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
