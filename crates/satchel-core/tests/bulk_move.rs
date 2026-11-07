//! Bulk moves: archive, trash, spam and folder moves of many threads.
//!
//! `bulk_moves_match_the_recorded_results` replays a mixed fixture and
//! compares queue rows, Undo receipts and folder state with
//! `fixtures/bulk_move.golden.json`, recorded from the per-thread
//! implementation that came before the batched one. In one batch the result
//! is identical; in smaller batches only the Done marker rows may sit later
//! in the queue, never after their threads' moves. Rewrite it only for an
//! intended change in behaviour, with
//! `BULK_MOVE_WRITE_GOLDEN=1 cargo test -p satchel-core --test bulk_move -- --ignored write_bulk_move_golden`.
//!
//! `bulk_move_wall_clock` is ignored by default: timings swing with machine
//! load, so it measures, it does not gate. Run with
//! `cargo test -p satchel-core --release --test bulk_move -- --ignored --nocapture`.

// Snapshot rows decode straight into wide tuples, one per table.
#![allow(clippy::type_complexity)]

use std::time::Instant;

mod bulk_move_fixture;

use bulk_move_fixture::{pool, seed, ACCOUNTS};
use satchel_core::mail_actions::{self, MoveTarget};
use satchel_core::sync::queue::SyncQueue;
use sqlx::SqlitePool;

#[tokio::test]
#[ignore = "benchmark; run explicitly"]
async fn bulk_move_wall_clock() {
    for target in ["archive", "trash", "folder"] {
        for n in [200, 1_000, 2_000] {
            let pool = pool().await;
            let ids = seed(&pool, n).await;
            let queue = SyncQueue::new();
            let started = Instant::now();
            match target {
                "archive" => {
                    mail_actions::move_threads(&pool, &queue, &ids, MoveTarget::Archive)
                        .await
                        .expect("archive");
                }
                "trash" => {
                    mail_actions::trash_threads(&pool, &queue, &ids)
                        .await
                        .expect("trash");
                }
                _ => {
                    let ids: Vec<String> = ids
                        .iter()
                        .enumerate()
                        .filter(|(index, _)| index % ACCOUNTS.len() == 0)
                        .map(|(_, id)| id.clone())
                        .collect();
                    mail_actions::move_threads(
                        &pool,
                        &queue,
                        &ids,
                        MoveTarget::Folder("imap-a-projects"),
                    )
                    .await
                    .expect("folder");
                }
            }
            let elapsed = started.elapsed();
            let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_queue")
                .fetch_one(&pool)
                .await
                .unwrap();
            println!("{target:>8} n={n:>5}: {elapsed:>10.2?}  queue rows={rows}");
        }
    }
    for n in [200, 1_000, 2_000] {
        use satchel_core::workflow::{
            states::WorkflowState,
            transitions::{transition_threads_inner, BatchTransitionEntry, BatchTransitionInput},
        };
        let pool = pool().await;
        let ids = seed(&pool, n).await;
        let queue = SyncQueue::new();
        let started = Instant::now();
        let result = transition_threads_inner(
            &pool,
            &queue,
            BatchTransitionInput {
                threads: ids
                    .iter()
                    .map(|id| BatchTransitionEntry {
                        thread_id: id.clone(),
                        from: WorkflowState::Inbox,
                    })
                    .collect(),
                to: WorkflowState::Done,
                snooze_until: None,
            },
        )
        .await;
        let elapsed = started.elapsed();
        assert_eq!(result.applied.len(), n, "{:?}", result.failed.first());
        println!("    done n={n:>5}: {elapsed:>10.2?}");
    }
}

/// Everything a move leaves behind, with generated ids replaced by their order
/// of appearance and wall-clock timestamps left out.
///
/// Each prefix counts on its own — `m` for Done marker rows, `q` for other
/// queue rows, `r` for receipts — so where marker rows fall among the others
/// does not rename the rest.
struct Normalizer {
    ids: std::collections::HashMap<String, String>,
    counters: std::collections::HashMap<String, usize>,
}

impl Normalizer {
    fn new() -> Self {
        Self {
            ids: std::collections::HashMap::new(),
            counters: std::collections::HashMap::new(),
        }
    }

    fn id(&mut self, prefix: &str, raw: &str) -> String {
        if let Some(name) = self.ids.get(raw) {
            return name.clone();
        }
        let counter = self.counters.entry(prefix.to_string()).or_default();
        let name = format!("{prefix}{counter}");
        *counter += 1;
        self.ids.insert(raw.to_string(), name.clone());
        name
    }
}

