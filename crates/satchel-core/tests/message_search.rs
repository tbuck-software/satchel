use sqlx::sqlite::SqlitePoolOptions;
use sqlx::SqlitePool;

use satchel_core::db::data_repairs::{is_pending, run_repair, CC_INDEX};
use satchel_core::db::messages::{MessageSearchRepository, SearchQuery};

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

async fn seed_account(pool: &SqlitePool) {
    sqlx::query(
        "INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, sync_state, color, is_active, created_at, updated_at)
         VALUES ('acc-1', 'Me', 'me@example.com', 'imap', '{}', '', '{}', '#000', 1, '2026-01-01', '2026-01-01')",
    )
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO folders (id, account_id, name, full_path, folder_type, remote_id, message_count, unread_count, sync_enabled, sort_order, created_at, updated_at)
         VALUES ('fld-inbox', 'acc-1', 'Inbox', 'INBOX', 'inbox', 'INBOX', 0, 0, 1, 0, '2026-01-01', '2026-01-01')",
    )
    .execute(pool)
    .await
    .unwrap();
}

struct SeedMessage<'a> {
    id: &'a str,
    subject: &'a str,
    body_text: &'a str,
    from_name: &'a str,
    from_address: &'a str,
    to_addresses: &'a str,
    cc_addresses: Option<&'a str>,
    date: &'a str,
    is_read: bool,
    /// Defaults to a fresh thread per message (`th-<id>`).
    thread_id: Option<&'a str>,
}

impl Default for SeedMessage<'_> {
    fn default() -> Self {
        Self {
            id: "m-1",
            subject: "Subject",
            body_text: "Body",
            from_name: "Sender",
            from_address: "sender@example.com",
            to_addresses: r#"["me@example.com"]"#,
            cc_addresses: None,
            date: "2026-01-01T00:00:00Z",
            is_read: true,
            thread_id: None,
        }
    }
}

async fn seed_message(pool: &SqlitePool, message: SeedMessage<'_>) {
    let thread_id = message
        .thread_id
        .map(str::to_string)
        .unwrap_or_else(|| format!("th-{}", message.id));
    sqlx::query(
        "INSERT OR IGNORE INTO threads (id, account_id, subject, last_message_date, message_count, unread_count, has_attachments, participants, snippet, workflow_state, workflow_changed_at, created_at, updated_at)
         VALUES (?, 'acc-1', ?, ?, 1, 0, 0, '[]', '', 'inbox', '2026-01-01', '2026-01-01', '2026-01-01')",
    )
    .bind(&thread_id)
    .bind(message.subject)
    .bind(message.date)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO messages (id, account_id, folder_id, thread_id, message_id_header, from_address, from_name, to_addresses, cc_addresses, subject, date, body_text, snippet, is_read, is_starred, is_draft, is_sent, has_attachments, created_at, updated_at)
         VALUES (?, 'acc-1', 'fld-inbox', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 0, 0, 0, 0, '2026-01-01', '2026-01-01')",
    )
    .bind(message.id)
    .bind(&thread_id)
    .bind(format!("<{}@example.com>", message.id))
    .bind(message.from_address)
    .bind(message.from_name)
    .bind(message.to_addresses)
    .bind(message.cc_addresses)
    .bind(message.subject)
    .bind(message.date)
    .bind(message.body_text)
    .bind(message.body_text)
    .bind(message.is_read)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE threads
         SET last_message_date = (
                 SELECT MAX(date) FROM messages WHERE thread_id = ?
             ),
             message_count = (
                 SELECT COUNT(*) FROM messages WHERE thread_id = ?
             ),
             unread_count = (
                 SELECT COUNT(*) FROM messages WHERE thread_id = ? AND is_read = 0
             )
         WHERE id = ?",
    )
    .bind(&thread_id)
    .bind(&thread_id)
    .bind(&thread_id)
    .bind(&thread_id)
    .execute(pool)
    .await
    .unwrap();
}

fn query(raw: &str) -> SearchQuery {
    SearchQuery {
        query: raw.to_string(),
        account_id: None,
        limit: None,
        cursor: None,
    }
}

