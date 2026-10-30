//! Exercise provider HTTP responses, ingest savepoints and durable checkpoints.
use super::*;
use crate::providers::traits::MailProvider;
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn pool() -> SqlitePool {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::migrate!("../crates/satchel-core/src/db/migrations")
        .run(&pool)
        .await
        .unwrap();
    sqlx::raw_sql(
        "INSERT INTO accounts (id,display_name,email,provider,config,credentials_ref,created_at,updated_at)
         VALUES ('account','Fixture','me@example.test','gmail','{}','','','');
         INSERT INTO folders (id,account_id,name,full_path,folder_type,remote_id,created_at,updated_at)
         VALUES ('inbox','account','Inbox','INBOX','inbox','INBOX','','');",
    ).execute(&pool).await.unwrap();
    pool
}

fn message(labels: &[&str]) -> MessageEnvelope {
    serde_json::from_value(json!({
        "remote_id": "A", "remote_thread_id": "conversation",
        "message_id_header": "<a@example.test>",
        "subject": "Keep my organization", "snippet": "", "from_address": "sender@example.test",
        "to_addresses": [], "cc_addresses": [], "date_rfc3339": "2026-01-01T00:00:00Z",
        "mailbox_ids": labels, "body_text": "Fixture", "is_read": true
    }))
    .unwrap()
}

async fn ingest(pool: &SqlitePool, message: MessageEnvelope) -> FolderIngestOutcome {
    let account = MailAccountRow {
        id: "account".into(),
        display_name: "Fixture".into(),
        email: "me@example.test".into(),
        provider: "gmail".into(),
        config: "{}".into(),
        sync_state: None,
        outlook_immutable_ids_migrated: 1,
        ingest_cutoff: None,
    };
    ingest_folder_batch(
        pool,
        &FolderIngestContext {
            app: None,
            events: None,
            account: &account,
            config: &crate::commands::accounts::AccountConfig::from_plaintext("{}"),
            folder_id: "inbox",
            folder_remote_id: "INBOX",
            folder_type: "inbox",
        },
        vec![message],
        &mut HashSet::new(),
    )
    .await
    .unwrap()
}

async fn annotate(pool: &SqlitePool) -> String {
    let thread: String = sqlx::query_scalar("SELECT thread_id FROM messages LIMIT 1")
        .fetch_one(pool)
        .await
        .unwrap();
    sqlx::raw_sql(
        "INSERT INTO tags (id,name) VALUES ('tag','Keep');
         INSERT INTO topics (id,name,created_at,updated_at) VALUES ('topic','Keep','','');",
    )
    .execute(pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO thread_tags VALUES (?, 'tag')")
        .bind(&thread)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO topic_items (topic_id,item_type,item_id,added_at) VALUES ('topic','thread',?,'')")
        .bind(&thread).execute(pool).await.unwrap();
    sqlx::query(
        "INSERT INTO workflow_events (id,thread_id,from_state,to_state,reason,created_at)
                 VALUES ('history',?,'inbox','todo','user','')",
    )
    .bind(&thread)
    .execute(pool)
    .await
    .unwrap();
    thread
}

async fn assert_organization(pool: &SqlitePool, thread: &str, labels: &[&str]) {
    let copies: Vec<(String, String)> = sqlx::query_as(
        "SELECT m.thread_id,f.remote_id FROM messages m JOIN folders f ON f.id=m.folder_id ORDER BY f.remote_id",
    ).fetch_all(pool).await.unwrap();
    assert_eq!(
        copies,
        labels
            .iter()
            .map(|label| (thread.to_string(), label.to_string()))
            .collect::<Vec<_>>()
    );
    for query in [
        "SELECT COUNT(*) FROM thread_tags WHERE thread_id=?",
        "SELECT COUNT(*) FROM topic_items WHERE item_type='thread' AND item_id=?",
        "SELECT COUNT(*) FROM workflow_events WHERE thread_id=? AND id='history'",
    ] {
        assert_eq!(
            sqlx::query_scalar::<_, i64>(query)
                .bind(thread)
                .fetch_one(pool)
                .await
                .unwrap(),
            1
        );
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM threads")
            .fetch_one(pool)
            .await
            .unwrap(),
        1
    );
    assert!(sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(pool)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM attachments a LEFT JOIN messages m ON m.id=a.message_id WHERE m.id IS NULL",
    ).fetch_one(pool).await.unwrap(), 0);
}