/// The name prefix of a queue row of this payload kind.
fn queue_prefix(kind: Option<&str>) -> &'static str {
    if kind == Some("workflow_marker") {
        "m"
    } else {
        "q"
    }
}

/// Queued ids a move returned, named by the kind of row each one is.
async fn queued_names(
    pool: &SqlitePool,
    normalizer: &mut Normalizer,
    result: Result<Vec<String>, String>,
) -> serde_json::Value {
    let ids = match result {
        Ok(ids) => ids,
        Err(error) => return serde_json::json!({"error": error}),
    };
    let mut names = Vec::new();
    for id in ids {
        let kind: Option<String> = sqlx::query_scalar(
            "SELECT json_extract(payload, '$.kind') FROM sync_queue WHERE id = ?",
        )
        .bind(&id)
        .fetch_optional(pool)
        .await
        .unwrap()
        .flatten();
        names.push(normalizer.id(queue_prefix(kind.as_deref()), &id));
    }
    serde_json::json!({"queued": names})
}

/// Rows keyed by generated ids, ordered by their normalized form.
fn sort_rows(rows: &mut [serde_json::Value]) {
    rows.sort_by_key(|row| row.to_string());
}

async fn snapshot(pool: &SqlitePool, normalizer: &mut Normalizer) -> serde_json::Value {
    use serde_json::json;
    let queue: Vec<(String, String, String, String, Option<String>, Option<String>)> =
        sqlx::query_as(
            "SELECT id, account_id, action_type, payload, status, scheduled_for FROM sync_queue ORDER BY rowid",
        )
        .fetch_all(pool)
        .await
        .unwrap();
    let queue: Vec<_> = queue
        .into_iter()
        .map(|(id, account, action_type, payload, status, scheduled)| {
            json!({
                "id": normalizer.id(
                    queue_prefix(serde_json::from_str::<serde_json::Value>(&payload).unwrap()["kind"].as_str()),
                    &id,
                ),
                "account": account,
                "type": action_type,
                "payload": serde_json::from_str::<serde_json::Value>(&payload).unwrap(),
                "status": status,
                "scheduled": scheduled.is_some(),
            })
        })
        .collect();
    let receipts: Vec<(String, String)> =
        sqlx::query_as("SELECT id, state FROM mail_action_receipts ORDER BY id")
            .fetch_all(pool)
            .await
            .unwrap();
    let mut receipts: Vec<_> = receipts
        .into_iter()
        .map(|(id, state)| json!([normalizer.id("r", &id), state]))
        .collect();
    sort_rows(&mut receipts);
    let receipt_threads: Vec<(String, String, String, String, String, Option<String>)> =
        sqlx::query_as(
            "SELECT receipt_id, thread_id, account_id, workflow_state, board_column, snooze_until
             FROM mail_action_threads ORDER BY thread_id, receipt_id",
        )
        .fetch_all(pool)
        .await
        .unwrap();
    let mut receipt_threads: Vec<_> = receipt_threads
        .into_iter()
        .map(|(receipt, thread, account, state, column, snooze)| {
            json!([
                normalizer.id("r", &receipt),
                thread,
                account,
                state,
                column,
                snooze
            ])
        })
        .collect();
    sort_rows(&mut receipt_threads);
    let receipt_messages: Vec<(String, String, String, String)> = sqlx::query_as(
        "SELECT receipt_id, message_id, original_folder_id, target_folder_id
         FROM mail_action_messages ORDER BY message_id, receipt_id",
    )
    .fetch_all(pool)
    .await
    .unwrap();
    let mut receipt_messages: Vec<_> = receipt_messages
        .into_iter()
        .map(|(receipt, message, from, to)| {
            json!([normalizer.id("r", &receipt), message, from, to])
        })
        .collect();
    sort_rows(&mut receipt_messages);
    let receipt_queue: Vec<(String, String)> =
        sqlx::query_as("SELECT receipt_id, action_id FROM mail_action_queue")
            .fetch_all(pool)
            .await
            .unwrap();
    let mut receipt_queue: Vec<_> = receipt_queue
        .into_iter()
        .map(|(receipt, action)| (normalizer.id("r", &receipt), normalizer.id("q", &action)))
        .collect();
    receipt_queue.sort();
    let messages: Vec<(String, String, Option<String>, i64, String)> = sqlx::query_as(
        "SELECT id, folder_id, pending_folder_id, is_read, COALESCE(thread_id, '') FROM messages ORDER BY id",
    )
    .fetch_all(pool)
    .await
    .unwrap();
    let threads: Vec<(String, String, String, Option<String>, i64, i64, i64, i64, Option<String>, Option<String>, String, bool)> =
        sqlx::query_as(
            "SELECT id, workflow_state, COALESCE(board_column, ''), snooze_until, message_count, unread_count,
                    is_starred, has_attachments, subject, snippet, last_message_date,
                    workflow_changed_at IS NOT NULL
             FROM threads ORDER BY id",
        )
        .fetch_all(pool)
        .await
        .unwrap();
    let folders: Vec<(String, String, String, String, String, Option<String>, i64, i64)> =
        sqlx::query_as(
            "SELECT id, account_id, name, full_path, folder_type, remote_id, thread_count, thread_unread_count
             FROM folders ORDER BY id",
        )
        .fetch_all(pool)
        .await
        .unwrap();
    let thread_folders: Vec<(String, String, i64, i64, i64)> = sqlx::query_as(
        "SELECT folder_id, thread_id, message_count, unread_count, visible FROM thread_folders ORDER BY folder_id, thread_id",
    )
    .fetch_all(pool)
    .await
    .unwrap();
    json!({
        "queue": queue,
        "receipts": receipts,
        "receipt_threads": receipt_threads,
        "receipt_messages": receipt_messages,
        "receipt_queue": receipt_queue,
        "messages": messages,
        "threads": threads,
        "folders": folders,
        "thread_folders": thread_folders,
    })
}

