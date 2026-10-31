//! POP3 provider (RFC 1939).
//!
//! POP3 is a download protocol, not a mailbox protocol: it has no folders, no
//! server-side flags, and no search. Everything Satchel shows beyond "this
//! message exists" — read state, stars, archive — is therefore local only, and
//! `SourceCapabilities` says so rather than leaving the buttons looking broken.
//!
//! Messages are tracked by UIDL, the one stable identifier POP3 offers. Message
//! *numbers* are not usable as identity: they are renumbered whenever anything
//! is deleted, so a number-based cursor would re-download or skip mail.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::time::{timeout, Duration};
use tokio_rustls::TlsConnector;

use super::imap::{
    build_snippet, mail_tls_client_config, parse_from_address, parse_from_name,
    parse_header_address_list, parse_header_value, parse_message_body, parse_message_date,
    split_references,
};
use super::smtp::{send_via_smtp, SmtpRuntimeConfig};
use super::traits::{
    FlagUpdate, MailProvider, MessageEnvelope, MessageRef, OutgoingMessage, RemoteFolder,
    SyncBatch, SyncCheckpoint,
};

/// POP3 exposes exactly one mailbox and has no name for it.
pub const POP3_FOLDER: &str = "INBOX";

#[derive(Debug, Clone)]
pub struct Pop3RuntimeConfig {
    pub host: String,
    pub port: u16,
    pub tls: bool,
    pub username: String,
    pub password: String,
    pub smtp: SmtpRuntimeConfig,
}

#[derive(Debug, Clone, Default)]
pub struct Pop3Provider {
    pub config: Option<Pop3RuntimeConfig>,
}

/// Whether this account wants its mail removed from the server once it is
/// stored locally.
///
/// Read from the stored account config rather than from a live provider: the
/// removal is queued by the sync run per UIDL it committed, not by the fetch.
pub fn deletes_after_fetch(
    account_config_json: &crate::accounts::AccountConfig,
) -> Result<bool, String> {
    let nested = serde_json::from_str::<NestedPop3Config>(account_config_json).unwrap_or_default();
    Ok(!nested
        .pop3
        .unwrap_or_default()
        .leave_on_server
        .unwrap_or(true))
}

/// Which UIDLs have already been ingested.
///
/// Stored rather than recomputed because POP3 offers no "since" query: the only
/// way to tell new mail from old is to compare the server's UIDL list against
/// what we have seen.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Pop3CursorState {
    #[serde(default)]
    seen_uidls: Vec<String>,
}

impl Pop3CursorState {
    fn from_checkpoint(since: &SyncCheckpoint) -> Self {
        since
            .cursor
            .as_deref()
            .and_then(|raw| serde_json::from_str(raw).ok())
            .unwrap_or_default()
    }

    fn to_cursor(&self) -> Option<String> {
        serde_json::to_string(self).ok()
    }
}

#[derive(Debug, Deserialize, Clone, Default)]
struct FlatPop3Config {
    host: Option<String>,
    port: Option<u16>,
    tls: Option<bool>,
    username: Option<String>,
    password: Option<String>,
    leave_on_server: Option<bool>,
}

#[derive(Debug, Deserialize, Clone, Default)]
struct FlatSmtpConfig {
    host: Option<String>,
    port: Option<u16>,
    tls: Option<bool>,
    username: Option<String>,
    password: Option<String>,
    from: Option<String>,
    from_address: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct NestedPop3Config {
    pop3: Option<FlatPop3Config>,
    smtp: Option<FlatSmtpConfig>,
}

/// Refusing plaintext needs a key of its own: the caller has to be able to say
/// *why* the account will not connect.
const PLAINTEXT_REJECTED_KEY: &str = "backend.errors.pop3PlaintextRejected";

/// Port 110 carries `USER`/`PASS` in the clear, and `STLS` is not implemented,
/// so nothing upgrades the session afterwards. Only a stored `tls: false` —
/// never the default and never the port alone — opts into that.
fn rejects_plaintext(cfg: &Pop3RuntimeConfig) -> bool {
    cfg.port == 110 && cfg.tls
}

impl Pop3Provider {
    pub fn from_account_config(config_json: &str, fallback_username: &str) -> Result<Self, String> {
        let nested = serde_json::from_str::<NestedPop3Config>(config_json).unwrap_or_default();
        let pop3 = nested.pop3.unwrap_or_default();

        let host = pop3
            .host
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "missing POP3 host in account config".to_string())?
            .to_string();

        // 995 is implicit TLS. 110 is plaintext: there is no STARTTLS here,
        // so it stays reachable only through a stored `tls: false`.
        let tls = pop3.tls.unwrap_or(true);
        let port = pop3.port.unwrap_or(if tls { 995 } else { 110 });

        let username = pop3
            .username
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(fallback_username)
            .to_string();

