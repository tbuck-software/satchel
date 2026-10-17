use satchel_core::db::connection_state::{
    ConnectionStateError, ConnectionStateRepository, ConnectionStateSnapshot,
};
use satchel_core::sync::{
    queue::{ACTION_SEND_MESSAGE, ACTION_SYNC_DRAFT},
    remote_mutations::ACTION_REMOTE_MUTATION,
};
use sqlx::sqlite::SqlitePoolOptions;
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

async fn seed_account(pool: &SqlitePool, id: &str, last_sync_at: Option<&str>) {
    sqlx::query(
        "INSERT INTO accounts
            (id, display_name, email, provider, config, credentials_ref,
             sync_state, color, is_active, created_at, updated_at)
         VALUES (?, ?, ?, 'imap', '{}', 'encrypted-in-config', '{}', '#123456', 1, ?, ?)",
    )
    .bind(id)
    .bind(format!("Name {id}"))
    .bind(format!("{id}@example.test"))
    .bind("2026-01-01")
    .bind("2026-01-01")
    .execute(pool)
    .await
    .expect("account");

    if let Some(last_sync_at) = last_sync_at {
        sqlx::query("UPDATE accounts SET last_sync_at = ? WHERE id = ?")
            .bind(last_sync_at)
            .bind(id)
            .execute(pool)
            .await
            .expect("last sync");
    }
}

async fn queue_row(pool: &SqlitePool, id: &str, action_type: &str, status: &str) {
    sqlx::query(
        "INSERT INTO sync_queue
            (id, account_id, action_type, payload, status, retry_count, created_at, updated_at)
         VALUES (?, 'acc-one', ?, '{}', ?, 0, '2026-01-01', '2026-01-01')",
    )
    .bind(id)
    .bind(action_type)
    .bind(status)
    .execute(pool)
    .await
    .expect("queue row");
}

#[tokio::test]
async fn snapshot_preserves_queue_status_and_action_type_semantics() {
    let pool = pool().await;
    seed_account(&pool, "acc-one", Some("2026-01-02T12:00:00Z")).await;
    seed_account(&pool, "acc-two", Some("2026-01-03T12:00:00Z")).await;

    queue_row(&pool, "pending-mutation", ACTION_REMOTE_MUTATION, "pending").await;
    queue_row(&pool, "scheduled-send", ACTION_SEND_MESSAGE, "scheduled").await;
    queue_row(
        &pool,
        "notification:quick-action",
        ACTION_REMOTE_MUTATION,
        "sending",
    )
    .await;
    queue_row(&pool, "failed-send", ACTION_SEND_MESSAGE, "failed").await;
    queue_row(&pool, "delivered-send", ACTION_SEND_MESSAGE, "delivered").await;
    queue_row(
        &pool,
        "unknown-mutation",
        ACTION_REMOTE_MUTATION,
        "delivery_unknown",
    )
    .await;
    queue_row(&pool, "failed-mutation", ACTION_REMOTE_MUTATION, "failed").await;
    queue_row(&pool, "failed-draft", ACTION_SYNC_DRAFT, "failed").await;
    queue_row(
        &pool,
        "notification:failed-action",
        ACTION_REMOTE_MUTATION,
        "failed",
    )
    .await;

    let snapshot = ConnectionStateRepository::new(&pool)
        .snapshot()
        .await
        .expect("connection state");

    assert_eq!(
        snapshot,
        ConnectionStateSnapshot {
            last_sync_at: Some("2026-01-03T12:00:00Z".into()),
            pending_outbox: 3,
            pending_sends: 1,
            failed_outbox: 4,
            failed_sends: 2,
            failed_remote_mutations: 4,
        }
    );
}

#[tokio::test]
async fn snapshot_returns_a_typed_persistence_error_when_schema_is_missing() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory db");

    let error = ConnectionStateRepository::new(&pool)
        .snapshot()
        .await
        .expect_err("missing schema");
    assert!(matches!(error, ConnectionStateError::Persistence(_)));
}
