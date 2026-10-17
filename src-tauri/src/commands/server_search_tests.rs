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
async fn server_hits_already_synced_locally_are_dropped() {
    use crate::commands::server_search::{drop_locally_known_hits, ServerSearchHit};

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

    let remaining =
        drop_locally_known_hits(&pool, "acc-1", vec![hit("remote-known"), hit("remote-new")])
            .await
            .unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].remote_id, "remote-new");

    // Same remote id under a different account must not be filtered.
    let remaining = drop_locally_known_hits(&pool, "acc-other", vec![hit("remote-known")])
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
    sqlx::query("UPDATE messages SET remote_id = 'remote-1' WHERE id = 'm-1'")
        .execute(&pool)
        .await
        .unwrap();

    // Must return the local ids without any network access (the seeded account
    // has an empty config; a provider call would fail loudly).
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
