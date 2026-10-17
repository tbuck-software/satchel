//! An expired Gmail `historyId` must re-list the label, not just catch up.
//!
//! The `after:` catch-up filters on `internalDate`, so mail that was labelled
//! while the cursor was dead never matches it, and the folder snapshot only
//! deletes. Without a forced backfill that mail stays invisible forever.

use std::sync::{Arc, Mutex};

use satchel_core::providers::gmail::GmailProvider;
use satchel_core::providers::traits::{MailProvider, SyncCheckpoint};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// 2019-06-12, well before any timestamp the catch-up would ask for.
const OLD_INTERNAL_DATE_MS: &str = "1560000000000";
const RECENT_CURSOR_TS: u64 = 1_785_000_000;

fn route(request_line: &str) -> (&'static str, String) {
    if request_line.contains("/profile") {
        return ("200 OK", r#"{"historyId":"999"}"#.to_string());
    }
    if request_line.contains("/history") {
        // The stored cursor fell out of Gmail's history window.
        if request_line.contains("startHistoryId=100") {
            return (
                "404 Not Found",
                r#"{"error":{"code":404,"status":"FAILED_PRECONDITION"}}"#.to_string(),
            );
        }
        return ("200 OK", r#"{"historyId":"999"}"#.to_string());
    }
    if request_line.contains("/messages/old-2019") {
        return (
            "200 OK",
            format!(
                r#"{{"id":"old-2019","threadId":"t-2019","historyId":"120","labelIds":["Label_kunden"],
                    "internalDate":"{OLD_INTERNAL_DATE_MS}",
                    "payload":{{"mimeType":"text/plain",
                      "headers":[{{"name":"Subject","value":"Angebot 2019"}},
                                 {{"name":"From","value":"kunde@example.test"}}],
                      "body":{{"size":0}}}}}}"#
            ),
        );
    }
    if request_line.contains("/messages?") {
        // The catch-up query. It cannot see 2019 mail by construction.
        if request_line.contains("q=after") {
            return ("200 OK", r#"{"resultSizeEstimate":0}"#.to_string());
        }
        return (
            "200 OK",
            r#"{"messages":[{"id":"old-2019"}],"resultSizeEstimate":1}"#.to_string(),
        );
    }
    ("404 Not Found", "{}".to_string())
}

async fn spawn_fake_gmail() -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let requests: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let recorded = requests.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let recorded = recorded.clone();
            tokio::spawn(async move {
                let mut buffer = vec![0u8; 8192];
                let Ok(read) = socket.read(&mut buffer).await else {
                    return;
                };
                let request = String::from_utf8_lossy(&buffer[..read]).to_string();
                let request_line = request.lines().next().unwrap_or_default().to_string();
                let (status, body) = route(&request_line);
                recorded.lock().unwrap().push(request_line);
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.flush().await;
            });
        }
    });
    (format!("http://{address}"), requests)
}

fn message_list_requests(requests: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
    requests
        .lock()
        .unwrap()
        .iter()
        .filter(|line| line.contains("/messages?"))
        .cloned()
        .collect()
}

#[tokio::test]
async fn an_expired_history_id_relists_the_label_and_then_stops() {
    let (base, requests) = spawn_fake_gmail().await;
    std::env::set_var("SATCHEL_GMAIL_API_BASE", &base);

    let provider =
        GmailProvider::from_account_config(&satchel_core::accounts::AccountConfig::from_plaintext(
            r#"{"oauth":{"access_token":"test-token"}}"#,
        ));

    // Run 1: the stored history cursor is gone. The catch-up runs and finds
    // nothing, because it can only ask for mail received after the cursor.
    let first = provider
        .sync_messages(
            "Label_kunden",
            SyncCheckpoint {
                cursor: Some(format!(
                    r#"{{"latest_ts":{RECENT_CURSOR_TS},"history_id":"100","format_version":3}}"#
                )),
            },
        )
        .await
        .expect("first run succeeds");
    assert!(first.messages.is_empty(), "catch-up cannot see 2019 mail");
    let after_first: serde_json::Value =
        serde_json::from_str(first.next.cursor.as_deref().expect("cursor persisted")).unwrap();
    assert_eq!(after_first["history_expired"], serde_json::json!(true));
    assert!(after_first["backfill_page_token"].is_null());

    let lists_after_first = message_list_requests(&requests);
    assert_eq!(lists_after_first.len(), 1);
    assert!(
        lists_after_first[0].contains("q=after"),
        "run 1 only catches up: {lists_after_first:?}"
    );

    // Run 2: the flag forces a backfill bootstrap, which lists the label
    // without `after:` and therefore sees the newly labelled 2019 message.
    let second = provider
        .sync_messages("Label_kunden", first.next)
        .await
        .expect("second run succeeds");
    assert_eq!(
        second
            .messages
            .iter()
            .map(|message| message.remote_id.as_str())
            .collect::<Vec<_>>(),
        vec!["old-2019"]
    );
    let after_second: serde_json::Value =
        serde_json::from_str(second.next.cursor.as_deref().expect("cursor persisted")).unwrap();
    assert_eq!(after_second["history_expired"], serde_json::json!(false));

    let lists_after_second = message_list_requests(&requests);
    assert_eq!(lists_after_second.len(), 2);
    assert!(
        !lists_after_second[1].contains("q=after"),
        "the re-list must not be bounded by the receive timestamp: {lists_after_second:?}"
    );

    // Run 3: the gap is closed, so the label is not enumerated again.
    let third = provider
        .sync_messages("Label_kunden", second.next)
        .await
        .expect("third run succeeds");
    assert!(third.messages.is_empty());
    assert_eq!(
        message_list_requests(&requests).len(),
        2,
        "a closed gap must not re-list the label on every run"
    );

    std::env::remove_var("SATCHEL_GMAIL_API_BASE");
}
