use satchel_core::db::{
    self,
    body_window::{self, BODY_WINDOW},
    data_repairs::{self, RepairRun, LEGACY_MIME},
    maintenance,
    threads::ThreadRepository,
    trash,
};
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
use std::time::Duration;

async fn fixture() -> SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    db::init(&pool).await.unwrap();
    sqlx::query("INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, sync_state, created_at, updated_at) VALUES ('a', 'A', 'a@b.test', 'imap', '{}', '', '{}', '', '')").execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO folders (id, account_id, name, full_path, folder_type, created_at, updated_at) VALUES ('f', 'a', 'Trash', 'Trash', 'trash', '', '')").execute(&pool).await.unwrap();
    pool
}

async fn message(pool: &SqlitePool, id: &str, body: &str) {
    sqlx::query("INSERT INTO messages (id, account_id, folder_id, from_address, to_addresses, subject, body_text, date, remote_id, created_at, updated_at) VALUES (?, 'a', 'f', 'a@b.test', '[]', 'Subject', ?, '2020-01-01', ?, '', '')")
        .bind(id).bind(body).bind(id).execute(pool).await.unwrap();
}

async fn marker(pool: &SqlitePool, key: &str) -> Option<String> {
    sqlx::query_scalar("SELECT value FROM app_settings WHERE key = ?")
        .bind(key)
        .fetch_optional(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn changed_body_window_is_deferred_resumable_and_not_reset_by_other_saves() {
    let pool = fixture().await;
    for id in ["a", "b", "c", "d", "e"] {
        message(&pool, id, "Body").await;
    }
    assert!(body_window::set_days(&pool, 90, 0).await.unwrap());
    let bodies: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE body_text IS NOT NULL")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        bodies, 5,
        "settings must not evict mail on the command path"
    );
    assert_eq!(
        data_repairs::run_repair(&pool, &BODY_WINDOW, 2, Some(1), |_| {})
            .await
            .unwrap(),
        RepairRun::Paused { blocks: 1 }
    );
    let saved = marker(&pool, &BODY_WINDOW.marker_key()).await;
    assert!(saved.as_ref().unwrap().contains("\"after\":\"b\""));
    assert!(!body_window::set_days(&pool, 90, 0).await.unwrap());
    assert_eq!(marker(&pool, &BODY_WINDOW.marker_key()).await, saved);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM messages WHERE body_text IS NULL")
            .fetch_one(&pool)
            .await
            .unwrap(),
        2
    );
    data_repairs::run_repair(&pool, &BODY_WINDOW, 2, None, |_| {})
        .await
        .unwrap();
    assert!(marker(&pool, &BODY_WINDOW.marker_key()).await.is_none());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM messages WHERE body_text IS NULL")
            .fetch_one(&pool)
            .await
            .unwrap(),
        5
    );
    assert!(!body_window::set_days(&pool, 90, 0).await.unwrap());
    assert!(
        marker(&pool, &BODY_WINDOW.marker_key()).await.is_none(),
        "same window must not rescan a completed job"
    );
}

#[tokio::test]
async fn body_window_evicts_the_unsanitized_original_but_keeps_imports() {
    let pool = fixture().await;
    message(&pool, "remote", "Body").await;
    message(&pool, "only-original", "").await;
    message(&pool, "imported", "Body").await;
    sqlx::query("UPDATE messages SET body_text = NULL WHERE id = 'only-original'")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE messages SET body_html = '<p>Body</p>', original_body_html = '<p onclick=x()>Body</p>', original_source = CAST('raw' AS BLOB)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE messages SET body_html = NULL WHERE id = 'only-original'")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE messages SET remote_id = 'import-1' WHERE id = 'imported'")
        .execute(&pool)
        .await
        .unwrap();
    body_window::set_days(&pool, 90, 0).await.unwrap();
    data_repairs::run_repair(&pool, &BODY_WINDOW, 10, None, |_| {})
        .await
        .unwrap();

    let stored = |id: &'static str| {
        let pool = pool.clone();
        async move {
            sqlx::query_as::<_, (Option<String>, Option<String>, Option<String>, Option<Vec<u8>>)>(
                "SELECT body_text, body_html, original_body_html, original_source FROM messages WHERE id = ?",
            )
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };
    assert_eq!(stored("remote").await, (None, None, None, None));
    assert_eq!(stored("only-original").await, (None, None, None, None));
    let imported = stored("imported").await;
    assert!(imported.0.is_some() && imported.1.is_some());
    assert!(imported.2.is_some() && imported.3.is_some());
    let source = db::messages::MessageReadRepository::new(&pool)
        .get_message_source("remote")
        .await
        .unwrap();
    assert!(
        source.body_html.is_none(),
        "show source must not keep serving it"
    );
}