        let password = pop3
            .password
            .clone()
            .or_else(|| crate::utils::environment::debug_env("SATCHEL_POP3_PASSWORD"))
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                "missing POP3 password in account config and SATCHEL_POP3_PASSWORD env".to_string()
            })?;

        let smtp = nested.smtp.unwrap_or_default();
        let smtp_host = smtp
            .host
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(&host)
            .to_string();
        let smtp_username = smtp
            .username
            .clone()
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| username.clone());
        let smtp_password = smtp
            .password
            .clone()
            .or_else(|| crate::utils::environment::debug_env("SATCHEL_SMTP_PASSWORD"))
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| password.clone());
        let from_address = smtp
            .from_address
            .clone()
            .or_else(|| smtp.from.clone())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| fallback_username.to_string());

        Ok(Self {
            config: Some(Pop3RuntimeConfig {
                host,
                port,
                tls,
                username,
                password,
                smtp: SmtpRuntimeConfig {
                    host: smtp_host,
                    port: smtp.port.unwrap_or(587),
                    tls: smtp.tls.unwrap_or(true),
                    username: smtp_username,
                    password: smtp_password,
                    from_address,
                },
            }),
        })
    }

    fn op_timeout() -> Duration {
        Duration::from_secs(
            std::env::var("SATCHEL_POP3_TIMEOUT_SECS")
                .ok()
                .and_then(|raw| raw.parse().ok())
                .unwrap_or(30),
        )
    }

    /// Retrieve an existing message by its stable UIDL without deleting it.
    pub async fn fetch_raw_message(&self, uidl: &str) -> Result<Vec<u8>, String> {
        let mut session = self.connect().await?;
        let listing = parse_uidl_listing(&session.multiline_command("UIDL").await?);
        let number = listing
            .iter()
            .find(|(_, listed)| listed == uidl)
            .map(|(number, _)| *number)
            .ok_or_else(|| "backend.errors.exportOriginalUnavailable".to_string())?;
        let result = match session.retr(number).await? {
            Retrieved::Body(lines) => Ok(join_crlf(&lines)),
            Retrieved::Rejected(reason) => Err(reason),
        };
        let _ = session.quit().await;
        result
    }

    fn cfg(&self) -> Result<&Pop3RuntimeConfig, String> {
        self.config
            .as_ref()
            .ok_or_else(|| "POP3 provider config missing".to_string())
    }

    /// Connect and authenticate, then hang up. Used by account discovery to
    /// test a host/username combination without ingesting anything.
    pub async fn verify_credentials(&self) -> Result<(), String> {
        let mut session = self.connect().await?;
        // STAT proves the mailbox is actually readable, not just that the
        // login was accepted.
        let result = session.command("STAT").await.map(|_| ());
        let _ = session.quit().await;
        result
    }

    async fn connect(&self) -> Result<Pop3Session, String> {
        let cfg = self.cfg()?;
        if rejects_plaintext(cfg) {
            return Err(PLAINTEXT_REJECTED_KEY.to_string());
        }
        let tcp = timeout(
            Self::op_timeout(),
            TcpStream::connect((cfg.host.as_str(), cfg.port)),
        )
        .await
        .map_err(|_| "pop3 tcp connect timed out".to_string())?
        .map_err(|err| format!("pop3 tcp connect failed: {err}"))?;

        crate::utils::network::enable_mail_keepalive(&tcp)
            .map_err(|error| format!("pop3 keepalive setup failed: {error}"))?;

        let mut session = if cfg.tls {
            let connector = TlsConnector::from(Arc::new(mail_tls_client_config()?));
            let server_name = rustls_pki_types::ServerName::try_from(cfg.host.clone())
                .map_err(|err| format!("invalid POP3 TLS server name: {err}"))?;
            let stream = timeout(Self::op_timeout(), connector.connect(server_name, tcp))
                .await
                .map_err(|_| "pop3 tls handshake timed out".to_string())?
                .map_err(|err| format!("pop3 tls handshake failed: {err}"))?;
            Pop3Session::new(Box::new(stream))
        } else {
            Pop3Session::new(Box::new(tcp))
        };

        // Greeting first, then credentials.
        session.read_status().await?;
        session
            .command(&format!("USER {}", cfg.username))
            .await
            .map_err(|err| format!("pop3 USER failed: {err}"))?;
        session
            .command(&format!("PASS {}", cfg.password))
            .await
            .map_err(|err| format!("pop3 authentication failed: {err}"))?;
        Ok(session)
    }
}

/// One POP3 connection. Boxed so plain and TLS streams share a type.
struct Pop3Session {
    stream: BufReader<Box<dyn Pop3Stream>>,
}

trait Pop3Stream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Pop3Stream for T {}

impl Pop3Session {
    fn new(stream: Box<dyn Pop3Stream>) -> Self {
        Self {
            stream: BufReader::new(stream),
        }
    }

    /// Read one CRLF-terminated line as bytes.
    ///
    /// Mail bodies are not UTF-8: a single 8-bit byte would abort a decoding
    /// read *after* consuming the bytes, leaving the session out of step.
    async fn read_response_line(&mut self) -> Result<Vec<u8>, String> {
        let mut line = Vec::new();
        let read = timeout(
            Pop3Provider::op_timeout(),
            self.stream.read_until(b'\n', &mut line),
        )
        .await
        .map_err(|_| "pop3 read timed out".to_string())?
        .map_err(|err| format!("pop3 read failed: {err}"))?;
        if read == 0 {
            return Err("pop3 connection closed by server".to_string());
        }
        while matches!(line.last(), Some(b'\r' | b'\n')) {
            line.pop();
        }
        Ok(line)
    }

    /// Read a status line, mapping `-ERR` onto a Rust error.
    ///
    /// The only lossy conversion in this session: RFC 1939 status lines are ASCII.
    async fn read_status(&mut self) -> Result<String, String> {
        let line = String::from_utf8_lossy(&self.read_response_line().await?).into_owned();
        if line.split_whitespace().next() == Some("+OK") {
            return Ok(line[3..].trim().to_string());
        }
        Err(line
            .strip_prefix("-ERR")
            .map(|rest| rest.trim().to_string())
            .unwrap_or(line))
    }

