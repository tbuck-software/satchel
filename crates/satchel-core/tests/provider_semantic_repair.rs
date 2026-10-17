use sqlx::sqlite::SqlitePoolOptions;

const REPAIR_SQL: &str = include_str!("../src/db/migrations/0031_provider_semantic_repair.sql");

#[tokio::test]
async fn legacy_sent_and_draft_flags_are_repaired_from_folder_roles() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("pool");
    for statement in [
        "CREATE TABLE folders (id TEXT PRIMARY KEY, folder_type TEXT NOT NULL)",
        "CREATE TABLE messages (
           id TEXT PRIMARY KEY,
           folder_id TEXT NOT NULL,
           thread_id TEXT,
           message_id_header TEXT,
           remote_id TEXT,
           is_read INTEGER DEFAULT 1,
           is_starred INTEGER DEFAULT 0,
           is_draft INTEGER DEFAULT 0,
           is_sent INTEGER DEFAULT 0
         )",
        "CREATE TABLE threads (
           id TEXT PRIMARY KEY,
           message_count INTEGER DEFAULT 0,
           unread_count INTEGER DEFAULT 0,
           is_starred INTEGER DEFAULT 0
         )",
    ] {
        sqlx::query(statement).execute(&pool).await.expect("schema");
    }
    sqlx::query(
        "INSERT INTO threads (id, message_count, unread_count, is_starred)
         VALUES ('thread', 99, 99, 0), ('thread-imap', 99, 99, 0)",
    )
    .execute(&pool)
    .await
    .expect("thread");
    for (id, kind) in [("drafts", "drafts"), ("sent", "sent"), ("inbox", "inbox")] {
        sqlx::query("INSERT INTO folders (id, folder_type) VALUES (?, ?)")
            .bind(id)
            .bind(kind)
            .execute(&pool)
            .await
            .expect("folder");
        sqlx::query(
            "INSERT INTO messages
             (id, folder_id, thread_id, message_id_header, is_read, is_starred, is_draft, is_sent)
             VALUES (?, ?, 'thread', ?, 1, ?, 0, 0)",
        )
        .bind(format!("message-{id}"))
        .bind(id)
        .bind(format!("<{id}@example.com>"))
        .bind(i64::from(id == "sent"))
        .execute(&pool)
        .await
        .expect("message");
    }
    for (id, folder_id, remote_id) in [
        ("m-imap-account-inbox-1", "inbox", "1"),
        ("m-imap-account-sent-1", "sent", "1"),
    ] {
        sqlx::query(
            "INSERT INTO messages
             (id, folder_id, thread_id, message_id_header, remote_id, is_read,
              is_starred, is_draft, is_sent)
             VALUES (?, ?, 'thread-imap', '<1@imap.flowmail.local>', ?, 0, 0, 0, 0)",
        )
        .bind(id)
        .bind(folder_id)
        .bind(remote_id)
        .execute(&pool)
        .await
        .expect("imap message");
    }

    sqlx::raw_sql(REPAIR_SQL)
        .execute(&pool)
        .await
        .expect("repair migration");

    let flags: Vec<(String, i64, i64)> = sqlx::query_as(
        "SELECT id, is_draft, is_sent FROM messages
             WHERE id LIKE 'message-%' ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .expect("flags");
    assert_eq!(
        flags,
        vec![
            ("message-drafts".to_string(), 1, 0),
            ("message-inbox".to_string(), 0, 0),
            ("message-sent".to_string(), 0, 1),
        ]
    );
    let projection: (i64, i64, i64) = sqlx::query_as(
        "SELECT message_count, unread_count, is_starred FROM threads WHERE id = 'thread'",
    )
    .fetch_one(&pool)
    .await
    .expect("projection");
    assert_eq!(projection, (3, 0, 1));
    let imap_projection: (i64, i64) =
        sqlx::query_as("SELECT message_count, unread_count FROM threads WHERE id = 'thread-imap'")
            .fetch_one(&pool)
            .await
            .expect("imap projection");
    assert_eq!(imap_projection, (2, 2));
}