#[tokio::test]
async fn prefix_matches_partial_last_term() {
    let pool = pool().await;
    seed_account(&pool).await;
    seed_message(
        &pool,
        SeedMessage {
            subject: "Rechnung März",
            ..Default::default()
        },
    )
    .await;

    let hits = MessageSearchRepository::new(&pool)
        .search(&query("rechn"))
        .await
        .unwrap()
        .hits;
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].subject, "Rechnung März");
}

#[tokio::test]
async fn umlaut_prefix_matches_diacritic_terms() {
    let pool = pool().await;
    seed_account(&pool).await;
    seed_message(
        &pool,
        SeedMessage {
            subject: "Rechnung März",
            ..Default::default()
        },
    )
    .await;

    // remove_diacritics 2 normalizes index and query alike.
    for raw in ["mä", "ma", "märz", "marz"] {
        let hits = MessageSearchRepository::new(&pool)
            .search(&query(raw))
            .await
            .unwrap()
            .hits;
        assert_eq!(hits.len(), 1, "expected {raw:?} to match");
    }
}

#[tokio::test]
async fn ranks_subject_matches_above_body_matches() {
    let pool = pool().await;
    seed_account(&pool).await;
    // Same date on purpose: this test isolates the bm25 column weighting; the
    // age-penalty tradeoff is covered by
    // `recent_sender_match_outranks_stale_subject_match`.
    seed_message(
        &pool,
        SeedMessage {
            id: "m-subject",
            subject: "Rechnung Dezember",
            body_text: "Viele Grüße",
            date: "2026-01-01T00:00:00Z",
            ..Default::default()
        },
    )
    .await;
    seed_message(
        &pool,
        SeedMessage {
            id: "m-body",
            subject: "Kurze Frage",
            body_text: "Die Rechnung kommt später",
            date: "2026-01-01T00:00:00Z",
            ..Default::default()
        },
    )
    .await;

    let hits = MessageSearchRepository::new(&pool)
        .search(&query("rechnung"))
        .await
        .unwrap()
        .hits;
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].id, "m-subject");
    assert_eq!(hits[1].id, "m-body");
}

#[tokio::test]
async fn indexes_recipient_addresses() {
    let pool = pool().await;
    seed_account(&pool).await;
    seed_message(
        &pool,
        SeedMessage {
            subject: "Ohne Treffer im Betreff",
            body_text: "Ohne Treffer im Text",
            to_addresses: r#"["anna@example.com"]"#,
            ..Default::default()
        },
    )
    .await;

    // Proves both the 0036 rebuild (column exists) and the insert trigger.
    let hits = MessageSearchRepository::new(&pool)
        .search(&query("anna"))
        .await
        .unwrap()
        .hits;
    assert_eq!(hits.len(), 1);
}

#[tokio::test]
async fn punctuation_only_terms_return_empty_not_error() {
    let pool = pool().await;
    seed_account(&pool).await;
    seed_message(
        &pool,
        SeedMessage {
            is_read: false,
            ..Default::default()
        },
    )
    .await;

    // Tokenizes to nothing: must short-circuit, not raise an FTS syntax error.
    let hits = MessageSearchRepository::new(&pool)
        .search(&query("@@@"))
        .await
        .unwrap()
        .hits;
    assert!(hits.is_empty());

    // With an operator present the filter branch still runs.
    let hits = MessageSearchRepository::new(&pool)
        .search(&query("@@@ is:unread"))
        .await
        .unwrap()
        .hits;
    assert_eq!(hits.len(), 1);
}

#[tokio::test]
async fn operator_only_queries_order_by_date_desc() {
    let pool = pool().await;
    seed_account(&pool).await;
    seed_message(
        &pool,
        SeedMessage {
            id: "m-old",
            is_read: false,
            date: "2026-01-01T00:00:00Z",
            ..Default::default()
        },
    )
    .await;
    seed_message(
        &pool,
        SeedMessage {
            id: "m-new",
            is_read: false,
            date: "2026-03-01T00:00:00Z",
            ..Default::default()
        },
    )
    .await;

    let hits = MessageSearchRepository::new(&pool)
        .search(&query("is:unread"))
        .await
        .unwrap()
        .hits;
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].id, "m-new");
    assert_eq!(hits[1].id, "m-old");
}