#[tokio::test]
async fn gmail_last_copy_moves_keep_thread_organization_and_true_deletion_still_cleans_up() {
    for (source, target) in [
        ("INBOX", "TRASH"),
        ("Label_old", "Label_new"),
        ("Label_old", "ARCHIVE"),
    ] {
        let pool = pool().await;
        assert_eq!(ingest(&pool, message(&[source])).await.ingested, 1);
        let thread = annotate(&pool).await;
        let local_message: String = sqlx::query_scalar("SELECT id FROM messages")
            .fetch_one(&pool)
            .await
            .unwrap();

        let moved = message(if target == "ARCHIVE" {
            &[]
        } else {
            std::slice::from_ref(&target)
        });
        assert_eq!(ingest(&pool, moved.clone()).await.failed, 0);
        assert_organization(&pool, &thread, &[target]).await;
        assert_eq!(
            sqlx::query_scalar::<_, String>("SELECT id FROM messages")
                .fetch_one(&pool)
                .await
                .unwrap(),
            local_message
        );
        // Replaying after a crash is idempotent.
        ingest(&pool, moved).await;
        assert_organization(&pool, &thread, &[target]).await;
        reconcile_remote_message_removals(
            &pool,
            "account",
            &[crate::providers::traits::RemoteMessageRemoval {
                remote_id: "A".into(),
                source_folder: None,
            }],
        )
        .await
        .unwrap();
        for table in [
            "messages",
            "threads",
            "thread_tags",
            "topic_items",
            "workflow_events",
        ] {
            assert_eq!(
                sqlx::query_scalar::<_, i64>(&format!("SELECT COUNT(*) FROM {table}"))
                    .fetch_one(&pool)
                    .await
                    .unwrap(),
                0,
                "{table}"
            );
        }
    }
}

#[tokio::test]
async fn gmail_existing_target_keeps_message_links_and_their_metadata() {
    for fail_after_transfer in [false, true] {
        let pool = pool().await;
        ingest(&pool, message(&["INBOX", "Label_one"])).await;
        let thread = annotate(&pool).await;
        let source: String =
            sqlx::query_scalar("SELECT id FROM messages WHERE folder_id = 'inbox'")
                .fetch_one(&pool)
                .await
                .unwrap();
        let target: String =
            sqlx::query_scalar("SELECT id FROM messages WHERE folder_id != 'inbox'")
                .fetch_one(&pool)
                .await
                .unwrap();
        sqlx::raw_sql(
            "INSERT INTO topics (id,name,created_at,updated_at) VALUES ('shared','Shared','','');",
        )
        .execute(&pool)
        .await
        .unwrap();
        for (topic, id, added_at, added_by) in [
            ("topic", &source, "source-date", "user"),
            ("shared", &source, "source-date", "rule"),
            ("shared", &target, "target-date", "user"),
        ] {
            sqlx::query("INSERT INTO topic_items (topic_id,item_type,item_id,added_at,added_by) VALUES (?,'message',?,?,?)")
                .bind(topic).bind(id).bind(added_at).bind(added_by).execute(&pool).await.unwrap();
        }
        sqlx::query("INSERT INTO calendar_invites (id,message_id,account_id,uid,raw_ics,created_at,updated_at) VALUES ('invite',?,'account','invite-uid','fixture','','')")
            .bind(&source).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO follow_up_reminders (id,account_id,thread_id,message_id,due_at,created_at,updated_at) VALUES ('reminder','account',?,?,'2030-01-01','','')")
            .bind(&thread).bind(&source).execute(&pool).await.unwrap();
        if fail_after_transfer {
            sqlx::raw_sql(
                "CREATE TRIGGER fail_link_transfer BEFORE UPDATE ON threads
                WHEN NOT EXISTS (SELECT 1 FROM messages WHERE folder_id = 'inbox')
                BEGIN SELECT RAISE(ABORT, 'fixture projection failure'); END;",
            )
            .execute(&pool)
            .await
            .unwrap();
            let failed = ingest(&pool, message(&["Label_one"])).await;
            assert_eq!(failed.failed, 1);
            assert!(!failed.cursor_may_advance());
            assert_organization(&pool, &thread, &["INBOX", "Label_one"]).await;
            let links: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM topic_items WHERE item_type='message' AND item_id=?",
            )
            .bind(&source)
            .fetch_one(&pool)
            .await
            .unwrap();
            assert_eq!(links, 2);
            for table in ["calendar_invites", "follow_up_reminders"] {
                let id: String = sqlx::query_scalar(&format!("SELECT message_id FROM {table}"))
                    .fetch_one(&pool)
                    .await
                    .unwrap();
                assert_eq!(id, source);
            }
            sqlx::raw_sql("DROP TRIGGER fail_link_transfer")
                .execute(&pool)
                .await
                .unwrap();
        }
        for _ in 0..2 {
            assert_eq!(ingest(&pool, message(&["Label_one"])).await.failed, 0);
            assert_organization(&pool, &thread, &["Label_one"]).await;
            let links: Vec<(String, String, String, String)> = sqlx::query_as(
                "SELECT ti.topic_id,m.id,ti.added_at,ti.added_by FROM topic_items ti
                 JOIN messages m ON m.id=ti.item_id WHERE ti.item_type='message' ORDER BY ti.topic_id",
            ).fetch_all(&pool).await.unwrap();
            assert_eq!(
                links,
                vec![
                    (
                        "shared".into(),
                        target.clone(),
                        "target-date".into(),
                        "user".into()
                    ),
                    (
                        "topic".into(),
                        target.clone(),
                        "source-date".into(),
                        "user".into()
                    ),
                ]
            );
            assert_eq!(
                sqlx::query_scalar::<_, i64>(
                    "SELECT COUNT(*) FROM topic_items WHERE item_type='message'"
                )
                .fetch_one(&pool)
                .await
                .unwrap(),
                2
            );
            for table in ["calendar_invites", "follow_up_reminders"] {
                let id: String = sqlx::query_scalar(&format!("SELECT message_id FROM {table}"))
                    .fetch_one(&pool)
                    .await
                    .unwrap();
                assert_eq!(id, target);
            }
        }
    }
}

