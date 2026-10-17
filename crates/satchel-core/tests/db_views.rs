//! The `v_*` read views are a published contract (ADR 0005): AI tools open
//! satchel.db directly instead of calling an API. A view that silently stops
//! resolving — because a column it selects was renamed under it — breaks those
//! tools with no signal from the app, so every view is queried here.

use sqlx::{sqlite::SqlitePoolOptions, Row, SqlitePool};

/// Every view the contract promises. Removing one is a breaking change.
const VIEWS: &[&str] = &[
    "v_accounts",
    "v_folders",
    "v_threads",
    "v_messages",
    "v_attachments",
    "v_topics",
    "v_tags",
];

/// Substrings that must never appear in a view's column names.
const SECRET_COLUMN_PATTERNS: &[&str] = &["config", "token", "secret", "password", "credential"];

async fn migrated_pool() -> SqlitePool {
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

/// One row per view, mirroring the shape the app writes.
///
/// Deliberately not `db::seed_if_empty`: that seeder is private, opt-in via
/// `SATCHEL_SEED_DEMO_DATA`, and covers neither attachments nor topics.
async fn seed(pool: &SqlitePool) {
    let now = "2026-08-28T09:00:00Z";
    let statements = [
        "INSERT INTO accounts
           (id, display_name, email, provider, config, credentials_ref, kind, created_at, updated_at)
         VALUES ('acc', 'Arbeit', 'me@example.com', 'imap', '{\"secret\":true}', 'ref', 'email', ?, ?)",
        "INSERT INTO folders
           (id, account_id, name, full_path, folder_type, created_at, updated_at)
         VALUES ('fld', 'acc', 'Posteingang', 'INBOX', 'inbox', ?, ?)",
        "INSERT INTO threads
           (id, account_id, subject, last_message_date, message_count, unread_count,
            participants, workflow_state, is_starred, created_at, updated_at)
         VALUES ('thr', 'acc', 'Rechnung März', '2026-08-28T08:00:00Z', 1, 1,
            '[{\"name\":\"Sarah\",\"email\":\"sarah@app.io\"},\"bob@example.org\"]',
            'todo', 1, ?, ?)",
        "INSERT INTO messages
           (id, account_id, folder_id, thread_id, message_id_header, from_address, from_name,
            to_addresses, cc_addresses, subject, date, body_text, body_html, has_attachments,
            created_at, updated_at)
         VALUES ('msg', 'acc', 'fld', 'thr', '<rechnung-1@app.io>', 'sarah@app.io', 'Sarah',
            '[\"me@example.com\"]',
            '[\"bob@example.org\"]', 'Rechnung März', '2026-08-28T08:00:00Z',
            'Anbei die Rechnung.', '<p>Anbei die Rechnung.</p>', 1, ?, ?)",
        "INSERT INTO attachments
           (id, message_id, filename, content_type, size, is_inline, created_at)
         VALUES ('att', 'msg', 'rechnung.pdf', 'application/pdf', 4096, 0, ?)",
        "INSERT INTO tags (id, name) VALUES ('tag', 'Wichtig')",
        "INSERT INTO thread_tags (thread_id, tag_id) VALUES ('thr', 'tag')",
        "INSERT INTO topics (id, name, created_at, updated_at) VALUES ('top', 'Umzug', ?, ?)",
        "INSERT INTO topic_items (topic_id, item_type, item_id, added_at)
         VALUES ('top', 'thread', 'thr', ?)",
    ];

    for statement in statements {
        let mut query = sqlx::query(statement);
        for _ in 0..statement.matches('?').count() {
            query = query.bind(now);
        }
        query.execute(pool).await.expect("seed row");
    }
}

async fn view_columns(pool: &SqlitePool, view: &str) -> Vec<String> {
    sqlx::query("SELECT name FROM pragma_table_info(?)")
        .bind(view)
        .fetch_all(pool)
        .await
        .expect("view columns")
        .into_iter()
        .map(|row| row.get::<String, _>("name"))
        .collect()
}

#[tokio::test]
async fn every_read_view_exists_and_resolves() {
    let pool = migrated_pool().await;
    seed(&pool).await;

    for view in VIEWS {
        let row = sqlx::query(&format!("SELECT * FROM {view} LIMIT 1"))
            .fetch_optional(&pool)
            .await
            .unwrap_or_else(|err| panic!("{view} must resolve against the schema: {err}"));
        assert!(row.is_some(), "{view} returned no row for the seeded data");
    }
}

#[tokio::test]
async fn read_views_expose_no_secret_columns() {
    let pool = migrated_pool().await;

    for view in VIEWS {
        let columns = view_columns(&pool, view).await;
        assert!(!columns.is_empty(), "{view} is missing");
        for column in &columns {
            let lowered = column.to_ascii_lowercase();
            for pattern in SECRET_COLUMN_PATTERNS {
                assert!(
                    !lowered.contains(pattern),
                    "{view}.{column} looks like a secret ({pattern})"
                );
            }
        }
    }
}

#[tokio::test]
async fn v_messages_keeps_the_html_body_out() {
    let pool = migrated_pool().await;
    seed(&pool).await;

    let columns = view_columns(&pool, "v_messages").await;
    assert!(columns.contains(&"body_text".to_string()));
    assert!(
        !columns.contains(&"body_html".to_string()),
        "the sanitized HTML body is the largest column in the database and stays out"
    );

    let row = sqlx::query("SELECT body_text, has_body_html FROM v_messages WHERE id = 'msg'")
        .fetch_one(&pool)
        .await
        .expect("message row");
    assert_eq!(row.get::<String, _>("body_text"), "Anbei die Rechnung.");
    assert_eq!(row.get::<i64, _>("has_body_html"), 1);
}

/// The lookup docs/DB_READ_ACCESS.md gives for a `satchel://message/<id>` link.
#[tokio::test]
async fn v_messages_finds_a_linked_message_by_its_message_id() {
    let pool = migrated_pool().await;
    seed(&pool).await;

    let thread_id: String = sqlx::query_scalar(
        "SELECT thread_id FROM v_messages WHERE message_id_header = '<' || ? || '>'",
    )
    .bind("rechnung-1@app.io")
    .fetch_one(&pool)
    .await
    .expect("linked message");
    assert_eq!(thread_id, "thr");
}

#[tokio::test]
async fn v_threads_joins_tags_topics_and_participants() {
    let pool = migrated_pool().await;
    seed(&pool).await;

    let row = sqlx::query(
        "SELECT participants, tags, topic_ids, snoozed_until, first_message_date
         FROM v_threads WHERE id = 'thr'",
    )
    .fetch_one(&pool)
    .await
    .expect("thread row");

    // The stored JSON mixes `{name,email}` objects and bare address strings.
    assert_eq!(
        row.get::<String, _>("participants"),
        "sarah@app.io, bob@example.org"
    );
    assert_eq!(row.get::<String, _>("tags"), "Wichtig");
    assert_eq!(row.get::<String, _>("topic_ids"), "top");
    assert_eq!(
        row.get::<Option<String>, _>("snoozed_until"),
        None,
        "snoozed_until maps threads.snooze_until"
    );
    assert_eq!(
        row.get::<Option<String>, _>("first_message_date")
            .as_deref(),
        Some("2026-08-28T08:00:00Z")
    );
}

#[tokio::test]
async fn v_threads_survives_participants_that_are_not_json() {
    let pool = migrated_pool().await;
    seed(&pool).await;
    sqlx::query("UPDATE threads SET participants = 'sarah@app.io' WHERE id = 'thr'")
        .execute(&pool)
        .await
        .expect("legacy participants");

    // json_each() raises on malformed input; the view guards with json_valid()
    // so one bad row cannot make the whole view unreadable.
    let row = sqlx::query("SELECT participants FROM v_threads WHERE id = 'thr'")
        .fetch_one(&pool)
        .await
        .expect("thread row");
    assert_eq!(row.get::<Option<String>, _>("participants"), None);
}

#[tokio::test]
async fn v_attachments_reports_metadata_without_content() {
    let pool = migrated_pool().await;
    seed(&pool).await;

    let row = sqlx::query(
        "SELECT filename, content_type, size_bytes, is_downloaded, thread_id
         FROM v_attachments WHERE id = 'att'",
    )
    .fetch_one(&pool)
    .await
    .expect("attachment row");
    assert_eq!(row.get::<String, _>("filename"), "rechnung.pdf");
    assert_eq!(row.get::<i64, _>("size_bytes"), 4096);
    assert_eq!(row.get::<i64, _>("is_downloaded"), 0);
    assert_eq!(row.get::<String, _>("thread_id"), "thr");
}

/// The counters used to be handed straight out of `folders`, where they are set
/// to 0 once and never updated again.
#[tokio::test]
async fn v_folders_counters_match_the_messages_table() {
    let pool = migrated_pool().await;
    seed(&pool).await;
    for id in ["msg2", "msg3"] {
        sqlx::query(
            "INSERT INTO messages
               (id, account_id, folder_id, thread_id, from_address, to_addresses, subject,
                date, is_read, created_at, updated_at)
             VALUES (?, 'acc', 'fld', 'thr', 'sarah@app.io', '[\"me@example.com\"]', 'Re',
                '2026-08-28T09:00:00Z', 1, '2026-08-28T09:00:00Z', '2026-08-28T09:00:00Z')",
        )
        .bind(id)
        .execute(&pool)
        .await
        .expect("seed a read message");
    }

    let row = sqlx::query("SELECT message_count, unread_count FROM v_folders WHERE id = 'fld'")
        .fetch_one(&pool)
        .await
        .expect("folder row");
    assert_eq!(row.get::<i64, _>("message_count"), 3);
    assert_eq!(row.get::<i64, _>("unread_count"), 1);

    let counted = sqlx::query(
        "SELECT COUNT(1) AS total, SUM(COALESCE(is_read, 0) = 0) AS unread
         FROM messages WHERE folder_id = 'fld'",
    )
    .fetch_one(&pool)
    .await
    .expect("count over messages");
    assert_eq!(
        row.get::<i64, _>("message_count"),
        counted.get::<i64, _>("total")
    );
    assert_eq!(
        row.get::<i64, _>("unread_count"),
        counted.get::<i64, _>("unread")
    );
}
