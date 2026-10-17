//! A Graph delta enumeration must keep the ground it gained.
//!
//! The pass used to follow `@odata.nextLink` without a bound and store its
//! cursor only after the final page, so any interruption threw away every page
//! it had read and the next run started at page 1 again.

use std::sync::{Arc, Mutex, OnceLock};

use satchel_core::providers::outlook::OutlookProvider;
use satchel_core::providers::traits::{MailProvider, SyncBatch, SyncCheckpoint};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const ACCOUNT_CONFIG: &str = r#"{"oauth":{"access_token":"test-token"}}"#;

/// `SATCHEL_GRAPH_API_BASE` is process-wide, so the tests that set it take turns.
async fn env_lock() -> tokio::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await
}

fn delta_page(address: &str, page: usize, total: usize) -> String {
    let value = format!(
        r#"{{"id":"message-{page}","conversationId":"conversation-{page}",
             "subject":"Page {page}","receivedDateTime":"2026-01-0{page}T10:00:00Z",
             "from":{{"emailAddress":{{"address":"sender@example.test"}}}}}}"#
    );
    if page < total {
        let next = page + 1;
        format!(r#"{{"value":[{value}],"@odata.nextLink":"http://{address}/delta-page-{next}"}}"#)
    } else {
        format!(r#"{{"value":[{value}],"@odata.deltaLink":"http://{address}/delta-final"}}"#)
    }
}

fn requested_page(request_line: &str) -> usize {
    request_line
        .split("/delta-page-")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|value| value.split('?').next())
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(1)
}

/// Serves `total` delta pages; `fail_on` answers that page number with a 500.
async fn spawn_fake_graph(
    total: usize,
    fail_on: Option<usize>,
) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let requests: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let recorded = requests.clone();
    let served_address = address.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let recorded = recorded.clone();
            let served_address = served_address.clone();
            tokio::spawn(async move {
                let mut buffer = vec![0u8; 8192];
                let Ok(read) = socket.read(&mut buffer).await else {
                    return;
                };
                let request = String::from_utf8_lossy(&buffer[..read]).to_string();
                let request_line = request.lines().next().unwrap_or_default().to_string();
                let page = requested_page(&request_line);
                recorded.lock().unwrap().push(request_line);
                let (status, body) = if fail_on == Some(page) {
                    (
                        "500 Internal Server Error",
                        "{\"error\":\"boom\"}".to_string(),
                    )
                } else {
                    ("200 OK", delta_page(&served_address, page, total))
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.flush().await;
            });
        }
    });
    (address, requests)
}

fn cursor_state(batch: &SyncBatch) -> serde_json::Value {
    serde_json::from_str(batch.next.cursor.as_deref().expect("cursor persisted")).unwrap()
}

fn message_ids(batch: &SyncBatch) -> Vec<String> {
    batch
        .messages
        .iter()
        .map(|message| message.remote_id.clone())
        .collect()
}

/// A synced folder whose delta token is gone: the enumeration starts over, and
/// the run has to hand the next one a usable starting point.
fn resync_checkpoint() -> SyncCheckpoint {
    SyncCheckpoint {
        cursor: Some(
            r#"{"latest_received":"2025-12-31T00:00:00Z","format_version":1}"#.to_string(),
        ),
    }
}

#[tokio::test]
async fn a_bounded_delta_pass_makes_progress_across_runs() {
    let _guard = env_lock().await;
    let (address, requests) = spawn_fake_graph(5, None).await;
    std::env::set_var("SATCHEL_GRAPH_API_BASE", format!("http://{address}"));
    std::env::set_var("SATCHEL_OUTLOOK_DELTA_PAGES", "2");

    let provider = OutlookProvider::from_account_config(
        &satchel_core::accounts::AccountConfig::from_plaintext(ACCOUNT_CONFIG),
    );

    let first = provider
        .sync_messages("inbox", resync_checkpoint())
        .await
        .expect("first run succeeds");
    assert_eq!(message_ids(&first), vec!["message-1", "message-2"]);
    assert_eq!(
        cursor_state(&first)["delta_next_link"],
        serde_json::json!(format!("http://{address}/delta-page-3"))
    );

    let second = provider
        .sync_messages("inbox", first.next)
        .await
        .expect("second run succeeds");
    assert_eq!(message_ids(&second), vec!["message-3", "message-4"]);
    assert_eq!(
        cursor_state(&second)["delta_next_link"],
        serde_json::json!(format!("http://{address}/delta-page-5"))
    );

    let third = provider
        .sync_messages("inbox", second.next)
        .await
        .expect("third run succeeds");
    assert_eq!(message_ids(&third), vec!["message-5"]);
    let final_state = cursor_state(&third);
    assert!(final_state["delta_next_link"].is_null());
    assert_eq!(
        final_state["delta_link"],
        serde_json::json!(format!("http://{address}/delta-final"))
    );

    // Five pages, five requests: nothing was enumerated twice.
    assert_eq!(requests.lock().unwrap().len(), 5);

    std::env::remove_var("SATCHEL_GRAPH_API_BASE");
    std::env::remove_var("SATCHEL_OUTLOOK_DELTA_PAGES");
}

