pub use super::smtp::{send_via_smtp, send_via_smtp_returning_wire_copy, SmtpRuntimeConfig};
use base64::Engine;
use futures::TryStreamExt;
use postroom::SourceId;
use postroom_imap::imap_message_identity;
use postroom_rfc5322::{parse_rfc822 as parse_core_rfc822, EmailMetadata, MimeContext};
#[cfg(any(target_os = "android", target_os = "ios"))]
use rustls_platform_verifier::BuilderVerifierExt;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Instant;
use tokio::net::TcpStream;
use tokio::time::{timeout, Duration};
use tokio_rustls::{client::TlsStream, TlsConnector};

use crate::email::signed_mime::{self, SignedProtocol};
use crate::providers::{
    cursor,
    imap_sessions::{read_with_pool, SessionOutcome, SessionPool, IDLE_SESSION_TTL},
    traits::{
        FetchedAttachment, FlagUpdate, IdleCallback, IdleEvent, MailProvider, MessageEnvelope,
        MessageRef, OutgoingMessage, RemoteAttachment, RemoteFolder, RemoteMessageState, SyncBatch,
        SyncCheckpoint,
    },
};
use crate::sync::errors::{SyncError, SyncResult};
use crate::utils::mime::{
    decode_bytes_with_charset, decode_mime_encoded_words, decode_quoted_printable_bytes,
    decode_rfc2231_segments, decode_rfc2231_value, extract_email_address,
    parse_address_list_tokens, AddressListDecodeStrategy,
};

async fn collect_imap_response<F, S, T>(
    duration: Duration,
    command: F,
    operation: &str,
) -> Result<Vec<T>, String>
where
    F: std::future::Future<Output = Result<S, async_imap::error::Error>>,
    S: futures::TryStream<Ok = T, Error = async_imap::error::Error>,
{
    timeout(duration, async { command.await?.try_collect().await })
        .await
        .map_err(|_| format!("imap {operation} timed out"))?
        .map_err(|error| format!("imap {operation} failed: {error}"))
}

type ImapSession = async_imap::Session<TlsStream<TcpStream>>;

/// Idle sessions for reads (bodies, sources, attachment parts), per account.
static READ_SESSIONS: std::sync::LazyLock<Arc<SessionPool<ImapSession>>> =
    std::sync::LazyLock::new(|| Arc::new(SessionPool::new(IDLE_SESSION_TTL)));

async fn close_session(mut session: ImapSession) {
    let _ = timeout(ImapProvider::op_timeout(), session.logout()).await;
}

/// SELECT `folder` and fetch `query` for one UID on a borrowed session.
///
/// `Ok(None)` means the server answered and has no such message: the session
/// is fine and goes back. Any protocol or timeout error drops it.
async fn fetch_one_on_session<T>(
    mut session: async_imap::Session<T>,
    folder: &str,
    uid: &str,
    query: &str,
    operation: &str,
    expected_uidvalidity: Option<u32>,
) -> SessionOutcome<async_imap::Session<T>, Option<async_imap::types::Fetch>>
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + std::fmt::Debug + Send,
{
    let selected = match timeout(ImapProvider::op_timeout(), session.select(folder)).await {
        Err(_) => Err(format!("imap SELECT {folder} timed out")),
        Ok(Err(err)) => Err(format!("imap SELECT {folder} failed: {err}")),
        Ok(Ok(mailbox)) => match expected_uidvalidity {
            Some(expected) if mailbox.uid_validity != Some(expected) => {
                Err("backend.errors.imapUidValidityChanged".to_string())
            }
            _ => Ok(()),
        },
    };
    if let Err(error) = selected {
        return (None, Err(error));
    }
    match collect_imap_response(
        ImapProvider::op_timeout(),
        session.uid_fetch(uid, query),
        operation,
    )
    .await
    {
        Ok(mut fetched) => (Some(session), Ok(fetched.pop())),
        Err(error) => (None, Err(error)),
    }
}

/// One message body, header plus text, on a borrowed session.
pub(crate) async fn fetch_body_on_session<T>(
    session: async_imap::Session<T>,
    folder: String,
    uid: String,
) -> SessionOutcome<async_imap::Session<T>, ParsedBody>
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + std::fmt::Debug + Send,
{
    let (session, fetched) = fetch_one_on_session(
        session,
        &folder,
        &uid,
        "(BODY.PEEK[HEADER] BODY.PEEK[TEXT])",
        "UID FETCH body",
        None,
    )
    .await;
    let parsed = fetched.and_then(|entry| {
        let entry = entry.ok_or_else(|| "imap message not found on the server".to_string())?;
        let header = entry
            .header()
            .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
            .unwrap_or_default();
        Ok(parse_message_body(&header, entry.text()))
    });
    (session, parsed)
}