#[tokio::test]
async fn gmail_reused_id_does_not_replace_another_current_label_copy() {
    for (source, destination) in [("Label_two", "Label_one"), ("Label_one", "Label_two")] {
        let pool = pool().await;
        ingest(&pool, message(&[source])).await;
        let thread = annotate(&pool).await;
        let original: String = sqlx::query_scalar("SELECT id FROM messages")
            .fetch_one(&pool)
            .await
            .unwrap();
        ingest(&pool, message(&[destination])).await;
        let mut labels = vec![source, destination];
        labels.sort();
        for _ in 0..2 {
            let result = ingest(&pool, message(&labels)).await;
            assert_eq!(result.failed, 0);
            assert!(result.cursor_may_advance());
            assert_organization(&pool, &thread, &labels).await;
            let destination_id: String = sqlx::query_scalar(
                "SELECT m.id FROM messages m JOIN folders f ON f.id=m.folder_id WHERE f.remote_id=?",
            ).bind(destination).fetch_one(&pool).await.unwrap();
            assert_eq!(destination_id, original);
        }
        // Removing and re-adding a label must also preserve the other copy.
        ingest(&pool, message(&[source])).await;
        let surviving: String = sqlx::query_scalar("SELECT id FROM messages")
            .fetch_one(&pool)
            .await
            .unwrap();
        ingest(&pool, message(&labels)).await;
        assert_organization(&pool, &thread, &labels).await;
        assert_eq!(sqlx::query_scalar::<_, String>(
            "SELECT m.id FROM messages m JOIN folders f ON f.id=m.folder_id WHERE f.remote_id=?",
        ).bind(source).fetch_one(&pool).await.unwrap(), surviving);
    }
}

#[tokio::test]
async fn gmail_failed_target_savepoint_retains_old_copy_and_retries_after_restart() {
    let pool = pool().await;
    ingest(&pool, message(&["INBOX"])).await;
    let thread = annotate(&pool).await;
    sqlx::query(
        "INSERT INTO attachments (id,message_id,filename,content_type,size,created_at)
                 SELECT 'old-attachment',id,'fixture.txt','text/plain',0,'' FROM messages",
    )
    .execute(&pool)
    .await
    .unwrap();
    // Fail after stale-copy cleanup, at the final target projection refresh.
    // This proves cleanup shares the target's rollback boundary.
    sqlx::raw_sql(
        "CREATE TRIGGER fail_target_projection BEFORE UPDATE ON threads
         WHEN EXISTS (SELECT 1 FROM messages m JOIN folders f ON f.id=m.folder_id WHERE f.remote_id='TRASH')
         BEGIN SELECT RAISE(ABORT, 'fixture target failure'); END;",
    ).execute(&pool).await.unwrap();
    let failed = ingest(&pool, message(&["TRASH"])).await;
    assert_eq!(failed.failed, 1);
    assert!(!failed.cursor_may_advance());
    assert_organization(&pool, &thread, &["INBOX"]).await;
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM attachments")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT attempts FROM sync_quarantine")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    sqlx::query("UPDATE sync_quarantine SET attempts=?")
        .bind(satchel_core::db::sync_quarantine::MAX_INGEST_ATTEMPTS)
        .execute(&pool)
        .await
        .unwrap();
    let exhausted = ingest(&pool, message(&["TRASH"])).await;
    assert_eq!(exhausted.skipped, 1);
    assert!(
        !exhausted.cursor_may_advance(),
        "quarantine must not authorize deletion of the old copy"
    );
    assert_organization(&pool, &thread, &["INBOX"]).await;
    // An explicit quarantine retry is durable and permits the replacement.
    sqlx::query("DELETE FROM sync_quarantine")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DROP TRIGGER fail_target_projection")
        .execute(&pool)
        .await
        .unwrap();
    // ingest reconstructs the quarantine/context from SQLite on every call.
    let retried = ingest(&pool, message(&["TRASH"])).await;
    assert_eq!(retried.ingested, 1);
    assert!(retried.cursor_may_advance());
    assert_organization(&pool, &thread, &["TRASH"]).await;
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_quarantine")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn gmail_exhausted_new_mail_quarantine_keeps_the_normal_skip_policy() {
    let pool = pool().await;
    sqlx::query(
        "INSERT INTO sync_quarantine
         (account_id,folder_id,remote_id,attempts,last_error,first_failed_at,last_failed_at)
         VALUES ('account','inbox','A',?,'fixture failure','','')",
    )
    .bind(satchel_core::db::sync_quarantine::MAX_INGEST_ATTEMPTS)
    .execute(&pool)
    .await
    .unwrap();
    let skipped = ingest(&pool, message(&["INBOX"])).await;
    assert_eq!(skipped.skipped, 1);
    assert!(skipped.cursor_may_advance());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM messages")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn gmail_relabel_reuses_a_legacy_thread_without_header_or_provider_key() {
    let pool = pool().await;
    sqlx::raw_sql(
        "INSERT INTO threads (id,account_id,subject,last_message_date,created_at,updated_at)
         VALUES ('legacy-thread','account','Fixture','','','');
         INSERT INTO messages (id,account_id,folder_id,thread_id,remote_id,from_address,to_addresses,date,created_at,updated_at)
         VALUES ('legacy-message','account','inbox','legacy-thread','A','','[]','','','');",
    ).execute(&pool).await.unwrap();
    let thread = annotate(&pool).await;
    assert_eq!(thread, "legacy-thread");
    sqlx::query("INSERT INTO topic_items (topic_id,item_type,item_id,added_at) VALUES ('topic','message','legacy-message','')")
        .execute(&pool).await.unwrap();
    let mut moved = message(&["TRASH"]);
    moved.message_id_header = None;
    ingest(&pool, moved).await;
    assert_organization(&pool, &thread, &["TRASH"]).await;
    assert_eq!(sqlx::query_scalar::<_, String>(
        "SELECT m.id FROM messages m JOIN topic_items ti ON ti.item_id=m.id WHERE ti.item_type='message'",
    ).fetch_one(&pool).await.unwrap(), "legacy-message");
}

