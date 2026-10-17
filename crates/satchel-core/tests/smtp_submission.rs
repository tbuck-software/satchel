//! Submit mail over a real socket, from a real account row, to a fake SMTP peer.
//!
//! The browser E2E suite stops at `invoke`; a WebDriver run would need a built
//! app and a Linux runner. What actually breaks on this path is what the app
//! writes onto the wire — 21092a3 is the reference case: a display name with a
//! dot made `send_via_smtp` fail before SMTP was ever contacted, and every
//! gate in the tree stayed green. So this test replaces the network peer and
//! nothing else: the account config comes out of a migrated SQLite database,
//! the provider is chosen by the production dispatcher, and lettre writes to a
//! socket the test reads back.
use satchel_core::providers::{provider_for_account, traits::OutgoingMessage};
use satchel_core::sync::errors::SyncError;
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
use std::sync::{Arc, Mutex};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
};

/// Everything one submission handed to the peer.
#[derive(Debug, Default, Clone)]
struct Delivery {
    mail_from: String,
    rcpt_to: Vec<String>,
    data: String,
}

/// What the peer answers where a real server would decide.
#[derive(Debug, Clone, Copy)]
struct SinkPolicy {
    /// Answer to `RCPT TO`, i.e. a verdict reached before any data moved.
    rcpt: &'static str,
    /// Answer to the end-of-data marker, i.e. after the message was handed over.
    data: &'static str,
    /// Hang up instead of answering the end-of-data marker. The server has the
    /// whole message and may well deliver it; the client learns nothing.
    hang_up_after_data: bool,
    /// Hang up instead of answering `EHLO` — a handshake that dies before any
    /// message data is on the wire.
    hang_up_at_greeting: bool,
}

impl Default for SinkPolicy {
    fn default() -> Self {
        Self {
            rcpt: "250 2.1.5 recipient ok\r\n",
            data: "250 2.0.0 queued\r\n",
            hang_up_after_data: false,
            hang_up_at_greeting: false,
        }
    }
}

/// Everything the peer was told, in order. Lets a test assert on commands that
/// never reach a `Delivery` because the submission was refused first.
type Transcript = Arc<Mutex<Vec<String>>>;

/// A single-session SMTP sink.
///
/// AUTH is advertised on purpose: lettre sends credentials whenever the
/// transport was built with them, and `send_via_smtp` always builds it with
/// them. A server that stays silent about AUTH fails the submission before
/// MAIL FROM, which would test the fake rather than the app.
async fn spawn_smtp_sink() -> (u16, Arc<Mutex<Vec<Delivery>>>, Transcript) {
    spawn_smtp_sink_with(SinkPolicy::default()).await
}

async fn spawn_smtp_sink_with(policy: SinkPolicy) -> (u16, Arc<Mutex<Vec<Delivery>>>, Transcript) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let deliveries: Arc<Mutex<Vec<Delivery>>> = Arc::default();
    let transcript: Transcript = Arc::default();
    let sink = deliveries.clone();
    let log = transcript.clone();

    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let sink = sink.clone();
            let log = log.clone();
            tokio::spawn(async move {
                let (read_half, mut write) = stream.into_split();
                let mut reader = BufReader::new(read_half);
                let mut line = String::new();
                let mut delivery = Delivery::default();

                let _ = write.write_all(b"220 satchel-test ESMTP\r\n").await;
                loop {
                    line.clear();
                    if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
                        return;
                    }
                    let command = line.trim_end().to_string();
                    let upper = command.to_uppercase();
                    log.lock().unwrap().push(command.clone());

                    if upper.starts_with("EHLO") || upper.starts_with("HELO") {
                        if policy.hang_up_at_greeting {
                            return;
                        }
                        let _ = write
                            .write_all(
                                b"250-satchel-test\r\n250-AUTH PLAIN LOGIN\r\n\
                                  250-8BITMIME\r\n250 SMTPUTF8\r\n",
                            )
                            .await;
                    } else if upper.starts_with("AUTH") {
                        let _ = write.write_all(b"235 2.7.0 authenticated\r\n").await;
                    } else if upper.starts_with("MAIL FROM:") {
                        delivery.mail_from = command["MAIL FROM:".len()..].trim().to_string();
                        let _ = write.write_all(b"250 2.1.0 sender ok\r\n").await;
                    } else if upper.starts_with("RCPT TO:") {
                        delivery
                            .rcpt_to
                            .push(command["RCPT TO:".len()..].trim().to_string());
                        let _ = write.write_all(policy.rcpt.as_bytes()).await;
                    } else if upper.starts_with("DATA") {
                        let _ = write.write_all(b"354 end with <CRLF>.<CRLF>\r\n").await;
                        let mut body = String::new();
                        loop {
                            line.clear();
                            if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
                                return;
                            }
                            if line.trim_end() == "." {
                                break;
                            }
                            body.push_str(&line);
                        }
                        delivery.data = body;
                        sink.lock().unwrap().push(std::mem::take(&mut delivery));
                        if policy.hang_up_after_data {
                            return;
                        }
                        let _ = write.write_all(policy.data.as_bytes()).await;
                    } else if upper.starts_with("QUIT") {
                        let _ = write.write_all(b"221 2.0.0 bye\r\n").await;
                        return;
                    } else {
                        let _ = write.write_all(b"250 2.0.0 ok\r\n").await;
                    }
                }
            });
        }
    });

    (port, deliveries, transcript)
}

