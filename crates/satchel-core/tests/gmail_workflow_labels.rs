//! Done and Snooze as Gmail labels (ADR 0009), against a fake Gmail API.
//!
//! One test function on purpose: the API base is a process-wide variable,
//! and parallel tests in this binary would point each other's providers at
//! the wrong fake.

use std::sync::{Arc, Mutex};

use chrono::TimeZone;
use satchel_core::providers::gmail::GmailProvider;
use satchel_core::providers::traits::{
    MailProvider, MessageRef, WorkflowMarker, WorkflowMarkerWrite,
};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

#[derive(Debug, Clone)]
struct Request {
    line: String,
    body: Value,
}

fn labels() -> Value {
    json!({"labels":[
        {"id":"INBOX","name":"INBOX","type":"system"},
        {"id":"UNREAD","name":"UNREAD","type":"system"},
        {"id":"Label_1","name":"Projects","type":"user"},
        {"id":"Label_7","name":"Satchel/Done","type":"user"},
        {"id":"Label_8","name":"Satchel/Wake/20200101T080000Z","type":"user"},
        {"id":"Label_9","name":"Satchel/Snoozed","type":"user"}
    ]})
}

fn route(request: &Request) -> (&'static str, Value) {
    let line = &request.line;
    if line.starts_with("GET") && line.contains("/labels") {
        return ("200 OK", labels());
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
            &[reference("m1"), reference("m2"), reference("m1")],
            &WorkflowMarker::Snoozed(wake),
        )
        .await
        .expect("snooze labels");
    assert_eq!(outcome, WorkflowMarkerWrite::Applied);
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
    assert_eq!(
        batch_modifies(&requests),
        vec![json!({
            "ids": ["m1", "m2"],
            "addLabelIds": ["Label_9", "Label_new"],
            "removeLabelIds": ["Label_7", "Label_8"],
        })]
    );
    // The wake label of a snooze long past is cleaned up afterwards.
    assert!(requests
        .lock()
        .unwrap()
        .iter()
        .any(|request| request.line.starts_with("DELETE")
            && request.line.contains("/labels/Label_8")));

    // Back to the Inbox: only removals.
    requests.lock().unwrap().clear();
    provider
        .set_workflow_marker(&[reference("m1")], &WorkflowMarker::None)
        .await
        .expect("clear labels");
    assert_eq!(
        batch_modifies(&requests),
        vec![json!({
            "ids": ["m1"],
            "addLabelIds": [],
            "removeLabelIds": ["Label_7", "Label_8", "Label_9"],
        })]
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
    assert!(!names.iter().any(|name| name.starts_with("Satchel/")));
}
