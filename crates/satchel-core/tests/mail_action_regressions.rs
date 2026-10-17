use satchel_core::sync::{queue::SyncQueue, remote_mutations};
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
async fn seed_pool() -> SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::migrate!("./src/db/migrations")
        .run(&pool)
        .await
        .unwrap();

    sqlx::query(
            "INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, sync_state, color, is_active, created_at, updated_at, kind)
             VALUES ('acc-1', 'A', 'a@example.com', 'imap', '{}', '', '{}', '#000', 1, '2026-01-01', '2026-01-01', 'email')",
        )
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
            "INSERT INTO folders (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at)
             VALUES ('fld-inbox', 'acc-1', 'INBOX', 'INBOX', 'inbox', 'INBOX', '2026-01-01', '2026-01-01')",
        )
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
            "INSERT INTO threads (id, account_id, subject, participants, message_count, unread_count, has_attachments, workflow_state, board_column, priority, last_message_date, created_at, updated_at)
             VALUES ('th-1', 'acc-1', 'Betreff', '[]', 1, 0, 0, 'inbox', 'inbox', 0, '2026-01-01', '2026-01-01', '2026-01-01')",
        )
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
            "INSERT INTO messages (id, account_id, folder_id, thread_id, from_address, to_addresses, date, remote_id, created_at, updated_at)
             VALUES ('msg-1', 'acc-1', 'fld-inbox', 'th-1', 'a@example.com', '[]', '2026-01-01', '42', '2026-01-01', '2026-01-01')",
        )
        .execute(&pool)
        .await
        .unwrap();
    pool
}

#[tokio::test]
async fn followup_flags_must_not_reuse_a_source_uid_in_the_destination() {
    let pool = seed_pool().await;
    sqlx::query("INSERT INTO folders (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at) VALUES ('target', 'acc-1', 'Target', 'Target', 'custom', 'Target', '2026-01-01', '2026-01-01')").execute(&pool).await.unwrap();
    let queue = SyncQueue::new();
    remote_mutations::enqueue_thread_move_to_folder(&pool, &queue, &["th-1".into()], "target")
        .await
        .unwrap();
    let identity: (String, String, Option<String>) = sqlx::query_as(
        "SELECT remote_id, folder_id, pending_folder_id FROM messages WHERE id = 'msg-1'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        identity,
        ("42".into(), "fld-inbox".into(), Some("target".into()))
    );
    let error =
        remote_mutations::enqueue_thread_flags(&pool, &queue, &["th-1".into()], Some(true), None)
            .await
            .unwrap_err();
    assert_eq!(error, "backend.errors.mailMovePending");
    let flags: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sync_queue WHERE json_extract(payload, '$.kind') = 'flags'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(flags, 0);
}
#[tokio::test]
async fn moving_to_another_accounts_folder_must_be_rejected() {
    let pool = seed_pool().await;
    sqlx::query("INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, created_at, updated_at) VALUES ('acc-2', 'B', 'b@example.com', 'imap', '{}', '', '2026-01-01', '2026-01-01')").execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO folders (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at) VALUES ('other', 'acc-2', 'Other', 'Other', 'custom', 'Other', '2026-01-01', '2026-01-01')").execute(&pool).await.unwrap();
    let result = remote_mutations::enqueue_thread_move_to_folder(
        &pool,
        &SyncQueue::new(),
        &["th-1".into()],
        "other",
    )
    .await;
    let queued: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_queue")
        .fetch_one(&pool)
        .await
        .unwrap();
    let mismatched: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages m JOIN folders f ON f.id = m.folder_id WHERE m.account_id <> f.account_id").fetch_one(&pool).await.unwrap();
    assert_eq!((queued, mismatched), (0, 0));
    assert!(
        result.is_err(),
        "Accepted cross-account move; queued={queued}, inconsistent messages={mismatched}"
    );
}