#[tokio::test]
async fn body_window_keeps_download_only_pop3_originals() {
    let pool = fixture().await;
    sqlx::query("UPDATE accounts SET provider = 'pop3'")
        .execute(&pool)
        .await
        .unwrap();
    message(&pool, "downloaded", "Only local copy").await;
    sqlx::query("UPDATE messages SET original_source = ?")
        .bind(b"Subject: Original\r\n\r\n\xff".as_slice())
        .execute(&pool)
        .await
        .unwrap();
    body_window::set_days(&pool, 90, 0).await.unwrap();
    data_repairs::run_repair(&pool, &BODY_WINDOW, 10, None, |_| {})
        .await
        .unwrap();
    let stored: Vec<u8> =
        sqlx::query_scalar("SELECT original_source FROM messages WHERE id = 'downloaded'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(stored, b"Subject: Original\r\n\r\n\xff");
}

#[tokio::test]
async fn disabling_or_changing_the_body_window_cancels_or_restarts_its_cursor() {
    let pool = fixture().await;
    message(&pool, "a", "Body").await;
    body_window::set_days(&pool, 90, 0).await.unwrap();
    data_repairs::run_repair(&pool, &BODY_WINDOW, 1, Some(1), |_| {})
        .await
        .unwrap();
    assert!(body_window::set_days(&pool, 365, 0).await.unwrap());
    assert!(marker(&pool, &BODY_WINDOW.marker_key())
        .await
        .unwrap()
        .contains("\"after\":\"\""));
    assert!(body_window::set_days(&pool, 0, 0).await.unwrap());
    assert!(marker(&pool, &BODY_WINDOW.marker_key()).await.is_none());
}

#[tokio::test]
async fn mime_repair_advances_past_unrepairable_rows_and_runs_once() {
    let pool = fixture().await;
    message(
        &pool,
        "a",
        "--bad\r\nContent-Type: application/octet-stream\r\n\r\nnot text\r\n--bad--",
    )
    .await;
    message(
        &pool,
        "b",
        "--good\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nHello repaired\r\n--good--",
    )
    .await;
    assert_eq!(
        data_repairs::run_repair(&pool, &LEGACY_MIME, 1, Some(1), |_| {})
            .await
            .unwrap(),
        RepairRun::Paused { blocks: 1 }
    );
    assert_eq!(
        marker(&pool, &LEGACY_MIME.marker_key()).await.as_deref(),
        Some("a")
    );
    data_repairs::run_repair(&pool, &LEGACY_MIME, 1, None, |_| {})
        .await
        .unwrap();
    let body: String = sqlx::query_scalar("SELECT body_text FROM messages WHERE id = 'b'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(body, "Hello repaired");
    assert!(marker(&pool, &LEGACY_MIME.marker_key()).await.is_none());
    // A later launch must not even try to open the messages table for this repair.
    sqlx::query("DROP TABLE messages")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        data_repairs::run_repair(&pool, &LEGACY_MIME, 1, None, |_| {})
            .await
            .unwrap(),
        RepairRun::NotPending
    );
}

#[tokio::test]
async fn busy_checkpoints_retry_without_claiming_an_hourly_success() {
    let dir = tempfile::tempdir().unwrap();
    let pools = db::connect_pools(dir.path().join("satchel.db"))
        .await
        .unwrap();
    sqlx::query("PRAGMA busy_timeout = 1")
        .execute(&pools.checkpoint)
        .await
        .unwrap();
    maintenance::checkpoint(&pools.checkpoint).await.unwrap();
    let mut snapshot = pools.read.as_ref().begin().await.unwrap();
    let _: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM accounts")
        .fetch_one(&mut *snapshot)
        .await
        .unwrap();
    maintenance::request_fts_check(&pools.write).await.unwrap();
    let result = maintenance::checkpoint(&pools.checkpoint).await;
    let row = result.as_ref().unwrap();
    assert_eq!(row.busy, 1);
    assert!(row.log > row.checkpointed);
    assert_eq!(
        maintenance::checkpoint_delay(&result),
        Duration::from_secs(60)
    );
    snapshot.rollback().await.unwrap();
    let result = maintenance::checkpoint(&pools.checkpoint).await;
    assert!(result.as_ref().unwrap().complete());
    assert_eq!(result.unwrap().log, 0);
    assert_eq!(
        maintenance::checkpoint_delay(&Ok(maintenance::Checkpoint {
            busy: 0,
            log: 0,
            checkpointed: 0
        })),
        Duration::from_secs(3600)
    );
    assert_eq!(
        maintenance::checkpoint_delay(&Err(sqlx::Error::PoolClosed)),
        Duration::from_secs(60)
    );
}

#[tokio::test]
async fn vacuum_only_when_more_than_one_fifth_is_free_and_checkpoint_afterwards() {
    let dir = tempfile::tempdir().unwrap();
    let pools = db::connect_pools(dir.path().join("satchel.db"))
        .await
        .unwrap();
    assert!(!maintenance::needs_vacuum(&pools.write).await.unwrap());
    sqlx::query("CREATE TABLE ballast (id INTEGER PRIMARY KEY, payload BLOB)")
        .execute(&pools.write)
        .await
        .unwrap();
    sqlx::query("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<500) INSERT INTO ballast SELECT x, zeroblob(8192) FROM n").execute(&pools.write).await.unwrap();
    sqlx::query("DELETE FROM ballast WHERE id > 100")
        .execute(&pools.write)
        .await
        .unwrap();
    assert!(maintenance::needs_vacuum(&pools.write).await.unwrap());
    sqlx::query("VACUUM").execute(&pools.write).await.unwrap();
    assert!(!maintenance::needs_vacuum(&pools.write).await.unwrap());
    let result = maintenance::checkpoint(&pools.checkpoint).await.unwrap();
    assert!(result.complete());
    assert_eq!((result.log, result.checkpointed), (0, 0));
}

#[tokio::test]
async fn a_large_trash_thread_is_deleted_in_bounded_batches_without_losing_its_age() {
    let pool = fixture().await;
    sqlx::query("INSERT INTO threads (id, account_id, subject, last_message_date, workflow_state, message_count, created_at, updated_at) VALUES ('trash', 'a', 'Old', '2020-01-01', 'done', 1200, '2020-01-01', '2020-01-01')").execute(&pool).await.unwrap();
    sqlx::query("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<1200) INSERT INTO messages (id, account_id, folder_id, thread_id, from_address, to_addresses, subject, date, created_at, updated_at) SELECT printf('m%06d', x), 'a', 'f', 'trash', 'a@b.test', '[]', 'Old', '2020-01-01', '', '' FROM n").execute(&pool).await.unwrap();
    let first = trash::purge_batch(&pool, "2021-01-01", "").await.unwrap();
    assert_eq!((first.removed_messages, first.removed_threads), (500, 0));
    let age: String = sqlx::query_scalar("SELECT updated_at FROM threads WHERE id = 'trash'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(age, "2020-01-01");
    let second = trash::purge_batch(&pool, "2021-01-01", first.cursor.as_deref().unwrap())
        .await
        .unwrap();
    assert_eq!((second.removed_messages, second.removed_threads), (500, 0));
    assert_eq!(
        ThreadRepository::new(&pool)
            .purge_trash_before("2021-01-01")
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM messages")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn vacuum_preserves_gapped_message_rowids_and_search_projection() {
    let pool = fixture().await;
    message(&pool, "a", "Firstword").await;
    message(&pool, "b", "Secondword").await;
    sqlx::query("DELETE FROM messages WHERE id = 'a'")
        .execute(&pool)
        .await
        .unwrap();
    message(&pool, "c", "Thirdword").await;
    let before: Vec<(i64, String)> =
        sqlx::query_as("SELECT rowid, id FROM messages ORDER BY rowid")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(before[0].0, 2);
    sqlx::query("VACUUM").execute(&pool).await.unwrap();
    let after: Vec<(i64, String)> = sqlx::query_as("SELECT rowid, id FROM messages ORDER BY rowid")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(after, before);
    let mismatches: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM message_search_projection p JOIN messages m ON p.message_id = m.id WHERE p.message_rowid <> m.rowid").fetch_one(&pool).await.unwrap();
    assert_eq!(mismatches, 0);
    assert!(db::message_index_intact(&pool).await.unwrap());
}