#[tokio::test]
async fn unread_flag_keeps_one_newest_unread_hit_per_thread() {
    let pool = pool().await;
    seed_account(&pool).await;
    seed_message(
        &pool,
        SeedMessage {
            id: "m-unread-old",
            is_read: false,
            date: "2026-01-01T00:00:00Z",
            thread_id: Some("th-shared"),
            ..Default::default()
        },
    )
    .await;
    seed_message(
        &pool,
        SeedMessage {
            id: "m-unread-new",
            is_read: false,
            date: "2026-02-01T00:00:00Z",
            thread_id: Some("th-shared"),
            ..Default::default()
        },
    )
    .await;
    seed_message(
        &pool,
        SeedMessage {
            id: "m-read-latest",
            is_read: true,
            date: "2026-03-01T00:00:00Z",
            thread_id: Some("th-shared"),
            ..Default::default()
        },
    )
    .await;

    let hits = MessageSearchRepository::new(&pool)
        .search(&query("is:unread"))
        .await
        .unwrap()
        .hits;
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, "m-unread-new");
}

#[tokio::test]
async fn attachment_flag_keeps_one_newest_hit_per_matching_thread() {
    let pool = pool().await;
    seed_account(&pool).await;
    seed_message(
        &pool,
        SeedMessage {
            id: "m-attachment-old",
            date: "2026-01-01T00:00:00Z",
            thread_id: Some("th-shared"),
            ..Default::default()
        },
    )
    .await;
    seed_message(
        &pool,
        SeedMessage {
            id: "m-newest",
            date: "2026-02-01T00:00:00Z",
            thread_id: Some("th-shared"),
            ..Default::default()
        },
    )
    .await;
    sqlx::query("UPDATE messages SET has_attachments = 1 WHERE id = 'm-attachment-old'")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE threads SET has_attachments = 1 WHERE id = 'th-shared'")
        .execute(&pool)
        .await
        .unwrap();

    let hits = MessageSearchRepository::new(&pool)
        .search(&query("has:attachment"))
        .await
        .unwrap()
        .hits;
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, "m-newest");
}

#[tokio::test]
async fn snippet_markers_survive_transport() {
    let pool = pool().await;
    seed_account(&pool).await;
    seed_message(
        &pool,
        SeedMessage {
            subject: "Kurze Frage",
            body_text: "Die Rechnung für den März liegt bei",
            ..Default::default()
        },
    )
    .await;

    let hits = MessageSearchRepository::new(&pool)
        .search(&query("rechnung"))
        .await
        .unwrap()
        .hits;
    assert_eq!(hits.len(), 1);
    assert!(
        hits[0].snippet.contains("\u{1}Rechnung\u{2}"),
        "expected marked match in {:?}",
        hits[0].snippet
    );
}

#[tokio::test]
async fn reports_ignored_operator_values_as_warnings() {
    let pool = pool().await;
    seed_account(&pool).await;
    seed_message(
        &pool,
        SeedMessage {
            subject: "Rechnung",
            ..Default::default()
        },
    )
    .await;

    // Unknown in: value: ignored as a filter, but reported.
    let response = MessageSearchRepository::new(&pool)
        .search(&query("in:xyz rechnung"))
        .await
        .unwrap();
    assert_eq!(response.hits.len(), 1);
    assert_eq!(response.warnings.len(), 1);
    assert_eq!(response.warnings[0].operator, "in");
    assert_eq!(response.warnings[0].value, "xyz");

    // Alone it constrains nothing: empty result, but the warning explains why.
    let response = MessageSearchRepository::new(&pool)
        .search(&query("in:xyz"))
        .await
        .unwrap();
    assert!(response.hits.is_empty());
    assert_eq!(response.warnings.len(), 1);
}

