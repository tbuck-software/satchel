//! Protocol transcripts: the server's bytes are scripted, the client's are
//! compared verbatim.

use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};

enum Step {
    /// The server sends these bytes.
    S(&'static str),
    /// The client must send exactly these bytes.
    C(&'static str),
}
use Step::{C, S};

async fn run_server(mut stream: DuplexStream, steps: Vec<Step>) {
    for step in steps {
        match step {
            S(bytes) => stream.write_all(bytes.as_bytes()).await.unwrap(),
            C(expected) => {
                let mut got = vec![0; expected.len()];
                stream.read_exact(&mut got).await.unwrap_or_else(|err| {
                    panic!("client sent too little, wanted {expected:?}: {err}")
                });
                assert_eq!(String::from_utf8_lossy(&got), expected);
            }
        }
    }
}

/// A client whose server follows `steps`; the returned handle finishes once
/// every step ran (and panics on a mismatch).
async fn transcript(
    steps: Vec<Step>,
    secure: bool,
) -> (
    Result<ManageSieveClient, SieveError>,
    tokio::task::JoinHandle<()>,
) {
    let (client_end, server_end) = tokio::io::duplex(1 << 16);
    let server = tokio::spawn(run_server(server_end, steps));
    let client = ManageSieveClient::from_stream(Box::new(client_end), secure).await;
    (client, server)
}

const GREETING: &str = "\"IMPLEMENTATION\" \"Dovecot Pigeonhole\"\r\n\
\"SIEVE\" \"fileinto reject envelope encoded-character vacation subaddress comparator-i;ascii-numeric relational regex imap4flags copy include variables body enotify environment mailbox date index ihave duplicate mime foreverypart extracttext\"\r\n\
\"NOTIFY\" \"mailto\"\r\n\
\"SASL\" \"PLAIN LOGIN\"\r\n\
\"STARTTLS\"\r\n\
\"VERSION\" \"1.0\"\r\n\
OK \"Dovecot ready.\"\r\n";

#[tokio::test]
async fn reads_the_greeting_capabilities() {
    let (client, server) = transcript(vec![S(GREETING)], false).await;
    let client = client.unwrap();
    server.await.unwrap();
    let caps = client.capabilities();
    assert_eq!(caps.implementation.as_deref(), Some("Dovecot Pigeonhole"));
    assert!(caps.has_extension("vacation"));
    assert!(caps.has_extension("IMAP4FLAGS"));
    assert!(caps.has_extension("comparator-i;ascii-numeric"));
    assert!(!caps.has_extension("vnd.dovecot.pipe"));
    assert_eq!(caps.sasl, vec!["PLAIN", "LOGIN"]);
    assert!(caps.starttls);
    assert_eq!(caps.version.as_deref(), Some("1.0"));
}

#[tokio::test]
async fn refuses_to_send_a_password_in_the_clear() {
    let (client, server) = transcript(vec![S(GREETING)], false).await;
    let mut client = client.unwrap();
    server.await.unwrap();
    assert!(matches!(
        client.authenticate("user", "secret").await,
        Err(SieveError::Insecure)
    ));
}

#[tokio::test]
async fn starttls_replaces_the_plaintext_capabilities() {
    let (client, server) = transcript(
        vec![
            S(GREETING),
            C("STARTTLS\r\n"),
            S("OK \"Begin TLS negotiation now.\"\r\n"),
            // Stands in for the handshake: the server waits for it.
            C("HANDSHAKE\r\n"),
            // After the handshake: a fresh list, now without STARTTLS.
            S("\"IMPLEMENTATION\" \"Dovecot Pigeonhole\"\r\n\"SIEVE\" \"fileinto vacation\"\r\n\"SASL\" \"PLAIN\"\r\n\"VERSION\" \"1.0\"\r\nOK \"TLS negotiation successful.\"\r\n"),
        ],
        false,
    )
    .await;
    let client = client.unwrap();
    // The test pipe stands in for the TLS stream.
    let client = client
        .starttls(|mut raw| async move {
            raw.write_all(b"HANDSHAKE\r\n").await.map_err(io_error)?;
            Ok(raw)
        })
        .await
        .unwrap();
    server.await.unwrap();
    assert!(client.is_secure());
    assert!(!client.capabilities().starttls);
    assert_eq!(client.capabilities().sieve, vec!["fileinto", "vacation"]);
}

#[tokio::test]
async fn starttls_rejects_data_injected_after_the_ok() {
    let (client, server) = transcript(
        vec![
            S(GREETING),
            C("STARTTLS\r\n"),
            S("OK \"Begin TLS\"\r\n\"SIEVE\" \"evil\"\r\nOK\r\n"),
        ],
        false,
    )
    .await;
    let client = client.unwrap();
    let result = client.starttls(|raw| async move { Ok(raw) }).await;
    server.await.unwrap();
    assert!(matches!(result, Err(SieveError::Protocol(_))));
}

#[tokio::test]
async fn starttls_is_required_when_not_offered() {
    let (client, server) = transcript(
        vec![S("\"SIEVE\" \"fileinto\"\r\n\"SASL\" \"PLAIN\"\r\nOK\r\n")],
        false,
    )
    .await;
    server.await.unwrap();
    let result = client.unwrap().starttls(|raw| async move { Ok(raw) }).await;
    assert!(matches!(result, Err(SieveError::NoStartTls)));
}

#[tokio::test]
async fn authenticates_with_plain() {
    let (client, server) = transcript(
        vec![
            S(GREETING),
            // base64("\0user\0p\"ss") — the quote travels inside base64.
            C("AUTHENTICATE \"PLAIN\" \"AHVzZXIAcCJzcw==\"\r\n"),
            S("OK \"Logged in.\"\r\n"),
        ],
        true,
    )
    .await;
    let mut client = client.unwrap();
    client.authenticate("user", "p\"ss").await.unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn authenticates_with_login_challenges_as_strings_and_literals() {
    let (client, server) = transcript(
        vec![
            S("\"SASL\" \"LOGIN\"\r\nOK\r\n"),
            C("AUTHENTICATE \"LOGIN\"\r\n"),
            S("\"VXNlcm5hbWU6\"\r\n"),
            C("\"dXNlcg==\"\r\n"),
            S("{12}\r\nUGFzc3dvcmQ6\r\n"),
            C("\"c2VjcmV0\"\r\n"),
            S("OK\r\n"),
        ],
        true,
    )
    .await;
    let mut client = client.unwrap();
    client.authenticate("user", "secret").await.unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn reports_a_refused_login() {
    let (client, server) = transcript(
        vec![
            S("\"SASL\" \"PLAIN\"\r\nOK\r\n"),
            C("AUTHENTICATE \"PLAIN\" \"AHVzZXIAd3Jvbmc=\"\r\n"),
            S("NO \"Authentication failed.\"\r\n"),
        ],
        true,
    )
    .await;
    let mut client = client.unwrap();
    let error = client.authenticate("user", "wrong").await.unwrap_err();
    server.await.unwrap();
    assert!(matches!(error, SieveError::AuthFailed(_)));
    assert_eq!(error.message_key(), "backend.errors.serverRules.authFailed");
}

#[tokio::test]
async fn needs_a_mechanism_it_knows() {
    let (client, server) = transcript(vec![S("\"SASL\" \"GSSAPI\"\r\nOK\r\n")], true).await;
    let mut client = client.unwrap();
    server.await.unwrap();
    assert!(matches!(
        client.authenticate("user", "secret").await,
        Err(SieveError::NoMechanism)
    ));
}

#[tokio::test]
async fn lists_scripts_with_quoted_and_literal_names() {
    let (client, server) = transcript(
        vec![
            S("\"SASL\" \"PLAIN\"\r\nOK\r\n"),
            C("LISTSCRIPTS\r\n"),
            S("\"summer \\\"script\\\"\"\r\n\"main\" ACTIVE\r\n{12}\r\nmit Umlaut\u{e4}\r\nOK \"Listscripts completed.\"\r\n"),
        ],
        true,
    )
    .await;
    let mut client = client.unwrap();
    let scripts = client.list_scripts().await.unwrap();
    server.await.unwrap();
    assert_eq!(
        scripts,
        vec![
            ScriptInfo {
                name: "summer \"script\"".into(),
                active: false
            },
            ScriptInfo {
                name: "main".into(),
                active: true
            },
            ScriptInfo {
                name: "mit Umlaut\u{e4}".into(),
                active: false
            },
        ]
    );
}

#[tokio::test]
async fn gets_a_script_from_a_literal() {
    let script =
        "require \"fileinto\";\r\nif header :contains \"subject\" \"x\" { fileinto \"X\"; }\r\n";
    let response: &'static str = Box::leak(
        format!(
            "{{{}}}\r\n{script}\r\nOK \"Getscript completed.\"\r\n",
            script.len()
        )
        .into_boxed_str(),
    );
    let (client, server) = transcript(
        vec![
            S("\"SASL\" \"PLAIN\"\r\nOK\r\n"),
            C("GETSCRIPT \"main\"\r\n"),
            S(response),
        ],
        true,
    )
    .await;
    let mut client = client.unwrap();
    assert_eq!(client.get_script("main").await.unwrap(), script);
    server.await.unwrap();
}

#[tokio::test]
async fn puts_scripts_as_non_synchronizing_literals_and_reads_warnings() {
    let (client, server) = transcript(
        vec![
            S("\"SASL\" \"PLAIN\"\r\n\"VERSION\" \"1.0\"\r\nOK\r\n"),
            C("HAVESPACE \"satchel\" 9\r\n"),
            S("OK\r\n"),
            C("CHECKSCRIPT {9+}\r\nkeep;\r\n# \r\n"),
            S("OK\r\n"),
            C("PUTSCRIPT \"satchel\" {9+}\r\nkeep;\r\n# \r\n"),
            S("OK (WARNINGS) \"line 2: comment is empty\"\r\n"),
            C("SETACTIVE \"satchel\"\r\n"),
            S("OK\r\n"),
        ],
        true,
    )
    .await;
    let mut client = client.unwrap();
    client.have_space("satchel", 9).await.unwrap();
    assert_eq!(client.check_script("keep;\r\n# ").await.unwrap(), None);
    assert_eq!(
        client
            .put_script("satchel", "keep;\r\n# ")
            .await
            .unwrap()
            .as_deref(),
        Some("line 2: comment is empty")
    );
    client.set_active("satchel").await.unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn surfaces_a_checkscript_error_sent_as_a_literal() {
    let (client, server) = transcript(
        vec![
            S("\"SASL\" \"PLAIN\"\r\nOK\r\n"),
            C("CHECKSCRIPT {6+}\r\nbogus;\r\n"),
            S("NO {31}\r\nline 1: unknown command 'bogus'\r\n"),
        ],
        true,
    )
    .await;
    let mut client = client.unwrap();
    let error = client.check_script("bogus;").await.unwrap_err();
    server.await.unwrap();
    match error {
        SieveError::Refused(response) => {
            assert_eq!(
                response.text.as_deref(),
                Some("line 1: unknown command 'bogus'")
            );
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[tokio::test]
async fn maps_response_codes() {
    let (client, server) = transcript(
        vec![
            S("\"SASL\" \"PLAIN\"\r\nOK\r\n"),
            C("HAVESPACE \"satchel\" 99999\r\n"),
            S("NO (QUOTA/MAXSIZE) \"Quota exceeded\"\r\n"),
            C("DELETESCRIPT \"main\"\r\n"),
            S("NO (ACTIVE) \"You may not delete an active script\"\r\n"),
            C("GETSCRIPT \"gone\"\r\n"),
            S("no (nonexistent) \"There is no script by that name\"\r\n"),
        ],
        true,
    )
    .await;
    let mut client = client.unwrap();
    let quota = client.have_space("satchel", 99_999).await.unwrap_err();
    assert_eq!(quota.code(), Some("QUOTA/MAXSIZE"));
    assert_eq!(quota.message_key(), "backend.errors.serverRules.quota");
    let active = client.delete_script("main").await.unwrap_err();
    assert_eq!(active.code(), Some("ACTIVE"));
    let missing = client.get_script("gone").await.unwrap_err();
    assert_eq!(missing.code(), Some("NONEXISTENT"));
    server.await.unwrap();
}

#[tokio::test]
async fn a_bye_ends_the_session() {
    let (client, server) = transcript(
        vec![
            S("\"SASL\" \"PLAIN\"\r\nOK\r\n"),
            C("LISTSCRIPTS\r\n"),
            S("BYE (TRYLATER) \"Server shutting down\"\r\n"),
        ],
        true,
    )
    .await;
    let mut client = client.unwrap();
    let error = client.list_scripts().await.unwrap_err();
    server.await.unwrap();
    assert!(matches!(error, SieveError::Bye(_)));
    assert_eq!(error.code(), Some("TRYLATER"));
}

#[tokio::test]
async fn a_closed_connection_is_an_io_error_not_a_hang() {
    let (client_end, server_end) = tokio::io::duplex(1024);
    drop(server_end);
    let result = ManageSieveClient::from_stream(Box::new(client_end), true).await;
    assert!(matches!(result, Err(SieveError::Io(_))));
}

#[tokio::test]
async fn rejects_oversized_literals() {
    let (client, server) = transcript(vec![S("{999999999}\r\n")], true).await;
    server.await.unwrap();
    assert!(matches!(client, Err(SieveError::Protocol(_))));
}

#[test]
fn client_strings_switch_to_literals_when_quoting_is_not_allowed() {
    assert_eq!(encode_string("plain"), "\"plain\"");
    assert_eq!(encode_string("a\"b\\c"), "\"a\\\"b\\\\c\"");
    assert_eq!(encode_string("two\r\nlines"), "{10+}\r\ntwo\r\nlines");
    let long = "x".repeat(1025);
    assert!(encode_string(&long).starts_with("{1025+}\r\n"));
}
