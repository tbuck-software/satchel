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
    // Neither the folder cursor nor the account's snapshots wait for it.
    // Gmail snapshots delete only after a 404 and replace only after the
    // target commits, so the old copy survives the skip.
    assert_eq!(exhausted.failed, 0);
    assert!(exhausted.cursor_may_advance());
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
    assert!(state.resolved_snapshot_copies.is_empty());
    assert_eq!(state.cursors["INBOX"], "old");
}

#[test]
fn a_vanished_folder_leaves_no_snapshot_state_behind() {
    let mut state = ImapSyncState::default();
    for folder in ["Label_kept", "Label_gone"] {
        state.cursors.insert(folder.into(), "cursor".into());
        state
            .folder_snapshot_at
            .insert(folder.into(), "2026-01-01T00:00:00Z".into());
        state.pending_snapshots.insert(folder.into());
        state
            .snapshot_resolutions
            .insert(folder.into(), HashSet::from(["owed".into()]));
        state
            .resolved_snapshot_copies
            .insert(folder.into(), HashSet::from(["resolved".into()]));
    }
    state.retain_folders(&HashSet::from(["Label_kept".into()]));
    let kept = ["Label_kept".to_string()];
    assert!(state.cursors.keys().eq(&kept));
    assert!(state.folder_snapshot_at.keys().eq(&kept));
    assert!(state.pending_snapshots.iter().eq(&kept));
    assert!(state.snapshot_resolutions.keys().eq(&kept));
    assert!(state.resolved_snapshot_copies.keys().eq(&kept));
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
    server_async(move |line, origin| {
        let response = route(&line, &origin);
        async move { response }
    })
    .await
}

async fn server_async<F: std::future::Future<Output = (&'static str, String)> + Send>(
    route: impl Fn(String, String) -> F + Send + Sync + 'static,
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
            let Ok((mut socket, _)) = listener.accept().await else {
                continue;
            };
            let mut buffer = [0; 8192];
            // The code under test writes its request right after connecting.
            // A silent foreign connection must not stall the fixture.
            let read =
                tokio::time::timeout(std::time::Duration::from_secs(5), socket.read(&mut buffer))
                    .await;
            let Ok(Ok(read)) = read else {
                continue;
            };
            let Some(line) = api_request_line(&buffer[..read]) else {
                // Not an HTTP request for an API path: a probe from another
                // local process, never a request the code under test made.
                let _ = socket
                    .write_all(
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    )
                    .await;
                continue;
            };
            recorded.lock().unwrap().push(line.clone());
            let (status, body) = route(line, origin.clone()).await;
            // The client may already have given up (deadline tests).
            let _ = socket.write_all(format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len(),
            ).as_bytes()).await;
        }
    });
    (base, requests, task)
}

/// Provider APIs never live at the root, so `GET /` is a probe as well.
fn api_request_line(request: &[u8]) -> Option<String> {
    let line = std::str::from_utf8(request).ok()?.lines().next()?;
    let mut parts = line.split(' ');
    let (method, target, version) = (parts.next()?, parts.next()?, parts.next()?);
    let well_formed = parts.next().is_none()
        && !method.is_empty()
        && method.bytes().all(|byte| byte.is_ascii_uppercase())
        && target.starts_with('/')
        && target != "/"
        && version.starts_with("HTTP/1.");
    well_formed.then(|| line.to_string())
}

/// Local tooling on a developer machine may probe every new loopback listener
/// with `GET /` or a TLS ClientHello. Neither may stop the fixture, nor appear
/// among the requests the code under test made.
#[tokio::test]
async fn fixture_server_survives_foreign_probes_and_records_only_api_requests() {
    let (base, requests, task) = server(|line, _| {
        assert!(line.starts_with("GET /messages/A"), "routed {line}");
        ("200 OK", json!({"id":"A"}).to_string())
    })
    .await;
    let address = base.trim_start_matches("http://").to_string();
    let tls_client_hello = [
        0x16, 0x03, 0x01, 0x06, 0x3c, 0x01, 0x00, 0x06, 0x38, 0x03, 0x03,
    ];
    for probe in [
        b"GET / HTTP/1.1\r\nhost: localhost\r\nuser-agent: node\r\n\r\n".as_slice(),
        tls_client_hello.as_slice(),
        b"".as_slice(),
    ] {
        let mut socket = tokio::net::TcpStream::connect(&address).await.unwrap();
        socket.write_all(probe).await.unwrap();
        socket.shutdown().await.unwrap();
        let mut ignored = Vec::new();
        let _ = socket.read_to_end(&mut ignored).await;
    }
    let response = reqwest::get(format!("{base}/messages/A?format=full"))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        *requests.lock().unwrap(),
        ["GET /messages/A?format=full HTTP/1.1"]
    );
    task.abort();
}