#[derive(Debug, Clone, Deserialize)]
struct FlatImapConfig {
    host: String,
    port: Option<u16>,
    tls: Option<bool>,
    username: Option<String>,
    password: Option<String>,
    smtp: Option<FlatSmtpConfig>,
    smtp_host: Option<String>,
    smtp_port: Option<u16>,
    smtp_tls: Option<bool>,
    smtp_username: Option<String>,
    smtp_password: Option<String>,
    smtp_from: Option<String>,
    smtp_from_address: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ImapDraftRef {
    folder: String,
    id: String,
}

fn encode_imap_draft_ref(folder: &str, id: &str) -> String {
    serde_json::to_string(&ImapDraftRef {
        folder: folder.to_string(),
        id: id.to_string(),
    })
    .unwrap_or_else(|_| id.to_string())
}

fn decode_imap_draft_ref(value: &str) -> Option<ImapDraftRef> {
    serde_json::from_str(value).ok()
}

fn is_numeric_imap_uid(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn permanent_imap_error(key: &str, folder: &str) -> SyncError {
    SyncError::Permanent(serde_json::json!({"key": key, "params": {"folder": folder}}).to_string())
}

fn is_missing_namespace_error(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    (error.contains("nonexistent namespace") && error.contains("inbox"))
        || error.contains("should probably be prefixed with: inbox")
}

fn inbox_scoped_folder_path(
    inbox_name: &str,
    delimiter: Option<&str>,
    path: &str,
) -> Option<String> {
    let inbox_name = inbox_name.trim();
    let delimiter = delimiter?.trim();
    let path = path.trim();
    if inbox_name.is_empty() || delimiter.is_empty() || path.is_empty() {
        return None;
    }

    let prefix = format!("{inbox_name}{delimiter}");
    if path.eq_ignore_ascii_case(inbox_name)
        || path
            .get(..prefix.len())
            .is_some_and(|value| value.eq_ignore_ascii_case(&prefix))
    {
        return None;
    }
    Some(format!("{prefix}{path}"))
}

fn namespace_retry_folder(
    error: &str,
    inbox_name: &str,
    delimiter: Option<&str>,
    path: &str,
) -> Option<String> {
    is_missing_namespace_error(error)
        .then(|| inbox_scoped_folder_path(inbox_name, delimiter, path))
        .flatten()
}

#[derive(Debug, Clone, Deserialize, Default)]
struct FlatSmtpConfig {
    host: Option<String>,
    port: Option<u16>,
    tls: Option<bool>,
    username: Option<String>,
    password: Option<String>,
    from: Option<String>,
    from_address: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct NestedImapConfig {
    imap: Option<FlatImapConfig>,
    smtp: Option<FlatSmtpConfig>,
}

#[derive(Debug, Clone)]
pub struct ImapRuntimeConfig {
    pub host: String,
    pub port: u16,
    pub tls: bool,
    pub username: String,
    pub password: String,
    pub smtp: SmtpRuntimeConfig,
}

#[derive(Debug, Clone, Default)]
pub struct ImapProvider {
    pub config: Option<ImapRuntimeConfig>,
    /// How far back history is kept with its body, in days. `None` keeps all.
    pub body_window_days: Option<u32>,
}

/// How much history a fresh account keeps in full.
///
/// Thirty days because that is what Gmail for Android and Outlook Mobile both
/// default to — Outlook will not go past ninety at all. A mailbox stored whole
/// costs what a mailbox costs and a phone has other uses for the space, so it
/// keeps a month and fetches anything older when it is opened. A desktop keeps
/// everything, which is what makes offline search over years of mail worth
/// having in the first place.
fn default_body_window_days() -> Option<u32> {
    #[cfg(mobile)]
    {
        Some(30)
    }
    #[cfg(not(mobile))]
    {
        None
    }
}

/// Read the per-account override, where 0 means "keep everything".
fn configured_body_window_days(config_json: &str) -> Option<u32> {
    let configured = serde_json::from_str::<serde_json::Value>(config_json)
        .ok()
        .and_then(|value| value.get("body_window_days")?.as_u64());
    match configured {
        Some(0) => None,
        Some(days) => Some(days.min(u64::from(u32::MAX)) as u32),
        None => default_body_window_days(),
    }
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct ImapCursorState {
    #[serde(default)]
    high_uid: u32,
    #[serde(default)]
    high_seq: u32,
    #[serde(default)]
    backfill_next_seq: u32,
    #[serde(default)]
    backfill_total_seq: u32,
    /// New messages the last run listed above `high_uid` but left for the next
    /// one, because they did not fit into its fetch budget or a chunk broke
    /// off. Counted as pending work so the caller comes back soon.
    #[serde(default)]
    incremental_pending: u32,
    #[serde(default)]
    format_version: u8,
    /// UIDVALIDITY of the mailbox this cursor belongs to. When the server reports
    /// a different value, every stored UID is void (RFC 3501) and the folder must
    /// be resynced from scratch. 0 = unknown (cursor written before this field).
    #[serde(default)]
    uidvalidity: u32,
    /// Generation `folders.uidvalidity` holds, put here by the caller through
    /// [`imap_checkpoint`]. Inbound only — it is cleared before the cursor is
    /// written back, so a run never inherits its own guess.
    #[serde(default)]
    db_uidvalidity: u32,
}

type ParsedCursorState = cursor::ParsedCursor<ImapCursorState>;

/// What the known UIDVALIDITY generations say about a stored UID anchor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenerationDecision {
    /// A recorded generation matches the server's: stored UIDs stay meaningful.
    Continue,
    /// A recorded generation differs from the server's: every stored UID is
    /// void (RFC 3501) and the folder is refetched from scratch.
    Reset,
    /// Nothing ever recorded a generation. The anchor may belong to another
    /// one, so it is not used as a lower bound — the window plus the backfill
    /// carry the folder until this run writes the generation down.
    WindowOnly,
}

/// Decide what a folder's stored UID anchor is worth.
///
/// Two places remember the generation: the cursor (since format version 4) and
/// `folders.uidvalidity`. Either one is enough to compare against the server.
/// Only when neither knows is the anchor unusable — that is the upgrade window
/// in which a generation change used to pass unnoticed and freeze the folder.
pub fn imap_generation_decision(
    cursor_uidvalidity: u32,
    db_uidvalidity: Option<u32>,
    server_uidvalidity: u32,
) -> GenerationDecision {
    let known = match cursor_uidvalidity {
        0 => db_uidvalidity.filter(|value| *value != 0),
        value => Some(value),
    };
    match known {
        Some(known) if known != server_uidvalidity => GenerationDecision::Reset,
        Some(_) => GenerationDecision::Continue,
        None => GenerationDecision::WindowOnly,
    }
}

/// Where the backfill has to pick up, if it has to at all.
///
/// Without a UID anchor a run only ever sees the tail window, and every older
/// message has to come in through the backfill. A fresh folder, a legacy
/// `seq:` cursor and a cursor whose UID never arrived are the same case, so
/// the anchor — not the cursor's provenance — is what decides.
fn backfill_resume_seq(since_uid: u32, backfill_next_seq: u32, tail_start: u32) -> Option<u32> {
    (since_uid == 0 && backfill_next_seq == 0 && tail_start > 1).then(|| tail_start - 1)
}

/// Build the checkpoint for one IMAP folder from its stored cursor.
///
/// `db_uidvalidity` is what `folders.uidvalidity` holds. Cursors written before
/// the cursor carried a generation have none of their own, and the database is
/// then the only witness of which generation their UIDs belong to.
pub fn imap_checkpoint(cursor: Option<String>, db_uidvalidity: Option<u32>) -> SyncCheckpoint {
    let Some(db_uidvalidity) = db_uidvalidity.filter(|value| *value != 0) else {
        return SyncCheckpoint { cursor };
    };
    let Some(raw) = cursor
        .as_deref()
        .map(str::trim)
        .filter(|raw| !raw.is_empty())
    else {
        return SyncCheckpoint { cursor };
    };
    // Only a current JSON cursor round-trips losslessly. A legacy one is
    // rewritten by the run anyway and keeps its own upgrade path.
    let Ok(mut state) = serde_json::from_str::<ImapCursorState>(raw) else {
        return SyncCheckpoint { cursor };
    };
    state.db_uidvalidity = db_uidvalidity;
    SyncCheckpoint {
        cursor: serde_json::to_string(&state).ok().or(cursor),
    }
}

#[derive(Debug)]
struct UidMutationGroup {
    expected_uidvalidity: u32,
    uids: Vec<u32>,
}

impl UidMutationGroup {
    fn validate_generation(&self, folder: &str, selected: Option<u32>) -> SyncResult<()> {
        match selected.filter(|value| *value != 0) {
            Some(current) if current == self.expected_uidvalidity => Ok(()),
            Some(_) => Err(permanent_imap_error(
                "backend.errors.imapUidValidityChanged",
                folder,
            )),
            None => Err(permanent_imap_error(
                "backend.errors.imapUidValidityMissing",
                folder,
            )),
        }
    }
}

impl ImapProvider {
    async fn inbox_namespace(
        session: &mut async_imap::Session<TlsStream<TcpStream>>,
    ) -> Option<(String, Option<String>)> {
        let names = collect_imap_response(
            Self::op_timeout(),
            session.list(None, Some("INBOX")),
            "LIST INBOX",
        )
        .await
        .ok()?;
        names
            .into_iter()
            .find(|name| name.name().eq_ignore_ascii_case("INBOX"))
            .map(|name| {
                (
                    name.name().to_string(),
                    name.delimiter().map(str::to_string),
                )
            })
    }

    async fn select_draft_folder(
        session: &mut async_imap::Session<TlsStream<TcpStream>>,
        folder: &str,
    ) -> Result<String, String> {
        let select_result = timeout(Self::op_timeout(), session.select(folder))
            .await
            .map_err(|_| format!("imap SELECT {folder} timed out"))?;
        if let Err(err) = select_result {
            let original_error = format!("imap SELECT {folder} failed: {err}");
            if !is_missing_namespace_error(&original_error) {
                return Err(original_error);
            }

            let inbox = Self::inbox_namespace(session).await;
            let Some(fallback) = inbox.as_ref().and_then(|(name, delimiter)| {
                namespace_retry_folder(&original_error, name, delimiter.as_deref(), folder)
            }) else {
                return Err(original_error);
            };

            timeout(Self::op_timeout(), session.select(fallback.as_str()))
                .await
                .map_err(|_| format!("imap SELECT {fallback} timed out"))?
                .map_err(|err| format!("imap SELECT {fallback} failed: {err}"))?;
            return Ok(fallback);
        }

        Ok(folder.to_string())
    }

    /// Fetch the bodies of the batch entries that are still inside the window.
    ///
    /// The backfill walks backwards, so the first rounds are mostly inside it
    /// and the later ones entirely outside — at which point this asks for
    /// nothing and costs a comparison per message. A message whose Date header
    /// cannot be read keeps its body: guessing it is old would throw away
    /// something we cannot get back cheaply.
    async fn fetch_recent_backfill_bodies<T>(
        session: &mut async_imap::Session<T>,
        batch: &[async_imap::types::Fetch],
        cutoff: chrono::DateTime<chrono::Utc>,
    ) -> Result<std::collections::HashMap<u32, Vec<u8>>, String>
    where
        T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + std::fmt::Debug + Send,
    {
        let wanted: Vec<String> = batch
            .iter()
            .filter_map(|entry| {
                let uid = entry.uid?;
                let header = entry
                    .header()
                    .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
                    .unwrap_or_default();
                let inside = chrono::DateTime::parse_from_rfc3339(&parse_message_date(
                    &header,
                    entry_internal_date(entry),
                    chrono::Utc::now(),
                ))
                .map(|date| date.with_timezone(&chrono::Utc) >= cutoff)
                .unwrap_or(true);
                inside.then(|| uid.to_string())
            })
            .collect();

        let mut bodies = std::collections::HashMap::new();
        if wanted.is_empty() {
            return Ok(bodies);
        }

        let entries = collect_imap_response(
            Self::op_timeout(),
            session.uid_fetch(wanted.join(","), "(UID BODY.PEEK[TEXT])"),
            "BACKFILL body",
        )
        .await?;

        for entry in entries {
            if let (Some(uid), Some(text)) = (entry.uid, entry.text()) {
                bodies.insert(uid, text.to_vec());
            }
        }
        Ok(bodies)
    }

    /// The oldest date whose body is still stored, if history is bounded.
    fn body_cutoff(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        let days = self.body_window_days?;
        Some(chrono::Utc::now() - chrono::Duration::days(i64::from(days)))
    }

    /// Fetch the body of one message that was stored without one.
    ///
    /// Subject, sender, date and threading are already right in the database —
    /// they came from the header the sync did keep. Only the body is missing,
    /// so only the body is asked for, and the caller writes it into the row
    /// that already exists rather than ingesting a second copy.
    pub async fn fetch_message_body(&self, message: &MessageRef) -> Result<ParsedBody, String> {
        let folder = message.source_folder.as_deref().unwrap_or("INBOX");
        read_with_pool(
            &READ_SESSIONS,
            &self.session_key(),
            || self.login_session(),
            |session| fetch_body_on_session(session, folder.to_string(), message.remote_id.clone()),
            close_session,
        )
        .await
    }

    /// Which pooled sessions may serve this provider: same server, same login.
    fn session_key(&self) -> String {
        self.config
            .as_ref()
            .map(|config| format!("{}:{}/{}", config.host, config.port, config.username))
            .unwrap_or_default()
    }

    /// Fetch one complete RFC 822 message without marking it as read.
    ///
    /// This stays separate from body hydration: callers that inspect source or
    /// repair cryptographic metadata need the exact headers, MIME boundaries
    /// and transfer encodings, not the decoded display body.
    pub async fn fetch_raw_message(&self, message: &MessageRef) -> Result<Vec<u8>, String> {
        if message
            .source_uidvalidity
            .filter(|value| *value != 0)
            .is_none()
        {
            return Err("backend.errors.imapUidValidityMissing".into());
        }
        let folder = message.source_folder.as_deref().unwrap_or("INBOX");
        read_with_pool(
            &READ_SESSIONS,
            &self.session_key(),
            || self.login_session(),
            |session| async move {
                let (session, fetched) = fetch_one_on_session(
                    session,
                    folder,
                    &message.remote_id,
                    "(BODY.PEEK[])",
                    "UID FETCH source",
                    message.source_uidvalidity,
                )
                .await;
                let raw = fetched.and_then(|entry| {
                    entry
                        .and_then(|entry| entry.body().map(<[u8]>::to_vec))
                        .ok_or_else(|| "imap message source not found on the server".to_string())
                });
                (session, raw)
            },
            close_session,
        )
        .await
    }

    /// Identifies a message's parsed parts across provider instances.
    ///
    /// Each attachment used to build its own provider, so the cache has to live
    /// outside them; the account fields keep two accounts' UID 42 apart.
    fn attachment_cache_key(&self, folder: &str, message: &MessageRef) -> AttachmentCacheKey {
        let account = self
            .config
            .as_ref()
            .map(|config| format!("{}:{}/{}", config.host, config.port, config.username))
            .unwrap_or_default();
        AttachmentCacheKey {
            account,
            folder: folder.to_string(),
            // A UID identifies a message only within one UIDVALIDITY.
            uidvalidity: message.source_uidvalidity,
            remote_id: message.remote_id.clone(),
        }
    }

    /// Every attachment part of a message, downloaded and decoded at most once.
    ///
    /// The caller used to fetch the whole message and decode every part to keep
    /// one: opening five attachments downloaded all five five times over, and a
    /// newsletter with ten inline images made one sync run fetch the same body
    /// ten times.
    async fn attachment_parts<F, Fut>(
        key: &AttachmentCacheKey,
        load: F,
    ) -> crate::sync::errors::SyncResult<Arc<Vec<ParsedAttachment>>>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = crate::sync::errors::SyncResult<Vec<ParsedAttachment>>>,
    {
        if let Some(parts) = cached_attachment_parts(key, Instant::now()) {
            return Ok(parts);
        }
        let parts = Arc::new(load().await?);
        store_attachment_parts(key, &parts, Instant::now());
        Ok(parts)
    }

    /// Fetch a message body and parse out its attachment parts.
    async fn load_attachment_parts(
        &self,
        folder: &str,
        remote_id: &str,
    ) -> crate::sync::errors::SyncResult<Vec<ParsedAttachment>> {
        let attachments = read_with_pool(
            &READ_SESSIONS,
            &self.session_key(),
            || self.login_session(),
            |session| async move {
                let (session, fetched) = fetch_one_on_session(
                    session,
                    folder,
                    remote_id,
                    "(BODY.PEEK[HEADER] BODY.PEEK[TEXT])",
                    "UID FETCH attachment",
                    None,
                )
                .await;
                let parts = fetched.and_then(|entry| {
                    let entry =
                        entry.ok_or_else(|| "imap attachment message not found".to_string())?;
                    let header = entry
                        .header()
                        .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
                        .unwrap_or_default();
                    let content_type = parse_header_value(&header, "Content-Type")
                        .unwrap_or_else(|| "text/plain; charset=utf-8".to_string());
                    let transfer_encoding =
                        parse_header_value(&header, "Content-Transfer-Encoding")
                            .unwrap_or_else(|| "8bit".to_string());
                    Ok(parse_attachments_from_body(
                        &content_type,
                        &transfer_encoding,
                        entry.text().unwrap_or_default(),
                    ))
                });
                (session, parts)
            },
            close_session,
        )
        .await?;
        Ok(attachments)
    }

    // v4 deliberately replays every envelope once so installations that
    // predate reply-aware IMAP sync recover Message-ID/In-Reply-To/References.
    const CURSOR_FORMAT_VERSION: u8 = 4;

    fn op_timeout() -> Duration {
        let secs = std::env::var("SATCHEL_IMAP_TIMEOUT_SECS")
            .ok()
            .and_then(|raw| raw.parse::<u64>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(20);
        Duration::from_secs(secs)
    }

    fn sync_all_folders() -> bool {
        match std::env::var("SATCHEL_IMAP_SYNC_ALL_FOLDERS")
            .ok()
            .as_deref()
            .map(str::trim)
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("0" | "false" | "no" | "off") => false,
            Some(_) => true,
            None => true,
        }
    }

    fn initial_window() -> u32 {
        std::env::var("SATCHEL_IMAP_INITIAL_WINDOW")
            .ok()
            .and_then(|raw| raw.parse::<u32>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(100)
    }

    fn backfill_window(remaining: u32) -> u32 {
        if let Some(override_window) = std::env::var("SATCHEL_IMAP_BACKFILL_WINDOW")
            .ok()
            .and_then(|raw| raw.parse::<u32>().ok())
            .filter(|value| *value > 0)
        {
            return override_window.min(2_000);
        }

        if remaining > 20_000 {
            1_000
        } else if remaining > 10_000 {
            800
        } else if remaining > 5_000 {
            600
        } else if remaining > 2_000 {
            400
        } else {
            250
        }
    }

    pub fn from_account_config(config_json: &str, fallback_username: &str) -> Result<Self, String> {
        let nested = serde_json::from_str::<NestedImapConfig>(config_json).ok();
        let flat = nested
            .as_ref()
            .and_then(|value| value.imap.clone())
            .or_else(|| serde_json::from_str::<FlatImapConfig>(config_json).ok())
            .ok_or_else(|| "invalid imap config JSON".to_string())?;

        let username = flat
            .username
            .unwrap_or_else(|| fallback_username.to_string());

        let password = flat.password.or_else(|| {
            crate::utils::environment::debug_env("SATCHEL_IMAP_PASSWORD")
                .or_else(|| crate::utils::environment::debug_env("SATCHEL_IMAP_TEST_PASSWORD"))
        });

        let password = password.ok_or_else(|| {
            "missing IMAP password in account config and SATCHEL_IMAP_PASSWORD env".to_string()
        })?;

        let nested_smtp = nested.and_then(|value| value.smtp);
        let inline_smtp = flat.smtp.clone();
        let smtp_tls = nested_smtp
            .as_ref()
            .and_then(|value| value.tls)
            .or_else(|| inline_smtp.as_ref().and_then(|value| value.tls))
            .or(flat.smtp_tls)
            .unwrap_or(true);
        let smtp_host = nested_smtp
            .as_ref()
            .and_then(|value| value.host.as_ref())
            .or_else(|| inline_smtp.as_ref().and_then(|value| value.host.as_ref()))
            .or(flat.smtp_host.as_ref())
            .cloned()
            .unwrap_or_else(|| flat.host.clone());
        let smtp_port = nested_smtp
            .as_ref()
            .and_then(|value| value.port)
            .or_else(|| inline_smtp.as_ref().and_then(|value| value.port))
            .or(flat.smtp_port)
            .unwrap_or(if smtp_tls { 587 } else { 25 });
        let smtp_username = nested_smtp
            .as_ref()
            .and_then(|value| value.username.as_ref())
            .or_else(|| {
                inline_smtp
                    .as_ref()
                    .and_then(|value| value.username.as_ref())
            })
            .or(flat.smtp_username.as_ref())
            .cloned()
            .unwrap_or_else(|| username.clone());
        let smtp_password = nested_smtp
            .as_ref()
            .and_then(|value| value.password.as_ref())
            .or_else(|| {
                inline_smtp
                    .as_ref()
                    .and_then(|value| value.password.as_ref())
            })
            .or(flat.smtp_password.as_ref())
            .cloned()
            .or_else(|| crate::utils::environment::debug_env("SATCHEL_SMTP_PASSWORD"))
            .unwrap_or_else(|| password.clone());
        let smtp_from_address = nested_smtp
            .as_ref()
            .and_then(|value| value.from_address.as_ref().or(value.from.as_ref()))
            .or_else(|| {
                inline_smtp
                    .as_ref()
                    .and_then(|value| value.from_address.as_ref().or(value.from.as_ref()))
            })
            .or(flat.smtp_from_address.as_ref().or(flat.smtp_from.as_ref()))
            .cloned()
            .unwrap_or_else(|| fallback_username.to_string());

        Ok(Self {
            config: Some(ImapRuntimeConfig {
                host: flat.host,
                port: flat.port.unwrap_or(993),
                tls: flat.tls.unwrap_or(true),
                username,
                password,
                smtp: SmtpRuntimeConfig {
                    host: smtp_host,
                    port: smtp_port,
                    tls: smtp_tls,
                    username: smtp_username,
                    password: smtp_password,
                    from_address: smtp_from_address,
                },
            }),
            body_window_days: configured_body_window_days(config_json),
        })
    }

    pub async fn test_smtp_connection(&self) -> Result<(), String> {
        let smtp = &self
            .config
            .as_ref()
            .ok_or_else(|| "IMAP provider config missing".to_string())?
            .smtp;
        if !smtp.tls {
            return Err("SMTP connection test requires TLS".to_string());
        }
        let transport = super::smtp::smtp_transport(smtp, &smtp.from_address)?;

        let connected = timeout(Self::op_timeout(), transport.test_connection())
            .await
            .map_err(|_| "smtp connection test timed out".to_string())?
            .map_err(|err| format!("smtp connection test failed: {err}"))?;
        if connected {
            Ok(())
        } else {
            Err("SMTP server rejected the connection test".to_string())
        }
    }

    async fn login_session(&self) -> Result<async_imap::Session<TlsStream<TcpStream>>, String> {
        let cfg = self
            .config
            .as_ref()
            .ok_or_else(|| "IMAP provider config missing".to_string())?;

        if !cfg.tls {
            return Err("IMAP MVP currently requires tls=true".into());
        }

        let tcp_stream = timeout(
            Self::op_timeout(),
            TcpStream::connect((cfg.host.as_str(), cfg.port)),
        )
        .await
        .map_err(|_| "imap tcp connect timed out".to_string())?
        .map_err(|err| format!("imap tcp connect failed: {err}"))?;

        crate::utils::network::enable_mail_keepalive(&tcp_stream)
            .map_err(|error| format!("imap keepalive setup failed: {error}"))?;
        let tls = TlsConnector::from(Arc::new(mail_tls_client_config()?));
        let server_name = rustls_pki_types::ServerName::try_from(cfg.host.clone())
            .map_err(|err| format!("invalid IMAP TLS server name: {err}"))?;
        let client = if cfg.port == 143 {
            let mut plain_client = async_imap::Client::new(tcp_stream);
            let _ = timeout(Self::op_timeout(), plain_client.read_response())
                .await
                .map_err(|_| "imap greeting read timed out".to_string())?
                .map_err(|err| format!("imap greeting read failed: {err}"))?;
            timeout(
                Self::op_timeout(),
                plain_client.run_command_and_check_ok("STARTTLS", None),
            )
            .await
            .map_err(|_| "imap STARTTLS timed out".to_string())?
            .map_err(|err| format!("imap STARTTLS failed: {err}"))?;
            let tls_stream = timeout(
                Self::op_timeout(),
                tls.connect(server_name.clone(), plain_client.into_inner()),
            )
            .await
            .map_err(|_| "imap tls handshake timed out".to_string())?
            .map_err(|err| format!("imap tls handshake failed: {err}"))?;
            async_imap::Client::new(tls_stream)
        } else {
            let tls_stream = timeout(Self::op_timeout(), tls.connect(server_name, tcp_stream))
                .await
                .map_err(|_| "imap tls handshake timed out".to_string())?
                .map_err(|err| format!("imap tls handshake failed: {err}"))?;
            let mut client = async_imap::Client::new(tls_stream);
            let _ = timeout(Self::op_timeout(), client.read_response())
                .await
                .map_err(|_| "imap greeting read timed out".to_string())?
                .map_err(|err| format!("imap greeting read failed: {err}"))?;
            client
        };

        let session = timeout(
            Self::op_timeout(),
            client.login(cfg.username.as_str(), cfg.password.as_str()),
        )
        .await
        .map_err(|_| "imap login timed out".to_string())?
        .map_err(|(err, _)| login_error_text(&err))?;

        Ok(session)
    }

    fn parse_cursor_state(since: &SyncCheckpoint) -> ParsedCursorState {
        cursor::parse_state(since.cursor.as_deref(), |raw| {
            let mut state = ImapCursorState::default();
            if let Some(value) = raw
                .strip_prefix("uid:")
                .and_then(|value| value.parse::<u32>().ok())
            {
                state.high_uid = value;
            } else if let Some(value) = raw
                .strip_prefix("seq:")
                .and_then(|value| value.parse::<u32>().ok())
            {
                state.high_seq = value;
            }
            state
        })
    }

    fn message_refs_by_folder(
        messages: &[MessageRef],
    ) -> SyncResult<std::collections::HashMap<String, UidMutationGroup>> {
        let mut grouped = std::collections::HashMap::<String, UidMutationGroup>::new();
        for message in messages {
            let folder = message
                .source_folder
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| permanent_imap_error("backend.errors.imapMutationInvalidRef", ""))?;
            let uid = message
                .remote_id
                .trim()
                .parse::<u32>()
                .ok()
                .filter(|value| *value > 0)
                .ok_or_else(|| {
                    permanent_imap_error("backend.errors.imapMutationInvalidRef", folder)
                })?;
            let expected_uidvalidity = message
                .source_uidvalidity
                .filter(|value| *value != 0)
                .ok_or_else(|| {
                    permanent_imap_error("backend.errors.imapUidValidityMissing", folder)
                })?;

            let entry = grouped
                .entry(folder.to_string())
                .or_insert_with(|| UidMutationGroup {
                    expected_uidvalidity,
                    uids: Vec::new(),
                });
            if entry.expected_uidvalidity != expected_uidvalidity {
                return Err(permanent_imap_error(
                    "backend.errors.imapMixedUidValidity",
                    folder,
                ));
            }
            if !entry.uids.contains(&uid) {
                entry.uids.push(uid);
            }
        }
        Ok(grouped)
    }

    fn uid_set(uids: &[u32]) -> String {
        uids.iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(",")
    }

    async fn apply_uid_store(
        session: &mut async_imap::Session<TlsStream<TcpStream>>,
        uid_set: &str,
        query: &str,
    ) -> Result<(), String> {
        collect_imap_response(
            Self::op_timeout(),
            session.uid_store(uid_set, query),
            "UID STORE",
        )
        .await?;
        Ok(())
    }

    /// Remove exactly these UIDs from the selected mailbox.
    ///
    /// A server without UIDPLUS rejects `UID EXPUNGE`, and the plain `EXPUNGE`
    /// that remains removes *every* `\Deleted` message of the mailbox — Alpine
    /// and mutt mark mail that way and never expunge it. The fallback therefore
    /// runs only once `UID SEARCH DELETED` has shown that nothing but our own
    /// UIDs would go.
    async fn expunge_uids(
        session: &mut async_imap::Session<TlsStream<TcpStream>>,
        uids: &[u32],
    ) -> Result<(), String> {
        if uids.is_empty() {
            return Ok(());
        }

        match timeout(Self::op_timeout(), async {
            session
                .uid_expunge(Self::uid_set(uids))
                .await?
                .try_collect::<Vec<_>>()
                .await
        })
        .await
        {
            Ok(Ok(_)) => return Ok(()),
            Ok(Err(err)) if !is_server_rejection(&err) => {
                return Err(format!("imap UID EXPUNGE failed: {err}"));
            }
            Ok(Err(_)) => {}
            Err(_) => return Err("imap UID EXPUNGE timed out".to_string()),
        }

        Self::ensure_plain_expunge_is_safe(session, uids).await?;
        collect_imap_response(Self::op_timeout(), session.expunge(), "EXPUNGE").await?;

        Ok(())
    }

    /// Refuse a plain `EXPUNGE` while the mailbox holds a `\Deleted` message
    /// that is not ours. The flag we set stays, and the queued retry tries
    /// again once the other client has cleaned up.
    async fn ensure_plain_expunge_is_safe(
        session: &mut async_imap::Session<TlsStream<TcpStream>>,
        uids: &[u32],
    ) -> Result<(), String> {
        let deleted = timeout(Self::op_timeout(), session.uid_search("DELETED"))
            .await
            .map_err(|_| "imap UID SEARCH DELETED timed out".to_string())?
            .map_err(|err| format!("imap UID SEARCH DELETED failed: {err}"))?;
        if plain_expunge_is_safe(&deleted, uids) {
            return Ok(());
        }
        Err("imap EXPUNGE unsafe: mailbox holds foreign \\Deleted messages".to_string())
    }

    /// Settle before the `UID COPY` whether the source copies can be removed
    /// afterwards.
    ///
    /// A move either leaves the target untouched or completes. Copying first
    /// and discovering only then that the source cannot be expunged leaves a
    /// copy behind that the queued retry would duplicate.
    async fn ensure_expunge_is_possible(
        session: &mut async_imap::Session<TlsStream<TcpStream>>,
        uids: &[u32],
    ) -> Result<(), String> {
        let capabilities = timeout(Self::op_timeout(), session.capabilities())
            .await
            .map_err(|_| "imap CAPABILITY timed out".to_string())?
            .map_err(|err| format!("imap CAPABILITY failed: {err}"))?;
        if capabilities.has_str("UIDPLUS") {
            return Ok(());
        }
        Self::ensure_plain_expunge_is_safe(session, uids).await
    }
}

/// Whether the server declined a command it understood.
///
/// Only a `NO`/`BAD` answer leaves the session in a known state and says
/// something about the command. Every other error is the exchange itself
/// breaking, and a fallback would run blind on a connection whose state is
/// unknown.
fn is_server_rejection(error: &async_imap::error::Error) -> bool {
    matches!(
        error,
        async_imap::error::Error::No(_) | async_imap::error::Error::Bad(_)
    )
}

/// Whether a plain `EXPUNGE` would remove nothing but the given UIDs.
fn plain_expunge_is_safe(deleted: &std::collections::HashSet<u32>, uids: &[u32]) -> bool {
    deleted.iter().all(|uid| uids.contains(uid))
}

/// What the incremental fetch asks for, per message.
const MESSAGE_FETCH_ITEMS: &str = "(UID FLAGS INTERNALDATE BODY.PEEK[HEADER] BODY.PEEK[TEXT])";

/// What the listing ahead of it asks for: enough to plan, nothing to parse.
const MESSAGE_LISTING_ITEMS: &str = "(UID RFC822.SIZE)";

/// One new message as the listing reported it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ListedMessage {
    uid: u32,
    seq: u32,
    size: u64,
}

/// How much one command and one run may download.
///
/// A chunk is one `UID FETCH` and has to finish inside its own deadline; the
/// run is what one `sync_messages` call holds in memory and what the account
/// lock waits for. A message above the chunk budget still goes, alone.
#[derive(Debug, Clone, Copy)]
struct FetchLimits {
    chunk_messages: usize,
    chunk_bytes: u64,
    run_messages: usize,
    run_bytes: u64,
}

const FETCH_LIMITS: FetchLimits = FetchLimits {
    chunk_messages: 50,
    chunk_bytes: 4 * 1024 * 1024,
    run_messages: 500,
    run_bytes: 48 * 1024 * 1024,
};

/// The slowest link a chunk deadline still accepts, in bytes per second.
///
/// One deadline for a whole response failed every mail above roughly what the
/// line moves in 20 seconds — and failed it again on every retry, because the
/// cursor never got past it. The deadline grows with what the chunk carries.
const MIN_FETCH_THROUGHPUT: u64 = 64 * 1024;

/// The chunks one run fetches, and how many listed messages wait for the next.
#[derive(Debug, Default, PartialEq, Eq)]
struct FetchPlan {
    chunks: Vec<Vec<ListedMessage>>,
    deferred: usize,
}

/// Keep the listing's new messages, ascending by UID.
///
/// `n:*` always names the highest message, even when `n` is past it (RFC 3501
/// §6.4.8), so the cursor filter still applies. Entries without a UID are a
/// protocol violation and only counted.
fn listed_messages(
    entries: impl IntoIterator<Item = (Option<u32>, u32, Option<u32>)>,
    since_uid: u32,
    since_seq: u32,
) -> (Vec<ListedMessage>, usize) {
    let mut uidless = 0usize;
    let mut listed = Vec::new();
    for (uid, seq, size) in entries {
        let Some(uid) = uid.filter(|uid| *uid > 0) else {
            uidless += 1;
            continue;
        };
        let new = if since_uid > 0 {
            uid > since_uid
        } else if since_seq > 0 {
            seq > since_seq
        } else {
            true
        };
        if new {
            listed.push(ListedMessage {
                uid,
                seq,
                size: u64::from(size.unwrap_or(0)),
            });
        }
    }
    listed.sort_by_key(|message| message.uid);
    listed.dedup_by_key(|message| message.uid);
    (listed, uidless)
}

/// Cut the listed messages into chunks, oldest first, until the run is full.
///
/// Order matters: the cursor may only move past a UID once everything below it
/// has arrived, so the chunks run in ascending UID order and the run stops at
/// a chunk boundary. The first chunk is always taken, however large.
fn plan_incremental_fetch(listed: &[ListedMessage], limits: FetchLimits) -> FetchPlan {
    let mut plan = FetchPlan::default();
    let mut current = Vec::new();
    let mut current_bytes = 0u64;
    let mut run_bytes = 0u64;
    // `index` is also the number of messages the run has taken so far.
    for (index, message) in listed.iter().enumerate() {
        let run_full = index > 0
            && (index >= limits.run_messages
                || run_bytes.saturating_add(message.size) > limits.run_bytes);
        if run_full {
            plan.deferred = listed.len() - index;
            break;
        }
        let chunk_full = !current.is_empty()
            && (current.len() >= limits.chunk_messages
                || current_bytes.saturating_add(message.size) > limits.chunk_bytes);
        if chunk_full {
            plan.chunks.push(std::mem::take(&mut current));
            current_bytes = 0;
        }
        current.push(*message);
        current_bytes = current_bytes.saturating_add(message.size);
        run_bytes = run_bytes.saturating_add(message.size);
    }
    if !current.is_empty() {
        plan.chunks.push(current);
    }
    plan
}

/// The deadline for one chunk: the usual one, plus the time its bytes need on
/// a slow line.
fn chunk_timeout(base: Duration, chunk: &[ListedMessage]) -> Duration {
    let bytes: u64 = chunk.iter().map(|message| message.size).sum();
    base + Duration::from_secs(bytes / MIN_FETCH_THROUGHPUT)
}

/// Where the cursor stands once the first `completed` chunks have arrived.
///
/// Returns `(high_uid, high_seq, pending)`. Everything at or below `high_uid`
/// has been fetched — a message the server expunged between listing and
/// fetching is gone for good and needs no second look. `pending` is what the
/// next run still has to fetch.
fn advance_after_chunks(
    since_uid: u32,
    since_seq: u32,
    plan: &FetchPlan,
    completed: usize,
) -> (u32, u32, u32) {
    let done = &plan.chunks[..completed.min(plan.chunks.len())];
    let high_uid = done
        .iter()
        .flatten()
        .map(|message| message.uid)
        .fold(since_uid, u32::max);
    let high_seq = done
        .iter()
        .flatten()
        .map(|message| message.seq)
        .fold(since_seq, u32::max);
    let pending = plan.chunks[done.len()..]
        .iter()
        .map(Vec::len)
        .sum::<usize>()
        + plan.deferred;
    (
        high_uid,
        high_seq,
        u32::try_from(pending).unwrap_or(u32::MAX),
    )
}

/// What the chunked fetch brought back.
#[derive(Debug, Default)]
struct ChunkedFetch {
    entries: Vec<async_imap::types::Fetch>,
    /// Chunks that arrived in full, counted from the first.
    completed: usize,
    /// Why the run stopped early. The session is then in an unknown state.
    interrupted: Option<String>,
}

/// Fetch the planned chunks one `UID FETCH` at a time.
///
/// A chunk that breaks off ends the run but keeps what came before it: the
/// caller commits those messages and moves the cursor to their end, so the
/// next run picks up at the broken chunk instead of downloading everything
/// again. Only a failure of the very first chunk is an error.
async fn fetch_planned_chunks<T>(
    session: &mut async_imap::Session<T>,
    plan: &FetchPlan,
    base_timeout: Duration,
) -> Result<ChunkedFetch, String>
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + std::fmt::Debug + Send,
{
    let mut fetched = ChunkedFetch::default();
    for chunk in &plan.chunks {
        let wanted: std::collections::HashSet<u32> =
            chunk.iter().map(|message| message.uid).collect();
        let uid_set = chunk
            .iter()
            .map(|message| message.uid.to_string())
            .collect::<Vec<_>>()
            .join(",");
        match collect_imap_response(
            chunk_timeout(base_timeout, chunk),
            session.uid_fetch(uid_set, MESSAGE_FETCH_ITEMS),
            "UID FETCH",
        )
        .await
        {
            Ok(entries) => {
                // An unsolicited FETCH for another message (a flag change)
                // carries no body and must not land as an empty message.
                fetched.entries.extend(
                    entries
                        .into_iter()
                        .filter(|entry| entry.uid.is_none_or(|uid| wanted.contains(&uid))),
                );
                fetched.completed += 1;
            }
            Err(error) if fetched.completed == 0 => return Err(error),
            Err(error) => {
                fetched.interrupted = Some(error);
                break;
            }
        }
    }
    Ok(fetched)
}

/// What every envelope of one fetch shares.
#[derive(Clone)]
struct EnvelopeContext {
    folder: String,
    uidvalidity: u32,
    source_id: SourceId,
    ingested_at: chrono::DateTime<chrono::Utc>,
    receiver: String,
}

/// Parse a batch of fetched messages into envelopes off the async runtime.
///
/// MIME parsing a backfill batch of a thousand messages, or one message with
/// a 25 MB attachment, is CPU work measured in seconds; on a runtime worker it
/// stalls every IPC call scheduled behind it. `bodies` replaces each entry's
/// own `BODY[TEXT]` when the bodies came from a separate fetch. Returns the
/// envelopes and the number of entries discarded for lacking a UID.
async fn envelopes_off_runtime(
    entries: Vec<async_imap::types::Fetch>,
    bodies: Option<std::collections::HashMap<u32, Vec<u8>>>,
    context: EnvelopeContext,
) -> Result<(Vec<MessageEnvelope>, usize), String> {
    if entries.is_empty() {
        return Ok((Vec::new(), 0));
    }
    tokio::task::spawn_blocking(move || {
        let mut uidless = 0usize;
        let mut envelopes = Vec::with_capacity(entries.len());
        for entry in &entries {
            // The FETCH asked for the UID. Answering without one is a protocol
            // violation, and the sequence number this used to fall back to
            // ends up in `messages.remote_id` — from where a later UID MOVE
            // or UID STORE addresses a different message entirely.
            let Some(uid) = entry.uid.filter(|uid| *uid > 0) else {
                uidless += 1;
                continue;
            };
            let body = match &bodies {
                Some(bodies) => bodies.get(&uid).map(Vec::as_slice),
                None => entry.text(),
            };
            envelopes.push(envelope_from_fetch(entry, body, uid, &context));
        }
        (envelopes, uidless)
    })
    .await
    .map_err(|error| format!("imap message parsing failed: {error}"))
}

#[async_trait::async_trait]
impl MailProvider for ImapProvider {
    async fn sync_folders(&self) -> crate::sync::errors::SyncResult<Vec<RemoteFolder>> {
        let mut session = self.login_session().await?;
        let names =
            collect_imap_response(Self::op_timeout(), session.list(None, Some("*")), "LIST")
                .await?;

        let mut folders = names
            .into_iter()
            .map(|name| {
                let kind = name
                    .attributes()
                    .iter()
                    .find_map(|attribute| match attribute {
                        async_imap::types::NameAttribute::Archive => Some("archive"),
                        async_imap::types::NameAttribute::Drafts => Some("drafts"),
                        async_imap::types::NameAttribute::Junk => Some("spam"),
                        async_imap::types::NameAttribute::Sent => Some("sent"),
                        async_imap::types::NameAttribute::Trash => Some("trash"),
                        _ => None,
                    });
                RemoteFolder {
                    id: name.name().to_string(),
                    name: name.name().to_string(),
                    kind: kind.map(str::to_string),
                }
            })
            .collect::<Vec<_>>();

        if folders.is_empty() {
            folders.push(RemoteFolder {
                id: "INBOX".into(),
                name: "INBOX".into(),
                kind: Some("inbox".into()),
            });
        }

        if !Self::sync_all_folders() {
            let aside = folders
                .iter()
                .find(|folder| crate::workflow::states::is_aside_folder(&folder.name))
                .cloned();
            if let Some(inbox) = folders
                .iter()
                .find(|folder| folder.name.eq_ignore_ascii_case("INBOX"))
                .cloned()
            {
                folders = vec![inbox];
                if let Some(aside) = aside {
                    folders.push(aside);
                }
            } else {
                folders = folders.into_iter().take(1).collect();
            }
        }

        let _ = timeout(Self::op_timeout(), session.logout()).await;
        Ok(folders)
    }

    async fn sync_messages(
        &self,
        folder: &str,
        since: SyncCheckpoint,
    ) -> crate::sync::errors::SyncResult<SyncBatch> {
        let cfg = self
            .config
            .as_ref()
            .ok_or_else(|| "IMAP provider config missing".to_string())?;
        let core_source_id = SourceId::new(format!("imap/{}/{}", cfg.host, cfg.username));
        let mut session = self.login_session().await?;

        let mailbox = timeout(Self::op_timeout(), session.select(folder))
            .await
            .map_err(|_| format!("imap SELECT {folder} timed out"))?
            .map_err(|err| format!("imap SELECT {folder} failed: {err}"))?;

        let exists = mailbox.exists;
        let parsed_cursor = Self::parse_cursor_state(&since);
        let mut cursor_state = parsed_cursor.state;
        // RFC 3501 requires UIDVALIDITY in the SELECT response. Reading a
        // missing one as 0 turned a generation change into a silent no-op:
        // the folder kept a UID anchor from a generation that no longer
        // exists and never saw another message.
        let server_uidvalidity = mailbox
            .uid_validity
            .filter(|value| *value != 0)
            .ok_or_else(|| {
                format!("imap protocol violation: SELECT {folder} reported no UIDVALIDITY")
            })?;
        let db_uidvalidity =
            (cursor_state.db_uidvalidity != 0).then_some(cursor_state.db_uidvalidity);
        // Inbound only. The caller supplies it fresh from the database on
        // every run; carrying it back out would freeze one run's guess.
        cursor_state.db_uidvalidity = 0;
        let mut has_cursor = parsed_cursor.had_checkpoint;
        let decision =
            imap_generation_decision(cursor_state.uidvalidity, db_uidvalidity, server_uidvalidity);
        let reset_folder = has_cursor && decision == GenerationDecision::Reset;
        if reset_folder {
            // A UID is only meaningful inside its UIDVALIDITY generation. The
            // manager clears the folder before this fresh snapshot is applied.
            cursor_state = ImapCursorState::default();
            has_cursor = false;
        }
        if exists == 0 {
            // Nothing left to list, so nothing the last run deferred either.
            cursor_state.incremental_pending = 0;
            let cursor = if reset_folder {
                cursor_state.format_version = Self::CURSOR_FORMAT_VERSION;
                cursor_state.uidvalidity = server_uidvalidity;
                serde_json::to_string(&cursor_state).ok()
            } else if has_cursor && !parsed_cursor.was_legacy {
                // Same cursor, minus the database hint that only travels
                // inwards, plus the generation an empty folder would otherwise
                // never get to record.
                cursor_state.uidvalidity = server_uidvalidity;
                serde_json::to_string(&cursor_state).ok().or(since.cursor)
            } else {
                since.cursor
            };
            let _ = timeout(Self::op_timeout(), session.logout()).await;
            return Ok(SyncBatch {
                messages: vec![],
                removals: Vec::new(),
                remote_uidvalidity: Some(server_uidvalidity),
                reset_folder,
                snapshot_required: true,
                next: SyncCheckpoint { cursor },
            });
        }

        let fetch_window = std::env::var("SATCHEL_IMAP_FETCH_WINDOW")
            .ok()
            .and_then(|raw| raw.parse::<u32>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(200);
        let initial_window = Self::initial_window();
        if cursor_state.backfill_next_seq > 0
            && cursor_state.backfill_total_seq < cursor_state.backfill_next_seq
        {
            cursor_state.backfill_total_seq = cursor_state.backfill_next_seq;
        }
        // Without a generation on either side the anchors could belong to a
        // mailbox that no longer exists. The window plus the backfill carry
        // this one run; the cursor records the generation for the next.
        let anchored = decision != GenerationDecision::WindowOnly;
        let since_uid = if anchored { cursor_state.high_uid } else { 0 };
        let since_seq = if anchored { cursor_state.high_seq } else { 0 };
        let needs_envelope_repair =
            has_cursor && cursor_state.format_version < Self::CURSOR_FORMAT_VERSION;

        if needs_envelope_repair && cursor_state.backfill_next_seq == 0 {
            // One-time repair pass: rewrite legacy rows with complete envelope
            // headers so reply threading can recover existing conversations.
            cursor_state.backfill_next_seq = exists;
            cursor_state.backfill_total_seq = exists;
        }

        let incremental_window = if has_cursor {
            fetch_window
        } else {
            initial_window
        };
        let tail_start = if exists > incremental_window {
            exists - incremental_window + 1
        } else {
            1
        };

        // Incremental sync with a known UID cursor uses UID FETCH so that *every*
        // message newer than the cursor is fetched, no matter how many arrived since
        // the last run. A fixed seq-number tail window would silently skip mail once
        // more than `fetch_window` messages arrive between syncs (the cursor would
        // jump past them for good). Legacy seq-only cursors and fresh folders keep
        // the bounded window; older history flows in via the backfill below.
        //
        // The first FETCH lists only UIDs and sizes. The bodies follow in
        // bounded chunks, oldest first, and the cursor moves only as far as the
        // chunks that arrived: a run that fills its budget or loses a chunk
        // leaves the rest to the next run, instead of failing the folder and
        // starting the same download over on every retry.
        let use_uid_fetch = has_cursor && since_uid > 0;
        let listing = if use_uid_fetch {
            let uid_set = format!("{}:*", since_uid.saturating_add(1));
            collect_imap_response(
                Self::op_timeout(),
                session.uid_fetch(uid_set, MESSAGE_LISTING_ITEMS),
                "UID FETCH",
            )
            .await?
        } else {
            let seq_set = if has_cursor {
                let start_seq = if exists > fetch_window {
                    exists - fetch_window + 1
                } else {
                    1
                };
                format!("{start_seq}:*")
            } else {
                format!("{tail_start}:*")
            };
            collect_imap_response(
                Self::op_timeout(),
                session.fetch(seq_set, MESSAGE_LISTING_ITEMS),
                "FETCH",
            )
            .await?
        };
        let (listed, mut uidless_entries) = listed_messages(
            listing
                .iter()
                .map(|entry| (entry.uid, entry.message, entry.size)),
            since_uid,
            since_seq,
        );
        let plan = plan_incremental_fetch(&listed, FETCH_LIMITS);
        let fetched = fetch_planned_chunks(&mut session, &plan, Self::op_timeout()).await?;
        let (high_uid, high_seq, incremental_pending) =
            advance_after_chunks(since_uid, since_seq, &plan, fetched.completed);
        // A broken-off chunk leaves the connection mid-response. The backfill
        // and the LOGOUT would only wait out their own deadlines on it.
        let session_usable = fetched.interrupted.is_none();
        if let Some(error) = fetched.interrupted.as_deref() {
            tracing::warn!(
                folder,
                %error,
                committed_chunks = fetched.completed,
                pending = incremental_pending,
                "imap incremental fetch broke off, keeping the chunks that arrived"
            );
        }

        let mut entries = fetched.entries;
        entries.sort_by_key(|entry| entry.uid.unwrap_or(entry.message));
        // One reference point for the whole batch: two messages of the same
        // run must not be clamped against two different clocks.
        let envelope_context = EnvelopeContext {
            folder: folder.to_string(),
            uidvalidity: server_uidvalidity,
            source_id: core_source_id.clone(),
            ingested_at: chrono::Utc::now(),
            receiver: cfg.host.clone(),
        };
        let (mut messages, uidless) =
            envelopes_off_runtime(entries, None, envelope_context.clone()).await?;
        uidless_entries += uidless;

        cursor_state.high_uid = high_uid;
        cursor_state.high_seq = high_seq;
        cursor_state.incremental_pending = incremental_pending;

        if let Some(resume) =
            backfill_resume_seq(since_uid, cursor_state.backfill_next_seq, tail_start)
        {
            cursor_state.backfill_next_seq = resume;
            cursor_state.backfill_total_seq = resume;
        }

        // A run whose incremental fetch broke off leaves the backfill where it
        // is; the next run continues both.
        if cursor_state.backfill_next_seq > 0 && session_usable {
            let backfill_end = cursor_state.backfill_next_seq.min(exists);
            if backfill_end > 0 {
                let backfill_window = Self::backfill_window(backfill_end);
                let backfill_start = if backfill_end > backfill_window {
                    backfill_end - backfill_window + 1
                } else {
                    1
                };
                let backfill_set = format!("{backfill_start}:{backfill_end}");
                // Headers alone when history is bounded. The bodies for the
                // part of this batch that is still inside the window follow in
                // a second fetch below; once the walk is past the cutoff that
                // second fetch asks for nothing and stops happening.
                let body_cutoff = self.body_cutoff();
                let backfill_items = if body_cutoff.is_some() {
                    "(UID FLAGS INTERNALDATE BODY.PEEK[HEADER])"
                } else {
                    "(UID FLAGS INTERNALDATE BODY.PEEK[HEADER] BODY.PEEK[TEXT])"
                };
                let mut older = collect_imap_response(
                    Self::op_timeout(),
                    session.fetch(backfill_set, backfill_items),
                    "BACKFILL",
                )
                .await?;
                older.sort_by_key(|entry| entry.uid.unwrap_or(entry.message));

                // Outside the window there is no body to parse, and the
                // message is stored as the envelope it already is. Opening it
                // fetches the body then — see `load_message_body`.
                let recent_bodies = match body_cutoff {
                    Some(cutoff) => Some(
                        Self::fetch_recent_backfill_bodies(&mut session, &older, cutoff).await?,
                    ),
                    None => None,
                };

                let (older_messages, uidless) =
                    envelopes_off_runtime(older, recent_bodies, envelope_context).await?;
                uidless_entries += uidless;
                messages.extend(older_messages);

                if backfill_start <= 1 {
                    cursor_state.backfill_next_seq = 0;
                } else {
                    cursor_state.backfill_next_seq = backfill_start - 1;
                }
            } else {
                cursor_state.backfill_next_seq = 0;
            }
        }

        cursor_state.format_version = Self::CURSOR_FORMAT_VERSION;
        cursor_state.uidvalidity = server_uidvalidity;

        let next_cursor = if cursor_state.high_uid == 0 && cursor_state.high_seq == 0 {
            since.cursor.unwrap_or_else(|| "seq:0".to_string())
        } else {
            serde_json::to_string(&cursor_state).unwrap_or_else(|_| {
                if cursor_state.high_uid > 0 {
                    format!("uid:{}", cursor_state.high_uid)
                } else {
                    format!("seq:{}", cursor_state.high_seq)
                }
            })
        };

        if uidless_entries > 0 {
            tracing::warn!(
                folder,
                discarded = uidless_entries,
                "imap protocol violation: FETCH answered without a UID, entries discarded"
            );
        }

        if session_usable {
            let _ = timeout(Self::op_timeout(), session.logout()).await;
        }
        Ok(SyncBatch {
            messages,
            removals: Vec::new(),
            remote_uidvalidity: Some(server_uidvalidity),
            reset_folder,
            snapshot_required: true,
            next: SyncCheckpoint {
                cursor: Some(next_cursor),
            },
        })
    }

    async fn fetch_messages_by_remote_ids(
        &self,
        folder: &str,
        remote_ids: &[String],
    ) -> crate::sync::errors::SyncResult<Option<SyncBatch>> {
        let uids: Vec<u32> = remote_ids
            .iter()
            .filter_map(|id| id.parse::<u32>().ok().filter(|uid| *uid > 0))
            .collect();
        if uids.len() != remote_ids.len() {
            return Err(format!("imap UID FETCH in {folder}: not a UID in {remote_ids:?}").into());
        }
        let cfg = self
            .config
            .as_ref()
            .ok_or_else(|| "IMAP provider config missing".to_string())?;
        let core_source_id = SourceId::new(format!("imap/{}/{}", cfg.host, cfg.username));
        let mut session = self.login_session().await?;
        let mailbox = timeout(Self::op_timeout(), session.select(folder))
            .await
            .map_err(|_| format!("imap SELECT {folder} timed out"))?
            .map_err(|err| format!("imap SELECT {folder} failed: {err}"))?;
        let server_uidvalidity = mailbox
            .uid_validity
            .filter(|value| *value != 0)
            .ok_or_else(|| {
                format!("imap protocol violation: SELECT {folder} reported no UIDVALIDITY")
            })?;
        let mut messages = Vec::new();
        if !uids.is_empty() {
            let uid_set = uids
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(",");
            let fetched = collect_imap_response(
                Self::op_timeout(),
                session.uid_fetch(uid_set, MESSAGE_FETCH_ITEMS),
                "UID FETCH",
            )
            .await?
            .into_iter()
            .filter(|entry| entry.uid.is_some_and(|uid| uids.contains(&uid)))
            .collect();
            (messages, _) = envelopes_off_runtime(
                fetched,
                None,
                EnvelopeContext {
                    folder: folder.to_string(),
                    uidvalidity: server_uidvalidity,
                    source_id: core_source_id,
                    ingested_at: chrono::Utc::now(),
                    receiver: cfg.host.clone(),
                },
            )
            .await?;
        }
        let _ = timeout(Self::op_timeout(), session.logout()).await;
        Ok(Some(SyncBatch {
            messages,
            removals: Vec::new(),
            remote_uidvalidity: Some(server_uidvalidity),
            reset_folder: false,
            snapshot_required: false,
            next: SyncCheckpoint { cursor: None },
        }))
    }

    async fn sync_message_state_snapshot(
        &self,
        folder: &str,
    ) -> crate::sync::errors::SyncResult<Option<crate::providers::traits::RemoteMessageStateSnapshot>>
    {
        let mut session = self.login_session().await?;
        let mailbox = timeout(Self::op_timeout(), session.select(folder))
            .await
            .map_err(|_| format!("imap SELECT {folder} timed out"))?
            .map_err(|err| format!("imap SELECT {folder} failed: {err}"))?;

        // Same protocol requirement as `sync_messages`: without the generation
        // the snapshot cannot say which mailbox its UIDs belong to, and the
        // reconciler would either delete the wrong rows or skip the run.
        let remote_uidvalidity = mailbox
            .uid_validity
            .filter(|value| *value != 0)
            .ok_or_else(|| {
                format!("imap protocol violation: SELECT {folder} reported no UIDVALIDITY")
            })?;

        if mailbox.exists == 0 {
            let _ = timeout(Self::op_timeout(), session.logout()).await;
            return Ok(Some(crate::providers::traits::RemoteMessageStateSnapshot {
                states: Vec::new(),
                remote_uidvalidity: Some(remote_uidvalidity),
            }));
        }

        let fetched = collect_imap_response(
            Self::op_timeout(),
            session.uid_fetch("1:*", "(UID FLAGS)"),
            "UID FETCH state snapshot",
        )
        .await?;

        let states = fetched
            .into_iter()
            .filter_map(|entry| {
                let uid = entry.uid?;
                Some(RemoteMessageState {
                    remote_id: uid.to_string(),
                    is_read: Some(
                        entry
                            .flags()
                            .any(|flag| matches!(flag, async_imap::types::Flag::Seen)),
                    ),
                    is_starred: Some(
                        entry
                            .flags()
                            .any(|flag| matches!(flag, async_imap::types::Flag::Flagged)),
                    ),
                })
            })
            .collect();

        let _ = timeout(Self::op_timeout(), session.logout()).await;
        Ok(Some(crate::providers::traits::RemoteMessageStateSnapshot {
            states,
            remote_uidvalidity: Some(remote_uidvalidity),
        }))
    }

    async fn send(&self, message: OutgoingMessage) -> crate::sync::errors::SyncResult<()> {
        self.send_returning_wire_copy(message).await.map(|_| ())
    }

    async fn send_returning_wire_copy(
        &self,
        message: OutgoingMessage,
    ) -> crate::sync::errors::SyncResult<Option<Vec<u8>>> {
        let cfg = self
            .config
            .as_ref()
            .ok_or_else(|| "IMAP provider config missing".to_string())?;
        Ok(Some(
            send_via_smtp_returning_wire_copy(&cfg.smtp, message).await?,
        ))
    }

    async fn set_flags(
        &self,
        _messages: &[MessageRef],
        _flags: FlagUpdate,
    ) -> crate::sync::errors::SyncResult<()> {
        let grouped = Self::message_refs_by_folder(_messages)?;
        if grouped.is_empty() {
            return Ok(());
        }

        let mut session = self.login_session().await?;
        for (folder, group) in grouped {
            let uid_set = Self::uid_set(&group.uids);
            let mailbox = timeout(Self::op_timeout(), session.select(folder.as_str()))
                .await
                .map_err(|_| format!("imap SELECT {folder} timed out"))?
                .map_err(|err| format!("imap SELECT {folder} failed: {err}"))?;
            group.validate_generation(&folder, mailbox.uid_validity)?;

            let mut add_flags = Vec::new();
            let mut remove_flags = Vec::new();
            if let Some(read) = _flags.read {
                if read {
                    add_flags.push("\\Seen");
                } else {
                    remove_flags.push("\\Seen");
                }
            }
            if let Some(starred) = _flags.starred {
                if starred {
                    add_flags.push("\\Flagged");
                } else {
                    remove_flags.push("\\Flagged");
                }
            }

            if !add_flags.is_empty() {
                Self::apply_uid_store(
                    &mut session,
                    &uid_set,
                    &format!("+FLAGS.SILENT ({})", add_flags.join(" ")),
                )
                .await?;
            }
            if !remove_flags.is_empty() {
                Self::apply_uid_store(
                    &mut session,
                    &uid_set,
                    &format!("-FLAGS.SILENT ({})", remove_flags.join(" ")),
                )
                .await?;
            }
        }

        let _ = timeout(Self::op_timeout(), session.logout()).await;
        Ok(())
    }

    async fn create_folder(&self, path: &str) -> crate::sync::errors::SyncResult<()> {
        let path = path.trim();
        if path.is_empty() {
            return Err("folder name is empty".into());
        }

        let mut session = self.login_session().await?;
        let mut created_path = path.to_string();
        let mut create_result = timeout(Self::op_timeout(), session.create(path))
            .await
            .map_err(|_| format!("imap CREATE {path} timed out"))?
            .map_err(|err| format!("imap CREATE {path} failed: {err}"));

        if create_result
            .as_ref()
            .is_err_and(|error| is_missing_namespace_error(error))
        {
            // A number of Dovecot configurations reject top-level mailboxes
            // and require them below INBOX. LIST tells us the server's actual
            // hierarchy delimiter, so this also works for INBOX/ descendants.
            let inbox = Self::inbox_namespace(&mut session).await;
            if let Some(fallback_path) = inbox.as_ref().and_then(|(name, delimiter)| {
                namespace_retry_folder(
                    create_result.as_ref().expect_err("namespace rejection"),
                    name,
                    delimiter.as_deref(),
                    path,
                )
            }) {
                created_path = fallback_path;
                create_result = timeout(Self::op_timeout(), session.create(created_path.as_str()))
                    .await
                    .map_err(|_| format!("imap CREATE {created_path} timed out"))?
                    .map_err(|err| format!("imap CREATE {created_path} failed: {err}"));
            }
        }
        if let Err(err) = create_result {
            let lower = err.to_ascii_lowercase();
            if !lower.contains("already exists") && !lower.contains("alreadyexists") {
                return Err(err.into());
            }
        }
        // Servers with the SUBSCRIBE extension hide unsubscribed folders from
        // other clients; subscribing keeps the new folder visible everywhere.
        let _ = timeout(Self::op_timeout(), session.subscribe(created_path.as_str())).await;
        let _ = timeout(Self::op_timeout(), session.logout()).await;
        Ok(())
    }

    async fn rename_folder(
        &self,
        from_path: &str,
        to_path: &str,
    ) -> crate::sync::errors::SyncResult<()> {
        let from_path = from_path.trim();
        let to_path = to_path.trim();
        if from_path.is_empty() || to_path.is_empty() {
            return Err("folder rename needs both a source and a target".into());
        }

        let mut session = self.login_session().await?;
        timeout(Self::op_timeout(), session.rename(from_path, to_path))
            .await
            .map_err(|_| format!("imap RENAME {from_path} timed out"))?
            .map_err(|err| format!("imap RENAME {from_path} -> {to_path} failed: {err}"))?;
        let _ = timeout(Self::op_timeout(), session.subscribe(to_path)).await;
        let _ = timeout(Self::op_timeout(), session.logout()).await;
        Ok(())
    }

    async fn delete_folder(&self, path: &str) -> crate::sync::errors::SyncResult<()> {
        let path = path.trim();
        if path.is_empty() {
            return Err("folder name is empty".into());
        }

        let mut session = self.login_session().await?;
        // Unsubscribe first: DELETE on a subscribed mailbox leaves a dangling
        // subscription on several servers (Dovecot among them).
        let _ = timeout(Self::op_timeout(), session.unsubscribe(path)).await;
        timeout(Self::op_timeout(), session.delete(path))
            .await
            .map_err(|_| format!("imap DELETE {path} timed out"))?
            .map_err(|err| format!("imap DELETE {path} failed: {err}"))?;
        let _ = timeout(Self::op_timeout(), session.logout()).await;
        Ok(())
    }

    async fn move_messages(
        &self,
        _messages: &[MessageRef],
        _target: &str,
    ) -> crate::sync::errors::SyncResult<()> {
        let target = _target.trim();
        if target.is_empty() {
            return Err("imap move target is empty".into());
        }

        let grouped = Self::message_refs_by_folder(_messages)?;
        if grouped.is_empty() {
            return Ok(());
        }

        let mut session = self.login_session().await?;
        for (folder, group) in grouped {
            let uid_set = Self::uid_set(&group.uids);
            let mailbox = timeout(Self::op_timeout(), session.select(folder.as_str()))
                .await
                .map_err(|_| format!("imap SELECT {folder} timed out"))?
                .map_err(|err| format!("imap SELECT {folder} failed: {err}"))?;
            group.validate_generation(&folder, mailbox.uid_validity)?;

            let move_result = timeout(Self::op_timeout(), session.uid_mv(&uid_set, target))
                .await
                .map_err(|_| "imap UID MOVE timed out".to_string())?;

            if let Err(error) = move_result {
                // Only a declined MOVE is a case for COPY. A broken exchange
                // may have moved part of the set already, and copying on top
                // of that duplicates mail in the target.
                if !is_server_rejection(&error) {
                    return Err(format!("imap UID MOVE failed: {error}").into());
                }
                Self::ensure_expunge_is_possible(&mut session, &group.uids).await?;
                timeout(Self::op_timeout(), session.uid_copy(&uid_set, target))
                    .await
                    .map_err(|_| "imap UID COPY timed out".to_string())?
                    .map_err(|err| format!("imap UID COPY failed: {err}"))?;
                Self::apply_uid_store(&mut session, &uid_set, "+FLAGS.SILENT (\\Deleted)").await?;
                Self::expunge_uids(&mut session, &group.uids).await?;
            }
        }

        let _ = timeout(Self::op_timeout(), session.logout()).await;
        Ok(())
    }

    async fn delete_messages(
        &self,
        _messages: &[MessageRef],
    ) -> crate::sync::errors::SyncResult<()> {
        let grouped = Self::message_refs_by_folder(_messages)?;
        if grouped.is_empty() {
            return Ok(());
        }

        let mut session = self.login_session().await?;
        for (folder, group) in grouped {
            let uid_set = Self::uid_set(&group.uids);
            let mailbox = timeout(Self::op_timeout(), session.select(folder.as_str()))
                .await
                .map_err(|_| format!("imap SELECT {folder} timed out"))?
                .map_err(|err| format!("imap SELECT {folder} failed: {err}"))?;
            group.validate_generation(&folder, mailbox.uid_validity)?;
            Self::apply_uid_store(&mut session, &uid_set, "+FLAGS.SILENT (\\Deleted)").await?;
            Self::expunge_uids(&mut session, &group.uids).await?;
        }

        let _ = timeout(Self::op_timeout(), session.logout()).await;
        Ok(())
    }

    async fn append_sent(
        &self,
        raw_rfc822: &[u8],
        folder_hint: Option<&str>,
    ) -> crate::sync::errors::SyncResult<()> {
        let mut session = self.login_session().await?;
        let folder = folder_hint.unwrap_or("Sent");
        // The literal has to cross the uplink inside the deadline. The flat
        // operation timeout cut large copies off on every attempt, and when
        // the server had stored the copy anyway, the retry filed it twice.
        let append_timeout =
            crate::utils::http::upload_timeout(Self::op_timeout(), raw_rfc822.len());
        let first = timeout(
            append_timeout,
            session.append(folder, Some("(\\Seen)"), None, raw_rfc822),
        )
        .await
        .map_err(|_| format!("imap APPEND {folder} timed out"))?
        .map_err(|err| format!("imap APPEND {folder} failed: {err}"));
        if let Err(err) = first {
            // RFC 3501 §6.3.11: TRYCREATE says the mailbox is missing and a
            // CREATE would help. Once, not in a loop — a server that keeps
            // answering TRYCREATE after a successful CREATE is not fixed by a
            // second round.
            if !is_trycreate_response(&err) {
                return Err(err.into());
            }
            timeout(Self::op_timeout(), session.create(folder))
                .await
                .map_err(|_| format!("imap CREATE {folder} timed out"))?
                .map_err(|create_err| format!("imap CREATE {folder} failed: {create_err}"))?;
            timeout(
                append_timeout,
                session.append(folder, Some("(\\Seen)"), None, raw_rfc822),
            )
            .await
            .map_err(|_| format!("imap APPEND {folder} timed out"))?
            .map_err(|err| format!("imap APPEND {folder} failed: {err}"))?;
        }
        let _ = timeout(Self::op_timeout(), session.logout()).await;
        Ok(())
    }

    async fn save_draft(
        &self,
        raw_rfc822: &[u8],
        folder_hint: Option<&str>,
    ) -> crate::sync::errors::SyncResult<Option<String>> {
        let mut session = self.login_session().await?;
        let folder =
            Self::select_draft_folder(&mut session, folder_hint.unwrap_or("Drafts")).await?;
        timeout(
            Self::op_timeout(),
            session.append(folder.as_str(), Some("(\\Draft)"), None, raw_rfc822),
        )
        .await
        .map_err(|_| format!("imap APPEND {folder} timed out"))?
        .map_err(|err| format!("imap APPEND {folder} failed: {err}"))?;
        let _ = timeout(Self::op_timeout(), session.logout()).await;
        Ok(None)
    }

    async fn upsert_draft(
        &self,
        message: OutgoingMessage,
        remote_draft_id: Option<&str>,
        folder_hint: Option<&str>,
    ) -> crate::sync::errors::SyncResult<Option<String>> {
        let decoded_remote = remote_draft_id.and_then(decode_imap_draft_ref);
        let folder = decoded_remote
            .as_ref()
            .map(|draft| draft.folder.as_str())
            .or(folder_hint)
            .unwrap_or("Drafts");
        let message_id = message
            .message_id_header
            .clone()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| "IMAP draft replacement requires a stable Message-ID".to_string())?;
        let raw = crate::email::build::build_rfc822(&message, None, None)?;
        let mut session = self.login_session().await?;
        let folder = Self::select_draft_folder(&mut session, folder).await?;

        let mut old_uids = std::collections::HashSet::<u32>::new();
        let previous_identifier = decoded_remote
            .as_ref()
            .map(|draft| draft.id.as_str())
            .or(remote_draft_id)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if let Some(identifier) = previous_identifier {
            // A legacy numeric draft ref has no UIDVALIDITY. It might now
            // identify another message, so only stable Message-ID refs may
            // select old drafts for deletion.
            if !is_numeric_imap_uid(identifier) {
                let escaped = identifier.replace('\\', "\\\\").replace('"', "\\\"");
                old_uids.extend(
                    timeout(
                        Self::op_timeout(),
                        session.uid_search(format!("HEADER Message-ID \"{escaped}\"")),
                    )
                    .await
                    .map_err(|_| "imap draft search timed out".to_string())?
                    .map_err(|err| format!("imap draft search failed: {err}"))?,
                );
            }
        }

        timeout(
            Self::op_timeout(),
            session.append(folder.as_str(), Some("(\\Draft)"), None, raw.as_bytes()),
        )
        .await
        .map_err(|_| format!("imap APPEND {folder} timed out"))?
        .map_err(|err| format!("imap APPEND {folder} failed: {err}"))?;
        let _ = timeout(Self::op_timeout(), session.noop()).await;

        let escaped = message_id.replace('\\', "\\\\").replace('"', "\\\"");
        let current_uids = timeout(
            Self::op_timeout(),
            session.uid_search(format!("HEADER Message-ID \"{escaped}\"")),
        )
        .await
        .map_err(|_| "imap replacement draft search timed out".to_string())?
        .map_err(|err| format!("imap replacement draft search failed: {err}"))?;
        let newest_uid = current_uids.iter().max().copied();
        old_uids.extend(current_uids);
        if let Some(newest_uid) = newest_uid {
            old_uids.remove(&newest_uid);
        }
        if !old_uids.is_empty() {
            let uids = old_uids.into_iter().collect::<Vec<_>>();
            Self::apply_uid_store(
                &mut session,
                &Self::uid_set(&uids),
                "+FLAGS.SILENT (\\Deleted)",
            )
            .await?;
            Self::expunge_uids(&mut session, &uids).await?;
        }
        let _ = timeout(Self::op_timeout(), session.logout()).await;
        Ok(Some(encode_imap_draft_ref(&folder, &message_id)))
    }

    async fn delete_draft(&self, remote_draft_id: &str) -> crate::sync::errors::SyncResult<()> {
        let decoded = decode_imap_draft_ref(remote_draft_id);
        let folder = decoded
            .as_ref()
            .map(|draft| draft.folder.as_str())
            .unwrap_or("Drafts");
        let identifier = decoded
            .as_ref()
            .map(|draft| draft.id.as_str())
            .unwrap_or(remote_draft_id)
            .trim();
        if is_numeric_imap_uid(identifier) {
            return Err(permanent_imap_error(
                "backend.errors.imapDraftUidWithoutValidity",
                folder,
            ));
        }
        let mut session = self.login_session().await?;
        let _folder = Self::select_draft_folder(&mut session, folder).await?;
        let escaped = identifier.replace('\\', "\\\\").replace('"', "\\\"");
        let uids = timeout(
            Self::op_timeout(),
            session.uid_search(format!("HEADER Message-ID \"{escaped}\"")),
        )
        .await
        .map_err(|_| "imap draft delete search timed out".to_string())?
        .map_err(|err| format!("imap draft delete search failed: {err}"))?
        .into_iter()
        .collect::<Vec<_>>();
        if !uids.is_empty() {
            Self::apply_uid_store(
                &mut session,
                &Self::uid_set(&uids),
                "+FLAGS.SILENT (\\Deleted)",
            )
            .await?;
            Self::expunge_uids(&mut session, &uids).await?;
        }
        let _ = timeout(Self::op_timeout(), session.logout()).await;
        Ok(())
    }

    async fn resolve_draft_id(
        &self,
        message_remote_id: &str,
        folder_hint: Option<&str>,
    ) -> crate::sync::errors::SyncResult<Option<String>> {
        if is_numeric_imap_uid(message_remote_id.trim()) {
            return Ok(None);
        }
        Ok(Some(encode_imap_draft_ref(
            folder_hint.unwrap_or("Drafts"),
            message_remote_id,
        )))
    }

    async fn fetch_attachment(
        &self,
        message: &MessageRef,
        remote_part_id: &str,
    ) -> crate::sync::errors::SyncResult<FetchedAttachment> {
        let folder = message.source_folder.as_deref().unwrap_or("INBOX");
        let key = self.attachment_cache_key(folder, message);
        let parts = Self::attachment_parts(&key, || {
            self.load_attachment_parts(folder, &message.remote_id)
        })
        .await?;

        Ok(parts
            .iter()
            .find(|attachment| attachment.remote_part_id == remote_part_id)
            .map(|attachment| FetchedAttachment {
                filename: attachment.filename.clone(),
                content_type: crate::email::mime_type::normalize_mime_type(
                    &attachment.content_type,
                ),
                bytes: attachment.bytes.clone(),
            })
            .ok_or_else(|| format!("imap attachment part `{remote_part_id}` not found"))?)
    }

    async fn idle(
        &self,
        folder: &str,
        on_event: IdleCallback,
    ) -> crate::sync::errors::SyncResult<()> {
        let mut reconnect = IdleReconnectBackoff::default();
        loop {
            let mut session = self.login_session().await?;
            timeout(Self::op_timeout(), session.select(folder))
                .await
                .map_err(|_| format!("imap SELECT {folder} timed out"))?
                .map_err(|err| format!("imap SELECT {folder} failed: {err}"))?;

            let capabilities = timeout(Self::op_timeout(), session.capabilities())
                .await
                .map_err(|_| "imap CAPABILITY timed out".to_string())?
                .map_err(|err| format!("imap CAPABILITY failed: {err}"))?;
            if !capabilities.has_str("IDLE") {
                return Err("imap IDLE unavailable: server does not advertise IDLE".into());
            }

            let mut handle = session.idle();
            timeout(Self::op_timeout(), handle.init())
                .await
                .map_err(|_| "imap IDLE init timed out".to_string())?
                .map_err(|error| format!("imap IDLE init failed: {error}"))?;

            let (idle_wait, interrupt) = handle.wait();
            // Refresh the IDLE every 25 minutes to stay under the 29-minute server timeout.
            let refresh = tokio::time::sleep(Duration::from_secs(25 * 60));
            tokio::pin!(refresh);
            let waiting_since = Instant::now();

            let _server_signalled = tokio::select! {
                result = idle_wait => result.is_ok(),
                _ = &mut refresh => {
                    drop(interrupt);
                    false
                }
            };

            // Always end the IDLE before issuing any other command.
            match timeout(Self::op_timeout(), handle.done()).await {
                Ok(Ok(_)) => {}
                Ok(Err(error)) => tracing::warn!(target: "idle", %error, "IDLE done failed"),
                Err(_) => tracing::warn!(target: "idle", "IDLE done timed out"),
            }

            // Fire the callback after every IDLE round, not only on a server event:
            // mail that arrived while the connection was down (laptop standby, Wi-Fi
            // change → idle_wait errors) would otherwise stay invisible until the
            // next server push. Syncs are incremental and serialized, so the extra
            // run every 25 minutes is cheap.
            (on_event)(IdleEvent::NewMessages);

            // Sleep and reconnect for the next IDLE round. A server or
            // middlebox that drops every IDLE at once used to get a fresh
            // login and an inbox sync twice a second; consecutive short rounds
            // now back off until a round lasts again.
            let delay = reconnect.after_round(waiting_since.elapsed());
            tokio::time::sleep(delay).await;
            if delay > IDLE_RECONNECT_BASE {
                // Mail delivered during the wait raises no IDLE event on
                // the next connection; only a sync sees it.
                (on_event)(IdleEvent::NewMessages);
            }
        }
    }
}

/// The pause before the next IDLE round when the last one was healthy.
const IDLE_RECONNECT_BASE: Duration = Duration::from_millis(500);

/// The longest pause between IDLE rounds that keep ending at once.
const IDLE_RECONNECT_MAX: Duration = Duration::from_secs(120);

/// An IDLE round that waited at least this long counts as healthy.
const IDLE_HEALTHY_ROUND: Duration = Duration::from_secs(30);

/// Spaces out IDLE reconnects while rounds keep ending right after they began.
///
/// The 25-minute refresh and a quiet mailbox make long rounds; a dropped
/// connection, an end of stream or an immediate `BYE` make short ones. Each
/// short round in a row doubles the pause, a healthy round resets it.
#[derive(Debug, Default)]
struct IdleReconnectBackoff {
    short_rounds: u32,
}

impl IdleReconnectBackoff {
    /// Record a finished round that waited `waited`; return the pause before
    /// the next one.
    fn after_round(&mut self, waited: Duration) -> Duration {
        if waited >= IDLE_HEALTHY_ROUND {
            self.short_rounds = 0;
        } else {
            self.short_rounds = self.short_rounds.saturating_add(1);
        }
        IDLE_RECONNECT_BASE
            .saturating_mul(1u32 << self.short_rounds.min(16))
            .min(IDLE_RECONNECT_MAX)
    }
}

/// Only a tagged NO/BAD is the server refusing the credentials; a dropped
/// connection mid-login must not read as a wrong password.
fn login_error_text(err: &async_imap::error::Error) -> String {
    match err {
        async_imap::error::Error::No(_) | async_imap::error::Error::Bad(_) => {
            format!("imap login failed: {err}")
        }
        _ => format!("imap login interrupted: {err}"),
    }
}

/// TLS settings shared by every mail transport (IMAP here, POP3 next door).
///
/// The crypto provider is named explicitly rather than left to `rustls`'
/// process-wide default: several dependencies install one, and whichever
/// happens to register first would otherwise decide our cipher suites — a
/// handshake failure that only appears once an unrelated crate is added.
///
/// Trust anchors differ by platform on purpose. Desktop pins the bundled
/// webpki roots; mobile defers to the OS store, because both platforms let the
/// user and an MDM profile install CAs, and mail servers behind a corporate
/// proxy depend on exactly that.
pub fn mail_tls_client_config() -> Result<rustls::ClientConfig, String> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|err| format!("failed to configure mail TLS protocol versions: {err}"))?;

