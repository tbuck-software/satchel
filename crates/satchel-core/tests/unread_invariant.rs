//! `threads.unread_count` has to agree with the messages it counts.
//!
//! The counter is a projection: written by `refresh_thread_projection_tx`,
//! never by a trigger. A write path that forgets the refresh leaves an Inbox
//! badge that is wrong until something else happens to refresh the thread —
//! which is why a startup repair exists at all. These scenarios run the
//! operations that change read state or membership, and check the counter
//! against the rows after each one.

use satchel_core::db::threads::ThreadRepository;
use satchel_core::mail_actions::{move_threads, set_flags, MoveTarget};
use satchel_core::sync::queue::SyncQueue;
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

async fn seeded_pool() -> SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("pool");
    satchel_core::db::init(&pool).await.expect("migrations");
    for statement in [
        "INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, sync_state, color, is_active, created_at, updated_at)
         VALUES ('acc', 'A', 'me@example.test', 'imap', '{}', '', '{}', '#000', 1, '2026-01-01', '2026-01-01')",
        "INSERT INTO folders (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at)
         VALUES ('inbox', 'acc', 'INBOX', 'INBOX', 'inbox', 'INBOX', '2026-01-01', '2026-01-01')",
        "INSERT INTO folders (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at)
         VALUES ('archive', 'acc', 'Archive', 'Archive', 'archive', 'Archive', '2026-01-01', '2026-01-01')",
        "INSERT INTO folders (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at)
         VALUES ('trash', 'acc', 'Trash', 'Trash', 'trash', 'Trash', '2026-01-01', '2026-01-01')",
        "INSERT INTO threads (id, account_id, subject, last_message_date, message_count, unread_count, workflow_state, created_at, updated_at)
         VALUES ('t', 'acc', 'Thread', '2026-07-03', 3, 3, 'inbox', '2026-01-01', '2026-01-01')",
    ] {
        sqlx::query(statement).execute(&pool).await.expect("seed");
    }
    for (id, date) in [
        ("m1", "2026-07-01"),
        ("m2", "2026-07-02"),
        ("m3", "2026-07-03"),
    ] {
        insert_message(&pool, id, date, false).await;
    }
    pool
}

async fn insert_message(pool: &SqlitePool, id: &str, date: &str, read: bool) {
    sqlx::query(
        "INSERT INTO messages (id, account_id, folder_id, thread_id, message_id_header, from_address, to_addresses, subject, date, is_read, remote_id, created_at, updated_at)
         VALUES (?, 'acc', 'inbox', 't', ?, 'sender@example.test', '[]', 'Thread', ?, ?, ?, '2026-01-01', '2026-01-01')",
    )
    .bind(id)
    .bind(format!("<{id}@example.test>"))
    .bind(date)
    .bind(i64::from(read))
    .bind(id.trim_start_matches('m').to_string())
    .execute(pool)
    .await
    .expect("message");
}

/// Where the stored counter and the rows disagree: `(thread, stored, counted)`.
///
/// Counts distinct Message-IDs, the same identity the projection collapses
/// provider copies by; a message has one here, so id and header agree.
async fn unread_drift(pool: &SqlitePool) -> Vec<(String, i64, i64)> {
    sqlx::query_as(
        "SELECT threads.id, threads.unread_count,
                (SELECT COUNT(DISTINCT COALESCE(NULLIF(TRIM(message_id_header), ''), id))
                 FROM messages WHERE messages.thread_id = threads.id AND is_read = 0)
         FROM threads
         WHERE threads.unread_count <>
               (SELECT COUNT(DISTINCT COALESCE(NULLIF(TRIM(message_id_header), ''), id))
                FROM messages WHERE messages.thread_id = threads.id AND is_read = 0)",
    )
    .fetch_all(pool)
    .await
    .expect("drift")
}

async fn assert_invariant(pool: &SqlitePool, after: &str) {
    let drift = unread_drift(pool).await;
    assert!(
        drift.is_empty(),
        "unread_count drifted after {after}: {drift:?}"
    );
}

#[tokio::test]
async fn reading_a_thread_keeps_the_counter_true() {
    let pool = seeded_pool().await;
    assert_invariant(&pool, "seeding").await;

    ThreadRepository::new(&pool)
        .set_read_state(&["t".to_string()], true)
        .await
        .expect("read");
    assert_invariant(&pool, "set_read_state(true)").await;

    set_flags(
        &pool,
        &SyncQueue::new(),
        &["t".to_string()],
        Some(false),
        None,
        None,
    )
    .await
    .expect("unread");
    assert_invariant(&pool, "set_flags(read: false)").await;
}

#[tokio::test]
async fn archiving_a_thread_keeps_the_counter_true() {
    let pool = seeded_pool().await;

    move_threads(
        &pool,
        &SyncQueue::new(),
        &["t".to_string()],
        MoveTarget::Archive,
    )
    .await
    .expect("archive");

    assert_invariant(&pool, "archive").await;
}

#[tokio::test]
async fn a_synced_message_keeps_the_counter_true() {
    let pool = seeded_pool().await;
    ThreadRepository::new(&pool)
        .set_read_state(&["t".to_string()], true)
        .await
        .expect("read");

    // What ingest does: store the row, then refresh the thread projection.
    insert_message(&pool, "m4", "2026-07-04", false).await;
    ThreadRepository::new(&pool)
        .refresh_projection(&["t".to_string()])
        .await
        .expect("refresh");

    assert_invariant(&pool, "ingest").await;
    let stored: i64 = sqlx::query_scalar("SELECT unread_count FROM threads WHERE id = 't'")
        .fetch_one(&pool)
        .await
        .expect("counter");
    assert_eq!(stored, 1);
}

#[tokio::test]
async fn deleting_a_message_keeps_the_counter_true() {
    let pool = seeded_pool().await;
    move_threads(
        &pool,
        &SyncQueue::new(),
        &["t".to_string()],
        MoveTarget::Trash,
    )
    .await
    .expect("to trash");
    assert_invariant(&pool, "trash").await;

    sqlx::query("DELETE FROM messages WHERE id = 'm3'")
        .execute(&pool)
        .await
        .expect("delete");
    ThreadRepository::new(&pool)
        .refresh_projection(&["t".to_string()])
        .await
        .expect("refresh");

    assert_invariant(&pool, "delete").await;
}

/// The check has to be able to fail, or it proves nothing.
#[tokio::test]
async fn a_tampered_counter_is_reported() {
    let pool = seeded_pool().await;
    sqlx::query("UPDATE threads SET unread_count = 7 WHERE id = 't'")
        .execute(&pool)
        .await
        .expect("tamper");

    let drift = unread_drift(&pool).await;

    assert_eq!(drift, vec![("t".to_string(), 7, 3)]);
}
