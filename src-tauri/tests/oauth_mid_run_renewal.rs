//! A mailbox run builds its provider once and may run for ten minutes; a send
//! or a queued mutation keeps its provider across several requests. The
//! access token it started with can expire on the way. The provider renews
//! it through the stored account, persisting the rotation, instead of failing
//! every later request of the run.
//!
//! Its own test binary: the Graph and token endpoint overrides are
//! process-wide.

use std::sync::{Arc, Mutex};

use satchel_core::accounts::{decrypt_account_config, encrypt_account_config, AccountConfig};
use satchel_core::providers::provider_for_stored_account;
use satchel_core::providers::traits::SyncCheckpoint;
use sqlx::SqlitePool;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Credentials renew a token this close to its expiry.
const FRESHNESS_FLOOR_SECONDS: i64 =
    satchel_core::utils::oauth::ACCESS_TOKEN_RENEWAL_MARGIN.as_secs() as i64;

#[derive(Default)]
struct Seen {
    /// `Authorization` header of every Graph request, in order.
    bearer: Vec<String>,
    /// Form bodies posted to the token endpoint.
    refreshes: Vec<String>,
}

fn delta_page(folder: &str) -> String {
    format!(
        r#"{{"value":[{{"id":"{folder}-message","conversationId":"{folder}-conversation",
             "subject":"{folder}","receivedDateTime":"2026-01-02T10:00:00Z",
             "from":{{"emailAddress":{{"address":"sender@example.test"}}}}}}],
           "@odata.deltaLink":"http://graph.invalid/{folder}-final"}}"#
    )
}

/// Serves Graph delta pages and an OAuth token endpoint on one listener.
async fn spawn_fake_microsoft() -> (String, Arc<Mutex<Seen>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let seen = Arc::new(Mutex::new(Seen::default()));
    let recorded = seen.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let recorded = recorded.clone();
            tokio::spawn(async move {
                let mut request = Vec::new();
                let mut buffer = [0u8; 8192];
                // Read the head, then the declared body.
                let (head, body) = loop {
                    let Ok(read) = socket.read(&mut buffer).await else {
                        return;
                    };
                    if read == 0 {
                        return;
                    }
                    request.extend_from_slice(&buffer[..read]);
                    let text = String::from_utf8_lossy(&request).to_string();
                    let Some((head, body)) = text.split_once("\r\n\r\n") else {
                        continue;
                    };
                    let length = head
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().ok())?
                        })
                        .unwrap_or(0);
                    if body.len() >= length {
                        break (head.to_string(), body.to_string());
                    }
                };
                let (status, response) = if head.starts_with("POST /token ") {
                    recorded.lock().unwrap().refreshes.push(body);
                    ("200 OK", r#"{"access_token":"renewed-access","refresh_token":"renewed-refresh","token_type":"Bearer","expires_in":3600}"#.to_string())
                } else if let Some(folder) = head
                    .strip_prefix("GET /me/mailFolders/")
                    .and_then(|path| path.split_once('/'))
                    .map(|(folder, _)| folder.to_string())
                {
                    let bearer = head
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("authorization")
                                .then(|| value.trim().to_string())
                        })
                        .unwrap_or_default();
                    recorded.lock().unwrap().bearer.push(bearer);
                    ("200 OK", delta_page(&folder))
                } else {
                    // Other local tools probe open ports; they are not part
                    // of the run.
                    ("404 Not Found", String::new())
                };
                let reply = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
                    response.len()
                );
                let _ = socket.write_all(reply.as_bytes()).await;
                let _ = socket.flush().await;
            });
        }
    });
    (address, seen)
}

fn oauth_config(access: &str, refresh: &str, expires_at: &str) -> String {
    serde_json::json!({"oauth": {
        "provider": "outlook", "client_id": "fixture-client",
        "access_token": access, "refresh_token": refresh,
        "expires_at": expires_at
    }})
    .to_string()
}

fn resync() -> SyncCheckpoint {
    SyncCheckpoint {
        cursor: Some(
            r#"{"latest_received":"2025-12-31T00:00:00Z","format_version":1}"#.to_string(),
        ),
    }
}

#[tokio::test]
async fn a_token_that_expires_mid_run_is_renewed_and_the_run_succeeds() {
    satchel_core::utils::crypto::seed_master_key_for_tests();
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    satchel_core::db::init(&pool).await.unwrap();
    let (address, seen) = spawn_fake_microsoft().await;
    std::env::set_var("SATCHEL_GRAPH_API_BASE", format!("http://{address}"));
    std::env::set_var(
        "SATCHEL_OUTLOOK_TOKEN_URL",
        format!("http://{address}/token"),
    );

    // Two seconds above the freshness floor: fresh for the first folder only.
    let expires_at = chrono::Utc::now() + chrono::Duration::seconds(FRESHNESS_FLOOR_SECONDS + 2);
    let started = oauth_config(
        "started-access",
        "started-refresh",
        &expires_at.to_rfc3339(),
    );
    sqlx::query(
        "INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
         VALUES ('account', 'Fixture', 'me@example.test', 'outlook', ?, 'encrypted-in-config', '2026-01-01', '2026-01-01')",
    )
    .bind(encrypt_account_config("account", &started).unwrap())
    .execute(&pool)
    .await
    .unwrap();

    // mailbox_sync builds the provider once per run, like this.
    let provider = provider_for_stored_account(
        &pool,
        "account",
        "outlook",
        &AccountConfig::from_plaintext(started),
        "me@example.test",
        None,
    )
    .unwrap();
    provider
        .sync_messages("inbox", resync())
        .await
        .expect("the first folder runs on the starting token");

    let stale_at = expires_at - chrono::Duration::seconds(FRESHNESS_FLOOR_SECONDS)
        + chrono::Duration::milliseconds(300);
    let wait = (stale_at - chrono::Utc::now()).to_std().unwrap_or_default();
    tokio::time::sleep(wait).await;

    let archive = provider
        .sync_messages("archive", resync())
        .await
        .expect("the run outlives the token it started with");
    assert_eq!(archive.messages.len(), 1);
    provider
        .sync_messages("sentitems", resync())
        .await
        .expect("the renewed token serves the rest of the run");

    let seen = std::mem::take(&mut *seen.lock().unwrap());
    assert_eq!(
        seen.bearer,
        [
            "Bearer started-access",
            "Bearer renewed-access",
            "Bearer renewed-access"
        ]
    );
    assert_eq!(seen.refreshes.len(), 1, "one refresh for the whole run");
    assert!(seen.refreshes[0].contains("grant_type=refresh_token"));
    assert!(seen.refreshes[0].contains("refresh_token=started-refresh"));

    // The rotation is persisted, so the next worker does not reuse a
    // consumed refresh token.
    let raw: String = sqlx::query_scalar("SELECT config FROM accounts WHERE id = 'account'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let stored: serde_json::Value =
        serde_json::from_str(&decrypt_account_config("account", &raw).unwrap()).unwrap();
    assert_eq!(stored["oauth"]["access_token"], "renewed-access");
    assert_eq!(stored["oauth"]["refresh_token"], "renewed-refresh");
}
