//! HTTP submission failures must never permit a second delivery.
use satchel_core::{
    accounts::AccountConfig,
    providers::{
        gmail::GmailProvider,
        outlook::OutlookProvider,
        traits::{MailProvider, OutgoingMessage},
    },
    sync::{
        queue::{QueuedAction, SyncQueue},
        worker::{settle_action, Outcome},
    },
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn fake_server(status: Option<u16>) -> (String, tokio::task::JoinHandle<String>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut data = Vec::new();
        let mut chunk = [0; 8192];
        let end = loop {
            let n = socket.read(&mut chunk).await.unwrap();
            assert_ne!(n, 0);
            data.extend_from_slice(&chunk[..n]);
            if let Some(i) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                break i + 4;
            }
        };
        let head = String::from_utf8_lossy(&data[..end]).to_string();
        let length: usize = head
            .lines()
            .find_map(|l| {
                let (name, value) = l.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse().unwrap())
            })
            .unwrap_or(0);
        while data.len() < end + length {
            let n = socket.read(&mut chunk).await.unwrap();
            assert_ne!(n, 0);
            data.extend_from_slice(&chunk[..n]);
        }
        if let Some(code) = status {
            socket.write_all(format!("HTTP/1.1 {code} Test\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}").as_bytes()).await.unwrap();
        }
        head.lines().next().unwrap().to_string()
    });
    (base, task)
}

#[tokio::test]
async fn queued_and_immediate_http_submission_is_terminal_after_5xx_or_disconnect() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::migrate!("./src/db/migrations")
        .run(&pool)
        .await
        .unwrap();
    let queue = SyncQueue::new();
    let config = AccountConfig::from_plaintext(r#"{"oauth":{"access_token":"test-token"}}"#);
    for provider_name in ["gmail", "outlook"] {
        let env = if provider_name == "gmail" {
            "SATCHEL_GMAIL_API_BASE"
        } else {
            "SATCHEL_GRAPH_API_BASE"
        };
        for immediate in [false, true] {
            for status in [Some(500), Some(504), Some(408), None, Some(400)] {
                let (base, server) = fake_server(status).await;
                std::env::set_var(env, base);
                let provider: Box<dyn MailProvider> = if provider_name == "gmail" {
                    Box::new(GmailProvider::from_account_config(&config))
                } else {
                    Box::new(OutlookProvider::from_account_config(&config))
                };
                let mut action =
                    QueuedAction::new("test-account".into(), "send_message", serde_json::json!({}));
                if immediate {
                    action.status = "sending".into();
                }
                queue.enqueue(&pool, action.clone()).await.unwrap();
                if !immediate {
                    queue.claim_ready(&pool, &action.id).await.unwrap().unwrap();
                }
                queue
                    .mark_submission_started(&pool, &action.id)
                    .await
                    .unwrap();
                let error = provider
                    .send(OutgoingMessage {
                        from_address: Some("me@example.test".into()),
                        to: vec!["you@example.test".into()],
                        subject: "Status".into(),
                        body: "Latest text".into(),
                        ..Default::default()
                    })
                    .await
                    .unwrap_err();
                assert!(server.await.unwrap().starts_with("POST "));
                if status == Some(400) {
                    assert!(error.is_permanent());
                    assert!(!error.is_delivery_unknown());
                } else {
                    assert!(
                        error.is_delivery_unknown(),
                        "{provider_name} {status:?}: {error:?}"
                    );
                    assert!(matches!(
                        settle_action(&pool, &queue, &action, Err(error))
                            .await
                            .unwrap(),
                        Outcome::DeliveryUnknown { .. }
                    ));
                    assert!(queue.requeue_mail(&pool, &action.id).await.is_err());
                    assert!(queue
                        .claim_ready(&pool, &action.id)
                        .await
                        .unwrap()
                        .is_none());
                }
                queue.mark_completed(&pool, &action.id).await.unwrap();
                std::env::remove_var(env);
            }
        }
    }
}
