//! Done and Snooze as Gmail labels (ADR 0009), against a fake Gmail API.
//!
//! One test function on purpose: the API base is a process-wide variable,
//! and parallel tests in this binary would point each other's providers at
//! the wrong fake.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use chrono::TimeZone;
use satchel_core::providers::gmail::GmailProvider;
use satchel_core::providers::traits::{
    MailProvider, MarkerTarget, MessageRef, WorkflowMarker, WorkflowMarkerReport,
    WorkflowMarkerWrite,
};
use satchel_core::workflow::remote_marker::Snooze;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

#[derive(Debug, Clone)]
struct Request {
    line: String,
    body: Value,
}

/// The label list request fails while set.
static LABELS_DOWN: AtomicBool = AtomicBool::new(false);
/// Another device has created `Satchel/Done` as `Label_77` while set.
static LABEL_77_EXISTS: AtomicBool = AtomicBool::new(false);

fn labels() -> Value {
    let mut listed = json!({"labels":[
        {"id":"INBOX","name":"INBOX","type":"system"},
        {"id":"UNREAD","name":"UNREAD","type":"system"},
        {"id":"Label_1","name":"Projects","type":"user"},
        {"id":"Label_7","name":"Satchel/Done","type":"user"},
        {"id":"Label_8","name":"Satchel/Wake/20200101T080000Z","type":"user"},
        {"id":"Label_9","name":"Satchel/Snoozed","type":"user"},
        {"id":"Label_10","name":"Satchel/Invoices","type":"user"}
    ]});
    if LABEL_77_EXISTS.load(Ordering::SeqCst) {
        listed["labels"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":"Label_77","name":"Satchel/Done","type":"user"}));
    }
    listed
}

fn message(id: &str, labels: &[&str]) -> Value {
    json!({
        "id": id, "threadId": id, "historyId": "5",
        "labelIds": labels,
        "internalDate": "1780000000000",
        "payload": {
            "mimeType": "text/plain",
            "headers": [
                {"name": "Subject", "value": "Angebot"},
                {"name": "From", "value": "kunde@example.test"}
            ],
            "body": {"size": 0}
        }
    })
}

fn route(request: &Request) -> (&'static str, Value) {
    let line = &request.line;
    if line.starts_with("GET") && line.contains("/labels") {
        if LABELS_DOWN.load(Ordering::SeqCst) {
            return ("503 Service Unavailable", json!({}));
        }
        return ("200 OK", labels());
    }
    if line.contains("/messages/m77") {
        return ("200 OK", message("m77", &["Label_77"]));
    }
    if line.contains("/messages/mnew") {
        return ("200 OK", message("mnew", &["Label_new"]));
    }
    if line.starts_with("POST") && line.contains("/labels") {
        let name = request.body["name"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        return (
            "200 OK",
            json!({"id": "Label_new", "name": name, "type": "user"}),
        );
    }
    // The messages still carrying the expired wake label.
    if line.contains("/messages?") && line.contains("labelIds=Label_8") {
        return ("200 OK", json!({"messages": [{"id": "old"}]}));
    }
    if line.starts_with("DELETE") && line.contains("/labels/") {
        return ("204 No Content", Value::Null);
    }
    if line.contains("/messages/batchModify") {
        return ("204 No Content", Value::Null);
    }
    if line.contains("/messages/m1") {
        return (
            "200 OK",
            json!({
                "id": "m1", "threadId": "t1", "historyId": "5",
                "labelIds": ["Label_1", "Label_7", "UNREAD"],
                "internalDate": "1780000000000",
                "payload": {
                    "mimeType": "text/plain",
                    "headers": [
                        {"name": "Subject", "value": "Angebot"},
                        {"name": "From", "value": "kunde@example.test"}
                    ],
                    "body": {"size": 0}
                }
            }),
        );
    }
    ("404 Not Found", json!({}))
}

async fn spawn_fake_gmail() -> (String, Arc<Mutex<Vec<Request>>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(Mutex::new(Vec::<Request>::new()));
    let recorded = requests.clone();
    tokio::spawn(async move {
        loop {
            let Ok((socket, _)) = listener.accept().await else {
                return;
            };
            let recorded = recorded.clone();
            tokio::spawn(async move {
                let (reader, mut writer) = socket.into_split();
                let mut reader = BufReader::new(reader);
                let mut line = String::new();
                if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
                    return;
                }
                let mut length = 0usize;
                loop {
                    let mut header = String::new();
                    if reader.read_line(&mut header).await.unwrap_or(0) == 0 {
                        return;
                    }
                    let header = header.trim_end();
                    if header.is_empty() {
                        break;
                    }
                    if let Some((name, value)) = header.split_once(':') {
                        if name.eq_ignore_ascii_case("content-length") {
                            length = value.trim().parse().unwrap_or(0);
                        }
                    }
                }
                let mut body = vec![0u8; length];
                if reader.read_exact(&mut body).await.is_err() {
                    return;
                }
                let request = Request {
                    line: line.trim_end().to_string(),
                    body: serde_json::from_slice(&body).unwrap_or(Value::Null),
                };
                let (status, payload) = route(&request);
                recorded.lock().unwrap().push(request);
                let payload = if payload.is_null() {
                    String::new()
                } else {
                    payload.to_string()
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
                    payload.len()
                );
                let _ = writer.write_all(response.as_bytes()).await;
                let _ = writer.flush().await;
            });
        }
    });
    (format!("http://{address}"), requests)
}

fn reference(id: &str) -> MessageRef {
    MessageRef {
        remote_id: id.into(),
        source_folder: Some("INBOX".into()),
        source_uidvalidity: None,
    }
}

fn target(ids: &[&str]) -> MarkerTarget {
    MarkerTarget {
        refs: ids.iter().map(|id| reference(id)).collect(),
        expected: None,
    }
}

fn batch_modifies(requests: &Arc<Mutex<Vec<Request>>>) -> Vec<Value> {
    requests
        .lock()
        .unwrap()
        .iter()
        .filter(|request| request.line.contains("/messages/batchModify"))
        .map(|request| request.body.clone())
        .collect()
}

#[tokio::test]
async fn done_and_snooze_travel_as_satchel_labels() {
    let (base, requests) = spawn_fake_gmail().await;
    std::env::set_var("SATCHEL_GMAIL_API_BASE", &base);
    let provider =
        GmailProvider::from_account_config(&satchel_core::accounts::AccountConfig::from_plaintext(
            r#"{"oauth":{"access_token":"test-token"}}"#,
        ));

    // Snooze: the existing Snoozed label is reused, the wake label is
    // created, Done and every other Satchel label come off, the person's own
    // labels are not mentioned at all.
    let wake = chrono::Utc.with_ymd_and_hms(2026, 11, 7, 8, 0, 0).unwrap();
    let outcome = provider
        .set_workflow_marker(
            // Two threads in one row; a message listed twice is sent once.
            &[target(&["m1", "m2"]), target(&["m1"])],
            &WorkflowMarker::Snoozed(Snooze {
                wake: Some(wake),
                generation: 1,
            }),
        )
        .await
        .expect("snooze labels");
    assert_eq!(
        outcome,
        WorkflowMarkerReport {
            targets: vec![WorkflowMarkerWrite::Applied; 2],
            refused_in_inbox: false,
            applied_in_inbox: false,
        }
    );
    let created = requests
        .lock()
        .unwrap()
        .iter()
        .filter(|request| request.line.starts_with("POST") && request.line.contains("/labels"))
        .map(|request| {
            (
                request.body["name"].clone(),
                request.body["labelListVisibility"].clone(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        created,
        vec![(json!("Satchel/Wake/20261107T080000Z"), json!("labelHide"))]
    );
    // `Satchel/Invoices` is the person's label and is never touched.
    assert_eq!(
        batch_modifies(&requests),
        vec![
            json!({
                "ids": ["m1", "m2"],
                "addLabelIds": ["Label_9", "Label_new"],
                "removeLabelIds": ["Label_7", "Label_8"],
            }),
            // The snooze that ran out with the expired wake label is cleared
            // on exactly the messages that still carry that label.
            json!({
                "ids": ["old"],
                "addLabelIds": [],
                "removeLabelIds": ["Label_9", "Label_8"],
            }),
        ]
    );
    let lists = |requests: &Arc<Mutex<Vec<Request>>>| {
        requests
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request.line.starts_with("GET") && request.line.contains("/labels "))
            .count()
    };
    let lists_after_snooze = lists(&requests);
    // The wake label this device created above is in the cache already: a
    // message carrying it needs no list and is not mistaken for a folder.
    let created = provider
        .fetch_message_by_id("mnew")
        .await
        .expect("created label");
    assert!(created.mailbox_ids.is_empty());
    assert_eq!(lists(&requests), lists_after_snooze);

    // Then the expired label itself goes.
    assert!(requests
        .lock()
        .unwrap()
        .iter()
        .any(|request| request.line.starts_with("DELETE")
            && request.line.contains("/labels/Label_8")));

    // Back to the Inbox: only removals.
    requests.lock().unwrap().clear();
    provider
        .set_workflow_marker(&[target(&["m1"])], &WorkflowMarker::None)
        .await
        .expect("clear labels");
    assert_eq!(
        batch_modifies(&requests)[0],
        json!({
            "ids": ["m1"],
            "addLabelIds": [],
            "removeLabelIds": ["Label_7", "Label_8", "Label_9"],
        })
    );

    // Reading: the Done label becomes the envelope's marker and leaves the
    // mailbox list, where sync would otherwise make a folder of it.
    let envelope = provider.fetch_message_by_id("m1").await.expect("message");
    assert_eq!(envelope.workflow_marker, Some(WorkflowMarker::Done));
    assert_eq!(envelope.mailbox_ids, vec!["Label_1", "UNREAD"]);

    // And the folder list never shows Satchel's labels.
    let folders = provider.sync_folders().await.expect("folders");
    let names = folders
        .iter()
        .map(|folder| folder.name.as_str())
        .collect::<Vec<_>>();
    assert!(names.contains(&"Projects"));
    assert!(names.contains(&"Satchel/Invoices"));
    assert!(!names
        .iter()
        .any(|name| ["Satchel/Done", "Satchel/Snoozed"].contains(name)
            || name.starts_with("Satchel/Wake/")));

    // The label list was read once for the message above and is reused
    // while the token lasts; a second fetch asks for no list.
    let before = lists(&requests);
    provider
        .fetch_message_by_id("m1")
        .await
        .expect("message again");
    assert_eq!(lists(&requests), before);

    // A label id the cache has never seen (made on another device) is not
    // guessed at: without the list, the page fails and is retried.
    LABEL_77_EXISTS.store(true, Ordering::SeqCst);
    LABELS_DOWN.store(true, Ordering::SeqCst);
    assert!(provider.fetch_message_by_id("m77").await.is_err());
    LABELS_DOWN.store(false, Ordering::SeqCst);
    let retried = provider
        .fetch_message_by_id("m77")
        .await
        .expect("retried page");
    assert_eq!(retried.workflow_marker, Some(WorkflowMarker::Done));
    assert!(retried.mailbox_ids.is_empty());
}