#[test]
fn old_sync_state_decodes_without_a_snapshot_obligation() {
    let state: ImapSyncState =
        serde_json::from_str(r#"{"cursors":{"INBOX":"old"},"folder_snapshot_at":{}}"#).unwrap();
    assert!(state.pending_snapshots.is_empty());
    assert_eq!(state.cursors["INBOX"], "old");
}

#[tokio::test]
async fn gmail_multiple_labels_and_synthetic_archive_remain_exclusive() {
    let pool = pool().await;
    ingest(&pool, message(&["INBOX"])).await;
    let thread = annotate(&pool).await;
    ingest(
        &pool,
        message(&["Label_one", "Label_two", "IMPORTANT", "CATEGORY_PERSONAL"]),
    )
    .await;
    assert_organization(&pool, &thread, &["Label_one", "Label_two"]).await;
    ingest(&pool, message(&["IMPORTANT"])).await;
    assert_organization(&pool, &thread, &["ARCHIVE"]).await;
    ingest(&pool, message(&["INBOX", "Label_one"])).await;
    assert_organization(&pool, &thread, &["INBOX", "Label_one"]).await;
}

async fn server(
    route: impl Fn(&str, &str) -> (&'static str, String) + Send + Sync + 'static,
) -> (
    String,
    Arc<std::sync::Mutex<Vec<String>>>,
    tokio::task::JoinHandle<()>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
    let recorded = requests.clone();
    let origin = base.clone();
    let task = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = [0; 8192];
            let read = socket.read(&mut buffer).await.unwrap();
            let request = String::from_utf8_lossy(&buffer[..read]);
            let line = request.lines().next().unwrap();
            recorded.lock().unwrap().push(line.to_string());
            let (status, body) = route(line, &origin);
            socket.write_all(format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len(),
            ).as_bytes()).await.unwrap();
        }
    });
    (base, requests, task)
}

fn config() -> crate::commands::accounts::AccountConfig {
    crate::commands::accounts::AccountConfig::from_plaintext(
        r#"{"oauth":{"access_token":"fixture-only","expires_at":"2099-01-01T00:00:00Z"}}"#,
    )
}

static PROVIDER_HTTP_FIXTURES: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

// HTTP fixtures serialize their process-wide URL overrides.
#[tokio::test]
async fn provider_history_progress_and_snapshot_obligations_survive_restart() {
    let _guard = PROVIDER_HTTP_FIXTURES.lock().await;
    let fail_page = Arc::new(AtomicBool::new(true));
    let page_failure = fail_page.clone();
    let (base, requests, server_task) = server(move |line, _| {
        if line.contains("pageToken=second") && page_failure.load(Ordering::SeqCst) {
            return ("400 Bad Request", "{}".into());
        }
        let body = if line.contains("/messages/") {
            let id = if line.contains("/messages/A") { "A" } else { "B" };
            json!({"id":id,"threadId":id,"historyId":"130","labelIds":["INBOX"],
                "internalDate":"1700000000000","payload":{"mimeType":"text/plain","body":{"data":"aGk="}}})
        } else if line.contains("startHistoryId=100") {
            json!({"historyId":"110","history":[{"id":"110","messagesAdded":[{"message":{"id":"A"}}]}]})
        } else if line.contains("startHistoryId=110") && !line.contains("pageToken") {
            json!({"historyId":"130","nextPageToken":"second","history":[{"id":"120","messagesAdded":[{"message":{"id":"B"}}]}]})
        } else if line.contains("startHistoryId=110") && line.contains("pageToken=second") {
            json!({"historyId":"130","history":[]})
        } else {
            panic!("unexpected history request {line}");
        };
        ("200 OK", body.to_string())
    }).await;
    std::env::set_var("SATCHEL_GMAIL_API_BASE", &base);
    let gmail = crate::providers::gmail::GmailProvider::from_account_config(&config());
    let first = gmail
        .sync_messages(
            "INBOX",
            SyncCheckpoint {
                cursor: Some(
                    r#"{"history_id":"100","format_version":3,"latest_ts":1700000000}"#.into(),
                ),
            },
        )
        .await
        .unwrap();
    let next: serde_json::Value =
        serde_json::from_str(first.next.cursor.as_deref().unwrap()).unwrap();
    assert_eq!(next["history_id"], "110");
    assert!(
        gmail
            .sync_messages("INBOX", first.next.clone())
            .await
            .is_err(),
        "a failed continuation must not authorize partial progress"
    );
    fail_page.store(false, Ordering::SeqCst);
    let second = gmail.sync_messages("INBOX", first.next).await.unwrap();
    assert_eq!(
        second
            .messages
            .iter()
            .map(|m| m.remote_id.as_str())
            .collect::<Vec<_>>(),
        ["B"]
    );
    assert!(requests
        .lock()
        .unwrap()
        .iter()
        .any(|r| r.contains("startHistoryId=110") && r.contains("pageToken=second")));
    server_task.abort();

    for outlook in [false, true] {
        snapshot_restart(outlook).await;
    }
    std::env::remove_var("SATCHEL_GMAIL_API_BASE");
    std::env::remove_var("SATCHEL_GRAPH_API_BASE");
}

