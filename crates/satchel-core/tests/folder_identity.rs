//! One remote folder, one local row.
//!
//! A move into a folder the sync had not listed yet created a row with a
//! random id; the next sync upserted by its derived id and added a second one.

use satchel_core::db::folders::{folder_db_id, resolve_provider_folder_id};
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
    // This shipped migration deliberately recognizes its original ID prefix.
    let derived = "fld-imap-acc-Archive".to_string();
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
async fn resolving_a_legacy_folder_keeps_its_id_and_all_persisted_references() {
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

    // Cover references that the old rekey helper either rewrote or omitted.
    // A missing remote ID exercises the full_path fallback as well.
    exec(
        &pool,
        "UPDATE folders SET remote_id = NULL WHERE id = 'fld-workflow-legacy'",
    )
    .await;
    exec(&pool, "INSERT INTO threads (id, account_id, last_message_date, created_at, updated_at) VALUES ('thread', 'acc', '', '', '')").await;
    exec(&pool, "UPDATE messages SET pending_folder_id = 'fld-workflow-legacy', thread_id = 'thread' WHERE id = 'm'").await;
    exec(&pool, "INSERT INTO sync_quarantine (account_id, folder_id, remote_id, first_failed_at, last_failed_at) VALUES ('acc', 'fld-workflow-legacy', '42', '', '')").await;
    exec(
        &pool,
        "INSERT INTO mail_action_receipts (id, created_at) VALUES ('receipt', '')",
    )
    .await;
    exec(&pool, "INSERT INTO mail_action_messages (receipt_id, message_id, original_folder_id, target_folder_id) VALUES ('receipt', 'm', 'fld-workflow-legacy', 'fld-workflow-legacy')").await;
    let references = |pool: SqlitePool| async move {
        sqlx::query_as::<_, (String, Option<String>, String, String, String, String)>(
            "SELECT m.folder_id, m.pending_folder_id, q.folder_id,
                    r.original_folder_id, r.target_folder_id, tf.folder_id
             FROM messages m JOIN sync_quarantine q ON q.remote_id = '42'
             JOIN mail_action_messages r ON r.message_id = m.id
             JOIN thread_folders tf ON tf.thread_id = m.thread_id
             WHERE m.id = 'm'",
        )
        .fetch_one(&pool)
        .await
        .unwrap()
    };
    let before = references(pool.clone()).await;
    let id = resolve_provider_folder_id(&pool, "acc", "Archive")
        .await
        .expect("resolve");

    assert_eq!(id, "fld-workflow-legacy");
    assert_eq!(folder_ids(&pool).await, std::slice::from_ref(&id));
    assert_eq!(references(pool.clone()).await, before);
    assert_eq!(orphaned_messages(&pool).await, 0);
    let violations: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM pragma_foreign_key_check")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(violations, 0);
}

#[tokio::test]
async fn remote_folder_punctuation_does_not_overwrite_another_folder() {
    let pool = migrated_pool().await;
    seed_account(&pool).await;
    for pass in 0..2 {
        for (remote, message) in [("Projects/2026", "slash"), ("Projects-2026", "dash")] {
            let id = resolve_provider_folder_id(&pool, "acc", remote)
                .await
                .unwrap();
            sqlx::query(
                "INSERT INTO folders (id, account_id, name, full_path, remote_id, folder_type, created_at, updated_at)
                 VALUES (?, 'acc', ?, ?, ?, 'custom', '', '')
                 ON CONFLICT(id) DO UPDATE SET name = excluded.name, full_path = excluded.full_path, remote_id = excluded.remote_id",
            ).bind(&id).bind(remote).bind(remote).bind(remote).execute(&pool).await.unwrap();
            if pass == 0 {
                insert_message(&pool, message, &id).await;
            }
        }
    }
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT m.id, f.id, f.remote_id FROM messages m JOIN folders f ON f.id = m.folder_id ORDER BY m.id",
    ).fetch_all(&pool).await.unwrap();
    assert_eq!(rows.len(), 2);
    assert_ne!(rows[0].1, rows[1].1);
    assert_eq!(rows[0].2, "Projects-2026");
    assert_eq!(rows[1].2, "Projects/2026");
    assert_eq!(folder_ids(&pool).await.len(), 2);
    assert_ne!(
        folder_db_id("acc.a", "Remote"),
        folder_db_id("acc-a", "Remote")
    );
    assert_ne!(folder_db_id("a-b", "c"), folder_db_id("a", "b-c"));
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
    let mut expected = vec![folder_db_id("acc", "Archive"), folder_db_id("acc", "INBOX")];
    expected.sort();
    assert_eq!(
        folder_ids(&pool).await,
        expected,
        "the move creates the archive under its derived id"
    );

    let folder_id = resolve_provider_folder_id(&pool, "acc", "Archive")
        .await
        .expect("resolve");
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