    #[cfg(any(target_os = "android", target_os = "ios"))]
    let config = builder
        .with_platform_verifier()
        .map_err(|err| format!("failed to configure the mobile platform TLS verifier: {err}"))?
        .with_no_client_auth();

    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    let config = builder
        .with_root_certificates(rustls::RootCertStore::from_iter(
            webpki_roots::TLS_SERVER_ROOTS.iter().cloned(),
        ))
        .with_no_client_auth();

    Ok(config)
}

/// Whether an IMAP failure carries the `TRYCREATE` response code.
///
/// It is a bracketed protocol token (`NO [TRYCREATE] ...`), not prose, so a
/// match on it is a match on the protocol, not on a server's wording.
fn is_trycreate_response(error: &str) -> bool {
    error.to_ascii_uppercase().contains("[TRYCREATE]")
}

pub fn parse_header_value(header: &str, key: &str) -> Option<String> {
    parse_raw_header_value(header, key).map(|value| decode_mime_encoded_words(&value))
}

/// The unfolded value of a header, encoded words left as they are.
pub(crate) fn parse_raw_header_value(header: &str, key: &str) -> Option<String> {
    let mut collecting = false;
    let mut value = String::new();

    for raw_line in header.lines() {
        let line = raw_line.trim_end_matches('\r');
        if line.is_empty() {
            break;
        }

        if line.starts_with(' ') || line.starts_with('\t') {
            if collecting {
                if !value.is_empty() {
                    value.push(' ');
                }
                value.push_str(line.trim());
            }
            continue;
        }

        if collecting {
            break;
        }

        if let Some((name, line_value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case(key) {
                collecting = true;
                value.push_str(line_value.trim());
            }
        }
    }

    collecting.then_some(value)
}

pub fn parse_from_address(header: &str) -> Option<String> {
    let raw = parse_header_value(header, "From")?;
    if let Some(address) = extract_email_address(&raw) {
        return Some(address);
    }

    Some(raw.trim().to_string())
}

pub fn parse_from_name(header: &str) -> Option<String> {
    let raw = parse_header_value(header, "From")?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Some(start) = trimmed.find('<') {
        let display = trimmed[..start].trim().trim_matches('"').trim();
        if !display.is_empty() {
            return Some(display.to_string());
        }
    }

    if extract_email_address(trimmed).is_some() {
        return None;
    }

    Some(trimmed.to_string())
}

pub fn parse_header_address_list(header: &str, key: &str) -> Vec<String> {
    let Some(raw) = parse_header_value(header, key) else {
        return vec![];
    };

    parse_address_list_tokens(&raw, AddressListDecodeStrategy::AlreadyDecoded)
}

pub fn split_references(raw: &str) -> Vec<String> {
    raw.split_whitespace()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .collect()
}

#[derive(Debug, Default)]
pub struct ParsedBody {
    pub text: Option<String>,
    pub html: Option<String>,
    /// Raw VCALENDAR content from `text/calendar` parts. Surfaced separately so
    /// the user-visible body stays clean while the calendar pipeline still picks
    /// up the invitation.
    pub calendar: Option<String>,
    pub smime_encrypted: Option<Vec<u8>>,
    pub smime_signed: Option<Vec<u8>>,
    pub pgp_signed: Option<Vec<u8>>,
    pub attachments: Vec<RemoteAttachment>,
    pub trust: super::traits::MessageTrust,
}

/// Whether a parsed part carries its bytes along in the payload.
///
/// The two body parsers need different answers, and the difference is not a
/// preference: it is whether the bytes exist anywhere else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EmbedPolicy {
    /// Everything. Used on a decrypted payload, whose attachments live only in
    /// that plaintext — the server holds one encrypted part, not these. Drop
    /// the bytes here and an attachment on encrypted mail becomes unopenable.
    All,
    /// Inline parts, because a `cid:` reference resolves against nothing else,
    /// and `text/calendar`, because the invite is read straight out of the
    /// payload. The rest is fetched on demand by `remote_part_id`; embedding it
    /// would hold the decoded bytes and a base64 string a third larger at once.
    InlineAndInvites,
}

