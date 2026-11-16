//! A bulk move a later batch stops: the batches before stay applied, the
//! caller learns which threads moved and which stayed, Undo covers exactly
//! the moved ones, and nothing of the rest changed — not even a Done marker.
//!
//! A trigger fails the batch, as a busy database, a target folder sync
//! removed or a move another action queued in between would.

mod bulk_move_fixture;

use bulk_move_fixture::{pool, seed};
use satchel_core::mail_actions::{self, MoveTarget};
use satchel_core::sync::queue::SyncQueue;
use sqlx::SqlitePool;

/// Every message write of `thread` fails, as a locked database would.
async fn fail_writes_of(pool: &SqlitePool, thread: &str) {
    sqlx::query(&format!(
        "CREATE TRIGGER stop_bulk_move BEFORE UPDATE ON messages WHEN new.thread_id = '{thread}'
         BEGIN SELECT RAISE(ABORT, 'database is locked'); END"
    ))
    .execute(pool)
    .await
    .unwrap();
}

async fn workflow_state(pool: &SqlitePool, thread: &str) -> String {
    sqlx::query_scalar("SELECT workflow_state FROM threads WHERE id = ?")
        .bind(thread)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// Threads any queued row names, by payload kind.
async fn queued_threads(pool: &SqlitePool, kind: &str) -> Vec<String> {
    let mut threads: Vec<String> = sqlx::query_scalar(
        "SELECT COALESCE(json_extract(payload, '$.workflow_thread_id'), t.value ->> '$.thread_id')
         FROM sync_queue LEFT JOIN json_each(payload, '$.threads') t
         WHERE json_extract(payload, '$.kind') = ?",
    )
    .bind(kind)
    .fetch_all(pool)
    .await
    .unwrap();
    threads.sort();
    threads
}

/// Untouched: still in the Inbox, read state and folders as seeded.
async fn assert_untouched(pool: &SqlitePool, thread: &str) {
    assert_eq!(workflow_state(pool, thread).await, "inbox", "{thread}");
    let moved: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM messages WHERE thread_id = ?
           AND (pending_folder_id IS NOT NULL OR (is_read = 1 AND is_sent = 0))",
    )
    .bind(thread)
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(moved, 0, "{thread} kept its messages where they were");
}

fn ids(range: std::ops::Range<usize>) -> Vec<String> {
    range.map(|n| format!("t{n:05}")).collect()
}

#[tokio::test]
async fn an_archive_a_later_batch_stops_reports_what_moved_and_what_stayed() {
    let pool = pool().await;
    let threads = seed(&pool, 6).await;
    // Batches of two: t0+t1 commit, t2+t3 fail on t3's messages.
    fail_writes_of(&pool, "t00003").await;
    let queue = SyncQueue::new();
    mail_actions::set_move_batch_threads_for_tests(Some(2));
    let outcome =
        mail_actions::move_threads_reporting(&pool, &queue, &threads, MoveTarget::Archive)
            .await
            .expect("the first batch committed");
    mail_actions::set_move_batch_threads_for_tests(None);

    let stopped = outcome.stopped_early.expect("stopped early");
    assert_eq!(stopped.applied, ids(0..2));
    assert_eq!(stopped.remaining, ids(2..6));
    assert_eq!(stopped.reason_key, "backend.errors.mailMoveInterrupted");
    for thread in ids(0..2) {
        assert_eq!(workflow_state(&pool, &thread).await, "done");
    }
    for thread in ids(2..6) {
        assert_untouched(&pool, &thread).await;
    }
    // Only what moved has its provider move, its read flags and its Done
    // marker; t1 is imap-b, t0 imap-a.
    assert_eq!(queued_threads(&pool, "workflow_move").await, ids(0..2));
    assert_eq!(queued_threads(&pool, "workflow_marker").await, ids(0..2));
    // Every reported id names a row that is there.
    for id in &outcome.queued {
        let present: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sync_queue WHERE id = ?)")
                .bind(id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(present, "{id}");
    }
}

#[tokio::test]
async fn the_strict_move_reports_a_stop_as_its_reason() {
    let pool = pool().await;
    let threads = seed(&pool, 4).await;
    fail_writes_of(&pool, "t00002").await;
    let queue = SyncQueue::new();
    mail_actions::set_move_batch_threads_for_tests(Some(2));
    let result = mail_actions::move_threads(&pool, &queue, &threads, MoveTarget::Archive).await;
    mail_actions::set_move_batch_threads_for_tests(None);
    assert_eq!(
        result,
        Err("backend.errors.mailMoveInterrupted".to_string())
    );
    assert_eq!(workflow_state(&pool, "t00000").await, "done");
    assert_untouched(&pool, "t00002").await;
}