fn config() -> crate::commands::accounts::AccountConfig {
    crate::commands::accounts::AccountConfig::from_plaintext(
        r#"{"oauth":{"access_token":"fixture-only","expires_at":"2099-01-01T00:00:00Z"}}"#,
    )
}

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
                "internalDate":"1700000000000","payload":{"mimeType":"text/plain","headers":[{"name":"From","value":"sender@example.test"}],"body":{"data":"aGk="}}})
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
        "empty-payload",
        "target",
        "listing-before-target",
        "quarantine",
        "quarantine-retry",
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
                        "empty-payload" => return ("200 OK", json!({"id":"A","payload":{}}).to_string()),
                        "malformed" => return ("200 OK", json!({"id":"wrong-id","payload":{"mimeType":"text/plain"}}).to_string()),
                        _ => {}
                    }
                }
                let labels = if scenario == "still-inbox" { vec!["INBOX"] } else if scenario == "archive" { vec![] } else if scenario == "existing" || scenario == "target" || scenario == "listing-before-target" {
                    vec!["Label_one", "Label_two"]
                } else { vec!["Label_one"] };
                return ("200 OK", json!({"id":"A","threadId":"conversation","historyId":"300","labelIds":labels,
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
        if scenario.starts_with("quarantine") {
            let (folder, _) = ensure_gmail_label_folder(&pool, "account", "Label_one")
                .await
                .unwrap();
            let repo = SyncQuarantineRepository::new(&pool);
            let attempts = if scenario == "quarantine" {
                satchel_core::db::sync_quarantine::MAX_INGEST_ATTEMPTS
            } else {
                1
            };
            for _ in 0..attempts {
                repo.record_failure("account", &folder, "A", "fixture")
                    .await
                    .unwrap();
            }
        }
        let mut state = ImapSyncState::default();
        if scenario == "quarantine-retry" {
            // A later partial delivery must override an earlier completion.
            state
                .resolved_snapshot_copies
                .insert("INBOX".into(), HashSet::from(["A".into()]));
        }
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
        let before: String = sqlx::query_scalar("SELECT json_object('subject',subject,'from',from_address,'date',date,'text',body_text,'html',body_html) FROM messages WHERE id=?")
            .bind(&source).fetch_one(&pool).await.unwrap();
        let result = reconcile_test_snapshot(
            &pool, &provider, "account", "inbox", "INBOX", &mut state, &queue,
        )
        .await;
        // Only a failed listing errors the folder. A deferred ID stays owed
        // while the rest of the snapshot proceeds.
        let listing_failed = scenario == "listing-before-target";
        let deferred = matches!(
            scenario,
            "lookup"
                | "auth"
                | "malformed"
                | "empty-payload"
                | "target"
                | "quarantine"
                | "pending"
                | "pending-deleted"
        );
        // The queue or the quarantine owns these; the others failed.
        let held = matches!(scenario, "quarantine" | "pending" | "pending-deleted");
        let should_fail = listing_failed || deferred;
        if listing_failed {
            assert!(result.is_err(), "{scenario}: {result:?}");
        } else {
            let progress = result.unwrap_or_else(|error| panic!("{scenario}: {error}"));
            assert_eq!(
                progress.deferred.keys().collect::<Vec<_>>(),
                if deferred { vec!["A"] } else { vec![] },
                "{scenario}"
            );
            if deferred {
                assert_eq!(
                    matches!(progress.deferred["A"], SnapshotDeferral::Failed(_)),
                    !held,
                    "{scenario}: {:?}",
                    progress.deferred["A"]
                );
            }
        }
        if should_fail {
            let after: String = sqlx::query_scalar("SELECT json_object('subject',subject,'from',from_address,'date',date,'text',body_text,'html',body_html) FROM messages WHERE id=?")
                .bind(&source).fetch_one(&pool).await.unwrap();
            assert_eq!(after, before, "{scenario}: cached metadata/content changed");
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
        // Only a failed listing lists again. Every ID a completed listing
        // found absent is owed, and owed IDs need lookups only.
        assert_eq!(
            restored.pending_snapshots.contains("INBOX"),
            listing_failed,
            "{scenario}"
        );
        assert_eq!(
            restored.folder_snapshot_at.contains_key("INBOX"),
            !listing_failed,
            "{scenario}"
        );
        if deferred {
            assert!(
                restored.snapshot_resolutions["INBOX"].contains("A"),
                "{scenario}"
            );
        }
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
            assert_eq!(
                restored.request_snapshot("INBOX", true, next.snapshot_required),
                listing_failed,
                "{scenario}"
            );
            if listing_failed {
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
            } else {
                // The next ordinary poll looks up A once its holder let go.
                resolve_test_owed(&pool, &provider, "inbox", "INBOX", &mut restored, &queue)
                    .await
                    .unwrap();
            }
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

fn test_account(account_id: &str, remote_folder_id: &str) -> MailAccountRow {
    let provider_kind = if remote_folder_id == "inbox" {
        "outlook"
    } else {
        "gmail"
    };
    MailAccountRow {
        id: account_id.into(),
        display_name: "Fixture".into(),
        email: "me@example.test".into(),
        provider: provider_kind.into(),
        config: "{}".into(),
        sync_state: None,
        outlook_immutable_ids_migrated: 1,
        ingest_cutoff: None,
    }
}

async fn reconcile_test_snapshot(
    pool: &SqlitePool,
    provider: &dyn MailProvider,
    account_id: &str,
    folder_id: &str,
    remote_folder_id: &str,
    state: &mut ImapSyncState,
    queue: &SyncQueue,
) -> Result<SnapshotProgress, String> {
    let account = test_account(account_id, remote_folder_id);
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

/// The listing-free pass an ordinary poll runs for a folder with owed IDs.
async fn resolve_test_owed(
    pool: &SqlitePool,
    provider: &dyn MailProvider,
    folder_id: &str,
    remote_folder_id: &str,
    state: &mut ImapSyncState,
    queue: &SyncQueue,
) -> Result<SnapshotProgress, String> {
    assert!(
        state.owes_resolutions(remote_folder_id),
        "an ordinary poll would not run this pass"
    );
    let account = test_account("account", remote_folder_id);
    resolve_owed_snapshot_ids(
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

/// Stop during the second 100-ID page, then reconstruct both provider and
/// state. Completed moves, deletions and copies still in the source must not
/// spend the next budget.
#[tokio::test]
async fn gmail_snapshot_resolution_progress_survives_budget_and_restart() {
    let _guard = PROVIDER_HTTP_FIXTURES.lock().await;
    let reached = Arc::new(tokio::sync::Notify::new());
    let reached_server = reached.clone();
    let (base, requests, task) = server_async(move |line, _| {
        let reached = reached_server.clone();
        async move {
            if line.contains("/messages?") {
                return ("200 OK", json!({"messages":[]}).to_string());
            }
            let id = line
                .split("/messages/")
                .nth(1)
                .unwrap()
                .split('?')
                .next()
                .unwrap();
            if id == "M150" {
                reached.notify_one();
                std::future::pending::<()>().await;
            }
            if id == "M000" {
                return ("404 Not Found", "{}".into());
            }
            (
                "200 OK",
                json!({"id":id,"threadId":id,"historyId":"300","labelIds": if id == "M001" { vec!["INBOX"] } else { vec!["Label_one"] },
                "internalDate":"1767225600000","payload":{"mimeType":"text/plain",
                "headers":[{"name":"From","value":"sender@example.test"}],
                "body":{"data":"Rml4dHVyZQ=="}}})
                .to_string(),
            )
        }
    })
    .await;
    std::env::set_var("SATCHEL_GMAIL_API_BASE", &base);
    let pool = pool().await;
    for n in 0..205 {
        let mut m = message(&["INBOX"]);
        m.remote_id = format!("M{n:03}");
        m.remote_thread_id = Some(m.remote_id.clone());
        m.message_id_header = Some(format!("<M{n:03}@example.test>"));
        assert_eq!(ingest(&pool, m).await.ingested, 1);
    }
    let queue = SyncQueue::new();
    let mut state = ImapSyncState::default();
    state
        .cursors
        .insert("Label_other".into(), "unrelated-cursor".into());
    state.pending_snapshots.insert("Label_other".into());
    state.snapshot_resolutions.insert(
        "Label_other".into(),
        HashSet::from(["unresolved-other".into()]),
    );
    state.resolved_snapshot_copies.insert(
        "Label_other".into(),
        HashSet::from(["completed-other".into()]),
    );
    let provider = crate::providers::gmail::GmailProvider::from_account_config(&config());
    let mut run = Box::pin(reconcile_test_snapshot(
        &pool, &provider, "account", "inbox", "INBOX", &mut state, &queue,
    ));
    tokio::select! {
        _ = reached.notified() => {},
        result = &mut run => panic!("sync finished before interruption: {result:?}"),
    }
    // Exercise the production deadline wrapper at a deterministic network wait.
    let result = account_run_with_deadline(std::time::Duration::from_millis(50), run).await;
    assert!(result.unwrap_err().contains("timed out"));
    drop(state);
    drop(provider);
    task.abort();

    let raw: String = sqlx::query_scalar("SELECT sync_state FROM accounts")
        .fetch_one(&pool)
        .await
        .unwrap();
    let mut restored: ImapSyncState = serde_json::from_str(&raw).unwrap();
    assert!(restored.pending_snapshots.contains("INBOX"));
    assert!(!restored.folder_snapshot_at.contains_key("INBOX"));
    let owed = &restored.snapshot_resolutions["INBOX"];
    for n in 0..148 {
        assert!(
            !owed.contains(&format!("M{n:03}")),
            "committed M{n:03} still consumes retry budget"
        );
    }
    assert!(owed.contains("M150"));
    assert_eq!(
        restored.resolved_snapshot_copies["INBOX"],
        HashSet::from(["M001".into()])
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM messages WHERE remote_id='M000'")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0,
        "global deletion committed"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM messages WHERE folder_id='inbox'")
            .fetch_one(&pool)
            .await
            .unwrap(),
        58
    );

    let (base, retried, task) = server(|line, _| {
        if line.contains("/messages?") {
            return ("200 OK", json!({"messages":[]}).to_string());
        }
        let id = line
            .split("/messages/")
            .nth(1)
            .unwrap()
            .split('?')
            .next()
            .unwrap();
        (
            "200 OK",
            json!({"id":id,"threadId":id,"historyId":"300","labelIds": if id == "M001" { vec!["INBOX"] } else { vec!["Label_one"] },
            "internalDate":"1767225600000","payload":{"mimeType":"text/plain",
            "headers":[{"name":"From","value":"sender@example.test"}],
            "body":{"data":"Rml4dHVyZQ=="}}})
            .to_string(),
        )
    })
    .await;
    std::env::set_var("SATCHEL_GMAIL_API_BASE", &base);
    let provider = crate::providers::gmail::GmailProvider::from_account_config(&config());
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
    let retry_ids: Vec<String> = retried
        .lock()
        .unwrap()
        .iter()
        .filter_map(|line| line.split("/messages/").nth(1))
        .map(|path| path.split('?').next().unwrap().to_string())
        .collect();
    assert_eq!(
        retry_ids,
        (148..205).map(|n| format!("M{n:03}")).collect::<Vec<_>>()
    );
    assert_eq!(
        requests
            .lock()
            .unwrap()
            .iter()
            .filter(|line| line.contains("/messages/M000?"))
            .count(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM messages")
            .fetch_one(&pool)
            .await
            .unwrap(),
        204
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM messages WHERE folder_id='inbox'")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    let raw: String = sqlx::query_scalar("SELECT sync_state FROM accounts")
        .fetch_one(&pool)
        .await
        .unwrap();
    let completed: ImapSyncState = serde_json::from_str(&raw).unwrap();
    assert!(!completed.pending_snapshots.contains("INBOX"));
    assert!(completed.folder_snapshot_at.contains_key("INBOX"));
    assert!(!completed.snapshot_resolutions.contains_key("INBOX"));
    assert!(!completed.resolved_snapshot_copies.contains_key("INBOX"));
    assert_eq!(
        completed.resolved_snapshot_copies["Label_other"],
        HashSet::from(["completed-other".into()])
    );
    assert_eq!(completed.cursors["Label_other"], "unrelated-cursor");
    assert!(completed.pending_snapshots.contains("Label_other"));
    assert_eq!(
        completed.snapshot_resolutions["Label_other"],
        HashSet::from(["unresolved-other".into()])
    );
    std::env::remove_var("SATCHEL_GMAIL_API_BASE");
    task.abort();
}

fn full_response() -> serde_json::Value {
    json!({"id":"A","threadId":"conversation","historyId":"300","labelIds":[],
        "internalDate":"1767225600000","payload":{"mimeType":"text/plain",
        "headers":[{"name":"From","value":"sender@example.test"},
                   {"name":"Subject","value":"Keep my organization"},
                   {"name":"Message-ID","value":"<a@example.test>"}],
        "body":{"size":7,"data":"Rml4dHVyZQ=="}}})
}

#[tokio::test]
async fn gmail_incomplete_snapshot_responses_preserve_cached_content_and_work() {
    use base64::Engine;
    let _guard = PROVIDER_HTTP_FIXTURES.lock().await;
    for scenario in [
        "missing-id",
        "missing-thread-id",
        "blank-thread-id",
        "missing-history-id",
        "blank-history-id",
        "bad-history-id",
        "identity-only",
        "missing-labels",
        "unconfirmed-labels",
        "empty-payload",
        "wrong-id",
        "missing-payload",
        "bad-labels",
        "blank-label",
        "missing-headers",
        "empty-headers",
        "bad-headers",
        "bad-header-name",
        "bad-date",
        "missing-date",
        "missing-mime",
        "missing-body",
        "bad-data",
        "missing-data",
        "negative-size",
        "empty-parts",
        "invalid-part",
        "bad-raw",
        "garbage-raw",
        "truncated-raw",
        "wrong-raw-id",
    ] {
        let fail = Arc::new(AtomicBool::new(true));
        let failure = fail.clone();
        let (base, _, task) = server(move |line, _| {
            if line.contains("/messages?") {
                return ("200 OK", json!({"messages":[]}).to_string());
            }
            let mut response = full_response();
            if failure.load(Ordering::SeqCst) {
                let fields = response.as_object_mut().unwrap();
                match scenario {
                    "missing-id" => { fields.remove("id"); },
                    "missing-thread-id" => { fields.remove("threadId"); },
                    "blank-thread-id" => response["threadId"] = json!(" "),
                    "missing-history-id" => { fields.remove("historyId"); },
                    "blank-history-id" => response["historyId"] = json!(""),
                    "bad-history-id" => response["historyId"] = json!("not-a-number"),
                    // Probe: only the fields a usable body needs.
                    "identity-only" => fields.retain(|key, _| matches!(key.as_str(), "id" | "internalDate" | "payload")),
                    "missing-labels" | "unconfirmed-labels" => {
                        fields.remove("labelIds");
                        if line.contains("format=minimal") {
                            fields.remove("payload");
                            // The label set disagrees, or its confirmation is
                            // itself cut short.
                            if scenario == "missing-labels" {
                                fields.insert("labelIds".into(), json!(["INBOX"]));
                            } else {
                                fields.remove("historyId");
                            }
                        }
                    }
                    "empty-payload" => response["payload"] = json!({}),
                    "wrong-id" => response["id"] = json!("B"),
                    "missing-payload" => { response.as_object_mut().unwrap().remove("payload"); },
                    "bad-labels" => response["labelIds"] = json!([1]),
                    "blank-label" => response["labelIds"] = json!([""]),
                    "missing-headers" => { response["payload"].as_object_mut().unwrap().remove("headers"); },
                    "empty-headers" => response["payload"]["headers"] = json!([]),
                    "bad-headers" => response["payload"]["headers"] = json!([{"name":"From"}]),
                    "bad-header-name" => response["payload"]["headers"][0]["name"] = json!(""),
                    "bad-date" => response["internalDate"] = json!("invalid"),
                    "missing-date" => { response.as_object_mut().unwrap().remove("internalDate"); },
                    "missing-mime" => { response["payload"].as_object_mut().unwrap().remove("mimeType"); },
                    "missing-body" => { response["payload"].as_object_mut().unwrap().remove("body"); },
                    "bad-data" => response["payload"]["body"]["data"] = json!("!invalid!"),
                    "missing-data" => response["payload"]["body"] = json!({"size":7}),
                    "negative-size" => response["payload"]["body"]["size"] = json!(-1),
                    "empty-parts" => { response["payload"]["mimeType"] = json!("multipart/mixed"); response["payload"]["parts"] = json!([]); },
                    "invalid-part" => { response["payload"]["mimeType"] = json!("multipart/mixed"); response["payload"]["parts"] = json!([{}]); },
                    _ => {
                        // Crypto always requests source. Full metadata and MIME
                        // structure are usable, but the source must be checked too.
                        response["payload"]["mimeType"] = json!("application/pkcs7-mime");
                        if line.contains("format=raw") {
                            let raw = match scenario {
                                "bad-raw" => "!invalid!".to_string(),
                                "garbage-raw" => base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b"not a message"),
                                "truncated-raw" => base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b"Content-Type: multipart/mixed; boundary=lost\r\n\r\ntruncated"),
                                _ => base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b"From: sender@example.test\r\nSubject: Fixture\r\n\r\nbody"),
                            };
                            return ("200 OK", json!({"id": if scenario == "wrong-raw-id" {"B"} else {"A"}, "raw":raw}).to_string());
                        }
                    }
                }
            }
            ("200 OK", response.to_string())
        }).await;
        std::env::set_var("SATCHEL_GMAIL_API_BASE", &base);
        let pool = pool().await;
        assert_eq!(ingest(&pool, message(&["INBOX"])).await.ingested, 1);
        let thread = annotate(&pool).await;
        let before: (String, String, String, Option<String>, Option<String>) =
            sqlx::query_as("SELECT subject,from_address,date,body_text,body_html FROM messages")
                .fetch_one(&pool)
                .await
                .unwrap();
        let queue = SyncQueue::new();
        let mut state = ImapSyncState::default();
        let provider = crate::providers::gmail::GmailProvider::from_account_config(&config());
        let result = reconcile_test_snapshot(
            &pool, &provider, "account", "inbox", "INBOX", &mut state, &queue,
        )
        .await;
        let progress =
            result.unwrap_or_else(|error| panic!("{scenario} errored the folder: {error}"));
        let Some(SnapshotDeferral::Failed(error)) = progress.deferred.get("A") else {
            panic!("{scenario} was acknowledged: {progress:?}");
        };
        // A disagreement between the two lookups is not a missing label list.
        let cause = match scenario {
            "missing-labels" => Some("omitted labels that its label lookup returned"),
            "unconfirmed-labels" => Some("label lookup returned an incomplete message"),
            _ => None,
        };
        if let Some(cause) = cause {
            assert!(error.to_string().contains(cause), "{scenario}: {error}");
        }
        assert_organization(&pool, &thread, &["INBOX"]).await;
        let after: (String, String, String, Option<String>, Option<String>) =
            sqlx::query_as("SELECT subject,from_address,date,body_text,body_html FROM messages")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(after, before, "{scenario}");
        let raw: String = sqlx::query_scalar("SELECT sync_state FROM accounts")
            .fetch_one(&pool)
            .await
            .unwrap();
        let mut restored: ImapSyncState = serde_json::from_str(&raw).unwrap();
        assert!(!restored.pending_snapshots.contains("INBOX"));
        assert!(restored.snapshot_resolutions["INBOX"].contains("A"));
        fail.store(false, Ordering::SeqCst);
        resolve_test_owed(&pool, &provider, "inbox", "INBOX", &mut restored, &queue)
            .await
            .unwrap();
        assert_organization(&pool, &thread, &["ARCHIVE"]).await;
        task.abort();
    }
    std::env::remove_var("SATCHEL_GMAIL_API_BASE");
}

#[tokio::test]
async fn gmail_complete_empty_multipart_and_crypto_messages_remain_usable() {
    use base64::Engine;
    let _guard = PROVIDER_HTTP_FIXTURES.lock().await;
    for scenario in [
        "empty-size",
        "empty-data",
        "no-subject",
        "omitted-labels",
        "multipart",
        "signed",
    ] {
        let (base, requests, task) = server(move |line, _| {
            let mut response = full_response();
            match scenario {
                "empty-size" => response["payload"]["body"] = json!({"size":0}),
                "empty-data" => response["payload"]["body"] = json!({"size":0,"data":""}),
                "no-subject" => { response["payload"]["headers"].as_array_mut().unwrap().remove(1); },
                "omitted-labels" => { response.as_object_mut().unwrap().remove("labelIds"); },
                _ => {
                    response["payload"]["mimeType"] = json!(if scenario == "signed" {"multipart/signed"} else {"multipart/alternative"});
                    response["payload"]["body"] = json!({"size":0});
                    response["payload"]["parts"] = json!([
                        {"mimeType":"text/plain","body":{"data":"Rml4dHVyZQ==","size":7}},
                        {"mimeType":"text/html","body":{"data":base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b"<p>Fixture</p>"),"size":14}}
                    ]);
                    if scenario == "signed" {
                        response["payload"]["headers"].as_array_mut().unwrap().push(json!({
                            "name":"Content-Type", "value":"multipart/signed; protocol=application/x-pkcs7-signature; boundary=smime"
                        }));
                        response["payload"]["parts"][1] = json!({"mimeType":"application/x-pkcs7-signature", "body":{"data":"AQID","size":3}});
                        if line.contains("format=raw") {
                            let raw = b"From: sender@example.test\r\nContent-Type: multipart/signed; protocol=\"application/x-pkcs7-signature\"; boundary=smime\r\n\r\n--smime\r\nContent-Type: text/plain\r\n\r\nFixture\r\n--smime\r\nContent-Type: application/x-pkcs7-signature\r\n\r\nAQID\r\n--smime--\r\n";
                            return ("200 OK", json!({"id":"A","raw":base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw)}).to_string());
                        }
                    }
                }
            }
            ("200 OK", response.to_string())
        }).await;
        std::env::set_var("SATCHEL_GMAIL_API_BASE", &base);
        let provider = crate::providers::gmail::GmailProvider::from_account_config(&config());
        let message = provider
            .resolve_snapshot_message("A")
            .await
            .unwrap_or_else(|error| panic!("{scenario}: {error:?}"))
            .unwrap();
        assert!(message.mailbox_ids.is_empty(), "{scenario}");
        // Gmail's JSON drops an empty label list. Only its absence needs the
        // label-only lookup to tell an unlabeled message from a cut-short one.
        assert_eq!(
            requests
                .lock()
                .unwrap()
                .iter()
                .any(|r| r.contains("format=minimal")),
            scenario == "omitted-labels",
            "{scenario}"
        );
        assert_eq!(message.from_address, "sender@example.test");
        assert_eq!(message.date_rfc3339, "2026-01-01T00:00:00+00:00");
        if scenario.starts_with("empty-") {
            assert!(message.body_text.as_deref().unwrap_or_default().is_empty());
        } else {
            assert!(message.body_text.as_deref().unwrap().contains("Fixture"));
        }
        if scenario == "multipart" {
            assert_eq!(message.body_html.as_deref(), Some("<p>Fixture</p>"));
        }
        if scenario == "signed" {
            assert!(message.smime_signed.is_some());
            assert!(requests
                .lock()
                .unwrap()
                .iter()
                .any(|r| r.contains("format=raw")));
        }
        let pool = pool().await;
        assert_eq!(ingest(&pool, message).await.ingested, 1, "{scenario}");
        assert_eq!(
            sqlx::query_scalar::<_, String>(
                "SELECT f.remote_id FROM messages m JOIN folders f ON f.id=m.folder_id"
            )
            .fetch_one(&pool)
            .await
            .unwrap(),
            "ARCHIVE"
        );
        task.abort();
    }
    std::env::remove_var("SATCHEL_GMAIL_API_BASE");
}

#[tokio::test]
async fn gmail_snapshot_commit_before_progress_failure_is_safe_to_replay() {
    let _guard = PROVIDER_HTTP_FIXTURES.lock().await;
    let (base, requests, task) = server(|line, _| {
        if line.contains("/messages?") {
            ("200 OK", json!({"messages":[]}).to_string())
        } else {
            ("200 OK", full_response().to_string())
        }
    })
    .await;
    std::env::set_var("SATCHEL_GMAIL_API_BASE", &base);
    let pool = pool().await;
    ingest(&pool, message(&["INBOX"])).await;
    let thread = annotate(&pool).await;
    sqlx::raw_sql("CREATE TRIGGER fail_progress BEFORE UPDATE OF sync_state ON accounts
        WHEN EXISTS (SELECT 1 FROM json_each(OLD.sync_state, '$.snapshot_resolutions.INBOX') WHERE value='A')
        AND NOT EXISTS (SELECT 1 FROM json_each(NEW.sync_state, '$.snapshot_resolutions.INBOX') WHERE value='A')
        BEGIN SELECT RAISE(ABORT, 'fixture progress interruption'); END;")
        .execute(&pool).await.unwrap();
    let queue = SyncQueue::new();
    let mut state = ImapSyncState::default();
    let provider = crate::providers::gmail::GmailProvider::from_account_config(&config());
    let result = reconcile_test_snapshot(
        &pool, &provider, "account", "inbox", "INBOX", &mut state, &queue,
    )
    .await;
    assert!(result
        .unwrap_err()
        .contains("fixture progress interruption"));
    assert_organization(&pool, &thread, &["ARCHIVE"]).await;
    let raw: String = sqlx::query_scalar("SELECT sync_state FROM accounts")
        .fetch_one(&pool)
        .await
        .unwrap();
    let mut restored: ImapSyncState = serde_json::from_str(&raw).unwrap();
    assert!(restored.snapshot_resolutions["INBOX"].contains("A"));
    assert!(restored.pending_snapshots.contains("INBOX"));
    assert!(!restored.folder_snapshot_at.contains_key("INBOX"));
    sqlx::raw_sql("DROP TRIGGER fail_progress")
        .execute(&pool)
        .await
        .unwrap();
    let provider = crate::providers::gmail::GmailProvider::from_account_config(&config());
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
    assert_organization(&pool, &thread, &["ARCHIVE"]).await;
    assert!(!restored.pending_snapshots.contains("INBOX"));
    assert!(!restored.snapshot_resolutions.contains_key("INBOX"));
    assert_eq!(
        requests
            .lock()
            .unwrap()
            .iter()
            .filter(|line| line.contains("/messages/A?"))
            .count(),
        2
    );
    std::env::remove_var("SATCHEL_GMAIL_API_BASE");
    task.abort();
}

fn labelled_response(id: &str, labels: &[&str]) -> String {
    json!({"id":id,"threadId":id,"historyId":"300","labelIds":labels,
        "internalDate":"1767225600000","payload":{"mimeType":"text/plain",
        "headers":[{"name":"From","value":"sender@example.test"}],
        "body":{"data":"Rml4dHVyZQ=="}}})
    .to_string()
}

fn delivery(id: &str) -> MessageEnvelope {
    let mut message = message(&["INBOX"]);
    message.remote_id = id.into();
    message.remote_thread_id = Some(id.into());
    message.message_id_header = Some(format!("<{id}@example.test>"));
    message
}

async fn locations(pool: &SqlitePool) -> Vec<(String, String)> {
    sqlx::query_as(
        "SELECT m.remote_id, f.remote_id FROM messages m JOIN folders f ON f.id=m.folder_id
         ORDER BY 1, 2",
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

async fn persisted_state(pool: &SqlitePool) -> ImapSyncState {
    let raw: String = sqlx::query_scalar("SELECT sync_state FROM accounts")
        .fetch_one(pool)
        .await
        .unwrap();
    serde_json::from_str(&raw).unwrap()
}

fn fetched_ids(requests: &std::sync::Mutex<Vec<String>>) -> Vec<String> {
    requests
        .lock()
        .unwrap()
        .iter()
        .filter_map(|line| line.split("/messages/").nth(1))
        .map(|path| path.split('?').next().unwrap().to_string())
        .collect()
}

fn ids(values: &[&str]) -> HashSet<String> {
    values.iter().map(|value| value.to_string()).collect()
}

/// A failed mutation, an exhausted target quarantine and a failed lookup each
/// hold back one ID. The rest of the folder must still converge, and IDs held
/// by another owner must not re-list the folder on every poll.
#[tokio::test]
async fn gmail_blocked_snapshot_ids_stay_owed_without_starving_the_folder() {
    let _guard = PROVIDER_HTTP_FIXTURES.lock().await;
    let fail_lookup = Arc::new(AtomicBool::new(true));
    let failing = fail_lookup.clone();
    let (base, requests, task) = server(move |line, _| {
        if line.contains("/messages?") {
            return ("200 OK", json!({"messages":[]}).to_string());
        }
        let id = line
            .split("/messages/")
            .nth(1)
            .unwrap()
            .split('?')
            .next()
            .unwrap();
        if id == "C" && failing.load(Ordering::SeqCst) {
            return ("503 Service Unavailable", "{}".into());
        }
        ("200 OK", labelled_response(id, &["Label_one"]))
    })
    .await;
    std::env::set_var("SATCHEL_GMAIL_API_BASE", &base);
    let pool = pool().await;
    for id in ["A", "B", "C", "D", "E"] {
        assert_eq!(ingest(&pool, delivery(id)).await.ingested, 1);
    }
    let queue = SyncQueue::new();
    queue
        .enqueue(
            &pool,
            QueuedAction::new(
                "account".into(),
                ACTION_REMOTE_MUTATION,
                json!({"kind":"move","refs":[{"remote_id":"B","source_folder":"INBOX"}],
                    "target_remote_folder_id":"Label_one"}),
            ),
        )
        .await
        .unwrap();
    sqlx::query("UPDATE sync_queue SET status='failed'")
        .execute(&pool)
        .await
        .unwrap();
    let (label_one, _) = ensure_gmail_label_folder(&pool, "account", "Label_one")
        .await
        .unwrap();
    let quarantine = SyncQuarantineRepository::new(&pool);
    for _ in 0..satchel_core::db::sync_quarantine::MAX_INGEST_ATTEMPTS {
        quarantine
            .record_failure("account", &label_one, "E", "fixture")
            .await
            .unwrap();
    }
    let provider = crate::providers::gmail::GmailProvider::from_account_config(&config());
    let mut state = ImapSyncState::default();
    assert!(state.request_snapshot("INBOX", true, true));

    // C's lookup fails; B and E are held by the queue and the quarantine.
    let first = reconcile_test_snapshot(
        &pool, &provider, "account", "inbox", "INBOX", &mut state, &queue,
    )
    .await
    .unwrap();
    assert_eq!(first.deferred.keys().collect::<Vec<_>>(), ["B", "C", "E"]);
    assert_eq!(first.deferred["B"], SnapshotDeferral::PendingMutation);
    assert!(matches!(first.deferred["C"], SnapshotDeferral::Failed(_)));
    assert_eq!(first.deferred["E"], SnapshotDeferral::Quarantined);
    assert_eq!(
        locations(&pool).await,
        [
            ("A".into(), "Label_one".into()),
            ("B".into(), "INBOX".into()),
            ("C".into(), "INBOX".into()),
            ("D".into(), "Label_one".into()),
            ("E".into(), "INBOX".into()),
        ]
    );
    let mut persisted = persisted_state(&pool).await;
    assert_eq!(
        persisted.snapshot_resolutions["INBOX"],
        ids(&["B", "C", "E"])
    );
    assert!(
        !persisted.request_snapshot("INBOX", true, false),
        "owed IDs, failed ones included, never list the folder again"
    );
    assert!(persisted.folder_snapshot_at.contains_key("INBOX"));

    // The lookup recovers after a restart. Only C is fetched again: the queue
    // and the quarantine still hold B and E, which costs no request.
    fail_lookup.store(false, Ordering::SeqCst);
    requests.lock().unwrap().clear();
    let mut state = persisted;
    let second = resolve_test_owed(&pool, &provider, "inbox", "INBOX", &mut state, &queue)
        .await
        .unwrap();
    assert_eq!(second.deferred.keys().collect::<Vec<_>>(), ["B", "E"]);
    assert_eq!(fetched_ids(&requests), ["C"]);
    assert_eq!(listings(&requests), 0);
    assert_eq!(
        locations(&pool).await,
        [
            ("A".into(), "Label_one".into()),
            ("B".into(), "INBOX".into()),
            ("C".into(), "Label_one".into()),
            ("D".into(), "Label_one".into()),
            ("E".into(), "INBOX".into()),
        ]
    );
    let mut persisted = persisted_state(&pool).await;
    assert_eq!(persisted.snapshot_resolutions["INBOX"], ids(&["B", "E"]));
    assert!(!persisted.request_snapshot("INBOX", true, false));

    // Once their owners release them, the next ordinary poll resolves both.
    sqlx::query("DELETE FROM sync_queue")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM sync_quarantine")
        .execute(&pool)
        .await
        .unwrap();
    requests.lock().unwrap().clear();
    resolve_test_owed(&pool, &provider, "inbox", "INBOX", &mut persisted, &queue)
        .await
        .unwrap();
    assert_eq!(fetched_ids(&requests), ["B", "E"]);
    assert_eq!(listings(&requests), 0);
    assert!(locations(&pool)
        .await
        .iter()
        .all(|(_, folder)| folder == "Label_one"));
    let done = persisted_state(&pool).await;
    assert!(!done.pending_snapshots.contains("INBOX"));
    assert!(!done.snapshot_resolutions.contains_key("INBOX"));
    std::env::remove_var("SATCHEL_GMAIL_API_BASE");
    task.abort();
}

/// Gmail snapshots DRAFT on every poll. An exhausted quarantine cannot be
/// resolved by a lookup, so other folders' snapshots must not fetch it again
/// and again. Retry work below the limit is still picked up.
#[tokio::test]
async fn gmail_exhausted_quarantine_is_not_refetched_by_every_folder_snapshot() {
    let _guard = PROVIDER_HTTP_FIXTURES.lock().await;
    let (base, requests, task) = server(|line, _| {
        if line.contains("/messages?") {
            return ("200 OK", json!({"messages":[]}).to_string());
        }
        let id = line
            .split("/messages/")
            .nth(1)
            .unwrap()
            .split('?')
            .next()
            .unwrap();
        ("200 OK", labelled_response(id, &["Label_one"]))
    })
    .await;
    std::env::set_var("SATCHEL_GMAIL_API_BASE", &base);
    let pool = pool().await;
    for id in ["A", "B"] {
        assert_eq!(ingest(&pool, delivery(id)).await.ingested, 1);
    }
    let (label_one, _) = ensure_gmail_label_folder(&pool, "account", "Label_one")
        .await
        .unwrap();
    let (drafts, _) = ensure_gmail_label_folder(&pool, "account", "DRAFT")
        .await
        .unwrap();
    let quarantine = SyncQuarantineRepository::new(&pool);
    for _ in 0..satchel_core::db::sync_quarantine::MAX_INGEST_ATTEMPTS {
        quarantine
            .record_failure("account", &label_one, "A", "fixture")
            .await
            .unwrap();
    }
    quarantine
        .record_failure("account", &label_one, "B", "fixture")
        .await
        .unwrap();
    let provider = crate::providers::gmail::GmailProvider::from_account_config(&config());
    let queue = SyncQueue::new();
    let mut state = ImapSyncState::default();
    let progress = reconcile_test_snapshot(
        &pool, &provider, "account", &drafts, "DRAFT", &mut state, &queue,
    )
    .await
    .unwrap();
    assert!(progress.deferred.is_empty(), "{progress:?}");
    assert_eq!(fetched_ids(&requests), ["B"]);
    assert_eq!(
        locations(&pool).await,
        [
            ("A".into(), "INBOX".into()),
            ("B".into(), "Label_one".into()),
        ]
    );
    let persisted = persisted_state(&pool).await;
    assert!(!persisted.pending_snapshots.contains("DRAFT"));
    assert!(!persisted.snapshot_resolutions.contains_key("DRAFT"));
    std::env::remove_var("SATCHEL_GMAIL_API_BASE");
    task.abort();
}

const GMAIL_CONFIG: &str =
    r#"{"oauth":{"access_token":"fixture-only","expires_at":"2099-01-01T00:00:00Z"}}"#;

/// The fixture Gmail account as `sync_mailboxes` loads it. INBOX's history
/// cursor has already expired.
async fn gmail_account(pool: &SqlitePool) {
    satchel_core::utils::crypto::seed_master_key_for_tests();
    let config =
        crate::commands::accounts::encrypt_account_config("account", GMAIL_CONFIG).unwrap();
    let mut state = ImapSyncState::default();
    state.cursors.insert(
        "INBOX".into(),
        json!({"history_id":"100","format_version":3,"latest_ts":1790000000}).to_string(),
    );
    sqlx::query("UPDATE accounts SET config = ?, sync_state = ? WHERE id = 'account'")
        .bind(config)
        .bind(serde_json::to_string(&state).unwrap())
        .execute(pool)
        .await
        .unwrap();
}

/// One production sync run of the fixture account.
async fn poll(pool: &SqlitePool, folders: FolderSelection) -> MailboxSyncSummary {
    sync_mailboxes(
        pool,
        MailboxSyncRequest {
            events: None,
            app: None,
            accounts: AccountSelection::One("account"),
            folders,
        },
    )
    .await
    .unwrap()
}

/// Gmail endpoints of an ordinary poll with nothing new. History before 200
/// has expired.
fn quiet_gmail(line: &str) -> Option<(&'static str, String)> {
    if line.contains("startHistoryId=100") {
        return Some(("404 Not Found", "{}".into()));
    }
    let body = if line.contains("/history") || line.contains("/profile") {
        json!({"historyId":"200","history":[]})
    } else if line.contains("/messages?") {
        json!({"messages":[]})
    } else if line.contains("/drafts") {
        json!({"drafts":[]})
    } else if line.contains("/labels") {
        json!({"labels":[
            {"id":"INBOX","name":"INBOX","type":"system"},
            {"id":"DRAFT","name":"DRAFT","type":"system"},
            {"id":"Label_one","name":"Label one","type":"user"},
            {"id":"Label_aside","name":"Aside","type":"user"}
        ]})
    } else {
        return None;
    };
    Some(("200 OK", body.to_string()))
}

fn requested_id(line: &str) -> &str {
    line.split("/messages/")
        .nth(1)
        .unwrap()
        .split('?')
        .next()
        .unwrap()
}

/// Folder listings, as opposed to point lookups, history and the time-based
/// catch-up.
fn listings(requests: &std::sync::Mutex<Vec<String>>) -> usize {
    requests
        .lock()
        .unwrap()
        .iter()
        .filter(|line| line.contains("/messages?") && !line.contains("q=after"))
        .count()
}

fn failure_keys(summary: &MailboxSyncSummary) -> Vec<String> {
    summary
        .failures
        .iter()
        .map(|failure| {
            serde_json::from_str::<serde_json::Value>(failure).unwrap()["key"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect()
}

/// A snapshot after expired history finds X absent while a queued flag change
/// still pins it. Ordinary polls look X up again as soon as the queue lets go,
/// without listing INBOX again.
#[tokio::test]
async fn gmail_held_snapshot_id_is_retried_by_an_ordinary_poll_once_released() {
    let _guard = PROVIDER_HTTP_FIXTURES.lock().await;
    let (base, requests, task) = server(|line, _| {
        quiet_gmail(line).unwrap_or_else(|| {
            (
                "200 OK",
                labelled_response(requested_id(line), &["Label_one"]),
            )
        })
    })
    .await;
    std::env::set_var("SATCHEL_GMAIL_API_BASE", &base);
    let pool = pool().await;
    gmail_account(&pool).await;
    assert_eq!(ingest(&pool, delivery("X")).await.ingested, 1);
    SyncQueue::new()
        .enqueue(
            &pool,
            QueuedAction::new(
                "account".into(),
                ACTION_REMOTE_MUTATION,
                json!({"kind":"flags","refs":[{"remote_id":"X","source_folder":"INBOX"}],
                    "read":true,"starred":null}),
            ),
        )
        .await
        .unwrap();

    let expired = poll(&pool, FolderSelection::Inbox).await;
    assert!(expired.failures.is_empty(), "{:?}", expired.failures);
    assert_eq!(locations(&pool).await, [("X".into(), "INBOX".into())]);
    let state = persisted_state(&pool).await;
    assert_eq!(state.snapshot_resolutions["INBOX"], ids(&["X"]));
    assert!(state.pending_snapshots.is_empty());

    // Still pinned: an ordinary poll asks the server nothing about X.
    requests.lock().unwrap().clear();
    poll(&pool, FolderSelection::Inbox).await;
    assert!(fetched_ids(&requests).is_empty());
    assert_eq!(locations(&pool).await, [("X".into(), "INBOX".into())]);

    sqlx::query("DELETE FROM sync_queue")
        .execute(&pool)
        .await
        .unwrap();
    requests.lock().unwrap().clear();
    let released = poll(&pool, FolderSelection::Inbox).await;
    assert!(released.failures.is_empty(), "{:?}", released.failures);
    assert_eq!(fetched_ids(&requests), ["X"]);
    assert_eq!(listings(&requests), 0, "an owed ID never relists INBOX");
    assert_eq!(locations(&pool).await, [("X".into(), "Label_one".into())]);
    assert!(!persisted_state(&pool)
        .await
        .snapshot_resolutions
        .contains_key("INBOX"));
    std::env::remove_var("SATCHEL_GMAIL_API_BASE");
    task.abort();
}

/// A lookup that keeps failing costs one request per poll, never a listing.
/// A permanent error stops the retries at once, a transient one after a few
/// polls. The hold is reported once, and the account still counts as synced.
/// A day later the ID gets exactly one more lookup.
#[tokio::test]
async fn gmail_failing_owed_lookups_are_capped_and_reported_once() {
    let _guard = PROVIDER_HTTP_FIXTURES.lock().await;
    for (status, attempts) in [("403 Forbidden", 1), ("503 Service Unavailable", 3)] {
        let recovered = Arc::new(AtomicBool::new(false));
        let recovery = recovered.clone();
        let (base, requests, task) = server(move |line, _| {
            quiet_gmail(line).unwrap_or_else(|| {
                if recovery.load(Ordering::SeqCst) {
                    (
                        "200 OK",
                        labelled_response(requested_id(line), &["Label_one"]),
                    )
                } else {
                    (status, json!({"error":{"code":0}}).to_string())
                }
            })
        })
        .await;
        std::env::set_var("SATCHEL_GMAIL_API_BASE", &base);
        let pool = pool().await;
        gmail_account(&pool).await;
        assert_eq!(ingest(&pool, delivery("X")).await.ingested, 1);
        let mut reported = Vec::new();
        for run in 1..=attempts + 2 {
            requests.lock().unwrap().clear();
            let summary = poll(&pool, FolderSelection::Inbox).await;
            let lookups = fetched_ids(&requests).len();
            let listed = listings(&requests);
            // Run 1 lists INBOX for the snapshot. Run 2 lists it once more for
            // the backfill that expired history restarts; after that, nothing.
            assert_eq!(
                (lookups, listed),
                (usize::from(run <= attempts), usize::from(run <= 2)),
                "{status}: run {run}"
            );
            for key in failure_keys(&summary) {
                reported.push((run, key));
            }
            assert_eq!(
                sqlx::query_scalar::<_, String>("SELECT sync_status FROM accounts")
                    .fetch_one(&pool)
                    .await
                    .unwrap(),
                "idle",
                "{status}: run {run}"
            );
        }
        assert_eq!(
            reported,
            [(attempts, "backend.errors.snapshotMessagesHeld".to_string())],
            "{status}"
        );
        assert_eq!(locations(&pool).await, [("X".into(), "INBOX".into())]);
        assert_eq!(
            persisted_state(&pool).await.snapshot_resolutions["INBOX"],
            ids(&["X"])
        );

        // A day later: one lookup. A repeated failure holds X again and is
        // reported once more; a success moves it and forgets the failures.
        for recovers in [false, true] {
            recovered.store(recovers, Ordering::SeqCst);
            let mut state = persisted_state(&pool).await;
            for failure in state
                .snapshot_failures
                .get_mut("INBOX")
                .unwrap()
                .values_mut()
            {
                failure.failed_at -= chrono::Duration::hours(HELD_RETRY_AFTER_HOURS);
            }
            persist_sync_cursors(&pool, "account", &state, &SyncQueue::new(), None)
                .await
                .unwrap();
            requests.lock().unwrap().clear();
            let summary = poll(&pool, FolderSelection::Inbox).await;
            assert_eq!(fetched_ids(&requests), ["X"], "{status}");
            assert_eq!(listings(&requests), 0, "{status}");
            assert_eq!(
                failure_keys(&summary),
                if recovers {
                    vec![]
                } else {
                    vec!["backend.errors.snapshotMessagesHeld".to_string()]
                },
                "{status}"
            );
            requests.lock().unwrap().clear();
            poll(&pool, FolderSelection::Inbox).await;
            assert!(fetched_ids(&requests).is_empty(), "{status}");
        }
        assert_eq!(locations(&pool).await, [("X".into(), "Label_one".into())]);
        let done = persisted_state(&pool).await;
        assert!(done.snapshot_resolutions.is_empty(), "{status}");
        assert!(done.snapshot_failures.is_empty(), "{status}");
        task.abort();
    }
    std::env::remove_var("SATCHEL_GMAIL_API_BASE");
}

/// A relabel whose target cannot be stored is skipped once its attempts are
/// used up. That is reported once, and a day later it gets another attempt,
/// so the stale copy in its old label can still heal.
#[tokio::test]
async fn gmail_exhausted_relabel_is_reported_once_and_retried_after_a_day() {
    let _guard = PROVIDER_HTTP_FIXTURES.lock().await;
    let (base, requests, task) = server(|line, _| {
        if line.contains("startHistoryId=200") && line.contains("labelId=INBOX") {
            let history = json!({"historyId":"210","history":[{"id":"210",
                "labelsRemoved":[{"message":{"id":"A"},"labelIds":["INBOX"]}]}]});
            return ("200 OK", history.to_string());
        }
        if line.contains("startHistoryId=210") {
            return (
                "200 OK",
                json!({"historyId":"210","history":[]}).to_string(),
            );
        }
        quiet_gmail(line).unwrap_or_else(|| {
            (
                "200 OK",
                labelled_response(requested_id(line), &["Label_one"]),
            )
        })
    })
    .await;
    std::env::set_var("SATCHEL_GMAIL_API_BASE", &base);
    let pool = pool().await;
    gmail_account(&pool).await;
    let mut state = persisted_state(&pool).await;
    state.cursors.insert(
        "INBOX".into(),
        json!({"history_id":"200","format_version":3,"latest_ts":1790000000}).to_string(),
    );
    persist_sync_cursors(&pool, "account", &state, &SyncQueue::new(), None)
        .await
        .unwrap();
    assert_eq!(ingest(&pool, delivery("A")).await.ingested, 1);
    ensure_gmail_label_folder(&pool, "account", "Label_one")
        .await
        .unwrap();
    sqlx::raw_sql(
        "CREATE TRIGGER fail_label_one BEFORE INSERT ON messages
         WHEN NEW.folder_id IN (SELECT id FROM folders WHERE remote_id='Label_one')
         BEGIN SELECT RAISE(ABORT, 'fixture target failure'); END;",
    )
    .execute(&pool)
    .await
    .unwrap();

    let max = satchel_core::db::sync_quarantine::MAX_INGEST_ATTEMPTS as usize;
    let mut skipped_reports = Vec::new();
    for run in 1..=max + 2 {
        let summary = poll(&pool, FolderSelection::Inbox).await;
        for key in failure_keys(&summary) {
            if key == "backend.errors.messagesSkipped" {
                skipped_reports.push(run);
            }
        }
    }
    assert_eq!(skipped_reports, [max], "reported once, when it ran out");
    assert_eq!(locations(&pool).await, [("A".into(), "INBOX".into())]);

    // A day later the target works again.
    sqlx::raw_sql("DROP TRIGGER fail_label_one")
        .execute(&pool)
        .await
        .unwrap();
    let yesterday = (chrono::Utc::now() - chrono::Duration::hours(25)).to_rfc3339();
    sqlx::query("UPDATE sync_quarantine SET last_failed_at = ?")
        .bind(yesterday)
        .execute(&pool)
        .await
        .unwrap();
    requests.lock().unwrap().clear();
    let healed = poll(&pool, FolderSelection::All).await;
    assert!(healed.failures.is_empty(), "{:?}", healed.failures);
    assert_eq!(locations(&pool).await, [("A".into(), "Label_one".into())]);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_quarantine")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    std::env::remove_var("SATCHEL_GMAIL_API_BASE");
    task.abort();
}
