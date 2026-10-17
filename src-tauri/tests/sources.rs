use satchel_lib::commands::sources::{
    create_source_inner, ingest_source_inner, publish_source_event, CreateSourceInput, IngestItem,
};
use satchel_lib::events::bus::{AppEvent, EventBus};
use sqlx::sqlite::SqlitePoolOptions;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Give this process a credential key, without touching the environment.
///
/// `create_source_inner` writes an account row, and credentials are refused
/// rather than stored in the clear (SEC-10). A test process has no keychain
/// entry, so the file fallback is seeded directly: setting
/// `SATCHEL_ALLOW_PLAINTEXT_KEY_FALLBACK` instead would be a `set_var` racing
/// every other test thread's `var` — which is exactly how this flaked.
fn init_credential_key() {
    use std::sync::OnceLock;
    static PROFILE: OnceLock<tempfile::TempDir> = OnceLock::new();
    let profile = PROFILE.get_or_init(|| {
        let dir = tempfile::tempdir().expect("temp profile");
        // Base64 of 32 bytes, the shape `load_existing_file_master_key` reads.
        std::fs::write(
            dir.path().join("pgp-master-key-v1"),
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, [7_u8; 32]),
        )
        .expect("seed master key");
        dir
    });
    satchel_core::utils::crypto::init_master_key(profile.path()).expect("credential key");
}

