use satchel_core::db::messages::{MessageReadError, MessageReadRepository};
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::SqlitePool;

async fn pool() -> SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    sqlx::migrate!("./src/db/migrations")
        .run(&pool)
        .await
        .expect("migrations");
    sqlx::query(
        "INSERT INTO accounts
         (id, display_name, email, provider, config, credentials_ref, is_active, created_at, updated_at)
         VALUES ('account', 'Person', 'person@example.test', 'imap', '{}', '', 1, '2026-01-01', '2026-01-01')",
    )
    .execute(&pool)
    .await
    .expect("account");
    sqlx::query(
        "INSERT INTO folders
         (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at)
         VALUES ('inbox', 'account', 'Inbox', 'INBOX', 'inbox', 'INBOX', '2026-01-01', '2026-01-01')",
    )
    .execute(&pool)
    .await
    .expect("folder");
    pool
}

#[allow(clippy::too_many_arguments)]
async fn insert_message(
    pool: &SqlitePool,
    id: &str,
    thread_id: &str,
    message_id_header: Option<&str>,
    remote_id: Option<&str>,
    date: &str,
    is_read: bool,
    is_draft: bool,
    body_text: Option<&str>,
    body_html: Option<&str>,
    raw_headers: Option<&str>,
) {
    sqlx::query(
        "INSERT INTO messages
         (id, account_id, folder_id, thread_id, message_id_header, remote_id,
          from_address, to_addresses, subject, date, body_text, body_html, snippet,
          is_read, is_draft, raw_size, raw_headers, auth_spf, auth_dkim, auth_dmarc,
          smime_encrypted, smime_signed, pgp_signed, source_payload, created_at, updated_at)
         VALUES (?, 'account', 'inbox', ?, ?, ?, 'sender@example.test', '[]', ?, ?, ?, ?, ?, ?, ?,
                 42, ?, 'pass', 'neutral', 'none', X'01', X'02', X'03', 'source', '2026-01-01', '2026-01-01')",
    )
    .bind(id)
    .bind(thread_id)
    .bind(message_id_header)
    .bind(remote_id)
    .bind(format!("Subject {id}"))
    .bind(date)
    .bind(body_text)
    .bind(body_html)
    .bind(body_text.unwrap_or("snippet"))
    .bind(is_read)
    .bind(is_draft)
    .bind(raw_headers)
    .execute(pool)
    .await
    .expect("message");
}

#[tokio::test]
async fn list_messages_deduplicates_and_orders_like_the_thread_view() {
    let pool = pool().await;
    insert_message(
        &pool,
        "m-old",
        "thread",
        Some("<old@example.test>"),
        None,
        "2026-01-01",
        true,
        true,
        Some("old"),
        None,
        None,
    )
    .await;
    insert_message(
        &pool,
        "m-duplicate-read",
        "thread",
        Some("<duplicate@example.test>"),
        None,
        "2026-01-02",
        true,
        false,
        Some("read"),
        None,
        None,
    )
    .await;
    insert_message(
        &pool,
        "m-duplicate-unread",
        "thread",
        Some("<duplicate@example.test>"),
        None,
        "2026-01-03",
        false,
        false,
        Some("unread"),
        None,
        None,
    )
    .await;
    sqlx::query(
        "UPDATE messages
         SET subject = 'Preview subject', snippet = 'Preview snippet',
             from_name = 'Preview sender', to_addresses = '[\"to@example.test\"]',
             cc_addresses = '[\"cc@example.test\"]', reply_to = 'reply@example.test'
         WHERE id = 'm-old'",
    )
    .execute(&pool)
    .await
    .expect("preview values");

    let repository = MessageReadRepository::new(&pool);
    let rows = repository.list_messages("thread").await.expect("messages");
    assert_eq!(
        rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
        vec!["m-old", "m-duplicate-unread"]
    );

    let previews = repository
        .list_messages_preview("thread")
        .await
        .expect("previews");
    assert_eq!(
        previews
            .iter()
            .map(|row| row.id.as_str())
            .collect::<Vec<_>>(),
        vec!["m-old", "m-duplicate-unread"]
    );
    let preview = &previews[0];
    assert_eq!(preview.subject.as_deref(), Some("Preview subject"));
    assert_eq!(preview.snippet.as_deref(), Some("Preview snippet"));
    assert_eq!(preview.from_address.as_deref(), Some("sender@example.test"));
    assert_eq!(preview.from_name.as_deref(), Some("Preview sender"));
    assert_eq!(
        preview.to_addresses.as_deref(),
        Some("[\"to@example.test\"]")
    );
    assert_eq!(
        preview.cc_addresses.as_deref(),
        Some("[\"cc@example.test\"]")
    );
    assert_eq!(preview.reply_to.as_deref(), Some("reply@example.test"));
    assert_eq!(preview.date.as_deref(), Some("2026-01-01"));
    assert_eq!(preview.is_read, Some(1));
    assert_eq!(preview.is_draft, Some(1));
    // `source_payload` is not part of a preview any more; it comes with the body.
}

