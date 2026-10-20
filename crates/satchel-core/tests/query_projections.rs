//! Trigger projections remain correct during writes and interrupted backfills.
use satchel_core::db::{
    data_repairs::{run_repair, DataRepair, FTS_PREFIX, MESSAGE_ADDRESSES, THREAD_FOLDERS},
    mailboxes::{list_mailboxes, MAILBOX_SELECT},
    messages::{MessageSearchRepository, SearchQuery},
    threads::{ThreadListRequest, ThreadRepository},
};
use sqlx::{sqlite::SqlitePoolOptions, Row, SqlitePool};

async fn pool(legacy: bool) -> SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    if legacy {
        for migration in sqlx::migrate!("./src/db/migrations")
            .iter()
            .filter(|m| m.version < 95)
        {
            sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
        }
    } else {
        satchel_core::db::init(&pool).await.unwrap();
    }
    sqlx::raw_sql(
        "INSERT INTO accounts(id,display_name,email,provider,config,credentials_ref,created_at,updated_at)
         VALUES('a','Account','me@test.local','imap','{}','','2026-01-01','2026-01-01');
         INSERT INTO folders(id,account_id,name,full_path,folder_type,created_at,updated_at)
         VALUES('inbox','a','Inbox','INBOX','inbox','2026-01-01','2026-01-01'),
               ('archive','a','Archive','Archive','archive','2026-01-01','2026-01-01');
         INSERT INTO threads(id,account_id,last_message_date,workflow_state,created_at,updated_at)
         VALUES('t1','a','2026-01-01','inbox','2026-01-01','2026-01-01'),
               ('t2','a','2026-01-01','inbox','2026-01-01','2026-01-01');",
    ).execute(&pool).await.unwrap();
    for (id, thread, read) in [("m1", "t1", 0), ("m2", "t1", 0), ("m3", "t2", 1)] {
        sqlx::query("INSERT INTO messages(id,account_id,folder_id,thread_id,from_address,to_addresses,cc_addresses,bcc_addresses,subject,body_text,is_read,date,created_at,updated_at)
            VALUES(?,'a','inbox',?,' Sender@Test.Local ','[\"TO@test.local\",\"sender@test.local\"]','[\" CC@Test.Local \"]','[\"bcc@test.local\"]','meeting','market memo',?,'2026-01-01','2026-01-01','2026-01-01')")
            .bind(id).bind(thread).bind(read).execute(&pool).await.unwrap();
    }
    if legacy {
        for migration in sqlx::migrate!("./src/db/migrations")
            .iter()
            .filter(|m| m.version >= 95)
        {
            sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
        }
    }
    pool
}

async fn repair(pool: &SqlitePool, repair: &DataRepair) {
    run_repair(pool, repair, 1, None, |_| {}).await.unwrap();
}

async fn assert_counts(pool: &SqlitePool) {
    let expected: Vec<(String, i64, i64)> = sqlx::query(include_str!(
        "fixtures/query_benchmark/mailboxes_before.sql"
    ))
    .fetch_all(pool)
    .await
    .unwrap()
    .iter()
    .map(|r| (r.get("id"), r.get("thread_count"), r.get("unread_count")))
    .collect();
    let actual: Vec<_> = list_mailboxes(pool)
        .await
        .unwrap()
        .into_iter()
        .map(|r| (r.id, r.thread_count, r.unread_count))
        .collect();
    assert_eq!(actual, expected);
    let stored: Vec<(String, i64, i64)> =
        sqlx::query_as("SELECT id,thread_count,thread_unread_count FROM folders ORDER BY name")
            .fetch_all(pool)
            .await
            .unwrap();
    assert_eq!(stored, expected);
}

