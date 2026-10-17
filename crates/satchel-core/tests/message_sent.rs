use satchel_core::db::messages::{
    ReplyThreadContext, SentAttachmentMetadata, SentMessageError, SentMessageRepository,
    SentMessageWriteRequest,
};
use sqlx::{
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
    SqlitePool,
};

async fn pool() -> SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory db");
    sqlx::migrate!("./src/db/migrations")
        .run(&pool)
        .await
        .expect("migrations");
    pool
}

async fn concurrent_pool() -> (tempfile::TempDir, SqlitePool) {
    let directory = tempfile::tempdir().expect("temp directory");
    let options = SqliteConnectOptions::new()
        .filename(directory.path().join("message-sent.sqlite"))
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(std::time::Duration::from_secs(5));
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect_with(options)
        .await
        .expect("file db");
    sqlx::migrate!("./src/db/migrations")
        .run(&pool)
        .await
        .expect("migrations");
    (directory, pool)
}

async fn seed_account(pool: &SqlitePool, account_id: &str, email: &str) {
    sqlx::query(
        "INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, sync_state, color, is_active, created_at, updated_at)
         VALUES (?, 'Me', ?, 'imap', '{}', '', '{}', '#000', 1, '2026-01-01', '2026-01-01')",
    )
    .bind(account_id)
    .bind(email)
    .execute(pool)
    .await
    .expect("account");
}

fn request(account_id: &str) -> SentMessageWriteRequest {
    SentMessageWriteRequest {
        account_id: account_id.to_string(),
        account_email: "me@example.com".to_string(),
        account_display_name: "Me".to_string(),
        subject: "  Hello world  ".to_string(),
        body: "Hello\nthere".to_string(),
        from_address: None,
        from_name: None,
        to: vec!["friend@example.com".to_string()],
        cc: vec!["copy@example.com".to_string()],
        bcc: vec!["blind@example.com".to_string()],
        ..Default::default()
    }
}

