use std::collections::HashSet;

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

async fn columns(pool: &SqlitePool, table: &str) -> HashSet<String> {
    sqlx::query_scalar::<_, String>(&format!("SELECT name FROM pragma_table_info('{table}')"))
        .fetch_all(pool)
        .await
        .expect("table columns")
        .into_iter()
        .collect()
}

async fn tables(pool: &SqlitePool) -> HashSet<String> {
    sqlx::query_scalar::<_, String>(
        "SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name",
    )
    .fetch_all(pool)
    .await
    .expect("tables")
    .into_iter()
    .collect()
}

#[tokio::test]
async fn signature_v2_schema_supports_variants_identity_defaults_and_reviewable_discovery() {
    let pool = pool().await;
    let table_names = tables(&pool).await;

    for expected in [
        "signature_assignments",
        "signature_assets",
        "signature_suggestions",
    ] {
        assert!(
            table_names.contains(expected),
            "missing signature table {expected}"
        );
    }

    let signature_columns = columns(&pool, "signatures").await;
    assert!(signature_columns.contains("short_body_text"));
    assert!(signature_columns.contains("short_body_html"));

    let assignment_columns = columns(&pool, "signature_assignments").await;
    for expected in [
        "account_id",
        "identity_email",
        "signature_id",
        "first_contact_mode",
    ] {
        assert!(
            assignment_columns.contains(expected),
            "missing signature_assignments.{expected}"
        );
    }

    let asset_columns = columns(&pool, "signature_assets").await;
    for expected in [
        "signature_id",
        "filename",
        "content_type",
        "content_id",
        "content",
    ] {
        assert!(
            asset_columns.contains(expected),
            "missing signature_assets.{expected}"
        );
    }

    let suggestion_columns = columns(&pool, "signature_suggestions").await;
    for expected in [
        "account_id",
        "identity_email",
        "body_text",
        "body_html",
        "evidence_count",
        "status",
    ] {
        assert!(
            suggestion_columns.contains(expected),
            "missing signature_suggestions.{expected}"
        );
    }

    let message_columns = columns(&pool, "messages").await;
    assert!(message_columns.contains("signature_id"));
    assert!(message_columns.contains("signature_variant"));
}