#[tokio::test]
async fn thread_reads_dedupe_synthetic_imap_ids_with_folder_remote_fallback() {
    let pool = pool().await;
    sqlx::query(
        "INSERT INTO folders
         (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at)
         VALUES ('archive', 'account', 'Archive', 'Archive', 'archive', 'Archive', '2026-01-01', '2026-01-01')",
    )
    .execute(&pool)
    .await
    .expect("archive folder");

    for (id, message_id_header, remote_id, date, is_read, body) in [
        (
            "m-imap-z-read",
            Some("<synthetic@imap.satchel.local>"),
            Some("remote-42"),
            "2026-04-01",
            true,
            "read representative",
        ),
        (
            "m-imap-a-unread",
            Some("<synthetic@imap.satchel.local>"),
            Some("remote-42"),
            "2026-04-01",
            false,
            "unread representative",
        ),
        (
            "m-imap-b-other-remote",
            Some("<synthetic@imap.satchel.local>"),
            Some("remote-43"),
            "2026-04-02",
            true,
            "other remote",
        ),
        (
            "m-imap-c-other-folder",
            Some("<synthetic@imap.satchel.local>"),
            Some("remote-42"),
            "2026-04-03",
            true,
            "other folder",
        ),
        ("m-normal-z", None, None, "2026-04-04", true, "normal z"),
        ("m-normal-a", None, None, "2026-04-05", true, "normal a"),
        (
            "m-imap-z-same",
            Some("<synthetic@imap.satchel.local>"),
            Some("remote-44"),
            "2026-04-06",
            true,
            "same-date z",
        ),
        (
            "m-imap-a-same",
            Some("<synthetic@imap.satchel.local>"),
            Some("remote-44"),
            "2026-04-06",
            true,
            "same-date a",
        ),
    ] {
        insert_message(
            &pool,
            id,
            "imap-thread",
            message_id_header,
            remote_id,
            date,
            is_read,
            false,
            Some(body),
            None,
            None,
        )
        .await;
    }
    sqlx::query("UPDATE messages SET folder_id = 'archive' WHERE id = 'm-imap-c-other-folder'")
        .execute(&pool)
        .await
        .expect("move synthetic copy to archive");

    let repository = MessageReadRepository::new(&pool);
    let rows = repository
        .list_messages("imap-thread")
        .await
        .expect("full messages");
    let ids = rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>();
    assert_eq!(
        ids,
        vec![
            "m-imap-a-unread",
            "m-imap-b-other-remote",
            "m-imap-c-other-folder",
            "m-normal-z",
            "m-normal-a",
            "m-imap-a-same",
        ]
    );
    assert_eq!(rows[0].is_read, Some(0));
    assert_eq!(rows[0].body_text.as_deref(), Some("unread representative"));
    assert_eq!(rows[5].body_text.as_deref(), Some("same-date a"));

    let previews = repository
        .list_messages_preview("imap-thread")
        .await
        .expect("preview messages");
    assert_eq!(
        previews
            .iter()
            .map(|row| row.id.as_str())
            .collect::<Vec<_>>(),
        ids
    );
    assert_eq!(previews[0].is_read, Some(0));
    assert_eq!(
        previews[0].snippet.as_deref(),
        Some("unread representative")
    );
}