fn attachment_should_embed(attachment: &ParsedAttachment, policy: EmbedPolicy) -> bool {
    match policy {
        EmbedPolicy::All => true,
        EmbedPolicy::InlineAndInvites => {
            attachment.is_inline
                || attachment
                    .content_type
                    .to_ascii_lowercase()
                    .starts_with("text/calendar")
        }
    }
}

/// Build the wire attachment under a stated policy.
///
/// Both parsers built this struct by hand, which is how the policy came to be
/// written twice — and how a change to one of them could quietly not reach the
/// other.
fn remote_attachment_from(attachment: ParsedAttachment, policy: EmbedPolicy) -> RemoteAttachment {
    let content_base64 = attachment_should_embed(&attachment, policy)
        .then(|| base64::engine::general_purpose::STANDARD.encode(&attachment.bytes));
    RemoteAttachment {
        remote_part_id: attachment.remote_part_id,
        filename: attachment.filename,
        content_type: attachment.content_type,
        size: Some(i64::try_from(attachment.bytes.len()).unwrap_or(i64::MAX)),
        content_id: attachment.content_id,
        is_inline: attachment.is_inline,
        content_base64,
    }
}

/// Parse the body of a complete RFC822 message while retaining any exact
/// RFC3156 bundle needed for detached-signature verification.
pub fn parse_raw_message_body(raw: &[u8]) -> ParsedBody {
    let (header, body) = split_header_and_body(raw);
    let header = String::from_utf8_lossy(header);
    let content_type = parse_header_value(&header, "Content-Type")
        .unwrap_or_else(|| "text/plain; charset=utf-8".to_string());
    let transfer_encoding = parse_header_value(&header, "Content-Transfer-Encoding")
        .unwrap_or_else(|| "8bit".to_string());
    let mut parsed = parse_body_part(&content_type, &transfer_encoding, body, 0);
    parsed.trust = root_crypto_trust(&content_type, &parsed);
    parsed.attachments = parse_attachments_from_body(&content_type, &transfer_encoding, body)
        .into_iter()
        .map(|attachment| remote_attachment_from(attachment, EmbedPolicy::All))
        .collect();
    parsed
}

#[derive(Debug, Default)]
struct ExtractedImapEmail {
    text: Option<String>,
    html: Option<String>,
    attachments: Vec<RemoteAttachment>,
}

fn parse_extracted_imap_email(
    header: &str,
    body: Option<&[u8]>,
    source_id: SourceId,
    mailbox: &str,
    uid_validity: u32,
    uid: u32,
) -> Option<ExtractedImapEmail> {
    if uid == 0 || uid_validity == 0 {
        return None;
    }
    let identity = imap_message_identity(source_id, mailbox, uid_validity, uid);
    let mut raw = Vec::with_capacity(header.len() + body.map_or(0, <[u8]>::len) + 4);
    raw.extend_from_slice(header.as_bytes());
    if !header.ends_with("\r\n") {
        raw.extend_from_slice(b"\r\n");
    }
    raw.extend_from_slice(b"\r\n");
    if let Some(body) = body {
        raw.extend_from_slice(body);
    }

    let parsed = parse_core_rfc822(
        &raw,
        MimeContext::new(
            identity,
            chrono::Utc::now(),
            "imap",
            format!("{uid_validity}/{uid}"),
        ),
    )
    .ok()?;
    parsed
        .message()
        .extensions()
        .get::<EmailMetadata>()
        .ok()??;
    let attachments = parsed
        .email()
        .attachments()
        .iter()
        .map(|attachment| {
            let bytes = parsed.materialize(&attachment.content);
            RemoteAttachment {
                remote_part_id: attachment.part_id.clone(),
                filename: attachment.filename.clone(),
                content_type: crate::email::mime_type::normalize_mime_type(&attachment.media_type),
                size: bytes.and_then(|bytes| i64::try_from(bytes.len()).ok()),
                content_id: attachment.content_id.clone(),
                is_inline: attachment.inline,
                content_base64: attachment.inline.then(|| {
                    base64::engine::general_purpose::STANDARD.encode(bytes.unwrap_or_default())
                }),
            }
        })
        .collect();
    Some(ExtractedImapEmail {
        text: parsed.email().text_body().map(ToString::to_string),
        html: parsed.email().html_body().map(ToString::to_string),
        attachments,
    })
}

fn merge_extracted_imap_fallback(
    parsed_body: &mut ParsedBody,
    extracted: Option<ExtractedImapEmail>,
) {
    let Some(extracted) = extracted else {
        return;
    };
    if !parsed_body.trust.root_crypto {
        if parsed_body.text.is_none() {
            parsed_body.text = extracted.text;
        }
        if parsed_body.html.is_none() {
            parsed_body.html = extracted.html;
        }
    }
    if parsed_body.attachments.is_empty() {
        parsed_body.attachments = extracted.attachments;
    }
}

/// How long a message's parsed parts stay in memory.
///
/// Opening the second attachment of a mail is one click away; a message nobody
/// has touched for a minute is not worth the bytes.
const ATTACHMENT_PART_CACHE_TTL: Duration = Duration::from_secs(60);

/// Total decoded bytes the cache will hold on to.
///
/// A mail with five 10 MB parts is the case this exists for, so the budget sits
/// above it. Anything larger is still served — it is simply not retained.
const ATTACHMENT_PART_CACHE_BUDGET: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
struct AttachmentCacheKey {
    account: String,
    folder: String,
    uidvalidity: Option<u32>,
    remote_id: String,
}

struct CachedAttachmentParts {
    key: AttachmentCacheKey,
    parts: Arc<Vec<ParsedAttachment>>,
    bytes: usize,
    stored_at: Instant,
}

/// Parsed parts of the last few messages whose attachments were opened.
///
/// Process-wide rather than per provider: `attachment_bytes` builds a fresh
/// provider for every part it resolves, so an instance-level cache would never
/// see a second hit.
static ATTACHMENT_PART_CACHE: std::sync::Mutex<Vec<CachedAttachmentParts>> =
    std::sync::Mutex::new(Vec::new());

fn attachment_part_cache() -> std::sync::MutexGuard<'static, Vec<CachedAttachmentParts>> {
    ATTACHMENT_PART_CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn cached_attachment_parts(
    key: &AttachmentCacheKey,
    now: Instant,
) -> Option<Arc<Vec<ParsedAttachment>>> {
    let mut cache = attachment_part_cache();
    cache.retain(|entry| now.duration_since(entry.stored_at) <= ATTACHMENT_PART_CACHE_TTL);
    cache
        .iter()
        .find(|entry| entry.key == *key)
        .map(|entry| entry.parts.clone())
}

fn store_attachment_parts(
    key: &AttachmentCacheKey,
    parts: &Arc<Vec<ParsedAttachment>>,
    now: Instant,
) {
    let bytes = parts.iter().map(|part| part.bytes.len()).sum::<usize>();
    if bytes > ATTACHMENT_PART_CACHE_BUDGET {
        return;
    }
    let mut cache = attachment_part_cache();
    cache.retain(|entry| {
        entry.key != *key && now.duration_since(entry.stored_at) <= ATTACHMENT_PART_CACHE_TTL
    });
    cache.push(CachedAttachmentParts {
        key: key.clone(),
        parts: parts.clone(),
        bytes,
        stored_at: now,
    });
    // Oldest first, so the message someone is clicking through survives.
    while cache.len() > 1
        && cache.iter().map(|entry| entry.bytes).sum::<usize>() > ATTACHMENT_PART_CACHE_BUDGET
    {
        cache.remove(0);
    }
}

