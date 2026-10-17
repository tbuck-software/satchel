use satchel_core::db::templates::{TemplateError, TemplateRepository, TemplateUpsertRequest};
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
        "INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, sync_state, color, is_active, created_at, updated_at)
         VALUES (?, ?, ?, 'imap', '{}', '', '{}', '#000', 1, '2026-01-01', '2026-01-01')",
    )
    .bind(id)
    .bind(id)
    .bind(format!("{id}@example.test"))
    .execute(pool)
    .await
    .expect("account");
}

fn request(name: &str) -> TemplateUpsertRequest {
    TemplateUpsertRequest {
        id: None,
        account_id: None,
        name: name.into(),
        subject: None,
        body_text: String::new(),
        body_html: None,
        shortcut: None,
        cc: Vec::new(),
        bcc: Vec::new(),
        attachments_json: None,
    }
}

#[tokio::test]
async fn list_scopes_to_global_and_account_templates_and_orders_by_name() {
    let pool = pool().await;
    seed_account(&pool, "acc-one").await;
    seed_account(&pool, "acc-two").await;
    let repository = TemplateRepository::new(&pool);

    for (id, account_id, name) in [
        ("tpl-global-z", None, "Zulu"),
        ("tpl-one-a", Some("acc-one"), "Alpha"),
        ("tpl-two-b", Some("acc-two"), "Beta"),
        ("tpl-global-c", None, "Charlie"),
        ("tpl-one-y", Some("acc-one"), "Yankee"),
    ] {
        repository
            .upsert(TemplateUpsertRequest {
                id: Some(id.into()),
                account_id: account_id.map(Into::into),
                name: name.into(),
                ..request(name)
            })
            .await
            .expect("template");
    }

    let scoped = repository.list(Some("acc-one")).await.expect("scoped list");
    assert_eq!(
        scoped
            .iter()
            .map(|template| template.id.as_str())
            .collect::<Vec<_>>(),
        ["tpl-one-a", "tpl-global-c", "tpl-one-y", "tpl-global-z"]
    );

    let all = repository.list(None).await.expect("all templates");
    assert_eq!(
        all.iter()
            .map(|template| template.id.as_str())
            .collect::<Vec<_>>(),
        [
            "tpl-one-a",
            "tpl-two-b",
            "tpl-global-c",
            "tpl-one-y",
            "tpl-global-z"
        ]
    );
}

#[tokio::test]
async fn upsert_stores_defaults_nullable_fields_and_typed_recipients() {
    let pool = pool().await;
    let repository = TemplateRepository::new(&pool);

    let stored = repository
        .upsert(TemplateUpsertRequest {
            id: Some("tpl-fields".into()),
            name: "Fields".into(),
            cc: vec!["copy@example.test", "second@example.test"]
                .into_iter()
                .map(String::from)
                .collect(),
            bcc: vec!["blind@example.test"]
                .into_iter()
                .map(String::from)
                .collect(),
            ..request("Fields")
        })
        .await
        .expect("template");

    assert_eq!(stored.id, "tpl-fields");
    assert_eq!(stored.account_id, None);
    assert_eq!(stored.subject, None);
    assert_eq!(stored.body_text, "");
    assert_eq!(stored.body_html, None);
    assert_eq!(stored.shortcut, None);
    assert_eq!(
        stored.cc,
        Some(vec![
            "copy@example.test".to_string(),
            "second@example.test".to_string()
        ])
    );
    assert_eq!(stored.bcc, Some(vec!["blind@example.test".to_string()]));
    assert_eq!(stored.attachments_json, None);
}

#[tokio::test]
async fn upsert_generates_or_preserves_ids_and_updates_without_replacing_created_at() {
    let pool = pool().await;
    let repository = TemplateRepository::new(&pool);

    let generated = repository.upsert(request("Initial")).await.expect("create");
    assert!(generated.id.starts_with("tpl-"));
    assert_eq!(generated.created_at, generated.updated_at);

    sqlx::query(
        "UPDATE templates SET created_at = 'old-created', updated_at = 'old-updated' WHERE id = ?",
    )
    .bind(&generated.id)
    .execute(&pool)
    .await
    .expect("seed timestamps");
    let updated = repository
        .upsert(TemplateUpsertRequest {
            id: Some(generated.id.clone()),
            name: "Updated".into(),
            subject: Some("New subject".into()),
            body_text: "New body".into(),
            shortcut: Some("updated".into()),
            cc: vec!["new@example.test".into()],
            ..request("Updated")
        })
        .await
        .expect("update");
    assert_eq!(updated.id, generated.id);
    assert_eq!(updated.name, "Updated");
    assert_eq!(updated.subject.as_deref(), Some("New subject"));
    assert_eq!(updated.body_text, "New body");
    assert_eq!(updated.shortcut.as_deref(), Some("updated"));
    assert_eq!(updated.cc, Some(vec!["new@example.test".to_string()]));
    assert_eq!(updated.created_at, "old-created");
    assert_ne!(updated.updated_at, "old-updated");

    let provided = repository
        .upsert(TemplateUpsertRequest {
            id: Some("saved-template".into()),
            ..request("Provided")
        })
        .await
        .expect("provided id");
    assert_eq!(provided.id, "saved-template");
}

#[tokio::test]
async fn get_reports_a_typed_not_found_error() {
    let pool = pool().await;
    let error = TemplateRepository::new(&pool)
        .get("missing-template")
        .await
        .expect_err("missing template");
    assert!(matches!(error, TemplateError::NotFound));
}

#[tokio::test]
async fn delete_is_idempotent_and_removes_only_the_requested_template() {
    let pool = pool().await;
    let repository = TemplateRepository::new(&pool);
    let first = repository.upsert(request("First")).await.expect("first");
    let second = repository.upsert(request("Second")).await.expect("second");

    repository.delete(&first.id).await.expect("delete");
    repository.delete(&first.id).await.expect("repeated delete");

    assert!(matches!(
        repository.get(&first.id).await,
        Err(TemplateError::NotFound)
    ));
    let remaining = repository.list(None).await.expect("list");
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].id, second.id);
}

#[tokio::test]
async fn missing_table_is_reported_as_a_typed_persistence_error() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory db");
    let error = TemplateRepository::new(&pool)
        .list(None)
        .await
        .expect_err("missing table");
    assert!(matches!(error, TemplateError::Persistence(_)));
}
