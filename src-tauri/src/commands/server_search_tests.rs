use sqlx::sqlite::SqlitePoolOptions;
use sqlx::SqlitePool;

async fn pool() -> SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory db");
    sqlx::migrate!("../crates/satchel-core/src/db/migrations")
        .run(&pool)
        .await
        .expect("migrations");
    pool
}

async fn seed_account(pool: &SqlitePool) {
    sqlx::query(
        "INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, sync_state, color, is_active, created_at, updated_at)
         VALUES ('acc-1', 'Me', 'me@example.com', 'imap', '{}', '', '{}', '#000', 1, '2026-01-01', '2026-01-01')",
    )
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO folders (id, account_id, name, full_path, folder_type, remote_id, message_count, unread_count, sync_enabled, sort_order, created_at, updated_at)
         VALUES ('fld-inbox', 'acc-1', 'Inbox', 'INBOX', 'inbox', 'INBOX', 0, 0, 1, 0, '2026-01-01', '2026-01-01')",
    )
    .execute(pool)
    .await
    .unwrap();
}

struct SeedMessage<'a> {
    id: &'a str,
    subject: &'a str,
    body_text: &'a str,
    from_name: &'a str,
    from_address: &'a str,
    to_addresses: &'a str,
    date: &'a str,
    is_read: bool,
    /// Defaults to a fresh thread per message (`th-<id>`).
    thread_id: Option<&'a str>,
}

impl Default for SeedMessage<'_> {
    fn default() -> Self {
        Self {
            id: "m-1",
            subject: "Subject",
            body_text: "Body",
            from_name: "Sender",
            from_address: "sender@example.com",
            to_addresses: r#"["me@example.com"]"#,
            date: "2026-01-01T00:00:00Z",
            is_read: true,
            thread_id: None,
        }
    }
}

async fn seed_message(pool: &SqlitePool, message: SeedMessage<'_>) {
    let thread_id = message
        .thread_id
        .map(str::to_string)
        .unwrap_or_else(|| format!("th-{}", message.id));
    sqlx::query(
        "INSERT OR IGNORE INTO threads (id, account_id, subject, last_message_date, message_count, unread_count, has_attachments, participants, snippet, workflow_state, workflow_changed_at, created_at, updated_at)
         VALUES (?, 'acc-1', ?, ?, 1, 0, 0, '[]', '', 'inbox', '2026-01-01', '2026-01-01', '2026-01-01')",
    )
    .bind(&thread_id)
    .bind(message.subject)
    .bind(message.date)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO messages (id, account_id, folder_id, thread_id, message_id_header, from_address, from_name, to_addresses, subject, date, body_text, snippet, is_read, is_starred, is_draft, is_sent, has_attachments, created_at, updated_at)
         VALUES (?, 'acc-1', 'fld-inbox', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 0, 0, 0, 0, '2026-01-01', '2026-01-01')",
    )
    .bind(message.id)
    .bind(&thread_id)
    .bind(format!("<{}@example.com>", message.id))
    .bind(message.from_address)
    .bind(message.from_name)
    .bind(message.to_addresses)
    .bind(message.subject)
    .bind(message.date)
    .bind(message.body_text)
    .bind(message.body_text)
    .bind(message.is_read)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE threads
         SET last_message_date = (
                 SELECT MAX(date) FROM messages WHERE thread_id = ?
             ),
             message_count = (
                 SELECT COUNT(*) FROM messages WHERE thread_id = ?
             ),
             unread_count = (
                 SELECT COUNT(*) FROM messages WHERE thread_id = ? AND is_read = 0
             )
         WHERE id = ?",
    )
    .bind(&thread_id)
    .bind(&thread_id)
    .bind(&thread_id)
    .bind(&thread_id)
    .execute(pool)
    .await
    .unwrap();
}
#[tokio::test]
async fn server_hits_matching_the_local_query_are_dropped() {
    use crate::commands::server_search::{drop_local_query_matches, ServerSearchHit};

    let pool = pool().await;
    seed_account(&pool).await;
    seed_message(&pool, SeedMessage::default()).await;
    sqlx::query("UPDATE messages SET remote_id = 'remote-known' WHERE id = 'm-1'")
        .execute(&pool)
        .await
        .unwrap();

    let hit = |remote_id: &str| ServerSearchHit {
        remote_id: remote_id.into(),
        subject: "Betreff".into(),
        snippet: "Vorschau".into(),
        from: "mike@example.com".into(),
        date: None,
    };

    let remaining = drop_local_query_matches(
        &pool,
        "acc-1",
        "Body",
        vec![hit("remote-known"), hit("remote-new")],
    )
    .await
    .unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].remote_id, "remote-new");

    // A cached message that the local query does not match still belongs in
    // server results, including when provider and local query syntax differ.
    let remaining =
        drop_local_query_matches(&pool, "acc-1", "subject:absent", vec![hit("remote-known")])
            .await
            .unwrap();
    assert_eq!(remaining.len(), 1);

    // Same remote id under a different account must not be filtered.
    let remaining = drop_local_query_matches(&pool, "acc-other", "Body", vec![hit("remote-known")])
        .await
        .unwrap();
    assert_eq!(remaining.len(), 1);
}