#[tokio::test]
async fn a_failed_page_keeps_the_pages_before_it() {
    let _guard = env_lock().await;
    let (address, requests) = spawn_fake_graph(5, Some(2)).await;
    std::env::set_var("SATCHEL_GRAPH_API_BASE", format!("http://{address}"));

    let provider = OutlookProvider::from_account_config(
        &satchel_core::accounts::AccountConfig::from_plaintext(ACCOUNT_CONFIG),
    );
    let batch = provider
        .sync_messages("inbox", resync_checkpoint())
        .await
        .expect("a failure after the first page is not fatal");

    assert_eq!(message_ids(&batch), vec!["message-1"]);
    assert_eq!(
        cursor_state(&batch)["delta_next_link"],
        serde_json::json!(format!("http://{address}/delta-page-2"))
    );
    assert_eq!(requests.lock().unwrap().len(), 2);

    std::env::remove_var("SATCHEL_GRAPH_API_BASE");
}

#[tokio::test]
async fn a_failure_on_the_first_page_is_an_error() {
    let _guard = env_lock().await;
    let (address, _requests) = spawn_fake_graph(5, Some(1)).await;
    std::env::set_var("SATCHEL_GRAPH_API_BASE", format!("http://{address}"));

    let provider = OutlookProvider::from_account_config(
        &satchel_core::accounts::AccountConfig::from_plaintext(ACCOUNT_CONFIG),
    );
    let result = provider.sync_messages("inbox", resync_checkpoint()).await;
    assert!(result.is_err(), "nothing was read, so nothing is salvaged");

    std::env::remove_var("SATCHEL_GRAPH_API_BASE");
}

async fn spawn_ingest_graph(
    fail_backfill: bool,
    mime_failures: usize,
    throttle_delta_once: bool,
) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let requests = Arc::new(Mutex::new(Vec::<String>::new()));
    let recorded = requests.clone();
    let served = address.clone();
    tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = vec![0; 8192];
            let read = socket.read(&mut buffer).await.unwrap();
            let line = String::from_utf8_lossy(&buffer[..read])
                .lines()
                .next()
                .unwrap()
                .to_string();
            let mime_count = recorded
                .lock()
                .unwrap()
                .iter()
                .filter(|request| request.contains("/$value"))
                .count();
            let delta_count = recorded
                .lock()
                .unwrap()
                .iter()
                .filter(|request| request.contains("/delta"))
                .count();
            recorded.lock().unwrap().push(line.clone());
            let signed = serde_json::json!({
                "id": "signed", "conversationId": "conversation-signed", "subject": "Signed",
                "receivedDateTime": "2026-01-02T10:00:00Z",
                "internetMessageHeaders": [{"name":"Content-Type", "value":"multipart/signed; protocol=application/pgp-signature"}]
            });
            let (status, header, body) = if line.contains("/$value") {
                if mime_count < mime_failures {
                    ("500 Internal Server Error", "", "{}".to_string())
                } else {
                    (
                        "200 OK",
                        "",
                        "Content-Type: text/plain\r\n\r\nRecovered body".to_string(),
                    )
                }
            } else if line.contains("/me/messages/signed") {
                ("200 OK", "", signed.to_string())
            } else if line.contains("/delta") {
                if throttle_delta_once && delta_count == 0 {
                    (
                        "429 Too Many Requests",
                        "Retry-After: 0\r\n",
                        "{}".to_string(),
                    )
                } else {
                    ("200 OK", "", serde_json::json!({
                        "value": [{"id":"new-mail", "receivedDateTime":"2026-01-02T10:00:00Z"}, signed],
                        "@odata.deltaLink": format!("http://{served}/delta-next")
                    }).to_string())
                }
            } else if (line.contains("/backfill") || line.contains("/messages?")) && fail_backfill {
                (
                    "429 Too Many Requests",
                    "Retry-After: 3600\r\n",
                    "{}".to_string(),
                )
            } else if line.contains("/messages") || line.contains("/backfill") {
                ("200 OK", "", "{\"value\":[]}".to_string())
            } else {
                (
                    "200 OK",
                    "",
                    "{\"id\":\"inbox\",\"totalItemCount\":2}".to_string(),
                )
            };
            let response = format!("HTTP/1.1 {status}\r\n{header}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            let _ = socket.write_all(response.as_bytes()).await;
        }
    });
    (address, requests)
}

fn pending_backfill_checkpoint(address: &str) -> SyncCheckpoint {
    SyncCheckpoint {
        cursor: Some(
            serde_json::json!({
                "latest_received":"2026-01-01T00:00:00Z", "format_version":1,
                "delta_link":format!("http://{address}/delta"),
                "backfill_next_link":format!("http://{address}/backfill"),
                "backfill_remaining_messages":100, "backfill_total_messages":102
            })
            .to_string(),
        ),
    }
}

