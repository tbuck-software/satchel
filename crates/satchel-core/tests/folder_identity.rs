//! One remote folder, one local row.
//!
//! A move into a folder the sync had not listed yet created a row with a
//! random id; the next sync upserted by its derived id and added a second one.

use satchel_core::db::folders::{adopt_derived_folder_id, folder_db_id};
use satchel_core::mail_actions::{move_threads, MoveTarget};
use satchel_core::sync::queue::SyncQueue;
use sqlx::{
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
    SqlitePool,
};

const MERGE_MIGRATION: i64 = 77;

async fn empty_pool() -> SqlitePool {
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            "sqlite::memory:"
                .parse::<SqliteConnectOptions>()
                .unwrap()
                .foreign_keys(true),
        )
        .await
        .expect("pool")
}

/// The schema as it stood before the merge migration.
async fn pool_before_merge() -> SqlitePool {
    let pool = empty_pool().await;
    for migration in sqlx::migrate!("./src/db/migrations")
        .iter()
        .filter(|migration| migration.version < MERGE_MIGRATION)
    {
        sqlx::raw_sql(&migration.sql)
            .execute(&pool)
            .await
            .unwrap_or_else(|err| panic!("migration {}: {err}", migration.version));
    }
    pool
}

async fn run_merge_migration(pool: &SqlitePool) {
    let migrator = sqlx::migrate!("./src/db/migrations");
    let merge = migrator
        .iter()
        .find(|migration| migration.version == MERGE_MIGRATION)
        .expect("merge migration");
    sqlx::raw_sql(&merge.sql)
        .execute(pool)
        .await
        .expect("merge");
}

async fn migrated_pool() -> SqlitePool {
    let pool = empty_pool().await;
    satchel_core::db::init(&pool).await.expect("migrations");
    pool
}

async fn exec(pool: &SqlitePool, sql: &str) {
    sqlx::query(sql).execute(pool).await.expect(sql);
}

async fn seed_account(pool: &SqlitePool) {
    exec(
        pool,
        "INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, sync_state, color, is_active, created_at, updated_at)
         VALUES ('acc', 'A', 'me@example.test', 'imap', '{}', '', '{}', '#000', 1, '2026-01-01', '2026-01-01')",
    )
    .await;
}

async fn insert_folder(
    pool: &SqlitePool,
    id: &str,
    remote: &str,
    folder_type: &str,
    updated: &str,
) {
    sqlx::query(
        "INSERT INTO folders (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at)
         VALUES (?, 'acc', ?, ?, ?, ?, '2026-01-01', ?)",
    )
    .bind(id)
    .bind(remote)
    .bind(remote)
    .bind(folder_type)
    .bind(remote)
    .bind(updated)
    .execute(pool)
    .await
    .expect("folder");
}

async fn insert_message(pool: &SqlitePool, id: &str, folder_id: &str) {
    sqlx::query(
        "INSERT INTO messages (id, account_id, folder_id, from_address, to_addresses, subject, date, created_at, updated_at)
         VALUES (?, 'acc', ?, 'a@example.test', '[]', 'S', '2026-01-01', '2026-01-01', '2026-01-01')",
    )
    .bind(id)
    .bind(folder_id)
    .execute(pool)
    .await
    .expect("message");
}

async fn folder_ids(pool: &SqlitePool) -> Vec<String> {
    sqlx::query_scalar("SELECT id FROM folders ORDER BY id")
        .fetch_all(pool)
        .await
        .expect("folders")
}

async fn orphaned_messages(pool: &SqlitePool) -> i64 {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM messages m WHERE NOT EXISTS (SELECT 1 FROM folders f WHERE f.id = m.folder_id)",
    )
    .fetch_one(pool)
    .await
    .expect("orphans")
}