#[tokio::test]
async fn folder_counters_follow_reads_pending_moves_merges_workflow_and_deletes() {
    let pool = pool(false).await;
    repair(&pool, &THREAD_FOLDERS).await;
    assert_counts(&pool).await;
    for sql in [
        "UPDATE messages SET is_read=1 WHERE id='m1'",
        "UPDATE messages SET pending_folder_id='archive' WHERE id='m2'",
        "UPDATE threads SET workflow_state='done' WHERE id='t2'",
        "UPDATE messages SET folder_id='archive',pending_folder_id=NULL WHERE id='m2'",
        "UPDATE messages SET pending_folder_id='inbox' WHERE id='m2'",
        "UPDATE messages SET pending_folder_id=NULL WHERE id='m2'",
        "UPDATE messages SET is_read=0 WHERE id='m1'",
        "UPDATE threads SET workflow_state='snoozed' WHERE id='t1'",
        "UPDATE threads SET workflow_state='todo' WHERE id='t1'",
        "UPDATE messages SET thread_id='t2' WHERE id='m1'",
        "UPDATE threads SET workflow_state='inbox' WHERE id='t2'",
        "UPDATE folders SET folder_type='inbox' WHERE id='archive'",
        "UPDATE threads SET workflow_state='aside' WHERE id='t1'",
        "UPDATE folders SET folder_type='archive' WHERE id='archive'",
        "DELETE FROM messages WHERE id='m3'",
        "DELETE FROM messages WHERE id='m1'",
        "DELETE FROM messages WHERE id='m2'",
    ] {
        sqlx::query(sql).execute(&pool).await.unwrap();
        assert_counts(&pool).await;
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM thread_folders")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn counters_backfill_resumes_without_double_counting_concurrent_writes() {
    let pool = pool(true).await;
    // Existing rows must remain visible before the projection has been built.
    assert_eq!(
        list_mailboxes(&pool)
            .await
            .unwrap()
            .iter()
            .find(|r| r.id == "inbox")
            .unwrap()
            .thread_count,
        2
    );
    run_repair(&pool, &THREAD_FOLDERS, 1, Some(1), |_| {})
        .await
        .unwrap();
    sqlx::query("UPDATE messages SET pending_folder_id='archive',is_read=1 WHERE id='m3'")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM messages WHERE id='m1'")
        .execute(&pool)
        .await
        .unwrap();
    repair(&pool, &THREAD_FOLDERS).await;
    assert_counts(&pool).await;
}

#[tokio::test]
async fn addresses_backfill_and_triggers_cover_every_role_and_thread_reassignment() {
    let pool = pool(true).await;
    run_repair(&pool, &MESSAGE_ADDRESSES, 1, Some(1), |_| {})
        .await
        .unwrap();
    sqlx::query("UPDATE messages SET from_address='replacement@test.local',cc_addresses='[]',thread_id='t2' WHERE id='m2'")
        .execute(&pool).await.unwrap();
    sqlx::query("DELETE FROM messages WHERE id='m3'")
        .execute(&pool)
        .await
        .unwrap();
    repair(&pool, &MESSAGE_ADDRESSES).await;
    let addresses: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT message_id,thread_id,email FROM message_addresses ORDER BY message_id,email",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(addresses.len(), 8);
    assert!(addresses.contains(&("m1".into(), "t1".into(), "cc@test.local".into())));
    assert!(addresses.contains(&("m2".into(), "t2".into(), "replacement@test.local".into())));
    for address in [
        "sender@test.local",
        "to@test.local",
        "cc@test.local",
        "bcc@test.local",
    ] {
        let page = ThreadRepository::new(&pool)
            .list(ThreadListRequest {
                contact_email: Some(address.into()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(page.threads.iter().any(|t| t.id == "t1"));
    }
    sqlx::query("UPDATE messages SET to_addresses='[]',bcc_addresses='[]' WHERE id='m1'")
        .execute(&pool)
        .await
        .unwrap();
    let bcc = ThreadRepository::new(&pool)
        .list(ThreadListRequest {
            contact_email: Some(" BCC@Test.Local ".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(
        bcc.threads
            .iter()
            .map(|t| t.id.as_str())
            .collect::<Vec<_>>(),
        ["t2"]
    );
}

#[tokio::test]
async fn prefix_backfill_keeps_search_live_and_survives_update_delete_and_resume() {
    let pool = pool(true).await;
    run_repair(&pool, &FTS_PREFIX, 1, Some(1), |_| {})
        .await
        .unwrap();
    // Update one indexed and one not-yet-indexed row, and delete another old row.
    sqlx::query("UPDATE messages SET body_text='maple' WHERE id IN ('m1','m2')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM messages WHERE id='m3'")
        .execute(&pool)
        .await
        .unwrap();
    let search = MessageSearchRepository::new(&pool);
    assert_eq!(
        search
            .search(&SearchQuery {
                query: "map".into(),
                account_id: None,
                limit: None,
                cursor: None,
            })
            .await
            .unwrap()
            .hits
            .len(),
        1
    );
    repair(&pool, &FTS_PREFIX).await;
    let schema: String =
        sqlx::query_scalar("SELECT sql FROM sqlite_master WHERE name='messages_fts'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(schema.contains("prefix='2 3'"));
    sqlx::query("INSERT INTO messages_fts(messages_fts,rank) VALUES('integrity-check',1)")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        search
            .search(&SearchQuery {
                query: "ma".into(),
                account_id: None,
                limit: None,
                cursor: None,
            })
            .await
            .unwrap()
            .hits
            .len(),
        1
    );
    assert!(search
        .search(&SearchQuery {
            query: "m".into(),
            account_id: None,
            limit: None,
            cursor: None,
        })
        .await
        .unwrap()
        .hits
        .is_empty());
    sqlx::query("UPDATE messages SET body_text='garden' WHERE id='m1'")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM messages WHERE id='m2'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(search
        .search(&SearchQuery {
            query: "ma".into(),
            account_id: None,
            limit: None,
            cursor: None,
        })
        .await
        .unwrap()
        .hits
        .is_empty());
    sqlx::query("INSERT INTO messages_fts(messages_fts,rank) VALUES('integrity-check',1)")
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn sidebar_and_reconcile_plans_avoid_message_body_reads() {
    let pool = pool(false).await;
    let plans = sqlx::query(&format!("EXPLAIN QUERY PLAN {MAILBOX_SELECT}"))
        .fetch_all(&pool)
        .await
        .unwrap();
    let details = plans
        .iter()
        .map(|r| r.get::<String, _>("detail"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !details.contains("messages") && !details.contains("threads"),
        "{details}"
    );
    let plan=sqlx::query("EXPLAIN QUERY PLAN SELECT id,thread_id,remote_id,is_read,is_starred FROM messages WHERE account_id=? AND folder_id=?")
        .bind("a").bind("inbox").fetch_all(&pool).await.unwrap();
    assert!(
        plan.iter().any(|r| r
            .get::<String, _>("detail")
            .contains("COVERING INDEX idx_messages_imap_reconcile")),
        "{}",
        plan.iter()
            .map(|r| r.get::<String, _>("detail"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[tokio::test]
async fn projection_triggers_accept_outer_message_upserts_during_backfill() {
    let pool = pool(false).await;
    for _ in 0..2 {
        sqlx::query("INSERT INTO messages(id,account_id,folder_id,thread_id,from_address,to_addresses,cc_addresses,bcc_addresses,subject,body_text,date,created_at,updated_at)
            VALUES('m1','a','inbox','t1','sender@test.local','[\"sender@test.local\",\"to@test.local\"]','[\"cc@test.local\"]','[\"bcc@test.local\"]','updated','maple','2026-01-01','2026-01-01','2026-01-01')
            ON CONFLICT(id) DO UPDATE SET from_address=excluded.from_address,
            to_addresses=excluded.to_addresses,cc_addresses=excluded.cc_addresses,
            bcc_addresses=excluded.bcc_addresses,subject=excluded.subject,body_text=excluded.body_text")
            .execute(&pool).await.unwrap();
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM message_addresses WHERE message_id='m1'"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        4
    );
    repair(&pool, &THREAD_FOLDERS).await;
    assert_counts(&pool).await;
    repair(&pool, &FTS_PREFIX).await;
    sqlx::query("INSERT INTO messages_fts(messages_fts,rank) VALUES('integrity-check',1)")
        .execute(&pool)
        .await
        .unwrap();
}

/// `thread_folders` and the folder counters, held against a full recount.
async fn assert_thread_folders_match_recount(pool: &SqlitePool) {
    let recount: Vec<(String, String, i64, i64, i64)> = sqlx::query_as(
        "SELECT f.id, m.thread_id, COUNT(*), COALESCE(SUM(m.is_read = 0), 0),
                CASE WHEN LOWER(TRIM(f.folder_type)) = 'inbox'
                     THEN COALESCE(t.workflow_state NOT IN ('aside', 'done', 'snoozed'), 0) ELSE 1 END
         FROM messages m JOIN folders f ON f.id = COALESCE(m.pending_folder_id, m.folder_id)
         LEFT JOIN threads t ON t.id = m.thread_id
         WHERE m.thread_id IS NOT NULL
         GROUP BY f.id, m.thread_id ORDER BY 1, 2",
    )
    .fetch_all(pool)
    .await
    .unwrap();
    let stored: Vec<(String, String, i64, i64, i64)> = sqlx::query_as(
        "SELECT folder_id, thread_id, message_count, unread_count, visible
         FROM thread_folders ORDER BY 1, 2",
    )
    .fetch_all(pool)
    .await
    .unwrap();
    assert_eq!(stored, recount);
    let folders: Vec<(String, i64, i64, i64, i64)> = sqlx::query_as(
        "SELECT f.id, f.thread_count, f.thread_unread_count,
                COALESCE((SELECT SUM(visible) FROM thread_folders WHERE folder_id = f.id), 0),
                COALESCE((SELECT SUM(visible AND unread_count > 0) FROM thread_folders WHERE folder_id = f.id), 0)
         FROM folders f ORDER BY f.id",
    )
    .fetch_all(pool)
    .await
    .unwrap();
    for (id, threads, unread, expected_threads, expected_unread) in folders {
        assert_eq!(
            (threads, unread),
            (expected_threads, expected_unread),
            "{id}"
        );
    }
    assert_counts(pool).await;
}

#[tokio::test]
async fn incremental_folder_counts_match_a_full_recount() {
    let pool = pool(false).await;
    repair(&pool, &THREAD_FOLDERS).await;
    assert_thread_folders_match_recount(&pool).await;
    let insert = "INSERT INTO messages(id,account_id,folder_id,thread_id,from_address,to_addresses,subject,is_read,date,created_at,updated_at)
        VALUES(?,'a',?,?,'s@test.local','[]','s',?,'2026-01-01','2026-01-01','2026-01-01')";
    for (id, folder, thread, read) in [
        ("n1", "inbox", Some("t1"), Some(0)),
        ("n2", "archive", Some("t1"), Some(1)),
        ("n3", "inbox", Some("t2"), None),
        ("n4", "inbox", None, Some(0)),
    ] {
        sqlx::query(insert)
            .bind(id)
            .bind(folder)
            .bind(thread)
            .bind(read)
            .execute(&pool)
            .await
            .unwrap();
        assert_thread_folders_match_recount(&pool).await;
    }
    for sql in [
        // Rewriting a value with itself must not count it twice.
        "UPDATE messages SET is_read = is_read, folder_id = folder_id",
        "UPDATE messages SET is_read = 1 WHERE id = 'n3'",
        "UPDATE messages SET is_read = NULL WHERE id = 'm1'",
        "UPDATE messages SET is_read = 0, pending_folder_id = 'archive' WHERE id = 'm2'",
        "UPDATE messages SET thread_id = 't2', is_read = 0 WHERE id = 'n1'",
        "UPDATE messages SET thread_id = 't1' WHERE id = 'n4'",
        "UPDATE messages SET thread_id = NULL WHERE id = 'm3'",
        "UPDATE messages SET folder_id = 'archive', pending_folder_id = NULL WHERE id = 'm2'",
        "UPDATE threads SET workflow_state = 'done' WHERE id = 't2'",
        "INSERT INTO messages(id,account_id,folder_id,thread_id,from_address,to_addresses,subject,date,created_at,updated_at)
         VALUES('n6','a','inbox','t2','s@test.local','[]','s','2026-01-01','2026-01-01','2026-01-01')",
        "UPDATE threads SET workflow_state = 'inbox' WHERE id = 't2'",
        "DELETE FROM messages WHERE id IN ('n1', 'n2')",
        "DELETE FROM messages WHERE thread_id = 't1'",
    ] {
        sqlx::query(sql).execute(&pool).await.unwrap();
        assert_thread_folders_match_recount(&pool).await;
    }
}

#[tokio::test]
async fn marking_a_thread_read_leaves_already_read_messages_alone() {
    let pool = pool(false).await;
    repair(&pool, &THREAD_FOLDERS).await;
    sqlx::query("UPDATE messages SET is_read = 1, updated_at = 'untouched' WHERE id = 'm1'")
        .execute(&pool)
        .await
        .unwrap();
    ThreadRepository::new(&pool)
        .set_read_state(&["t1".to_string()], true)
        .await
        .unwrap();
    let rows: Vec<(String, i64, String)> = sqlx::query_as(
        "SELECT id, is_read, updated_at FROM messages WHERE thread_id = 't1' ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(rows[0], ("m1".into(), 1, "untouched".into()));
    assert_eq!(rows[1].1, 1);
    assert_ne!(rows[1].2, "2026-01-01");
    assert_thread_folders_match_recount(&pool).await;
    ThreadRepository::new(&pool)
        .set_read_state(&["t1".to_string(), "t2".to_string()], false)
        .await
        .unwrap();
    assert_thread_folders_match_recount(&pool).await;
}