#[tokio::test]
async fn persists_new_thread_and_repairs_missing_sent_folder() {
    let pool = pool().await;
    seed_account(&pool, "acc-1", "me@example.com").await;

    let result = SentMessageRepository::new(&pool)
        .persist(request("acc-1"))
        .await
        .expect("sent message");

    assert!(result.message_id.starts_with("m-"));
    assert!(result.thread_id.starts_with("th-"));

    let folder: (String, String) = sqlx::query_as(
        "SELECT id, folder_type FROM folders WHERE account_id = 'acc-1' AND folder_type = 'sent'",
    )
    .fetch_one(&pool)
    .await
    .expect("sent folder");
    let message: (String, String, String, String, i64, i64, String) = sqlx::query_as(
        "SELECT folder_id, subject, snippet, message_id_header, is_sent, has_attachments, to_addresses
         FROM messages WHERE id = ?",
    )
    .bind(&result.message_id)
    .fetch_one(&pool)
    .await
    .expect("sent message row");
    assert_eq!(message.0, folder.0);
    assert_eq!(folder.1, "sent");
    assert_eq!(message.1, "Hello world");
    assert_eq!(message.2, "Hello there");
    assert!(message.3.starts_with('<'));
    assert_eq!(message.4, 1);
    assert_eq!(message.5, 0);
    assert_eq!(message.6, r#"["friend@example.com"]"#);

    let thread: (String, i64, i64, String) = sqlx::query_as(
        "SELECT workflow_state, message_count, unread_count, participants FROM threads WHERE id = ?",
    )
    .bind(&result.thread_id)
    .fetch_one(&pool)
    .await
    .expect("sent thread");
    assert_eq!(thread.0, "waiting");
    assert_eq!(thread.1, 1);
    assert_eq!(thread.2, 0);
    assert_eq!(thread.3, r#"["friend@example.com"]"#);
}

async fn seed_thread(pool: &SqlitePool, thread_id: &str, state: &str) {
    sqlx::query(
        "INSERT INTO threads (id, account_id, subject, last_message_date, message_count, unread_count, has_attachments, participants, snippet, workflow_state, workflow_changed_at, board_column, board_position, created_at, updated_at)
         VALUES (?, 'acc-1', 'Old subject', '2026-01-01', 1, 1, 0, '[\"old@example.com\"]', 'Old body', ?, '2026-01-01', ?, 1, '2026-01-01', '2026-01-01')",
    )
    .bind(thread_id)
    .bind(state)
    .bind(state)
    .execute(pool)
    .await
    .expect("thread");
    sqlx::query(
        "INSERT INTO folders (id, account_id, name, full_path, folder_type, created_at, updated_at)
         VALUES ('fld-inbox', 'acc-1', 'Inbox', 'Inbox', 'inbox', '2026-01-01', '2026-01-01')",
    )
    .execute(pool)
    .await
    .expect("inbox folder");
    sqlx::query(
        "INSERT INTO messages (id, account_id, folder_id, thread_id, message_id_header, references_header, from_address, from_name, to_addresses, subject, date, body_text, snippet, is_read, is_sent, created_at, updated_at)
         VALUES ('old-message', 'acc-1', 'fld-inbox', ?, '<old@example.com>', '<root@example.com>', 'friend@example.com', 'Friend', '[\"me@example.com\"]', 'Old subject', '2026-01-01', 'Old body', 'Old body', 0, 0, '2026-01-01', '2026-01-01')",
    )
    .bind(thread_id)
    .execute(pool)
    .await
    .expect("old message");
}

#[tokio::test]
async fn existing_thread_transitions_once_and_waiting_replies_emit_no_extra_event() {
    let pool = pool().await;
    seed_account(&pool, "acc-1", "me@example.com").await;
    seed_thread(&pool, "thread-1", "todo").await;

    let mut first = request("acc-1");
    first.thread_id = Some("thread-1".to_string());
    let first_result = SentMessageRepository::new(&pool)
        .persist(first)
        .await
        .expect("first reply");

    let first_state: (String, i64, i64) = sqlx::query_as(
        "SELECT workflow_state, message_count, unread_count FROM threads WHERE id = 'thread-1'",
    )
    .fetch_one(&pool)
    .await
    .expect("first thread state");
    assert_eq!(first_state, ("waiting".to_string(), 2, 0));
    let event_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM workflow_events WHERE thread_id = 'thread-1'")
            .fetch_one(&pool)
            .await
            .expect("workflow event count");
    assert_eq!(event_count, 1);

    let mut second = request("acc-1");
    second.thread_id = Some("thread-1".to_string());
    second.body = "A second reply".to_string();
    let second_result = SentMessageRepository::new(&pool)
        .persist(second)
        .await
        .expect("second reply");
    assert_ne!(first_result.message_id, second_result.message_id);

    let event_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM workflow_events WHERE thread_id = 'thread-1'")
            .fetch_one(&pool)
            .await
            .expect("workflow event count");
    assert_eq!(event_count, 1);
}

#[tokio::test]
async fn persists_attachment_and_signature_metadata() {
    let pool = pool().await;
    seed_account(&pool, "acc-1", "me@example.com").await;
    let mut input = request("acc-1");
    input.body_html = Some("<p>Hello</p>".to_string());
    input.signature_id = Some("signature-1".to_string());
    input.signature_variant = Some("short".to_string());
    input.attachments = vec![SentAttachmentMetadata {
        filename: "hello.txt".to_string(),
        mime_type: "text/plain".to_string(),
        size: 6,
        content_id: Some("<hello@example.com>".to_string()),
        inline: true,
    }];

    let result = SentMessageRepository::new(&pool)
        .persist(input)
        .await
        .expect("sent message");
    let message: (i64, Option<String>, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT has_attachments, body_html, signature_id, signature_variant FROM messages WHERE id = ?",
    )
    .bind(&result.message_id)
    .fetch_one(&pool)
    .await
    .expect("message metadata");
    assert_eq!(message.0, 1);
    assert_eq!(message.1.as_deref(), Some("<p>Hello</p>"));
    assert_eq!(message.2.as_deref(), Some("signature-1"));
    assert_eq!(message.3.as_deref(), Some("short"));

    let attachment: (String, String, i64, Option<String>, i64) = sqlx::query_as(
        "SELECT filename, content_type, size, content_id, is_inline FROM attachments WHERE message_id = ?",
    )
    .bind(&result.message_id)
    .fetch_one(&pool)
    .await
    .expect("attachment metadata");
    assert_eq!(attachment.0, "hello.txt");
    assert_eq!(attachment.1, "text/plain");
    assert_eq!(attachment.2, 6);
    assert_eq!(attachment.3.as_deref(), Some("<hello@example.com>"));
    assert_eq!(attachment.4, 1);
}

#[tokio::test]
async fn reply_context_is_account_scoped_and_merges_references() {
    let pool = pool().await;
    seed_account(&pool, "acc-1", "me@example.com").await;
    seed_account(&pool, "acc-2", "other@example.com").await;
    seed_thread(&pool, "thread-1", "inbox").await;
    sqlx::query(
        "INSERT INTO threads (id, account_id, subject, last_message_date, created_at, updated_at)
         VALUES ('thread-2', 'acc-2', 'Other', '2026-01-01', '2026-01-01', '2026-01-01')",
    )
    .execute(&pool)
    .await
    .expect("other thread");
    sqlx::query(
        "INSERT INTO folders (id, account_id, name, full_path, folder_type, created_at, updated_at)
         VALUES ('fld-other', 'acc-2', 'Inbox', 'Inbox', 'inbox', '2026-01-01', '2026-01-01')",
    )
    .execute(&pool)
    .await
    .expect("other folder");
    sqlx::query(
        "INSERT INTO messages (id, account_id, folder_id, thread_id, message_id_header, references_header, from_address, to_addresses, subject, date, created_at, updated_at)
         VALUES ('other-message', 'acc-2', 'fld-other', 'thread-2', '<other@example.com>', '<other-root@example.com>', 'other@example.com', '[]', 'Other', '2026-01-02', '2026-01-02', '2026-01-02')",
    )
    .execute(&pool)
    .await
    .expect("other message");

    let context = SentMessageRepository::new(&pool)
        .load_reply_context("acc-1", "thread-1", None)
        .await
        .expect("reply context");
    assert_eq!(
        context,
        ReplyThreadContext {
            in_reply_to: Some("<old@example.com>".to_string()),
            references: vec![
                "<root@example.com>".to_string(),
                "<old@example.com>".to_string()
            ],
        }
    );

    let other_context = SentMessageRepository::new(&pool)
        .load_reply_context("acc-2", "thread-1", None)
        .await
        .expect("account-scoped empty context");
    assert_eq!(other_context, ReplyThreadContext::default());
}

/// Insert one more message into `thread-1`.
async fn seed_thread_message(
    pool: &SqlitePool,
    id: &str,
    message_id_header: &str,
    references: &str,
    date: &str,
) {
    sqlx::query(
        "INSERT INTO messages (id, account_id, folder_id, thread_id, message_id_header, references_header, from_address, to_addresses, subject, date, created_at, updated_at)
         VALUES (?, 'acc-1', 'fld-inbox', 'thread-1', ?, ?, 'friend@example.com', '[\"me@example.com\"]', 'Old subject', ?, '2026-01-01', '2026-01-01')",
    )
    .bind(id)
    .bind(message_id_header)
    .bind(references)
    .bind(date)
    .execute(pool)
    .await
    .expect("thread message");
}

#[tokio::test]
async fn a_named_message_wins_over_the_newest_one_in_the_thread() {
    let pool = pool().await;
    seed_account(&pool, "acc-1", "me@example.com").await;
    seed_thread(&pool, "thread-1", "inbox").await;
    seed_thread_message(
        &pool,
        "newer-message",
        "<newer@example.com>",
        "<root@example.com> <old@example.com>",
        "2026-01-05",
    )
    .await;

    let repository = SentMessageRepository::new(&pool);
    let answered = repository
        .load_reply_context("acc-1", "thread-1", Some("old-message"))
        .await
        .expect("reply context");
    assert_eq!(answered.in_reply_to.as_deref(), Some("<old@example.com>"));

    // Without the field the newest message stands in, as before.
    let newest = repository
        .load_reply_context("acc-1", "thread-1", None)
        .await
        .expect("reply context");
    assert_eq!(newest.in_reply_to.as_deref(), Some("<newer@example.com>"));

    // A message the account does not own falls back rather than answering it.
    let unknown = repository
        .load_reply_context("acc-1", "thread-1", Some("does-not-exist"))
        .await
        .expect("reply context");
    assert_eq!(unknown, newest);
}

#[tokio::test]
async fn messages_sharing_a_date_resolve_reproducibly() {
    let pool = pool().await;
    seed_account(&pool, "acc-1", "me@example.com").await;
    seed_thread(&pool, "thread-1", "inbox").await;
    // Same `date` as `old-message`, so only the tiebreaker decides.
    seed_thread_message(
        &pool,
        "z-tied-message",
        "<tied@example.com>",
        "<root@example.com>",
        "2026-01-01",
    )
    .await;

    let repository = SentMessageRepository::new(&pool);
    let first = repository
        .load_reply_context("acc-1", "thread-1", None)
        .await
        .expect("reply context");
    let second = repository
        .load_reply_context("acc-1", "thread-1", None)
        .await
        .expect("reply context");

    assert_eq!(first, second);
    assert_eq!(first.in_reply_to.as_deref(), Some("<tied@example.com>"));
}

#[tokio::test]
async fn an_open_draft_in_the_thread_is_not_what_the_reply_answers() {
    let pool = pool().await;
    seed_account(&pool, "acc-1", "me@example.com").await;
    seed_thread(&pool, "thread-1", "inbox").await;
    sqlx::query(
        "INSERT INTO messages (id, account_id, folder_id, thread_id, message_id_header, from_address, to_addresses, subject, date, is_draft, created_at, updated_at)
         VALUES ('draft-1', 'acc-1', 'fld-inbox', 'thread-1', '<draft-1@satchel.draft>', 'me@example.com', '[]', 'Old subject', '2026-01-07', 1, '2026-01-01', '2026-01-01')",
    )
    .execute(&pool)
    .await
    .expect("draft row");

    let context = SentMessageRepository::new(&pool)
        .load_reply_context("acc-1", "thread-1", None)
        .await
        .expect("reply context");

    assert_eq!(context.in_reply_to.as_deref(), Some("<old@example.com>"));
}

#[tokio::test]
async fn an_overlong_reference_chain_keeps_the_root_and_the_recent_entries() {
    let pool = pool().await;
    seed_account(&pool, "acc-1", "me@example.com").await;
    seed_thread(&pool, "thread-1", "inbox").await;
    let inherited: Vec<String> = (0..60)
        .map(|index| format!("<ref-{index:02}@example.com>"))
        .collect();
    seed_thread_message(
        &pool,
        "deep-message",
        "<deep@example.com>",
        &inherited.join(" "),
        "2026-01-09",
    )
    .await;

    let context = SentMessageRepository::new(&pool)
        .load_reply_context("acc-1", "thread-1", None)
        .await
        .expect("reply context");

    assert_eq!(context.references.len(), 20);
    assert_eq!(context.references[0], inherited[0]);
    assert_eq!(
        context.references.last().map(String::as_str),
        Some("<deep@example.com>")
    );
    assert_eq!(context.references[1], inherited[42]);
}

#[tokio::test]
async fn a_short_reference_chain_is_left_alone() {
    let pool = pool().await;
    seed_account(&pool, "acc-1", "me@example.com").await;
    seed_thread(&pool, "thread-1", "inbox").await;

    let context = SentMessageRepository::new(&pool)
        .load_reply_context("acc-1", "thread-1", None)
        .await
        .expect("reply context");

    assert_eq!(
        context.references,
        vec![
            "<root@example.com>".to_string(),
            "<old@example.com>".to_string()
        ]
    );
}

#[tokio::test]
async fn missing_reply_thread_is_typed_and_writes_no_partial_message_or_thread() {
    let pool = pool().await;
    seed_account(&pool, "acc-1", "me@example.com").await;
    let mut input = request("acc-1");
    input.thread_id = Some("missing-thread".to_string());

    let error = SentMessageRepository::new(&pool)
        .persist(input)
        .await
        .expect_err("missing reply thread");
    assert!(matches!(
        error,
        SentMessageError::ReplyThreadNotFound { ref thread_id }
            if thread_id == "missing-thread"
    ));
    let thread_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM threads")
        .fetch_one(&pool)
        .await
        .expect("thread count");
    let message_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages")
        .fetch_one(&pool)
        .await
        .expect("message count");
    let folder_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM folders")
        .fetch_one(&pool)
        .await
        .expect("folder count");
    assert_eq!(thread_count, 0);
    assert_eq!(message_count, 0);
    assert_eq!(folder_count, 0);
}

#[tokio::test]
async fn rolls_back_folder_thread_and_message_when_the_message_insert_fails() {
    let pool = pool().await;
    seed_account(&pool, "acc-1", "me@example.com").await;
    sqlx::query(
        "CREATE TRIGGER reject_sent_message
         BEFORE INSERT ON messages WHEN NEW.is_sent = 1
         BEGIN SELECT RAISE(ABORT, 'forced sent-message failure'); END",
    )
    .execute(&pool)
    .await
    .expect("failure trigger");

    let error = SentMessageRepository::new(&pool)
        .persist(request("acc-1"))
        .await
        .expect_err("forced persistence failure");
    assert!(matches!(error, SentMessageError::Persistence(_)));

    for table in ["folders", "threads", "messages"] {
        let count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(&pool)
            .await
            .expect("row count");
        assert_eq!(count, 0, "{table} must roll back with the message");
    }
}

#[tokio::test]
async fn concurrent_first_sends_create_one_sent_folder() {
    let (_directory, pool) = concurrent_pool().await;
    seed_account(&pool, "acc-1", "me@example.com").await;
    let repository = SentMessageRepository::new(&pool);

    let (first, second) = tokio::join!(
        repository.persist(request("acc-1")),
        repository.persist(request("acc-1")),
    );
    first.expect("first send");
    second.expect("second send");

    let folder_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM folders WHERE account_id = 'acc-1' AND folder_type = 'sent'",
    )
    .fetch_one(&pool)
    .await
    .expect("folder count");
    let message_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE is_sent = 1")
        .fetch_one(&pool)
        .await
        .expect("message count");
    assert_eq!(folder_count, 1);
    assert_eq!(message_count, 2);
}
