use satchel_core::db::vacation::{VacationError, VacationRepository, VacationSettingsUpdate};
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

async fn seed_account(pool: &SqlitePool, id: &str) {
    sqlx::query(
        "INSERT INTO accounts
            (id, display_name, email, provider, config, credentials_ref,
             sync_state, color, is_active, created_at, updated_at)
         VALUES (?, ?, ?, 'imap', '{}', 'encrypted-in-config', '{}', '#123456', 1,
                 '2026-01-01', '2026-01-01')",
    )
    .bind(id)
    .bind(id)
    .bind(format!("{id}@example.test"))
    .execute(pool)
    .await
    .expect("account");
}

fn update(account_id: &str) -> VacationSettingsUpdate {
    VacationSettingsUpdate {
        account_id: account_id.into(),
        enabled: true,
        subject: Some("Away from my desk".into()),
        body: "I will reply next week.".into(),
        starts_at: Some("2026-09-01T00:00:00Z".into()),
        ends_at: Some("2026-09-08T00:00:00Z".into()),
        only_contacts: true,
    }
}

#[tokio::test]
async fn get_returns_none_when_an_account_has_no_setting() {
    let pool = pool().await;
    seed_account(&pool, "account-a").await;

    let settings = VacationRepository::new(&pool)
        .get("account-a")
        .await
        .expect("vacation settings");

    assert_eq!(settings, None);
}

#[tokio::test]
async fn save_returns_all_fields_and_converts_boolean_flags_to_sqlite_integers() {
    let pool = pool().await;
    seed_account(&pool, "account-a").await;

    let saved = VacationRepository::new(&pool)
        .save(update("account-a"))
        .await
        .expect("vacation settings");

    assert_eq!(saved.account_id, "account-a");
    assert_eq!(saved.enabled, 1);
    assert_eq!(saved.subject.as_deref(), Some("Away from my desk"));
    assert_eq!(saved.body, "I will reply next week.");
    assert_eq!(saved.starts_at.as_deref(), Some("2026-09-01T00:00:00Z"));
    assert_eq!(saved.ends_at.as_deref(), Some("2026-09-08T00:00:00Z"));
    assert_eq!(saved.only_contacts, 1);
    assert!(!saved.updated_at.is_empty());

    let disabled = VacationRepository::new(&pool)
        .save(VacationSettingsUpdate {
            account_id: "account-a".into(),
            enabled: false,
            subject: None,
            body: String::new(),
            starts_at: None,
            ends_at: None,
            only_contacts: false,
        })
        .await
        .expect("replacement settings");
    assert_eq!(disabled.enabled, 0);
    assert_eq!(disabled.subject, None);
    assert_eq!(disabled.body, "");
    assert_eq!(disabled.starts_at, None);
    assert_eq!(disabled.ends_at, None);
    assert_eq!(disabled.only_contacts, 0);
}

#[tokio::test]
async fn save_upserts_without_crossing_account_scope_and_replaces_timestamp() {
    let pool = pool().await;
    seed_account(&pool, "account-a").await;
    seed_account(&pool, "account-b").await;
    let repository = VacationRepository::new(&pool);

    let first = repository.save(update("account-a")).await.expect("first");
    sqlx::query("UPDATE vacation_settings SET updated_at = 'old-updated-at' WHERE account_id = ?")
        .bind(&first.account_id)
        .execute(&pool)
        .await
        .expect("old timestamp");

    let replacement = repository
        .save(VacationSettingsUpdate {
            account_id: "account-a".into(),
            enabled: false,
            subject: None,
            body: "Replacement".into(),
            starts_at: None,
            ends_at: None,
            only_contacts: false,
        })
        .await
        .expect("replacement");
    assert_eq!(replacement.account_id, "account-a");
    assert_eq!(replacement.body, "Replacement");
    assert_ne!(replacement.updated_at, "old-updated-at");

    let other = repository.save(update("account-b")).await.expect("other");
    assert_eq!(other.account_id, "account-b");
    assert_eq!(
        repository.get("account-a").await.unwrap().unwrap().body,
        "Replacement"
    );
    assert_eq!(
        repository.get("account-b").await.unwrap().unwrap().body,
        update("account-b").body
    );

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM vacation_settings")
        .fetch_one(&pool)
        .await
        .expect("settings count");
    assert_eq!(count, 2);
}

#[tokio::test]
async fn missing_schema_is_reported_as_a_typed_persistence_error() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory db");

    let error = VacationRepository::new(&pool)
        .get("missing-account")
        .await
        .expect_err("missing vacation table");
    assert!(matches!(error, VacationError::Persistence(_)));
}
