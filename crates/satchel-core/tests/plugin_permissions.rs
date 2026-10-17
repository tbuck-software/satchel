use satchel_core::db::plugin_permissions::{
    PluginPermissionError, PluginPermissionRepository, PluginPermissionUpsertRequest,
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

fn request(plugin_id: &str, capability: &str, granted: bool) -> PluginPermissionUpsertRequest {
    PluginPermissionUpsertRequest {
        plugin_id: plugin_id.into(),
        capability: capability.into(),
        granted,
    }
}

#[tokio::test]
async fn list_groups_only_granted_rows_by_plugin() {
    let pool = pool().await;
    let repository = PluginPermissionRepository::new(&pool);

    repository
        .upsert(request("plugin-a", "read_threads", true))
        .await
        .expect("first grant");
    repository
        .upsert(request("plugin-a", "write_tags", true))
        .await
        .expect("second grant");
    repository
        .upsert(request("plugin-a", "send_mail", false))
        .await
        .expect("denied capability");
    repository
        .upsert(request("plugin-b", "read_contacts", true))
        .await
        .expect("other plugin grant");

    let grants = repository.list_granted().await.expect("grants");
    assert_eq!(grants["plugin-a"].len(), 2);
    assert!(grants["plugin-a"].contains("read_threads"));
    assert!(grants["plugin-a"].contains("write_tags"));
    assert!(!grants["plugin-a"].contains("send_mail"));
    assert_eq!(
        grants["plugin-b"].iter().collect::<Vec<_>>(),
        [&"read_contacts"]
    );
}

#[tokio::test]
async fn upsert_stores_true_and_false_and_toggles_existing_permission() {
    let pool = pool().await;
    let repository = PluginPermissionRepository::new(&pool);

    repository
        .upsert(request("plugin", "read_threads", true))
        .await
        .expect("grant");
    assert!(repository
        .list_granted()
        .await
        .expect("grants")
        .get("plugin")
        .is_some_and(|capabilities| capabilities.contains("read_threads")));

    repository
        .upsert(request("plugin", "read_threads", false))
        .await
        .expect("revoke");
    assert!(!repository
        .list_granted()
        .await
        .expect("grants")
        .contains_key("plugin"));

    repository
        .upsert(request("plugin", "read_threads", true))
        .await
        .expect("re-grant");
    assert!(repository
        .list_granted()
        .await
        .expect("grants")
        .get("plugin")
        .is_some_and(|capabilities| capabilities.contains("read_threads")));
}

#[tokio::test]
async fn upsert_updates_timestamp_without_replacing_the_conflict_row() {
    let pool = pool().await;
    let repository = PluginPermissionRepository::new(&pool);

    repository
        .upsert(request("plugin", "read_threads", true))
        .await
        .expect("initial permission");
    sqlx::query(
        "UPDATE plugin_permissions SET updated_at = 'old-updated'
         WHERE plugin_id = 'plugin' AND capability = 'read_threads'",
    )
    .execute(&pool)
    .await
    .expect("seed timestamp");
    let initial: (i64, String) = sqlx::query_as(
        "SELECT granted, updated_at FROM plugin_permissions
         WHERE plugin_id = 'plugin' AND capability = 'read_threads'",
    )
    .fetch_one(&pool)
    .await
    .expect("initial row");

    repository
        .upsert(request("plugin", "read_threads", false))
        .await
        .expect("updated permission");
    let updated: (i64, String) = sqlx::query_as(
        "SELECT granted, updated_at FROM plugin_permissions
         WHERE plugin_id = 'plugin' AND capability = 'read_threads'",
    )
    .fetch_one(&pool)
    .await
    .expect("updated row");

    assert_eq!(initial.0, 1);
    assert_eq!(updated.0, 0);
    assert_eq!(initial.1, "old-updated");
    assert_ne!(updated.1, "old-updated");
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM plugin_permissions
             WHERE plugin_id = 'plugin' AND capability = 'read_threads'",
        )
        .fetch_one(&pool)
        .await
        .expect("permission count"),
        1
    );
}

#[tokio::test]
async fn list_returns_an_empty_grouping_without_permissions() {
    let pool = pool().await;
    let grants = PluginPermissionRepository::new(&pool)
        .list_granted()
        .await
        .expect("grants");

    assert!(grants.is_empty());
}

#[tokio::test]
async fn missing_table_is_reported_as_a_typed_persistence_error() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory db");
    let error = PluginPermissionRepository::new(&pool)
        .list_granted()
        .await
        .expect_err("missing table");

    assert!(matches!(error, PluginPermissionError::Persistence(_)));
}
