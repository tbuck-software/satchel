use satchel_core::db::topics::{
    AddTopicItemsRequest, RemoveTopicItemRequest, TopicAddedBy, TopicError, TopicFileWriteRequest,
    TopicItemType, TopicMembershipsQuery, TopicNoteWriteRequest, TopicRepository, TopicStatus,
    TopicTimelineItemKind, TopicUpsertRequest,
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

async fn seed_thread(pool: &SqlitePool) {
    sqlx::query(
        "INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, sync_state, color, is_active, created_at, updated_at)
         VALUES ('acc-1', 'Me', 'me@example.com', 'imap', '{}', '', '{}', '#000', 1, '2026-01-01', '2026-01-01')",
    )
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO folders (id, account_id, name, full_path, folder_type, remote_id, message_count, unread_count, sync_enabled, sort_order, created_at, updated_at)
         VALUES ('fld-inbox', 'acc-1', 'Inbox', 'Inbox', 'inbox', 'Inbox', 0, 0, 1, 0, '2026-01-01', '2026-01-01')",
    )
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO threads (id, account_id, subject, last_message_date, message_count, unread_count, has_attachments, participants, snippet, workflow_state, workflow_changed_at, created_at, updated_at)
         VALUES ('th-1', 'acc-1', 'Projektstart', '2026-01-03T10:00:00Z', 1, 1, 0, '[]', 'los gehts', 'inbox', '2026-01-03', '2026-01-03', '2026-01-03')",
    )
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO messages (id, account_id, folder_id, thread_id, message_id_header, from_address, to_addresses, subject, date, body_text, snippet, is_read, is_starred, is_draft, is_sent, has_attachments, created_at, updated_at)
         VALUES ('m-1', 'acc-1', 'fld-inbox', 'th-1', '<m-1@example.com>', 'mike@example.com', '[\"me@example.com\"]', 'Projektstart', '2026-01-03T10:00:00Z', 'los gehts', 'los gehts', 0, 0, 0, 0, 0, '2026-01-03', '2026-01-03')",
    )
    .execute(pool)
    .await
    .unwrap();
    // A second thread/message pair for message-level linking.
    sqlx::query(
        "INSERT INTO threads (id, account_id, subject, last_message_date, message_count, unread_count, has_attachments, participants, snippet, workflow_state, workflow_changed_at, created_at, updated_at)
         VALUES ('th-2', 'acc-1', 'Randnotiz', '2026-01-02T09:00:00Z', 1, 0, 0, '[]', 'nebenbei', 'inbox', '2026-01-02', '2026-01-02', '2026-01-02')",
    )
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO messages (id, account_id, folder_id, thread_id, message_id_header, from_address, to_addresses, subject, date, body_text, snippet, is_read, is_starred, is_draft, is_sent, has_attachments, created_at, updated_at)
         VALUES ('m-2', 'acc-1', 'fld-inbox', 'th-2', '<m-2@example.com>', 'mike@example.com', '[\"me@example.com\"]', 'Randnotiz', '2026-01-02T09:00:00Z', 'nebenbei', 'nebenbei', 1, 0, 0, 0, 0, '2026-01-02', '2026-01-02')",
    )
    .execute(pool)
    .await
    .unwrap();
}

fn topic_input(name: &str) -> TopicUpsertRequest {
    TopicUpsertRequest {
        id: None,
        name: name.into(),
        description: None,
        color: None,
        icon: None,
        status: None,
        due_date: None,
    }
}

fn add_input(topic_id: &str, item_type: &str, item_ids: &[&str]) -> AddTopicItemsRequest {
    AddTopicItemsRequest {
        topic_id: topic_id.into(),
        item_type: item_type.try_into().unwrap(),
        item_ids: item_ids.iter().map(|s| s.to_string()).collect(),
        added_by: None,
    }
}

