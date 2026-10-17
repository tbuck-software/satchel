use std::time::Duration;

use satchel_core::db::followups::{FollowUpError, FollowUpRepository, FollowUpUpsertRequest};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;

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

async fn file_pool(path: &std::path::Path) -> SqlitePool {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(15));
    SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await
        .expect("file db")
}

async fn seed(pool: &SqlitePool) {
    sqlx::query(
        "INSERT INTO accounts
            (id, display_name, email, provider, config, credentials_ref,
             is_active, created_at, updated_at)
         VALUES ('account-1', 'Me', 'me@example.test', 'imap', '{}', '', 1,
                 '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
                ('account-2', 'Other', 'other@example.test', 'imap', '{}', '', 1,
                 '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
    )
    .execute(pool)
    .await
    .expect("accounts");
    sqlx::query(
        "INSERT INTO folders
            (id, account_id, name, full_path, folder_type, remote_id,
             created_at, updated_at)
         VALUES ('folder-1', 'account-1', 'Sent', 'Sent', 'sent', 'sent',
                 '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
                ('folder-2', 'account-2', 'Sent', 'Sent', 'sent', 'sent',
                 '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
    )
    .execute(pool)
    .await
    .expect("folders");
    for (id, account_id, subject, snippet) in [
        ("thread-1", "account-1", "First subject", "First snippet"),
        ("thread-2", "account-1", "Second subject", "Second snippet"),
        ("thread-3", "account-2", "Third subject", "Third snippet"),
    ] {
        sqlx::query(
            "INSERT INTO threads
                (id, account_id, subject, last_message_date, snippet,
                 created_at, updated_at)
             VALUES (?, ?, ?, '2026-01-01T00:00:00Z', ?,
                     '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(id)
        .bind(account_id)
        .bind(subject)
        .bind(snippet)
        .execute(pool)
        .await
        .expect("thread");
    }
    sqlx::query(
        "INSERT INTO messages
            (id, account_id, folder_id, thread_id, from_address,
             to_addresses, subject, date, created_at, updated_at)
         VALUES ('message-1', 'account-1', 'folder-1', 'thread-1',
                 'me@example.test', '[]', 'First subject',
                 '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z',
                 '2026-01-01T00:00:00Z')",
    )
    .execute(pool)
    .await
    .expect("message");
}

fn request(id: Option<&str>, thread_id: &str, due_at: &str) -> FollowUpUpsertRequest {
    FollowUpUpsertRequest {
        id: id.map(str::to_owned),
        thread_id: thread_id.to_owned(),
        due_at: due_at.to_owned(),
        note: Some("note".into()),
    }
}

#[tokio::test]
async fn list_defaults_to_pending_and_filters_in_due_order() {
    let pool = pool().await;
    seed(&pool).await;
    let repository = FollowUpRepository::new(&pool);
    for (id, due_at) in [
        ("follow-later", "2026-09-03T00:00:00Z"),
        ("follow-earlier", "2026-09-01T00:00:00Z"),
    ] {
        repository
            .upsert(request(Some(id), "thread-1", due_at))
            .await
            .expect("follow-up");
    }
    let pending = repository.list(None).await.expect("pending list");
    assert_eq!(
        pending
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        ["follow-earlier", "follow-later"]
    );

    sqlx::query("UPDATE follow_up_reminders SET status = 'due' WHERE id = 'follow-earlier'")
        .execute(&pool)
        .await
        .expect("status");
    assert_eq!(
        repository.list(Some("due")).await.expect("due list").len(),
        1
    );
}

#[tokio::test]
async fn upsert_uses_thread_account_and_conflicts_keep_association() {
    let pool = pool().await;
    seed(&pool).await;
    let repository = FollowUpRepository::new(&pool);
    repository
        .upsert(request(
            Some("follow-fixed"),
            "thread-1",
            "2026-09-01T00:00:00+02:00",
        ))
        .await
        .expect("first upsert");
    sqlx::query(
        "UPDATE follow_up_reminders SET message_id = 'message-1' WHERE id = 'follow-fixed'",
    )
    .execute(&pool)
    .await
    .expect("message association");

    let updated = repository
        .upsert(request(
            Some("follow-fixed"),
            "thread-2",
            "2026-09-02T00:00:00Z",
        ))
        .await
        .expect("conflicting upsert");
    assert_eq!(updated.account_id, "account-1");
    assert_eq!(updated.thread_id, "thread-1");
    assert_eq!(updated.message_id.as_deref(), Some("message-1"));
    assert_eq!(updated.due_at, "2026-09-02T00:00:00+00:00");
}

#[tokio::test]
async fn an_explicit_empty_id_is_not_replaced_with_a_generated_id() {
    let pool = pool().await;
    seed(&pool).await;
    let record = FollowUpRepository::new(&pool)
        .upsert(request(Some(""), "thread-1", "2026-09-01T00:00:00Z"))
        .await
        .expect("empty id is a valid explicit id");
    assert_eq!(record.id, "");
}

#[tokio::test]
async fn invalid_timestamp_and_missing_thread_do_not_write() {
    let pool = pool().await;
    seed(&pool).await;
    let repository = FollowUpRepository::new(&pool);
    let error = repository
        .upsert(request(Some("follow-invalid"), "thread-1", "tomorrow"))
        .await
        .expect_err("invalid timestamp");
    assert!(matches!(error, FollowUpError::InvalidDueAt));
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM follow_up_reminders WHERE id = 'follow-invalid'")
            .fetch_one(&pool)
            .await
            .expect("count");
    assert_eq!(count, 0);

    let error = repository
        .upsert(request(
            Some("follow-missing"),
            "missing-thread",
            "2026-09-01T00:00:00Z",
        ))
        .await
        .expect_err("missing thread");
    assert!(matches!(error, FollowUpError::ThreadNotFound));
}

#[tokio::test]
async fn cancel_sets_resolution_and_missing_ids_are_noop() {
    let pool = pool().await;
    seed(&pool).await;
    let repository = FollowUpRepository::new(&pool);
    repository
        .upsert(request(
            Some("follow-cancel"),
            "thread-1",
            "2026-09-01T00:00:00Z",
        ))
        .await
        .expect("follow-up");
    repository.cancel("follow-cancel").await.expect("cancel");
    repository.cancel("missing-follow-up").await.expect("no-op");
    let row: (String, Option<String>) = sqlx::query_as(
        "SELECT status, resolved_at FROM follow_up_reminders WHERE id = 'follow-cancel'",
    )
    .fetch_one(&pool)
    .await
    .expect("cancelled row");
    assert_eq!(row.0, "cancelled");
    assert!(row.1.is_some());
}

#[tokio::test]
async fn sent_creation_keeps_exact_association_and_none_is_a_noop() {
    let pool = pool().await;
    seed(&pool).await;
    let repository = FollowUpRepository::new(&pool);
    repository
        .create_for_sent_message("account-1", "thread-1", "message-1", None)
        .await
        .expect("none is a no-op");
    assert_eq!(repository.list(None).await.expect("empty list").len(), 0);
    repository
        .create_for_sent_message(
            "account-1",
            "thread-1",
            "message-1",
            Some("2026-09-01T00:00:00+02:00"),
        )
        .await
        .expect("sent follow-up");
    let record = repository
        .list(None)
        .await
        .expect("list")
        .pop()
        .expect("record");
    assert_eq!(record.account_id, "account-1");
    assert_eq!(record.thread_id, "thread-1");
    assert_eq!(record.message_id.as_deref(), Some("message-1"));
    assert_eq!(record.due_at, "2026-08-31T22:00:00+00:00");
}

#[tokio::test]
async fn only_external_replies_resolve_pending_reminders() {
    let pool = pool().await;
    seed(&pool).await;
    let repository = FollowUpRepository::new(&pool);
    for id in ["follow-pending", "follow-due", "follow-resolved"] {
        repository
            .upsert(request(Some(id), "thread-1", "2026-09-01T00:00:00Z"))
            .await
            .expect("follow-up");
    }
    sqlx::query(
        "UPDATE follow_up_reminders
         SET status = CASE id WHEN 'follow-due' THEN 'due' ELSE 'resolved' END,
             resolved_at = CASE id WHEN 'follow-resolved' THEN '2026-08-01T00:00:00Z' END
         WHERE id IN ('follow-due', 'follow-resolved')",
    )
    .execute(&pool)
    .await
    .expect("statuses");
    repository
        .resolve_for_thread_reply("thread-1", "me@example.test", "me@example.test")
        .await
        .expect("own reply");
    assert_eq!(repository.list(None).await.expect("pending").len(), 1);
    repository
        .resolve_for_thread_reply("thread-1", "client@example.test", "me@example.test")
        .await
        .expect("external reply");
    let statuses: Vec<(String, String)> =
        sqlx::query_as("SELECT id, status FROM follow_up_reminders ORDER BY id")
            .fetch_all(&pool)
            .await
            .expect("statuses");
    assert_eq!(
        statuses,
        vec![
            ("follow-due".into(), "due".into()),
            ("follow-pending".into(), "resolved".into()),
            ("follow-resolved".into(), "resolved".into()),
        ]
    );
}

#[tokio::test]
async fn whitespace_is_not_trimmed_when_identifying_an_own_reply() {
    let pool = pool().await;
    seed(&pool).await;
    let repository = FollowUpRepository::new(&pool);
    repository
        .upsert(request(
            Some("follow-whitespace"),
            "thread-1",
            "2026-09-01T00:00:00Z",
        ))
        .await
        .expect("follow-up");
    repository
        .resolve_for_thread_reply("thread-1", " me@example.test ", "me@example.test")
        .await
        .expect("reply resolution");
    let status: String =
        sqlx::query_scalar("SELECT status FROM follow_up_reminders WHERE id = 'follow-whitespace'")
            .fetch_one(&pool)
            .await
            .expect("status");
    assert_eq!(status, "resolved");
}

#[tokio::test]
async fn due_claim_is_limited_ordered_and_once_only() {
    let pool = pool().await;
    seed(&pool).await;
    let repository = FollowUpRepository::new(&pool);
    for index in 0..26 {
        repository
            .upsert(request(
                Some(&format!("follow-{index:02}")),
                "thread-1",
                &format!("2026-08-{:02}T00:00:00Z", index + 1),
            ))
            .await
            .expect("follow-up");
    }
    let first = repository.claim_due().await.expect("first claim");
    assert_eq!(first.len(), 25);
    assert_eq!(first[0].due_at, "2026-08-01T00:00:00+00:00");
    assert_eq!(first[0].subject.as_deref(), Some("First subject"));
    assert_eq!(first[0].snippet.as_deref(), Some("First snippet"));
    let second = repository.claim_due().await.expect("second claim");
    assert_eq!(second.len(), 1);
    assert!(repository
        .claim_due()
        .await
        .expect("third claim")
        .is_empty());
    let due_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM follow_up_reminders WHERE status = 'due'")
            .fetch_one(&pool)
            .await
            .expect("due count");
    assert_eq!(due_count, 26);
}

#[tokio::test]
async fn concurrent_claims_never_return_the_same_reminder() {
    let temporary = tempfile::tempdir().expect("temporary directory");
    let path = temporary.path().join("followups.sqlite");
    let first_pool = file_pool(&path).await;
    sqlx::migrate!("./src/db/migrations")
        .run(&first_pool)
        .await
        .expect("migrations");
    seed(&first_pool).await;
    let repository = FollowUpRepository::new(&first_pool);
    for index in 0..4 {
        repository
            .upsert(request(
                Some(&format!("follow-concurrent-{index}")),
                "thread-1",
                "2026-08-01T00:00:00Z",
            ))
            .await
            .expect("follow-up");
    }
    let second_pool = file_pool(&path).await;
    let first_repository = FollowUpRepository::new(&first_pool);
    let second_repository = FollowUpRepository::new(&second_pool);
    let (first, second) = tokio::join!(first_repository.claim_due(), second_repository.claim_due());
    let first = first.expect("first concurrent claim");
    let second = second.expect("second concurrent claim");
    let first_ids = first
        .iter()
        .map(|item| item.id.as_str())
        .collect::<Vec<_>>();
    assert!(second
        .iter()
        .all(|item| !first_ids.contains(&item.id.as_str())));
    assert_eq!(first.len() + second.len(), 4);
}

#[tokio::test]
async fn missing_schema_is_reported_as_typed_persistence_error() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory db");
    let error = FollowUpRepository::new(&pool)
        .list(None)
        .await
        .expect_err("missing schema");
    assert!(matches!(error, FollowUpError::Persistence(_)));
}

#[tokio::test]
async fn sent_none_is_a_noop_even_without_the_follow_up_schema() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory db");
    FollowUpRepository::new(&pool)
        .create_for_sent_message("account", "thread", "message", None)
        .await
        .expect("none must not touch schema");
}