    async fn write_line(&mut self, line: &str) -> Result<(), String> {
        let payload = format!("{line}\r\n");
        timeout(
            Pop3Provider::op_timeout(),
            self.stream.get_mut().write_all(payload.as_bytes()),
        )
        .await
        .map_err(|_| "pop3 write timed out".to_string())?
        .map_err(|err| format!("pop3 write failed: {err}"))?;
        Ok(())
    }

    async fn command(&mut self, line: &str) -> Result<String, String> {
        self.write_line(line).await?;
        self.read_status().await
    }

    /// Read a dot-terminated multi-line response, undoing byte-stuffing.
    async fn read_multiline(&mut self) -> Result<Vec<Vec<u8>>, String> {
        let mut lines = Vec::new();
        loop {
            let mut line = self.read_response_line().await?;
            if line == b"." {
                return Ok(lines);
            }
            // RFC 1939 §3: a leading dot in the payload is doubled on the wire.
            if line.first() == Some(&b'.') {
                line.remove(0);
            }
            lines.push(line);
        }
    }

    async fn multiline_command(&mut self, line: &str) -> Result<Vec<Vec<u8>>, String> {
        self.command(line).await?;
        self.read_multiline().await
    }

    /// `RETR` split into its two failure modes. A `-ERR` status arrives on a
    /// stream that is still in step, so the caller can skip that message and
    /// keep the run going. Anything else means the response was not consumed
    /// to its terminating dot and the session has to be dropped.
    async fn retr(&mut self, number: u32) -> Result<Retrieved, String> {
        self.write_line(&format!("RETR {number}")).await?;
        let status = String::from_utf8_lossy(&self.read_response_line().await?).into_owned();
        if let Some(reason) = status.strip_prefix("-ERR") {
            return Ok(Retrieved::Rejected(reason.trim().to_string()));
        }
        if !status.starts_with("+OK") {
            return Err(format!("pop3 unexpected RETR status: {status}"));
        }
        self.read_multiline().await.map(Retrieved::Body)
    }

    async fn quit(mut self) -> Result<(), String> {
        self.command("QUIT").await.map(|_| ())
    }
}

/// Outcome of a `RETR`. Separating the server's refusal from a broken stream
/// is what keeps one unreadable message from blocking a mailbox for good.
enum Retrieved {
    Body(Vec<Vec<u8>>),
    Rejected(String),
}

/// `UIDL` returns "<number> <unique-id>" per line, both ASCII per RFC 1939.
fn parse_uidl_listing<L: AsRef<[u8]>>(lines: &[L]) -> Vec<(u32, String)> {
    lines
        .iter()
        .filter_map(|line| {
            let line = String::from_utf8_lossy(line.as_ref());
            let mut parts = line.split_whitespace();
            let number = parts.next()?.parse::<u32>().ok()?;
            let uidl = parts.next()?.to_string();
            (!uidl.is_empty()).then_some((number, uidl))
        })
        .collect()
}

/// Join a multi-line response back into one raw message.
fn join_crlf(lines: &[Vec<u8>]) -> Vec<u8> {
    let mut raw = lines.join(&b"\r\n"[..]);
    if !lines.is_empty() {
        raw.extend_from_slice(b"\r\n");
    }
    raw
}

/// Split a raw message into its header block and the body offset.
///
/// Headers are read lossily; the body stays bytes so its part charset decides.
fn split_headers(raw: &[u8]) -> (String, usize) {
    for separator in [b"\r\n\r\n".as_slice(), b"\n\n".as_slice()] {
        if let Some(index) = raw
            .windows(separator.len())
            .position(|window| window == separator)
        {
            return (
                String::from_utf8_lossy(&raw[..index]).into_owned(),
                index + separator.len(),
            );
        }
    }
    (String::from_utf8_lossy(raw).into_owned(), raw.len())
}

fn envelope_from_raw(uidl: &str, raw: &[u8], receiver: &str) -> MessageEnvelope {
    let (header, body_offset) = split_headers(raw);
    let parsed_body = parse_message_body(&header, Some(&raw[body_offset..]));
    let subject = parse_header_value(&header, "Subject").unwrap_or_else(|| "(no subject)".into());
    let snippet = build_snippet(
        &subject,
        parsed_body.text.as_deref(),
        parsed_body.html.as_deref(),
    );

    let auth_results = parse_header_value(&header, "Authentication-Results")
        .map(|value| crate::providers::traits::trusted_authentication_results(&value, receiver));

    MessageEnvelope {
        remote_id: uidl.to_string(),
        remote_uidvalidity: None,
        remote_thread_id: None,
        original_source: Some(raw.to_vec()),
        mailbox_ids: vec![POP3_FOLDER.to_string()],
        message_id_header: parse_header_value(&header, "Message-ID"),
        in_reply_to: parse_header_value(&header, "In-Reply-To"),
        references: parse_header_value(&header, "References")
            .map(|raw| split_references(&raw))
            .unwrap_or_default(),
        subject,
        snippet,
        from_address: parse_from_address(&header).unwrap_or_else(|| "unknown@pop3.local".into()),
        from_name: parse_from_name(&header),
        to_addresses: parse_header_address_list(&header, "To"),
        cc_addresses: parse_header_address_list(&header, "Cc"),
        bcc_addresses: Vec::new(),
        reply_to: parse_header_value(&header, "Reply-To"),
        remote_draft_id: None,
        // POP3 has no server-side receive time to fall back on.
        received_at_rfc3339: None,
        date_rfc3339: parse_message_date(&header, None, chrono::Utc::now()),
        body_text: parsed_body.text,
        body_html: parsed_body.html,
        // POP3 has no server-side flags. Everything arrives unread, and read
        // state lives only in the local database from then on.
        is_read: false,
        is_starred: false,
        body_calendar: parsed_body.calendar,
        smime_encrypted: parsed_body.smime_encrypted,
        smime_signed: parsed_body.smime_signed,
        pgp_signed: parsed_body.pgp_signed,
        trust: super::traits::MessageTrust {
            auth_verified: auth_results.as_ref().is_some_and(|(spf, dkim, dmarc)| {
                spf.is_some() || dkim.is_some() || dmarc.is_some()
            }),
            ..parsed_body.trust
        },
        list_unsubscribe: parse_header_value(&header, "List-Unsubscribe"),
        list_unsubscribe_post: parse_header_value(&header, "List-Unsubscribe-Post"),
        auth_spf: auth_results.as_ref().and_then(|results| results.0.clone()),
        auth_dkim: auth_results.as_ref().and_then(|results| results.1.clone()),
        auth_dmarc: auth_results.as_ref().and_then(|results| results.2.clone()),
        attachments: parsed_body.attachments,
    }
}