#[tokio::test]
async fn excludes_negated_terms() {
    let pool = pool().await;
    seed_account(&pool).await;
    seed_message(
        &pool,
        SeedMessage {
            id: "m-zoom",
            subject: "Zoom-Meeting von Beispielfirma",
            from_name: "Mara Vogel",
            ..Default::default()
        },
    )
    .await;
    seed_message(
        &pool,
        SeedMessage {
            id: "m-plain",
            subject: "Angebot Website-Relaunch",
            from_name: "Mara Vogel",
            ..Default::default()
        },
    )
    .await;

    let hits = MessageSearchRepository::new(&pool)
        .search(&query("mara -zoom"))
        .await
        .unwrap()
        .hits;
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, "m-plain");

    // Exclusions alone are unbounded: no constraint, no result.
    let response = MessageSearchRepository::new(&pool)
        .search(&query("-zoom"))
        .await
        .unwrap();
    assert!(response.hits.is_empty());
}

#[tokio::test]
async fn recent_sender_match_outranks_stale_subject_match() {
    let pool = pool().await;
    seed_account(&pool).await;
    // Strong subject match, but two years old.
    seed_message(
        &pool,
        SeedMessage {
            id: "m-old",
            subject: "Zoom-Meeting von Beispielfirma | Mara Vogel",
            from_name: "Mara Vogel",
            date: "2024-07-01T00:00:00Z",
            ..Default::default()
        },
    )
    .await;
    // Weaker match (sender only), but from last week.
    seed_message(
        &pool,
        SeedMessage {
            id: "m-new",
            subject: "Kurze Frage zum Angebot",
            from_name: "Mara Vogel",
            date: "2026-07-17T00:00:00Z",
            ..Default::default()
        },
    )
    .await;

    let hits = MessageSearchRepository::new(&pool)
        .search(&query("mara"))
        .await
        .unwrap()
        .hits;
    assert_eq!(hits.len(), 2);
    assert_eq!(
        hits[0].id, "m-new",
        "the age penalty must lift last week's mail above a two-year-old subject match"
    );
}

#[tokio::test]
async fn collapses_hits_to_the_best_one_per_thread() {
    let pool = pool().await;
    seed_account(&pool).await;
    for (id, date) in [
        ("m-a1", "2026-01-01T00:00:00Z"),
        ("m-a2", "2026-02-01T00:00:00Z"),
        ("m-a3", "2026-03-01T00:00:00Z"),
    ] {
        seed_message(
            &pool,
            SeedMessage {
                id,
                subject: "Rechnung Projekt Alpha",
                date,
                thread_id: Some("th-shared"),
                ..Default::default()
            },
        )
        .await;
    }
    seed_message(
        &pool,
        SeedMessage {
            id: "m-other",
            subject: "Rechnung Projekt Beta",
            date: "2026-01-15T00:00:00Z",
            ..Default::default()
        },
    )
    .await;

    let hits = MessageSearchRepository::new(&pool)
        .search(&query("rechnung"))
        .await
        .unwrap()
        .hits;
    // One hit for the three-message thread, one for the other thread.
    assert_eq!(hits.len(), 2);
    let thread_ids: Vec<&str> = hits.iter().map(|hit| hit.thread_id.as_str()).collect();
    assert!(thread_ids.contains(&"th-shared"));
    assert!(thread_ids.contains(&"th-m-other"));
}

#[tokio::test]
async fn fts_survives_flag_updates_and_tracks_content_updates() {
    let pool = pool().await;
    seed_account(&pool).await;
    seed_message(
        &pool,
        SeedMessage {
            subject: "Rechnung März",
            ..Default::default()
        },
    )
    .await;

    // Flag flip: must not touch (or corrupt) the index.
    sqlx::query("UPDATE messages SET is_read = 0 WHERE id = 'm-1'")
        .execute(&pool)
        .await
        .unwrap();
    let hits = MessageSearchRepository::new(&pool)
        .search(&query("rechnung"))
        .await
        .unwrap()
        .hits;
    assert_eq!(hits.len(), 1);

    // Content update: index must follow.
    sqlx::query("UPDATE messages SET subject = 'Angebot April' WHERE id = 'm-1'")
        .execute(&pool)
        .await
        .unwrap();
    let old_hits = MessageSearchRepository::new(&pool)
        .search(&query("rechnung"))
        .await
        .unwrap()
        .hits;
    // Body still contains the default text only; subject term must be gone.
    assert!(old_hits.is_empty());
    let new_hits = MessageSearchRepository::new(&pool)
        .search(&query("angebot"))
        .await
        .unwrap()
        .hits;
    assert_eq!(new_hits.len(), 1);
}