const GOLDEN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/bulk_move.golden.json"
);

async fn insert_flags(
    pool: &SqlitePool,
    id: &str,
    account: &str,
    refs: serde_json::Value,
    read: Option<bool>,
    starred: Option<bool>,
    status: &str,
) {
    sqlx::query(
        "INSERT INTO sync_queue (id, account_id, action_type, payload, status, created_at, updated_at)
         VALUES (?, ?, 'remote_mutation', ?, ?, '2026-01-01', '2026-01-01')",
    )
    .bind(id)
    .bind(account)
    .bind(serde_json::json!({"kind": "flags", "refs": refs, "read": read, "starred": starred, "superseded": []}).to_string())
    .bind(status)
    .execute(pool)
    .await
    .unwrap();
}

/// Mixed accounts, folders, Gmail labels, IMAP and POP3, queued flags the
/// moves supersede, a thread entirely in Trash, one in Spam, one without
/// messages.
async fn scenario_pool() -> SqlitePool {
    let pool = pool().await;
    seed(&pool, 28).await;
    for (id, account, remote, kind, uidvalidity) in [
        ("imap-b-spam", "imap-b", "Junk", "spam", Some(11)),
        ("gmail-trash", "gmail", "TRASH", "trash", None),
    ] {
        sqlx::query(
            "INSERT INTO folders (id, account_id, name, full_path, folder_type, remote_id, uidvalidity, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, '2026-01-01', '2026-01-01')",
        )
        .bind(id).bind(account).bind(remote).bind(remote).bind(kind).bind(remote).bind(uidvalidity)
        .execute(&pool).await.unwrap();
    }
    for (thread, account) in [
        ("spam-1", "imap-b"),
        ("trashed-1", "gmail"),
        ("empty-1", "imap-a"),
    ] {
        sqlx::query(
            "INSERT INTO threads (id, account_id, subject, last_message_date, workflow_state, board_column, created_at, updated_at)
             VALUES (?, ?, 'Other', '2026-01-02', 'inbox', 'inbox', '2026-01-01', '2026-01-01')",
        )
        .bind(thread).bind(account).execute(&pool).await.unwrap();
    }
    for (id, account, folder, thread, remote) in [
        ("spam-1-m0", "imap-b", "imap-b-spam", "spam-1", "501"),
        ("spam-1-m1", "imap-b", "imap-b-spam", "spam-1", "502"),
        ("spam-1-m2", "imap-b", "imap-b-inbox", "spam-1", "503"),
        ("trashed-1-m0", "gmail", "gmail-trash", "trashed-1", "gt0"),
        ("trashed-1-m1", "gmail", "gmail-trash", "trashed-1", "gt1"),
    ] {
        sqlx::query(
            "INSERT INTO messages (id, account_id, folder_id, thread_id, from_address, to_addresses, subject, date, remote_id, created_at, updated_at)
             VALUES (?, ?, ?, ?, 'other@example.test', '[]', 'Other', '2026-01-02', ?, '2026-01-01', '2026-01-01')",
        )
        .bind(id).bind(account).bind(folder).bind(thread).bind(remote)
        .execute(&pool).await.unwrap();
    }
    let imap = |remote: &str| serde_json::json!({"remote_id": remote, "source_folder": "INBOX", "source_uidvalidity": 7});
    insert_flags(
        &pool,
        "pre-flags-read",
        "imap-a",
        serde_json::json!([imap("13")]),
        Some(false),
        None,
        "pending",
    )
    .await;
    insert_flags(
        &pool,
        "pre-flags-both",
        "imap-a",
        serde_json::json!([imap("14"), imap("999")]),
        Some(false),
        Some(true),
        "failed",
    )
    .await;
    insert_flags(
        &pool,
        "pre-flags-sending",
        "imap-a",
        serde_json::json!([imap("25")]),
        Some(false),
        None,
        "sending",
    )
    .await;
    insert_flags(&pool, "pre-flags-gmail", "gmail", serde_json::json!([{"remote_id": "g2x0", "source_folder": "INBOX", "source_uidvalidity": null}]), Some(false), None, "pending").await;
    pool
}

