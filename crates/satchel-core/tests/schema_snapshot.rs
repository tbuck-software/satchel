//! The migrated schema, held against a checked-in snapshot.
//!
//! Every test in the tree builds its database fresh from the migrations, so a
//! repair migration that rebuilds a table and forgets to recreate its trigger
//! — the notification outbox trigger from 0057, say — leaves every test green
//! and quietly stops the feature on real installs. The snapshot is what
//! notices: tables with their columns and types, indexes, triggers and views,
//! normalised and compared as a whole.
//!
//! Changing the schema on purpose means changing the snapshot on purpose:
//! `UPDATE_SCHEMA_SNAPSHOT=1 cargo test -p satchel-core --test schema_snapshot`
//! rewrites `tests/fixtures/schema.snapshot.sql`, and the diff goes into review
//! next to the migration that caused it.

use sqlx::{sqlite::SqlitePoolOptions, Row, SqlitePool};
use std::path::PathBuf;

fn snapshot_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/schema.snapshot.sql")
}

async fn migrated_pool() -> SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory db");
    satchel_core::db::init(&pool).await.expect("migrations");
    pool
}

/// Collapse whitespace and drop `IF NOT EXISTS`, which the stored SQL keeps
/// verbatim and which says nothing about the object itself.
fn normalise(sql: &str) -> String {
    sql.replace("IF NOT EXISTS ", "")
        .replace("if not exists ", "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// The schema as sorted, one-object-per-line text.
async fn schema_text(pool: &SqlitePool) -> String {
    let objects = sqlx::query(
        "SELECT type, name, tbl_name, sql FROM sqlite_master
         WHERE name NOT LIKE 'sqlite_%' AND name <> '_sqlx_migrations'
         ORDER BY type, name",
    )
    .fetch_all(pool)
    .await
    .expect("sqlite_master");

    let mut lines = Vec::new();
    for object in &objects {
        let kind: String = object.get("type");
        let name: String = object.get("name");
        let sql: Option<String> = object.get("sql");
        // Shadow tables of FTS5 are an implementation detail of the virtual
        // table, which is listed itself.
        if kind == "table"
            && sql
                .as_deref()
                .is_some_and(|sql| sql.starts_with("CREATE TABLE '"))
        {
            continue;
        }
        match kind.as_str() {
            "table" => {
                let columns = sqlx::query(&format!(
                    "SELECT name, type, \"notnull\", dflt_value, pk FROM pragma_table_xinfo('{}')",
                    name.replace('\'', "''")
                ))
                .fetch_all(pool)
                .await
                .expect("columns");
                let columns: Vec<String> = columns
                    .iter()
                    .map(|column| {
                        let column_name: String = column.get("name");
                        let column_type: String = column.get("type");
                        let not_null: i64 = column.get("notnull");
                        let default: Option<String> = column.get("dflt_value");
                        let pk: i64 = column.get("pk");
                        let mut rendered = format!("{column_name} {column_type}");
                        if not_null == 1 {
                            rendered.push_str(" NOT NULL");
                        }
                        if let Some(default) = default {
                            rendered.push_str(&format!(" DEFAULT {default}"));
                        }
                        if pk > 0 {
                            rendered.push_str(&format!(" PK{pk}"));
                        }
                        rendered
                    })
                    .collect();
                let virtual_table = sql
                    .as_deref()
                    .is_some_and(|sql| sql.contains("VIRTUAL TABLE"));
                if virtual_table {
                    lines.push(format!(
                        "table {name}: {}",
                        normalise(sql.as_deref().unwrap_or(""))
                    ));
                } else {
                    lines.push(format!("table {name} ({})", columns.join(", ")));
                }
            }
            _ => lines.push(format!(
                "{kind} {name}: {}",
                normalise(sql.as_deref().unwrap_or(""))
            )),
        }
    }
    lines.join("\n") + "\n"
}

/// Where the two schemas part company, or `None` when they do not.
fn first_difference(expected: &str, actual: &str) -> Option<String> {
    let expected: Vec<&str> = expected.lines().collect();
    let actual: Vec<&str> = actual.lines().collect();
    let missing: Vec<&&str> = expected
        .iter()
        .filter(|line| !actual.contains(line))
        .collect();
    let unexpected: Vec<&&str> = actual
        .iter()
        .filter(|line| !expected.contains(line))
        .collect();
    if missing.is_empty() && unexpected.is_empty() {
        return None;
    }
    Some(format!(
        "missing from the migrated schema:\n  {}\nnot in the snapshot:\n  {}",
        missing
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n  "),
        unexpected
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n  ")
    ))
}

#[tokio::test]
async fn the_migrated_schema_matches_the_snapshot() {
    let pool = migrated_pool().await;
    let actual = schema_text(&pool).await;

    if std::env::var("UPDATE_SCHEMA_SNAPSHOT").as_deref() == Ok("1") {
        std::fs::create_dir_all(snapshot_path().parent().expect("fixtures dir")).expect("mkdir");
        std::fs::write(snapshot_path(), &actual).expect("write snapshot");
        return;
    }

    let expected = std::fs::read_to_string(snapshot_path())
        .expect("tests/fixtures/schema.snapshot.sql — generate it with UPDATE_SCHEMA_SNAPSHOT=1");
    if let Some(difference) = first_difference(&expected, &actual) {
        panic!(
            "the schema changed. If that was the point, regenerate the snapshot with \
             UPDATE_SCHEMA_SNAPSHOT=1 and review the diff.\n{difference}"
        );
    }
}

/// The three trigger migrations are the objects a table rebuild loses first.
#[tokio::test]
async fn the_snapshot_covers_the_triggers_a_rebuild_would_drop() {
    let snapshot = std::fs::read_to_string(snapshot_path()).expect("snapshot");
    let triggers: Vec<&str> = snapshot
        .lines()
        .filter(|line| line.starts_with("trigger "))
        .collect();

    assert!(!triggers.is_empty(), "no triggers in the snapshot at all");
    for (migration, trigger) in [
        ("0053", "message_search_projection_ai"),
        ("0053", "message_search_projection_ad"),
        ("0053", "message_search_projection_au"),
        // Created by 0057, replaced by 0060 with the suppression reasons.
        ("0057/0060", "messages_notification_outbox_ai"),
    ] {
        assert!(
            triggers
                .iter()
                .any(|line| line.starts_with(&format!("trigger {trigger}:"))),
            "trigger {trigger} (migration {migration}) is not in the snapshot"
        );
    }
    let outbox = triggers
        .iter()
        .find(|line| line.starts_with("trigger messages_notification_outbox_ai:"))
        .expect("outbox trigger");
    assert!(
        outbox.contains("reason"),
        "the snapshot holds the 0057 trigger, not 0060's"
    );
}

/// A dropped column has to fail the comparison, not slip through it.
#[tokio::test]
async fn a_column_missing_from_the_schema_is_reported() {
    let pool = migrated_pool().await;
    let actual = schema_text(&pool).await;
    let threads_line = actual
        .lines()
        .find(|line| line.starts_with("table threads ("))
        .expect("threads table");
    let without_column = threads_line.replace(", snippet TEXT", "");
    assert_ne!(
        threads_line, without_column,
        "fixture did not remove a column"
    );
    let tampered = actual.replace(threads_line, &without_column);

    let difference = first_difference(&actual, &tampered).expect("a difference");

    assert!(difference.contains("snippet TEXT"), "{difference}");
}
async fn golden_pool() -> (tempfile::TempDir, SqlitePool) {
    let directory = tempfile::tempdir().expect("fixture directory");
    let path = directory.path().join("satchel.db");
    std::fs::write(&path, include_bytes!("fixtures/upgrade-v22.db")).expect("copy golden database");
    let options = sqlx::sqlite::SqliteConnectOptions::new().filename(&path);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .expect("golden database");
    (directory, pool)
}

#[tokio::test]
async fn an_older_database_with_retired_migrations_upgrades_without_losing_mail() {
    let (_directory, pool) = golden_pool().await;
    let old_version: i64 = sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(old_version, 22);
    // Daily installs retain these ledger entries after their SQL leaves the tree.
    for version in 23..=26 {
        sqlx::query("INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time) VALUES (?, 'retired daily migration', 1, ?, 0)")
            .bind(version).bind(vec![0_u8; 48]).execute(&pool).await.unwrap();
    }
    satchel_core::db::init(&pool)
        .await
        .expect("upgrade golden database");
    assert_eq!(
        schema_text(&pool).await,
        std::fs::read_to_string(snapshot_path()).unwrap()
    );
    let body: String =
        sqlx::query_scalar("SELECT body_text FROM messages WHERE id = 'upgrade-message'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(body, "preserved content");
    let retired: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations WHERE version BETWEEN 23 AND 26")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(retired, 4);
    assert!(satchel_core::db::message_index_intact(&pool).await.unwrap());
    satchel_core::db::init(&pool)
        .await
        .expect("idempotent reopen");
    pool.close().await;
}

#[tokio::test]
async fn an_older_database_still_rejects_a_changed_applied_migration() {
    let (_directory, pool) = golden_pool().await;
    sqlx::query("UPDATE _sqlx_migrations SET checksum = ? WHERE version = 1")
        .bind(vec![0_u8; 48])
        .execute(&pool)
        .await
        .unwrap();
    let error = satchel_core::db::init(&pool)
        .await
        .expect_err("checksum mismatch must fail");
    assert!(error.to_string().contains("modified"), "{error}");
    pool.close().await;
}