async fn snapshot_restart(outlook: bool) {
    let fail_snapshot = Arc::new(AtomicBool::new(true));
    let fail = fail_snapshot.clone();
    let fail_reset = Arc::new(AtomicBool::new(true));
    let reset_failure = fail_reset.clone();
    let (base, requests, server_task) = server(move |line, base| {
        if line.contains("/expired") || line.contains("startHistoryId=100") {
            return (
                if outlook { "410 Gone" } else { "404 Not Found" },
                "{}".into(),
            );
        }
        if reset_failure.load(Ordering::SeqCst)
            && (line.contains("q=after") || line.contains("/messages/delta"))
        {
            return ("400 Bad Request", "{}".into());
        }
        if line.contains("/messages/A") {
            return ("404 Not Found", "{}".into());
        }
        if line.contains("/profile") {
            return ("200 OK", json!({"historyId":"200"}).to_string());
        }
        if line.contains("/history") {
            return (
                "200 OK",
                json!({"historyId":"200","history":[]}).to_string(),
            );
        }
        if line.contains("/messages/delta") || line.contains("/fresh") {
            return (
                "200 OK",
                json!({"value":[],"@odata.deltaLink":format!("{base}/fresh")}).to_string(),
            );
        }
        // The snapshot fails on page two. A partial page must never reconcile.
        if line.contains("snapshot-page-2") {
            return if fail.load(Ordering::SeqCst) {
                ("400 Bad Request", "{}".into())
            } else if outlook {
                ("200 OK", json!({"value":[]}).to_string())
            } else {
                ("200 OK", json!({"messages":[]}).to_string())
            };
        }
        if line.contains("/messages?") {
            if line.contains("q=after") {
                return ("200 OK", json!({"messages":[]}).to_string());
            }
            return if outlook {
                (
                    "200 OK",
                    json!({"value":[],"@odata.nextLink":format!("{base}/snapshot-page-2")})
                        .to_string(),
                )
            } else {
                (
                    "200 OK",
                    json!({"messages":[],"nextPageToken":"snapshot-page-2"}).to_string(),
                )
            };
        }
        panic!("unexpected reset request {line}");
    })
    .await;
    let variable = if outlook {
        "SATCHEL_GRAPH_API_BASE"
    } else {
        "SATCHEL_GMAIL_API_BASE"
    };
    std::env::set_var(variable, &base);
    let provider: Box<dyn MailProvider> = if outlook {
        Box::new(crate::providers::outlook::OutlookProvider::from_account_config(&config()))
    } else {
        Box::new(crate::providers::gmail::GmailProvider::from_account_config(
            &config(),
        ))
    };
    let folder = if outlook { "inbox" } else { "INBOX" };
    let cursor = if outlook {
        json!({"delta_link":format!("{base}/expired"),"format_version":1})
    } else {
        json!({"history_id":"100","latest_ts":1700000000,"format_version":3})
    };
    let pool = pool().await;
    ingest(&pool, message(&["INBOX"])).await;
    let queue = SyncQueue::new();
    let mut state = ImapSyncState::default();
    // A failed reset has no replacement batch to commit. Keep the usable
    // persisted checkpoint so a restart retries the gap instead of losing it.
    state.cursors.insert(folder.into(), cursor.to_string());
    persist_sync_cursors(&pool, "account", &state, &queue, None)
        .await
        .unwrap();
    assert!(provider
        .sync_messages(
            folder,
            SyncCheckpoint {
                cursor: state.cursors.get(folder).cloned(),
            }
        )
        .await
        .is_err());
    let raw: String = sqlx::query_scalar("SELECT sync_state FROM accounts")
        .fetch_one(&pool)
        .await
        .unwrap();
    let restored: ImapSyncState = serde_json::from_str(&raw).unwrap();
    assert_eq!(restored.cursors[folder], cursor.to_string());
    fail_reset.store(false, Ordering::SeqCst);
    let batch = provider
        .sync_messages(
            folder,
            SyncCheckpoint {
                cursor: Some(cursor.to_string()),
            },
        )
        .await
        .unwrap();
    assert!(batch.snapshot_required);
    assert!(state.request_snapshot(folder, true, batch.snapshot_required));
    state
        .cursors
        .insert(folder.into(), batch.next.cursor.unwrap());
    persist_sync_cursors(&pool, "account", &state, &queue, None)
        .await
        .unwrap();
    assert!(reconcile_test_snapshot(
        &pool,
        provider.as_ref(),
        "account",
        "inbox",
        folder,
        &mut state,
        &queue
    )
    .await
    .is_err());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM messages")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    // Reconstruct exactly what a fresh process loads, with no transient batch.
    let raw: String = sqlx::query_scalar("SELECT sync_state FROM accounts")
        .fetch_one(&pool)
        .await
        .unwrap();
    let mut state: ImapSyncState = serde_json::from_str(&raw).unwrap();
    assert!(state.pending_snapshots.contains(folder));
    fail_snapshot.store(false, Ordering::SeqCst);
    let next = provider
        .sync_messages(
            folder,
            SyncCheckpoint {
                cursor: state.cursors.get(folder).cloned(),
            },
        )
        .await
        .unwrap();
    assert!(!next.snapshot_required);
    assert!(state.request_snapshot(folder, true, next.snapshot_required));
    state
        .cursors
        .insert(folder.into(), next.next.cursor.unwrap());
    persist_sync_cursors(&pool, "account", &state, &queue, None)
        .await
        .unwrap();
    reconcile_test_snapshot(
        &pool,
        provider.as_ref(),
        "account",
        "inbox",
        folder,
        &mut state,
        &queue,
    )
    .await
    .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM messages")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    let raw: String = sqlx::query_scalar("SELECT sync_state FROM accounts")
        .fetch_one(&pool)
        .await
        .unwrap();
    let mut state: ImapSyncState = serde_json::from_str(&raw).unwrap();
    assert!(state.pending_snapshots.is_empty());
    let ordinary = provider
        .sync_messages(
            folder,
            SyncCheckpoint {
                cursor: state.cursors.get(folder).cloned(),
            },
        )
        .await
        .unwrap();
    assert!(
        !state.request_snapshot(folder, true, ordinary.snapshot_required),
        "ordinary delta must not loop snapshots"
    );
    assert!(
        state.request_snapshot(folder, true, true),
        "periodic/draft snapshot requests remain supported"
    );
    assert!(requests
        .lock()
        .unwrap()
        .iter()
        .any(|line| line.contains("snapshot-page-2")));
    server_task.abort();
}

