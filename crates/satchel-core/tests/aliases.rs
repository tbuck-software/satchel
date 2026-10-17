use satchel_core::db::aliases::{AliasError, AliasRepository, AliasUpsertRequest};
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

fn request(
    id: Option<&str>,
    account_id: &str,
    email: &str,
    display_name: Option<&str>,
    is_default: bool,
) -> AliasUpsertRequest {
    AliasUpsertRequest {
        id: id.map(str::to_owned),
        account_id: account_id.to_owned(),
        email: email.to_owned(),
        display_name: display_name.map(str::to_owned),
        is_default,
    }
}

#[tokio::test]
async fn list_supports_account_filter_and_preserves_default_then_email_sorting() {
    let pool = pool().await;
    seed_account(&pool, "account-a").await;
    seed_account(&pool, "account-b").await;
    let repository = AliasRepository::new(&pool);

    repository
        .upsert(request(None, "account-a", "z@example.test", None, false))
        .await
        .expect("alias");
    repository
        .upsert(request(None, "account-a", "a@example.test", None, true))
        .await
        .expect("alias");
    repository
        .upsert(request(None, "account-b", "b@example.test", None, false))
        .await
        .expect("alias");

    let account_a = repository.list(Some("account-a")).await.expect("filtered");
    assert_eq!(
        account_a
            .iter()
            .map(|alias| alias.email.as_str())
            .collect::<Vec<_>>(),
        ["a@example.test", "z@example.test"]
    );
    assert!(account_a[0].is_default != 0);

    let global = repository.list(None).await.expect("global");
    assert_eq!(
        global
            .iter()
            .map(|alias| (alias.account_id.as_str(), alias.email.as_str()))
            .collect::<Vec<_>>(),
        [
            ("account-a", "a@example.test"),
            ("account-a", "z@example.test"),
            ("account-b", "b@example.test"),
        ]
    );
}

#[tokio::test]
async fn upsert_normalizes_email_and_demotes_other_defaults_atomically() {
    let pool = pool().await;
    seed_account(&pool, "account-a").await;
    let repository = AliasRepository::new(&pool);
    let first = repository
        .upsert(request(
            Some("alias-first"),
            "account-a",
            " FIRST@example.test ",
            Some("First"),
            true,
        ))
        .await
        .expect("first alias");
    let second = repository
        .upsert(request(
            Some("alias-second"),
            "account-a",
            "SECOND@example.test",
            Some("Second"),
            true,
        ))
        .await
        .expect("second alias");

    assert_eq!(first.email, "first@example.test");
    assert_eq!(second.email, "second@example.test");
    let listed = repository.list(Some("account-a")).await.expect("list");
    assert_eq!(
        listed
            .iter()
            .map(|alias| (alias.id.as_str(), alias.is_default))
            .collect::<Vec<_>>(),
        [("alias-second", 1), ("alias-first", 0)]
    );
}

#[tokio::test]
async fn existing_id_upsert_keeps_original_account_owner() {
    let pool = pool().await;
    seed_account(&pool, "account-a").await;
    seed_account(&pool, "account-b").await;
    let repository = AliasRepository::new(&pool);
    repository
        .upsert(request(
            Some("alias-existing"),
            "account-a",
            "old@example.test",
            Some("Old"),
            false,
        ))
        .await
        .expect("initial alias");

    let updated = repository
        .upsert(request(
            Some("alias-existing"),
            "account-b",
            " NEW@example.test ",
            Some("New"),
            true,
        ))
        .await
        .expect("updated alias");
    assert_eq!(updated.account_id, "account-a");
    assert_eq!(updated.email, "new@example.test");
    assert_eq!(updated.display_name.as_deref(), Some("New"));
    assert_eq!(updated.is_default, 1);
}

#[tokio::test]
async fn delete_is_idempotent_including_missing_ids() {
    let pool = pool().await;
    seed_account(&pool, "account-a").await;
    let repository = AliasRepository::new(&pool);
    let alias = repository
        .upsert(request(
            None,
            "account-a",
            "alias@example.test",
            None,
            false,
        ))
        .await
        .expect("alias");

    repository.delete(&alias.id).await.expect("delete");
    repository
        .delete("missing-alias")
        .await
        .expect("missing delete");
    assert!(repository.list(None).await.expect("list").is_empty());
}

#[tokio::test]
async fn contains_matches_case_and_surrounding_whitespace() {
    let pool = pool().await;
    seed_account(&pool, "account-a").await;
    sqlx::query(
        "INSERT INTO account_aliases
            (id, account_id, email, display_name, is_default, created_at, updated_at)
         VALUES ('alias-spaced', 'account-a', '  Mixed@Example.TEST  ', NULL, 0,
                 '2026-01-01', '2026-01-01')",
    )
    .execute(&pool)
    .await
    .expect("spaced alias");
    let repository = AliasRepository::new(&pool);

    assert!(repository
        .contains("account-a", " mixed@example.test ")
        .await
        .expect("contains"));
    assert!(!repository
        .contains("account-b", "mixed@example.test")
        .await
        .expect("contains"));
}

#[tokio::test]
async fn missing_schema_is_reported_as_typed_persistence_error() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory db");

    let error = AliasRepository::new(&pool)
        .list(None)
        .await
        .expect_err("missing aliases table");
    assert!(matches!(error, AliasError::Persistence(_)));
}