#[tokio::test]
async fn the_migration_merges_duplicates_into_the_derived_row() {
    let pool = pool_before_merge().await;
    seed_account(&pool).await;
    let derived = folder_db_id("acc", "Archive");
    insert_folder(&pool, "fld-move-1234", "Archive", "archive", "2026-03-01").await;
    insert_folder(&pool, &derived, "Archive", "archive", "2026-02-01").await;
    insert_folder(&pool, "fld-imap-acc-INBOX", "INBOX", "inbox", "2026-01-01").await;
    insert_message(&pool, "moved", "fld-move-1234").await;
    insert_message(&pool, "synced", &derived).await;
    exec(
        &pool,
        "UPDATE messages SET pending_folder_id = 'fld-move-1234' WHERE id = 'synced'",
    )
    .await;

    run_merge_migration(&pool).await;

    assert_eq!(
        folder_ids(&pool).await,
        [derived.clone(), "fld-imap-acc-INBOX".to_string()]
    );
    let homes: Vec<(String, Option<String>)> =
        sqlx::query_as("SELECT folder_id, pending_folder_id FROM messages ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(
        homes,
        [(derived.clone(), None), (derived.clone(), Some(derived))]
    );
    assert_eq!(orphaned_messages(&pool).await, 0);
}

#[tokio::test]
async fn a_second_row_for_the_same_remote_folder_is_rejected() {
    let pool = migrated_pool().await;
    seed_account(&pool).await;
    insert_folder(&pool, "one", "Archive", "archive", "2026-01-01").await;

    let second = sqlx::query(
        "INSERT INTO folders (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at)
         VALUES ('two', 'acc', 'Archive', 'Archive', 'archive', 'Archive', '', '')",
    )
    .execute(&pool)
    .await;

    assert!(second.is_err(), "the unique index lets a duplicate in");
}

#[tokio::test]
async fn a_row_with_a_random_id_takes_the_derived_one_with_its_messages() {
    let pool = migrated_pool().await;
    seed_account(&pool).await;
    insert_folder(
        &pool,
        "fld-workflow-legacy",
        "Archive",
        "archive",
        "2026-01-01",
    )
    .await;
    insert_message(&pool, "m", "fld-workflow-legacy").await;

    let id = adopt_derived_folder_id(&pool, "acc", "Archive")
        .await
        .expect("adopt");

    assert_eq!(id, folder_db_id("acc", "Archive"));
    assert_eq!(folder_ids(&pool).await, std::slice::from_ref(&id));
    assert_eq!(orphaned_messages(&pool).await, 0);
    let violations: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM pragma_foreign_key_check")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(violations, 0);
}

/// Archive into a folder the sync never listed, then run the folder upsert the
/// sync runs: still one Archive row, holding the message.
#[tokio::test]
async fn archiving_before_the_first_folder_sync_leaves_one_archive_folder() {
    let pool = migrated_pool().await;
    seed_account(&pool).await;
    insert_folder(
        &pool,
        &folder_db_id("acc", "INBOX"),
        "INBOX",
        "inbox",
        "2026-01-01",
    )
    .await;
    exec(
        &pool,
        "INSERT INTO threads (id, account_id, subject, last_message_date, message_count, unread_count, workflow_state, created_at, updated_at)
         VALUES ('t', 'acc', 'S', '2026-01-01', 1, 0, 'inbox', '2026-01-01', '2026-01-01')",
    )
    .await;
    insert_message(&pool, "m", &folder_db_id("acc", "INBOX")).await;
    exec(
        &pool,
        "UPDATE messages SET thread_id = 't', remote_id = '1'",
    )
    .await;

    move_threads(
        &pool,
        &SyncQueue::new(),
        &["t".to_string()],
        MoveTarget::Archive,
    )
    .await
    .expect("archive");
    assert_eq!(
        folder_ids(&pool).await,
        [folder_db_id("acc", "Archive"), folder_db_id("acc", "INBOX")],
        "the move creates the archive under its derived id"
    );

    let folder_id = adopt_derived_folder_id(&pool, "acc", "Archive")
        .await
        .expect("adopt");
    sqlx::query(
        "INSERT INTO folders (id, account_id, name, full_path, folder_type, remote_id, uidvalidity, message_count, unread_count, sync_enabled, sort_order, created_at, updated_at)
         VALUES (?, 'acc', 'Archive', 'Archive', 'archive', 'Archive', NULL, 0, 0, 1, 0, '', '')
         ON CONFLICT(id) DO UPDATE SET name = excluded.name, remote_id = excluded.remote_id",
    )
    .bind(&folder_id)
    .execute(&pool)
    .await
    .expect("sync upsert");

    let archives: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM folders WHERE COALESCE(remote_id, full_path) = 'Archive'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(archives, 1);
    assert_eq!(orphaned_messages(&pool).await, 0);
}