/// `max_connections(1)` is load-bearing: every connection to `sqlite::memory:`
/// opens its own database, so a second one would not see the migrations.
async fn migrated_pool() -> SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::migrate!("./src/db/migrations")
        .run(&pool)
        .await
        .unwrap();
    pool
}

async fn seed_account(pool: &SqlitePool, display_name: &str, smtp_port: u16) {
    let config = serde_json::json!({
        "host": "127.0.0.1",
        "port": 993,
        "tls": false,
        "username": "me@example.test",
        "password": "pw",
        "smtp": {
            "host": "127.0.0.1",
            "port": smtp_port,
            "tls": false,
            "username": "me@example.test",
            "password": "pw",
            "from_address": "me@example.test"
        }
    })
    .to_string();

    sqlx::query(
        "INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, \
         sync_state, color, is_active, created_at, updated_at) \
         VALUES ('acc-1', ?, 'me@example.test', 'imap', ?, '', '{}', '#000', 1, \
         '2026-01-01', '2026-01-01')",
    )
    .bind(display_name)
    .bind(config)
    .execute(pool)
    .await
    .unwrap();
}

/// Send the way `execute_send_action` does: read the row, hand the config to
/// the dispatcher, let the provider talk.
async fn submit(pool: &SqlitePool, message: OutgoingMessage) -> Result<(), SyncError> {
    let (provider, config, email): (String, String, String) =
        sqlx::query_as("SELECT provider, config, email FROM accounts WHERE id = 'acc-1'")
            .fetch_one(pool)
            .await
            .unwrap();
    // The fixture row holds plaintext; the dispatcher gets what decryption yields.
    let config = satchel_core::accounts::AccountConfig::from_plaintext(config);
    provider_for_account(&provider, &config, &email, None)?
        .send(message)
        .await
}

fn message(from_name: &str) -> OutgoingMessage {
    OutgoingMessage {
        from_name: Some(from_name.to_string()),
        from_address: Some("me@example.test".to_string()),
        to: vec!["mara@example.test".to_string()],
        subject: "Status".to_string(),
        body: "Staging is back up.".to_string(),
        ..Default::default()
    }
}

/// The 21092a3 regression, one test per name shape lettre's phrase grammar
/// rejects when the caller glues `Name <addr>` together itself.
#[tokio::test]
async fn delivers_display_names_lettre_cannot_parse() {
    for name in [
        "Dr. Alex Morgan",
        "Morgan, Alex",
        "Alex Mörgan",
        "Alex (Privat)",
    ] {
        let (port, deliveries, _) = spawn_smtp_sink().await;
        let pool = migrated_pool().await;
        seed_account(&pool, name, port).await;

        submit(&pool, message(name)).await.unwrap_or_else(|err| {
            panic!("submission failed for display name {name:?}: {err}");
        });

        let delivered = deliveries.lock().unwrap().clone();
        assert_eq!(delivered.len(), 1, "expected one delivery for {name:?}");
        let from = header(&delivered[0].data, "From");
        assert!(
            from.contains("me@example.test"),
            "From header lost the address for {name:?}: {from}",
        );
        assert!(
            from.len() > "<me@example.test>".len(),
            "From header lost the display name for {name:?}: {from}",
        );
    }
}