/// Cap per sync run so a first connection to a large mailbox does not block
/// the UI for minutes; the rest follows on the next pass.
fn fetch_limit() -> usize {
    std::env::var("SATCHEL_POP3_FETCH_LIMIT")
        .ok()
        .and_then(|raw| raw.parse().ok())
        .filter(|value| *value > 0)
        .unwrap_or(200)
}

#[async_trait]
impl MailProvider for Pop3Provider {
    async fn sync_folders(&self) -> crate::sync::errors::SyncResult<Vec<RemoteFolder>> {
        // POP3 has exactly one mailbox and no way to name it.
        Ok(vec![RemoteFolder {
            id: POP3_FOLDER.to_string(),
            name: POP3_FOLDER.to_string(),
            kind: Some("inbox".to_string()),
        }])
    }

    /// Download what the cursor has not seen yet. Never deletes: the sync run
    /// queues the removal of the UIDLs it managed to commit, so nothing leaves
    /// the server that is not stored locally.
    async fn sync_messages(
        &self,
        _folder: &str,
        since: SyncCheckpoint,
    ) -> crate::sync::errors::SyncResult<SyncBatch> {
        let mut state = Pop3CursorState::from_checkpoint(&since);
        let seen: HashSet<String> = state.seen_uidls.iter().cloned().collect();

        let mut session = self.connect().await?;
        let listing = parse_uidl_listing(&session.multiline_command("UIDL").await?);

        let pending: Vec<(u32, String)> = listing
            .iter()
            .filter(|(_, uidl)| !seen.contains(uidl))
            .cloned()
            .collect();

        let mut messages = Vec::new();
        let mut fetched_uidls = Vec::new();
        let mut source_bytes = 0usize;
        for (number, uidl) in pending.iter().take(fetch_limit()) {
            let raw = match session.retr(*number).await {
                Ok(Retrieved::Body(lines)) => join_crlf(&lines),
                // The server answered on a stream that is still in step. Skip
                // this message instead of ending the run: a single permanently
                // unreadable mail would otherwise stall every later sync for
                // this account. Its UIDL stays unseen, so a later run retries.
                Ok(Retrieved::Rejected(reason)) => {
                    tracing::warn!(uidl = %uidl, %reason, "pop3 server refused RETR, skipping");
                    continue;
                }
                // A response we did not read to its terminating dot leaves the
                // stream out of step, so every later RETR would return the
                // previous message's bytes under the wrong UIDL. Drop the
                // session rather than carry on with a desynchronised one.
                Err(err) => {
                    tracing::warn!(uidl = %uidl, %err, "pop3 RETR failed, aborting run");
                    return Err(format!("pop3 RETR failed for {uidl}: {err}").into());
                }
            };
            source_bytes = source_bytes.saturating_add(raw.len());
            messages.push(envelope_from_raw(
                uidl,
                &raw,
                self.config
                    .as_ref()
                    .map(|config| config.host.as_str())
                    .unwrap_or_default(),
            ));
            fetched_uidls.push(uidl.clone());
            // Originals now live until the batch commits. Bound the batch by
            // bytes too; 200 large attachments must not accumulate in RAM.
            if source_bytes >= 64 * 1024 * 1024 {
                break;
            }
        }

        let _ = session.quit().await;

        state.seen_uidls.extend(fetched_uidls);
        // A UIDL that remains on the server must remain in the cursor. Evicting
        // old entries redownloads them before newer mail on every sync run.
        let listed: HashSet<&str> = listing.iter().map(|(_, uidl)| uidl.as_str()).collect();
        state
            .seen_uidls
            .retain(|uidl| listed.contains(uidl.as_str()));

        Ok(SyncBatch {
            messages,
            // Deletions on the server are not observable in a way we can map to
            // local messages: a UIDL simply stops being listed, which also
            // happens with leave-on-server mailboxes pruned by the provider.
            removals: Vec::new(),
            remote_uidvalidity: None,
            reset_folder: false,
            snapshot_required: false,
            next: SyncCheckpoint {
                cursor: state.to_cursor(),
            },
        })
    }

    async fn send(&self, message: OutgoingMessage) -> crate::sync::errors::SyncResult<()> {
        // POP3 cannot send; a POP3 account is paired with an SMTP server.
        Ok(send_via_smtp(&self.cfg()?.smtp, message).await?)
    }