#[tokio::test]
async fn delta_precedes_a_throttled_backfill_and_keeps_new_mail() {
    let _guard = env_lock().await;
    let (address, requests) = spawn_ingest_graph(true, 0, false).await;
    std::env::set_var("SATCHEL_GRAPH_API_BASE", format!("http://{address}"));
    let provider = OutlookProvider::from_account_config(
        &satchel_core::accounts::AccountConfig::from_plaintext(ACCOUNT_CONFIG),
    );
    let batch = provider
        .sync_messages("inbox", pending_backfill_checkpoint(&address))
        .await
        .unwrap();
    assert!(message_ids(&batch).contains(&"new-mail".into()));
    assert_eq!(cursor_state(&batch)["backfill_remaining_messages"], 100);
    assert_eq!(
        cursor_state(&batch)["backfill_next_link"],
        format!("http://{address}/backfill")
    );
    let log = requests.lock().unwrap();
    assert!(log[0].contains("/delta"));
    assert!(log.last().unwrap().contains("/backfill"));
    std::env::remove_var("SATCHEL_GRAPH_API_BASE");
}

#[tokio::test]
async fn one_mime_failure_is_quarantined_without_losing_other_messages() {
    let _guard = env_lock().await;
    let (address, requests) = spawn_ingest_graph(false, usize::MAX, false).await;
    std::env::set_var("SATCHEL_GRAPH_API_BASE", format!("http://{address}"));
    let provider = OutlookProvider::from_account_config(
        &satchel_core::accounts::AccountConfig::from_plaintext(ACCOUNT_CONFIG),
    );
    let mut checkpoint = pending_backfill_checkpoint(&address);
    for attempt in 1..=3 {
        let batch = provider.sync_messages("inbox", checkpoint).await.unwrap();
        assert_eq!(message_ids(&batch), vec!["new-mail"]);
        assert_eq!(
            cursor_state(&batch)["mime_quarantine"]["signed"]["attempts"],
            attempt
        );
        checkpoint = batch.next;
    }
    let batch = provider.sync_messages("inbox", checkpoint).await.unwrap();
    assert_eq!(
        cursor_state(&batch)["mime_quarantine"]["signed"]["attempts"],
        3
    );
    assert_eq!(
        requests
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request.contains("/$value"))
            .count(),
        3
    );
    std::env::remove_var("SATCHEL_GRAPH_API_BASE");
}

#[tokio::test]
async fn sync_reads_retry_a_delta_429() {
    let _guard = env_lock().await;
    let (address, requests) = spawn_ingest_graph(false, 0, true).await;
    std::env::set_var("SATCHEL_GRAPH_API_BASE", format!("http://{address}"));
    let provider = OutlookProvider::from_account_config(
        &satchel_core::accounts::AccountConfig::from_plaintext(ACCOUNT_CONFIG),
    );
    let batch = provider
        .sync_messages("inbox", pending_backfill_checkpoint(&address))
        .await
        .unwrap();
    assert!(message_ids(&batch).contains(&"new-mail".into()));
    assert_eq!(
        requests
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request.contains("/delta"))
            .count(),
        2
    );
    std::env::remove_var("SATCHEL_GRAPH_API_BASE");
}

#[tokio::test]
async fn a_transient_mime_failure_is_recovered_on_the_next_run() {
    let _guard = env_lock().await;
    let (address, _) = spawn_ingest_graph(false, 1, false).await;
    std::env::set_var("SATCHEL_GRAPH_API_BASE", format!("http://{address}"));
    let provider = OutlookProvider::from_account_config(
        &satchel_core::accounts::AccountConfig::from_plaintext(ACCOUNT_CONFIG),
    );
    let first = provider
        .sync_messages("inbox", pending_backfill_checkpoint(&address))
        .await
        .unwrap();
    assert_eq!(
        cursor_state(&first)["mime_quarantine"]["signed"]["attempts"],
        1
    );
    let second = provider.sync_messages("inbox", first.next).await.unwrap();
    assert!(message_ids(&second).contains(&"signed".into()));
    assert!(cursor_state(&second)["mime_quarantine"]
        .as_object()
        .unwrap()
        .is_empty());
    std::env::remove_var("SATCHEL_GRAPH_API_BASE");
}

#[tokio::test]
async fn a_failed_initial_backfill_stays_pending_across_delta_runs() {
    let _guard = env_lock().await;
    let (address, requests) = spawn_ingest_graph(true, 0, false).await;
    std::env::set_var("SATCHEL_GRAPH_API_BASE", format!("http://{address}"));
    let provider = OutlookProvider::from_account_config(
        &satchel_core::accounts::AccountConfig::from_plaintext(ACCOUNT_CONFIG),
    );
    let first = provider
        .sync_messages("inbox", SyncCheckpoint::default())
        .await
        .unwrap();
    assert_eq!(cursor_state(&first)["backfill_bootstrap_pending"], true);
    let second = provider.sync_messages("inbox", first.next).await.unwrap();
    assert_eq!(cursor_state(&second)["backfill_bootstrap_pending"], true);
    assert_eq!(
        requests
            .lock()
            .unwrap()
            .iter()
            .filter(|line| line.contains("/messages?"))
            .count(),
        2
    );
    std::env::remove_var("SATCHEL_GRAPH_API_BASE");
}