#[tokio::test]
async fn puts_the_account_identity_on_the_envelope_and_in_the_headers() {
    let (port, deliveries, _) = spawn_smtp_sink().await;
    let pool = migrated_pool().await;
    seed_account(&pool, "Alex Morgan", port).await;

    submit(&pool, message("Alex Morgan")).await.unwrap();

    let delivered = deliveries.lock().unwrap().clone();
    assert_eq!(delivered.len(), 1);
    assert_eq!(delivered[0].mail_from, "<me@example.test>");
    assert_eq!(delivered[0].rcpt_to, vec!["<mara@example.test>"]);
    // lettre quotes the phrase; the point is that the name survives at all.
    assert_eq!(
        header(&delivered[0].data, "From"),
        "\"Alex Morgan\" <me@example.test>"
    );
    assert_eq!(header(&delivered[0].data, "To"), "mara@example.test");
    assert_eq!(header(&delivered[0].data, "Subject"), "Status");
    assert!(delivered[0].data.contains("Staging is back up."));
}

/// A refused submission has to surface as an error, not as a silent success —
/// that is what decides whether the queue retries or drops the mail.
#[tokio::test]
async fn reports_a_rejected_recipient_instead_of_claiming_delivery() {
    let (port, _deliveries, _) = spawn_smtp_sink().await;
    let pool = migrated_pool().await;
    seed_account(&pool, "Alex Morgan", port).await;

    let mut refused = message("Alex Morgan");
    refused.to = vec!["not-an-address".to_string()];

    let error = submit(&pool, refused)
        .await
        .expect_err("expected a rejection")
        .to_string();
    assert!(error.contains("not-an-address"), "unhelpful error: {error}");
}

/// A 5xx is the server's own verdict. Guessing it from the message text got a
/// hard `550` five retries over nearly three hours.
#[tokio::test]
async fn a_permanent_smtp_reply_is_not_retried() {
    let (port, _deliveries, _) = spawn_smtp_sink_with(SinkPolicy {
        rcpt: "550 5.1.1 <mara@example.test>: Recipient address rejected: User unknown\r\n",
        ..SinkPolicy::default()
    })
    .await;
    let pool = migrated_pool().await;
    seed_account(&pool, "Alex Morgan", port).await;

    let error = submit(&pool, message("Alex Morgan"))
        .await
        .expect_err("a rejected recipient is an error");

    assert!(matches!(error, SyncError::Permanent(_)), "{error:?}");
    assert!(!error.is_rate_limited());
}

#[tokio::test]
async fn a_transient_smtp_reply_stays_retryable() {
    let (port, _deliveries, _) = spawn_smtp_sink_with(SinkPolicy {
        rcpt: "451 4.3.0 <mara@example.test>: temporary lookup failure\r\n",
        ..SinkPolicy::default()
    })
    .await;
    let pool = migrated_pool().await;
    seed_account(&pool, "Alex Morgan", port).await;

    let error = submit(&pool, message("Alex Morgan"))
        .await
        .expect_err("a deferred recipient is an error");

    assert!(matches!(error, SyncError::Transient(_)), "{error:?}");
    assert!(!error.is_permanent());
}

/// Postfix quotes its queue id back at you. A digit run inside it used to read
/// as a rate limit, and a rate-limited action consumes no retry — the mail then
/// circulated forever and never showed up as failed.
#[tokio::test]
async fn a_queue_id_containing_429_is_not_read_as_a_rate_limit() {
    let (port, _deliveries, _) = spawn_smtp_sink_with(SinkPolicy {
        data: "451 4.3.0 deferred, queued as 8B4291C3\r\n",
        ..SinkPolicy::default()
    })
    .await;
    let pool = migrated_pool().await;
    seed_account(&pool, "Alex Morgan", port).await;

    let error = submit(&pool, message("Alex Morgan"))
        .await
        .expect_err("a deferred message is an error");

    assert!(!error.is_rate_limited(), "{error:?}");
    assert!(matches!(error, SyncError::Transient(_)), "{error:?}");
}

/// lettre's fallback announces `[127.0.0.1]`, which a submission server with
/// `reject_non_fqdn_helo_hostname` refuses for every mail on the account.
#[tokio::test]
async fn announces_the_sender_domain_in_ehlo() {
    let (port, _deliveries, transcript) = spawn_smtp_sink().await;
    let pool = migrated_pool().await;
    seed_account(&pool, "Alex Morgan", port).await;

    submit(&pool, message("Alex Morgan")).await.unwrap();

    let commands = transcript.lock().unwrap().clone();
    let ehlo = commands
        .iter()
        .find(|command| command.to_uppercase().starts_with("EHLO"))
        .expect("an EHLO command");
    assert_eq!(ehlo, "EHLO example.test", "{commands:?}");
}

