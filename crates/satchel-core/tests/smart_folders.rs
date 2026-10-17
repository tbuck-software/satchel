use satchel_core::db::smart_folders::{
    SmartFolderError, SmartFolderRepository, SmartFolderUpsertRequest,
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

fn request(name: &str) -> SmartFolderUpsertRequest {
    SmartFolderUpsertRequest {
        id: None,
        name: name.into(),
        icon: None,
        query: format!("subject:{name}"),
        sort_order: None,
    }
}

#[tokio::test]
async fn list_orders_by_sort_order_then_name() {
    let pool = pool().await;
    let repository = SmartFolderRepository::new(&pool);

    for (id, name, sort_order) in [
        ("sf-z", "Zulu", 2),
        ("sf-b", "Beta", 1),
        ("sf-a", "Alpha", 1),
        ("sf-zero", "Zero", 0),
    ] {
        repository
            .upsert(SmartFolderUpsertRequest {
                id: Some(id.into()),
                name: name.into(),
                icon: None,
                query: format!("from:{name}"),
                sort_order: Some(sort_order),
            })
            .await
            .expect("smart folder");
    }

    let listed = repository.list().await.expect("list");
    assert_eq!(
        listed
            .iter()
            .map(|folder| folder.id.as_str())
            .collect::<Vec<_>>(),
        ["sf-zero", "sf-a", "sf-b", "sf-z"]
    );
}

#[tokio::test]
async fn upsert_generates_ids_defaults_order_and_preserves_created_at() {
    let pool = pool().await;
    let repository = SmartFolderRepository::new(&pool);

    let created = repository.upsert(request("Initial")).await.expect("create");
    assert!(created.id.starts_with("sf-"));
    assert_eq!(created.sort_order, 0);
    assert_eq!(created.query, "subject:Initial");

    let updated = repository
        .upsert(SmartFolderUpsertRequest {
            id: Some(created.id.clone()),
            name: "Updated".into(),
            icon: Some("star".into()),
            query: "has:attachment".into(),
            sort_order: Some(4),
        })
        .await
        .expect("update");
    assert_eq!(updated.id, created.id);
    assert_eq!(updated.name, "Updated");
    assert_eq!(updated.icon.as_deref(), Some("star"));
    assert_eq!(updated.query, "has:attachment");
    assert_eq!(updated.sort_order, 4);
    assert_eq!(updated.created_at, created.created_at);
}

#[tokio::test]
async fn upsert_keeps_a_provided_id_and_stores_nullable_fields() {
    let pool = pool().await;
    let repository = SmartFolderRepository::new(&pool);

    let stored = repository
        .upsert(SmartFolderUpsertRequest {
            id: Some("saved-important".into()),
            name: "Important".into(),
            icon: Some("!".into()),
            query: "is:unread".into(),
            sort_order: Some(-2),
        })
        .await
        .expect("provided id");

    assert_eq!(stored.id, "saved-important");
    assert_eq!(stored.icon.as_deref(), Some("!"));
    assert_eq!(stored.sort_order, -2);
}

#[tokio::test]
async fn delete_is_idempotent_and_removes_only_the_requested_folder() {
    let pool = pool().await;
    let repository = SmartFolderRepository::new(&pool);
    let first = repository.upsert(request("First")).await.expect("first");
    let second = repository.upsert(request("Second")).await.expect("second");

    repository.delete(&first.id).await.expect("delete");
    repository.delete(&first.id).await.expect("repeated delete");

    let remaining = repository.list().await.expect("list");
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
    let error = SmartFolderRepository::new(&pool).list().await.unwrap_err();
    assert!(matches!(error, SmartFolderError::Persistence(_)));
}