#[tokio::test]
async fn an_interrupted_staging_copy_must_not_replace_the_live_database() {
    let profile = tempfile::tempdir().unwrap();
    let live = profile.path().join("satchel.db");
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            sqlx::sqlite::SqliteConnectOptions::new()
                .filename(&live)
                .create_if_missing(true),
        )
        .await
        .unwrap();
    sqlx::query("CREATE TABLE review_marker (id INTEGER)")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    let original = std::fs::read(&live).unwrap();
    // A process interrupted while copying to the final staging filename can leave this prefix.
    std::fs::write(
        profile.path().join("satchel.db.restore-pending"),
        b"SQLite format 3\0",
    )
    .unwrap();
    satchel_core::db::apply_pending_restore(profile.path()).unwrap();
    assert_eq!(
        std::fs::metadata(&live).unwrap().len(),
        original.len() as u64,
        "An incomplete staged copy replaced the valid live database"
    );
}
#[tokio::test]
async fn archive_provider_work_must_exclude_sent_copies_like_its_local_projection() {
    let pool = seed_pool().await;
    sqlx::query("INSERT INTO folders (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at) VALUES ('sent', 'acc-1', 'Sent', 'Sent', 'sent', 'Sent', '2026-01-01', '2026-01-01')").execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO messages (id, account_id, folder_id, thread_id, from_address, to_addresses, date, remote_id, created_at, updated_at) VALUES ('sent-copy', 'acc-1', 'sent', 'th-1', 'a@example.com', '[]', '2026-01-01', '99', '2026-01-01', '2026-01-01')").execute(&pool).await.unwrap();
    // The same Core action is used by command and notification adapters.
    remote_mutations::enqueue_thread_move_to_kind_tracked(
        &pool,
        &SyncQueue::new(),
        &["th-1".into()],
        "archive",
    )
    .await
    .unwrap();
    let raw: String = sqlx::query_scalar(
        "SELECT payload FROM sync_queue WHERE json_extract(payload, '$.kind') = 'workflow_move'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let payload: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert!(
        !payload["refs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["source_folder"] == "Sent"),
        "Archive queue includes the sent copy: {payload}"
    );
}

#[tokio::test]
async fn rejected_queue_insert_rolls_back_local_flags() {
    let pool = seed_pool().await;
    sqlx::query("CREATE TRIGGER reject_queue BEFORE INSERT ON sync_queue BEGIN SELECT RAISE(ABORT, 'queue unavailable'); END").execute(&pool).await.unwrap();
    assert!(satchel_core::mail_actions::set_flags(
        &pool,
        &SyncQueue::new(),
        &["th-1".into()],
        Some(true),
        Some(true),
        None
    )
    .await
    .is_err());
    let flags: (bool, bool) =
        sqlx::query_as("SELECT is_read, is_starred FROM messages WHERE id = 'msg-1'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(flags, (false, false));
}

#[tokio::test]
async fn rejected_queue_insert_rolls_back_folder_provisioning_and_workflow_state() {
    let pool = seed_pool().await;
    sqlx::query("CREATE TRIGGER reject_queue BEFORE INSERT ON sync_queue BEGIN SELECT RAISE(ABORT, 'queue unavailable'); END").execute(&pool).await.unwrap();
    assert!(satchel_core::mail_actions::move_threads(
        &pool,
        &SyncQueue::new(),
        &["th-1".into()],
        satchel_core::mail_actions::MoveTarget::Archive
    )
    .await
    .is_err());
    let identity: (String, Option<String>) =
        sqlx::query_as("SELECT folder_id, pending_folder_id FROM messages WHERE id = 'msg-1'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(identity, ("fld-inbox".into(), None));
    let state: String = sqlx::query_scalar("SELECT workflow_state FROM threads WHERE id = 'th-1'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(state, "inbox");
    let folders: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM folders")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(folders, 1);
}

#[tokio::test]
async fn completion_uses_exact_destination_among_custom_folders() {
    let pool = seed_pool().await;
    for (id, remote) in [("first", "First"), ("target", "Target")] {
        sqlx::query("INSERT INTO folders (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at) VALUES (?, 'acc-1', ?, ?, 'custom', ?, '2026-01-01', '2026-01-01')").bind(id).bind(remote).bind(remote).bind(remote).execute(&pool).await.unwrap();
    }
    let queue = SyncQueue::new();
    let ids = satchel_core::mail_actions::move_threads(
        &pool,
        &queue,
        &["th-1".into()],
        satchel_core::mail_actions::MoveTarget::Folder("target"),
    )
    .await
    .unwrap();
    let target =
        remote_mutations::ensure_local_workflow_target_folder(&pool, "acc-1", "custom", "Target")
            .await
            .unwrap();
    assert_eq!(target, "target");
    let source = satchel_core::providers::traits::MessageRef {
        remote_id: "42".into(),
        source_folder: Some("INBOX".into()),
        source_uidvalidity: None,
    };
    // For providers with globally stable IDs. IMAP completion instead ingests
    // the destination UID and removes the original source through reconciliation.
    remote_mutations::apply_completed_move_projection(&pool, "acc-1", &[source], &target)
        .await
        .unwrap();
    for id in ids {
        queue.mark_completed(&pool, &id).await.unwrap();
    }
    let identity: (String, Option<String>) =
        sqlx::query_as("SELECT folder_id, pending_folder_id FROM messages WHERE id = 'msg-1'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(identity, ("target".into(), None));
}

#[tokio::test]
async fn outlook_provisioning_preserves_well_known_remote_ids() {
    for (kind, name, path, remote) in [
        ("archive", "Archiviert", "Archive", "archive"),
        ("trash", "Papierkorb", "Deleted Items", "deleteditems"),
        ("spam", "junkemail", "junkemail", "junkemail"),
    ] {
        let pool = seed_pool().await;
        sqlx::query("UPDATE accounts SET provider = 'outlook'")
            .execute(&pool)
            .await
            .unwrap();
        remote_mutations::enqueue_thread_move_to_kind_tracked(
            &pool,
            &SyncQueue::new(),
            &["th-1".into()],
            kind,
        )
        .await
        .unwrap();
        let actual: (String, String, String) =
            sqlx::query_as("SELECT name, full_path, remote_id FROM folders WHERE folder_type = ?")
                .bind(kind)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(actual, (name.into(), path.into(), remote.into()));
    }
}

#[tokio::test]
async fn pending_destination_cannot_be_deleted_or_renamed() {
    let pool = seed_pool().await;
    sqlx::query("INSERT INTO folders (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at) VALUES ('target', 'acc-1', 'Target', 'Target', 'custom', 'Target', '2026-01-01', '2026-01-01')").execute(&pool).await.unwrap();
    let queue = SyncQueue::new();
    remote_mutations::enqueue_thread_move_to_folder(&pool, &queue, &["th-1".into()], "target")
        .await
        .unwrap();
    let folders = satchel_core::db::folders::FolderRepository::new(&pool, &queue);
    assert!(matches!(
        folders.delete("target").await,
        Err(satchel_core::db::folders::FolderError::ContainsMessages { .. })
    ));
    assert!(matches!(
        folders.rename("target", "Renamed").await,
        Err(satchel_core::db::folders::FolderError::PendingMove)
    ));
    let unexpected: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_queue WHERE json_extract(payload, '$.kind') IN ('delete_folder', 'rename_folder')").fetch_one(&pool).await.unwrap();
    assert_eq!(
        unexpected, 0,
        "rejected folder changes must not reach the provider"
    );
}

#[tokio::test]
async fn purging_an_account_discards_its_pending_destination() {
    let pool = seed_pool().await;
    let queue = SyncQueue::new();
    satchel_core::mail_actions::move_threads(
        &pool,
        &queue,
        &["th-1".into()],
        satchel_core::mail_actions::MoveTarget::Archive,
    )
    .await
    .unwrap();
    assert!(queue.purge_account(&pool, "acc-1").await.unwrap() > 0);
    let identity: (String, Option<String>) =
        sqlx::query_as("SELECT folder_id, pending_folder_id FROM messages WHERE id = 'msg-1'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(identity, ("fld-inbox".into(), None));
}

#[tokio::test]
async fn discarding_a_failed_move_restores_confirmed_folder_and_unblocks_flags() {
    let pool = seed_pool().await;
    let queue = SyncQueue::new();
    sqlx::query("INSERT INTO folders (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at) VALUES ('target', 'acc-1', 'Target', 'Target', 'custom', 'Target', '2026-01-01', '2026-01-01')").execute(&pool).await.unwrap();
    remote_mutations::enqueue_thread_move_to_folder(&pool, &queue, &["th-1".into()], "target")
        .await
        .unwrap();
    let id: String = sqlx::query_scalar("SELECT id FROM sync_queue LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(!satchel_core::mail_actions::discard_failed_move(&pool, &id)
        .await
        .unwrap());
    queue
        .mark_failed_permanent(&pool, &id, "folder removed")
        .await
        .unwrap();
    assert_eq!(queue.list_failed_moves(&pool).await.unwrap().len(), 1);
    assert!(remote_mutations::enqueue_thread_flags(
        &pool,
        &queue,
        &["th-1".into()],
        Some(true),
        None
    )
    .await
    .is_err());
    assert!(satchel_core::mail_actions::discard_failed_move(&pool, &id)
        .await
        .unwrap());
    let identity: (String, String, Option<String>) = sqlx::query_as(
        "SELECT remote_id, folder_id, pending_folder_id FROM messages WHERE id = 'msg-1'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(identity, ("42".into(), "fld-inbox".into(), None));
    assert!(queue.list_failed_moves(&pool).await.unwrap().is_empty());
    remote_mutations::enqueue_thread_flags(&pool, &queue, &["th-1".into()], Some(true), None)
        .await
        .unwrap();
    assert!(!satchel_core::mail_actions::discard_failed_move(&pool, &id)
        .await
        .unwrap());
}