/// Completed backfills leave the reset run with only time-based catch-up.
/// A predates that window; only resolving absent snapshot IDs can find its move.
#[tokio::test]
async fn gmail_expired_history_snapshots_preserve_moves_and_retry_unresolved_ids() {
    let _guard = PROVIDER_HTTP_FIXTURES.lock().await;
    for scenario in [
        "still-inbox",
        "archive",
        "move",
        "existing",
        "deleted",
        "lookup",
        "auth",
        "malformed",
        "target",
        "listing-before-target",
        "quarantine",
        "pending",
        "pending-deleted",
    ] {
        let fail = Arc::new(AtomicBool::new(true));
        let failure = fail.clone();
        let (base, requests, task) = server(move |line, _| {
            if line.contains("startHistoryId=100") {
                return ("404 Not Found", "{}".into());
            }
            if line.contains("/history") || line.contains("/profile") {
                return ("200 OK", json!({"historyId":"200"}).to_string());
            }
            if line.contains("/messages/A") {
                if scenario == "deleted" || scenario == "pending-deleted" { return ("404 Not Found", "{}".into()); }
                if failure.load(Ordering::SeqCst) {
                    match scenario {
                        "lookup" => return ("503 Service Unavailable", "{}".into()),
                        "auth" => return ("401 Unauthorized", "{}".into()),
                        "malformed" => return ("200 OK", json!({"id":"wrong-id","payload":{"mimeType":"text/plain"}}).to_string()),
                        _ => {}
                    }
                }
                let labels = if scenario == "still-inbox" { vec!["INBOX"] } else if scenario == "archive" { vec![] } else if scenario == "existing" || scenario == "target" || scenario == "listing-before-target" {
                    vec!["Label_one", "Label_two"]
                } else { vec!["Label_one"] };
                return ("200 OK", json!({"id":"A","threadId":"conversation","labelIds":labels,
                    "internalDate":"1767225600000", "payload":{"mimeType":"text/plain",
                    "headers":[{"name":"Message-ID","value":"<a@example.test>"}],"body":{"data":"Rml4dHVyZQ=="}}}).to_string());
            }
            if line.contains("/messages?") {
                if scenario == "listing-before-target" && failure.load(Ordering::SeqCst) && !line.contains("q=after") {
                    return ("503 Service Unavailable", "{}".into());
                }
                return ("200 OK", json!({"messages":[]}).to_string());
            }
            panic!("unexpected reset move request {line}");
        }).await;
        std::env::set_var("SATCHEL_GMAIL_API_BASE", &base);
        let provider = crate::providers::gmail::GmailProvider::from_account_config(&config());
        let pool = pool().await;
        // Folder discovery already completed, even when its backfill was empty.
        ensure_gmail_label_folder(&pool, "account", "Label_one")
            .await
            .unwrap();
        ingest(
            &pool,
            message(if scenario == "existing" || scenario == "deleted" {
                &["INBOX", "Label_one"]
            } else {
                &["INBOX"]
            }),
        )
        .await;
        let thread = annotate(&pool).await;
        let source: String = sqlx::query_scalar("SELECT id FROM messages WHERE folder_id='inbox'")
            .fetch_one(&pool)
            .await
            .unwrap();
        let target: Option<String> =
            sqlx::query_scalar("SELECT id FROM messages WHERE folder_id!='inbox'")
                .fetch_optional(&pool)
                .await
                .unwrap();
        sqlx::query("INSERT INTO topic_items (topic_id,item_type,item_id,added_at,added_by) VALUES ('topic','message',?,'source-date','user')")
            .bind(&source).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO attachments (id,message_id,filename,content_type,size,created_at) VALUES ('cached',?,'fixture.txt','text/plain',0,'')")
            .bind(&source).execute(&pool).await.unwrap();
        if scenario == "target" || scenario == "listing-before-target" {
            // Fail the SECOND target after the first has already transferred A.
            sqlx::raw_sql(
                "CREATE TRIGGER fail_second_target BEFORE INSERT ON messages
                WHEN NEW.folder_id IN (SELECT id FROM folders WHERE remote_id='Label_two')
                BEGIN SELECT RAISE(ABORT, 'fixture second target failure'); END;",
            )
            .execute(&pool)
            .await
            .unwrap();
        }
        if scenario == "quarantine" {
            let (folder, _) = ensure_gmail_label_folder(&pool, "account", "Label_one")
                .await
                .unwrap();
            let repo = SyncQuarantineRepository::new(&pool);
            for _ in 0..satchel_core::db::sync_quarantine::MAX_INGEST_ATTEMPTS {
                repo.record_failure("account", &folder, "A", "fixture")
                    .await
                    .unwrap();
            }
        }
        let mut state = ImapSyncState::default();
        let old = json!({"history_id":"100","format_version":3,"latest_ts":1790000000});
        for folder in ["INBOX", "Label_one"] {
            state.cursors.insert(folder.into(), old.to_string());
        }
        let queue = SyncQueue::new();
        if scenario.starts_with("pending") {
            queue
                .enqueue(
                    &pool,
                    QueuedAction::new(
                        "account".into(),
                        ACTION_REMOTE_MUTATION,
                        json!({"kind":"move", "refs":[{"remote_id":"A","source_folder":"INBOX"}],
                    "target_remote_folder_id":"Label_one"}),
                    ),
                )
                .await
                .unwrap();
        }
        for folder in ["INBOX", "Label_one"] {
            let batch = provider
                .sync_messages(
                    folder,
                    SyncCheckpoint {
                        cursor: state.cursors.get(folder).cloned(),
                    },
                )
                .await
                .unwrap();
            assert!(
                batch.messages.is_empty(),
                "old A must be absent from time-based catch-up"
            );
            assert!(batch.snapshot_required);
            assert!(state.request_snapshot(folder, true, batch.snapshot_required));
            state
                .cursors
                .insert(folder.into(), batch.next.cursor.unwrap());
        }
        persist_sync_cursors(&pool, "account", &state, &queue, None)
            .await
            .unwrap();
        let result = reconcile_test_snapshot(
            &pool, &provider, "account", "inbox", "INBOX", &mut state, &queue,
        )
        .await;
        let should_fail = matches!(
            scenario,
            "lookup"
                | "auth"
                | "malformed"
                | "target"
                | "listing-before-target"
                | "quarantine"
                | "pending"
                | "pending-deleted"
        );
        assert_eq!(result.is_err(), should_fail, "{scenario}: {result:?}");
        if should_fail {
            assert_organization(&pool, &thread, &["INBOX"]).await;
            assert_eq!(
                sqlx::query_scalar::<_, String>(
                    "SELECT item_id FROM topic_items WHERE item_type='message'"
                )
                .fetch_one(&pool)
                .await
                .unwrap(),
                source
            );
            assert_eq!(
                sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM attachments")
                    .fetch_one(&pool)
                    .await
                    .unwrap(),
                1
            );
        }
        let raw: String = sqlx::query_scalar("SELECT sync_state FROM accounts")
            .fetch_one(&pool)
            .await
            .unwrap();
        let mut restored: ImapSyncState = serde_json::from_str(&raw).unwrap();
        assert_eq!(restored.pending_snapshots.contains("INBOX"), should_fail);
        if should_fail {
            fail.store(false, Ordering::SeqCst);
            if scenario.starts_with("pending") {
                sqlx::query("DELETE FROM sync_queue")
                    .execute(&pool)
                    .await
                    .unwrap();
            }
            if scenario == "target" || scenario == "listing-before-target" {
                // Ordinary backfill can land the first label while the second
                // still fails. The durable ID must survive losing INBOX's row.
                let partial = ingest(&pool, message(&["Label_one", "Label_two"])).await;
                assert_eq!(partial.failed, 1);
                assert_organization(&pool, &thread, &["Label_one"]).await;
                if scenario == "target" {
                    assert!(restored.snapshot_resolutions["INBOX"].contains("A"));
                } else {
                    assert!(!restored.snapshot_resolutions.contains_key("INBOX"));
                }
            }
            if scenario == "target" || scenario == "listing-before-target" {
                sqlx::raw_sql("DROP TRIGGER fail_second_target")
                    .execute(&pool)
                    .await
                    .unwrap();
            }
            if scenario == "quarantine" {
                sqlx::query("DELETE FROM sync_quarantine")
                    .execute(&pool)
                    .await
                    .unwrap();
            }
            // A fresh provider and state use the persisted obligation even though
            // the next successful history response no longer requests a snapshot.
            let provider = crate::providers::gmail::GmailProvider::from_account_config(&config());
            let next = provider
                .sync_messages(
                    "INBOX",
                    SyncCheckpoint {
                        cursor: restored.cursors.get("INBOX").cloned(),
                    },
                )
                .await
                .unwrap();
            assert!(!next.snapshot_required);
            assert!(restored.request_snapshot("INBOX", true, next.snapshot_required));
            reconcile_test_snapshot(
                &pool,
                &provider,
                "account",
                "inbox",
                "INBOX",
                &mut restored,
                &queue,
            )
            .await
            .unwrap();
        }
        if scenario == "deleted" || scenario == "pending-deleted" {
            for table in [
                "messages",
                "threads",
                "thread_tags",
                "topic_items",
                "workflow_events",
                "attachments",
            ] {
                assert_eq!(
                    sqlx::query_scalar::<_, i64>(&format!("SELECT COUNT(*) FROM {table}"))
                        .fetch_one(&pool)
                        .await
                        .unwrap(),
                    0,
                    "{table}"
                );
            }
        } else {
            let labels: &[&str] = if scenario == "still-inbox" {
                &["INBOX"]
            } else if scenario == "archive" {
                &["ARCHIVE"]
            } else if scenario == "existing"
                || scenario == "target"
                || scenario == "listing-before-target"
            {
                &["Label_one", "Label_two"]
            } else {
                &["Label_one"]
            };
            assert_organization(&pool, &thread, labels).await;
            let survivor: String = sqlx::query_scalar("SELECT m.id FROM messages m JOIN folders f ON f.id=m.folder_id WHERE f.remote_id=?")
                .bind(labels[0])
                .fetch_one(&pool).await.unwrap();
            assert_eq!(survivor, target.unwrap_or(source));
            assert_eq!(
                sqlx::query_as::<_, (String, String, String)>(
                    "SELECT item_id,added_at,added_by FROM topic_items WHERE item_type='message'"
                )
                .fetch_one(&pool)
                .await
                .unwrap(),
                (survivor, "source-date".into(), "user".into())
            );
        }
        let raw: String = sqlx::query_scalar("SELECT sync_state FROM accounts")
            .fetch_one(&pool)
            .await
            .unwrap();
        let done: ImapSyncState = serde_json::from_str(&raw).unwrap();
        assert!(!done.pending_snapshots.contains("INBOX"));
        assert!(!done.snapshot_resolutions.contains_key("INBOX"));
        assert!(done.pending_snapshots.contains("Label_one"));
        let recorded = requests.lock().unwrap();
        assert!(recorded.iter().any(|r| r.contains("q=after")));
        assert!(
            recorded.iter().any(|r| r.contains("/messages/A")),
            "absent A requires a global lookup"
        );
        assert!(
            !recorded
                .iter()
                .any(|r| r.contains("/messages/") && !r.contains("/messages/A")),
            "only absent IDs need downloading"
        );
        task.abort();
    }
    std::env::remove_var("SATCHEL_GMAIL_API_BASE");
}

async fn reconcile_test_snapshot(
    pool: &SqlitePool,
    provider: &dyn MailProvider,
    account_id: &str,
    folder_id: &str,
    remote_folder_id: &str,
    state: &mut ImapSyncState,
    queue: &SyncQueue,
) -> Result<HashSet<String>, String> {
    let provider_kind = if remote_folder_id == "inbox" {
        "outlook"
    } else {
        "gmail"
    };
    let account = MailAccountRow {
        id: account_id.into(),
        display_name: "Fixture".into(),
        email: "me@example.test".into(),
        provider: provider_kind.into(),
        config: "{}".into(),
        sync_state: None,
        outlook_immutable_ids_migrated: 1,
        ingest_cutoff: None,
    };
    reconcile_folder_snapshot(
        pool,
        provider,
        &FolderIngestContext {
            app: None,
            events: None,
            account: &account,
            config: &config(),
            folder_id,
            folder_remote_id: remote_folder_id,
            folder_type: "inbox",
        },
        state,
        queue,
    )
    .await
}
