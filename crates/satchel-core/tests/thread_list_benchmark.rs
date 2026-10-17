//! Wall-clock benchmark for the thread list query, 2000 threads / 20 000 mails.
//!
//! Ignored by default: timings swing with machine load, so it measures, it
//! does not gate. Run with
//! `cargo test -p satchel-core --release --test thread_list_benchmark -- --ignored --nocapture`.

use satchel_core::db::threads::{ThreadListRequest, ThreadRepository};
use sqlx::sqlite::SqlitePoolOptions;
use std::time::{Duration, Instant};

const THREADS: usize = 2_000;
const MESSAGES_PER_THREAD: usize = 10;

async fn seeded_pool() -> sqlx::SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("pool");
    satchel_core::db::init(&pool).await.expect("migrations");
    sqlx::query(
        "INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, sync_state, color, is_active, created_at, updated_at)
         VALUES ('acc', 'A', 'me@example.test', 'imap', '{}', '', '{}', '#000', 1, '2026-01-01', '2026-01-01')",
    )
    .execute(&pool)
    .await
    .expect("account");
    sqlx::query(
        "INSERT INTO folders (id, account_id, name, full_path, folder_type, created_at, updated_at)
         VALUES ('f', 'acc', 'INBOX', 'INBOX', 'inbox', '2026-01-01', '2026-01-01')",
    )
    .execute(&pool)
    .await
    .expect("folder");

    let mut tx = pool.begin().await.expect("tx");
    for thread in 0..THREADS {
        let thread_id = format!("t{thread:05}");
        let last = format!(
            "2026-07-{:02}T{:02}:{:02}:00Z",
            1 + thread % 28,
            thread % 24,
            thread % 60
        );
        sqlx::query(
            "INSERT INTO threads (id, account_id, subject, last_message_date, message_count, unread_count, workflow_state, created_at, updated_at)
             VALUES (?, 'acc', 'Thread', ?, ?, 0, 'inbox', '2026-01-01', '2026-01-01')",
        )
        .bind(&thread_id)
        .bind(&last)
        .bind(MESSAGES_PER_THREAD as i64)
        .execute(&mut *tx)
        .await
        .expect("thread");
        for message in 0..MESSAGES_PER_THREAD {
            sqlx::query(
                "INSERT INTO messages (id, account_id, folder_id, thread_id, from_address, from_name, to_addresses, cc_addresses, subject, snippet, date, is_draft, created_at, updated_at)
                 VALUES (?, 'acc', 'f', ?, ?, 'Sender', '[\"me@example.test\",\"x@example.test\"]', '[]', ?, 'Snippet', ?, ?, '2026-01-01', '2026-01-01')",
            )
            .bind(format!("{thread_id}-m{message:02}"))
            .bind(&thread_id)
            .bind(format!("sender{}@example.test", message % 3))
            .bind(format!("Subject {message}"))
            .bind(format!("2026-06-{:02}T10:{:02}:00Z", 1 + message, thread % 60))
            .bind(i64::from(message == MESSAGES_PER_THREAD - 1 && thread % 7 == 0))
            .execute(&mut *tx)
            .await
            .expect("message");
        }
    }
    tx.commit().await.expect("commit");
    pool
}

#[tokio::test]
#[ignore = "benchmark; run explicitly"]
async fn list_threads_200_rows_of_2000() {
    let pool = seeded_pool().await;
    let repository = ThreadRepository::new(&pool);
    // Warm the statement cache and the page cache once.
    repository
        .list(ThreadListRequest {
            limit: Some(200),
            ..Default::default()
        })
        .await
        .expect("warm-up");

    let mut samples: Vec<Duration> = Vec::new();
    for _ in 0..15 {
        let started = Instant::now();
        let page = repository
            .list(ThreadListRequest {
                limit: Some(200),
                ..Default::default()
            })
            .await
            .expect("page");
        samples.push(started.elapsed());
        assert_eq!(page.threads.len(), 200);
    }
    samples.sort();
    println!(
        "list_threads limit 200 of {THREADS} threads / {} messages: median {:?}, min {:?}",
        THREADS * MESSAGES_PER_THREAD,
        samples[samples.len() / 2],
        samples[0]
    );
}