#[tokio::test]
async fn source_ingest_is_idempotent_and_uses_unified_threads() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::migrate!("../crates/satchel-core/src/db/migrations")
        .run(&pool)
        .await
        .unwrap();
    init_credential_key();
    let source = create_source_inner(
        &pool,
        CreateSourceInput {
            name: "CI".into(),
            kind: "webhook".into(),
            identity: "ci".into(),
            color: None,
            config: serde_json::json!({}),
        },
    )
    .await
    .unwrap();
    let item = IngestItem {
        external_id: Some("build-1".into()),
        thread_key: Some("main".into()),
        title: "Build failed".into(),
        author: None,
        body: Some("tests".into()),
        body_html: None,
        timestamp: None,
        level: Some("error".into()),
        url: None,
        data: None,
    };
    let first = ingest_source_inner(&pool, &source.id, item.clone())
        .await
        .unwrap();
    let second = ingest_source_inner(&pool, &source.id, item).await.unwrap();
    assert!(first.created);
    assert!(!second.created);
    assert_eq!(first.thread_id, second.thread_id);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE account_id=?")
        .bind(&source.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
    let priority: i64 = sqlx::query_scalar("SELECT priority FROM threads WHERE id=?")
        .bind(first.thread_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(priority, 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_source_ingest_returns_one_create_and_one_dedup() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("concurrent.sqlite");
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect(&format!("sqlite://{}?mode=rwc", database.display()))
        .await
        .unwrap();
    sqlx::migrate!("../crates/satchel-core/src/db/migrations")
        .run(&pool)
        .await
        .unwrap();
    init_credential_key();
    let source = create_source_inner(
        &pool,
        CreateSourceInput {
            name: "Concurrent CI".into(),
            kind: "webhook".into(),
            identity: "concurrent-ci".into(),
            color: None,
            config: serde_json::json!({}),
        },
    )
    .await
    .unwrap();
    let item = IngestItem {
        external_id: Some("same-delivery".into()),
        thread_key: Some("main".into()),
        title: "Build status".into(),
        author: None,
        body: None,
        body_html: None,
        timestamp: None,
        level: Some("info".into()),
        url: None,
        data: None,
    };
    let first_pool = pool.clone();
    let first_source = source.id.clone();
    let first_item = item.clone();
    let first =
        tokio::spawn(
            async move { ingest_source_inner(&first_pool, &first_source, first_item).await },
        );
    let second_pool = pool.clone();
    let second_source = source.id.clone();
    let second =
        tokio::spawn(async move { ingest_source_inner(&second_pool, &second_source, item).await });
    let first = first.await.unwrap().unwrap();
    let second = second.await.unwrap().unwrap();
    assert_ne!(first.created, second.created);
    assert_eq!(first.thread_id, second.thread_id);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE account_id=?")
        .bind(&source.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn source_payload_limit_is_enforced() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::migrate!("../crates/satchel-core/src/db/migrations")
        .run(&pool)
        .await
        .unwrap();
    init_credential_key();
    let source = create_source_inner(
        &pool,
        CreateSourceInput {
            name: "Script".into(),
            kind: "script".into(),
            identity: "job".into(),
            color: None,
            config: serde_json::json!({}),
        },
    )
    .await
    .unwrap();
    let item = IngestItem {
        external_id: None,
        thread_key: None,
        title: "Large".into(),
        author: None,
        body: Some("x".repeat(300_000)),
        body_html: None,
        timestamp: None,
        level: None,
        url: None,
        data: None,
    };
    assert!(ingest_source_inner(&pool, &source.id, item).await.is_err());
}

#[tokio::test]
async fn successful_webhook_ingest_publishes_the_existing_incoming_event() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::migrate!("../crates/satchel-core/src/db/migrations")
        .run(&pool)
        .await
        .unwrap();
    init_credential_key();
    let source = create_source_inner(
        &pool,
        CreateSourceInput {
            name: "Monitoring".into(),
            kind: "webhook".into(),
            identity: "monitoring".into(),
            color: None,
            config: serde_json::json!({"notify":true}),
        },
    )
    .await
    .unwrap();
    let item = IngestItem {
        external_id: Some("alert-1".into()),
        thread_key: None,
        title: "Backup failed".into(),
        author: Some("Backup".into()),
        body: Some("Disk unavailable".into()),
        body_html: None,
        timestamp: None,
        level: Some("error".into()),
        url: None,
        data: None,
    };
    let result = ingest_source_inner(&pool, &source.id, item.clone())
        .await
        .unwrap();
    let events = EventBus::new(4);
    let mut receiver = events.subscribe();
    publish_source_event(&pool, &events, &source.id, &item, &result).await;

    match receiver.recv().await.unwrap() {
        AppEvent::NewMail {
            thread_id, subject, ..
        } => {
            assert_eq!(thread_id, result.thread_id);
            assert_eq!(subject, "Backup failed");
        }
        event => panic!("unexpected event: {event:?}"),
    }
}

#[tokio::test]
async fn rss_initial_watermark_subsequent_item_and_conditional_get_work_end_to_end() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        for attempt in 0..3 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = vec![0u8; 4096];
            let read = socket.read(&mut request).await.unwrap();
            let request = String::from_utf8_lossy(&request[..read]);
            let (status, etag, body) = match attempt {
                0 => (
                    "200 OK",
                    "v1",
                    "<rss><channel><item><title>Old</title><guid>old</guid><pubDate>Wed, 15 Jul 2026 10:00:00 +0000</pubDate></item><item><title>Current</title><guid>current</guid><pubDate>Thu, 16 Jul 2026 10:00:00 +0000</pubDate></item></channel></rss>",
                ),
                1 => {
                    assert!(request.contains("if-none-match: v1") || request.contains("If-None-Match: v1"));
                    (
                        "200 OK",
                        "v2",
                        "<rss><channel><item><title>Next</title><guid>next</guid><pubDate>Fri, 17 Jul 2026 10:00:00 +0000</pubDate></item><item><title>Current</title><guid>current</guid><pubDate>Thu, 16 Jul 2026 10:00:00 +0000</pubDate></item></channel></rss>",
                    )
                }
                _ => {
                    assert!(request.contains("if-none-match: v2") || request.contains("If-None-Match: v2"));
                    ("304 Not Modified", "v2", "")
                }
            };
            let response = format!(
                "HTTP/1.1 {status}\r\nETag: {etag}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        }
    });

    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::migrate!("../crates/satchel-core/src/db/migrations")
        .run(&pool)
        .await
        .unwrap();
    init_credential_key();
    let source = create_source_inner(
        &pool,
        CreateSourceInput {
            name: "Feed".into(),
            kind: "rss".into(),
            identity: format!("http://{address}/feed"),
            color: None,
            config: serde_json::json!({"poll_minutes":30,"notify":false}),
        },
    )
    .await
    .unwrap();
    let initial_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE account_id=?")
        .bind(&source.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(initial_count, 1);
    assert_eq!(
        satchel_lib::commands::sources::poll_rss_inner(&pool, &source.id, false)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        satchel_lib::commands::sources::poll_rss_inner(&pool, &source.id, false)
            .await
            .unwrap(),
        0
    );
    let final_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE account_id=?")
        .bind(&source.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(final_count, 2);
    server.await.unwrap();
}
