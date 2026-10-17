//! Outlook bulk mutations go through Graph `$batch` and wait out throttling.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use satchel_core::accounts::AccountConfig;
use satchel_core::providers::outlook::OutlookProvider;
use satchel_core::providers::traits::{FlagUpdate, MailProvider, MessageRef};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const ACCOUNT_CONFIG: &str = r#"{"oauth":{"access_token":"test-token"}}"#;

/// `SATCHEL_GRAPH_API_BASE` is process-wide, so the tests take turns.
async fn env_lock() -> tokio::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await
}

#[derive(Debug, Clone)]
struct Recorded {
    path: String,
    body: Value,
}

/// A scripted answer: status line, extra headers, and per-operation statuses
/// by remote id (default 204) for a 200 batch response.
struct Answer {
    status: &'static str,
    headers: &'static str,
    per_id: Vec<(&'static str, u16, &'static str)>,
}

fn ok() -> Answer {
    Answer {
        status: "200 OK",
        headers: "",
        per_id: Vec::new(),
    }
}

async fn read_request(socket: &mut tokio::net::TcpStream) -> Option<(String, Vec<u8>)> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 8192];
    let header_end = loop {
        let read = socket.read(&mut chunk).await.ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(position) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
    };
    let head = String::from_utf8_lossy(&buffer[..header_end]).to_string();
    let length = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or(0);
    while buffer.len() < header_end + length {
        let read = socket.read(&mut chunk).await.ok()?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
    Some((head, buffer[header_end..].to_vec()))
}

async fn spawn_graph(answers: Vec<Answer>) -> (String, Arc<Mutex<Vec<Recorded>>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let recorded: Arc<Mutex<Vec<Recorded>>> = Arc::default();
    let answers = Arc::new(Mutex::new(VecDeque::from(answers)));
    let log = recorded.clone();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let log = log.clone();
            let answers = answers.clone();
            tokio::spawn(async move {
                while let Some((head, body)) = read_request(&mut socket).await {
                    let path = head
                        .lines()
                        .next()
                        .and_then(|line| line.split_whitespace().nth(1))
                        .unwrap_or_default()
                        .to_string();
                    let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                    log.lock().unwrap().push(Recorded {
                        path,
                        body: body.clone(),
                    });
                    let answer = answers.lock().unwrap().pop_front().unwrap_or_else(ok);
                    let payload = if answer.status.starts_with("200") {
                        let responses: Vec<Value> = body["requests"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .map(|request| {
                                let url = request["url"].as_str().unwrap_or_default();
                                let scripted = answer
                                    .per_id
                                    .iter()
                                    .find(|(id, _, _)| url.ends_with(&format!("/{id}")));
                                match scripted {
                                    Some((_, status, retry_after)) if !retry_after.is_empty() => {
                                        json!({ "id": request["id"], "status": status,
                                                "headers": { "Retry-After": retry_after } })
                                    }
                                    Some((_, status, _)) => json!({ "id": request["id"],
                                        "status": status, "body": { "error": { "code": "Denied" } } }),
                                    None => json!({ "id": request["id"], "status": 204 }),
                                }
                            })
                            .collect();
                        json!({ "responses": responses }).to_string()
                    } else {
                        "{\"error\":{\"code\":\"TooManyRequests\"}}".to_string()
                    };
                    let response = format!(
                        "HTTP/1.1 {}\r\nContent-Type: application/json\r\n{}Content-Length: {}\r\n\r\n{payload}",
                        answer.status,
                        answer.headers,
                        payload.len()
                    );
                    if socket.write_all(response.as_bytes()).await.is_err() {
                        return;
                    }
                }
            });
        }
    });
    (address, recorded)
}

fn refs(count: usize) -> Vec<MessageRef> {
    (0..count)
        .map(|index| MessageRef {
            remote_id: format!("m-{index}"),
            source_folder: None,
            source_uidvalidity: None,
        })
        .collect()
}

fn operations_in(request: &Recorded) -> usize {
    request.body["requests"].as_array().map_or(0, Vec::len)
}