    async fn set_flags(
        &self,
        _messages: &[MessageRef],
        _flags: FlagUpdate,
    ) -> crate::sync::errors::SyncResult<()> {
        // Not an error: read state and stars are local by design for POP3, and
        // failing here would surface a red sync error for a normal action.
        Ok(())
    }

    async fn move_messages(
        &self,
        _messages: &[MessageRef],
        _target: &str,
    ) -> crate::sync::errors::SyncResult<()> {
        Err("POP3 has no folders; moving messages is not supported".into())
    }

    async fn delete_messages(
        &self,
        messages: &[MessageRef],
    ) -> crate::sync::errors::SyncResult<()> {
        if messages.is_empty() {
            return Ok(());
        }

        // DELE addresses messages by their per-session number, which is not
        // stable, so map UIDLs to numbers within this connection.
        let mut session = self.connect().await?;
        let listing = parse_uidl_listing(&session.multiline_command("UIDL").await?);
        let wanted: HashSet<&str> = messages
            .iter()
            .map(|message| message.remote_id.as_str())
            .collect();

        for (number, uidl) in listing {
            if wanted.contains(uidl.as_str()) {
                session.command(&format!("DELE {number}")).await?;
            }
        }

        // QUIT commits DELE. A refusal or dropped connection stays retryable.
        session
            .quit()
            .await
            .map_err(crate::sync::errors::SyncError::Transient)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = r#"{"pop3":{"host":"pop.example.test","username":"me@example.test","password":"secret"},"smtp":{"host":"smtp.example.test","port":587,"from_address":"me@example.test"}}"#;

    #[tokio::test]
    async fn deletions_require_a_successful_quit() {
        for reply in [
            Some("+OK committed\r\n"),
            Some("-ERR update failed\r\n"),
            Some("+OKAY invalid status\r\n"),
            None,
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let server = tokio::spawn(async move {
                let (socket, _) = listener.accept().await.unwrap();
                let mut socket = BufReader::new(socket);
                socket.get_mut().write_all(b"+OK ready\r\n").await.unwrap();
                loop {
                    let mut line = String::new();
                    if socket.read_line(&mut line).await.unwrap() == 0 {
                        break;
                    }
                    let response = match line.trim() {
                        "UIDL" => "+OK listing\r\n1 uidl\r\n.\r\n",
                        "QUIT" => {
                            if let Some(reply) = reply {
                                socket.get_mut().write_all(reply.as_bytes()).await.unwrap();
                            }
                            break;
                        }
                        _ => "+OK\r\n",
                    };
                    socket
                        .get_mut()
                        .write_all(response.as_bytes())
                        .await
                        .unwrap();
                }
            });
            let result = provider_on(port)
                .delete_messages(&[MessageRef {
                    remote_id: "uidl".into(),
                    source_folder: None,
                    source_uidvalidity: None,
                }])
                .await;
            assert_eq!(result.is_ok(), reply == Some("+OK committed\r\n"));
            if let Err(error) = result {
                assert!(!error.is_permanent());
            }
            server.await.unwrap();
        }
    }

    /// POP3 has no second parser to fall back on: a signed structure that is
    /// refused as signed must still arrive as readable, unsigned mail.
    #[test]
    fn a_refused_signed_structure_arrives_unsigned_not_blank() {
        let raw = include_bytes!("../../tests/corpus/pgp-mime/forged-quoted-boundary.eml");
        let envelope = super::envelope_from_raw("x", raw, "unknown");
        assert!(envelope.pgp_signed.is_none());
        assert!(!envelope.trust.root_crypto);
        assert!(
            envelope
                .body_text
                .as_deref()
                .is_some_and(|text| !text.trim().is_empty()),
            "{:?}",
            envelope.body_text
        );
    }

    #[test]
    fn rejects_markup_in_attachment_mime_types() {
        let raw = b"Content-Type: multipart/related; boundary=x\r\n\r\n--x\r\nContent-Type: image/png\"><link rel=preconnect href=https://u1.t.example>\r\nContent-ID: <x>\r\nContent-Disposition: inline; filename=logo.png\r\n\r\nAAAA\r\n--x--\r\n";
        let envelope = super::envelope_from_raw("x", raw, "unknown");
        assert_eq!(
            envelope.attachments[0].content_type,
            "application/octet-stream"
        );
    }

    #[test]
    fn defaults_to_implicit_tls_on_995() {
        let provider = Pop3Provider::from_account_config(CONFIG, "me@example.test").unwrap();
        let cfg = provider.config.unwrap();
        assert!(cfg.tls);
        assert_eq!(cfg.port, 995);
    }

    #[test]
    fn plain_configuration_uses_110() {
        let json = CONFIG.replace(
            r#""host":"pop.example.test""#,
            r#""host":"pop.example.test","tls":false"#,
        );
        let cfg = Pop3Provider::from_account_config(&json, "me@example.test")
            .unwrap()
            .config
            .unwrap();
        assert!(!cfg.tls);
        assert_eq!(cfg.port, 110);
    }

    fn runtime_config(port: u16, tls: bool) -> Pop3RuntimeConfig {
        Pop3RuntimeConfig {
            // TEST-NET-1: routable nowhere, so a socket attempt cannot succeed
            // by accident and the assertion stays about the refusal itself.
            host: "192.0.2.1".to_owned(),
            port,
            tls,
            username: "me@example.test".to_owned(),
            password: "secret".to_owned(),
            smtp: SmtpRuntimeConfig {
                host: "192.0.2.1".to_owned(),
                port: 587,
                tls: true,
                username: "me@example.test".to_owned(),
                password: "secret".to_owned(),
                from_address: "me@example.test".to_owned(),
            },
        }
    }

    /// The password would otherwise go out as `PASS hunter2` on a port that
    /// has no way to upgrade the session.
    #[tokio::test]
    async fn port_110_without_explicit_plaintext_opt_in_is_rejected() {
        let provider = Pop3Provider {
            config: Some(runtime_config(110, true)),
        };

        let error = match provider.connect().await {
            Err(error) => error,
            Ok(_) => panic!("port 110 must not be connected to with TLS requested"),
        };

        // The refusal, not a connect timeout: no socket was opened.
        assert_eq!(error, "backend.errors.pop3PlaintextRejected");
    }

    /// A configuration that says `tls: false` has made the choice already.
    #[test]
    fn an_explicit_plaintext_configuration_still_reaches_the_clear_path() {
        let cfg = Pop3Provider::from_account_config(
            &CONFIG.replace(
                r#""host":"pop.example.test""#,
                r#""host":"pop.example.test","tls":false,"port":110"#,
            ),
            "me@example.test",
        )
        .expect("plaintext config")
        .config
        .expect("runtime config");

        assert!(!super::rejects_plaintext(&cfg));
        assert!(super::rejects_plaintext(&runtime_config(110, true)));
        assert!(!super::rejects_plaintext(&runtime_config(995, true)));
    }

    /// Deleting the user's only copy of their mail must never be the default.
    #[test]
    fn leaves_mail_on_the_server_unless_asked_otherwise() {
        use crate::accounts::AccountConfig;
        assert!(!super::deletes_after_fetch(&AccountConfig::from_plaintext(CONFIG)).unwrap());

        let opt_in = CONFIG.replace(
            r#""password":"secret""#,
            r#""password":"secret","leave_on_server":false"#,
        );
        assert!(super::deletes_after_fetch(&AccountConfig::from_plaintext(opt_in)).unwrap());
    }

    #[test]
    fn smtp_falls_back_to_the_pop3_credentials() {
        let minimal = r#"{"pop3":{"host":"mail.example.test","username":"me@example.test","password":"secret"}}"#;
        let cfg = Pop3Provider::from_account_config(minimal, "me@example.test")
            .unwrap()
            .config
            .unwrap();
        assert_eq!(cfg.smtp.host, "mail.example.test");
        assert_eq!(cfg.smtp.username, "me@example.test");
        assert_eq!(cfg.smtp.password, "secret");
    }

    #[test]
    fn a_missing_host_is_an_error_rather_than_a_silent_default() {
        assert!(Pop3Provider::from_account_config(r#"{"pop3":{}}"#, "me@example.test").is_err());
    }

    #[test]
    fn parses_uidl_listings_and_ignores_malformed_lines() {
        let lines = vec![
            "1 abc123".to_string(),
            "2 def456".to_string(),
            "garbage".to_string(),
            "3".to_string(),
            "4 ghi789 extra".to_string(),
        ];
        assert_eq!(
            parse_uidl_listing(&lines),
            vec![
                (1, "abc123".to_string()),
                (2, "def456".to_string()),
                (4, "ghi789".to_string()),
            ]
        );
    }

    #[test]
    fn a_cursor_round_trips_through_the_checkpoint() {
        let state = Pop3CursorState {
            seen_uidls: vec!["a".into(), "b".into()],
        };
        let restored = Pop3CursorState::from_checkpoint(&SyncCheckpoint {
            cursor: state.to_cursor(),
        });
        assert_eq!(restored.seen_uidls, vec!["a".to_string(), "b".to_string()]);
    }

    #[tokio::test]
    async fn a_large_leave_on_server_mailbox_keeps_old_uidls_seen() {
        let messages = (1..=50_001).map(plain_message).collect();
        let port = spawn_fake_server(messages, None, None).await;
        let previous = Pop3CursorState {
            seen_uidls: (1..=50_000).map(|n| format!("UIDL-{n}")).collect(),
        };
        let batch = provider_on(port)
            .sync_messages(
                POP3_FOLDER,
                SyncCheckpoint {
                    cursor: previous.to_cursor(),
                },
            )
            .await
            .expect("sync newest message");

        assert_eq!(batch.messages.len(), 1);
        assert_eq!(batch.messages[0].remote_id, "UIDL-50001");
        let remembered = Pop3CursorState::from_checkpoint(&batch.next);
        assert!(remembered.seen_uidls.contains(&"UIDL-1".to_string()));
        assert!(remembered.seen_uidls.contains(&"UIDL-50001".to_string()));

        let port = spawn_fake_server((1..=50_001).map(plain_message).collect(), None, None).await;
        let next = provider_on(port)
            .sync_messages(POP3_FOLDER, batch.next)
            .await
            .expect("next sync");
        assert!(
            next.messages.is_empty(),
            "seen mail must not be fetched again"
        );
    }

    #[test]
    fn an_absent_or_broken_cursor_starts_from_scratch() {
        for cursor in [None, Some("not json".to_string())] {
            let state = Pop3CursorState::from_checkpoint(&SyncCheckpoint { cursor });
            assert!(state.seen_uidls.is_empty());
        }
    }

    #[test]
    fn builds_an_envelope_from_a_raw_message() {
        let raw = "From: Alice <alice@example.test>\r\nTo: me@example.test\r\nSubject: Angebot\r\nMessage-ID: <m1@example.test>\r\nDate: Wed, 15 Jul 2026 10:00:00 +0000\r\n\r\nHallo Welt\r\n";
        let envelope = envelope_from_raw("uidl-1", raw.as_bytes(), "unknown");

        assert_eq!(envelope.remote_id, "uidl-1");
        assert_eq!(envelope.original_source.as_deref(), Some(raw.as_bytes()));
        assert_eq!(envelope.subject, "Angebot");
        assert_eq!(envelope.from_address, "alice@example.test");
        assert_eq!(envelope.from_name.as_deref(), Some("Alice"));
        assert_eq!(
            envelope.message_id_header.as_deref(),
            Some("<m1@example.test>")
        );
        // POP3 reports no flags, so everything starts unread.
        assert!(!envelope.is_read);
        assert!(!envelope.is_starred);
    }

    #[test]
    fn carries_rfc3156_signed_mime_into_the_envelope() {
        let raw = "From: Alice <alice@example.test>\r\n\
Subject: Signed\r\n\
Content-Type: multipart/signed; protocol=\"application/pgp-signature\"; boundary=\"pgp\"\r\n\
\r\n\
--pgp\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
Signed body\r\n\
--pgp\r\n\
Content-Type: application/pgp-signature\r\n\
\r\n\
-----BEGIN PGP SIGNATURE-----\r\n\
-----END PGP SIGNATURE-----\r\n\
--pgp--\r\n";

        let envelope = envelope_from_raw("uidl-pgp", raw.as_bytes(), "unknown");
        let signed = String::from_utf8(envelope.pgp_signed.expect("PGP signed MIME"))
            .expect("ASCII fixture");

        assert!(signed.starts_with(
            "Content-Type: multipart/signed; protocol=\"application/pgp-signature\"; boundary=\"pgp\"\r\n\r\n--pgp\r\n"
        ));
        assert!(signed.ends_with("--pgp--\r\n"));
    }

    #[test]
    fn splits_headers_from_body_for_both_line_endings() {
        let (header, offset) = split_headers(b"A: 1\r\nB: 2\r\n\r\nbody");
        assert_eq!(header, "A: 1\r\nB: 2");
        assert_eq!(&"A: 1\r\nB: 2\r\n\r\nbody"[offset..], "body");

        let (header, _) = split_headers(b"A: 1\nB: 2\n\nbody");
        assert_eq!(header, "A: 1\nB: 2");
    }

    #[test]
    fn a_message_without_a_body_still_parses() {
        let envelope = envelope_from_raw("uidl-2", b"Subject: Nur Kopf\r\n", "unknown");
        assert_eq!(envelope.subject, "Nur Kopf");
    }

    fn plain_message(index: usize) -> Vec<u8> {
        format!(
            "From: Alice <alice@example.test>\r\n\
Subject: msg-{index}\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
BODY-OF-MSG-{index}"
        )
        .into_bytes()
    }

    /// A body byte that is not valid UTF-8 — the case that used to abort RETR.
    fn latin1_message(index: usize) -> Vec<u8> {
        let mut raw = format!(
            "From: Alice <alice@example.test>\r\n\
Subject: msg-{index}\r\n\
Content-Type: text/plain; charset=iso-8859-1\r\n\
Content-Transfer-Encoding: 8bit\r\n\
\r\n\
BODY-OF-MSG-{index} Gr"
        )
        .into_bytes();
        raw.push(0xE4);
        raw.extend_from_slice(b"sse");
        raw
    }

    /// Minimal POP3 server over a loopback socket. `refuse_retr` answers that
    /// message with `-ERR` on an intact stream; `cut_off_retr` makes it hang
    /// up in the middle of that message's response.
    async fn spawn_fake_server(
        messages: Vec<Vec<u8>>,
        cut_off_retr: Option<usize>,
        refuse_retr: Option<usize>,
    ) -> u16 {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback");
        let port = listener.local_addr().expect("local addr").port();

        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept");
            let mut reader = BufReader::new(stream);
            if reader
                .get_mut()
                .write_all(b"+OK fake pop3 ready\r\n")
                .await
                .is_err()
            {
                return;
            }

            loop {
                let mut request = Vec::new();
                if reader.read_until(b'\n', &mut request).await.unwrap_or(0) == 0 {
                    return;
                }
                let request = String::from_utf8_lossy(&request).trim_end().to_string();
                let (verb, argument) = match request.split_once(' ') {
                    Some((verb, argument)) => {
                        (verb.to_ascii_uppercase(), argument.trim().to_string())
                    }
                    None => (request.to_ascii_uppercase(), String::new()),
                };

                let mut response = Vec::new();
                match verb.as_str() {
                    "UIDL" => {
                        response.extend_from_slice(b"+OK listing follows\r\n");
                        for number in 1..=messages.len() {
                            response.extend_from_slice(
                                format!("{number} UIDL-{number}\r\n").as_bytes(),
                            );
                        }
                        response.extend_from_slice(b".\r\n");
                    }
                    "RETR" => {
                        let number: usize = argument.parse().unwrap_or(0);
                        if refuse_retr == Some(number) {
                            response.extend_from_slice(b"-ERR message is corrupt\r\n");
                            if reader.get_mut().write_all(&response).await.is_err() {
                                return;
                            }
                            continue;
                        }
                        if cut_off_retr == Some(number) {
                            let _ = reader
                                .get_mut()
                                .write_all(b"+OK message follows\r\nSubject: cut off\r\n")
                                .await;
                            let _ = reader.get_mut().shutdown().await;
                            return;
                        }
                        match number.checked_sub(1).and_then(|index| messages.get(index)) {
                            Some(message) => {
                                response.extend_from_slice(b"+OK message follows\r\n");
                                response.extend_from_slice(message);
                                response.extend_from_slice(b"\r\n.\r\n");
                            }
                            None => response.extend_from_slice(b"-ERR no such message\r\n"),
                        }
                    }
                    "QUIT" => {
                        let _ = reader.get_mut().write_all(b"+OK bye\r\n").await;
                        return;
                    }
                    _ => response.extend_from_slice(b"+OK\r\n"),
                }

                if reader.get_mut().write_all(&response).await.is_err() {
                    return;
                }
            }
        });

        port
    }

    fn provider_on(port: u16) -> Pop3Provider {
        let json = format!(
            r#"{{"pop3":{{"host":"127.0.0.1","port":{port},"tls":false,"username":"me@example.test","password":"secret"}}}}"#
        );
        Pop3Provider::from_account_config(&json, "me@example.test").expect("provider config")
    }

    fn seen_uidls(batch: &SyncBatch) -> Vec<String> {
        Pop3CursorState::from_checkpoint(&batch.next).seen_uidls
    }

    #[tokio::test]
    async fn retrieves_original_bytes_by_uidl_instead_of_sequence_position() {
        let expected = latin1_message(2);
        let port = spawn_fake_server(vec![plain_message(1), expected.clone()], None, None).await;
        let raw = provider_on(port).fetch_raw_message("UIDL-2").await.unwrap();
        assert_eq!(raw, [expected.as_slice(), b"\r\n"].concat());
    }

    /// The regression: an 8-bit body used to abort RETR after its bytes were
    /// already consumed, so every following message was stored under the
    /// previous one's UIDL.
    #[tokio::test]
    async fn retr_with_an_eight_bit_body_keeps_the_stream_aligned() {
        let messages = vec![
            plain_message(1),
            plain_message(2),
            latin1_message(3),
            plain_message(4),
        ];
        let port = spawn_fake_server(messages, None, None).await;
        let batch = provider_on(port)
            .sync_messages(POP3_FOLDER, SyncCheckpoint { cursor: None })
            .await
            .expect("sync succeeds despite the 8-bit body");

        assert_eq!(batch.messages.len(), 4);
        for (index, envelope) in batch.messages.iter().enumerate() {
            let number = index + 1;
            assert_eq!(envelope.remote_id, format!("UIDL-{number}"));
            assert_eq!(envelope.subject, format!("msg-{number}"));
            let body = envelope.body_text.clone().unwrap_or_default();
            assert!(
                body.contains(&format!("BODY-OF-MSG-{number}")),
                "UIDL-{number} carries foreign content: {body:?}"
            );
        }

        let latin1_body = batch.messages[2].body_text.clone().unwrap_or_default();
        assert!(
            latin1_body.contains('ä'),
            "expected decoded ä, got {latin1_body:?}"
        );
        assert!(!latin1_body.contains('\u{fffd}'));

        assert_eq!(
            seen_uidls(&batch),
            vec!["UIDL-1", "UIDL-2", "UIDL-3", "UIDL-4"]
        );
    }

    /// A response cut short leaves the connection unusable, not just that one
    /// message: the run must end without recording any further UIDL.
    #[tokio::test]
    async fn a_truncated_retr_response_aborts_the_run() {
        let messages = vec![
            plain_message(1),
            plain_message(2),
            plain_message(3),
            plain_message(4),
        ];
        let port = spawn_fake_server(messages, Some(2), None).await;
        let error = provider_on(port)
            .sync_messages(POP3_FOLDER, SyncCheckpoint { cursor: None })
            .await
            .expect_err("a truncated RETR must fail the run");

        assert!(
            error.to_string().contains("UIDL-2"),
            "unexpected error: {error}"
        );
    }

    /// `-ERR` is answered on a stream that is still in step. One message the
    /// server will never hand over must not stall the mailbox for good.
    #[tokio::test]
    async fn a_refused_retr_skips_that_message_and_keeps_going() {
        let messages = vec![
            plain_message(1),
            plain_message(2),
            plain_message(3),
            plain_message(4),
        ];
        let port = spawn_fake_server(messages, None, Some(2)).await;
        let batch = provider_on(port)
            .sync_messages(POP3_FOLDER, SyncCheckpoint { cursor: None })
            .await
            .expect("a refused RETR must not fail the run");

        let fetched: Vec<&str> = batch
            .messages
            .iter()
            .map(|envelope| envelope.remote_id.as_str())
            .collect();
        assert_eq!(fetched, vec!["UIDL-1", "UIDL-3", "UIDL-4"]);
        for envelope in &batch.messages {
            let number = envelope.remote_id.trim_start_matches("UIDL-");
            let body = envelope.body_text.clone().unwrap_or_default();
            assert!(
                body.contains(&format!("BODY-OF-MSG-{number}")),
                "{} carries foreign content: {body:?}",
                envelope.remote_id
            );
        }

        // Left unseen on purpose: a later run tries it again rather than
        // writing it off.
        let seen = seen_uidls(&batch);
        assert!(
            !seen.iter().any(|uidl| uidl == "UIDL-2"),
            "refused UIDL must stay unseen"
        );
        assert!(
            seen.iter().any(|uidl| uidl == "UIDL-4"),
            "later messages must be recorded"
        );
    }
}
