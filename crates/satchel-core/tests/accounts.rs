use satchel_core::db::accounts::{AccountError, AccountRepository};
use satchel_core::sync::queue::SyncQueue;
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

async fn seed_account(pool: &SqlitePool, id: &str, created_at: &str, active: bool, config: &str) {
    sqlx::query(
        "INSERT INTO accounts
            (id, display_name, email, provider, config, credentials_ref,
             sync_state, color, is_active, created_at, updated_at)
         VALUES (?, ?, ?, 'imap', ?, 'encrypted-in-config', '{}', '#123456', ?, ?, ?)",
    )
    .bind(id)
    .bind(format!("Name {id}"))
    .bind(format!("{id}@example.test"))
    .bind(config)
    .bind(if active { 1 } else { 0 })
    .bind(created_at)
    .bind(created_at)
    .execute(pool)
    .await
    .expect("account");
}

#[tokio::test]
async fn list_returns_only_active_accounts_in_creation_order() {
    let pool = pool().await;
    seed_account(&pool, "acc-later", "2026-02-01", true, "{}").await;
    seed_account(&pool, "acc-inactive", "2026-01-01", false, "{}").await;
    seed_account(&pool, "acc-first", "2025-12-01", true, "{}").await;

    let accounts = AccountRepository::new(&pool)
        .list_active()
        .await
        .expect("accounts");

    assert_eq!(
        accounts
            .iter()
            .map(|account| account.id.as_str())
            .collect::<Vec<_>>(),
        ["acc-first", "acc-later"]
    );
    assert!(accounts.iter().all(|account| account.kind == "email"));
}

#[tokio::test]
async fn clear_oauth_error_clears_both_fields() {
    let pool = pool().await;
    seed_account(&pool, "acc-oauth", "2026-01-01", true, "{}").await;
    sqlx::query(
        "UPDATE accounts
         SET last_oauth_error = 'refresh failed', last_oauth_error_at = '2026-02-01'
         WHERE id = 'acc-oauth'",
    )
    .execute(&pool)
    .await
    .expect("oauth error");

    AccountRepository::new(&pool)
        .clear_oauth_error("acc-oauth")
        .await
        .expect("clear oauth error");

    let fields: (Option<String>, Option<String>) = sqlx::query_as(
        "SELECT last_oauth_error, last_oauth_error_at FROM accounts WHERE id = 'acc-oauth'",
    )
    .fetch_one(&pool)
    .await
    .expect("fields");
    assert_eq!(fields, (None, None));
}

#[tokio::test]
async fn deactivate_changes_the_account_and_reports_missing_ids() {
    let pool = pool().await;
    seed_account(&pool, "acc-active", "2026-01-01", true, "{}").await;
    let repository = AccountRepository::new(&pool);

    repository
        .deactivate("acc-active")
        .await
        .expect("deactivate");
    let active: i64 = sqlx::query_scalar("SELECT is_active FROM accounts WHERE id = 'acc-active'")
        .fetch_one(&pool)
        .await
        .expect("active flag");
    assert_eq!(active, 0);

    assert!(matches!(
        repository.deactivate("missing-account").await,
        Err(AccountError::NotFound)
    ));
}

/// A plaintext config row was not written by Satchel: loading it is refused
/// (SEC-10), and inactive or missing accounts stay hidden. The decrypting path
/// itself is covered by the repository's unit tests, which can seed a key.
#[tokio::test]
async fn connection_load_refuses_plaintext_and_hides_other_rows() {
    let pool = pool().await;
    seed_account(
        &pool,
        "acc-active",
        "2026-01-01",
        true,
        r#"{"imap":{"host":"mail.example.test"}}"#,
    )
    .await;
    seed_account(&pool, "acc-inactive", "2026-01-02", false, "{}").await;
    let repository = AccountRepository::new(&pool);

    assert!(matches!(
        repository.load_active_imap_connection("acc-active").await,
        Err(AccountError::CredentialAccess { detail })
            if detail == satchel_core::accounts::ACCOUNT_CONFIG_NOT_ENCRYPTED
    ));

    for id in ["acc-inactive", "missing-account"] {
        assert!(matches!(
            repository.load_active_imap_connection(id).await,
            Err(AccountError::NotFound)
        ));
    }
}