#[tokio::test]
async fn upsert_creates_then_updates() {
    let pool = pool().await;
    let repository = TopicRepository::new(&pool);
    let created = repository.upsert(topic_input("Umzug")).await.unwrap();
    assert_eq!(created.status, TopicStatus::Active);
    assert_eq!(created.item_count, 0);

    let updated = repository
        .upsert(TopicUpsertRequest {
            id: Some(created.id.clone()),
            name: "Umzug Büro".into(),
            description: Some("Alles zum Büroumzug".into()),
            color: None,
            icon: None,
            status: Some(TopicStatus::Done),
            due_date: None,
        })
        .await
        .unwrap();
    assert_eq!(updated.id, created.id);
    assert_eq!(updated.name, "Umzug Büro");
    assert_eq!(updated.status, TopicStatus::Done);
    assert_eq!(updated.created_at, created.created_at);

    assert!(repository.upsert(topic_input("   ")).await.is_err());
    assert!(TopicStatus::try_from("closed").is_err());
}

#[tokio::test]
async fn add_topic_items_is_idempotent() {
    let pool = pool().await;
    seed_thread(&pool).await;
    let repository = TopicRepository::new(&pool);
    let topic = repository.upsert(topic_input("Umzug")).await.unwrap();
    repository
        .add_items(add_input(&topic.id, "thread", &["th-1"]))
        .await
        .unwrap();
    repository
        .add_items(add_input(&topic.id, "thread", &["th-1"]))
        .await
        .unwrap();
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM topic_items")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
    let link: (String, String, String) =
        sqlx::query_as("SELECT topic_id, item_type, item_id FROM topic_items WHERE topic_id = ?")
            .bind(&topic.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(link, (topic.id, "thread".into(), "th-1".into()));

    assert!(TopicItemType::try_from("folder").is_err());
}

#[tokio::test]
async fn remove_thread_links_is_idempotent_and_only_removes_thread_links() {
    let pool = pool().await;
    let repository = TopicRepository::new(&pool);
    let topic = repository.upsert(topic_input("Umzug")).await.unwrap();

    sqlx::query(
        "INSERT INTO topic_items (topic_id, item_type, item_id, added_at, added_by)
         VALUES (?, 'thread', 'thread-old', '2026-01-01T10:00:00Z', 'rule'),
                (?, 'message', 'message-1', '2026-01-01T11:00:00Z', 'user')",
    )
    .bind(&topic.id)
    .bind(&topic.id)
    .execute(&pool)
    .await
    .unwrap();

    repository.remove_thread_links("thread-old").await.unwrap();
    repository
        .remove_thread_links("thread-missing")
        .await
        .unwrap();

    let remaining: Vec<(String, String)> =
        sqlx::query_as("SELECT item_type, item_id FROM topic_items ORDER BY item_type, item_id")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(remaining, vec![("message".into(), "message-1".into())]);
}

#[tokio::test]
async fn transfer_thread_links_preserves_metadata_deduplicates_and_is_idempotent() {
    let pool = pool().await;
    let repository = TopicRepository::new(&pool);
    let topic_a = repository.upsert(topic_input("Umzug")).await.unwrap();
    let topic_b = repository.upsert(topic_input("Steuern")).await.unwrap();

    sqlx::query(
        "INSERT INTO topic_items (topic_id, item_type, item_id, added_at, added_by)
         VALUES (?, 'thread', 'thread-child', '2026-01-01T10:00:00Z', 'rule'),
                (?, 'thread', 'thread-child', '2026-01-02T10:00:00Z', 'llm'),
                (?, 'thread', 'thread-parent', '2026-01-03T10:00:00Z', 'user')",
    )
    .bind(&topic_a.id)
    .bind(&topic_b.id)
    .bind(&topic_a.id)
    .execute(&pool)
    .await
    .unwrap();

    repository
        .transfer_thread_links("thread-child", "thread-parent")
        .await
        .unwrap();

    let links: Vec<(String, String, String, String)> = sqlx::query_as(
        "SELECT topic_id, item_id, added_at, added_by
         FROM topic_items WHERE item_type = 'thread' ORDER BY topic_id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    let mut expected = vec![
        (
            topic_a.id.clone(),
            "thread-parent".into(),
            "2026-01-03T10:00:00Z".into(),
            "user".into(),
        ),
        (
            topic_b.id.clone(),
            "thread-parent".into(),
            "2026-01-02T10:00:00Z".into(),
            "llm".into(),
        ),
    ];
    expected.sort_by(|left, right| left.0.cmp(&right.0));
    assert_eq!(links, expected);

    // Missing sources and equal ids must not disturb already transferred
    // links, and both operations are valid no-ops.
    repository
        .transfer_thread_links("thread-missing", "thread-parent")
        .await
        .unwrap();
    repository
        .transfer_thread_links("thread-parent", "thread-parent")
        .await
        .unwrap();
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM topic_items")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 2);
}

#[tokio::test]
async fn timeline_interleaves_threads_messages_and_notes() {
    let pool = pool().await;
    seed_thread(&pool).await;
    let repository = TopicRepository::new(&pool);
    let topic = repository.upsert(topic_input("Umzug")).await.unwrap();
    repository
        .add_items(add_input(&topic.id, "thread", &["th-1"]))
        .await
        .unwrap();
    repository
        .add_items(add_input(&topic.id, "message", &["m-2"]))
        .await
        .unwrap();
    let note_new = repository
        .upsert_note(TopicNoteWriteRequest {
            id: None,
            topic_id: topic.id.clone(),
            content: "Telefonat mit Mike: Termin steht".into(),
        })
        .await
        .unwrap();
    sqlx::query("UPDATE topic_notes SET created_at = '2026-01-04T08:00:00Z' WHERE id = ?")
        .bind(&note_new.id)
        .execute(&pool)
        .await
        .unwrap();
    let note_old = repository
        .upsert_note(TopicNoteWriteRequest {
            id: None,
            topic_id: topic.id.clone(),
            content: "Erster Gedanke".into(),
        })
        .await
        .unwrap();
    sqlx::query("UPDATE topic_notes SET created_at = '2026-01-01T08:00:00Z' WHERE id = ?")
        .bind(&note_old.id)
        .execute(&pool)
        .await
        .unwrap();

    let timeline = repository.timeline(&topic.id).await.unwrap();
    let kinds: Vec<(&str, &str)> = timeline
        .iter()
        .map(|item| (item.item_type.as_str(), item.item_id.as_str()))
        .collect();
    assert_eq!(
        kinds,
        vec![
            ("note", note_new.id.as_str()),
            ("thread", "th-1"),
            ("message", "m-2"),
            ("note", note_old.id.as_str()),
        ]
    );

    let thread_row = &timeline[1];
    assert_eq!(thread_row.title.as_deref(), Some("Projektstart"));
    assert_eq!(thread_row.thread_id.as_deref(), Some("th-1"));
    assert_eq!(thread_row.unread_count, 1);
    assert!(thread_row.content.is_none());

    let note_row = &timeline[0];
    assert_eq!(
        note_row.content.as_deref(),
        Some("Telefonat mit Mike: Termin steht")
    );
    assert!(note_row.thread_id.is_none());

    let message_row = &timeline[2];
    assert_eq!(message_row.thread_id.as_deref(), Some("th-2"));
    assert_eq!(message_row.unread_count, 0);
}

#[tokio::test]
async fn list_topics_reports_newest_linked_sender() {
    let pool = pool().await;
    seed_thread(&pool).await;
    sqlx::query(
        "UPDATE messages SET from_name = 'Sarah', from_address = 'sarah@app.io' WHERE id = 'm-1'",
    )
    .execute(&pool)
    .await
    .unwrap();
    let repository = TopicRepository::new(&pool);
    let topic = repository.upsert(topic_input("Umzug")).await.unwrap();
    assert_eq!(topic.last_sender_email, None);

    repository
        .add_items(add_input(&topic.id, "message", &["m-2"]))
        .await
        .unwrap();
    let listed = repository.list().await.unwrap();
    assert_eq!(
        listed[0].last_sender_email.as_deref(),
        Some("mike@example.com")
    );

    repository
        .add_items(add_input(&topic.id, "thread", &["th-1"]))
        .await
        .unwrap();
    let listed = repository.list().await.unwrap();
    assert_eq!(listed[0].last_sender_name.as_deref(), Some("Sarah"));
    assert_eq!(listed[0].last_sender_email.as_deref(), Some("sarah@app.io"));

    // Drafts never represent a topic.
    sqlx::query("UPDATE messages SET is_draft = 1 WHERE id = 'm-1'")
        .execute(&pool)
        .await
        .unwrap();
    let listed = repository.list().await.unwrap();
    assert_eq!(
        listed[0].last_sender_email.as_deref(),
        Some("mike@example.com")
    );
}

#[tokio::test]
async fn delete_topic_cascades_items_and_notes() {
    let pool = pool().await;
    seed_thread(&pool).await;
    let repository = TopicRepository::new(&pool);
    let topic = repository.upsert(topic_input("Umzug")).await.unwrap();
    repository
        .add_items(add_input(&topic.id, "thread", &["th-1"]))
        .await
        .unwrap();
    repository
        .upsert_note(TopicNoteWriteRequest {
            id: None,
            topic_id: topic.id.clone(),
            content: "Notiz".into(),
        })
        .await
        .unwrap();

    repository.delete(&topic.id).await.unwrap();
    let items: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM topic_items")
        .fetch_one(&pool)
        .await
        .unwrap();
    let notes: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM topic_notes")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!((items, notes), (0, 0));
    let threads: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM threads WHERE id = 'th-1'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(threads, 1);
}

#[tokio::test]
async fn memberships_report_thread_in_two_topics() {
    let pool = pool().await;
    seed_thread(&pool).await;
    let repository = TopicRepository::new(&pool);
    let a = repository.upsert(topic_input("Umzug")).await.unwrap();
    let b = repository.upsert(topic_input("Steuer")).await.unwrap();
    repository
        .add_items(add_input(&a.id, "thread", &["th-1"]))
        .await
        .unwrap();
    repository
        .add_items(add_input(&b.id, "thread", &["th-1", "th-2"]))
        .await
        .unwrap();

    let memberships = repository
        .list_memberships(TopicMembershipsQuery {
            item_type: TopicItemType::Thread,
            item_ids: vec!["th-1".into()],
        })
        .await
        .unwrap();
    let mut topic_ids: Vec<&str> = memberships
        .iter()
        .map(|entry| entry.topic_id.as_str())
        .collect();
    topic_ids.sort();
    let mut expected = vec![a.id.as_str(), b.id.as_str()];
    expected.sort();
    assert_eq!(topic_ids, expected);
}

#[tokio::test]
async fn deleted_thread_disappears_from_timeline_and_counts() {
    let pool = pool().await;
    seed_thread(&pool).await;
    let repository = TopicRepository::new(&pool);
    let topic = repository.upsert(topic_input("Umzug")).await.unwrap();
    repository
        .add_items(add_input(&topic.id, "thread", &["th-1"]))
        .await
        .unwrap();

    sqlx::query("DELETE FROM messages WHERE thread_id = 'th-1'")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM threads WHERE id = 'th-1'")
        .execute(&pool)
        .await
        .unwrap();

    let timeline = repository.timeline(&topic.id).await.unwrap();
    assert!(timeline.is_empty());
    let listed = repository.list().await.unwrap();
    assert_eq!(listed[0].item_count, 0);

    let removed = repository.cleanup_orphaned_items().await.unwrap();
    assert_eq!(removed, 1);
    let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM topic_items")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(remaining, 0);
}

#[tokio::test]
async fn topic_files_round_trip_and_cascade() {
    let pool = pool().await;
    let repository = TopicRepository::new(&pool);
    let topic = repository.upsert(topic_input("Umzug")).await.unwrap();
    let file = repository
        .add_file(TopicFileWriteRequest {
            topic_id: topic.id.clone(),
            filename: "liste.csv".into(),
            mime_type: Some("text/csv".into()),
            content: b"hallo".to_vec(),
        })
        .await
        .unwrap();
    assert_eq!(file.size, 5);
    assert_eq!(file.mime_type, "text/csv");

    let downloaded = repository.download_file(&file.id).await.unwrap();
    assert_eq!(downloaded.filename, "liste.csv");
    assert_eq!(downloaded.mime_type, "text/csv");
    assert_eq!(downloaded.content, b"hallo");

    assert!(repository
        .add_file(TopicFileWriteRequest {
            topic_id: topic.id.clone(),
            filename: "  ".into(),
            mime_type: None,
            content: b"hallo".to_vec(),
        })
        .await
        .is_err());

    repository.delete(&topic.id).await.unwrap();
    let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM topic_files")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(remaining, 0, "deleting the topic must remove its files");
}

#[test]
fn domain_values_reject_unknown_storage_strings() {
    assert!(matches!(
        TopicStatus::try_from("closed"),
        Err(TopicError::InvalidStatus { value }) if value == "closed"
    ));
    assert!(matches!(
        TopicItemType::try_from("folder"),
        Err(TopicError::InvalidItemType { value }) if value == "folder"
    ));
    assert!(matches!(
        TopicAddedBy::try_from("automation"),
        Err(TopicError::InvalidAddedBy { value }) if value == "automation"
    ));
}

#[tokio::test]
async fn remove_items_and_event_links_keep_membership_semantics() {
    let pool = pool().await;
    seed_thread(&pool).await;
    let repository = TopicRepository::new(&pool);
    let topic = repository.upsert(topic_input("Umzug")).await.unwrap();

    repository
        .add_items(add_input(&topic.id, "thread", &["th-1"]))
        .await
        .unwrap();
    repository
        .remove_item(satchel_core::db::topics::RemoveTopicItemRequest {
            topic_id: topic.id.clone(),
            item_type: TopicItemType::Thread,
            item_id: "th-1".into(),
        })
        .await
        .unwrap();
    let removed = repository
        .list_memberships(TopicMembershipsQuery {
            item_type: TopicItemType::Thread,
            item_ids: vec!["th-1".into()],
        })
        .await
        .unwrap();
    assert!(removed.is_empty());

    repository
        .add_items(AddTopicItemsRequest {
            topic_id: topic.id.clone(),
            item_type: TopicItemType::Event,
            item_ids: vec!["event-1".into()],
            added_by: Some(TopicAddedBy::Rule),
        })
        .await
        .unwrap();
    let event_membership = repository
        .list_memberships(TopicMembershipsQuery {
            item_type: TopicItemType::Event,
            item_ids: vec!["event-1".into()],
        })
        .await
        .unwrap();
    assert_eq!(event_membership.len(), 1);
    assert!(repository.timeline(&topic.id).await.unwrap().is_empty());
    assert_eq!(repository.list().await.unwrap()[0].item_count, 1);
    assert_eq!(repository.cleanup_orphaned_items().await.unwrap(), 0);
    assert_eq!(
        repository
            .list_memberships(TopicMembershipsQuery {
                item_type: TopicItemType::Event,
                item_ids: vec!["event-1".into()],
            })
            .await
            .unwrap()
            .len(),
        1
    );

    assert!(repository
        .list_memberships(TopicMembershipsQuery {
            item_type: TopicItemType::Event,
            item_ids: Vec::new(),
        })
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn notes_update_and_delete_without_affecting_other_notes() {
    let pool = pool().await;
    let repository = TopicRepository::new(&pool);
    let topic = repository.upsert(topic_input("Umzug")).await.unwrap();
    let note = repository
        .upsert_note(TopicNoteWriteRequest {
            id: None,
            topic_id: topic.id.clone(),
            content: "Erster Stand".into(),
        })
        .await
        .unwrap();
    let updated = repository
        .upsert_note(TopicNoteWriteRequest {
            id: Some(note.id.clone()),
            topic_id: topic.id.clone(),
            content: "Aktualisiert".into(),
        })
        .await
        .unwrap();
    assert_eq!(updated.id, note.id);
    assert_eq!(updated.content, "Aktualisiert");
    assert_eq!(updated.created_at, note.created_at);
    assert!(matches!(
        repository
            .upsert_note(TopicNoteWriteRequest {
                id: None,
                topic_id: topic.id.clone(),
                content: "  ".into(),
            })
            .await,
        Err(TopicError::NoteContentEmpty)
    ));

    repository.delete_note(&note.id).await.unwrap();
    repository.delete_note("note-missing").await.unwrap();
    assert!(repository.timeline(&topic.id).await.unwrap().is_empty());
}

#[tokio::test]
async fn timeline_uses_item_id_as_deterministic_tie_breaker() {
    let pool = pool().await;
    let repository = TopicRepository::new(&pool);
    let topic = repository.upsert(topic_input("Umzug")).await.unwrap();
    repository
        .upsert_note(TopicNoteWriteRequest {
            id: Some("note-a".into()),
            topic_id: topic.id.clone(),
            content: "A".into(),
        })
        .await
        .unwrap();
    repository
        .upsert_note(TopicNoteWriteRequest {
            id: Some("note-b".into()),
            topic_id: topic.id.clone(),
            content: "B".into(),
        })
        .await
        .unwrap();
    sqlx::query("UPDATE topic_notes SET created_at = '2026-01-01T00:00:00Z' WHERE topic_id = ?")
        .bind(&topic.id)
        .execute(&pool)
        .await
        .unwrap();

    let timeline = repository.timeline(&topic.id).await.unwrap();
    assert_eq!(timeline.len(), 2);
    assert_eq!(timeline[0].item_type, TopicTimelineItemKind::Note);
    assert_eq!(timeline[0].item_id, "note-b");
    assert_eq!(timeline[1].item_id, "note-a");
}

#[tokio::test]
async fn files_default_mime_support_empty_content_and_report_missing_downloads() {
    let pool = pool().await;
    let repository = TopicRepository::new(&pool);
    let topic = repository.upsert(topic_input("Umzug")).await.unwrap();
    let file = repository
        .add_file(TopicFileWriteRequest {
            topic_id: topic.id,
            filename: "leer.bin".into(),
            mime_type: Some("  ".into()),
            content: Vec::new(),
        })
        .await
        .unwrap();
    assert_eq!(file.mime_type, "application/octet-stream");
    assert_eq!(file.size, 0);
    let downloaded = repository.download_file(&file.id).await.unwrap();
    assert!(downloaded.content.is_empty());
    assert!(matches!(
        repository.download_file("tfile-missing").await,
        Err(TopicError::FileNotFound)
    ));
    repository.delete_file("tfile-missing").await.unwrap();
}

#[test]
fn timeline_item_kind_keeps_notes_outside_membership_item_types() {
    assert_eq!(TopicTimelineItemKind::Note.as_str(), "note");
    assert!(TopicItemType::try_from("note").is_err());
}

#[tokio::test]
async fn membership_receipts_cover_duplicates_concurrent_adds_and_removals() {
    let pool = pool().await;
    seed_thread(&pool).await;
    let repository = TopicRepository::new(&pool);
    let topic = repository.upsert(topic_input("Receipt")).await.unwrap();
    let (first, second) = tokio::join!(
        repository.add_items(add_input(&topic.id, "thread", &["th-1", "th-1"])),
        repository.add_items(add_input(&topic.id, "thread", &["th-1"]))
    );
    let receipts: Vec<String> = first
        .unwrap()
        .into_iter()
        .chain(second.unwrap())
        .map(|r| r.item_id)
        .collect();
    assert_eq!(receipts, vec!["th-1"]);
    let remove = || RemoveTopicItemRequest {
        topic_id: topic.id.clone(),
        item_type: TopicItemType::Thread,
        item_id: "th-1".into(),
    };
    assert!(repository.remove_item(remove()).await.unwrap().is_some());
    assert!(repository.remove_item(remove()).await.unwrap().is_none());
}

#[tokio::test]
async fn membership_undo_rejects_aba_replay_and_independent_writers() {
    let pool = pool().await;
    seed_thread(&pool).await;
    let repository = TopicRepository::new(&pool);
    let topic = repository.upsert(topic_input("Guarded")).await.unwrap();
    let remove = || RemoveTopicItemRequest {
        topic_id: topic.id.clone(),
        item_type: TopicItemType::Thread,
        item_id: "th-1".into(),
    };
    let first_add = repository
        .add_items(add_input(&topic.id, "thread", &["th-1"]))
        .await
        .unwrap();
    let first_remove = repository.remove_item(remove()).await.unwrap().unwrap();
    let second_add = repository
        .add_items(add_input(&topic.id, "thread", &["th-1"]))
        .await
        .unwrap();
    assert_ne!(first_add[0].revision, second_add[0].revision);
    assert!(repository
        .undo_memberships(&topic.id, TopicItemType::Thread, &first_add, false)
        .await
        .unwrap()
        .is_empty());
    // An independent SQL writer also invalidates the receipt, without a store cache.
    sqlx::query("DELETE FROM topic_items WHERE topic_id = ?")
        .bind(&topic.id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(repository
        .undo_memberships(&topic.id, TopicItemType::Thread, &[first_remove], true)
        .await
        .unwrap()
        .is_empty());
    let latest = repository
        .add_items(add_input(&topic.id, "thread", &["th-1", "th-1"]))
        .await
        .unwrap();
    assert_eq!(latest.len(), 1);
    assert_eq!(
        repository
            .undo_memberships(&topic.id, TopicItemType::Thread, &latest, false)
            .await
            .unwrap(),
        vec!["th-1"]
    );
    assert!(repository
        .undo_memberships(&topic.id, TopicItemType::Thread, &latest, false)
        .await
        .unwrap()
        .is_empty());
    let added = repository
        .add_items(add_input(&topic.id, "thread", &["th-1"]))
        .await
        .unwrap();
    sqlx::query("UPDATE topic_items SET added_by = 'rule' WHERE topic_id = ?")
        .bind(&topic.id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(repository
        .undo_memberships(&topic.id, TopicItemType::Thread, &added, false)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn membership_undo_is_partial_batched_and_preserves_removed_metadata() {
    let pool = pool().await;
    let repository = TopicRepository::new(&pool);
    let topic = repository.upsert(topic_input("Events")).await.unwrap();
    let mut input = add_input(&topic.id, "event", &["a", "b", "c"]);
    input.added_by = Some(TopicAddedBy::Rule);
    let added = repository.add_items(input).await.unwrap();
    let b = repository
        .remove_item(RemoveTopicItemRequest {
            topic_id: topic.id.clone(),
            item_type: TopicItemType::Event,
            item_id: "b".into(),
        })
        .await
        .unwrap()
        .unwrap();
    // Only still-owned members are removed, all under one transaction.
    assert_eq!(
        repository
            .undo_memberships(&topic.id, TopicItemType::Event, &added, false)
            .await
            .unwrap(),
        vec!["a", "c"]
    );
    assert_eq!(
        repository
            .undo_memberships(
                &topic.id,
                TopicItemType::Event,
                std::slice::from_ref(&b),
                true
            )
            .await
            .unwrap(),
        vec!["b"]
    );
    assert!(repository
        .undo_memberships(&topic.id, TopicItemType::Event, &[b], true)
        .await
        .unwrap()
        .is_empty());
    let by: String =
        sqlx::query_scalar("SELECT added_by FROM topic_items WHERE topic_id = ? AND item_id = 'b'")
            .bind(&topic.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(by, "rule");
    repository.delete(&topic.id).await.unwrap();
    assert!(repository
        .undo_memberships(&topic.id, TopicItemType::Event, &added, true)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn membership_undo_and_another_connection_serialize_the_currentness_check() {
    let directory = tempfile::tempdir().unwrap();
    let url = format!(
        "sqlite://{}?mode=rwc",
        directory.path().join("topics.db").display()
    );
    let first = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .unwrap();
    sqlx::migrate!("./src/db/migrations")
        .run(&first)
        .await
        .unwrap();
    let second = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .unwrap();
    let repo = TopicRepository::new(&first);
    let other = TopicRepository::new(&second);
    let topic = repo.upsert(topic_input("Two windows")).await.unwrap();
    for _ in 0..10 {
        let receipt = repo
            .add_items(add_input(&topic.id, "event", &["event"]))
            .await
            .unwrap();
        let writer = async {
            other
                .remove_item(RemoveTopicItemRequest {
                    topic_id: topic.id.clone(),
                    item_type: TopicItemType::Event,
                    item_id: "event".into(),
                })
                .await
                .unwrap();
            other
                .add_items(add_input(&topic.id, "event", &["event"]))
                .await
                .unwrap();
        };
        let (undo, ()) = tokio::join!(
            repo.undo_memberships(&topic.id, TopicItemType::Event, &receipt, false),
            writer
        );
        undo.unwrap();
        assert_eq!(
            repo.list_memberships(TopicMembershipsQuery {
                item_type: TopicItemType::Event,
                item_ids: vec!["event".into()]
            })
            .await
            .unwrap()
            .len(),
            1
        );
        repo.remove_item(RemoveTopicItemRequest {
            topic_id: topic.id.clone(),
            item_type: TopicItemType::Event,
            item_id: "event".into(),
        })
        .await
        .unwrap();
    }
}