fn thread_range(range: std::ops::Range<usize>) -> Vec<String> {
    range.map(|n| format!("t{n:05}")).collect()
}

/// Every step's return value, then the final state.
async fn run_scenario() -> serde_json::Value {
    use serde_json::json;
    let pool = scenario_pool().await;
    let queue = SyncQueue::new();
    let mut normalizer = Normalizer::new();
    let mut steps = Vec::new();
    let receipt = |normalizer: &mut Normalizer,
                   result: Result<mail_actions::TrashReceipt, String>| {
        match result {
            Ok(receipt) => json!({
                "undo_token": receipt.undo_token.map(|token| normalizer.id("r", &token)),
                "undo_thread_ids": receipt.undo_thread_ids,
                "permanently_deleted": receipt.permanently_deleted,
            }),
            Err(error) => json!({"error": error}),
        }
    };

    let mut archive = thread_range(0..12);
    archive.push("empty-1".into());
    archive.push("t00003".into());
    let result = mail_actions::move_threads(&pool, &queue, &archive, MoveTarget::Archive).await;
    steps.push(queued_names(&pool, &mut normalizer, result).await);

    let mut trash = thread_range(12..18);
    trash.push("trashed-1".into());
    let result = mail_actions::trash_threads(&pool, &queue, &trash).await;
    let trash_token = result.as_ref().ok().and_then(|r| r.undo_token.clone());
    steps.push(receipt(&mut normalizer, result));

    let result = mail_actions::move_spam_with_receipt(&pool, &queue, &thread_range(18..20)).await;
    let spam_token = result.as_ref().ok().and_then(|r| r.undo_token.clone());
    steps.push(receipt(&mut normalizer, result));

    let result =
        mail_actions::move_threads(&pool, &queue, &["spam-1".into()], MoveTarget::Inbox).await;
    steps.push(queued_names(&pool, &mut normalizer, result).await);

    let result = mail_actions::move_threads(
        &pool,
        &queue,
        &["t00020".into(), "t00024".into()],
        MoveTarget::Folder("imap-a-projects"),
    )
    .await;
    steps.push(queued_names(&pool, &mut normalizer, result).await);

    let before = snapshot(&pool, &mut normalizer).await;
    for (ids, target) in [
        (
            vec!["t00025".to_string(), "t00000".into()],
            MoveTarget::Archive,
        ),
        (
            vec!["t00021".to_string(), "t00022".into()],
            MoveTarget::Folder("imap-a-projects"),
        ),
        (
            vec!["t00026".to_string(), "missing".into()],
            MoveTarget::Trash,
        ),
    ] {
        let result = mail_actions::move_threads(&pool, &queue, &ids, target).await;
        steps.push(queued_names(&pool, &mut normalizer, result).await);
        assert_eq!(
            snapshot(&pool, &mut normalizer).await,
            before,
            "a refused move changed nothing"
        );
    }

    // A rule moves a thread's Inbox messages inside the rule run's own
    // transaction; its Sent copy stays where it is.
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
    let result = mail_actions::move_inbox_messages_in_transaction(
        &mut tx,
        &queue,
        "t00025",
        "imap-b-projects",
    )
    .await;
    tx.commit().await.unwrap();
    steps.push(queued_names(&pool, &mut normalizer, result).await);

    let mut undone = Vec::new();
    for token in [trash_token, spam_token] {
        let token = token.expect("an undo token");
        undone.push(
            match mail_actions::undo::undo_trash(&pool, &queue, &token).await {
                Ok(()) => json!("undone"),
                Err(error) => json!({"error": error}),
            },
        );
    }
    let after_undo = snapshot(&pool, &mut normalizer).await;

    json!({"steps": steps, "state": before, "undo": undone, "after_undo": after_undo})
}

