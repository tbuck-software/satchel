use satchel_lib::providers::{
    folder_mapping::provider_folder_type,
    traits::{RemoteIdTranslation, RemoteMessageRemoval},
};
use satchel_lib::sync::folder_reconcile::reconcile_remote_message_removals;
use satchel_lib::sync::outlook_migration::apply_outlook_id_translations;
use sqlx::sqlite::SqlitePoolOptions;

#[test]
fn provider_folder_roles_do_not_depend_on_english_display_names() {
    assert_eq!(
        provider_folder_type("outlook", "Gesendete Elemente", "sentitems", None),
        "sent"
    );
    assert_eq!(
        provider_folder_type("outlook", "Entwürfe", "drafts", None),
        "drafts"
    );
    assert_eq!(
        provider_folder_type("outlook", "Gelöschte Elemente", "deleteditems", None),
        "trash"
    );
    assert_eq!(
        provider_folder_type("imap", "Papierkorb", "Papierkorb", None),
        "trash"
    );
    assert_eq!(
        provider_folder_type("imap", "Gesendet", "Gesendet", Some("sent")),
        "sent"
    );
}

#[tokio::test]
async fn remote_tombstone_removes_only_the_matching_folder_copy_and_repairs_thread_count() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("pool");
    sqlx::migrate!("../crates/satchel-core/src/db/migrations")
        .run(&pool)
        .await
        .expect("migrations");

    let now = "2026-07-16T10:00:00Z";
    sqlx::query(
        "INSERT INTO accounts
         (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
         VALUES ('account', 'Account', 'me@example.com', 'outlook', '{}', '', ?, ?)",
    )
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await
    .expect("account");
    for (id, name, kind, remote_id) in [
        ("inbox", "Posteingang", "inbox", "inbox"),
        ("archive", "Archiv", "archive", "archive"),
    ] {
        sqlx::query(
            "INSERT INTO folders
             (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at)
             VALUES (?, 'account', ?, ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(name)
        .bind(name)
        .bind(kind)
        .bind(remote_id)
        .bind(now)
        .bind(now)
        .execute(&pool)
        .await
        .expect("folder");
    }
    sqlx::query(
        "INSERT INTO threads
         (id, account_id, subject, last_message_date, message_count, unread_count,
          workflow_state, created_at, updated_at)
         VALUES ('thread', 'account', 'Subject', ?, 2, 0, 'inbox', ?, ?)",
    )
    .bind(now)
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await
    .expect("thread");
    for (id, folder, remote_id, header) in [
        ("old-copy", "inbox", "remote-old", "<old@example.com>"),
        ("keep-copy", "archive", "remote-keep", "<keep@example.com>"),
    ] {
        sqlx::query(
            "INSERT INTO messages
             (id, account_id, folder_id, thread_id, message_id_header, from_address,
              to_addresses, subject, date, is_read, remote_id, created_at, updated_at)
             VALUES (?, 'account', ?, 'thread', ?, 'sender@example.com', '[]',
                     'Subject', ?, 1, ?, ?, ?)",
        )
        .bind(id)
        .bind(folder)
        .bind(header)
        .bind(now)
        .bind(remote_id)
        .bind(now)
        .bind(now)
        .execute(&pool)
        .await
        .expect("message");
    }

    reconcile_remote_message_removals(
        &pool,
        "account",
        &[RemoteMessageRemoval {
            remote_id: "remote-old".to_string(),
            source_folder: Some("inbox".to_string()),
        }],
    )
    .await
    .expect("reconcile tombstone");

    let remaining: Vec<String> = sqlx::query_scalar("SELECT id FROM messages ORDER BY id")
        .fetch_all(&pool)
        .await
        .expect("messages");
    let count: i64 = sqlx::query_scalar("SELECT message_count FROM threads WHERE id = 'thread'")
        .fetch_one(&pool)
        .await
        .expect("thread count");
    assert_eq!(remaining, vec!["keep-copy"]);
    assert_eq!(count, 1);
}

#[tokio::test]
async fn gmail_mailbox_tombstone_removes_every_label_copy() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("pool");
    sqlx::migrate!("../crates/satchel-core/src/db/migrations")
        .run(&pool)
        .await
        .expect("migrations");
    let now = "2026-07-16T10:00:00Z";
    sqlx::query(
        "INSERT INTO accounts
         (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
         VALUES ('account', 'Account', 'me@example.com', 'gmail', '{}', '', ?, ?)",
    )
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await
    .expect("account");
    for (id, kind, remote_id) in [("inbox", "inbox", "INBOX"), ("custom", "custom", "Label_1")] {
        sqlx::query(
            "INSERT INTO folders
             (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at)
             VALUES (?, 'account', ?, ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(id)
        .bind(id)
        .bind(kind)
        .bind(remote_id)
        .bind(now)
        .bind(now)
        .execute(&pool)
        .await
        .expect("folder");
    }
    sqlx::query(
        "INSERT INTO threads
         (id, account_id, subject, last_message_date, message_count, unread_count,
          workflow_state, created_at, updated_at)
         VALUES ('thread', 'account', 'Subject', ?, 1, 0, 'inbox', ?, ?)",
    )
    .bind(now)
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await
    .expect("thread");
    for (id, folder) in [("copy-inbox", "inbox"), ("copy-label", "custom")] {
        sqlx::query(
            "INSERT INTO messages
             (id, account_id, folder_id, thread_id, message_id_header, from_address,
              to_addresses, subject, date, is_read, remote_id, created_at, updated_at)
             VALUES (?, 'account', ?, 'thread', '<same@example.com>', 'sender@example.com',
                     '[]', 'Subject', ?, 1, 'gmail-message', ?, ?)",
        )
        .bind(id)
        .bind(folder)
        .bind(now)
        .bind(now)
        .bind(now)
        .execute(&pool)
        .await
        .expect("message");
    }

    reconcile_remote_message_removals(
        &pool,
        "account",
        &[RemoteMessageRemoval {
            remote_id: "gmail-message".to_string(),
            source_folder: None,
        }],
    )
    .await
    .expect("mailbox removal");
    let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages")
        .fetch_one(&pool)
        .await
        .expect("remaining");
    assert_eq!(remaining, 0);
}

#[tokio::test]
async fn outlook_mutable_ids_are_upgraded_atomically() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("pool");
    sqlx::migrate!("../crates/satchel-core/src/db/migrations")
        .run(&pool)
        .await
        .expect("migrations");
    let now = "2026-07-16T10:00:00Z";
    sqlx::query(
        "INSERT INTO accounts
         (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
         VALUES ('account', 'Account', 'me@example.com', 'outlook', '{}', '', ?, ?)",
    )
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await
    .expect("account");
    sqlx::query(
        "INSERT INTO folders
         (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at)
         VALUES ('drafts', 'account', 'Drafts', 'drafts', 'drafts', 'drafts', ?, ?)",
    )
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await
    .expect("folder");
    sqlx::query(
        "INSERT INTO threads
         (id, account_id, subject, last_message_date, message_count, unread_count,
          workflow_state, created_at, updated_at)
         VALUES ('thread', 'account', 'Draft', ?, 1, 0, 'inbox', ?, ?)",
    )
    .bind(now)
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await
    .expect("thread");
    sqlx::query(
        "INSERT INTO messages
         (id, account_id, folder_id, thread_id, message_id_header, from_address,
          to_addresses, subject, date, is_read, is_draft, remote_id, remote_draft_id,
          created_at, updated_at)
         VALUES ('message', 'account', 'drafts', 'thread', '<draft@example.com>',
                 'me@example.com', '[]', 'Draft', ?, 1, 1, NULL, 'mutable-id', ?, ?)",
    )
    .bind(now)
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await
    .expect("message");
    sqlx::query(
        "INSERT INTO attachments
         (id, message_id, content_type, remote_uid, created_at)
         VALUES ('attachment', 'message', 'text/plain', 'mutable-id', ?)",
    )
    .bind(now)
    .execute(&pool)
    .await
    .expect("attachment");

    apply_outlook_id_translations(
        &pool,
        "account",
        &[RemoteIdTranslation {
            source_id: "mutable-id".to_string(),
            target_id: "immutable-id".to_string(),
        }],
    )
    .await
    .expect("translation");
    let row: (String, String, i64) = sqlx::query_as(
        "SELECT m.remote_id, m.remote_draft_id, a.outlook_immutable_ids_migrated
         FROM messages m JOIN accounts a ON a.id = m.account_id
         WHERE m.id = 'message'",
    )
    .fetch_one(&pool)
    .await
    .expect("translated row");
    assert_eq!(row, ("immutable-id".into(), "immutable-id".into(), 1));
    let attachment_remote_id: String =
        sqlx::query_scalar("SELECT remote_uid FROM attachments WHERE id = 'attachment'")
            .fetch_one(&pool)
            .await
            .expect("attachment remote id");
    assert_eq!(attachment_remote_id, "immutable-id");
}