#[derive(Debug, Clone)]
struct ParsedAttachment {
    remote_part_id: String,
    filename: Option<String>,
    content_type: String,
    content_id: Option<String>,
    is_inline: bool,
    bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct ParsedRawEmail {
    pub subject: String,
    pub from_address: String,
    pub from_name: Option<String>,
    pub to_addresses: Vec<String>,
    pub cc_addresses: Vec<String>,
    pub date_rfc3339: String,
    pub message_id_header: String,
    pub body_text: Option<String>,
    pub body_html: Option<String>,
    pub attachments: Vec<ParsedRawAttachment>,
    pub raw_headers: String,
}

#[derive(Debug, Clone)]
pub struct ParsedRawAttachment {
    pub remote_part_id: String,
    pub filename: Option<String>,
    pub content_type: String,
    pub content_id: Option<String>,
    pub is_inline: bool,
    pub bytes: Vec<u8>,
}

pub fn parse_raw_rfc822(raw: &[u8]) -> Result<ParsedRawEmail, String> {
    let (header_bytes, body) = split_header_and_body(raw);
    // Headers stay lossy on purpose: RFC 5322 makes them ASCII, anything else
    // arrives RFC 2047 encoded, and undeclared 8-bit bytes in a header are
    // common enough that refusing the message would be worse. Bodies are the
    // part that must survive as bytes — see `decode_part_content`.
    let header = String::from_utf8_lossy(header_bytes);
    let header = header.as_ref();
    if header.trim().is_empty() {
        return Err("message has no RFC822 headers".to_string());
    }
    let content_type = parse_header_value(header, "Content-Type")
        .unwrap_or_else(|| "text/plain; charset=utf-8".to_string());
    let transfer_encoding = parse_header_value(header, "Content-Transfer-Encoding")
        .unwrap_or_else(|| "8bit".to_string());
    let parsed = parse_body_part(&content_type, &transfer_encoding, body, 0);
    let attachments = parse_attachments_from_body(&content_type, &transfer_encoding, body)
        .into_iter()
        .map(|attachment| ParsedRawAttachment {
            remote_part_id: attachment.remote_part_id,
            filename: attachment.filename,
            content_type: attachment.content_type,
            content_id: attachment.content_id,
            is_inline: attachment.is_inline,
            bytes: attachment.bytes,
        })
        .collect::<Vec<_>>();

    Ok(ParsedRawEmail {
        subject: parse_header_value(header, "Subject").unwrap_or_else(|| "(no subject)".into()),
        from_address: parse_from_address(header).unwrap_or_else(|| "unknown@import.local".into()),
        from_name: parse_from_name(header),
        to_addresses: parse_header_address_list(header, "To"),
        cc_addresses: parse_header_address_list(header, "Cc"),
        date_rfc3339: parse_message_date(header, None, chrono::Utc::now()),
        message_id_header: parse_header_value(header, "Message-ID")
            .unwrap_or_else(|| format!("<import-{}@satchel.local>", uuid::Uuid::new_v4())),
        body_text: parsed.text,
        body_html: parsed.html,
        attachments,
        raw_headers: header.to_string(),
    })
}

pub fn parse_message_body(header: &str, raw_text: Option<&[u8]>) -> ParsedBody {
    let Some(bytes) = raw_text else {
        return ParsedBody::default();
    };
    if bytes.is_empty() {
        return ParsedBody::default();
    }

    let content_type = parse_header_value(header, "Content-Type")
        .unwrap_or_else(|| "text/plain; charset=utf-8".to_string());
    let transfer_encoding = parse_header_value(header, "Content-Transfer-Encoding")
        .unwrap_or_else(|| "8bit".to_string());
    let effective_content_type = effective_body_content_type(&content_type, bytes);
    let parse_content_type = effective_content_type.as_deref().unwrap_or(&content_type);
    let mut parsed = parse_body_part(parse_content_type, &transfer_encoding, bytes, 0);
    parsed.trust = root_crypto_trust(parse_content_type, &parsed);
    parsed.attachments = parse_attachments_from_body(parse_content_type, &transfer_encoding, bytes)
        .into_iter()
        .map(|attachment| remote_attachment_from(attachment, EmbedPolicy::InlineAndInvites))
        .collect();
    parsed
}

pub fn parse_mime_body_fragment(raw_body: &str) -> Option<(Option<String>, Option<String>)> {
    let boundary = detect_leading_multipart_boundary(raw_body.as_bytes())?;
    let content_type = format!("multipart/mixed; boundary=\"{boundary}\"");
    let parsed = parse_body_part(&content_type, "8bit", raw_body.as_bytes(), 0);
    if parsed.text.is_some() || parsed.html.is_some() {
        Some((parsed.text, parsed.html))
    } else {
        None
    }
}

fn effective_body_content_type(content_type: &str, raw_body: &[u8]) -> Option<String> {
    let lowered = content_type.to_ascii_lowercase();
    if lowered.starts_with("multipart/") {
        return None;
    }
    detect_leading_multipart_boundary(raw_body)
        .map(|boundary| format!("multipart/mixed; boundary=\"{boundary}\""))
}

fn parse_body_part(
    content_type: &str,
    transfer_encoding: &str,
    raw_body: &[u8],
    depth: usize,
) -> ParsedBody {
    const MAX_DEPTH: usize = 10;
    if depth > MAX_DEPTH {
        return ParsedBody::default();
    }

    let lowered_content_type = content_type.to_ascii_lowercase();
    if lowered_content_type.starts_with("application/pkcs7-mime")
        || lowered_content_type.starts_with("application/x-pkcs7-mime")
    {
        let decoded = decode_part_bytes(raw_body, transfer_encoding);
        let is_enveloped = match extract_header_param(content_type, "smime-type").as_deref() {
            Some(kind) => kind.eq_ignore_ascii_case("enveloped-data"),
            None => lowered_content_type.contains("smime.p7m"),
        };
        return if is_enveloped {
            ParsedBody {
                text: None,
                html: None,
                calendar: None,
                smime_encrypted: Some(decoded),
                smime_signed: None,
                pgp_signed: None,
                trust: Default::default(),
                attachments: Vec::new(),
            }
        } else {
            ParsedBody {
                text: None,
                html: None,
                calendar: None,
                smime_encrypted: None,
                smime_signed: Some(decoded),
                pgp_signed: None,
                trust: Default::default(),
                attachments: Vec::new(),
            }
        };
    }

    if lowered_content_type.starts_with("message/rfc822") {
        // Only the transfer encoding is undone here. The embedded message
        // declares its own charset per part, so decoding it with the enclosing
        // part's charset first would destroy exactly what those headers exist
        // to describe.
        let decoded = decode_part_bytes(raw_body, transfer_encoding);
        let (embedded_header, embedded_body) = split_header_and_body(&decoded);
        let embedded_header = String::from_utf8_lossy(embedded_header);
        let embedded_content_type = parse_header_value(&embedded_header, "Content-Type")
            .unwrap_or_else(|| "text/plain; charset=utf-8".to_string());
        let embedded_encoding = parse_header_value(&embedded_header, "Content-Transfer-Encoding")
            .unwrap_or_else(|| "8bit".to_string());
        let nested = parse_body_part(
            &embedded_content_type,
            &embedded_encoding,
            embedded_body,
            depth + 1,
        );
        if nested.text.is_some() || nested.html.is_some() {
            return nested;
        }

        let charset = extract_header_param(content_type, "charset")
            .unwrap_or_else(|| "utf-8".to_string())
            .to_ascii_lowercase();
        let fallback = normalize_body_text(&decode_bytes_with_charset(&decoded, &charset));
        return if fallback.is_empty() {
            ParsedBody::default()
        } else {
            ParsedBody {
                text: Some(fallback),
                html: None,
                calendar: None,
                smime_encrypted: None,
                smime_signed: None,
                pgp_signed: None,
                trust: Default::default(),
                attachments: Vec::new(),
            }
        };
    }

    if lowered_content_type.starts_with("multipart/") {
        if lowered_content_type.starts_with("multipart/signed")
            && SignedProtocol::declared(content_type).is_some()
        {
            return parse_signed_body(content_type, raw_body, depth);
        }
        if lowered_content_type.starts_with("multipart/encrypted")
            && lowered_content_type.contains("application/pgp-encrypted")
        {
            if depth != 0 {
                return ParsedBody::default();
            }
            let sections = extract_header_param(content_type, "boundary")
                .map(|boundary| split_multipart_sections(raw_body, &boundary))
                .unwrap_or_default();
            if sections.len() != 2 {
                return ParsedBody::default();
            }
            let (header, ciphertext) = split_header_and_body(sections[1]);
            let header = String::from_utf8_lossy(header);
            let encoding = parse_header_value(&header, "Content-Transfer-Encoding")
                .unwrap_or_else(|| "8bit".into());
            return ParsedBody {
                text: Some(
                    String::from_utf8_lossy(&decode_part_bytes(ciphertext, &encoding))
                        .trim()
                        .to_string(),
                ),
                ..Default::default()
            };
        }
        if let Some(boundary) = extract_header_param(content_type, "boundary") {
            let mut parsed = ParsedBody::default();
            for section in split_multipart_sections(raw_body, &boundary) {
                let (part_header, part_body) = split_header_and_body(section);
                let part_header = String::from_utf8_lossy(part_header);
                let part_type = parse_header_value(&part_header, "Content-Type")
                    .unwrap_or_else(|| "text/plain; charset=utf-8".to_string());
                let part_disposition = parse_header_value(&part_header, "Content-Disposition")
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                let part_encoding = parse_header_value(&part_header, "Content-Transfer-Encoding")
                    .unwrap_or_else(|| "8bit".to_string());

                let lowered_part = part_type.to_ascii_lowercase();

                // text/calendar deserves first-class handling — it shows up either
                // inline (Outlook/Exchange/Gmail invites) or as an attachment with
                // Content-Disposition: attachment (some Office365 setups).
                if lowered_part.starts_with("text/calendar") && parsed.calendar.is_none() {
                    let decoded = decode_part_content(part_body, &part_encoding, &part_type);
                    let normalized = normalize_body_text(&decoded);
                    if !normalized.is_empty() {
                        parsed.calendar = Some(normalized);
                    }
                    continue;
                }

                if part_disposition.contains("attachment") {
                    continue;
                }

                let nested = parse_body_part(&part_type, &part_encoding, part_body, depth + 1);
                if nested.text.is_some()
                    || nested.html.is_some()
                    || nested.calendar.is_some()
                    || nested.smime_encrypted.is_some()
                    || nested.smime_signed.is_some()
                    || nested.pgp_signed.is_some()
                {
                    merge_parsed_body(&mut parsed, nested);
                    continue;
                }
                // An inline image is the attachment scan's; decoding it here
                // only to discard it doubled the cost of every such message.
                if !lowered_part.starts_with("text/") {
                    continue;
                }

                let decoded = decode_part_content(part_body, &part_encoding, &part_type);
                let normalized = normalize_body_text(&decoded);
                if normalized.is_empty() {
                    continue;
                }

                if lowered_part.starts_with("text/plain")
                    || (lowered_part.starts_with("text/")
                        && !lowered_part.starts_with("text/html")
                        && parsed.text.is_none())
                {
                    if parsed.text.is_none() {
                        parsed.text = Some(normalized);
                    }
                } else if lowered_part.starts_with("text/html") && parsed.html.is_none() {
                    parsed.html = Some(normalized);
                }
            }

            if parsed.text.is_none() && parsed.html.is_none() {
                let fallback = normalize_body_text(&decode_part_content(
                    raw_body,
                    transfer_encoding,
                    content_type,
                ));
                if !fallback.is_empty() {
                    parsed.text = Some(fallback);
                }
            }
            return parsed;
        }
    }

    if !lowered_content_type.starts_with("text/") {
        return ParsedBody::default();
    }

    let decoded = decode_part_content(raw_body, transfer_encoding, content_type);
    let normalized = normalize_body_text(&decoded);
    if normalized.is_empty() {
        return ParsedBody::default();
    }

    if lowered_content_type.starts_with("text/html") {
        ParsedBody {
            text: None,
            html: Some(normalized),
            calendar: None,
            smime_encrypted: None,
            smime_signed: None,
            pgp_signed: None,
            trust: Default::default(),
            attachments: Vec::new(),
        }
    } else if lowered_content_type.starts_with("text/calendar") {
        ParsedBody {
            text: None,
            html: None,
            calendar: Some(normalized),
            smime_encrypted: None,
            smime_signed: None,
            pgp_signed: None,
            trust: Default::default(),
            attachments: Vec::new(),
        }
    } else {
        ParsedBody {
            text: Some(normalized),
            html: None,
            calendar: None,
            smime_encrypted: None,
            smime_signed: None,
            pgp_signed: None,
            trust: Default::default(),
            attachments: Vec::new(),
        }
    }
}

/// Display exactly the entity a `multipart/signed` signature covers, and keep
/// the bundle that verification later reads it back from.
///
/// Both sides take the entity from `signed_mime`. What it refuses is shown as
/// nothing here, so no signature state can attach to it.
fn parse_signed_body(content_type: &str, raw_body: &[u8], depth: usize) -> ParsedBody {
    let Ok(parts) = signed_mime::split(content_type, raw_body) else {
        return ParsedBody::default();
    };
    let mut bundle = format!("Content-Type: {content_type}\r\n\r\n").into_bytes();
    bundle.extend_from_slice(raw_body);
    // Verification splits the stored bundle, not this body. Show nothing it
    // would not find there byte for byte.
    if signed_mime::split_bundle(&bundle).map(|stored| stored.entity) != Ok(parts.entity) {
        return ParsedBody::default();
    }

    let mut parsed = display_signed_entity(parts.entity, depth + 1);
    parsed.smime_encrypted = None;
    parsed.smime_signed = None;
    parsed.pgp_signed = None;
    match parts.protocol {
        SignedProtocol::OpenPgp => parsed.pgp_signed = Some(bundle),
        SignedProtocol::Smime => parsed.smime_signed = Some(bundle),
    }
    parsed
}

/// What the reader is shown of a signed entity: its headers name the type
/// and transfer encoding of the content that follows them.
pub(crate) fn display_signed_entity(entity: &[u8], depth: usize) -> ParsedBody {
    let (part_header, part_body) = signed_mime::entity_header_and_body(entity);
    let part_header = String::from_utf8_lossy(part_header);
    let part_type = parse_header_value(&part_header, "Content-Type")
        .unwrap_or_else(|| "text/plain; charset=utf-8".to_string());
    let part_encoding = parse_header_value(&part_header, "Content-Transfer-Encoding")
        .unwrap_or_else(|| "8bit".to_string());
    parse_body_part(&part_type, &part_encoding, part_body, depth)
}

pub(super) fn root_crypto_trust(
    content_type: &str,
    body: &ParsedBody,
) -> super::traits::MessageTrust {
    let mime = content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let text = body.text.as_deref().unwrap_or_default().trim();
    let inline = mime == "text/plain" && body.html.is_none();
    let root_inline_signature = inline
        && text.starts_with("-----BEGIN PGP SIGNED MESSAGE-----")
        && text.ends_with("-----END PGP SIGNATURE-----");
    let root_encryption = body.smime_encrypted.is_some()
        && matches!(
            mime.as_str(),
            "application/pkcs7-mime" | "application/x-pkcs7-mime"
        )
        || ((inline || mime == "multipart/encrypted")
            && text.starts_with("-----BEGIN PGP MESSAGE-----")
            && text.ends_with("-----END PGP MESSAGE-----"));
    let root_crypto = root_encryption
        || root_inline_signature
        || match mime.as_str() {
            "multipart/signed" => body.smime_signed.is_some() || body.pgp_signed.is_some(),
            "application/pkcs7-mime" | "application/x-pkcs7-mime" => body.smime_signed.is_some(),
            _ => false,
        };
    super::traits::MessageTrust {
        root_crypto,
        root_encryption,
        root_inline_signature,
        ..Default::default()
    }
}

fn merge_parsed_body(target: &mut ParsedBody, candidate: ParsedBody) {
    if target.text.is_none() {
        target.text = candidate.text;
    }
    if target.html.is_none() {
        target.html = candidate.html;
    }
    if target.calendar.is_none() {
        target.calendar = candidate.calendar;
    }

    if target.smime_signed.is_none() {
        target.smime_signed = candidate.smime_signed;
    }
    if target.pgp_signed.is_none() {
        target.pgp_signed = candidate.pgp_signed;
    }
    target.attachments.extend(candidate.attachments);
}

fn parse_attachments_from_body(
    content_type: &str,
    transfer_encoding: &str,
    raw_body: &[u8],
) -> Vec<ParsedAttachment> {
    let mut out = Vec::new();
    let mut counter = 0usize;
    collect_attachment_parts(
        content_type,
        transfer_encoding,
        raw_body,
        None,
        None,
        0,
        &mut counter,
        &mut out,
    );
    out
}

#[allow(clippy::too_many_arguments)]
fn collect_attachment_parts(
    content_type: &str,
    transfer_encoding: &str,
    raw_body: &[u8],
    disposition: Option<&str>,
    content_id: Option<String>,
    depth: usize,
    counter: &mut usize,
    out: &mut Vec<ParsedAttachment>,
) {
    const MAX_DEPTH: usize = 10;
    if depth > MAX_DEPTH {
        return;
    }

    let lowered_content_type = content_type.to_ascii_lowercase();
    if lowered_content_type.starts_with("multipart/") {
        if let Some(boundary) = extract_header_param(content_type, "boundary") {
            for section in split_multipart_sections(raw_body, &boundary) {
                let (part_header, part_body) = split_header_and_body(section);
                let part_header = String::from_utf8_lossy(part_header);
                let part_type = parse_header_value(&part_header, "Content-Type")
                    .unwrap_or_else(|| "application/octet-stream".to_string());
                let part_disposition = parse_header_value(&part_header, "Content-Disposition");
                let part_encoding = parse_header_value(&part_header, "Content-Transfer-Encoding")
                    .unwrap_or_else(|| "8bit".to_string());
                let part_content_id = parse_header_value(&part_header, "Content-ID")
                    .map(|value| value.trim_matches(['<', '>']).to_string())
                    .filter(|value| !value.is_empty());
                collect_attachment_parts(
                    &part_type,
                    &part_encoding,
                    part_body,
                    part_disposition.as_deref(),
                    part_content_id,
                    depth + 1,
                    counter,
                    out,
                );
            }
        }
        return;
    }

    if lowered_content_type.starts_with("message/rfc822") {
        let filename = extract_filename_param(disposition.unwrap_or_default())
            .or_else(|| extract_filename_param(content_type))
            .map(|value| decode_mime_encoded_words(&value));
        let lowered_disposition = disposition.unwrap_or_default().to_ascii_lowercase();
        if lowered_disposition.contains("attachment") || filename.is_some() {
            *counter += 1;
            let bytes = decode_part_bytes(raw_body, transfer_encoding);
            out.push(ParsedAttachment {
                remote_part_id: format!("mime-{counter}"),
                filename: filename.or_else(|| Some("attached-message.eml".to_string())),
                content_type: "message/rfc822".to_string(),
                content_id,
                is_inline: false,
                bytes,
            });
            return;
        }

        let decoded = decode_part_bytes(raw_body, transfer_encoding);
        let (embedded_header, embedded_body) = split_header_and_body(&decoded);
        let embedded_header = String::from_utf8_lossy(embedded_header);
        let embedded_content_type = parse_header_value(&embedded_header, "Content-Type")
            .unwrap_or_else(|| "text/plain; charset=utf-8".to_string());
        let embedded_encoding = parse_header_value(&embedded_header, "Content-Transfer-Encoding")
            .unwrap_or_else(|| "8bit".to_string());
        collect_attachment_parts(
            &embedded_content_type,
            &embedded_encoding,
            embedded_body,
            disposition,
            content_id,
            depth + 1,
            counter,
            out,
        );
        return;
    }

    let lowered_disposition = disposition.unwrap_or_default().to_ascii_lowercase();
    if lowered_content_type.starts_with("application/pgp-signature")
        || lowered_content_type.starts_with("application/pkcs7-signature")
        || lowered_content_type.starts_with("application/x-pkcs7-signature")
    {
        return;
    }
    let filename = extract_filename_param(disposition.unwrap_or_default())
        .or_else(|| extract_filename_param(content_type))
        .map(|value| decode_mime_encoded_words(&value));
    let disposition_type = lowered_disposition
        .split(';')
        .next()
        .unwrap_or_default()
        .trim();
    // Outlook gives regular image attachments a Content-ID too. An explicit
    // attachment disposition takes precedence over that inline-image hint.
    let is_inline_content = disposition_type == "inline"
        || (disposition_type != "attachment"
            && content_id.is_some()
            && lowered_content_type.starts_with("image/"));
    let is_attachment = disposition_type == "attachment" || filename.is_some() || is_inline_content;
    if !is_attachment || lowered_content_type.starts_with("text/calendar") {
        return;
    }

    let bytes = decode_part_bytes(raw_body, transfer_encoding);
    if is_tnef_part(&lowered_content_type, filename.as_deref()) {
        let expanded = expand_tnef_attachments(&bytes, counter);
        if !expanded.is_empty() {
            out.extend(expanded);
            return;
        }
    }

    *counter += 1;
    out.push(ParsedAttachment {
        remote_part_id: format!("mime-{counter}"),
        filename,
        content_type: crate::email::mime_type::normalize_mime_type(content_type),
        content_id,
        is_inline: is_inline_content,
        bytes,
    });
}

fn is_tnef_part(lowered_content_type: &str, filename: Option<&str>) -> bool {
    lowered_content_type.starts_with("application/ms-tnef")
        || lowered_content_type.starts_with("application/vnd.ms-tnef")
        || filename.is_some_and(|value| value.eq_ignore_ascii_case("winmail.dat"))
}

fn expand_tnef_attachments(bytes: &[u8], counter: &mut usize) -> Vec<ParsedAttachment> {
    let Ok(attachments) = tnef::read_attachments(bytes) else {
        return Vec::new();
    };
    attachments
        .into_iter()
        .filter(|attachment| !attachment.data.is_empty())
        .map(|attachment| {
            *counter += 1;
            let filename = attachment
                .transport_filename
                .filter(|value| !value.trim().is_empty())
                .or_else(|| {
                    if attachment.title.trim().is_empty() {
                        None
                    } else {
                        Some(attachment.title)
                    }
                });
            ParsedAttachment {
                remote_part_id: format!("mime-{counter}"),
                filename,
                content_type: "application/octet-stream".to_string(),
                content_id: None,
                is_inline: false,
                bytes: attachment.data.to_vec(),
            }
        })
        .collect()
}

/// `[T]::split` only splits on a single element, and a MIME boundary is a
/// whole string.
fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    memchr::memmem::find(haystack, needle)
}

/// The offset just past the end of a delimiter line.
///
/// `None` means the line carries something besides trailing whitespace — then
/// the match belongs to a longer boundary that merely starts like this one.
fn delimiter_line_end(body: &[u8], from: usize) -> Option<usize> {
    let mut index = from;
    while matches!(body.get(index), Some(b' ' | b'\t')) {
        index += 1;
    }
    match body.get(index) {
        None => Some(body.len()),
        Some(b'\n') => Some(index + 1),
        Some(b'\r') if body.get(index + 1) == Some(&b'\n') => Some(index + 2),
        _ => None,
    }
}

/// Split a multipart body into its parts.
///
/// A delimiter is a line of its own (RFC 2046): the body's very first line, or
/// one introduced by a newline. Matching `--boundary` at any byte position let
/// an outer boundary that is a prefix of an inner one — `=_b_1` against
/// `=_b_12` — cut the inner parts apart, and their remains were stored as the
/// message text. A part whose own content begins with `--` no longer ends the
/// walk either; only the real closing delimiter does.
fn split_multipart_sections<'a>(body: &'a [u8], boundary: &str) -> Vec<&'a [u8]> {
    let marker = format!("--{boundary}").into_bytes();
    if marker.len() > body.len() {
        return Vec::new();
    }

    let mut sections = Vec::new();
    let mut part_start: Option<usize> = None;
    for index in memchr::memmem::find_iter(body, &marker) {
        if index != 0 && body.get(index.wrapping_sub(1)) != Some(&b'\n') {
            continue;
        }
        let after = index + marker.len();
        let closing = body[after..].starts_with(b"--");
        let Some(next_line) = delimiter_line_end(body, if closing { after + 2 } else { after })
        else {
            continue;
        };

        if let Some(start) = part_start.take() {
            // The newline that introduced this delimiter belongs to the
            // delimiter, not to the part before it.
            let mut end = index;
            if end > start && body[end - 1] == b'\n' {
                end -= 1;
            }
            if end > start && body[end - 1] == b'\r' {
                end -= 1;
            }
            let part = body[start..end].trim_ascii();
            if !part.is_empty() {
                sections.push(part);
            }
        }
        if closing {
            return sections;
        }
        part_start = Some(next_line);
    }

    // No closing delimiter: keep what the sender did write.
    if let Some(start) = part_start {
        let part = body[start..].trim_ascii();
        if !part.is_empty() {
            sections.push(part);
        }
    }
    sections
}

fn detect_leading_multipart_boundary(body: &[u8]) -> Option<String> {
    let first = body
        .split(|byte| *byte == b'\n')
        .map(<[u8]>::trim_ascii)
        .find(|line| !line.is_empty())?;
    // A boundary is `bcharsnospace` per RFC 2046, so anything unreadable as
    // UTF-8 here is not one.
    let first = std::str::from_utf8(first).ok()?;
    let boundary = first.strip_prefix("--")?;
    if boundary.is_empty()
        || boundary.starts_with("--")
        || boundary.len() > 180
        || boundary.chars().any(char::is_whitespace)
    {
        return None;
    }

    let first_part = split_multipart_sections(body, boundary)
        .into_iter()
        .next()?;
    let (part_header, _) = split_header_and_body(first_part);
    parse_header_value(&String::from_utf8_lossy(part_header), "Content-Type")?;

    Some(boundary.to_string())
}

fn split_header_and_body(section: &[u8]) -> (&[u8], &[u8]) {
    if let Some(index) = find_bytes(section, b"\r\n\r\n") {
        return (&section[..index], &section[index + 4..]);
    }
    if let Some(index) = find_bytes(section, b"\n\n") {
        return (&section[..index], &section[index + 2..]);
    }
    (b"", section)
}