#[tokio::test]
#[ignore = "writes the golden file; run with BULK_MOVE_WRITE_GOLDEN=1"]
async fn write_bulk_move_golden() {
    if std::env::var_os("BULK_MOVE_WRITE_GOLDEN").is_none() {
        return;
    }
    let value = run_scenario().await;
    std::fs::write(GOLDEN, serde_json::to_string_pretty(&value).unwrap() + "\n").unwrap();
}

/// The same result with Done marker rows taken out of the queue order.
///
/// A move in several batches queues each batch's markers ahead of that
/// batch's moves, and a later batch fills the marker row an earlier one left
/// waiting. Rows therefore hold the same threads but can sit later in the
/// queue than one up-front enqueue puts them; every other row keeps its
/// place.
fn without_marker_positions(value: &serde_json::Value) -> serde_json::Value {
    use serde_json::{json, Value};
    let mut value = value.clone();
    for state in ["state", "after_undo"] {
        let queue = value[state]["queue"].as_array().unwrap().clone();
        let (mut markers, rows): (Vec<Value>, Vec<Value>) = queue
            .into_iter()
            .partition(|row| row["payload"]["kind"] == "workflow_marker");
        sort_rows(&mut markers);
        value[state]["queue"] = json!(rows);
        value[state]["markers"] = json!(markers);
    }
    for step in value["steps"].as_array_mut().unwrap() {
        let Some(queued) = step.get("queued").and_then(Value::as_array).cloned() else {
            continue;
        };
        let (mut markers, rows): (Vec<Value>, Vec<Value>) = queued
            .into_iter()
            .partition(|id| id.as_str().unwrap().starts_with('m'));
        sort_rows(&mut markers);
        step["queued"] = json!(rows);
        step["markers"] = json!(markers);
    }
    value
}

/// Every thread's Done marker sits ahead of its moves: MOVE carries the
/// keyword along, and the source UID is gone afterwards.
fn assert_markers_lead_moves(state: &serde_json::Value) {
    let queue = state["queue"].as_array().unwrap();
    let mut marked = std::collections::HashMap::new();
    for (position, row) in queue.iter().enumerate() {
        if row["payload"]["kind"] == "workflow_marker" {
            for thread in row["payload"]["threads"].as_array().unwrap() {
                marked
                    .entry(thread["thread_id"].as_str().unwrap().to_string())
                    .or_insert(position);
            }
        }
    }
    for (position, row) in queue.iter().enumerate() {
        if row["payload"]["kind"] != "workflow_move" {
            continue;
        }
        let thread = row["payload"]["workflow_thread_id"].as_str().unwrap();
        if let Some(marker) = marked.get(thread) {
            assert!(
                *marker < position,
                "{thread}: its move is queued ahead of its marker"
            );
        }
    }
}

#[tokio::test]
async fn bulk_moves_match_the_recorded_results() {
    let golden: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(GOLDEN).expect("golden file"))
            .expect("golden json");
    // One batch for the whole fixture: exactly what main recorded.
    let replayed = run_scenario().await;
    assert!(
        replayed == golden,
        "one batch differs from the recorded results:\n{}",
        serde_json::to_string_pretty(&replayed).unwrap()
    );
    assert_markers_lead_moves(&golden["state"]);
    // Every thread its own batch, and uneven batches: the same rows, receipts
    // and folders; marker rows may sit later, never after their threads' moves.
    for batch in [1, 3, 5] {
        mail_actions::set_move_batch_threads_for_tests(Some(batch));
        let replayed = run_scenario().await;
        mail_actions::set_move_batch_threads_for_tests(None);
        assert_markers_lead_moves(&replayed["state"]);
        assert!(
            without_marker_positions(&replayed) == without_marker_positions(&golden),
            "batches of {batch} differ from the recorded results:\n{}",
            serde_json::to_string_pretty(&replayed).unwrap()
        );
    }
}