#[tokio::test]
async fn search_projection_tracks_message_lifecycle() {
    let pool = pool().await;
    seed_account(&pool).await;
    seed_message(&pool, SeedMessage::default()).await;

    let projected: (String, String, String) = sqlx::query_as(
        "SELECT message_id, thread_id, date
         FROM message_search_projection
         WHERE message_id = 'm-1'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(projected.0, "m-1");
    assert_eq!(projected.1, "th-m-1");
    assert_eq!(projected.2, "2026-01-01T00:00:00Z");

    sqlx::query("UPDATE messages SET date = '2026-02-01T00:00:00Z' WHERE id = 'm-1'")
        .execute(&pool)
        .await
        .unwrap();
    let projected_date: String =
        sqlx::query_scalar("SELECT date FROM message_search_projection WHERE message_id = 'm-1'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(projected_date, "2026-02-01T00:00:00Z");

    sqlx::query("DELETE FROM messages WHERE id = 'm-1'")
        .execute(&pool)
        .await
        .unwrap();
    let remaining: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM message_search_projection WHERE message_id = 'm-1'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(remaining, 0);
}

#[tokio::test]
async fn quoted_phrases_match_adjacent_words_only() {
    let pool = pool().await;
    seed_account(&pool).await;
    seed_message(
        &pool,
        SeedMessage {
            id: "m-phrase",
            body_text: "Das Zoom Meeting beginnt gleich",
            ..Default::default()
        },
    )
    .await;
    seed_message(
        &pool,
        SeedMessage {
            id: "m-scattered",
            body_text: "Zoom ist toll. Das Meeting fällt aus.",
            ..Default::default()
        },
    )
    .await;

    // Unquoted: both words anywhere → two hits.
    let hits = MessageSearchRepository::new(&pool)
        .search(&query("zoom meeting"))
        .await
        .unwrap()
        .hits;
    assert_eq!(hits.len(), 2);

    // Quoted: adjacent words only → one hit.
    let hits = MessageSearchRepository::new(&pool)
        .search(&query("\"zoom meeting\""))
        .await
        .unwrap()
        .hits;
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, "m-phrase");
}

#[tokio::test]
async fn from_operator_matches_display_names() {
    let pool = pool().await;
    seed_account(&pool).await;
    seed_message(
        &pool,
        SeedMessage {
            from_name: "Mara Vogel | Beispielfirma",
            from_address: "mara@example.com",
            ..Default::default()
        },
    )
    .await;

    let hits = MessageSearchRepository::new(&pool)
        .search(&query("from:\"Mara Vogel\""))
        .await
        .unwrap()
        .hits;
    assert_eq!(hits.len(), 1);
}
#[tokio::test]
async fn from_operator_matches_full_address_exactly_and_fragments_loosely() {
    let pool = pool().await;
    seed_account(&pool).await;
    seed_message(
        &pool,
        SeedMessage {
            id: "m-gabi",
            from_name: "Mara Vogel",
            from_address: "Mara.Vogel@town.example",
            ..Default::default()
        },
    )
    .await;
    seed_message(
        &pool,
        SeedMessage {
            id: "m-other",
            from_name: "Other",
            from_address: "other@example.com",
            ..Default::default()
        },
    )
    .await;

    // Full address, different casing → exact NOCASE match.
    let hits = MessageSearchRepository::new(&pool)
        .search(&query("from:mara.vogel@town.example"))
        .await
        .unwrap()
        .hits;
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, "m-gabi");

    // Name fragment → LIKE fallback still works.
    let hits = MessageSearchRepository::new(&pool)
        .search(&query("from:vogel"))
        .await
        .unwrap()
        .hits;
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, "m-gabi");
}

