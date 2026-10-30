//! Gmail's full representation can contain UTF-8 text with the original
//! Windows-1252 MIME header. Decode the original source at the MIME boundary.

use base64::Engine;
use satchel_core::providers::gmail::GmailProvider;
use serde_json::json;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const TEXT: &str = "Freundliche Grüße\nTelefon – Büro — Ende\nLiteral =DE and Ã¶";
const HTML: &str = "<p>Freundliche Grüße<br>Telefon – Büro — Ende</p><pre>Literal =DE and Ã¶</pre>";

fn base64url(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn full_message() -> serde_json::Value {
    json!({
        "id": "legacy-charset", "threadId": "thread", "labelIds": ["INBOX"],
        "internalDate": "1767225600000",
        "payload": {
            "mimeType": "multipart/alternative",
            "headers": [{"name": "From", "value": "sender@example.test"}],
            "parts": [
                {"mimeType": "text/plain", "headers": [
                    {"name": "Content-Type", "value": "text/plain; charset=Windows-1252"},
                    {"name": "Content-Transfer-Encoding", "value": "quoted-printable"}
                ], "body": {"data": base64url(TEXT.as_bytes())}},
                {"mimeType": "text/html", "headers": [
                    {"name": "Content-Type", "value": "text/html; charset=Windows-1252"},
                    {"name": "Content-Transfer-Encoding", "value": "quoted-printable"}
                ], "body": {"data": base64url(HTML.as_bytes())}}
            ]
        }
    })
}

fn raw_message() -> serde_json::Value {
    let raw = include_bytes!("../../../src-tauri/tests/corpus/13-gmail-windows1252-source.eml");
    json!({"id": "legacy-charset", "raw": base64url(raw)})
}

#[tokio::test]
async fn gmail_legacy_charset_uses_original_mime_bytes() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let recorded = requests.clone();
    let server = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = vec![0; 8192];
            let read = socket.read(&mut request).await.unwrap();
            let request = String::from_utf8_lossy(&request[..read]);
            let line = request.lines().next().unwrap();
            recorded.lock().unwrap().push(line.to_string());
            let (status, body) = if line.contains("format=raw") {
                if line.contains("/missing-source?") {
                    ("200 OK", json!({"id": "missing-source"}))
                } else if line.contains("/deleted-source?") {
                    ("404 Not Found", json!({}))
                } else {
                    ("200 OK", raw_message())
                }
            } else {
                let mut message = full_message();
                for part in message["payload"]["parts"].as_array_mut().unwrap() {
                    if line.contains("/utf8?") {
                        let mime = part["mimeType"].as_str().unwrap();
                        part["headers"][0]["value"] = json!(format!("{mime}; charset=UTF-8"));
                    } else if line.contains("/no-charset?") {
                        part["headers"][0]["value"] = part["mimeType"].clone();
                    } else if line.contains("/original-legacy?") {
                        let text = if part["mimeType"] == "text/plain" {
                            TEXT
                        } else {
                            HTML
                        };
                        let (bytes, _, errors) = encoding_rs::WINDOWS_1252.encode(text);
                        assert!(!errors);
                        part["body"]["data"] = json!(base64url(&bytes));
                    }
                }
                ("200 OK", message)
            };
            let body = body.to_string();
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        }
    });
    std::env::set_var("SATCHEL_GMAIL_API_BASE", format!("http://{address}"));
    let provider =
        GmailProvider::from_account_config(&satchel_core::accounts::AccountConfig::from_plaintext(
            r#"{"oauth":{"access_token":"test-token"}}"#,
        ));
    for id in ["legacy-charset", "original-legacy", "no-charset", "utf8"] {
        let message = provider.fetch_message_by_id(id).await.unwrap();
        assert_eq!(message.body_text.as_deref(), Some(TEXT), "{id}");
        assert_eq!(message.body_html.as_deref(), Some(HTML), "{id}");
    }
    for id in ["missing-source", "deleted-source"] {
        assert!(provider.fetch_message_by_id(id).await.is_err(), "{id}");
    }
    std::env::remove_var("SATCHEL_GMAIL_API_BASE");
    server.abort();

    let requests = requests.lock().unwrap();
    for id in ["legacy-charset", "original-legacy", "no-charset"] {
        assert!(requests
            .iter()
            .any(|line| line.contains(&format!("/{id}?format=raw"))));
    }
    assert!(!requests
        .iter()
        .any(|line| line.contains("/utf8?format=raw")));
}