#[tokio::test]
async fn archiving_two_hundred_messages_takes_ten_requests() {
    let _guard = env_lock().await;
    let (address, recorded) = spawn_graph(Vec::new()).await;
    std::env::set_var("SATCHEL_GRAPH_API_BASE", format!("http://{address}"));

    OutlookProvider::from_account_config(&AccountConfig::from_plaintext(ACCOUNT_CONFIG))
        .move_messages(&refs(200), "archive-folder")
        .await
        .expect("move");

    let requests = recorded.lock().unwrap().clone();
    assert_eq!(requests.len(), 10);
    assert!(requests.iter().all(|request| request.path == "/$batch"));
    assert_eq!(requests.iter().map(operations_in).sum::<usize>(), 200);
    assert_eq!(
        requests[0].body["requests"][0]["body"],
        json!({ "destinationId": "archive-folder" })
    );
    std::env::remove_var("SATCHEL_GRAPH_API_BASE");
}

#[tokio::test]
async fn a_throttled_batch_is_sent_again_after_retry_after() {
    let _guard = env_lock().await;
    let (address, recorded) = spawn_graph(vec![
        Answer {
            status: "429 Too Many Requests",
            headers: "Retry-After: 2\r\n",
            per_id: Vec::new(),
        },
        ok(),
    ])
    .await;
    std::env::set_var("SATCHEL_GRAPH_API_BASE", format!("http://{address}"));
    let started = Instant::now();

    OutlookProvider::from_account_config(&AccountConfig::from_plaintext(ACCOUNT_CONFIG))
        .set_flags(
            &refs(3),
            FlagUpdate {
                read: Some(true),
                starred: None,
            },
        )
        .await
        .expect("flags after the throttle lifts");

    assert_eq!(recorded.lock().unwrap().len(), 2);
    assert!(started.elapsed() >= Duration::from_secs(2));
    std::env::remove_var("SATCHEL_GRAPH_API_BASE");
}

#[tokio::test]
async fn persistent_throttling_gives_up_as_rate_limited() {
    let _guard = env_lock().await;
    let throttled = || Answer {
        status: "429 Too Many Requests",
        headers: "Retry-After: 0\r\n",
        per_id: Vec::new(),
    };
    let (address, recorded) = spawn_graph(vec![throttled(), throttled(), throttled()]).await;
    std::env::set_var("SATCHEL_GRAPH_API_BASE", format!("http://{address}"));

    let error =
        OutlookProvider::from_account_config(&AccountConfig::from_plaintext(ACCOUNT_CONFIG))
            .delete_messages(&refs(2))
            .await
            .unwrap_err();

    assert!(error.is_rate_limited(), "{error}");
    assert_eq!(recorded.lock().unwrap().len(), 3);
    std::env::remove_var("SATCHEL_GRAPH_API_BASE");
}

#[tokio::test]
async fn one_failed_operation_leaves_the_others_done_and_is_named() {
    let _guard = env_lock().await;
    let (address, recorded) = spawn_graph(vec![Answer {
        status: "200 OK",
        headers: "",
        per_id: vec![("m-1", 403, "")],
    }])
    .await;
    std::env::set_var("SATCHEL_GRAPH_API_BASE", format!("http://{address}"));

    let error =
        OutlookProvider::from_account_config(&AccountConfig::from_plaintext(ACCOUNT_CONFIG))
            .delete_messages(&refs(25))
            .await
            .unwrap_err();

    // Both batches went out; only m-1 is reported.
    let requests = recorded.lock().unwrap().clone();
    assert_eq!(requests.iter().map(operations_in).sum::<usize>(), 25);
    let message = error.to_string();
    assert!(message.contains("[m-1]"), "{message}");
    assert!(error.is_permanent());
    std::env::remove_var("SATCHEL_GRAPH_API_BASE");
}

#[tokio::test]
async fn only_the_throttled_operations_are_sent_again() {
    let _guard = env_lock().await;
    let (address, recorded) = spawn_graph(vec![
        Answer {
            status: "200 OK",
            headers: "",
            per_id: vec![("m-2", 429, "1")],
        },
        ok(),
    ])
    .await;
    std::env::set_var("SATCHEL_GRAPH_API_BASE", format!("http://{address}"));

    OutlookProvider::from_account_config(&AccountConfig::from_plaintext(ACCOUNT_CONFIG))
        .delete_messages(&refs(4))
        .await
        .expect("delete");

    let requests = recorded.lock().unwrap().clone();
    assert_eq!(requests.len(), 2);
    assert_eq!(operations_in(&requests[1]), 1);
    assert_eq!(requests[1].body["requests"][0]["url"], "/me/messages/m-2");
    std::env::remove_var("SATCHEL_GRAPH_API_BASE");
}