async fn search_ids(pool: &SqlitePool, raw: &str) -> Vec<String> {
    let mut ids: Vec<String> = MessageSearchRepository::new(pool)
        .search(&query(raw))
        .await
        .unwrap()
        .hits
        .into_iter()
        .map(|hit| hit.id)
        .collect();
    ids.sort();
    ids
}

async fn seed_recipients(pool: &SqlitePool, field: &str) {
    for (id, list) in [
        ("m-ann", r#"["ann@example.com"]"#),
        ("m-ann-meier", r#"["Ann Meier <ann.meier@example.com>"]"#),
        ("m-joann", r#"["joann@example.com"]"#),
        ("m-hoffmann", r#"["hoffmann@example.com"]"#),
    ] {
        let mut message = SeedMessage {
            id,
            ..Default::default()
        };
        if field == "to" {
            message.to_addresses = list;
        } else {
            message.cc_addresses = Some(list);
        }
        seed_message(pool, message).await;
    }
}

async fn finish_cc_backfill(pool: &SqlitePool) {
    run_repair(pool, &CC_INDEX, 2, None, |_| {}).await.unwrap();
    assert!(!is_pending(pool, &CC_INDEX).await.unwrap());
}

#[tokio::test]
async fn to_operator_matches_whole_address_parts_and_names() {
    let pool = pool().await;
    seed_account(&pool).await;
    seed_recipients(&pool, "to").await;

    assert_eq!(search_ids(&pool, "to:ann").await, ["m-ann", "m-ann-meier"]);
    assert_eq!(search_ids(&pool, "to:ann@example.com").await, ["m-ann"]);
    assert_eq!(search_ids(&pool, "to:\"Ann Meier\"").await, ["m-ann-meier"]);
    // Next to free text, where messages_fts already drives the outer query.
    assert_eq!(
        search_ids(&pool, "body to:ann").await,
        ["m-ann", "m-ann-meier"]
    );

    // What the old substring filter returned for the same operator.
    let like_hits: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE to_addresses LIKE '%ann%'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(like_hits, 4);
}

#[tokio::test]
async fn cc_operator_matches_whole_address_parts_once_indexed() {
    let pool = pool().await;
    seed_account(&pool).await;
    seed_recipients(&pool, "cc").await;

    // While the backfill is pending the index is incomplete: substring match.
    assert!(is_pending(&pool, &CC_INDEX).await.unwrap());
    assert_eq!(search_ids(&pool, "cc:ann").await.len(), 4);

    finish_cc_backfill(&pool).await;

    assert_eq!(search_ids(&pool, "cc:ann").await, ["m-ann", "m-ann-meier"]);
    assert_eq!(search_ids(&pool, "cc:joann").await, ["m-joann"]);
    assert!(search_ids(&pool, "cc:@").await.is_empty());
}

#[tokio::test]
async fn cc_backfill_indexes_rows_stored_before_the_migration() {
    let pool = pool().await;
    seed_account(&pool).await;
    seed_recipients(&pool, "cc").await;
    // A database migrated with rows already in it: the triggers never saw them.
    sqlx::query("DELETE FROM message_cc_fts")
        .execute(&pool)
        .await
        .unwrap();

    finish_cc_backfill(&pool).await;

    let indexed: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM message_cc_fts WHERE message_cc_fts MATCH 'example'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(indexed, 4);
    assert_eq!(search_ids(&pool, "cc:ann").await, ["m-ann", "m-ann-meier"]);
}

#[tokio::test]
async fn cc_index_follows_updates_and_deletes() {
    let pool = pool().await;
    seed_account(&pool).await;
    seed_recipients(&pool, "cc").await;
    finish_cc_backfill(&pool).await;

    sqlx::query("UPDATE messages SET cc_addresses = '[\"ann@example.com\"]' WHERE id = 'm-joann'")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM messages WHERE id = 'm-ann'")
        .execute(&pool)
        .await
        .unwrap();

    assert_eq!(
        search_ids(&pool, "cc:ann").await,
        ["m-ann-meier", "m-joann"]
    );
    assert!(search_ids(&pool, "cc:joann").await.is_empty());
}

#[tokio::test]
async fn pages_all_search_branches_without_duplicate_threads() {
    let pool = pool().await;
    seed_account(&pool).await;
    for index in 0..67 {
        let id = format!("page-{index:03}");
        seed_message(
            &pool,
            SeedMessage {
                id: &id,
                subject: "Pagination",
                body_text: if index % 2 == 0 {
                    "Body"
                } else {
                    "Body with more words to vary relevance weights"
                },
                date: if index % 3 == 0 {
                    "2026-02-01T00:00:00Z"
                } else {
                    "2026-01-01T00:00:00Z"
                },
                is_read: false,
                ..Default::default()
            },
        )
        .await;
    }
    seed_message(
        &pool,
        SeedMessage {
            id: "duplicate-thread-message",
            thread_id: Some("th-page-000"),
            subject: "Pagination",
            is_read: false,
            ..Default::default()
        },
    )
    .await;
    for raw in [
        "Pagination",
        "subject:Pagination",
        "is:unread",
        "Pagination is:unread",
    ] {
        let repository = MessageSearchRepository::new(&pool);
        let mut input = query(raw);
        let first = repository.search(&input).await.unwrap();
        assert_eq!(first.hits.len(), 50, "{raw}");
        assert!(first.has_more, "{raw}");
        input.cursor = first.next_cursor;
        let second = repository.search(&input).await.unwrap();
        assert_eq!(second.hits.len(), 17, "{raw}");
        assert!(!second.has_more, "{raw}");
        assert!(second.next_cursor.is_none());
        let ids = first
            .hits
            .iter()
            .chain(&second.hits)
            .map(|hit| &hit.thread_id)
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(ids.len(), 67, "{raw}");
        input.cursor = None;
        input.account_id = Some("acc-1".into());
        let repeated = repository.search(&input).await.unwrap();
        assert_eq!(
            first.hits.iter().map(|hit| &hit.id).collect::<Vec<_>>(),
            repeated.hits.iter().map(|hit| &hit.id).collect::<Vec<_>>()
        );
        input.account_id = Some("missing-account".into());
        let missing = repository.search(&input).await.unwrap();
        assert!(missing.hits.is_empty());
        assert!(!missing.has_more);
        assert!(missing.next_cursor.is_none());
    }
}

#[tokio::test]
async fn flag_cursor_uses_thread_sort_date_when_a_newer_draft_is_displayed() {
    let pool = pool().await;
    seed_account(&pool).await;
    seed_message(
        &pool,
        SeedMessage {
            id: "older",
            date: "2026-01-01T00:00:00Z",
            ..Default::default()
        },
    )
    .await;
    seed_message(
        &pool,
        SeedMessage {
            id: "newer",
            date: "2026-02-01T00:00:00Z",
            ..Default::default()
        },
    )
    .await;
    seed_message(
        &pool,
        SeedMessage {
            id: "draft",
            thread_id: Some("th-newer"),
            date: "2026-03-01T00:00:00Z",
            ..Default::default()
        },
    )
    .await;
    sqlx::query("UPDATE messages SET is_draft = 1 WHERE id = 'draft'")
        .execute(&pool)
        .await
        .unwrap();
    let mut tx = pool.begin().await.unwrap();
    satchel_core::db::threads::refresh_thread_projections_tx(&mut tx, &["th-newer".into()])
        .await
        .unwrap();
    tx.commit().await.unwrap();
    sqlx::query("UPDATE threads SET is_starred = 1, has_attachments = 1")
        .execute(&pool)
        .await
        .unwrap();
    for raw in ["is:starred", "has:attachment"] {
        let repository = MessageSearchRepository::new(&pool);
        let mut input = query(raw);
        input.limit = Some(1);
        let first = repository.search(&input).await.unwrap();
        assert_eq!(first.hits[0].id, "draft");
        assert_eq!(
            first.next_cursor.as_ref().unwrap().date,
            "2026-02-01T00:00:00Z"
        );
        input.cursor = first.next_cursor;
        let second = repository.search(&input).await.unwrap();
        assert_eq!(second.hits[0].id, "older");
        assert!(!second.has_more);
    }
}