#[tokio::test]
async fn server_hit_fetch_short_circuits_when_already_local() {
    use crate::commands::server_search::{fetch_server_search_hit_impl, FetchServerHitInput};

    let pool = pool().await;
    seed_account(&pool).await;
    seed_message(&pool, SeedMessage::default()).await;
    sqlx::query("UPDATE messages SET remote_id = '42', remote_uidvalidity = 7 WHERE id = 'm-1'")
        .execute(&pool)
        .await
        .unwrap();

    sqlx::query("UPDATE folders SET uidvalidity = 7 WHERE id = 'fld-inbox'")
        .execute(&pool)
        .await
        .unwrap();

    // Must return the local ids without any network access (the seeded account
    // has an empty config; a provider call would fail loudly).
    let fetched = fetch_server_search_hit_impl(
        &pool,
        &FetchServerHitInput {
            account_id: "acc-1".into(),
            remote_id: "42".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(fetched.message_id, "m-1");
    assert_eq!(fetched.thread_id, "th-m-1");
}

#[tokio::test]
async fn server_hit_rejects_unverified_or_stale_cached_imap_content() {
    use crate::commands::server_search::{fetch_server_search_hit_impl, FetchServerHitInput};

    let pool = pool().await;
    seed_account(&pool).await;
    seed_message(&pool, SeedMessage::default()).await;
    sqlx::query("UPDATE folders SET uidvalidity = 8 WHERE id = 'fld-inbox'")
        .execute(&pool)
        .await
        .unwrap();
    for generation in [None, Some(7)] {
        sqlx::query(
            "UPDATE messages SET remote_id = '42', remote_uidvalidity = ? WHERE id = 'm-1'",
        )
        .bind(generation)
        .execute(&pool)
        .await
        .unwrap();
        let error = fetch_server_search_hit_impl(
            &pool,
            &FetchServerHitInput {
                account_id: "acc-1".into(),
                remote_id: "42".into(),
            },
        )
        .await
        .unwrap_err();
        let error: serde_json::Value = serde_json::from_str(&error).unwrap();
        assert_eq!(error["params"]["folder"], "INBOX");
        assert_eq!(
            error["key"],
            if generation.is_none() {
                "backend.errors.imapUidValidityMissing"
            } else {
                "backend.errors.imapUidValidityChanged"
            }
        );
    }
}

#[tokio::test]
async fn fetched_messages_land_in_the_matching_local_folder() {
    use crate::commands::server_search::resolve_target_folder;
    use crate::providers::traits::MessageEnvelope;

    let pool = pool().await;
    seed_account(&pool).await;

    let envelope = |mailbox_ids: Vec<&str>| -> MessageEnvelope {
        serde_json::from_value(serde_json::json!({
            "remote_id": "remote-1",
            "remote_thread_id": null,
            "mailbox_ids": mailbox_ids,
            "subject": "Betreff",
            "snippet": "Vorschau",
            "from_address": "mike@example.com",
            "from_name": "Mike",
            "to_addresses": [],
            "cc_addresses": [],
            "date_rfc3339": "2026-07-01T00:00:00Z",
            "body_text": "Text",
            "body_html": null,
        }))
        .expect("envelope")
    };

    // Gmail label matching the local inbox folder (remote_id 'INBOX').
    let (folder_id, remote_id, folder_type) =
        resolve_target_folder(&pool, "acc-1", "gmail", &envelope(vec!["INBOX"]), None)
            .await
            .unwrap();
    assert_eq!(
        (folder_id.as_str(), remote_id.as_str(), folder_type.as_str()),
        ("fld-inbox", "INBOX", "inbox")
    );

    // Unknown labels fall back to Gmail's synthetic archive folder.
    let (_, remote_id, folder_type) =
        resolve_target_folder(&pool, "acc-1", "gmail", &envelope(vec!["Label_42"]), None)
            .await
            .unwrap();
    assert_eq!(
        (remote_id.as_str(), folder_type.as_str()),
        ("ARCHIVE", "archive")
    );

    // Outlook: the Graph parent folder wins when it exists locally …
    let (folder_id, _, _) = resolve_target_folder(
        &pool,
        "acc-1",
        "outlook",
        &envelope(vec![]),
        Some("INBOX".into()),
    )
    .await
    .unwrap();
    assert_eq!(folder_id, "fld-inbox");

    // … and an unknown parent falls back to the account inbox.
    let (folder_id, _, _) = resolve_target_folder(
        &pool,
        "acc-1",
        "outlook",
        &envelope(vec![]),
        Some("unknown-folder".into()),
    )
    .await
    .unwrap();
    assert_eq!(folder_id, "fld-inbox");
}

#[tokio::test]
async fn retained_header_with_evicted_body_stays_available_on_server() {
    use crate::commands::server_search::{drop_local_query_matches, ServerSearchHit};
    use crate::db::messages::{MessageSearchRepository, SearchQuery};
    let pool = pool().await;
    seed_account(&pool).await;
    seed_message(
        &pool,
        SeedMessage {
            body_text: "needle",
            ..Default::default()
        },
    )
    .await;
    sqlx::query("UPDATE messages SET remote_id = 'remote-1' WHERE id = 'm-1'")
        .execute(&pool)
        .await
        .unwrap();
    let query = SearchQuery {
        query: "needle".into(),
        account_id: Some("acc-1".into()),
        ..Default::default()
    };
    assert_eq!(
        MessageSearchRepository::new(&pool)
            .search(&query)
            .await
            .unwrap()
            .hits
            .len(),
        1
    );
    sqlx::query("UPDATE messages SET body_text = NULL, body_html = NULL WHERE id = 'm-1'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(MessageSearchRepository::new(&pool)
        .search(&query)
        .await
        .unwrap()
        .hits
        .is_empty());
    let remaining = drop_local_query_matches(
        &pool,
        "acc-1",
        "needle",
        vec![ServerSearchHit {
            remote_id: "remote-1".into(),
            subject: "Subject".into(),
            snippet: "needle".into(),
            from: "sender@example.com".into(),
            date: None,
        }],
    )
    .await
    .unwrap();
    assert_eq!(
        remaining.len(),
        1,
        "a retained header must not hide a remote body match"
    );
    // Eviction must not produce duplicate results for a header match.
    let ids = MessageSearchRepository::new(&pool)
        .matching_remote_ids("acc-1", "subject:Subject", &["remote-1".into()])
        .await
        .unwrap();
    assert_eq!(ids, vec!["remote-1"]);
}

#[tokio::test]
async fn retained_header_body_errors_preserve_translation_parameters() {
    use crate::commands::error::CommandError;
    use crate::commands::server_search::{fetch_server_search_hit_impl, FetchServerHitInput};

    let pool = pool().await;
    seed_account(&pool).await;
    seed_message(&pool, SeedMessage::default()).await;
    crate::utils::crypto::seed_master_key_for_tests();
    let config = crate::commands::accounts::encrypt_account_config("acc-1", "{}").unwrap();
    sqlx::query("UPDATE accounts SET provider = 'test-unsupported', config = ? WHERE id = 'acc-1'")
        .bind(config)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE messages SET remote_id = 'remote-1', body_text = NULL, body_html = NULL WHERE id = 'm-1'")
        .execute(&pool).await.unwrap();

    let error = fetch_server_search_hit_impl(
        &pool,
        &FetchServerHitInput {
            account_id: "acc-1".into(),
            remote_id: "remote-1".into(),
        },
    )
    .await
    .map_err(CommandError::from)
    .unwrap_err();

    assert_eq!(
        serde_json::to_value(error).unwrap(),
        serde_json::json!({
            "key": "backend.errors.messageBodyProviderUnsupported",
            "params": { "provider": "test-unsupported" },
        })
    );
}

#[tokio::test]
async fn fetching_evicted_body_preserves_identity_and_local_state() {
    use crate::commands::server_search::{fetch_server_search_hit_impl, FetchServerHitInput};
    use crate::db::messages::{MessageSearchRepository, SearchQuery};
    use base64::Engine;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let pool = pool().await;
    seed_account(&pool).await;
    seed_message(&pool, SeedMessage::default()).await;
    crate::utils::crypto::seed_master_key_for_tests();
    let config = crate::commands::accounts::encrypt_account_config(
        "acc-1",
        r#"{"oauth":{"access_token":"fixture-token"}}"#,
    )
    .unwrap();
    sqlx::query("UPDATE accounts SET provider = 'gmail', config = ? WHERE id = 'acc-1'")
        .bind(config)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE messages SET remote_id = 'remote-1', body_text = NULL, body_html = NULL, is_starred = 1, is_read = 0 WHERE id = 'm-1'")
        .execute(&pool).await.unwrap();
    sqlx::query("UPDATE threads SET workflow_state = 'aside', priority = 3 WHERE id = 'th-m-1'")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, sync_state, color, is_active, created_at, updated_at) SELECT 'acc-2', display_name, email, provider, config, credentials_ref, sync_state, color, is_active, created_at, updated_at FROM accounts WHERE id = 'acc-1';
        INSERT INTO folders (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at) VALUES ('fld-other', 'acc-2', 'Inbox', 'INBOX', 'inbox', 'INBOX', '2026-01-01', '2026-01-01');
        INSERT INTO threads (id, account_id, subject, last_message_date, created_at, updated_at) VALUES ('th-other', 'acc-2', 'Other', '2026-01-01', '2026-01-01', '2026-01-01');
        INSERT INTO messages (id, account_id, folder_id, thread_id, remote_id, from_address, to_addresses, subject, body_text, date, created_at, updated_at) VALUES ('m-other', 'acc-2', 'fld-other', 'th-other', 'remote-1', 'other@example.com', '[]', 'Other', 'Other cached body', '2026-01-01', '2026-01-01', '2026-01-01')")
        .execute(&pool).await.unwrap();
    let state_sql = "SELECT json_object('id', m.id, 'account', m.account_id, 'folder', m.folder_id, 'thread', m.thread_id, 'subject', m.subject, 'read', m.is_read, 'starred', m.is_starred, 'state', t.workflow_state, 'priority', t.priority, 'category', m.category, 'trust', m.trust_metadata) FROM messages m JOIN threads t ON t.id = m.thread_id WHERE m.id = 'm-1'";
    let before: String = sqlx::query_scalar(state_sql)
        .fetch_one(&pool)
        .await
        .unwrap();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let html = "<p onclick=bad()>needle</p><script>bad()</script>";
    let response = serde_json::json!({
        "id": "remote-1", "threadId": "different-provider-thread", "labelIds": ["INBOX"],
        "payload": { "mimeType": "multipart/alternative", "headers": [], "parts": [
            { "mimeType": "text/plain", "body": { "data": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode("needle") } },
            { "mimeType": "text/html", "body": { "data": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(html) } },
        ] },
    }).to_string();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0; 8192];
        let len = socket.read(&mut request).await.unwrap();
        assert!(String::from_utf8_lossy(&request[..len]).contains("/messages/remote-1?format=full"));
        socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", response.len(), response).as_bytes()).await.unwrap();
    });
    // No other app-crate test overrides the Gmail endpoint. Restore even on panic.
    struct EndpointGuard(Option<String>);
    impl Drop for EndpointGuard {
        fn drop(&mut self) {
            if let Some(previous) = &self.0 {
                std::env::set_var("SATCHEL_GMAIL_API_BASE", previous);
            } else {
                std::env::remove_var("SATCHEL_GMAIL_API_BASE");
            }
        }
    }
    let _endpoint = EndpointGuard(std::env::var("SATCHEL_GMAIL_API_BASE").ok());
    std::env::set_var("SATCHEL_GMAIL_API_BASE", format!("http://{address}"));
    let fetched = fetch_server_search_hit_impl(
        &pool,
        &FetchServerHitInput {
            account_id: "acc-1".into(),
            remote_id: "remote-1".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(fetched.message_id, "m-1");
    assert_eq!(fetched.thread_id, "th-m-1");
    let stored: Option<String> =
        sqlx::query_scalar("SELECT body_html FROM messages WHERE id = 'm-1'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(stored.as_deref(), Some("<p>needle</p>"));
    tokio::time::timeout(std::time::Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap();
    let original: String =
        sqlx::query_scalar("SELECT original_body_html FROM messages WHERE id = 'm-1'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(original, html);
    let after: String = sqlx::query_scalar(state_sql)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(before, after);
    let other: Option<String> =
        sqlx::query_scalar("SELECT body_html FROM messages WHERE id = 'm-other'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(other, None);
    let other_fetched = fetch_server_search_hit_impl(
        &pool,
        &FetchServerHitInput {
            account_id: "acc-2".into(),
            remote_id: "remote-1".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(other_fetched.message_id, "m-other");
    assert_eq!(other_fetched.thread_id, "th-other");
    let cached: String = sqlx::query_scalar("SELECT body_text FROM messages WHERE id = 'm-other'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(cached, "Other cached body");
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 2);
    let local = MessageSearchRepository::new(&pool)
        .search(&SearchQuery {
            query: "needle".into(),
            account_id: Some("acc-1".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(local.hits[0].id, "m-1");
}