#[tokio::test]
async fn list_message_bodies_selects_requested_ids_and_orders_by_date() {
    let pool = pool().await;
    insert_message(
        &pool,
        "m-old",
        "thread-old",
        None,
        None,
        "2026-01-01",
        true,
        false,
        Some("old"),
        Some("<p>old</p>"),
        None,
    )
    .await;
    insert_message(
        &pool,
        "m-new",
        "thread-new",
        None,
        None,
        "2026-03-01",
        true,
        false,
        Some("new"),
        Some("<p>new</p>"),
        None,
    )
    .await;

    let repository = MessageReadRepository::new(&pool);
    let rows = repository
        .list_message_bodies(&["m-new".into(), "missing".into(), "m-old".into()])
        .await
        .expect("bodies");
    assert_eq!(
        rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
        vec!["m-old", "m-new"]
    );
    assert_eq!(rows[0].body_html.as_deref(), Some("<p>old</p>"));
    assert_eq!(rows[1].body_text.as_deref(), Some("new"));

    assert!(repository
        .list_message_bodies(&[])
        .await
        .expect("empty body selection")
        .is_empty());
}

#[tokio::test]
async fn source_reads_normalize_blank_headers_and_report_missing_messages() {
    let pool = pool().await;
    insert_message(
        &pool,
        "m-blank-header",
        "thread",
        None,
        None,
        "2026-01-01",
        true,
        false,
        Some("body"),
        Some("<p>body</p>"),
        Some(" \n\t"),
    )
    .await;
    insert_message(
        &pool,
        "m-source",
        "thread",
        None,
        None,
        "2026-01-02",
        true,
        false,
        Some("body"),
        None,
        Some("Subject: Source\n"),
    )
    .await;

    let repository = MessageReadRepository::new(&pool);
    assert_eq!(
        repository
            .get_message_source("m-blank-header")
            .await
            .expect("blank source")
            .raw_headers,
        None
    );
    let source = repository
        .get_message_source("m-source")
        .await
        .expect("source");
    assert_eq!(source.message_id, "m-source");
    assert_eq!(source.raw_headers.as_deref(), Some("Subject: Source\n"));
    assert_eq!(source.raw_size, Some(42));

    assert!(matches!(
        repository.get_message_source("missing").await,
        Err(MessageReadError::NotFound)
    ));
}

#[tokio::test]
async fn message_row_projection_decodes_every_column_owned_by_the_repository() {
    let pool = pool().await;
    insert_message(
        &pool,
        "m-columns",
        "thread",
        None,
        None,
        "2026-01-01",
        false,
        true,
        Some("Text"),
        Some("<p>Text</p>"),
        None,
    )
    .await;

    let repository = MessageReadRepository::new(&pool);
    let body = repository
        .list_message_bodies(&["m-columns".into()])
        .await
        .expect("body projection");
    assert_eq!(body.len(), 1);
    assert_eq!(body[0].auth_spf.as_deref(), Some("pass"));
    assert_eq!(body[0].auth_dkim.as_deref(), Some("neutral"));
    assert_eq!(body[0].auth_dmarc.as_deref(), Some("none"));
    assert_eq!(body[0].smime_encrypted.as_deref(), Some(&[1_u8][..]));
    assert_eq!(body[0].smime_signed.as_deref(), Some(&[2_u8][..]));
    assert_eq!(body[0].pgp_signed.as_deref(), Some(&[3_u8][..]));
    assert_eq!(body[0].source_payload.as_deref(), Some("source"));

    let thread = repository.list_messages("thread").await.expect("thread");
    assert_eq!(thread.len(), 1);
    assert_eq!(thread[0].body_html.as_deref(), Some("<p>Text</p>"));
}