#[tokio::test]
async fn a_trash_a_later_batch_stops_keeps_an_undo_for_exactly_what_moved() {
    let pool = pool().await;
    let threads = seed(&pool, 6).await;
    fail_writes_of(&pool, "t00004").await;
    let queue = SyncQueue::new();
    mail_actions::set_move_batch_threads_for_tests(Some(2));
    let receipt = mail_actions::trash_threads(&pool, &queue, &threads)
        .await
        .expect("the first batches committed");
    mail_actions::set_move_batch_threads_for_tests(None);

    let stopped = receipt.stopped_early.clone().expect("stopped early");
    assert_eq!(stopped.applied, ids(0..4));
    assert_eq!(stopped.remaining, ids(4..6));
    assert_eq!(receipt.undo_thread_ids, ids(0..4));
    let token = receipt.undo_token.clone().expect("an Undo for what moved");
    let covered: Vec<String> = sqlx::query_scalar(
        "SELECT thread_id FROM mail_action_threads WHERE receipt_id = ? ORDER BY thread_id",
    )
    .bind(&token)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(covered, ids(0..4));
    for thread in ids(4..6) {
        assert_untouched(&pool, &thread).await;
    }

    // Undo puts back exactly what moved.
    sqlx::query("DROP TRIGGER stop_bulk_move")
        .execute(&pool)
        .await
        .unwrap();
    mail_actions::undo::undo_trash(&pool, &queue, &token)
        .await
        .expect("undo");
    for thread in ids(0..6) {
        assert_untouched(&pool, &thread).await;
    }
    let receipts: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM mail_action_receipts WHERE state = 'available'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        receipts, 0,
        "the consumed receipt is the only one there was"
    );
}

#[tokio::test]
async fn a_stopped_trash_that_only_deleted_permanently_leaves_no_empty_receipt() {
    let pool = pool().await;
    let threads = seed(&pool, 2).await;
    sqlx::query(
        "INSERT INTO folders (id, account_id, name, full_path, folder_type, remote_id, uidvalidity, created_at, updated_at)
         VALUES ('imap-a-trash', 'imap-a', 'Trash', 'Trash', 'trash', 'Trash', 12, '2026-01-01', '2026-01-01')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("UPDATE messages SET folder_id = 'imap-a-trash' WHERE thread_id = 't00000'")
        .execute(&pool)
        .await
        .unwrap();
    fail_writes_of(&pool, "t00001").await;
    let queue = SyncQueue::new();
    mail_actions::set_move_batch_threads_for_tests(Some(1));
    let receipt = mail_actions::trash_threads(&pool, &queue, &threads)
        .await
        .expect("the first batch committed");
    mail_actions::set_move_batch_threads_for_tests(None);

    assert_eq!(receipt.permanently_deleted, ids(0..1));
    assert!(receipt.undo_thread_ids.is_empty());
    assert!(receipt.undo_token.is_none());
    assert_eq!(receipt.stopped_early.expect("stopped").remaining, ids(1..2));
    let receipts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM mail_action_receipts")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(receipts, 0, "no Undo receipt without threads stays behind");
    assert_untouched(&pool, "t00001").await;
}

#[tokio::test]
async fn a_first_batch_failure_changes_nothing() {
    let pool = pool().await;
    let threads = seed(&pool, 4).await;
    fail_writes_of(&pool, "t00001").await;
    let queue = SyncQueue::new();
    mail_actions::set_move_batch_threads_for_tests(Some(2));
    let result = mail_actions::trash_threads(&pool, &queue, &threads).await;
    mail_actions::set_move_batch_threads_for_tests(None);
    assert!(result.is_err());
    for thread in ids(0..4) {
        assert_untouched(&pool, &thread).await;
    }
    for table in ["sync_queue", "mail_action_receipts", "mail_action_threads"] {
        let rows: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(rows, 0, "{table}");
    }
}

/// The failing batch noted a permanent delete before its writes failed; the
/// receipt must not report a deletion that rolled back.
#[tokio::test]
async fn a_failed_batch_reports_none_of_its_permanent_deletes() {
    let pool = pool().await;
    let threads = seed(&pool, 4).await;
    sqlx::query(
        "INSERT INTO folders (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at)
         VALUES ('gmail-trash', 'gmail', 'TRASH', 'TRASH', 'trash', 'TRASH', '2026-01-01', '2026-01-01')",
    )
    .execute(&pool)
    .await
    .unwrap();
    // Batches of two: t0+t1 commit; t2 (Gmail, all in Trash) is deleted for
    // good, then t3's writes fail and take t2's deletion back with them.
    sqlx::query("UPDATE messages SET folder_id = 'gmail-trash' WHERE thread_id = 't00002'")
        .execute(&pool)
        .await
        .unwrap();
    fail_writes_of(&pool, "t00003").await;
    let queue = SyncQueue::new();
    mail_actions::set_move_batch_threads_for_tests(Some(2));
    let receipt = mail_actions::trash_threads(&pool, &queue, &threads)
        .await
        .expect("the first batch committed");
    mail_actions::set_move_batch_threads_for_tests(None);

    assert!(
        receipt.permanently_deleted.is_empty(),
        "{:?}",
        receipt.permanently_deleted
    );
    assert_eq!(receipt.undo_thread_ids, ids(0..2));
    assert_eq!(receipt.stopped_early.expect("stopped").remaining, ids(2..4));
    let kept: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE thread_id = 't00002'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(kept, 3, "t2 kept its messages");
    let deletes: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sync_queue WHERE json_extract(payload, '$.kind') = 'delete'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(deletes, 0);
}