pub(crate) fn extract_header_param(input: &str, key: &str) -> Option<String> {
    let needle = key.to_ascii_lowercase();
    for raw_part in input.split(';').skip(1) {
        // An empty or valueless segment is not the end of the header. Bounce
        // generators write `multipart/mixed; ; boundary="x"`, and giving up
        // there cost the message its body and every attachment.
        let Some((name, value)) = raw_part.split_once('=') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case(&needle) {
            let trimmed = value.trim().trim_matches('"').trim_matches('\'').trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn extract_filename_param(input: &str) -> Option<String> {
    extract_rfc2231_param(input, "filename").or_else(|| extract_rfc2231_param(input, "name"))
}

fn extract_rfc2231_param(input: &str, key: &str) -> Option<String> {
    let key_lc = key.to_ascii_lowercase();
    let mut plain = None::<String>;
    let mut extended = None::<String>;
    let mut segments = Vec::<(usize, bool, String)>::new();

    for raw_part in input.split(';').skip(1) {
        let Some((name_raw, value_raw)) = raw_part.split_once('=') else {
            continue;
        };
        let name = name_raw.trim().to_ascii_lowercase();
        let value = value_raw
            .trim()
            .trim_matches('"')
            .trim_matches('\'')
            .to_string();
        if name == key_lc {
            plain = Some(value);
            continue;
        }
        if name == format!("{key_lc}*") {
            extended = Some(value);
            continue;
        }
        if let Some(rest) = name.strip_prefix(&(key_lc.clone() + "*")) {
            let extended_segment = rest.ends_with('*');
            let index_raw = rest.trim_end_matches('*');
            if let Ok(index) = index_raw.parse::<usize>() {
                segments.push((index, extended_segment, value));
            }
        }
    }

    if let Some(value) = decode_rfc2231_segments(&segments) {
        return Some(value);
    }
    if let Some(value) = extended {
        return Some(decode_rfc2231_value(&value));
    }
    plain.map(|value| decode_mime_encoded_words(&value))
}

fn decode_part_content(raw: &[u8], transfer_encoding: &str, content_type: &str) -> String {
    let decoded_bytes = decode_part_bytes(raw, transfer_encoding);

    let charset = extract_header_param(content_type, "charset")
        .unwrap_or_else(|| "utf-8".to_string())
        .to_ascii_lowercase();
    decode_bytes_with_charset(&decoded_bytes, &charset)
}

/// Base64 as senders actually emit it: padding optional, non-zero bits in the
/// last quantum tolerated. The canonical engine rejected both, and the
/// rejection used to become an empty body or a zero-byte attachment.
const LENIENT_BASE64: base64::engine::GeneralPurpose = base64::engine::GeneralPurpose::new(
    &base64::alphabet::STANDARD,
    base64::engine::GeneralPurposeConfig::new()
        .with_decode_padding_mode(base64::engine::DecodePaddingMode::Indifferent)
        .with_decode_allow_trailing_bits(true),
);

/// Undoes only the transfer encoding. The result is still in the part's own
/// charset — `decode_part_content` is what turns it into text.
fn decode_part_bytes(raw: &[u8], transfer_encoding: &str) -> Vec<u8> {
    match transfer_encoding.trim().to_ascii_lowercase().as_str() {
        "base64" => {
            let compact = raw
                .iter()
                .copied()
                .filter(|byte| !byte.is_ascii_whitespace())
                .collect::<Vec<u8>>();
            match LENIENT_BASE64.decode(&compact) {
                Ok(decoded) => decoded,
                Err(error) => decode_base64_prefix(&compact, &error),
            }
        }
        "quoted-printable" => decode_quoted_printable_bytes(raw),
        _ => raw.to_vec(),
    }
}

/// Keep the longest decodable prefix of a base64 part.
///
/// Whatever the sender broke sits at one offset; everything before it is real
/// content. Returning nothing is the one answer that is certainly wrong.
///
/// The cut is the earlier of the first byte outside the alphabet and the
/// offset the decoder named — the decoder reports the end of the chunk it
/// choked on, which is further along than the byte that spoiled it.
fn decode_base64_prefix(compact: &[u8], error: &base64::DecodeError) -> Vec<u8> {
    let reported = match *error {
        base64::DecodeError::InvalidByte(offset, _)
        | base64::DecodeError::InvalidLastSymbol(offset, _)
        | base64::DecodeError::InvalidLength(offset) => offset,
        base64::DecodeError::InvalidPadding => compact.len(),
    };
    let first_foreign = compact
        .iter()
        .position(
            |byte| !matches!(byte, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'+' | b'/' | b'='),
        )
        .unwrap_or(compact.len());
    let cut = reported.min(first_foreign).min(compact.len());
    // A single leftover symbol carries no whole byte.
    let usable = if cut % 4 == 1 { cut - 1 } else { cut };
    let decoded = LENIENT_BASE64
        .decode(&compact[..usable])
        .or_else(|_| LENIENT_BASE64.decode(&compact[..usable / 4 * 4]))
        .unwrap_or_default();
    tracing::warn!(
        cut = usable,
        encoded = compact.len(),
        decoded = decoded.len(),
        "base64 part did not decode in full: {error}"
    );
    decoded
}

fn normalize_body_text(input: &str) -> String {
    let mut text = input.replace("\r\n", "\n").replace('\r', "\n");
    text = text.replace('\0', "");

    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    trimmed.to_string()
}

/// Bodies stored by versions with the old MIME normaliser can be recognised
/// without guessing from their markup: the parser cut the decoded part at this
/// exact byte boundary. Keep the cached prefix until a replacement fetch has
/// succeeded, but do not mistake it for a complete body.
pub fn is_legacy_truncated_body(body: Option<&str>) -> bool {
    // The old truncation stepped back to the preceding character boundary, so a
    // multibyte character at the cutoff can leave up to three bytes fewer.
    body.is_some_and(|content| (64 * 1024 - 3..=64 * 1024).contains(&content.len()))
}

pub fn build_snippet(subject: &str, body_text: Option<&str>, body_html: Option<&str>) -> String {
    // The snippet feeds the thread list and the new-mail notification, whose
    // body renders as `subject\nsnippet`. Stopping at the first line break
    // would show nothing but the salutation ("Hallo zusammen,") for the plain
    // mails people actually write, so whitespace — including newlines —
    // collapses and the snippet runs on into the real content.
    const MAX_SNIPPET: usize = 220;

    let snippet_from = |text: &str| -> Option<String> {
        let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if collapsed.is_empty() {
            return None;
        }
        let mut snippet: String = collapsed.chars().take(MAX_SNIPPET).collect();
        if collapsed.chars().count() > MAX_SNIPPET {
            snippet.push('…');
        }
        Some(snippet)
    };

    if let Some(body) = body_text {
        if let Some(snippet) = snippet_from(body) {
            return snippet;
        }
    }

    if let Some(html) = body_html {
        if let Some(snippet) = snippet_from(&html_snippet_text(html)) {
            return snippet;
        }
    }

    subject.trim().to_string()
}

fn html_snippet_text(html: &str) -> String {
    // Stripping only the tags leaves the contents of non-visible elements such
    // as <style>, <script> and <title> behind. That CSS then becomes the stored
    // snippet and, for new mail, the native notification preview. Ammonia's
    // default clean-content set removes style/script contents; title is added
    // explicitly because a document title is not part of the message body.
    let visible_html = ammonia::Builder::default()
        .add_clean_content_tags(&["title"])
        .clean(html)
        .to_string();
    crate::email::text::strip_html_tags(&visible_html)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// How far ahead of the ingest a `Date` header may point before it stops
/// counting as evidence.
///
/// Clock skew of a few hours is ordinary. A year is not: an unclamped future
/// date pins its thread to the top of every list and, through the
/// `last_message_date >= …` guard of the thread projection, blocks every later
/// message from updating that thread's subject and snippet.
const FUTURE_DATE_TOLERANCE: chrono::TimeDelta = chrono::TimeDelta::hours(24);

/// The date a message is filed under.
///
/// `server_timestamp` is the provider's own receive time — IMAP INTERNALDATE;
/// POP3 and file imports have none. Without a readable `Date` header and
/// without it the epoch is used: the only substitute that is identical on
/// every refetch and never sorts a message above mail that really is new.
/// Beyond the tolerance the header says nothing, and `now` — when this run saw
/// the message — is what is left.
pub fn parse_message_date(
    header: &str,
    server_timestamp: Option<chrono::DateTime<chrono::Utc>>,
    now: chrono::DateTime<chrono::Utc>,
) -> String {
    let from_header = parse_header_value(header, "Date").and_then(|raw| {
        let cleaned = strip_parenthesized_comments(raw.trim());
        chrono::DateTime::parse_from_rfc2822(&cleaned)
            .or_else(|_| chrono::DateTime::parse_from_rfc2822(raw.trim()))
            .or_else(|_| chrono::DateTime::parse_from_rfc3339(&cleaned))
            .or_else(|_| chrono::DateTime::parse_from_rfc3339(raw.trim()))
            .ok()
            .map(|value| value.with_timezone(&chrono::Utc))
    });

    let resolved = from_header
        .or(server_timestamp)
        .unwrap_or(chrono::DateTime::UNIX_EPOCH);
    let filed = if resolved > now + FUTURE_DATE_TOLERANCE {
        now
    } else {
        resolved
    };
    filed.to_rfc3339()
}

/// Whether the second MIME parser could add anything to the first one's result.
///
/// The fallback re-parses the whole message, and `merge_extracted_imap_fallback`
/// only ever fills a gap: a missing text or HTML body (not on a root crypto
/// part) or an empty attachment list. Without a gap its result would be
/// thrown away.
fn extracted_fallback_could_help(parsed: &ParsedBody) -> bool {
    parsed.attachments.is_empty()
        || (!parsed.trust.root_crypto && (parsed.text.is_none() || parsed.html.is_none()))
}

/// One `UID FETCH (UID FLAGS INTERNALDATE BODY.PEEK[HEADER] BODY.PEEK[TEXT])`
/// entry as an envelope. `body` is the entry's `BODY[TEXT]` or, for a backfill
/// that fetched bodies separately, what that fetch returned.
fn envelope_from_fetch(
    entry: &async_imap::types::Fetch,
    body: Option<&[u8]>,
    uid: u32,
    context: &EnvelopeContext,
) -> MessageEnvelope {
    let remote_id = uid.to_string();
    let folder = context.folder.as_str();
    let server_uidvalidity = context.uidvalidity;
    let ingested_at = context.ingested_at;
    let receiver = context.receiver.as_str();

    let header = entry
        .header()
        .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
        .unwrap_or_default();

    let subject = parse_header_value(&header, "Subject").unwrap_or_else(|| "(no subject)".into());
    let mut parsed_body = parse_message_body(&header, body);
    if extracted_fallback_could_help(&parsed_body) {
        merge_extracted_imap_fallback(
            &mut parsed_body,
            parse_extracted_imap_email(
                &header,
                body,
                context.source_id.clone(),
                folder,
                server_uidvalidity,
                uid,
            ),
        );
    }
    let attachments = parsed_body.attachments.clone();
    let snippet = build_snippet(
        &subject,
        parsed_body.text.as_deref(),
        parsed_body.html.as_deref(),
    );
    let from_address = parse_from_address(&header).unwrap_or_else(|| "unknown@imap.local".into());
    let from_name = parse_from_name(&header);
    let to_addresses = parse_header_address_list(&header, "To");
    let cc_addresses = parse_header_address_list(&header, "Cc");
    let bcc_addresses = parse_header_address_list(&header, "Bcc");
    let reply_to =
        parse_header_value(&header, "Reply-To").and_then(|raw| extract_email_address(&raw));
    let message_id_header = parse_header_value(&header, "Message-ID");
    let remote_draft_id = message_id_header
        .clone()
        .map(|id| encode_imap_draft_ref(folder, &id));
    let in_reply_to = parse_header_value(&header, "In-Reply-To");
    let references = parse_header_value(&header, "References")
        .map(|raw| split_references(&raw))
        .unwrap_or_default();
    let date_rfc3339 = parse_message_date(&header, entry_internal_date(entry), ingested_at);
    let is_read = entry
        .flags()
        .any(|flag| matches!(flag, async_imap::types::Flag::Seen));
    let is_starred = entry
        .flags()
        .any(|flag| matches!(flag, async_imap::types::Flag::Flagged));

    let list_unsubscribe = parse_header_value(&header, "List-Unsubscribe");
    let list_unsubscribe_post = parse_header_value(&header, "List-Unsubscribe-Post");
    let (auth_spf, auth_dkim, auth_dmarc) = parse_header_value(&header, "Authentication-Results")
        .map(|value| crate::providers::traits::trusted_authentication_results(&value, receiver))
        .unwrap_or((None, None, None));
    MessageEnvelope {
        remote_id,
        remote_uidvalidity: Some(server_uidvalidity),
        remote_thread_id: None,
        original_source: None,
        mailbox_ids: Vec::new(),
        message_id_header,
        in_reply_to,
        references,
        subject,
        snippet,
        from_address,
        from_name,
        to_addresses,
        cc_addresses,
        bcc_addresses,
        reply_to,
        remote_draft_id,
        received_at_rfc3339: entry_internal_date(entry).map(|date| date.to_rfc3339()),
        date_rfc3339,
        body_text: parsed_body.text,
        body_html: parsed_body.html,
        is_read,
        is_starred,
        body_calendar: parsed_body.calendar,
        smime_encrypted: parsed_body.smime_encrypted,
        smime_signed: parsed_body.smime_signed,
        pgp_signed: parsed_body.pgp_signed,
        trust: super::traits::MessageTrust {
            auth_verified: auth_spf.is_some() || auth_dkim.is_some() || auth_dmarc.is_some(),
            ..parsed_body.trust
        },
        list_unsubscribe,
        list_unsubscribe_post,
        auth_spf,
        auth_dkim,
        auth_dmarc,
        attachments,
    }
}

/// The server's own receive time for a fetched message, if it reported one.
fn entry_internal_date(entry: &async_imap::types::Fetch) -> Option<chrono::DateTime<chrono::Utc>> {
    entry
        .internal_date()
        .map(|value| value.with_timezone(&chrono::Utc))
}

fn strip_parenthesized_comments(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut depth = 0u32;

    for ch in input.chars() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(ch),
            _ => {}
        }
    }

    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn a_fetch_stalled_mid_response_hits_the_operation_deadline() {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let (reader, mut writer) = socket.into_split();
            let mut reader = tokio::io::BufReader::new(reader);
            writer.write_all(b"* OK ready\r\n").await.unwrap();
            let mut line = String::new();
            reader.read_line(&mut line).await.unwrap();
            let tag = line.split_whitespace().next().unwrap();
            writer
                .write_all(format!("{tag} OK logged in\r\n").as_bytes())
                .await
                .unwrap();
            line.clear();
            reader.read_line(&mut line).await.unwrap();
            assert!(line.contains("UID FETCH"));
            writer
                .write_all(b"* 1 FETCH (UID 1 BODY[TEXT] {100}\r\npartial")
                .await
                .unwrap();
            std::future::pending::<()>().await;
        });
        let socket = tokio::net::TcpStream::connect(address).await.unwrap();
        let mut client = async_imap::Client::new(socket);
        client.read_response().await.unwrap();
        let mut session = client.login("user", "password").await.unwrap();
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            super::collect_imap_response(
                std::time::Duration::from_millis(50),
                session.uid_fetch("1", "(UID BODY.PEEK[TEXT])"),
                "UID FETCH",
            ),
        )
        .await;
        server.abort();
        assert!(
            result.is_ok(),
            "response stream escaped the operation deadline"
        );
        assert!(result.unwrap().unwrap_err().contains("timed out"));
    }

    #[test]
    fn rejects_markup_in_attachment_mime_types() {
        let raw = b"Content-Type: multipart/related; boundary=x\r\n\r\n--x\r\nContent-Type: text/html\r\n\r\n<img src=\"cid:x\">\r\n--x\r\nContent-Type: image/png\"><link rel=preconnect href=https://u1.t.example>\r\nContent-ID: <x>\r\nContent-Disposition: inline; filename=logo.png\r\n\r\nAAAA\r\n--x--\r\n";
        let parsed = super::parse_raw_rfc822(raw).unwrap();
        assert_eq!(
            parsed.attachments[0].content_type,
            "application/octet-stream"
        );
    }

    #[test]
    fn queued_uid_mutation_requires_the_selected_mailbox_generation() {
        use super::{ImapProvider, MessageRef};

        let refs = vec![MessageRef {
            remote_id: "42".into(),
            source_folder: Some("INBOX".into()),
            source_uidvalidity: Some(17),
        }];
        let grouped = ImapProvider::message_refs_by_folder(&refs).expect("valid reference");
        let group = grouped.get("INBOX").expect("inbox group");
        assert_eq!(group.uids, vec![42]);
        let stale = group
            .validate_generation("INBOX", Some(18))
            .expect_err("changed generation");
        assert!(stale.is_permanent());
        assert!(stale.to_string().contains("imapUidValidityChanged"));
        let missing_generation = group
            .validate_generation("INBOX", None)
            .expect_err("missing generation");
        assert!(missing_generation.is_permanent());
        assert!(group.validate_generation("INBOX", Some(17)).is_ok());

        let mut missing = refs;
        missing[0].source_uidvalidity = None;
        assert!(ImapProvider::message_refs_by_folder(&missing)
            .expect_err("missing generation")
            .is_permanent());

        let mut mixed = missing;
        mixed[0].source_uidvalidity = Some(17);
        let mut reused_uid = mixed[0].clone();
        reused_uid.source_uidvalidity = Some(18);
        mixed.push(reused_uid);
        assert!(ImapProvider::message_refs_by_folder(&mixed)
            .expect_err("mixed generations")
            .is_permanent());
    }

    /// The two body parsers deliberately differ, and the reason is not taste.
    ///
    /// `parse_message_body` reads server-side MIME, where a part it leaves out
    /// is fetched later by `remote_part_id`. `parse_raw_message_body` reads a
    /// decrypted payload, whose attachments exist only in that plaintext — the
    /// server holds one encrypted part, not these. Sharing one rule would make
    /// an attachment on encrypted mail unopenable.
    #[test]
    fn the_raw_parser_keeps_bytes_the_server_does_not_have() {
        let raw = concat!(
            "Content-Type: multipart/mixed; boundary=\"b\"\r\n",
            "\r\n",
            "--b\r\n",
            "Content-Type: text/plain\r\n\r\n",
            "Hallo\r\n",
            "--b\r\n",
            "Content-Type: application/pdf\r\n",
            "Content-Disposition: attachment; filename=\"bericht.pdf\"\r\n\r\n",
            "PDFBYTES\r\n",
            "--b\r\n",
            "Content-Type: image/png\r\n",
            "Content-ID: <logo>\r\n",
            "Content-Disposition: inline; filename=\"logo.png\"\r\n\r\n",
            "PNGBYTES\r\n",
            "--b--\r\n",
        );

        let embedded = |parsed: &super::ParsedBody| {
            parsed
                .attachments
                .iter()
                .map(|attachment| {
                    (
                        attachment.content_type.to_ascii_lowercase(),
                        attachment.content_base64.is_some(),
                    )
                })
                .collect::<Vec<_>>()
        };

        let from_raw = super::parse_raw_message_body(raw.as_bytes());
        let (header, body) = super::split_header_and_body(raw.as_bytes());
        let from_parts = super::parse_message_body(&String::from_utf8_lossy(header), Some(body));

        assert_eq!(
            embedded(&from_raw),
            vec![
                ("application/pdf".to_string(), true),
                ("image/png".to_string(), true),
            ],
            "a decrypted attachment has nowhere else to come from"
        );
        assert_eq!(
            embedded(&from_parts),
            vec![
                ("application/pdf".to_string(), false),
                ("image/png".to_string(), true),
            ],
            "a server-side attachment is fetched on demand"
        );
    }

    /// Both policies, stated once each.
    #[test]
    fn the_embed_policies_say_what_they_mean() {
        use super::EmbedPolicy::{All, InlineAndInvites};
        let part = |content_type: &str, is_inline: bool| super::ParsedAttachment {
            remote_part_id: "1".into(),
            filename: Some("datei".into()),
            content_type: content_type.into(),
            content_id: None,
            is_inline,
            bytes: b"BYTES".to_vec(),
        };

        for content_type in ["image/png", "application/pdf", "text/calendar"] {
            for inline in [true, false] {
                assert!(
                    super::attachment_should_embed(&part(content_type, inline), All),
                    "{content_type} inline={inline}"
                );
            }
        }

        assert!(super::attachment_should_embed(
            &part("image/png", true),
            InlineAndInvites
        ));
        assert!(super::attachment_should_embed(
            &part("text/calendar; method=REQUEST", false),
            InlineAndInvites
        ));
        assert!(super::attachment_should_embed(
            &part("TEXT/CALENDAR", false),
            InlineAndInvites
        ));
        assert!(!super::attachment_should_embed(
            &part("application/pdf", false),
            InlineAndInvites
        ));
        assert!(!super::attachment_should_embed(
            &part("image/png", false),
            InlineAndInvites
        ));
    }

    use super::{
        build_snippet, cached_attachment_parts, decode_imap_draft_ref, encode_imap_draft_ref,
        inbox_scoped_folder_path, is_missing_namespace_error, is_server_rejection,
        login_error_text, namespace_retry_folder, parse_attachments_from_body,
        parse_extracted_imap_email, parse_message_body, parse_message_date, parse_raw_message_body,
        parse_raw_rfc822, plain_expunge_is_safe, store_attachment_parts,
        strip_parenthesized_comments, AttachmentCacheKey, ImapProvider, ATTACHMENT_PART_CACHE_TTL,
    };
    use base64::Engine as _;
    use postroom::SourceId;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, OnceLock};
    use std::time::{Duration, Instant};

    #[test]
    fn login_errors_tell_a_refusal_from_a_dropped_connection() {
        let refused = login_error_text(&async_imap::error::Error::No(
            "[AUTHENTICATIONFAILED] Invalid credentials".into(),
        ));
        assert!(refused.starts_with("imap login failed: "), "{refused}");
        let dropped = login_error_text(&async_imap::error::Error::ConnectionLost);
        assert!(dropped.starts_with("imap login interrupted: "), "{dropped}");
    }

    /// The part cache is process-wide, so the tests that measure its hits and
    /// misses take turns.
    fn cache_lock() -> &'static tokio::sync::Mutex<()> {
        static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
    }

    /// A `multipart/mixed` message with `parts` base64 attachments of
    /// `part_bytes` each.
    fn message_with_attachments(part_bytes: usize, parts: usize) -> (String, Vec<u8>) {
        let mut body = Vec::new();
        for index in 1..=parts {
            body.extend_from_slice(
                format!(
                    "--sep\r\nContent-Type: application/octet-stream\r\n\
                     Content-Disposition: attachment; filename=\"file-{index}.bin\"\r\n\
                     Content-Transfer-Encoding: base64\r\n\r\n"
                )
                .as_bytes(),
            );
            let payload = vec![b'x'; part_bytes];
            body.extend_from_slice(
                base64::engine::general_purpose::STANDARD
                    .encode(&payload)
                    .as_bytes(),
            );
            body.extend_from_slice(b"\r\n");
        }
        body.extend_from_slice(b"--sep--\r\n");
        ("multipart/mixed; boundary=\"sep\"".to_string(), body)
    }

    fn cache_key(remote_id: &str) -> AttachmentCacheKey {
        AttachmentCacheKey {
            account: "mail.example.test:993/user".to_string(),
            folder: "INBOX".to_string(),
            uidvalidity: Some(1),
            remote_id: remote_id.to_string(),
        }
    }

    /// Clicking five attachments used to download and decode the whole message
    /// five times, and throw four fifths of each download away.
    #[tokio::test]
    async fn opening_every_attachment_of_a_message_downloads_the_body_once() {
        let _guard = cache_lock().lock().await;
        let (content_type, body) = message_with_attachments(4 * 1024, 5);
        let key = cache_key("uid-42");
        let downloads = Arc::new(AtomicUsize::new(0));
        let decoded = Arc::new(AtomicUsize::new(0));

        for index in 1..=5 {
            let parts = ImapProvider::attachment_parts(&key, || {
                let downloads = downloads.clone();
                let decoded = decoded.clone();
                let content_type = content_type.clone();
                let body = body.clone();
                async move {
                    downloads.fetch_add(1, Ordering::SeqCst);
                    let parts = parse_attachments_from_body(&content_type, "8bit", &body);
                    decoded.fetch_add(
                        parts.iter().map(|part| part.bytes.len()).sum::<usize>(),
                        Ordering::SeqCst,
                    );
                    Ok(parts)
                }
            })
            .await
            .expect("parts resolve");
            assert!(parts
                .iter()
                .any(|part| part.remote_part_id == format!("mime-{index}")));
        }

        assert_eq!(
            downloads.load(Ordering::SeqCst),
            1,
            "five attachments, one body fetch"
        );
        assert_eq!(
            decoded.load(Ordering::SeqCst),
            5 * 4 * 1024,
            "each part is decoded once per fetch, not once per click"
        );
    }

    /// The same guarantee the sync path relies on: ten inline images of one
    /// message cost one body fetch, not ten.
    #[tokio::test]
    async fn ten_inline_parts_of_one_message_cost_one_download() {
        let _guard = cache_lock().lock().await;
        let (content_type, body) = message_with_attachments(512, 10);
        let key = cache_key("uid-inline");
        let downloads = Arc::new(AtomicUsize::new(0));

        for _ in 0..10 {
            ImapProvider::attachment_parts(&key, || {
                let downloads = downloads.clone();
                let content_type = content_type.clone();
                let body = body.clone();
                async move {
                    downloads.fetch_add(1, Ordering::SeqCst);
                    Ok(parse_attachments_from_body(&content_type, "8bit", &body))
                }
            })
            .await
            .expect("parts resolve");
        }

        assert_eq!(downloads.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn cached_parts_expire_so_a_reopened_message_is_refetched() {
        let _guard = cache_lock().blocking_lock();
        let (content_type, body) = message_with_attachments(64, 2);
        let key = cache_key("uid-stale");
        let parts = Arc::new(parse_attachments_from_body(&content_type, "8bit", &body));
        let stored_at = Instant::now();
        store_attachment_parts(&key, &parts, stored_at);

        assert!(cached_attachment_parts(&key, stored_at).is_some());
        assert!(
            cached_attachment_parts(
                &key,
                stored_at + ATTACHMENT_PART_CACHE_TTL + Duration::from_secs(1)
            )
            .is_none(),
            "a message nobody has touched must not be held forever"
        );
    }

    /// Measurement, not an assertion. Run it with
    /// `cargo test -p satchel-core --lib -- --ignored --nocapture attachment_fetch_cost`.
    #[tokio::test]
    #[ignore = "measurement: builds a 50 MB message"]
    async fn attachment_fetch_cost_for_a_message_with_five_ten_megabyte_parts() {
        let part_bytes = 10 * 1024 * 1024;
        let (content_type, body) = message_with_attachments(part_bytes, 5);
        println!(
            "message body on the wire: {} bytes ({} MiB base64)",
            body.len(),
            body.len() / (1024 * 1024)
        );

        let before = Instant::now();
        let mut before_wire = 0usize;
        let mut before_decoded = 0usize;
        for index in 1..=5 {
            before_wire += body.len();
            let parts = parse_attachments_from_body(&content_type, "8bit", &body);
            before_decoded += parts.iter().map(|part| part.bytes.len()).sum::<usize>();
            assert!(parts
                .iter()
                .any(|part| part.remote_part_id == format!("mime-{index}")));
        }
        let before_elapsed = before.elapsed();

        let _guard = cache_lock().lock().await;
        let key = cache_key("uid-measure");
        let after = Instant::now();
        let after_wire = Arc::new(AtomicUsize::new(0));
        let after_decoded = Arc::new(AtomicUsize::new(0));
        for index in 1..=5 {
            let parts = ImapProvider::attachment_parts(&key, || {
                let after_wire = after_wire.clone();
                let after_decoded = after_decoded.clone();
                let content_type = content_type.clone();
                let body = body.clone();
                async move {
                    after_wire.fetch_add(body.len(), Ordering::SeqCst);
                    let parts = parse_attachments_from_body(&content_type, "8bit", &body);
                    after_decoded.fetch_add(
                        parts.iter().map(|part| part.bytes.len()).sum::<usize>(),
                        Ordering::SeqCst,
                    );
                    Ok(parts)
                }
            })
            .await
            .expect("parts resolve");
            assert!(parts
                .iter()
                .any(|part| part.remote_part_id == format!("mime-{index}")));
        }
        let after_elapsed = after.elapsed();

        println!("opening all five attachments");
        println!(
            "  before: {before_wire} bytes fetched, {before_decoded} bytes decoded, {before_elapsed:?}"
        );
        println!(
            "  after:  {} bytes fetched, {} bytes decoded, {after_elapsed:?}",
            after_wire.load(Ordering::SeqCst),
            after_decoded.load(Ordering::SeqCst)
        );
    }

    /// A plain EXPUNGE takes every \Deleted message of the mailbox with it, so
    /// it may only run while the mailbox holds nothing but our own.
    #[test]
    fn a_foreign_deleted_message_makes_the_plain_expunge_unsafe() {
        assert!(!plain_expunge_is_safe(
            &std::collections::HashSet::from([42, 99]),
            &[42]
        ));
        assert!(plain_expunge_is_safe(
            &std::collections::HashSet::from([42]),
            &[42]
        ));
        // A UID another client already expunged is simply gone; removing the
        // rest still touches nothing foreign.
        assert!(plain_expunge_is_safe(
            &std::collections::HashSet::new(),
            &[42, 43]
        ));
    }

    /// Only a declined command may be answered with a fallback. After a broken
    /// exchange the server may have applied part of it.
    #[test]
    fn only_a_declined_command_earns_a_fallback() {
        assert!(is_server_rejection(&async_imap::error::Error::No(
            "not supported".into()
        )));
        assert!(is_server_rejection(&async_imap::error::Error::Bad(
            "unknown command".into()
        )));
        assert!(!is_server_rejection(
            &async_imap::error::Error::ConnectionLost
        ));
        assert!(!is_server_rejection(&async_imap::error::Error::Io(
            std::io::Error::other("socket")
        )));
    }

    #[test]
    fn trycreate_is_read_from_the_response_code() {
        assert!(super::is_trycreate_response(
            "imap APPEND Sent failed: No Response: [TRYCREATE] Mailbox doesn't exist"
        ));
        assert!(super::is_trycreate_response(
            "NO [trycreate] no such mailbox"
        ));
        assert!(!super::is_trycreate_response(
            "NO [OVERQUOTA] Quota exceeded"
        ));
        // The word alone, outside a response code, is not the token.
        assert!(!super::is_trycreate_response("NO please trycreate later"));
    }

    #[test]
    fn imap_draft_identity_keeps_folder_and_stable_message_id() {
        let encoded = encode_imap_draft_ref("INBOX.Entwürfe", "<draft@example.com>");
        let decoded = decode_imap_draft_ref(&encoded).expect("draft ref");
        assert_eq!(decoded.folder, "INBOX.Entwürfe");
        assert_eq!(decoded.id, "<draft@example.com>");
    }

    #[tokio::test]
    async fn legacy_numeric_draft_ref_cannot_delete_a_reused_uid() {
        use crate::providers::traits::MailProvider;

        let provider = ImapProvider::default();
        for reference in ["42".to_string(), encode_imap_draft_ref("Drafts", "42")] {
            let error = provider
                .delete_draft(&reference)
                .await
                .expect_err("numeric UID has no generation");
            assert!(error.is_permanent());
            assert!(error.to_string().contains("imapDraftUidWithoutValidity"));
        }
        assert!(provider
            .resolve_draft_id("42", Some("Drafts"))
            .await
            .expect("unversioned UID cannot resolve")
            .is_none());
    }

    #[test]
    fn retries_namespace_rejections_below_inbox_with_server_delimiter() {
        assert!(is_missing_namespace_error(
            "Client tried to access nonexistent namespace. (Mailbox name should probably be prefixed with: INBOX.)"
        ));
        assert_eq!(
            inbox_scoped_folder_path("INBOX", Some("."), "Aside"),
            Some("INBOX.Aside".to_string())
        );
        assert_eq!(
            inbox_scoped_folder_path("INBOX", Some("/"), "Aside"),
            Some("INBOX/Aside".to_string())
        );
        assert_eq!(
            inbox_scoped_folder_path("INBOX", Some("."), "INBOX.Already Scoped"),
            None
        );
    }

    #[test]
    fn draft_folder_retry_requires_an_explicit_namespace_rejection() {
        let rejection = "imap SELECT Work In Progress failed: no response: code: None, info: Some(\"Client tried to access nonexistent namespace. (Mailbox name should probably be prefixed with: INBOX.)\")";
        assert_eq!(
            namespace_retry_folder(rejection, "INBOX", Some("."), "Work In Progress"),
            Some("INBOX.Work In Progress".to_string())
        );
        assert_eq!(
            namespace_retry_folder(
                "imap SELECT Work In Progress failed: nonexistent namespace",
                "INBOX",
                Some("."),
                "Work In Progress",
            ),
            None
        );
    }

    #[test]
    fn extracted_imap_fallback_parses_standard_rfc822_body() {
        let header = "From: Alice <alice@example.com>\r\nTo: Bob <bob@example.com>\r\nSubject: Test\r\nContent-Type: text/plain; charset=utf-8\r\n";
        let parsed = parse_extracted_imap_email(
            header,
            Some(b"Hello from extracted MIME"),
            SourceId::new("imap/account-1"),
            "INBOX",
            77,
            42,
        )
        .expect("extracted parser");

        assert_eq!(parsed.text.as_deref(), Some("Hello from extracted MIME"));
        assert!(parsed.attachments.is_empty());
    }

    #[test]
    fn snippet_runs_past_the_salutation_into_the_body() {
        let body = "Hallo zusammen,\r\n\r\nfolgend findet ihr den wöchentlichen Report über die Supportanfragen, die von der Plattform automatisch erzeugt wurde.";

        assert_eq!(
            build_snippet("Weekly Report", Some(body), None),
            "Hallo zusammen, folgend findet ihr den wöchentlichen Report über die Supportanfragen, die von der Plattform automatisch erzeugt wurde."
        );
    }

    #[test]
    fn long_snippet_is_cut_at_a_character_boundary_with_an_ellipsis() {
        let body = format!("{} {}", "wort ".repeat(80), "ende");

        let snippet = build_snippet("Betreff", Some(&body), None);
        assert!(snippet.ends_with('…'));
        assert!(snippet.chars().count() <= 221);
        assert!(!snippet.ends_with('\u{fffd}'));
    }

    #[test]
    fn html_snippet_omits_css_and_other_non_visible_content() {
        let html = r#"
            <html>
                <head>
                    <title>Duplicate subject</title>
                    <style>/* Reset */ * { box-sizing: border-box; margin: 0; }</style>
                    <script>trackOpen()</script>
                </head>
                <body><p>Antwort gespeichert — DFN-Terminplaner</p></body>
            </html>
        "#;

        assert_eq!(
            build_snippet("Antwort abgegeben", None, Some(html)),
            "Antwort gespeichert — DFN-Terminplaner"
        );
        assert_eq!(
            build_snippet(
                "Antwort abgegeben",
                None,
                Some("<style>* { margin: 0; }</style>")
            ),
            "Antwort abgegeben"
        );
    }

    #[test]
    fn strips_date_comments() {
        let value = "Fri, 16 Feb 2024 10:11:12 +0100 (CET)";
        assert_eq!(
            strip_parenthesized_comments(value),
            "Fri, 16 Feb 2024 10:11:12 +0100"
        );
    }

    #[test]
    fn parses_rfc2822_date_header() {
        let header = "Date: Fri, 16 Feb 2024 10:11:12 +0100\r\nSubject: Test\r\n";
        assert_eq!(
            parse_message_date(header, None, chrono::Utc::now()),
            "2024-02-16T09:11:12+00:00".to_string()
        );
    }

    #[test]
    fn a_missing_date_falls_back_to_the_internal_date() {
        let internal = chrono::DateTime::parse_from_rfc3339("2019-04-02T08:00:00Z")
            .expect("internal date")
            .with_timezone(&chrono::Utc);
        assert_eq!(
            parse_message_date("Subject: Test\r\n", Some(internal), chrono::Utc::now()),
            internal.to_rfc3339()
        );
    }

    #[test]
    fn without_any_timestamp_the_result_is_stable() {
        let first = parse_message_date("Subject: Test\r\n", None, chrono::Utc::now());
        let second = parse_message_date("Subject: Test\r\n", None, chrono::Utc::now());
        assert_eq!(first, second);
        assert_eq!(first, chrono::DateTime::UNIX_EPOCH.to_rfc3339());
    }

    #[test]
    fn a_far_future_date_is_clamped_to_the_ingest_time() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-09-09T12:00:00Z")
            .expect("now")
            .with_timezone(&chrono::Utc);
        assert_eq!(
            parse_message_date("Date: Fri, 1 Jan 2038 00:00:00 +0000\r\n", None, now),
            now.to_rfc3339()
        );

        // Inside the tolerance the header is kept: clocks drift.
        let skewed = "Date: Wed, 9 Sep 2026 20:00:00 +0000\r\n";
        assert_eq!(
            parse_message_date(skewed, None, now),
            "2026-09-09T20:00:00+00:00".to_string()
        );
    }

    #[test]
    fn parses_nested_multipart_plain_text() {
        let header = "Content-Type: multipart/mixed; boundary=\"----=_NextPart_000_033D_01DC9C13.821D4420\"\r\n";
        let raw = "\
This is a multipart message in MIME format.\r\n\
\r\n\
------=_NextPart_000_033D_01DC9C13.821D4420\r\n\
Content-Type: multipart/alternative;\r\n\
\tboundary=\"----=_NextPart_001_033E_01DC9C13.821D6B30\"\r\n\
\r\n\
------=_NextPart_001_033E_01DC9C13.821D6B30\r\n\
Content-Type: text/plain; charset=\"utf-8\"\r\n\
Content-Transfer-Encoding: quoted-printable\r\n\
\r\n\
Hallo Alex,=0D=0A=0D=0Ader Zugriff funktioniert jetzt wieder.=0D=0A\r\n\
------=_NextPart_001_033E_01DC9C13.821D6B30--\r\n\
\r\n\
------=_NextPart_000_033D_01DC9C13.821D4420--\r\n";

        let parsed = parse_message_body(header, Some(raw.as_bytes()));
        assert_eq!(
            parsed.text,
            Some("Hallo Alex,\n\nder Zugriff funktioniert jetzt wieder.".to_string())
        );
        assert_eq!(parsed.html, None);
    }

    fn embed_under_attacker_html(entity: &[u8]) -> Vec<u8> {
        let mut raw = b"From: Bob <bob@bank.example>\r\nContent-Type: multipart/mixed; boundary=attack\r\n\r\n--attack\r\nContent-Type: text/html\r\n\r\n<p>Please wire EUR 10000 to IBAN X</p>\r\n--attack\r\n".to_vec();
        raw.extend_from_slice(entity);
        raw.extend_from_slice(b"\r\n--attack--\r\n");
        raw
    }

    #[test]
    fn nested_signed_entities_never_cover_attacker_html() {
        for protocol in ["application/pgp-signature", "application/pkcs7-signature"] {
            let entity = format!("Content-Type: multipart/signed; protocol=\"{protocol}\"; boundary=signed\r\n\r\n--signed\r\nContent-Type: text/plain\r\n\r\nOriginal signed body\r\n--signed\r\nContent-Type: {protocol}\r\n\r\nCaptured signature\r\n--signed--\r\n");
            let root = parse_raw_message_body(entity.as_bytes());
            assert!(root.trust.root_crypto);
            assert!(!root.trust.root_encryption);
            let attack = parse_raw_message_body(&embed_under_attacker_html(entity.as_bytes()));
            assert!(attack.html.as_deref().unwrap().contains("IBAN X"));
            assert!(attack.smime_signed.is_some() || attack.pgp_signed.is_some());
            assert!(!attack.trust.root_crypto);
            assert!(!attack.trust.root_encryption);
        }
    }

    #[test]
    fn captured_nested_ciphertexts_never_enable_automatic_decryption() {
        let armor = "-----BEGIN PGP MESSAGE-----\nCaptured ciphertext\n-----END PGP MESSAGE-----";
        let pgp = crate::email::pgp_mime::build_encrypted(
            "From: bob@bank.example\r\nContent-Type: text/plain\r\n\r\n",
            armor,
        );
        let smime = "Content-Type: application/pkcs7-mime; smime-type=enveloped-data\r\nContent-Transfer-Encoding: base64\r\n\r\nAQID";
        for entity in [pgp.as_bytes(), smime.as_bytes()] {
            assert!(parse_raw_message_body(entity).trust.root_encryption);
            let attack = parse_raw_message_body(&embed_under_attacker_html(entity));
            assert!(!attack.trust.root_encryption);
            assert!(attack.smime_encrypted.is_none());
            assert!(attack.html.as_deref().unwrap().contains("IBAN X"));
        }
        let inline = format!("Content-Type: text/plain\r\n\r\n{armor}");
        assert!(
            !parse_raw_message_body(&embed_under_attacker_html(inline.as_bytes()))
                .trust
                .root_encryption
        );
        let forwarded = format!("Content-Type: message/rfc822\r\n\r\n{inline}");
        assert!(
            !parse_raw_message_body(forwarded.as_bytes())
                .trust
                .root_encryption
        );
    }

    #[test]
    fn root_signature_does_not_enable_decryption_of_its_ciphertext_body() {
        let raw = "Content-Type: multipart/signed; protocol=\"application/pgp-signature\"; boundary=s\r\n\r\n--s\r\nContent-Type: text/plain\r\n\r\n-----BEGIN PGP MESSAGE-----\nCaptured ciphertext\n-----END PGP MESSAGE-----\r\n--s\r\nContent-Type: application/pgp-signature\r\n\r\nCaptured signature\r\n--s--\r\n";
        let parsed = parse_raw_message_body(raw.as_bytes());
        assert!(parsed.trust.root_crypto);
        assert!(!parsed.trust.root_encryption);
        assert!(!parsed.trust.root_inline_signature);
    }

    #[test]
    fn unsigned_alternative_does_not_inherit_another_alternatives_signature() {
        let signed = b"Content-Type: multipart/signed; protocol=\"application/pgp-signature\"; boundary=s\r\n\r\n--s\r\nContent-Type: text/plain\r\n\r\nOriginal\r\n--s\r\nContent-Type: application/pgp-signature\r\n\r\nSignature\r\n--s--\r\n";
        let attack = embed_under_attacker_html(signed);
        let attack = String::from_utf8(attack)
            .unwrap()
            .replace("multipart/mixed", "multipart/alternative");
        assert!(!parse_raw_message_body(attack.as_bytes()).trust.root_crypto);
    }

    #[test]
    fn preserves_rfc3156_multipart_signed_as_raw_bytes() {
        let content_type =
            "multipart/signed; protocol=\"application/pgp-signature\"; boundary=\"pgp\"";
        let header = format!("Content-Type: {content_type}\r\n");
        let mut raw = b"--pgp\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
Signed body\r\n\
--pgp\r\n\
Content-Type: application/pgp-signature\r\n\
Content-Disposition: attachment; filename=\"signature.asc\"\r\n\
\r\n\
"
        .to_vec();
        raw.extend_from_slice(&[0xff, 0x00, 0x7f]);
        raw.extend_from_slice(b"\r\n--pgp--\r\n");

        let parsed = parse_message_body(&header, Some(&raw));
        let mut expected = format!("Content-Type: {content_type}\r\n\r\n").into_bytes();
        expected.extend_from_slice(&raw);

        assert_eq!(parsed.pgp_signed, Some(expected));
        assert!(parsed.smime_signed.is_none());
        assert_eq!(parsed.text.as_deref(), Some("Signed body"));
        assert!(parsed.attachments.is_empty());
    }

    #[test]
    fn preserves_legacy_smime_multipart_signed_as_raw_bytes() {
        let content_type = "multipart/signed; micalg=sha-256; \
            protocol=\"application/x-pkcs7-signature\"; boundary=\"smime\"";
        let header = format!("Content-Type: {content_type}\r\n");
        let raw = b"--smime\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
Signed body\r\n\
--smime\r\n\
Content-Type: application/x-pkcs7-signature; name=smime.p7s\r\n\
Content-Transfer-Encoding: base64\r\n\
Content-Disposition: attachment; filename=smime.p7s\r\n\
\r\n\
AQID\r\n\
--smime--\r\n";

        let parsed = parse_message_body(&header, Some(raw));
        let mut expected = format!("Content-Type: {content_type}\r\n\r\n").into_bytes();
        expected.extend_from_slice(raw);

        assert_eq!(parsed.smime_signed, Some(expected));
        assert!(parsed.pgp_signed.is_none());
        assert_eq!(parsed.text.as_deref(), Some("Signed body"));
        assert!(parsed.attachments.is_empty());
    }

    #[test]
    fn a_signed_structure_the_stored_bundle_cannot_reproduce_shows_nothing() {
        let body = b"--s\r\nContent-Type: text/plain\r\n\r\nSigned body\r\n\
--s\r\nContent-Type: application/pgp-signature\r\n\r\nSignature\r\n--s--\r\n";
        let plain = "multipart/signed; protocol=\"application/pgp-signature\"; boundary=s";
        assert!(
            parse_message_body(&format!("Content-Type: {plain}\r\n"), Some(body))
                .pgp_signed
                .is_some()
        );

        // Decoded once for display, the type gains line breaks; written into
        // the bundle and read back, it would name a different structure.
        let header = format!("Content-Type: {plain}; x=\"=?us-ascii?q?=0D=0A=0D=0A?=\"\r\n");
        let parsed = parse_message_body(&header, Some(body));
        assert!(parsed.pgp_signed.is_none());
        assert!(parsed.text.is_none());
        assert!(!parsed.trust.root_crypto);
    }

    #[test]
    fn raw_message_parser_embeds_decrypted_attachments() {
        let raw = b"Content-Type: multipart/mixed; boundary=inner\r\n\r\n\
--inner\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nHello\r\n\
--inner\r\nContent-Type: application/octet-stream\r\n\
Content-Disposition: attachment; filename=report.bin\r\n\
Content-Transfer-Encoding: base64\r\n\r\nAQID\r\n--inner--\r\n";

        let parsed = parse_raw_message_body(raw);
        assert_eq!(parsed.text.as_deref(), Some("Hello"));
        assert_eq!(parsed.attachments.len(), 1);
        assert_eq!(
            parsed.attachments[0].filename.as_deref(),
            Some("report.bin")
        );
        assert_eq!(
            parsed.attachments[0].content_base64.as_deref(),
            Some("AQID")
        );
    }

    #[test]
    fn parses_message_rfc822_part() {
        let header = "Content-Type: message/rfc822\r\n";
        let raw = "\
Subject: Embedded\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
Content-Transfer-Encoding: quoted-printable\r\n\
\r\n\
Zeile=201\r\n";

        let parsed = parse_message_body(header, Some(raw.as_bytes()));
        assert_eq!(parsed.text, Some("Zeile 1".to_string()));
    }

    #[test]
    fn parses_leading_boundary_body_when_top_level_header_is_missing() {
        let raw = "\
--outer_123\r\n\
Content-Type: multipart/alternative; boundary=\"alt_456\"\r\n\
\r\n\
--alt_456\r\n\
Content-Type: text/plain; charset=\"utf-8\"\r\n\
Content-Transfer-Encoding: quoted-printable\r\n\
\r\n\
Hi Tim,=0D=0A=0D=0Awie besprochen hier zusammengef=C3=BChrt.\r\n\
--alt_456\r\n\
Content-Type: text/html; charset=\"utf-8\"\r\n\
Content-Transfer-Encoding: quoted-printable\r\n\
\r\n\
<html><body><p>Hi Tim,</p><p>wie besprochen hier zusammengef=C3=BChrt.</p></body></html>\r\n\
--alt_456--\r\n\
--outer_123--\r\n";

        let parsed = parse_message_body("", Some(raw.as_bytes()));
        assert_eq!(
            parsed.text.as_deref(),
            Some("Hi Tim,\n\nwie besprochen hier zusammengeführt.")
        );
        assert_eq!(
            parsed.html.as_deref(),
            Some("<html><body><p>Hi Tim,</p><p>wie besprochen hier zusammengeführt.</p></body></html>")
        );
    }

    #[test]
    fn parses_leading_boundary_gmail_body_with_base64_and_attachment() {
        let raw = "\
--outer\r\n\
Content-Type: multipart/alternative; boundary=\"alt\"\r\n\
\r\n\
--alt\r\n\
Content-Type: text/plain; charset=\"UTF-8\"; format=flowed; delsp=yes\r\n\
Content-Transfer-Encoding: base64\r\n\
\r\n\
R29vZ2xlIFBsYXkNCg0KRGV2ZWxvcGVyIHVwZGF0ZQ0KDQpIaSBEZXZlbG9wZXJz\r\n\
--alt\r\n\
Content-Type: text/html; charset=\"UTF-8\"\r\n\
Content-Transfer-Encoding: base64\r\n\
\r\n\
PGRpdj5Hb29nbGUgUGxheTwvZGl2PjxwPkRldmVsb3BlciB1cGRhdGU8L3A+\r\n\
--alt--\r\n\
--outer\r\n\
Content-Type: application/pdf; name=\"policy.pdf\"\r\n\
Content-Disposition: attachment; filename=\"policy.pdf\"\r\n\
Content-Transfer-Encoding: base64\r\n\
\r\n\
JVBERi0xLjQK\r\n\
--outer--\r\n";

        let parsed = parse_message_body("", Some(raw.as_bytes()));
        assert_eq!(
            parsed.text.as_deref(),
            Some("Google Play\n\nDeveloper update\n\nHi Developers")
        );
        assert_eq!(
            parsed.html.as_deref(),
            Some("<div>Google Play</div><p>Developer update</p>")
        );
        assert_eq!(parsed.attachments.len(), 1);
        assert_eq!(
            parsed.attachments[0].filename.as_deref(),
            Some("policy.pdf")
        );
    }

    #[test]
    fn parses_raw_related_html_with_cid_image_without_disposition() {
        let raw = "\
From: Alice <alice@example.com>\r\n\
To: Bob <bob@example.com>\r\n\
Subject: Inline\r\n\
Message-ID: <inline@example.com>\r\n\
Content-Type: multipart/related; boundary=\"rel\"\r\n\
\r\n\
--rel\r\n\
Content-Type: text/html; charset=utf-8\r\n\
\r\n\
<html><body><img src=\"cid:logo@example.com\"></body></html>\r\n\
--rel\r\n\
Content-Type: image/png; name*=utf-8''logo%20small.png\r\n\
Content-Transfer-Encoding: base64\r\n\
Content-ID: <logo@example.com>\r\n\
\r\n\
iVBORw0KGgo=\r\n\
--rel--\r\n";

        let parsed = parse_raw_rfc822(raw.as_bytes()).expect("parse related message");
        assert_eq!(
            parsed.body_html.as_deref(),
            Some("<html><body><img src=\"cid:logo@example.com\"></body></html>")
        );
        assert_eq!(parsed.attachments.len(), 1);
        assert_eq!(
            parsed.attachments[0].filename.as_deref(),
            Some("logo small.png")
        );
        assert_eq!(
            parsed.attachments[0].content_id.as_deref(),
            Some("logo@example.com")
        );
        assert!(parsed.attachments[0].is_inline);
    }

    #[test]
    fn parses_raw_windows_1252_quoted_printable_even_when_charset_lies() {
        let raw = "\
From: Alice <alice@example.com>\r\n\
Subject: Encoding\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
Content-Transfer-Encoding: quoted-printable\r\n\
\r\n\
Freundliche Gr=FC=DFe\r\n";

        let parsed = parse_raw_rfc822(raw.as_bytes()).expect("parse qp message");
        assert_eq!(parsed.body_text.as_deref(), Some("Freundliche Grüße"));
    }

    /// Body normalisation is shared by plain text, calendar and HTML parts.
    /// It used to cap every one of them at 64 KiB, which left large HTML mails
    /// ending in the middle of an element before sanitising or rendering them.
    #[test]
    fn preserves_html_bodies_larger_than_64_kib() {
        let repeated = "<tr><td>vollständiger Inhalt</td></tr>".repeat(2_000);
        let raw = format!("<html><body><table>{repeated}</table><p>MAIL-ENDE</p></body></html>");
        assert!(raw.len() > 64 * 1024);

        let parsed = parse_message_body(
            "Content-Type: text/html; charset=utf-8\r\n",
            Some(raw.as_bytes()),
        );
        let html = parsed.html.expect("HTML body");

        assert_eq!(html.len(), raw.len());
        assert!(html.ends_with("<p>MAIL-ENDE</p></body></html>"));
    }

    #[test]
    fn recognizes_the_legacy_body_cutoff_exactly() {
        assert!(super::is_legacy_truncated_body(Some(
            &"x".repeat(64 * 1024)
        )));
        assert!(super::is_legacy_truncated_body(Some(
            &"x".repeat(64 * 1024 - 3)
        )));
        assert!(!super::is_legacy_truncated_body(Some(
            &"x".repeat(64 * 1024 - 4)
        )));
        assert!(!super::is_legacy_truncated_body(None));
    }

    #[test]
    fn preserves_attached_rfc822_messages() {
        let raw = "\
From: Alice <alice@example.com>\r\n\
Subject: Attached mail\r\n\
Content-Type: multipart/mixed; boundary=\"mix\"\r\n\
\r\n\
--mix\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
See attached.\r\n\
--mix\r\n\
Content-Type: message/rfc822; name=\"forwarded.eml\"\r\n\
Content-Disposition: attachment; filename=\"forwarded.eml\"\r\n\
\r\n\
From: Carol <carol@example.com>\r\n\
Subject: Forwarded\r\n\
\r\n\
Nested body\r\n\
--mix--\r\n";

        let parsed = parse_raw_rfc822(raw.as_bytes()).expect("parse attached message");
        assert_eq!(parsed.body_text.as_deref(), Some("See attached."));
        assert_eq!(parsed.attachments.len(), 1);
        assert_eq!(
            parsed.attachments[0].filename.as_deref(),
            Some("forwarded.eml")
        );
        assert_eq!(parsed.attachments[0].content_type, "message/rfc822");
    }

    /// The window is a per-account override with a platform default behind it.
    ///
    /// Zero has to mean "keep everything" rather than "keep nothing": it is
    /// what a settings screen sends when the user turns the limit off, and the
    /// other reading would quietly empty their mailbox.
    #[test]
    fn a_zero_window_keeps_everything() {
        assert_eq!(
            super::configured_body_window_days(r#"{"body_window_days":0}"#),
            None
        );
    }

    #[test]
    fn an_explicit_window_wins_over_the_platform_default() {
        assert_eq!(
            super::configured_body_window_days(r#"{"body_window_days":30}"#),
            Some(30)
        );
    }

    /// An account written before the field existed must not be read as zero.
    #[test]
    fn a_config_without_the_field_falls_back_to_the_default() {
        assert_eq!(
            super::configured_body_window_days(r#"{"host":"imap.example.com"}"#),
            super::default_body_window_days()
        );
        assert_eq!(
            super::configured_body_window_days("not json"),
            super::default_body_window_days()
        );
    }

    /// The desktop keeps everything; that is what offline search rests on.
    #[test]
    fn the_desktop_default_is_unbounded() {
        #[cfg(not(mobile))]
        assert_eq!(super::default_body_window_days(), None);
        #[cfg(mobile)]
        assert_eq!(super::default_body_window_days(), Some(30));
    }

    #[test]
    fn base64_survives_missing_padding_and_a_dirty_last_quantum() {
        // 39 characters, no padding: 29 bytes of body.
        let unpadded = b"SGFsbG8gV2VsdCwgZGFzIGlzdCBlaW4gQm9keS4";
        assert_eq!(
            super::decode_part_bytes(unpadded, "base64"),
            b"Hallo Welt, das ist ein Body.".to_vec()
        );

        // Whitespace between the lines is not part of the encoding.
        assert_eq!(
            super::decode_part_bytes(b"SGFsbG8g\r\nV2VsdA==", "base64"),
            b"Hallo Welt".to_vec()
        );
    }

    #[test]
    fn a_broken_tail_keeps_the_decodable_prefix() {
        // Twelve valid characters, then a byte outside the alphabet.
        let truncated = b"SGFsbG8gV2Vs*****";
        let decoded = super::decode_part_bytes(truncated, "base64");
        assert_eq!(decoded, b"Hallo Wel".to_vec());
        assert!(!decoded.is_empty(), "an empty body is the one wrong answer");
    }

    #[test]
    fn extract_header_param_skips_segments_without_a_value() {
        assert_eq!(
            super::extract_header_param("multipart/mixed; ; boundary=\"x\"", "boundary"),
            Some("x".to_string())
        );
        assert_eq!(
            super::extract_header_param("text/plain; flowed; charset=utf-8", "charset"),
            Some("utf-8".to_string())
        );
        assert_eq!(
            super::extract_header_param("text/plain; charset=", "charset"),
            None
        );
    }

    #[test]
    fn a_boundary_is_only_a_boundary_on_a_line_of_its_own() {
        let body = b"--=_b_1\r\n\
Content-Type: text/plain\r\n\r\n\
Im Zitat stand: --=_b_1-- und danach ging es weiter.\r\n\
--=_b_1\r\n\
Content-Type: text/plain\r\n\r\n\
Der zweite Teil.\r\n\
--=_b_1--\r\n\
Nachwort.\r\n";

        let sections = super::split_multipart_sections(body, "=_b_1");

        assert_eq!(sections.len(), 2);
        assert!(String::from_utf8_lossy(sections[0]).contains("danach ging es weiter"));
        assert!(String::from_utf8_lossy(sections[1]).ends_with("Der zweite Teil."));
    }

    /// `=_b_1` is a prefix of `=_b_12`; splitting on the shorter one used to
    /// cut the inner parts apart and store their remains as message text.
    #[test]
    fn an_outer_boundary_does_not_split_a_longer_inner_one() {
        let body = b"--=_b_1\r\n\
Content-Type: multipart/alternative; boundary=\"=_b_12\"\r\n\r\n\
--=_b_12\r\n\
Content-Type: text/plain\r\n\r\n\
Textfassung.\r\n\
--=_b_12\r\n\
Content-Type: text/html\r\n\r\n\
<p>HTML-Fassung.</p>\r\n\
--=_b_12--\r\n\
--=_b_1--\r\n";

        let outer = super::split_multipart_sections(body, "=_b_1");
        assert_eq!(outer.len(), 1);

        let (header, inner_body) = super::split_header_and_body(outer[0]);
        assert!(String::from_utf8_lossy(header).contains("multipart/alternative"));
        let inner = super::split_multipart_sections(inner_body, "=_b_12");
        assert_eq!(inner.len(), 2);
        assert!(String::from_utf8_lossy(inner[0]).contains("Textfassung."));
        assert!(String::from_utf8_lossy(inner[1]).contains("HTML-Fassung."));
    }

    /// A 25 MB message with a long boundary and three levels. The bound is
    /// loose on purpose: it only has to catch the byte-at-a-time scan coming
    /// back, and that one took minutes, not seconds.
    #[test]
    fn a_large_nested_message_parses_in_bounded_time() {
        let boundary = "=".repeat(35) + &"_".repeat(35);
        let filler = "A".repeat(64);
        let mut body = String::with_capacity(26 * 1024 * 1024);
        body.push_str(&format!("--{boundary}\r\nContent-Type: text/plain\r\n\r\n"));
        while body.len() < 25 * 1024 * 1024 {
            body.push_str(&filler);
            body.push_str("\r\n");
        }
        body.push_str(&format!("\r\n--{boundary}--\r\n"));
        let header = format!("Content-Type: multipart/mixed; boundary=\"{boundary}\"\r\n");

        let started = std::time::Instant::now();
        let parsed = super::parse_message_body(&header, Some(body.as_bytes()));
        let elapsed = started.elapsed();

        assert!(parsed.text.is_some_and(|text| text.len() > 1024));
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "parsing 25 MB took {elapsed:?}"
        );
    }

    #[test]
    fn every_generation_combination_has_one_answer() {
        use super::GenerationDecision::{Continue, Reset, WindowOnly};

        // (cursor, database, server) -> decision
        let cases = [
            (0u32, None, 222u32, WindowOnly),
            (0, Some(0), 222, WindowOnly),
            (0, Some(111), 222, Reset),
            (0, Some(222), 222, Continue),
            (111, None, 222, Reset),
            (111, Some(111), 222, Reset),
            (111, Some(222), 222, Reset),
            (222, None, 222, Continue),
            (222, Some(111), 222, Continue),
            (222, Some(222), 222, Continue),
        ];

        for (cursor, database, server, expected) in cases {
            assert_eq!(
                super::imap_generation_decision(cursor, database, server),
                expected,
                "cursor {cursor}, database {database:?}, server {server}"
            );
        }
    }

    #[test]
    fn the_database_generation_reaches_a_cursor_that_has_none() {
        let stored = serde_json::to_string(&super::ImapCursorState {
            high_uid: 5000,
            format_version: 4,
            ..Default::default()
        })
        .expect("cursor");

        let checkpoint = super::imap_checkpoint(Some(stored), Some(111));
        let state = serde_json::from_str::<super::ImapCursorState>(
            checkpoint.cursor.as_deref().expect("cursor"),
        )
        .expect("state");

        assert_eq!(state.db_uidvalidity, 111);
        assert_eq!(state.high_uid, 5000);
        assert_eq!(
            super::imap_generation_decision(state.uidvalidity, Some(state.db_uidvalidity), 222),
            super::GenerationDecision::Reset
        );
    }

    #[test]
    fn a_checkpoint_without_anything_to_add_is_left_alone() {
        for (cursor, database) in [
            (None, Some(111u32)),
            (Some("uid:5000".to_string()), Some(111)),
            (Some("{\"high_uid\":7}".to_string()), None),
            (Some("{\"high_uid\":7}".to_string()), Some(0)),
        ] {
            assert_eq!(
                super::imap_checkpoint(cursor.clone(), database).cursor,
                cursor,
                "cursor {cursor:?}, database {database:?}"
            );
        }
    }

    #[test]
    fn a_run_without_a_uid_anchor_backfills_the_history_below_its_window() {
        // (since_uid, backfill_next_seq, tail_start) -> resume point
        assert_eq!(super::backfill_resume_seq(0, 0, 900), Some(899));
        assert_eq!(super::backfill_resume_seq(0, 0, 1), None);
        assert_eq!(super::backfill_resume_seq(0, 42, 900), None);
        assert_eq!(super::backfill_resume_seq(5000, 0, 900), None);
    }

    fn listed(uid: u32, size: u64) -> super::ListedMessage {
        super::ListedMessage {
            uid,
            seq: uid,
            size,
        }
    }

    const TEST_LIMITS: super::FetchLimits = super::FetchLimits {
        chunk_messages: 3,
        chunk_bytes: 1_000,
        run_messages: 7,
        run_bytes: 5_000,
    };

    fn chunk_uids(plan: &super::FetchPlan) -> Vec<Vec<u32>> {
        plan.chunks
            .iter()
            .map(|chunk| chunk.iter().map(|message| message.uid).collect())
            .collect()
    }

    #[test]
    fn the_listing_keeps_only_new_uids_in_ascending_order() {
        // `n:*` names the highest message even when it is below `n`.
        let entries = [
            (Some(12), 3, Some(10)),
            (Some(10), 1, Some(10)),
            (None, 4, Some(10)),
            (Some(11), 2, None),
        ];
        let (messages, uidless) = super::listed_messages(entries, 10, 0);
        assert_eq!(
            messages,
            vec![
                super::ListedMessage {
                    uid: 11,
                    seq: 2,
                    size: 0
                },
                super::ListedMessage {
                    uid: 12,
                    seq: 3,
                    size: 10
                },
            ]
        );
        assert_eq!(uidless, 1);

        let (by_seq, _) = super::listed_messages([(Some(5), 1, None), (Some(6), 2, None)], 0, 1);
        assert_eq!(by_seq.iter().map(|m| m.uid).collect::<Vec<_>>(), vec![6]);
    }

    #[test]
    fn chunks_are_bounded_by_count_and_bytes_and_run_oldest_first() {
        let messages: Vec<_> = (1..=5).map(|uid| listed(uid, 100)).collect();
        let plan = super::plan_incremental_fetch(&messages, TEST_LIMITS);
        assert_eq!(chunk_uids(&plan), vec![vec![1, 2, 3], vec![4, 5]]);
        assert_eq!(plan.deferred, 0);

        let messages = [listed(1, 400), listed(2, 400), listed(3, 400)];
        let plan = super::plan_incremental_fetch(&messages, TEST_LIMITS);
        assert_eq!(chunk_uids(&plan), vec![vec![1, 2], vec![3]]);
    }

    /// A 25 MB mail is above every budget and must still arrive.
    #[test]
    fn a_message_above_the_budgets_goes_alone() {
        let messages = [listed(1, 10), listed(2, 50_000), listed(3, 10)];
        let plan = super::plan_incremental_fetch(&messages, TEST_LIMITS);
        assert_eq!(chunk_uids(&plan), vec![vec![1]]);
        assert_eq!(plan.deferred, 2, "the large mail waits for its own run");

        let plan = super::plan_incremental_fetch(&messages[1..], TEST_LIMITS);
        assert_eq!(chunk_uids(&plan), vec![vec![2]]);
        assert_eq!(plan.deferred, 1);
    }

    #[test]
    fn a_backlog_beyond_the_run_budget_is_deferred() {
        let messages: Vec<_> = (1..=20).map(|uid| listed(uid, 10)).collect();
        let plan = super::plan_incremental_fetch(&messages, TEST_LIMITS);
        assert_eq!(
            chunk_uids(&plan),
            vec![vec![1, 2, 3], vec![4, 5, 6], vec![7]]
        );
        assert_eq!(plan.deferred, 13);
        assert_eq!(
            super::plan_incremental_fetch(&[], TEST_LIMITS),
            super::FetchPlan::default()
        );
    }

    /// The cursor may only pass UIDs whose chunk arrived in full; what is left
    /// is reported so the next run comes back for it.
    #[test]
    fn the_cursor_advances_per_completed_chunk() {
        let messages: Vec<_> = (101..=120).map(|uid| listed(uid, 10)).collect();
        let plan = super::plan_incremental_fetch(&messages, TEST_LIMITS);

        assert_eq!(super::advance_after_chunks(100, 7, &plan, 0), (100, 7, 20));
        assert_eq!(
            super::advance_after_chunks(100, 7, &plan, 1),
            (103, 103, 17)
        );
        assert_eq!(
            super::advance_after_chunks(100, 7, &plan, 3),
            (107, 107, 13)
        );
        let empty = super::FetchPlan::default();
        assert_eq!(super::advance_after_chunks(100, 7, &empty, 0), (100, 7, 0));
    }

    #[test]
    fn a_chunk_deadline_grows_with_its_bytes() {
        let base = Duration::from_secs(20);
        assert_eq!(super::chunk_timeout(base, &[listed(1, 1_000)]), base);
        let large = [listed(1, 25 * 1024 * 1024)];
        assert_eq!(
            super::chunk_timeout(base, &large),
            base + Duration::from_secs(400)
        );
    }

    #[test]
    fn idle_reconnects_back_off_while_rounds_stay_short() {
        let mut backoff = super::IdleReconnectBackoff::default();
        let short = Duration::from_millis(50);
        let delays: Vec<_> = (0..10).map(|_| backoff.after_round(short)).collect();
        assert_eq!(delays[0], Duration::from_secs(1));
        assert_eq!(delays[1], Duration::from_secs(2));
        assert!(delays.windows(2).all(|pair| pair[0] <= pair[1]));
        assert_eq!(*delays.last().unwrap(), super::IDLE_RECONNECT_MAX);

        assert_eq!(
            backoff.after_round(Duration::from_secs(25 * 60)),
            super::IDLE_RECONNECT_BASE,
            "a healthy round resets the pause"
        );
        assert_eq!(backoff.after_round(short), Duration::from_secs(1));
    }

    /// Skipping the fallback parse must never change what a message keeps.
    #[test]
    fn the_fallback_parse_is_skipped_only_where_it_adds_nothing() {
        let raw = concat!(
            "Content-Type: multipart/mixed; boundary=\"b\"\r\n\r\n",
            "--b\r\nContent-Type: multipart/alternative; boundary=\"a\"\r\n\r\n",
            "--a\r\nContent-Type: text/plain\r\n\r\nHallo\r\n",
            "--a\r\nContent-Type: text/html\r\n\r\n<p>Hallo</p>\r\n--a--\r\n",
            "--b\r\nContent-Type: application/pdf\r\n",
            "Content-Disposition: attachment; filename=\"a.pdf\"\r\n\r\nPDF\r\n--b--\r\n",
        );
        let (header, body) = super::split_header_and_body(raw.as_bytes());
        let header = String::from_utf8_lossy(header);
        let complete = parse_message_body(&header, Some(body));
        assert!(complete.text.is_some() && complete.html.is_some());
        assert!(!complete.attachments.is_empty());
        assert!(!super::extracted_fallback_could_help(&complete));

        // What skipping saves really is a no-op merge.
        let mut merged = parse_message_body(&header, Some(body));
        let extracted = parse_extracted_imap_email(
            &header,
            Some(body),
            SourceId::new("imap/account-1"),
            "INBOX",
            7,
            1,
        );
        assert!(extracted.is_some());
        super::merge_extracted_imap_fallback(&mut merged, extracted);
        assert_eq!(merged.text, complete.text);
        assert_eq!(merged.html, complete.html);
        assert_eq!(merged.attachments.len(), complete.attachments.len());

        let plain = parse_message_body("Content-Type: text/plain\r\n", Some(b"Hallo"));
        assert!(super::extracted_fallback_could_help(&plain));
    }

    /// A scripted IMAP server: answers LOGIN, then one FETCH response per
    /// chunk command until `stall_at`, where it goes silent mid-response.
    async fn chunk_server(
        stall_at: usize,
    ) -> (std::net::SocketAddr, tokio::task::JoinHandle<Vec<String>>) {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let (reader, mut writer) = socket.into_split();
            let mut reader = tokio::io::BufReader::new(reader);
            writer.write_all(b"* OK ready\r\n").await.unwrap();
            let mut commands = Vec::new();
            let mut line = String::new();
            reader.read_line(&mut line).await.unwrap();
            let tag = line.split_whitespace().next().unwrap().to_string();
            writer
                .write_all(format!("{tag} OK logged in\r\n").as_bytes())
                .await
                .unwrap();
            for index in 0.. {
                line.clear();
                if reader.read_line(&mut line).await.unwrap() == 0 {
                    break;
                }
                commands.push(line.trim_end().to_string());
                let tag = line.split_whitespace().next().unwrap().to_string();
                if !line.contains("UID FETCH") {
                    writer
                        .write_all(format!("{tag} OK bye\r\n").as_bytes())
                        .await
                        .unwrap();
                    break;
                }
                let uids = line
                    .split_whitespace()
                    .nth(3)
                    .unwrap()
                    .split(',')
                    .map(|uid| uid.parse::<u32>().unwrap())
                    .collect::<Vec<_>>();
                if index == stall_at {
                    writer
                        .write_all(b"* 1 FETCH (UID 1 BODY[TEXT] {100}\r\npartial")
                        .await
                        .unwrap();
                    std::future::pending::<()>().await;
                }
                for uid in uids {
                    let header = format!("Subject: mail {uid}\r\n\r\n");
                    let text = format!("body {uid}");
                    writer
                        .write_all(
                            format!(
                                "* {uid} FETCH (UID {uid} FLAGS () BODY[HEADER] {{{}}}\r\n{header} BODY[TEXT] {{{}}}\r\n{text})\r\n",
                                header.len(),
                                text.len()
                            )
                            .as_bytes(),
                        )
                        .await
                        .unwrap();
                }
                writer
                    .write_all(format!("{tag} OK done\r\n").as_bytes())
                    .await
                    .unwrap();
            }
            commands
        });
        (address, server)
    }

    async fn chunk_session(
        address: std::net::SocketAddr,
    ) -> async_imap::Session<tokio::net::TcpStream> {
        let socket = tokio::net::TcpStream::connect(address).await.unwrap();
        let mut client = async_imap::Client::new(socket);
        client.read_response().await.unwrap();
        client.login("user", "password").await.unwrap()
    }

    /// A chunk that breaks off keeps the chunks before it: the next run starts
    /// at the broken one instead of downloading everything again.
    #[tokio::test]
    async fn a_broken_chunk_keeps_the_chunks_that_arrived() {
        let (address, server) = chunk_server(1).await;
        let mut session = chunk_session(address).await;
        let messages: Vec<_> = (1..=7).map(|uid| listed(uid, 10)).collect();
        let plan = super::plan_incremental_fetch(&messages, TEST_LIMITS);

        let fetched = tokio::time::timeout(
            Duration::from_secs(5),
            super::fetch_planned_chunks(&mut session, &plan, Duration::from_millis(100)),
        )
        .await
        .expect("the chunk deadline applies")
        .expect("progress before the break is kept");
        server.abort();

        assert_eq!(fetched.completed, 1);
        assert!(fetched.interrupted.unwrap().contains("timed out"));
        let mut uids: Vec<_> = fetched.entries.iter().filter_map(|e| e.uid).collect();
        uids.sort_unstable();
        assert_eq!(uids, vec![1, 2, 3]);
        assert!(fetched
            .entries
            .iter()
            .all(|entry| entry.text().is_some_and(|text| text.starts_with(b"body "))));
        assert_eq!(
            super::advance_after_chunks(0, 0, &plan, fetched.completed),
            (3, 3, 4)
        );
    }

    #[tokio::test]
    async fn a_first_chunk_that_breaks_off_fails_the_fetch() {
        let (address, server) = chunk_server(0).await;
        let mut session = chunk_session(address).await;
        let plan = super::plan_incremental_fetch(&[listed(1, 10)], TEST_LIMITS);
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            super::fetch_planned_chunks(&mut session, &plan, Duration::from_millis(100)),
        )
        .await
        .expect("the chunk deadline applies");
        server.abort();
        assert!(result.unwrap_err().contains("timed out"));
    }

    #[tokio::test]
    async fn every_chunk_is_one_uid_fetch_in_ascending_order() {
        let (address, server) = chunk_server(usize::MAX).await;
        let mut session = chunk_session(address).await;
        let messages: Vec<_> = (1..=5).map(|uid| listed(uid, 10)).collect();
        let plan = super::plan_incremental_fetch(&messages, TEST_LIMITS);
        let fetched = super::fetch_planned_chunks(&mut session, &plan, Duration::from_secs(5))
            .await
            .expect("all chunks arrive");
        assert_eq!(fetched.completed, 2);
        assert!(fetched.interrupted.is_none());
        assert_eq!(fetched.entries.len(), 5);
        let _ = session.logout().await;
        let commands = server.await.unwrap();
        assert!(commands[0].ends_with(
            "UID FETCH 1,2,3 (UID FLAGS INTERNALDATE BODY.PEEK[HEADER] BODY.PEEK[TEXT])"
        ));
        assert!(commands[1].contains("UID FETCH 4,5 "));
    }
}