#[tokio::test]
async fn non_imap_accounts_are_rejected_before_credentials_are_decrypted() {
    let pool = pool().await;
    seed_account(
        &pool,
        "acc-gmail",
        "2026-01-01",
        true,
        "enc:v1:not-valid-ciphertext",
    )
    .await;
    sqlx::query("UPDATE accounts SET provider = 'gmail' WHERE id = 'acc-gmail'")
        .execute(&pool)
        .await
        .expect("provider");

    assert!(matches!(
        AccountRepository::new(&pool)
            .load_active_imap_connection("acc-gmail")
            .await,
        Err(AccountError::NotImap)
    ));
}

#[tokio::test]
async fn queue_purge_removes_only_actions_for_the_deleted_account() {
    let pool = pool().await;
    for (id, account_id) in [("action-one", "acc-one"), ("action-two", "acc-two")] {
        sqlx::query(
            "INSERT INTO sync_queue
                (id, account_id, action_type, payload, status, retry_count, created_at, updated_at)
             VALUES (?, ?, 'remote_mutation', '{}', 'pending', 0, '2026-01-01', '2026-01-01')",
        )
        .bind(id)
        .bind(account_id)
        .execute(&pool)
        .await
        .expect("queued action");
    }

    let removed = SyncQueue::new()
        .purge_account(&pool, "acc-one")
        .await
        .expect("purge account");
    assert_eq!(removed, 1);
    let remaining: Vec<String> =
        sqlx::query_scalar("SELECT account_id FROM sync_queue ORDER BY account_id")
            .fetch_all(&pool)
            .await
            .expect("remaining queue");
    assert_eq!(remaining, ["acc-two"]);
}

#[tokio::test]
async fn missing_table_is_a_typed_persistence_error() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory db");
    let error = AccountRepository::new(&pool)
        .list_active()
        .await
        .expect_err("missing accounts table");
    assert!(matches!(error, AccountError::Persistence(_)));
}

#[tokio::test]
async fn paused_account_keeps_queue_and_resumes_on_reactivation() {
    use satchel_core::sync::queue::QueuedAction;
    let pool = pool().await;
    seed_account(&pool, "paused", "2026-01-01", true, "{}").await;
    let queue = SyncQueue::new();
    let pending = QueuedAction::new("paused".into(), "send_message", serde_json::json!({}));
    let unknown = QueuedAction::new("paused".into(), "send_message", serde_json::json!({}));
    let scheduled = QueuedAction::new("paused".into(), "send_message", serde_json::json!({}))
        .scheduled(chrono::Utc::now() + chrono::Duration::days(1));
    for action in [&pending, &unknown, &scheduled] {
        queue.enqueue(&pool, action.clone()).await.unwrap();
    }
    queue
        .mark_submission_started(&pool, &unknown.id)
        .await
        .unwrap();
    queue
        .mark_delivery_unknown(&pool, &unknown.id)
        .await
        .unwrap();
    AccountRepository::new(&pool)
        .deactivate("paused")
        .await
        .unwrap();
    assert!(queue.ready_account_heads(&pool).await.unwrap().is_empty());
    assert!(queue.peek_next_ready(&pool).await.unwrap().is_none());
    assert!(queue
        .claim_ready(&pool, &pending.id)
        .await
        .unwrap()
        .is_none());
    assert_eq!(queue.list_mail_outbox(&pool).await.unwrap().len(), 3);
    sqlx::query("UPDATE accounts SET is_active = 1 WHERE id = 'paused'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        queue.ready_account_heads(&pool).await.unwrap()[0].id,
        pending.id
    );
    assert!(queue
        .claim_ready(&pool, &pending.id)
        .await
        .unwrap()
        .is_some());
    queue.purge_account(&pool, "paused").await.unwrap();
    AccountRepository::new(&pool)
        .delete("paused")
        .await
        .unwrap();
    assert!(queue.list_mail_outbox(&pool).await.unwrap().is_empty());
}
