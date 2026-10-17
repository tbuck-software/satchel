use satchel_core::db::webhooks::{WebhookError, WebhookRepository, WebhookUpsertRequest};
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::SqlitePool;

async fn pool() -> SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory db");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("foreign keys");
    sqlx::migrate!("./src/db/migrations")
        .run(&pool)
        .await
        .expect("migrations");
    pool
}

async fn insert_webhook(
    pool: &SqlitePool,
    id: &str,
    url: &str,
    events_json: &str,
    enabled: i64,
    secret: Option<&str>,
    created_at: &str,
) {
    sqlx::query(
        "INSERT INTO local_webhooks
            (id, url, events_json, enabled, secret, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(url)
    .bind(events_json)
    .bind(enabled)
    .bind(secret)
    .bind(created_at)
    .bind(created_at)
    .execute(pool)
    .await
    .expect("webhook");
}

async fn insert_delivery(
    pool: &SqlitePool,
    id: &str,
    webhook_id: &str,
    attempt_count: i64,
    status: &str,
    next_attempt_at: &str,
    created_at: &str,
) {
    sqlx::query(
        "INSERT INTO webhook_deliveries
            (id, webhook_id, event_name, idempotency_key, payload_json,
             status, attempt_count, next_attempt_at, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(webhook_id)
    .bind("message.created")
    .bind(format!("key-{id}"))
    .bind("{\"id\":1}")
    .bind(status)
    .bind(attempt_count)
    .bind(next_attempt_at)
    .bind(created_at)
    .bind(created_at)
    .execute(pool)
    .await
    .expect("delivery");
}

fn request(
    id: Option<&str>,
    url: &str,
    events: Vec<&str>,
    enabled: bool,
    secret: Option<&str>,
) -> WebhookUpsertRequest {
    WebhookUpsertRequest {
        id: id.map(str::to_owned),
        url: url.to_owned(),
        events: events.into_iter().map(str::to_owned).collect(),
        enabled,
        secret: secret.map(str::to_owned),
    }
}

#[tokio::test]
async fn list_and_upsert_preserve_sorting_id_and_timestamp_semantics() {
    let pool = pool().await;
    insert_webhook(
        &pool,
        "wh-old",
        "https://old.example.test",
        "[]",
        1,
        None,
        "2026-01-01T00:00:00Z",
    )
    .await;
    insert_webhook(
        &pool,
        "wh-new",
        "https://new.example.test",
        "[]",
        0,
        None,
        "2026-02-01T00:00:00Z",
    )
    .await;
    let repository = WebhookRepository::new(&pool);

    let listed = repository.list().await.expect("webhook list");
    assert_eq!(
        listed
            .iter()
            .map(|webhook| webhook.id.as_str())
            .collect::<Vec<_>>(),
        ["wh-new", "wh-old"]
    );

    let created = repository
        .upsert(request(
            Some(""),
            "https://generated.example.test",
            vec!["message.created"],
            true,
            Some(""),
        ))
        .await
        .expect("generated webhook");
    assert!(created.id.starts_with("wh-"));
    assert_eq!(created.events_json, "[\"message.created\"]");
    assert_eq!(created.enabled, 1);
    let stored_secret: Option<String> =
        sqlx::query_scalar("SELECT secret FROM local_webhooks WHERE id = ?")
            .bind(&created.id)
            .fetch_one(&pool)
            .await
            .expect("secret");
    assert_eq!(
        stored_secret, None,
        "an explicitly empty secret is SQL NULL"
    );

    sqlx::query("UPDATE local_webhooks SET created_at = 'old-created-at' WHERE id = ?")
        .bind(&created.id)
        .execute(&pool)
        .await
        .expect("created timestamp");
    let updated = repository
        .upsert(request(
            Some(&created.id),
            "https://updated.example.test",
            vec!["message.updated"],
            false,
            Some("secret-value"),
        ))
        .await
        .expect("updated webhook");
    assert_eq!(updated.id, created.id);
    assert_eq!(updated.url, "https://updated.example.test");
    assert_eq!(updated.enabled, 0);
    assert_eq!(updated.created_at, "old-created-at");
    assert_ne!(updated.updated_at, "old-created-at");
}

#[tokio::test]
async fn enqueue_filters_enabled_subscriptions_and_skips_invalid_event_json() {
    let pool = pool().await;
    insert_webhook(
        &pool,
        "wh-match",
        "https://match.example.test",
        "[\"message.created\"]",
        1,
        None,
        "2026-01-01",
    )
    .await;
    insert_webhook(
        &pool,
        "wh-other-event",
        "https://other.example.test",
        "[\"message.deleted\"]",
        1,
        None,
        "2026-01-02",
    )
    .await;
    insert_webhook(
        &pool,
        "wh-disabled",
        "https://disabled.example.test",
        "[\"message.created\"]",
        0,
        None,
        "2026-01-03",
    )
    .await;
    insert_webhook(
        &pool,
        "wh-invalid-json",
        "https://invalid.example.test",
        "not-json",
        1,
        None,
        "2026-01-04",
    )
    .await;

    WebhookRepository::new(&pool)
        .enqueue_deliveries(
            "message.created",
            "event-1",
            &serde_json::json!({ "thread_id": "thread-1" }),
        )
        .await
        .expect("fan-out");

    let rows: Vec<(String, String, String, i64)> = sqlx::query_as(
        "SELECT webhook_id, event_name, idempotency_key, attempt_count
         FROM webhook_deliveries",
    )
    .fetch_all(&pool)
    .await
    .expect("deliveries");
    assert_eq!(
        rows,
        vec![(
            "wh-match".into(),
            "message.created".into(),
            "event-1".into(),
            0
        )]
    );
}

#[tokio::test]
async fn enqueue_fanout_is_atomic_when_one_delivery_insert_fails() {
    let pool = pool().await;
    insert_webhook(
        &pool,
        "wh-first",
        "https://first.example.test",
        "[\"message.created\"]",
        1,
        None,
        "2026-01-01",
    )
    .await;
    insert_webhook(
        &pool,
        "wh-fails",
        "https://fails.example.test",
        "[\"message.created\"]",
        1,
        None,
        "2026-01-02",
    )
    .await;
    sqlx::query(
        "CREATE TRIGGER fail_webhook_delivery_insert
         BEFORE INSERT ON webhook_deliveries
         WHEN NEW.webhook_id = 'wh-fails'
         BEGIN SELECT RAISE(ABORT, 'intentional fan-out failure'); END",
    )
    .execute(&pool)
    .await
    .expect("trigger");

    let error = WebhookRepository::new(&pool)
        .enqueue_deliveries("message.created", "event-atomic", &serde_json::json!({}))
        .await
        .expect_err("fan-out failure");
    assert!(matches!(error, WebhookError::Persistence(_)));
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM webhook_deliveries")
        .fetch_one(&pool)
        .await
        .expect("delivery count");
    assert_eq!(count, 0, "the transaction rolls back prior fan-out inserts");
}

#[tokio::test]
async fn delete_cascades_deliveries_with_foreign_keys_enabled() {
    let pool = pool().await;
    let foreign_keys: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
        .fetch_one(&pool)
        .await
        .expect("foreign keys pragma");
    assert_eq!(foreign_keys, 1);
    insert_webhook(
        &pool,
        "wh-cascade",
        "https://cascade.example.test",
        "[]",
        1,
        None,
        "2026-01-01",
    )
    .await;
    insert_delivery(
        &pool,
        "delivery-cascade",
        "wh-cascade",
        0,
        "pending",
        "2020-01-01",
        "2020-01-01",
    )
    .await;

    WebhookRepository::new(&pool)
        .delete("wh-cascade")
        .await
        .expect("delete webhook");
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM webhook_deliveries WHERE webhook_id = 'wh-cascade'",
    )
    .fetch_one(&pool)
    .await
    .expect("cascade count");
    assert_eq!(count, 0);
}

#[tokio::test]
async fn load_due_filters_enabled_state_orders_and_limits_to_25() {
    let pool = pool().await;
    insert_webhook(
        &pool,
        "wh-due",
        "https://due.example.test/hook",
        "[]",
        1,
        Some("delivery-secret"),
        "2026-01-01",
    )
    .await;
    insert_webhook(
        &pool,
        "wh-disabled-due",
        "https://disabled.example.test/hook",
        "[]",
        0,
        Some("disabled-secret"),
        "2026-01-01",
    )
    .await;

    for index in 0..26 {
        let id = format!("delivery-{index:02}");
        let created_at = format!("2020-01-01T00:00:{index:02}Z");
        insert_delivery(
            &pool,
            &id,
            "wh-due",
            0,
            "pending",
            "2020-01-01",
            &created_at,
        )
        .await;
    }
    insert_delivery(
        &pool,
        "delivery-disabled",
        "wh-disabled-due",
        0,
        "pending",
        "2020-01-01",
        "2020-01-01",
    )
    .await;
    insert_delivery(
        &pool,
        "delivery-delivered",
        "wh-due",
        0,
        "delivered",
        "2020-01-01",
        "2020-01-01",
    )
    .await;
    insert_delivery(
        &pool,
        "delivery-future",
        "wh-due",
        0,
        "pending",
        "2999-01-01",
        "2020-01-01",
    )
    .await;

    let due = WebhookRepository::new(&pool)
        .load_due()
        .await
        .expect("due deliveries");
    assert_eq!(due.len(), 25);
    assert_eq!(
        due.first().map(|delivery| delivery.id.as_str()),
        Some("delivery-00")
    );
    assert_eq!(
        due.last().map(|delivery| delivery.id.as_str()),
        Some("delivery-24")
    );
    assert!(due.iter().all(|delivery| {
        delivery.url == "https://due.example.test/hook"
            && delivery.secret.as_deref() == Some("delivery-secret")
    }));
}

#[tokio::test]
async fn delivered_and_retry_updates_preserve_attempt_and_backoff_semantics() {
    let pool = pool().await;
    insert_webhook(
        &pool,
        "wh-state",
        "https://state.example.test",
        "[]",
        1,
        None,
        "2026-01-01",
    )
    .await;
    insert_delivery(
        &pool,
        "delivery-success",
        "wh-state",
        2,
        "pending",
        "2020-01-01",
        "2020-01-01",
    )
    .await;
    insert_delivery(
        &pool,
        "delivery-retry",
        "wh-state",
        1,
        "pending",
        "2020-01-01",
        "2020-01-01",
    )
    .await;
    insert_delivery(
        &pool,
        "delivery-failed",
        "wh-state",
        4,
        "pending",
        "2020-01-01",
        "2020-01-01",
    )
    .await;
    let repository = WebhookRepository::new(&pool);

    repository
        .mark_delivered("delivery-success")
        .await
        .expect("mark delivered");
    let success: (String, i64) = sqlx::query_as(
        "SELECT status, attempt_count FROM webhook_deliveries WHERE id = 'delivery-success'",
    )
    .fetch_one(&pool)
    .await
    .expect("success state");
    assert_eq!(success, ("delivered".into(), 3));

    repository
        .schedule_retry("delivery-retry", 1, "HTTP 503")
        .await
        .expect("schedule retry");
    let retry: (String, i64, String, String, String) = sqlx::query_as(
        "SELECT status, attempt_count, next_attempt_at, last_error, updated_at
         FROM webhook_deliveries WHERE id = 'delivery-retry'",
    )
    .fetch_one(&pool)
    .await
    .expect("retry state");
    assert_eq!(retry.0, "pending");
    assert_eq!(retry.1, 2);
    assert_eq!(retry.3, "HTTP 503");
    let retry_at = chrono::DateTime::parse_from_rfc3339(&retry.2).expect("retry timestamp");
    let updated_at = chrono::DateTime::parse_from_rfc3339(&retry.4).expect("updated timestamp");
    assert_eq!(retry_at - updated_at, chrono::Duration::seconds(120));

    repository
        .schedule_retry("delivery-failed", 4, "connection refused")
        .await
        .expect("schedule final retry");
    let failed: (String, i64, String, String) = sqlx::query_as(
        "SELECT status, attempt_count, next_attempt_at, updated_at
         FROM webhook_deliveries WHERE id = 'delivery-failed'",
    )
    .fetch_one(&pool)
    .await
    .expect("failed state");
    assert_eq!(failed.0, "failed");
    assert_eq!(failed.1, 5);
    let failed_next_at = chrono::DateTime::parse_from_rfc3339(&failed.2).expect("failed timestamp");
    let failed_updated_at =
        chrono::DateTime::parse_from_rfc3339(&failed.3).expect("failed updated timestamp");
    assert_eq!(
        failed_next_at - failed_updated_at,
        chrono::Duration::seconds(960)
    );
}

#[tokio::test]
async fn missing_schema_is_reported_as_a_typed_persistence_error() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory db");

    let error = WebhookRepository::new(&pool)
        .list()
        .await
        .expect_err("missing webhook table");
    assert!(matches!(error, WebhookError::Persistence(_)));
}