/*
 * MAIL-06. The queue used to retry any unclassified SMTP failure. A connection
 * that dies between the last byte of the body and the server's answer is
 * exactly such a failure — and the server may have accepted and delivered the
 * mail. The recipient then got it twice.
 */

/// The server has the whole message and never answers. Nothing may be
/// concluded, so nothing is retried.
#[tokio::test]
async fn a_connection_lost_after_the_message_went_over_is_not_retried() {
    let (port, deliveries, _) = spawn_smtp_sink_with(SinkPolicy {
        hang_up_after_data: true,
        ..SinkPolicy::default()
    })
    .await;
    let pool = migrated_pool().await;
    seed_account(&pool, "Alex Morgan", port).await;

    let error = submit(&pool, message("Alex Morgan"))
        .await
        .expect_err("a missing answer is not a success");

    // The peer really did receive the message — that is what makes a retry a
    // duplicate rather than a second try.
    assert_eq!(deliveries.lock().unwrap().len(), 1);
    assert!(
        matches!(error, SyncError::DeliveryUnknown(_)),
        "would have been retried: {error:?}"
    );
    assert!(error.is_delivery_unknown());
    assert!(!error.was_delivered(), "delivery is unknown, not confirmed");
}

/// A server that cannot be reached took nothing. Ordinary retry.
#[tokio::test]
async fn a_server_that_cannot_be_reached_stays_retryable() {
    // Bind to learn a free port, then drop the listener so the connect is
    // refused rather than merely slow.
    let port = {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        listener.local_addr().unwrap().port()
    };
    let pool = migrated_pool().await;
    seed_account(&pool, "Alex Morgan", port).await;

    let error = submit(&pool, message("Alex Morgan"))
        .await
        .expect_err("an unreachable server is an error");

    assert!(
        matches!(error, SyncError::Transient(_)),
        "an unreachable server must stay retryable: {error:?}"
    );
    assert!(!error.is_permanent());
}

/// A 5xx after the data is the server's own refusal: nothing was delivered,
/// and asking again will not change its mind.
#[tokio::test]
async fn a_refusal_after_the_data_is_permanent_not_unknown() {
    let (port, _deliveries, _) = spawn_smtp_sink_with(SinkPolicy {
        data: "550 5.7.1 message rejected by policy\r\n",
        ..SinkPolicy::default()
    })
    .await;
    let pool = migrated_pool().await;
    seed_account(&pool, "Alex Morgan", port).await;

    let error = submit(&pool, message("Alex Morgan"))
        .await
        .expect_err("a rejected message is an error");

    assert!(matches!(error, SyncError::Permanent(_)), "{error:?}");
    assert!(!error.is_delivery_unknown());
}

/// A handshake that dies takes the cautious classification too: the client
/// cannot tell it apart from a body already handed over. The cost is one
/// acknowledgement, against a duplicate in the recipient's mailbox.
#[tokio::test]
async fn a_handshake_that_dies_is_not_retried_either() {
    let (port, deliveries, _) = spawn_smtp_sink_with(SinkPolicy {
        hang_up_at_greeting: true,
        ..SinkPolicy::default()
    })
    .await;
    let pool = migrated_pool().await;
    seed_account(&pool, "Alex Morgan", port).await;

    let error = submit(&pool, message("Alex Morgan"))
        .await
        .expect_err("a dead handshake is an error");

    assert!(deliveries.lock().unwrap().is_empty());
    assert!(matches!(error, SyncError::DeliveryUnknown(_)), "{error:?}");
}

/// Unfolded header lookup; the sink hands over raw DATA.
fn header(raw: &str, name: &str) -> String {
    let mut value = String::new();
    let mut collecting = false;
    for line in raw.lines() {
        if line.is_empty() {
            break;
        }
        if collecting {
            if line.starts_with(' ') || line.starts_with('\t') {
                value.push(' ');
                value.push_str(line.trim());
                continue;
            }
            break;
        }
        if let Some(rest) = line.strip_prefix(&format!("{name}: ")) {
            collecting = true;
            value.push_str(rest.trim_end());
        }
    }
    value
}
