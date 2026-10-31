mod composer_body;
pub mod eml_attachments;

use base64::Engine as _;
use chrono::{DateTime, Utc};
use satchel_core::db::accounts::AccountRepository;
use satchel_core::db::aliases::AliasRepository;
use satchel_core::db::followups::FollowUpRepository;
use satchel_core::db::messages::{
    MessagePreviewRecord, MessageReadError, MessageReadRecord, MessageReadRepository,
    MessageSearchError, MessageSearchRepository, SentAttachmentMetadata, SentMessageError,
    SentMessageRepository, SentMessageWriteRequest,
};
use satchel_core::db::smime::SmimeCertificateRepository;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::{sync::Arc, time::Instant};
use tauri::Manager as _;

use crate::{
    commands::error::{CommandError, CommandResult},
    commands::pgp::{
        decrypt_message_with_unlocked_keys, verify_incoming_cleartext_signature,
        verify_incoming_detached_signature, IncomingPgpDecryptError, IncomingPgpDetachedSignature,
        IncomingPgpSignature,
    },
    commands::signatures::{
        resolve_signature_for_compose, ResolveSignatureInput, SignaturePreference, SignatureVariant,
    },
    commands::smime::{
        decrypt_message_with_unlocked_smime_certs, verify_smime_message, SmimeVerifyMessageResult,
    },
    db::remote_content::{saved_uidvalidity, ContentReference},
    email::sanitizer::sanitize_email_html,
    email::signed_mime::{self, EntityDisplay, SignedMimeError, StoredSignedBody},
    events::bus::AppEvent,
    plugins::runtime::{MessageTransformInput, PluginRuntime},
    providers::imap::{
        build_snippet, is_legacy_truncated_body, parse_header_value, parse_raw_message_body,
    },
    providers::traits::MessageTrust,
    sync::outbox::{build_send_action, execute_send_action},
    utils::mime::{
        decode_html_entities, decode_mime_encoded_words, extract_email_address,
        repair_likely_utf8_mojibake, resolve_sender_display_name,
    },
    AppState,
};

/// Transient content provenance. Never reconstructed from persisted body text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/messages/")
)]
#[serde(tag = "kind", content = "content", rename_all = "snake_case")]
pub enum BodySegment {
    Text(String),
    PgpDecrypted(String),
    SmimeDecrypted(String),
    /// An armored block among other text that the unlocked keys could not
    /// decrypt, such as one quoted from a message to someone else.
    PgpUndecrypted(String),
}

impl BodySegment {
    pub fn content(&self) -> &str {
        match self {
            Self::Text(content)
            | Self::PgpDecrypted(content)
            | Self::SmimeDecrypted(content)
            | Self::PgpUndecrypted(content) => content,
        }
    }
}

/// Exact transient render inputs issued together after decoding and
/// sanitizing. The renderer grants captions only while the body matches this
/// snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/messages/")
)]
pub struct BodyProvenance {
    pub segments: Vec<BodySegment>,
    pub html: Option<String>,
    pub attachments: Vec<DecryptedAttachmentBinding>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/messages/")
)]
pub struct DecryptedAttachmentBinding {
    pub id: String,
    pub filename: String,
    pub content_type: String,
    pub size: i64,
    pub content_id: Option<String>,
    pub is_inline: bool,
    pub cached_path: Option<String>,
    pub data_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/messages/")
)]
pub struct MessageDto {
    pub id: String,
    /// The saved IMAP generation also identifies cached bodies and inline bytes.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ipc-types", ts(optional = nullable))]
    pub remote_uidvalidity: Option<u32>,
    /// A remote identity that cannot serve content until its folder is synced.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ipc-types", ts(optional = nullable))]
    pub content_error: Option<String>,
    pub thread_id: String,
    pub subject: String,
    pub snippet: String,
    pub from_address: String,
    pub from_name: String,
    pub to_addresses: Vec<String>,
    pub cc_addresses: Vec<String>,
    pub reply_to: Option<String>,
    pub date: String,
    pub body_text: String,
    pub body_segments: Option<Vec<BodySegment>>,
    pub body_provenance: Option<BodyProvenance>,
    pub body_html: Option<String>,
    pub body_loaded: bool,
    pub is_read: bool,
    pub is_draft: bool,
    pub pgp_state: Option<String>,
    pub pgp_error_detail: Option<String>,
    pub pgp_signer_name: Option<String>,
    pub pgp_signer_fingerprint: Option<String>,
    pub smime_state: Option<String>,
    pub smime_signer_email: Option<String>,
    pub smime_signer_cn: Option<String>,
    pub smime_error_detail: Option<String>,
    pub list_unsubscribe: Option<String>,
    pub list_unsubscribe_post: Option<String>,
    pub auth_spf: Option<String>,
    pub auth_dkim: Option<String>,
    pub auth_dmarc: Option<String>,
    pub source_payload: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/messages/")
)]
pub struct MessageBodyDto {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ipc-types", ts(optional = nullable))]
    pub remote_uidvalidity: Option<u32>,
    /// A remote identity that cannot serve content until its folder is synced.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ipc-types", ts(optional = nullable))]
    pub content_error: Option<String>,
    pub snippet: String,
    pub body_text: String,
    pub body_segments: Option<Vec<BodySegment>>,
    pub body_provenance: Option<BodyProvenance>,
    pub body_html: Option<String>,
    pub body_loaded: bool,
    pub pgp_state: Option<String>,
    pub pgp_error_detail: Option<String>,
    pub pgp_signer_name: Option<String>,
    pub pgp_signer_fingerprint: Option<String>,
    pub smime_state: Option<String>,
    pub smime_signer_email: Option<String>,
    pub smime_signer_cn: Option<String>,
    pub smime_error_detail: Option<String>,
    /// The headers behind the unsubscribe banner. The preview leaves them
    /// empty; the banner sits beside a loaded body, so the body carries them.
    pub list_unsubscribe: Option<String>,
    pub list_unsubscribe_post: Option<String>,
    pub auth_spf: Option<String>,
    pub auth_dkim: Option<String>,
    pub auth_dmarc: Option<String>,
    /// Structured item data for source messages (feeds, webhooks). Sent with
    /// the body rather than the preview: it is only rendered beside a body.
    pub source_payload: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/messages/", optional_fields = nullable))]
pub struct MessageBodyIdsInput {
    pub message_ids: Vec<String>,
}

pub use satchel_core::drafts::outgoing::{OutgoingAttachmentInput, OutgoingMessageInput};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/messages/")
)]
pub struct SendMessageResult {
    pub status: String,
    pub thread_id: String,
    pub message_id: String,
    #[serde(default)]
    pub queued_id: Option<String>,
    /// End of the Undo Send window, so a composer popout can offer Undo itself.
    #[serde(default)]
    pub undo_until: Option<String>,
}

#[tauri::command]
pub async fn list_messages_preview(
    pool: tauri::State<'_, crate::db::ReadPool>,
    thread_id: String,
) -> CommandResult<Vec<MessageDto>> {
    let started_at = Instant::now();
    let rows = MessageReadRepository::new(pool.inner().as_ref())
        .list_messages_preview(&thread_id)
        .await
        .map_err(CommandError::from)?;

    let messages = validated_previews(pool.inner().as_ref(), rows).await?;
    log_perf(
        "list_messages_preview",
        started_at,
        format!("thread_id={} rows={}", thread_id, messages.len()),
    );
    Ok(messages)
}

async fn validated_previews(
    pool: &SqlitePool,
    rows: Vec<MessagePreviewRecord>,
) -> CommandResult<Vec<MessageDto>> {
    let ids = rows.iter().map(|row| row.id.clone()).collect::<Vec<_>>();
    let references = ContentReference::load_many(pool, &ids).await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let mut dto = map_message_preview_row(row);
            dto.content_error = references
                .get(&dto.id)
                .map(|reference| {
                    reference.validate_cached_read().err().or_else(|| {
                        (dto.remote_uidvalidity != reference.uidvalidity())
                            .then(|| reference.changed_error())
                    })
                })
                .unwrap_or_else(|| Some("backend.errors.messageNotAvailableLocally".to_string()));
            if dto.content_error.is_some() {
                dto.snippet.clear();
                dto.body_html = None;
                dto.source_payload = None;
                dto.body_text.clear();
                dto.body_loaded = true;
            }
            dto
        })
        .collect())
}

/// Older parser versions stored the detached signature as an attachment but not
/// as a verifiable MIME bundle. Refresh only those explicit candidates, never
/// the rest of a thread. Returns the threads that changed.
///
/// Each attempt is recorded, so a message the parser still cannot read is
/// fetched again after [`SMIME_REPAIR_RETRY`] rather than on every open, and
/// a message already being fetched is not fetched a second time alongside.
async fn repair_unparsed_smime_messages(
    pool: &SqlitePool,
    write_pool: &SqlitePool,
    message_ids: &[String],
) -> Vec<String> {
    if message_ids.is_empty() {
        return Vec::new();
    }
    let placeholders = vec!["?"; message_ids.len()].join(",");
    let sql = format!(
        "SELECT DISTINCT m.id, m.thread_id
         FROM messages m
         JOIN attachments a ON a.message_id = m.id
         WHERE m.id IN ({placeholders})
           AND m.smime_signed IS NULL
           AND (m.smime_repair_attempted_at IS NULL OR m.smime_repair_attempted_at < ?)
           AND LOWER(a.content_type) IN (
             'application/pkcs7-signature',
             'application/x-pkcs7-signature'
           )
         LIMIT 20"
    );
    let mut query = sqlx::query_as::<_, (String, String)>(&sql);
    for id in message_ids {
        query = query.bind(id);
    }
    let retry_before = (Utc::now() - SMIME_REPAIR_RETRY).to_rfc3339();
    let Ok(candidates) = query.bind(retry_before).fetch_all(pool).await else {
        return Vec::new();
    };
    let claim = SmimeRepairClaim::new(candidates);

    let mut repaired: Vec<String> = Vec::new();
    for (message_id, thread_id) in claim.candidates.iter().cloned() {
        let Ok(reference) = ContentReference::load(pool, &message_id).await else {
            continue;
        };
        let attempt = fetch_remote_message_source(pool, write_pool, &message_id).await;
        if let Err(error) = record_smime_repair_attempt(write_pool, &reference).await {
            tracing::warn!(%message_id, %error, "could not record S/MIME repair attempt");
            continue;
        }
        match attempt {
            Ok(Some(raw)) => {
                match persist_raw_message_metadata(write_pool, &reference, &raw).await {
                    Ok(true) => {
                        if !repaired.contains(&thread_id) {
                            repaired.push(thread_id);
                        }
                    }
                    Ok(false) => {}
                    Err(error) => {
                        tracing::warn!(%message_id, %error, "could not persist repaired S/MIME metadata");
                    }
                }
            }
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(%message_id, %error, "could not refresh unparsed S/MIME message");
            }
        }
    }
    repaired
}

async fn record_smime_repair_attempt(
    pool: &SqlitePool,
    reference: &ContentReference,
) -> Result<(), String> {
    let mut tx = reference.begin_write(pool).await?;
    sqlx::query("UPDATE messages SET smime_repair_attempted_at = ? WHERE id = ?")
        .bind(Utc::now().to_rfc3339())
        .bind(&reference.message_id)
        .execute(&mut *tx)
        .await
        .map_err(|error| error.to_string())?;
    tx.commit().await.map_err(|error| error.to_string())
}

/// How long a message the S/MIME repair could not fix waits for another try.
const SMIME_REPAIR_RETRY: chrono::Duration = chrono::Duration::days(1);

/// S/MIME repair candidates this caller fetches; released on drop.
struct SmimeRepairClaim {
    candidates: Vec<(String, String)>,
}

fn smime_repairs_in_flight() -> &'static std::sync::Mutex<std::collections::HashSet<String>> {
    static IN_FLIGHT: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> =
        std::sync::OnceLock::new();
    IN_FLIGHT.get_or_init(Default::default)
}

impl SmimeRepairClaim {
    /// Keep the candidates no other caller is fetching right now.
    fn new(candidates: Vec<(String, String)>) -> Self {
        let mut in_flight = smime_repairs_in_flight()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let candidates = candidates
            .into_iter()
            .filter(|(message_id, _)| in_flight.insert(message_id.clone()))
            .collect();
        Self { candidates }
    }
}

impl Drop for SmimeRepairClaim {
    fn drop(&mut self) {
        let mut in_flight = smime_repairs_in_flight()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for (message_id, _) in &self.candidates {
            in_flight.remove(message_id);
        }
    }
}

#[tauri::command]
pub async fn list_message_bodies(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    pool: tauri::State<'_, crate::db::ReadPool>,
    write_pool: tauri::State<'_, SqlitePool>,
    input: MessageBodyIdsInput,
) -> CommandResult<Vec<MessageBodyDto>> {
    message_bodies(
        &app,
        &state,
        pool.inner().as_ref(),
        write_pool.inner(),
        &input.message_ids,
    )
    .await
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/messages/", optional_fields = nullable))]
pub struct LoadThreadMessagesInput {
    pub thread_id: String,
    pub body_count: usize,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/messages/", optional_fields = nullable))]
pub struct OpenedThreadDto {
    pub messages: Vec<MessageDto>,
    pub bodies: Vec<MessageBodyDto>,
}

/// The latest `body_count` messages of a thread, plus every draft: what the
/// reader shows before anything is expanded.
fn opening_body_ids(previews: &[MessagePreviewRecord], body_count: usize) -> Vec<String> {
    let is_draft = |row: &MessagePreviewRecord| row.is_draft.unwrap_or(0) > 0;
    let conversation = previews
        .iter()
        .filter(|row| !is_draft(row))
        .collect::<Vec<_>>();
    let latest = conversation[conversation.len().saturating_sub(body_count)..]
        .iter()
        .map(|row| row.id.clone());
    let drafts = previews
        .iter()
        .filter(|row| is_draft(row))
        .map(|row| row.id.clone());
    latest.chain(drafts).collect()
}

/// Previews plus the bodies the reader paints first, in one round trip.
#[tauri::command]
pub async fn load_thread_messages(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    pool: tauri::State<'_, crate::db::ReadPool>,
    write_pool: tauri::State<'_, SqlitePool>,
    input: LoadThreadMessagesInput,
) -> CommandResult<OpenedThreadDto> {
    let started_at = Instant::now();
    let rows = MessageReadRepository::new(pool.inner().as_ref())
        .list_messages_preview(&input.thread_id)
        .await
        .map_err(CommandError::from)?;
    let body_ids = opening_body_ids(&rows, input.body_count);
    let messages = validated_previews(pool.inner().as_ref(), rows).await?;
    let bodies = message_bodies(
        &app,
        &state,
        pool.inner().as_ref(),
        write_pool.inner(),
        &body_ids,
    )
    .await?;
    log_perf(
        "load_thread_messages",
        started_at,
        format!(
            "thread_id={} rows={} bodies={}",
            input.thread_id,
            messages.len(),
            bodies.len()
        ),
    );
    Ok(OpenedThreadDto { messages, bodies })
}

fn gate_cached_body(row: &mut MessageReadRecord, reference: &ContentReference) -> Option<String> {
    let content_error = reference.validate_cached_read().err().or_else(|| {
        (saved_uidvalidity(row.remote_uidvalidity) != reference.uidvalidity())
            .then(|| reference.changed_error())
    });
    if content_error.is_some() {
        row.body_text = None;
        row.body_html = None;
        row.snippet = None;
        row.source_payload = None;
    }
    content_error
}

async fn message_bodies(
    app: &tauri::AppHandle,
    state: &AppState,
    pool: &SqlitePool,
    write_pool: &SqlitePool,
    message_ids: &[String],
) -> CommandResult<Vec<MessageBodyDto>> {
    let started_at = Instant::now();
    if message_ids.is_empty() {
        return Ok(vec![]);
    }

    let repair_pool = pool.clone();
    let repair_write_pool = write_pool.clone();
    let repair_ids = message_ids.to_vec();
    let repair_events = state.events.clone();
    tauri::async_runtime::spawn(async move {
        for thread_id in
            repair_unparsed_smime_messages(&repair_pool, &repair_write_pool, &repair_ids).await
        {
            repair_events.publish(AppEvent::ThreadsUpdated(vec![thread_id]));
        }
    });

    let runtime = state.plugin_runtime.clone();
    let plugin_ids = runtime.enabled_message_transform_plugins()?;
    let has_pgp_private_key = has_any_private_pgp_key(pool).await;
    let has_smime_private_cert = has_any_private_smime_cert(pool).await;
    let references = ContentReference::load_many(pool, message_ids).await?;
    let rows = MessageReadRepository::new(pool)
        .list_message_bodies(message_ids)
        .await
        .map_err(CommandError::from)?;

    let mut bodies = Vec::with_capacity(rows.len());
    for mut row in rows {
        let reference = references
            .get(&row.id)
            .ok_or_else(|| CommandError::new("backend.errors.messageNotAvailableLocally"))?;
        let content_error = gate_cached_body(&mut row, reference);
        let dto = if content_error.is_some() {
            map_stored_message_row(row)
        } else {
            map_message_row(
                row,
                app,
                state,
                write_pool,
                &runtime,
                &plugin_ids,
                has_pgp_private_key,
                has_smime_private_cert,
                reference,
            )
            .await
        };

        let body_loaded = dto.body_loaded || content_error.is_some();
        bodies.push(MessageBodyDto {
            id: dto.id,
            remote_uidvalidity: dto.remote_uidvalidity,
            content_error,
            snippet: dto.snippet,
            body_text: dto.body_text,
            body_segments: dto.body_segments,
            body_provenance: dto.body_provenance,
            body_html: dto.body_html,
            body_loaded,
            pgp_state: dto.pgp_state,
            pgp_error_detail: dto.pgp_error_detail,
            pgp_signer_name: dto.pgp_signer_name,
            pgp_signer_fingerprint: dto.pgp_signer_fingerprint,
            smime_state: dto.smime_state,
            smime_signer_email: dto.smime_signer_email,
            smime_signer_cn: dto.smime_signer_cn,
            smime_error_detail: dto.smime_error_detail,
            list_unsubscribe: dto.list_unsubscribe,
            list_unsubscribe_post: dto.list_unsubscribe_post,
            auth_spf: dto.auth_spf,
            auth_dkim: dto.auth_dkim,
            auth_dmarc: dto.auth_dmarc,
            source_payload: dto.source_payload,
        });
    }

    let current = ContentReference::load_many(pool, message_ids).await?;
    for body in &mut bodies {
        let reference = &references[&body.id];
        if current
            .get(&body.id)
            .is_none_or(|current| !reference.same_identity(current))
        {
            body.content_error = Some(reference.changed_error());
            body.body_text.clear();
            body.body_html = None;
            body.snippet.clear();
            body.source_payload = None;
            body.body_loaded = true;
            // A verdict covers the body it was shown with, which is gone.
            body.pgp_state = None;
            body.pgp_error_detail = None;
            body.pgp_signer_name = None;
            body.pgp_signer_fingerprint = None;
            body.smime_state = None;
            body.smime_signer_email = None;
            body.smime_signer_cn = None;
            body.smime_error_detail = None;
        }
    }
    log_perf(
        "list_message_bodies",
        started_at,
        format!("message_ids={} rows={}", message_ids.len(), bodies.len()),
    );
    Ok(bodies)
}

pub use satchel_core::db::messages::{
    MessageSearchHitDto, MessageSearchResponseDto, SearchQuery, SearchWarningDto,
};

impl From<MessageReadError> for CommandError {
    fn from(error: MessageReadError) -> Self {
        match error {
            MessageReadError::NotFound => Self::new("backend.errors.messageNotAvailableLocally"),
            MessageReadError::Persistence(error) => {
                tracing::error!(error = ?error, "message read persistence failed");
                Self::new("backend.errors.messageReadFailed")
            }
        }
    }
}

impl From<MessageSearchError> for CommandError {
    fn from(error: MessageSearchError) -> Self {
        tracing::error!(error = ?error, "message search failed");
        Self::new("backend.errors.messageSearchFailed")
    }
}

impl From<SentMessageError> for CommandError {
    fn from(error: SentMessageError) -> Self {
        match error {
            SentMessageError::Persistence(error) => {
                tracing::error!(error = ?error, "sent message persistence failed");
                Self::new("backend.errors.sentMessagePersistenceFailed")
            }
            SentMessageError::ReplyContextLoad(error) => {
                tracing::error!(error = ?error, "reply context load failed");
                Self::new("backend.errors.replyContextLoadFailed")
            }
            SentMessageError::ReplyThreadNotFound { thread_id } => {
                tracing::warn!(
                    thread_id,
                    "reply thread disappeared before sent-message persistence"
                );
                Self::new("backend.errors.replyThreadNotFound")
            }
        }
    }
}

#[tauri::command]
pub async fn search_message_hits(
    pool: tauri::State<'_, crate::db::ReadPool>,
    input: SearchQuery,
) -> CommandResult<MessageSearchResponseDto> {
    MessageSearchRepository::new(pool.inner().as_ref())
        .search(&input)
        .await
        .map_err(CommandError::from)
}

/// Map stored message data before optional plugin and cryptographic processing.
///
/// Bodies are already decoded Unicode. Transfer encodings and charsets belong
/// to the MIME/provider boundary, where their headers are available. Inferring
/// them here from URLs, code or quoted examples corrupts otherwise valid mail.
/// Historical cache repair must re-read the original MIME source instead.
fn map_stored_message_row(row: MessageReadRecord) -> MessageDto {
    let auth_verified = row
        .trust_metadata
        .as_deref()
        .and_then(|value| {
            serde_json::from_str::<crate::providers::traits::MessageTrust>(value).ok()
        })
        .is_some_and(|trust| trust.auth_verified);
    let subject = repair_likely_utf8_mojibake(&decode_mime_encoded_words(
        &row.subject.unwrap_or_else(|| "(no subject)".into()),
    ));
    let snippet = {
        let value = repair_likely_utf8_mojibake(&decode_html_entities(&decode_mime_encoded_words(
            &row.snippet.unwrap_or_default(),
        )));
        if value.trim().is_empty() {
            subject.clone()
        } else {
            value
        }
    };
    let from_address = decode_mime_encoded_words(
        row.from_address
            .as_deref()
            .unwrap_or("unknown@satchel.local"),
    );
    let from_name = resolve_sender_display_name(row.from_name.as_deref(), &from_address);
    let to_addresses = parse_address_json(row.to_addresses);
    let cc_addresses = parse_address_json(row.cc_addresses);
    let body_text = row
        .body_text
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| snippet.clone());
    let body_html = row.body_html.filter(|value| !value.trim().is_empty());
    MessageDto {
        id: row.id,
        remote_uidvalidity: saved_uidvalidity(row.remote_uidvalidity),
        content_error: None,
        thread_id: row.thread_id.unwrap_or_default(),
        subject,
        snippet,
        from_address,
        from_name,
        to_addresses,
        cc_addresses,
        reply_to: row.reply_to,
        date: row.date.unwrap_or_else(|| chrono::Utc::now().to_rfc3339()),
        body_text,
        body_segments: None,
        body_provenance: None,
        body_html,
        body_loaded: true,
        is_read: row.is_read.unwrap_or(1) > 0,
        is_draft: row.is_draft.unwrap_or(0) > 0,
        pgp_state: None,
        pgp_error_detail: None,
        pgp_signer_name: None,
        pgp_signer_fingerprint: None,
        smime_state: None,
        smime_signer_email: None,
        smime_signer_cn: None,
        smime_error_detail: None,
        list_unsubscribe: row.list_unsubscribe,
        list_unsubscribe_post: row.list_unsubscribe_post,
        auth_spf: row.auth_spf.filter(|_| auth_verified),
        auth_dkim: row.auth_dkim.filter(|_| auth_verified),
        auth_dmarc: row.auth_dmarc.filter(|_| auth_verified),
        source_payload: row.source_payload,
    }
}

async fn map_message_row(
    row: MessageReadRecord,
    app: &tauri::AppHandle,
    state: &AppState,
    pool: &SqlitePool,
    runtime: &Arc<PluginRuntime>,
    plugin_ids: &[String],
    has_pgp_private_key: bool,
    has_smime_private_cert: bool,
    reference: &ContentReference,
) -> MessageDto {
    let trust = row
        .trust_metadata
        .as_deref()
        .and_then(|value| {
            serde_json::from_str::<crate::providers::traits::MessageTrust>(value).ok()
        })
        .unwrap_or_default();
    let smime_encrypted = row
        .smime_encrypted
        .clone()
        .filter(|_| trust.root_encryption);
    let smime_signed = row.smime_signed.clone();
    let pgp_signed = row.pgp_signed.clone();
    let has_stored_body = row_has_body(&row);
    // `map_stored_message_row` replaces a missing body with the snippet so
    // list views always have readable text; whether the body is really there
    // comes from the stored columns. Old IMAP bodies at the former 64-KiB
    // cutoff likewise remain readable as a cached prefix while the full body
    // is fetched.
    let body_loaded = has_stored_body
        && !(row.id.starts_with("m-imap-")
            && (is_legacy_truncated_body(row.body_text.as_deref())
                || is_legacy_truncated_body(row.body_html.as_deref())));

    let mut dto = map_stored_message_row(row);
    dto.body_loaded = body_loaded;
    let mut decrypted_attachments = Vec::new();
    let mut publication_complete = true;

    // What a stored signature covers goes on the DTO before anything decodes
    // the body. An armored block inside a signed entity is then decrypted from
    // the entity the signature is checked over, not from a stored body that
    // only the binding mark may vouch for.
    let pgp_display = match pgp_signed.as_deref() {
        Some(bundle) => Some(show_signed_body(&mut dto, bundle, &trust, has_stored_body).await),
        None => None,
    };
    let smime_display = match smime_signed.as_deref() {
        Some(payload) => Some(show_signed_body(&mut dto, payload, &trust, has_stored_body).await),
        None => None,
    };
    // Opaque S/MIME carries its content inside the signature, and only the
    // engine's verification reads it out.
    let mut smime_encapsulated = match (smime_display, smime_signed.as_deref()) {
        (Some(SignedDisplay::Encapsulated), Some(payload)) => {
            let verified =
                verify_smime_message(app, pool, &dto.id, payload, &dto.from_address).await;
            Some(show_encapsulated_body(&mut dto, verified).await)
        }
        _ => None,
    };
    let original_body_text = dto.body_text.clone();

    if (trust.root_encryption || trust.root_inline_signature)
        && message_requires_plugin_transform(&dto.body_text)
    {
        // WASM execution is blocking (wasmtime instantiate + run, up to the
        // plugin timeout). Keep it off the tokio worker threads so a batch of
        // PGP messages cannot stall unrelated async work. On a join failure
        // (panic in the blocking task) fall back to the untransformed message.
        let fallback_dto = dto.clone();
        let blocking_runtime = runtime.clone();
        let blocking_plugin_ids = plugin_ids.to_vec();
        let switched_off;
        (dto, switched_off) = tokio::task::spawn_blocking(move || {
            let mut dto = dto;
            let switched_off = apply_message_plugins(
                &blocking_runtime,
                &blocking_plugin_ids,
                has_pgp_private_key,
                &mut dto,
            );
            (dto, switched_off)
        })
        .await
        .unwrap_or_else(|join_error| {
            tracing::warn!(error = %join_error, "plugin transform task failed");
            (fallback_dto, Vec::new())
        });
        for plugin_id in switched_off {
            persist_automatic_plugin_stop(app, pool, &plugin_id).await;
        }
    }
    if dto.body_text.contains("-----BEGIN PGP MESSAGE-----") {
        dto.body_html = None;
    }

    let mut inner_pgp = None;
    if decrypts_inline_pgp(&trust, &original_body_text) {
        match decrypt_message_with_unlocked_keys(app, state, pool, &original_body_text).await {
            Ok(Some(decrypted)) if !decrypted.plaintext.trim().is_empty() => {
                match decode_inline_pgp_body(trust.clone(), decrypted).await {
                    Ok(body) => {
                        match persist_decrypted_attachments(app, pool, reference, &body.attachments)
                            .await
                        {
                            Ok(bindings) => decrypted_attachments = bindings,
                            Err(error) => {
                                publication_complete = false;
                                tracing::warn!(message_id = %dto.id, %error, "could not cache decrypted PGP attachments")
                            }
                        }
                        dto.snippet = first_non_empty_line(&body.decrypted_text)
                            .unwrap_or(dto.snippet.clone());
                        dto.body_text = body.text;
                        dto.body_segments = Some(body.segments);
                        dto.body_html = body.html;
                        let mut verdict = PgpVerdict::decrypted();
                        for (bundle, root) in &body.signatures {
                            let checked = verify_pgp_signed_bundle(
                                app,
                                pool,
                                &dto.id,
                                &dto.from_address,
                                bundle,
                            )
                            .await;
                            verdict = combine_pgp_verdicts(
                                Some(verdict),
                                scoped(checked, *root && body.whole_body),
                            );
                        }
                        inner_pgp = Some(verdict);
                    }
                    Err(key) => inner_pgp = Some(PgpVerdict::failed(key)),
                }
            }
            Ok(_) => {}
            Err(error) => match pgp_decrypt_failure(&trust, &original_body_text, &error) {
                PgpDecryptFailure::Message(key) => inner_pgp = Some(PgpVerdict::failed(key)),
                PgpDecryptFailure::Block(segments) => {
                    dto.body_text = segments.iter().map(BodySegment::content).collect();
                    dto.body_segments = Some(segments);
                }
            },
        }
    }

    if trust.root_inline_signature
        && original_body_text
            .trim_start()
            .starts_with("-----BEGIN PGP SIGNED MESSAGE-----")
    {
        dto.body_html = None;
        // Bound before the match: the arms mutate the DTO the borrow came from.
        let from_address = dto.from_address.clone();
        match verify_incoming_cleartext_signature(
            app,
            pool,
            &dto.id,
            &original_body_text,
            &from_address,
        )
        .await
        {
            Ok(IncomingPgpSignature::Valid {
                cleartext,
                signer_name,
                signer_fingerprint,
            }) => {
                dto.snippet = first_non_empty_line(&cleartext).unwrap_or(dto.snippet.clone());
                dto.body_text = cleartext;
                dto.pgp_state = Some("signature_valid".to_string());
                dto.pgp_error_detail = None;
                dto.pgp_signer_name = signer_name;
                dto.pgp_signer_fingerprint = Some(signer_fingerprint);
            }
            Ok(IncomingPgpSignature::IdentityMismatch {
                cleartext,
                signer_name,
                signer_fingerprint,
            }) => {
                dto.snippet = first_non_empty_line(&cleartext).unwrap_or(dto.snippet.clone());
                dto.body_text = cleartext;
                dto.pgp_state = Some("signature_identity_mismatch".to_string());
                dto.pgp_error_detail = None;
                // The signer is the point of this state: the reader has to see
                // whose key signed instead of the sender's.
                dto.pgp_signer_name = signer_name;
                dto.pgp_signer_fingerprint = Some(signer_fingerprint);
            }
            Ok(IncomingPgpSignature::Invalid { cleartext }) => {
                dto.snippet = first_non_empty_line(&cleartext).unwrap_or(dto.snippet.clone());
                dto.body_text = cleartext;
                dto.pgp_state = Some("signature_invalid".to_string());
                dto.pgp_signer_name = None;
                dto.pgp_signer_fingerprint = None;
            }
            Ok(IncomingPgpSignature::UnknownKey { cleartext }) => {
                dto.snippet = first_non_empty_line(&cleartext).unwrap_or(dto.snippet.clone());
                dto.body_text = cleartext;
                dto.pgp_state = Some("signature_unknown_key".to_string());
                dto.pgp_signer_name = None;
                dto.pgp_signer_fingerprint = None;
            }
            Err(_) => {
                dto.pgp_state = Some("error".to_string());
                dto.pgp_error_detail =
                    Some(CommandError::new("backend.errors.pgpVerificationFailed").encoded());
                dto.pgp_signer_name = None;
                dto.pgp_signer_fingerprint = None;
            }
        }
    }

    let outer_pgp = match (pgp_display, pgp_signed.as_deref()) {
        (Some(SignedDisplay::Covered), Some(bundle)) => {
            let checked =
                verify_pgp_signed_bundle(app, pool, &dto.id, &dto.from_address, bundle).await;
            Some(scoped(
                checked,
                root_signature_covers_body(&trust, dto.body_segments.as_deref()),
            ))
        }
        (Some(SignedDisplay::Unavailable), _) => Some(PgpVerdict::failed(
            "backend.errors.signedContentUnavailable",
        )),
        // OpenPGP has no opaque form: such a bundle signs nothing shown.
        (Some(SignedDisplay::Encapsulated | SignedDisplay::Malformed), _) => {
            Some(PgpVerdict::failed("backend.errors.signedMessageMalformed"))
        }
        _ => None,
    };
    match (inner_pgp, outer_pgp) {
        (inner, Some(outer)) => combine_pgp_verdicts(inner, outer).apply(&mut dto),
        (Some(inner), None) => inner.apply(&mut dto),
        (None, None) => {}
    }

    if let Some(ciphertext) = smime_encrypted.as_deref() {
        dto.body_html = None;
        match decrypt_message_with_unlocked_smime_certs(app, state, pool, ciphertext).await {
            Ok(Some(decrypted)) => match decode_smime_body(decrypted).await {
                Ok(parsed) => {
                    match persist_decrypted_attachments(
                        app,
                        pool,
                        reference,
                        &parsed.body.attachments,
                    )
                    .await
                    {
                        Ok(bindings) => decrypted_attachments = bindings,
                        Err(error) => {
                            publication_complete = false;
                            tracing::warn!(message_id = %dto.id, %error, "could not cache decrypted S/MIME attachments")
                        }
                    }
                    let segments =
                        decrypted_segments(&parsed.body.text_parts, BodySegment::SmimeDecrypted);
                    dto.body_text = segments.iter().map(BodySegment::content).collect();
                    dto.body_html = parsed.body.html;
                    dto.snippet =
                        first_non_empty_line(&dto.body_text).unwrap_or(dto.snippet.clone());
                    dto.body_segments = Some(segments);
                    dto.smime_state = Some("decrypted".into());
                    dto.smime_error_detail = None;
                    for (payload, root_crypto) in parsed.signatures {
                        let verified =
                            verify_smime_message(app, pool, &dto.id, &payload, &dto.from_address)
                                .await;
                        dto.smime_state = Some(verified.state);
                        apply_signature_scope(&mut dto.smime_state, root_crypto);
                        dto.smime_signer_email = verified.signer_email;
                        dto.smime_signer_cn = verified.signer_cn;
                        dto.smime_error_detail = smime_verification_error(verified.error_detail);
                        if !keeps_verifying_smime(dto.smime_state.as_deref()) {
                            break;
                        }
                    }
                }
                Err(error) => {
                    dto.smime_state = Some("error".into());
                    dto.smime_error_detail = Some(error.encoded());
                }
            },
            Ok(_) => {
                dto.smime_state = Some(if has_smime_private_cert {
                    "cert_available_locked".to_string()
                } else {
                    "encrypted_no_cert".to_string()
                });
                dto.body_text = "S/MIME encrypted message (smime.p7m)".to_string();
            }
            Err(err) => {
                dto.smime_state = Some("error".to_string());
                tracing::warn!(message_id = %dto.id, error = %err, "S/MIME decryption failed");
                dto.smime_error_detail =
                    Some(CommandError::new("backend.errors.smimeDecryptFailed").encoded());
            }
        }
    }

    if let (Some(display), Some(signed_payload)) = (smime_display, smime_signed.as_deref()) {
        let verified = match display {
            SignedDisplay::Covered => {
                verify_smime_message(app, pool, &dto.id, signed_payload, &dto.from_address).await
            }
            SignedDisplay::Encapsulated => smime_encapsulated
                .take()
                .unwrap_or_else(SmimeVerifyMessageResult::content_unavailable),
            SignedDisplay::Unavailable => SmimeVerifyMessageResult::content_unavailable(),
            SignedDisplay::Malformed => SmimeVerifyMessageResult::malformed(),
        };
        dto.smime_state = Some(verified.state);
        apply_signature_scope(
            &mut dto.smime_state,
            root_signature_covers_body(&trust, dto.body_segments.as_deref()),
        );
        dto.smime_signer_email = verified.signer_email;
        dto.smime_signer_cn = verified.signer_cn;
        dto.smime_error_detail = smime_verification_error(verified.error_detail);
    }

    if let Some(body_html) = dto.body_html.take() {
        let sanitized = super::message_processing::sanitize_message_html(dto.id.clone(), body_html)
            .await
            .unwrap_or_else(|error| {
                tracing::warn!(%error, "message sanitizer task failed");
                String::new()
            });
        if !sanitized.trim().is_empty() {
            dto.body_html = Some(sanitized);
        }
    }
    if publication_complete {
        if let Some(segments) = dto.body_segments.as_ref() {
            let referenced = dto
                .body_html
                .as_deref()
                .map(referenced_cids)
                .unwrap_or_default();
            // Include the outer encrypted file and any existing attachments too.
            // The renderer receives this same listing, not only the inner parts.
            match super::attachments::attachments_for_messages(pool, std::slice::from_ref(&dto.id))
                .await
            {
                Ok(rows) => {
                    let attachments = rows
                        .into_iter()
                        .map(|row| {
                            let data_url = provenance_data_url(
                                &row,
                                decrypted_attachments
                                    .iter()
                                    .find(|part| part.id == row.id)
                                    .and_then(|part| part.data_url.as_ref()),
                                &referenced,
                            );
                            DecryptedAttachmentBinding {
                                id: row.id,
                                filename: row.filename.unwrap_or_default(),
                                content_type: row.content_type,
                                size: row.size.unwrap_or(0),
                                content_id: row.content_id,
                                is_inline: row.is_inline != 0,
                                cached_path: row.cached_path,
                                data_url,
                            }
                        })
                        .collect();
                    dto.body_provenance = Some(BodyProvenance {
                        segments: segments.clone(),
                        html: dto.body_html.clone(),
                        attachments,
                    });
                }
                Err(error) => tracing::warn!(%error, "could not bind decrypted attachment listing"),
            }
        }
    }
    dto
}

/// Whether the row stores a body at all. A stored empty string counts as absent.
fn row_has_body(row: &MessageReadRecord) -> bool {
    let present = |value: Option<&str>| value.is_some_and(|value| !value.trim().is_empty());
    present(row.body_text.as_deref()) || present(row.body_html.as_deref())
}

/// What a stored signature may be shown next to, once the DTO holds the body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SignedDisplay {
    /// The DTO shows what the signature covers.
    Covered,
    /// Opaque S/MIME: only the engine reads the content, while verifying.
    Encapsulated,
    /// The signed entity displays nothing, so no verdict may stand over it.
    Unavailable,
    /// The bundle does not split; it signs nothing shown.
    Malformed,
}

/// Put on the DTO the body a stored signature covers, before it is verified.
///
/// A row the strict splitter wrote with its bundle keeps its body. Every other
/// root-signed row shows the entity decoded from the bundle, the bytes
/// verification reads, so display and verdict cannot part.
async fn show_signed_body(
    dto: &mut MessageDto,
    bundle: &[u8],
    trust: &MessageTrust,
    has_stored_body: bool,
) -> SignedDisplay {
    static CACHE: std::sync::LazyLock<
        std::sync::Mutex<
            super::message_processing::ProcessingCache<
                Result<(StoredSignedBody, String), SignedMimeError>,
            >,
        >,
    > = std::sync::LazyLock::new(|| std::sync::Mutex::new(Default::default()));
    let message_id = dto.id.clone();
    let subject = dto.subject.clone();
    let bundle = bundle.to_vec();
    let trust = trust.clone();
    let shown = tokio::task::spawn_blocking(move || {
        let fingerprint = super::message_processing::fingerprint(&[
            &bundle,
            &[
                u8::from(trust.root_crypto),
                u8::from(trust.signed_binding),
                u8::from(has_stored_body),
            ],
            subject.as_bytes(),
        ]);
        super::message_processing::cached(&CACHE, message_id, fingerprint, || {
            let shown =
                signed_mime::stored_signed_body(&bundle, &trust, has_stored_body).map(|shown| {
                    let snippet = match &shown {
                        StoredSignedBody::Entity(entity) => {
                            build_snippet(&subject, entity.text.as_deref(), entity.html.as_deref())
                        }
                        _ => String::new(),
                    };
                    (shown, snippet)
                });
            (shown, bundle.len())
        })
    })
    .await
    .unwrap_or_else(|error| {
        tracing::warn!(%error, "signed body task failed");
        Err(SignedMimeError::NotSigned)
    });
    match shown {
        Ok((StoredSignedBody::Entity(entity), _))
            if shows_nothing(
                entity.text.as_deref().unwrap_or_default(),
                entity.html.as_deref(),
            ) =>
        {
            show_entity(dto, EntityDisplay::default(), String::new());
            dto.snippet.clear();
            SignedDisplay::Unavailable
        }
        Ok((StoredSignedBody::Entity(entity), snippet)) => {
            show_entity(dto, entity, snippet);
            SignedDisplay::Covered
        }
        Ok((StoredSignedBody::Stored, _)) => SignedDisplay::Covered,
        Ok((StoredSignedBody::Encapsulated, _)) => SignedDisplay::Encapsulated,
        Err(error) => {
            tracing::debug!(message_id = %dto.id, %error, "stored signed bundle rejected");
            SignedDisplay::Malformed
        }
    }
}

/// Whether a signed display has nothing for a verdict to stand over.
fn shows_nothing(text: &str, html: Option<&str>) -> bool {
    text.trim().is_empty() && html.is_none_or(|html| html.trim().is_empty())
}

/// The DTO shows exactly this signed entity: nothing of the stored body, the
/// snippet or the subject stands in for a part it does not display.
fn show_entity(dto: &mut MessageDto, entity: EntityDisplay, snippet: String) {
    dto.body_text = entity.text.unwrap_or_default();
    dto.body_html = entity.html;
    dto.body_segments = None;
    if !snippet.is_empty() {
        dto.snippet = snippet;
    }
    // A body fetched again would not be what this verdict covers.
    dto.body_loaded = true;
}

/// Opaque S/MIME carries its content inside the signature. Show what the
/// engine read there while verifying, or no body and no verdict.
async fn show_encapsulated_body(
    dto: &mut MessageDto,
    mut verified: SmimeVerifyMessageResult,
) -> SmimeVerifyMessageResult {
    let content = verified.content.take();
    let entity = tokio::task::spawn_blocking(move || {
        content
            .as_deref()
            .map(signed_mime::display_entity)
            .filter(|entity| entity.text.is_some() || entity.html.is_some())
    })
    .await
    .ok()
    .flatten();
    match entity {
        Some(entity) => {
            let snippet = entity
                .text
                .as_deref()
                .and_then(first_non_empty_line)
                .unwrap_or_default();
            show_entity(dto, entity, snippet);
            verified
        }
        None => {
            show_entity(dto, EntityDisplay::default(), String::new());
            if verified.state == "error" {
                verified
            } else {
                SmimeVerifyMessageResult::content_unavailable()
            }
        }
    }
}

/// Whether `map_message_row` tries the unlocked keys on this stored body.
///
/// Armor after a cover note is decrypted too: the segments keep the cover and
/// the suffix as plain text, so only the block itself is captioned. A
/// cleartext-signed body belongs to the signature branch, which replaces it.
pub(super) fn decrypts_inline_pgp(
    trust: &crate::providers::traits::MessageTrust,
    stored_body: &str,
) -> bool {
    !trust.root_inline_signature && stored_body.contains("-----BEGIN PGP MESSAGE-----")
}

/// The display body of a decrypted inline OpenPGP block.
pub(super) struct InlinePgpBody {
    pub text: String,
    pub segments: Vec<BodySegment>,
    pub html: Option<String>,
    /// The readable decrypted content, without cover or suffix.
    pub decrypted_text: String,
    /// The block was the whole body, so its HTML and signature may stand for
    /// the message.
    pub whole_body: bool,
    pub attachments: Vec<crate::providers::traits::RemoteAttachment>,
    /// OpenPGP `multipart/signed` bundles in the plaintext, each with whether
    /// it covers the whole decrypted content.
    pub signatures: Vec<(Vec<u8>, bool)>,
}

pub(super) fn inline_pgp_body(
    trust: &crate::providers::traits::MessageTrust,
    decrypted: crate::commands::pgp::DecryptedIncomingPgp,
) -> Result<InlinePgpBody, satchel_core::email::decrypted::DecodeError> {
    // Only a block that is the entire root may speak for the message. As the
    // text part of a multipart mail it has unencrypted parts beside it.
    let whole_body = trust.root_encryption
        && decrypted.prefix.trim().is_empty()
        && decrypted.suffix.trim().is_empty();
    let (parts, html, attachments, signatures) = if declares_mime(&decrypted.plaintext) {
        // PGP/MIME: the plaintext has no second copy either, so it gets the
        // completeness check S/MIME gets. S/MIME signatures inside it are
        // shown as content but not verified, so they record nothing.
        let decoded =
            satchel_core::email::decrypted::parse_decrypted_body(decrypted.plaintext.as_bytes())?;
        (
            decoded.body.text_parts,
            decoded.body.html,
            decoded.body.attachments,
            decoded.pgp_signatures,
        )
    } else {
        (vec![decrypted.plaintext], None, Vec::new(), Vec::new())
    };
    let mut segments = vec![BodySegment::Text(decrypted.prefix)];
    segments.extend(decrypted_segments(&parts, BodySegment::PgpDecrypted));
    segments.push(BodySegment::Text(decrypted.suffix));
    Ok(InlinePgpBody {
        text: segments.iter().map(BodySegment::content).collect(),
        segments,
        html: html.filter(|html| whole_body && !html.trim().is_empty()),
        decrypted_text: parts.join(satchel_core::email::decrypted::PART_SEPARATOR),
        whole_body,
        attachments,
        signatures,
    })
}

/// One captioned segment per shown part, so a signed part never merges into
/// an unsigned one. An entity that shows no text still gets its caption.
fn decrypted_segments(parts: &[String], caption: fn(String) -> BodySegment) -> Vec<BodySegment> {
    if parts.is_empty() {
        return vec![caption(String::new())];
    }
    let mut segments = Vec::with_capacity(parts.len() * 2);
    for (index, part) in parts.iter().enumerate() {
        if index > 0 {
            segments.push(BodySegment::Text(
                satchel_core::email::decrypted::PART_SEPARATOR.into(),
            ));
        }
        segments.push(caption(part.clone()));
    }
    segments
}

/// Decode off the async worker: sanitizing a large HTML part is CPU-bound.
async fn decode_inline_pgp_body(
    trust: crate::providers::traits::MessageTrust,
    decrypted: crate::commands::pgp::DecryptedIncomingPgp,
) -> Result<InlinePgpBody, &'static str> {
    tokio::task::spawn_blocking(move || inline_pgp_body(&trust, decrypted))
        .await
        .map_err(|error| {
            tracing::warn!(%error, "OpenPGP decode task failed");
            "backend.errors.pgpDecodeFailed"
        })?
        .map_err(|error| {
            tracing::warn!(%error, "OpenPGP decode failed");
            pgp_decode_error_key(&error)
        })
}

pub(super) fn pgp_decode_error_key(
    error: &satchel_core::email::decrypted::DecodeError,
) -> &'static str {
    use satchel_core::email::decrypted::DecodeError;
    match error {
        DecodeError::NestingLimit => "backend.errors.pgpNestingLimit",
        DecodeError::RemainingEncryption => "backend.errors.pgpRemainingEncryption",
        DecodeError::MalformedSigned => "backend.errors.pgpMalformedSigned",
        DecodeError::MalformedMime => "backend.errors.pgpMalformedMime",
        DecodeError::UndecodedPart => "backend.errors.pgpUndecodedPart",
        DecodeError::InvalidSignedContent(_) => "backend.errors.pgpDecodeFailed",
    }
}

/// What a failed inline decryption changes.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum PgpDecryptFailure {
    /// The block is the message, so its failure is the message's.
    Message(&'static str),
    /// A block among other text, such as one quoted in a reply. The message
    /// is not an encrypted one that failed; the block's segment says it was
    /// not decrypted.
    Block(Vec<BodySegment>),
}

pub(super) fn pgp_decrypt_failure(
    trust: &crate::providers::traits::MessageTrust,
    stored_body: &str,
    error: &IncomingPgpDecryptError,
) -> PgpDecryptFailure {
    match error {
        IncomingPgpDecryptError::SeveralBlocks => {
            PgpDecryptFailure::Message("backend.errors.pgpSeveralEncryptedBlocks")
        }
        // `root_encryption` holds exactly when the block is the whole body.
        IncomingPgpDecryptError::Failed(_) if !trust.root_encryption => {
            match crate::commands::pgp::split_at_single_block(stored_body) {
                Some((cover, block, suffix)) => PgpDecryptFailure::Block(vec![
                    BodySegment::Text(cover),
                    BodySegment::PgpUndecrypted(block),
                    BodySegment::Text(suffix),
                ]),
                None => PgpDecryptFailure::Message("backend.errors.pgpDecryptFailed"),
            }
        }
        IncomingPgpDecryptError::Failed(_) => {
            PgpDecryptFailure::Message("backend.errors.pgpDecryptFailed")
        }
    }
}

/// Whether the root signature covers what the reader shows.
///
/// A root signature over a body in which a block was decrypted signs the
/// ciphertext, not the plaintext on screen: whoever signed may never have
/// read it. That is the surreptitious-forwarding shape, so the claim drops
/// to a partial one.
pub(super) fn root_signature_covers_body(
    trust: &crate::providers::traits::MessageTrust,
    segments: Option<&[BodySegment]>,
) -> bool {
    trust.root_crypto
        && !segments.is_some_and(|segments| {
            segments.iter().any(|segment| {
                matches!(
                    segment,
                    BodySegment::PgpDecrypted(_) | BodySegment::SmimeDecrypted(_)
                )
            })
        })
}

/// The inline image bytes a binding carries, so the renderer can check the
/// bytes it hydrated against the decrypted ones.
///
/// The renderer hydrates only images: inline ones, and those an image in the
/// HTML names by Content-ID or file name (`isNeededInlineImage` in
/// `src/stores/messages.ts`). Any other part's bytes would be a second copy in
/// the payload that nothing reads.
pub(super) fn provenance_data_url(
    row: &super::attachments::AttachmentDto,
    data_url: Option<&String>,
    referenced: &std::collections::HashSet<String>,
) -> Option<String> {
    let image = row.content_type.to_ascii_lowercase().starts_with("image/");
    let named = || {
        let content_id = row.content_id.as_deref().map(clean_cid);
        content_id
            .iter()
            .flat_map(|cid| [clean_cid(cid), clean_cid(&format!("<{cid}>"))])
            .chain(row.filename.as_deref().map(clean_cid))
            .any(|candidate| referenced.contains(&candidate.to_lowercase()))
    };
    data_url
        .filter(|_| image && (row.is_inline != 0 || named()))
        .cloned()
}

/// What `<img src="cid:…">` references the HTML holds, spelled as
/// `referencedInlineCids` in `src/utils/emailInlineImages.ts` spells them.
pub(super) fn referenced_cids(html: &str) -> std::collections::HashSet<String> {
    let has_cid = html
        .as_bytes()
        .windows(4)
        .any(|window| window.eq_ignore_ascii_case(b"cid:"));
    if !has_cid {
        return Default::default();
    }
    let document = scraper::Html::parse_document(html);
    let images = scraper::Selector::parse("img[src]").expect("static selector");
    document
        .select(&images)
        .filter_map(|image| image.value().attr("src"))
        .filter(|source| {
            source
                .get(..4)
                .is_some_and(|scheme| scheme.eq_ignore_ascii_case("cid:"))
        })
        .map(|source| clean_cid(source).to_lowercase())
        .collect()
}

/// `cleanCid` in `src/utils/emailInlineImages.ts`: without the `cid:` scheme
/// and angle brackets, percent-decoded where that is valid.
fn clean_cid(value: &str) -> String {
    let value = value.trim();
    let value = match value.get(..4) {
        Some(scheme) if scheme.eq_ignore_ascii_case("cid:") => &value[4..],
        _ => value,
    };
    let value = value.strip_prefix('<').unwrap_or(value);
    let value = value.strip_suffix('>').unwrap_or(value).trim();
    percent_decoded(value).unwrap_or_else(|| value.to_string())
}

/// `decodeURIComponent`: `None` where it would throw.
fn percent_decoded(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = bytes.get(index + 1..index + 3)?;
            if !hex.iter().all(u8::is_ascii_hexdigit) {
                return None;
            }
            decoded.push(u8::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

/// Inline plaintext is usually bare text. Read as MIME, its first blank line
/// would end a header block and drop the paragraph above it, so only a
/// plaintext that opens with a header field declaring MIME is parsed as such.
fn declares_mime(plaintext: &str) -> bool {
    let opens_with_field = plaintext
        .lines()
        .next()
        .and_then(|line| line.split_once(':'))
        .is_some_and(|(name, _)| {
            !name.is_empty()
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        });
    opens_with_field
        && (crate::providers::imap::parse_header_value(plaintext, "Content-Type").is_some()
            || crate::providers::imap::parse_header_value(plaintext, "MIME-Version").is_some())
}

async fn decode_smime_body(
    bytes: Vec<u8>,
) -> CommandResult<satchel_core::email::decrypted::DecryptedBody> {
    tokio::task::spawn_blocking(move || {
        satchel_core::email::decrypted::parse_decrypted_body(&bytes)
    })
    .await
    .map_err(|error| {
        tracing::warn!(%error, "S/MIME decode task failed");
        CommandError::new("backend.errors.smimeDecodeFailed")
    })?
    .map_err(|error| {
        use satchel_core::email::decrypted::DecodeError;
        let key = match &error {
            DecodeError::NestingLimit => "backend.errors.smimeNestingLimit",
            DecodeError::RemainingEncryption => "backend.errors.smimeRemainingEncryption",
            DecodeError::MalformedSigned => "backend.errors.smimeMalformedSigned",
            DecodeError::MalformedMime => "backend.errors.smimeMalformedMime",
            DecodeError::UndecodedPart => "backend.errors.smimeUndecodedPart",
            DecodeError::InvalidSignedContent(_) => "backend.errors.smimeDecodeFailed",
        };
        tracing::warn!(%error, "S/MIME decode failed");
        CommandError::new(key)
    })
}

/// Whether the next decrypted S/MIME signature still needs checking. A part
/// signed validly does not end the check: another part's signature may fail,
/// and the failure is what the reader must see.
fn keeps_verifying_smime(state: Option<&str>) -> bool {
    matches!(state, Some("signature_valid" | "signature_partial"))
}

/// The error a verdict carries: a message key, never the engine's own text.
/// A key the verification already chose, such as a refused structure, stays.
fn smime_verification_error(detail: Option<String>) -> Option<String> {
    detail.map(|error| match CommandError::from(error.clone()) {
        keyed if keyed.key != "backend.errors.unexpected" => keyed.encoded(),
        _ => {
            tracing::warn!(%error, "S/MIME verification failed");
            CommandError::new("backend.errors.smimeVerificationFailed").encoded()
        }
    })
}

fn apply_signature_scope(state: &mut Option<String>, root_crypto: bool) {
    if !root_crypto
        && matches!(
            state.as_deref(),
            Some("signature_valid" | "signature_unchained")
        )
    {
        *state = Some("signature_partial".into());
    }
}

/// One OpenPGP outcome for the displayed body, before it reaches the DTO.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PgpVerdict {
    pub state: String,
    pub error_detail: Option<String>,
    pub signer_name: Option<String>,
    pub signer_fingerprint: Option<String>,
}

impl PgpVerdict {
    fn decrypted() -> Self {
        Self {
            state: "decrypted".into(),
            error_detail: None,
            signer_name: None,
            signer_fingerprint: None,
        }
    }

    /// Content that could not be decrypted or decoded. It is the content's own
    /// verdict and the worst one, so a valid outer signature never replaces it.
    pub(super) fn failed(key: &'static str) -> Self {
        Self {
            state: "error".into(),
            error_detail: Some(CommandError::new(key).encoded()),
            signer_name: None,
            signer_fingerprint: None,
        }
    }

    fn apply(self, dto: &mut MessageDto) {
        dto.pgp_state = Some(self.state);
        dto.pgp_error_detail = self.error_detail;
        dto.pgp_signer_name = self.signer_name;
        dto.pgp_signer_fingerprint = self.signer_fingerprint;
    }
}

pub(super) fn pgp_detached_verdict(
    verification: Result<IncomingPgpDetachedSignature, String>,
) -> PgpVerdict {
    let verdict = |state: &str, signer: Option<(Option<String>, String)>| {
        let (signer_name, signer_fingerprint) = match signer {
            Some((name, fingerprint)) => (name, Some(fingerprint)),
            None => (None, None),
        };
        PgpVerdict {
            state: state.into(),
            error_detail: None,
            signer_name,
            signer_fingerprint,
        }
    };
    match verification {
        Ok(IncomingPgpDetachedSignature::Valid {
            signer_name,
            signer_fingerprint,
        }) => verdict("signature_valid", Some((signer_name, signer_fingerprint))),
        Ok(IncomingPgpDetachedSignature::IdentityMismatch {
            signer_name,
            signer_fingerprint,
        }) => verdict(
            "signature_identity_mismatch",
            Some((signer_name, signer_fingerprint)),
        ),
        Ok(IncomingPgpDetachedSignature::Invalid) => verdict("signature_invalid", None),
        Ok(IncomingPgpDetachedSignature::UnknownKey) => verdict("signature_unknown_key", None),
        Err(_) => PgpVerdict {
            error_detail: Some(CommandError::new("backend.errors.pgpVerificationFailed").encoded()),
            ..verdict("error", None)
        },
    }
}

pub(super) fn scoped(mut verdict: PgpVerdict, covers_whole_body: bool) -> PgpVerdict {
    let mut state = Some(verdict.state);
    apply_signature_scope(&mut state, covers_whole_body);
    verdict.state = state.unwrap_or_default();
    verdict
}

/// How much of a claim fails, so a weaker claim never hides a worse one.
fn pgp_severity(state: &str) -> u8 {
    match state {
        "error" => 6,
        "signature_invalid" => 5,
        "signature_identity_mismatch" => 4,
        "signature_unknown_key" => 3,
        "signature_partial" => 2,
        "signature_valid" => 1,
        _ => 0,
    }
}

/// The state shown for the decrypted content's own verdict and an outer one.
///
/// The outer signature does not overwrite what the content says about itself:
/// the worse claim wins, and a tie keeps the content's own, with its signer.
pub(super) fn combine_pgp_verdicts(inner: Option<PgpVerdict>, outer: PgpVerdict) -> PgpVerdict {
    match inner {
        Some(inner) if pgp_severity(&inner.state) >= pgp_severity(&outer.state) => inner,
        _ => outer,
    }
}

async fn verify_pgp_signed_bundle(
    app: &tauri::AppHandle,
    pool: &SqlitePool,
    message_id: &str,
    from_address: &str,
    signed_bundle: &[u8],
) -> PgpVerdict {
    let parsed = match crate::email::pgp_mime::parse_signed_message(signed_bundle) {
        Ok(parsed) => parsed,
        Err(error) => {
            // No key is consulted for a structure that does not tie the
            // signature to the displayed entity.
            tracing::debug!(%message_id, %error, "OpenPGP signed structure rejected");
            return PgpVerdict::failed("backend.errors.signedMessageMalformed");
        }
    };
    pgp_detached_verdict(
        verify_incoming_detached_signature(
            app,
            pool,
            message_id,
            &parsed.signed_entity,
            &parsed.armored_signature,
            from_address,
        )
        .await,
    )
}

async fn persist_decrypted_attachments(
    app: &tauri::AppHandle,
    pool: &SqlitePool,
    reference: &ContentReference,
    attachments: &[crate::providers::traits::RemoteAttachment],
) -> Result<Vec<DecryptedAttachmentBinding>, String> {
    if attachments.is_empty() {
        return Ok(Vec::new());
    }

    let cache_dir = app
        .path()
        .resolve("satchel/attachments", tauri::path::BaseDirectory::AppData)
        .map_err(|error| error.to_string())?;
    persist_decrypted_attachments_in_dir(&cache_dir, pool, reference, attachments).await
}

async fn persist_decrypted_attachments_in_dir(
    cache_dir: &std::path::Path,
    pool: &SqlitePool,
    reference: &ContentReference,
    attachments: &[crate::providers::traits::RemoteAttachment],
) -> Result<Vec<DecryptedAttachmentBinding>, String> {
    let message_id = &reference.message_id;
    tokio::fs::create_dir_all(cache_dir)
        .await
        .map_err(|error| format!("create attachment cache: {error}"))?;
    let now = chrono::Utc::now().to_rfc3339();
    let cached_paths = sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT id, cached_path FROM attachments WHERE message_id = ?",
    )
    .bind(message_id)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?
    .into_iter()
    .collect::<std::collections::HashMap<_, _>>();

    let mut bindings = Vec::new();
    for attachment in attachments {
        let Some(encoded) = attachment.content_base64.as_deref() else {
            continue;
        };
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|error| format!("decode decrypted attachment: {error}"))?;
        let attachment_id =
            satchel_core::email::decrypted::attachment_id(message_id, &attachment.remote_part_id);
        let filename = attachment
            .filename
            .clone()
            .unwrap_or_else(|| "attachment".to_string());
        let cached = cached_paths.get(&attachment_id).and_then(Option::as_deref);
        let reusable = if let Some(path) = cached {
            reference.owns_attachment_cache(path, &attachment_id)
                && crate::commands::attachments::validated_cached_file_size(cache_dir, path)
                    .await
                    .is_some()
                && tokio::fs::read(path)
                    .await
                    .is_ok_and(|saved| saved == bytes)
        } else {
            false
        };
        // Published files are immutable. Repeated decryption can reuse identical
        // bytes; a replacement request owns a fresh file until publication.
        let (path, staged) = if let Some(path) = cached.filter(|_| reusable) {
            (path.to_string(), false)
        } else {
            let path = crate::commands::attachments::attachment_cache_path(
                cache_dir,
                &reference.attachment_cache_id(&attachment_id),
                &format!("{}-{filename}", uuid::Uuid::new_v4()),
            );
            tokio::fs::write(&path, &bytes)
                .await
                .map_err(|error| format!("write decrypted attachment cache: {error}"))?;
            (path.to_string_lossy().to_string(), true)
        };
        // No remote part: the decoder numbers parts of the plaintext, and the
        // same number names a part of the encrypted message on the server.
        // Sync would match the row to that part, and a download would fetch
        // its ciphertext under this file's name. The row is written under the
        // current listing rules; the signed-entity rules for rows beside a
        // root signature do not apply to it (`decrypted::is_attachment_id`).
        let publication: Result<(), String> = async {
            let mut tx = reference.begin_write(pool).await?;
            sqlx::query(
                "INSERT INTO attachments
             (id, message_id, filename, content_type, size, content_id, storage_path, is_inline,
              cached_path, remote_uid, remote_part_id, created_at, listing_rules)
             VALUES (?, ?, ?, ?, ?, ?, NULL, ?, ?, NULL, NULL, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
               filename = excluded.filename,
               content_type = excluded.content_type,
               size = excluded.size,
               content_id = excluded.content_id,
               is_inline = excluded.is_inline,
               cached_path = excluded.cached_path,
               remote_uid = NULL, remote_part_id = NULL, content_base64 = NULL,
               listing_rules = excluded.listing_rules",
            )
            .bind(&attachment_id)
            .bind(message_id)
            .bind(&filename)
            .bind(&attachment.content_type)
            .bind(i64::try_from(bytes.len()).unwrap_or(i64::MAX))
            .bind(attachment.content_id.as_deref())
            .bind(if attachment.is_inline { 1 } else { 0 })
            .bind(&path)
            .bind(&now)
            .bind(satchel_core::db::signed_attachments::LISTING_RULES)
            .execute(&mut *tx)
            .await
            .map_err(|error| error.to_string())?;
            tx.commit().await.map_err(|error| error.to_string())
        }
        .await;
        if publication.is_err() && staged {
            let _ = tokio::fs::remove_file(&path).await;
        }
        publication?;
        bindings.push(DecryptedAttachmentBinding {
            id: attachment_id,
            filename,
            content_type: attachment.content_type.clone(),
            size: i64::try_from(bytes.len()).unwrap_or(i64::MAX),
            content_id: attachment.content_id.clone(),
            is_inline: attachment.is_inline,
            cached_path: Some(path),
            // Only an image can be hydrated inline; see `provenance_data_url`.
            data_url: attachment
                .content_type
                .to_ascii_lowercase()
                .starts_with("image/")
                .then(|| format!("data:{};base64,{encoded}", attachment.content_type)),
        });
    }

    let mut tx = reference.begin_write(pool).await?;
    sqlx::query("UPDATE messages SET has_attachments = 1 WHERE id = ?")
        .bind(message_id)
        .execute(&mut *tx)
        .await
        .map_err(|error| error.to_string())?;
    sqlx::query(
        "UPDATE threads SET has_attachments = 1
         WHERE id = (SELECT thread_id FROM messages WHERE id = ?)",
    )
    .bind(message_id)
    .execute(&mut *tx)
    .await
    .map_err(|error| error.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(bindings)
}

fn map_message_preview_row(row: MessagePreviewRecord) -> MessageDto {
    let subject = repair_likely_utf8_mojibake(&decode_mime_encoded_words(
        &row.subject.unwrap_or_else(|| "(no subject)".into()),
    ));
    let snippet = {
        let value = repair_likely_utf8_mojibake(&decode_html_entities(&decode_mime_encoded_words(
            &row.snippet.unwrap_or_default(),
        )));
        if value.trim().is_empty() {
            subject.clone()
        } else {
            value
        }
    };
    let from_address = decode_mime_encoded_words(
        row.from_address
            .as_deref()
            .unwrap_or("unknown@satchel.local"),
    );
    let from_name = resolve_sender_display_name(row.from_name.as_deref(), &from_address);

    MessageDto {
        id: row.id,
        remote_uidvalidity: saved_uidvalidity(row.remote_uidvalidity),
        content_error: None,
        thread_id: row.thread_id.unwrap_or_default(),
        subject,
        snippet: snippet.clone(),
        from_address,
        from_name,
        to_addresses: parse_address_json(row.to_addresses),
        cc_addresses: parse_address_json(row.cc_addresses),
        reply_to: row.reply_to,
        date: row.date.unwrap_or_else(|| chrono::Utc::now().to_rfc3339()),
        body_text: snippet,
        body_segments: None,
        body_provenance: None,
        body_html: None,
        body_loaded: false,
        is_read: row.is_read.unwrap_or(1) > 0,
        is_draft: row.is_draft.unwrap_or(0) > 0,
        pgp_state: None,
        pgp_error_detail: None,
        pgp_signer_name: None,
        pgp_signer_fingerprint: None,
        smime_state: None,
        smime_signer_email: None,
        smime_signer_cn: None,
        smime_error_detail: None,
        list_unsubscribe: None,
        list_unsubscribe_post: None,
        auth_spf: None,
        auth_dkim: None,
        auth_dmarc: None,
        source_payload: None,
    }
}

fn message_requires_plugin_transform(body_text: &str) -> bool {
    let text = body_text.trim();
    text.contains("-----BEGIN PGP SIGNED MESSAGE-----")
        || text.contains("-----BEGIN PGP MESSAGE-----")
}

/// Remembers a plugin the runtime switched off and tells the user.
async fn persist_automatic_plugin_stop(app: &tauri::AppHandle, pool: &SqlitePool, plugin_id: &str) {
    tracing::warn!(plugin_id, "plugin switched off after repeated timeouts");
    if let Err(error) = crate::commands::plugins::store_plugin_disabled(
        pool,
        plugin_id,
        Some(crate::plugins::registry::PluginDisabledReason::Timeouts),
    )
    .await
    {
        tracing::warn!(?error, plugin_id, "automatic plugin stop not persisted");
    }
    let _ = tauri::Emitter::emit(
        app,
        "plugin:auto-disabled",
        serde_json::json!({ "plugin_id": plugin_id }),
    );
}

/// Runs the transforms in order; returns the plugins this run switched off.
///
/// A failing plugin leaves the message as it was and says why in
/// `pgp_error_detail`, instead of showing the raw armor without a word.
fn apply_message_plugins(
    runtime: &PluginRuntime,
    plugin_ids: &[String],
    has_pgp_private_key: bool,
    dto: &mut MessageDto,
) -> Vec<String> {
    // Detection and decryption belong to the native engine. A denied or absent
    // optional plugin must not hide the key import/unlock action. The caller
    // has already checked that the ciphertext is the trusted root entity.
    if dto
        .body_text
        .trim_start()
        .starts_with("-----BEGIN PGP MESSAGE-----")
    {
        dto.pgp_state = Some(
            if has_pgp_private_key {
                "key_available_locked"
            } else {
                "encrypted_no_key"
            }
            .into(),
        );
    }
    let mut switched_off = Vec::new();
    for plugin_id in plugin_ids {
        let input = MessageTransformInput {
            subject: dto.subject.clone(),
            snippet: dto.snippet.clone(),
            body_text: dto.body_text.clone(),
            body_html: dto.body_html.clone(),
            has_pgp_private_key,
        };

        let output = match runtime.transform_message(plugin_id, &input) {
            Ok(value) => {
                runtime.record_transform_outcome(plugin_id, None);
                value
            }
            Err(error) => {
                if runtime.record_transform_outcome(plugin_id, Some(&error)) {
                    switched_off.push(plugin_id.clone());
                }
                if dto.pgp_error_detail.is_none() {
                    dto.pgp_error_detail = Some(format!("{plugin_id}: {error}"));
                }
                continue;
            }
        };

        if let Some(subject) = output.subject.filter(|value| !value.trim().is_empty()) {
            dto.subject = subject;
        }
        if let Some(snippet) = output.snippet.filter(|value| !value.trim().is_empty()) {
            dto.snippet = snippet;
        }
        if let Some(body_text) = output.body_text.filter(|value| !value.trim().is_empty()) {
            dto.body_text = body_text;
        }
        if let Some(body_html) = output.body_html.filter(|value| !value.trim().is_empty()) {
            // Stored mail HTML was sanitized on ingest; this HTML never was.
            dto.body_html = Some(sanitize_email_html(&body_html));
        }
        if let Some(state) = output.pgp_state.filter(|value| !value.trim().is_empty()) {
            dto.pgp_state = Some(state);
        }
        if let Some(detail) = output
            .pgp_error_detail
            .filter(|value| !value.trim().is_empty())
        {
            dto.pgp_error_detail = Some(detail);
        }
        if let Some(name) = output
            .pgp_signer_name
            .filter(|value| !value.trim().is_empty())
        {
            dto.pgp_signer_name = Some(name);
        }
        if let Some(fingerprint) = output
            .pgp_signer_fingerprint
            .filter(|value| !value.trim().is_empty())
        {
            dto.pgp_signer_fingerprint = Some(fingerprint);
        }
    }
    switched_off
}

async fn has_any_private_pgp_key(pool: &SqlitePool) -> bool {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(1) FROM pgp_keys WHERE key_type = 'private'")
        .fetch_one(pool)
        .await
        .map(|count| count > 0)
        .unwrap_or(false)
}

async fn has_any_private_smime_cert(pool: &SqlitePool) -> bool {
    match SmimeCertificateRepository::new(pool)
        .has_private_certificate()
        .await
    {
        Ok(has_private_certificate) => has_private_certificate,
        Err(error) => {
            tracing::error!(error = ?error, "S/MIME certificate lookup failed");
            false
        }
    }
}

fn first_non_empty_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.to_string())
}

fn parse_address_json(raw: Option<String>) -> Vec<String> {
    let Some(raw) = raw else {
        return vec![];
    };

    if let Ok(values) = serde_json::from_str::<Vec<String>>(&raw) {
        let mut normalized = Vec::new();
        for value in values {
            let decoded = decode_mime_encoded_words(value.trim());
            let item = extract_email_address(&decoded).unwrap_or(decoded);
            if !item.is_empty() && !normalized.iter().any(|existing| existing == &item) {
                normalized.push(item);
            }
        }
        return normalized;
    }

    let decoded = decode_mime_encoded_words(raw.trim());
    if decoded.is_empty() {
        vec![]
    } else {
        vec![extract_email_address(&decoded).unwrap_or(decoded)]
    }
}

/// Total attachment budget, mirrored in `AttachmentDropZone.vue`.
///
/// Enforced here as well because the composer is not the only caller: the local
/// HTTP API and the CLI reach `send_message` directly.
const MAX_TOTAL_ATTACHMENT_BYTES: usize = 20 * 1024 * 1024;

fn ensure_attachments_within_limit(attachments: &[OutgoingAttachmentInput]) -> Result<(), String> {
    // base64 expands by 4/3; compare decoded sizes against the budget.
    let total: usize = attachments
        .iter()
        .map(|attachment| attachment.content_base64.len() / 4 * 3)
        .sum();

    if total > MAX_TOTAL_ATTACHMENT_BYTES {
        return Err(format!(
            "attachments total {:.1} MB, limit is {:.0} MB",
            total as f64 / (1024.0 * 1024.0),
            MAX_TOTAL_ATTACHMENT_BYTES as f64 / (1024.0 * 1024.0),
        ));
    }

    Ok(())
}

/// Whether an address may appear in `From:` for this account.
///
/// The identity picker offers the primary address and the account's aliases and
/// nothing else, so a request naming anything else raced an alias deletion or
/// never came from the picker. Submission servers answer an unowned MAIL FROM
/// with `553 Sender address rejected`, and Gmail rewrites it silently — the
/// mail then leaves under an identity the user did not choose.
fn is_owned_sender(candidate: &str, primary: &str, aliases: &[String]) -> bool {
    let candidate = candidate.trim();
    if candidate.is_empty() {
        return false;
    }
    std::iter::once(primary)
        .chain(aliases.iter().map(String::as_str))
        .any(|owned| owned.trim().eq_ignore_ascii_case(candidate))
}

async fn ensure_sender_is_owned(
    pool: &SqlitePool,
    account_id: &str,
    from_address: Option<&str>,
) -> CommandResult<()> {
    let Some(candidate) = from_address
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(());
    };
    let primary =
        sqlx::query_scalar::<_, String>("SELECT email FROM accounts WHERE id = ? LIMIT 1")
            .bind(account_id)
            .fetch_optional(pool)
            .await
            .map_err(CommandError::unexpected)?
            .unwrap_or_default();
    let aliases = AliasRepository::new(pool)
        .list(Some(account_id))
        .await
        .map_err(|error| CommandError::new(crate::commands::aliases::alias_error_key(error)))?
        .into_iter()
        .map(|alias| alias.email)
        .collect::<Vec<_>>();

    if is_owned_sender(candidate, &primary, &aliases) {
        return Ok(());
    }
    Err(CommandError::new("backend.errors.senderAddressNotOwned")
        .with_param("address", candidate.to_string()))
}

#[tauri::command]
pub async fn send_message(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    pool: tauri::State<'_, SqlitePool>,
    account_id: String,
    input: OutgoingMessageInput,
) -> CommandResult<SendMessageResult> {
    let mut input = input;
    // Any of To, Cc and Bcc is enough: all three go into the envelope.
    let has_recipient = !normalize_recipients(&input.to)?.is_empty()
        || !normalize_recipients(&input.cc)?.is_empty()
        || !normalize_recipients(&input.bcc)?.is_empty();
    if !has_recipient {
        return Err(CommandError::from("missing recipient address"));
    }
    ensure_attachments_within_limit(&input.attachments)?;
    ensure_sender_is_owned(pool.inner(), &account_id, input.from_address.as_deref()).await?;
    apply_selected_signature(pool.inner(), &account_id, &mut input).await?;

    let mut scheduled_for = parse_scheduled_for(input.scheduled_for.as_deref())?;

    // Honor the configurable "Undo Send" delay: when the user hasn't picked an
    // explicit Send-Later time, defer the send by N seconds so they can hit Undo
    // before the network call fires.
    let undo_seconds = read_undo_send_seconds(pool.inner()).await;
    let mut undo_delay_active = false;
    if scheduled_for.is_none() && undo_seconds > 0 {
        scheduled_for = Some(Utc::now() + chrono::Duration::seconds(undo_seconds));
        undo_delay_active = true;
    }

    if input.draft_id.is_none() && scheduled_for.is_some() {
        input.draft_id = Some(format!("draft-{}", uuid::Uuid::new_v4()));
    }
    let action = build_send_action(account_id.clone(), input.clone(), scheduled_for)?;
    let queued_id = action.id.clone();

    satchel_core::drafts::enqueue_send_with_draft(pool.inner(), &state.queue, &action)
        .await
        .map_err(CommandError::from)?;

    if let Some(when) = scheduled_for {
        let event_name = if undo_delay_active {
            "outbox:undo-window"
        } else {
            "outbox:scheduled"
        };
        let _ = tauri::Emitter::emit(
            &app,
            event_name,
            serde_json::json!({
                "queued_id": queued_id,
                "scheduled_for": when.to_rfc3339(),
                "undo_seconds": if undo_delay_active { Some(undo_seconds) } else { None },
            }),
        );
        let status = if undo_delay_active {
            "queued"
        } else {
            "scheduled"
        };
        return Ok(SendMessageResult {
            status: status.into(),
            thread_id: input
                .thread_id
                .clone()
                .unwrap_or_else(|| format!("th-pending-{}", queued_id)),
            message_id: format!("queued-{queued_id}"),
            queued_id: Some(queued_id),
            undo_until: undo_delay_active.then(|| when.to_rfc3339()),
        });
    }

    // Try immediately. Worker will retry if this raises a retryable error.
    match execute_send_action(pool.inner(), &app, &state, &action).await {
        Ok(mut result) => {
            state.queue.mark_completed(pool.inner(), &queued_id).await?;
            state.events.publish(AppEvent::SyncCompleted);
            let _ = tauri::Emitter::emit(
                &app,
                "outbox:sent",
                serde_json::json!({
                    "queued_id": queued_id,
                    "thread_id": result.thread_id,
                    "message_id": result.message_id,
                }),
            );
            result.queued_id = Some(queued_id);
            Ok(result)
        }
        Err(err) => {
            use satchel_core::sync::worker::{settle_action, Outcome};
            let failure = err.clone();
            let outcome = settle_action(pool.inner(), &state.queue, &action, Err(err)).await?;
            let status = match outcome {
                Outcome::DeliveryUnknown { error } => {
                    let _ = tauri::Emitter::emit(
                        &app,
                        "outbox:failed",
                        serde_json::json!({
                            "queued_id": queued_id, "status": "delivery_unknown", "error": error,
                        }),
                    );
                    "delivery_unknown"
                }
                Outcome::Delivered { error } => {
                    let _ = tauri::Emitter::emit(
                        &app,
                        "outbox:failed",
                        serde_json::json!({
                            "queued_id": queued_id, "status": "delivered", "error": error,
                        }),
                    );
                    "delivered"
                }
                Outcome::DeliveryUnrecorded {
                    error,
                    marker_error,
                } => {
                    state
                        .delivery_reconciler
                        .enqueue(queued_id.clone(), error, marker_error);
                    "delivered"
                }
                Outcome::Failed { error } => {
                    let _ = tauri::Emitter::emit(
                        &app,
                        "outbox:failed",
                        serde_json::json!({
                            "queued_id": queued_id, "status": "failed", "error": error,
                        }),
                    );
                    return Err(CommandError::from(failure));
                }
                Outcome::Retry { next_at, error, .. } => {
                    let _ = tauri::Emitter::emit(
                        &app,
                        "outbox:queued",
                        serde_json::json!({
                            "queued_id": queued_id, "next_retry_at": next_at.to_rfc3339(), "error": error,
                        }),
                    );
                    "queued"
                }
                Outcome::Completed => unreachable!("a failed send cannot complete"),
            };
            Ok(SendMessageResult {
                status: status.into(),
                thread_id: input
                    .thread_id
                    .clone()
                    .unwrap_or_else(|| format!("th-pending-{queued_id}")),
                message_id: format!("queued-{queued_id}"),
                queued_id: Some(queued_id),
                undo_until: None,
            })
        }
    }
}

async fn read_undo_send_seconds(pool: &SqlitePool) -> i64 {
    sqlx::query_scalar::<_, String>(
        "SELECT value FROM app_settings WHERE key = 'compose.undo_send_seconds'",
    )
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    .and_then(|raw| raw.parse::<i64>().ok())
    .map(|n| n.clamp(0, 60))
    .unwrap_or(5)
}

/// Validate new submissions only; due outbox actions execute without this check.
fn parse_scheduled_for(raw: Option<&str>) -> Result<Option<DateTime<Utc>>, String> {
    let Some(raw) = raw.map(str::trim).filter(|raw| !raw.is_empty()) else {
        return Ok(None);
    };
    let parsed = DateTime::parse_from_rfc3339(raw)
        .map_err(|_| "backend.errors.scheduledTimeInvalid".to_string())?
        .with_timezone(&Utc);
    if parsed <= Utc::now() {
        return Err("backend.errors.scheduledTimeInvalid".to_string());
    }
    Ok(Some(parsed))
}

/// Persist the locally-shown "sent" record after the network send succeeded.
/// Called by both the sync command path and the background outbox worker.
pub async fn persist_sent_message_record(
    pool: &SqlitePool,
    account_id: &str,
    account_email: &str,
    account_display_name: &str,
    input: &OutgoingMessageInput,
    message_id_header: Option<&str>,
) -> CommandResult<SendMessageResult> {
    let normalized_to = normalize_recipients(&input.to)?;
    let normalized_cc = normalize_recipients(&input.cc)?;
    let normalized_bcc = normalize_recipients(&input.bcc)?;
    let from_address = input
        .from_address
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(account_email);
    let from_name = input
        .from_name
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(account_display_name);

    let persisted = SentMessageRepository::new(pool)
        .persist(SentMessageWriteRequest {
            account_id: account_id.to_string(),
            account_email: account_email.to_string(),
            account_display_name: account_display_name.to_string(),
            thread_id: input.thread_id.clone(),
            subject: input.subject.clone(),
            body: input.body.clone(),
            body_html: input.body_html.clone(),
            from_address: input.from_address.clone(),
            from_name: input.from_name.clone(),
            to: normalized_to.clone(),
            cc: normalized_cc.clone(),
            bcc: normalized_bcc.clone(),
            reply_to: input.reply_to.clone(),
            reply_to_message_id: input.reply_to_message_id.clone(),
            message_id_header: message_id_header.map(str::to_string),
            signature_id: input.signature_id.clone(),
            signature_variant: input.signature_variant.clone(),
            attachments: input
                .attachments
                .iter()
                .map(|attachment| SentAttachmentMetadata {
                    filename: attachment.filename.clone(),
                    mime_type: attachment.mime_type.clone(),
                    size: attachment_decoded_size(&attachment.content_base64),
                    content_id: attachment.content_id.clone(),
                    inline: attachment.inline,
                })
                .collect(),
        })
        .await
        .map_err(|error| {
            // Preserve the typed diagnostic in logs, but this path runs only
            // after provider delivery. The user-facing contract must make that
            // fact clearer than the individual database cause.
            let diagnostic = CommandError::from(error);
            tracing::error!(key = %diagnostic.key, "sent-message finalization failed after delivery");
            CommandError::new("backend.errors.sentMessagePersistenceFailed")
        })?;

    FollowUpRepository::new(pool)
        .create_for_sent_message(
            account_id,
            &persisted.thread_id,
            &persisted.message_id,
            input.follow_up_at.as_deref(),
        )
        .await
        .map_err(|error| {
            tracing::error!(error = ?error, "sent-message follow-up creation failed");
            CommandError::new("backend.errors.sentMessagePostProcessingFailed")
        })?;
    if let Err(error) =
        crate::commands::contacts::touch_contact(pool, from_address, Some(from_name)).await
    {
        tracing::warn!(
            error = ?error,
            operation = "sent_message_sender_contact_touch",
            "contact persistence failed after message delivery"
        );
    }
    for recipient in normalized_to
        .iter()
        .chain(normalized_cc.iter())
        .chain(normalized_bcc.iter())
    {
        if let Err(error) = crate::commands::contacts::touch_contact(pool, recipient, None).await {
            tracing::warn!(
                error = ?error,
                operation = "sent_message_recipient_contact_touch",
                "contact persistence failed after message delivery"
            );
        }
    }

    Ok(SendMessageResult {
        status: "sent".to_string(),
        thread_id: persisted.thread_id,
        message_id: persisted.message_id,
        queued_id: None,
        undo_until: None,
    })
}

async fn apply_selected_signature(
    pool: &SqlitePool,
    account_id: &str,
    input: &mut OutgoingMessageInput,
) -> Result<(), String> {
    let Some(raw_preference) = input.signature_variant.as_deref() else {
        return Ok(());
    };
    let preference = match raw_preference {
        "auto" => SignaturePreference::Auto,
        "long" => SignaturePreference::Long,
        "short" => SignaturePreference::Short,
        "none" => SignaturePreference::None,
        _ => {
            return Err(CommandError::new("backend.errors.signatureVariantInvalid").encoded());
        }
    };
    let account_email = AccountRepository::new(pool)
        .primary_email(account_id)
        .await
        .map_err(|error| CommandError::from(error).encoded())?
        .ok_or_else(|| CommandError::new("backend.errors.accountNotFound").encoded())?;
    let identity_email = input
        .from_address
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(&account_email)
        .to_string();
    let Some(resolved) = resolve_signature_for_compose(
        pool,
        ResolveSignatureInput {
            account_id: account_id.to_string(),
            identity_email,
            thread_id: input.thread_id.clone(),
            signature_id: input.signature_id.clone(),
            preference,
        },
    )
    .await?
    else {
        input.signature_id = None;
        input.signature_variant = Some("none".into());
        return Ok(());
    };

    let mut resolved = resolved;
    if let Some(text) = input.signature_override_text.take() {
        resolved.body_text = text;
    }
    if let Some(html) = input.signature_override_html.take() {
        resolved.body_html = Some(
            tokio::task::spawn_blocking(move || {
                crate::email::sanitizer::sanitize_email_html(&html)
            })
            .await
            .map_err(|error| error.to_string())?,
        );
    }

    for attachment in &resolved.attachments {
        if let Some(cid) = attachment.content_id.as_deref() {
            if input
                .attachments
                .iter()
                .any(|existing| existing.content_id.as_deref() == Some(cid))
            {
                return Err(format!("duplicate inline attachment content id: {cid}"));
            }
        }
    }

    (input.body, input.body_html) = composer_body::with_signature(
        &input.body,
        input.body_html.as_deref(),
        &resolved.signature_id,
        &resolved.body_text,
        resolved.body_html.as_deref(),
    );
    input
        .attachments
        .extend(
            resolved
                .attachments
                .into_iter()
                .map(|attachment| OutgoingAttachmentInput {
                    filename: attachment.filename,
                    mime_type: attachment.mime_type,
                    content_base64: attachment.content_base64,
                    content_id: attachment.content_id,
                    inline: true,
                }),
        );
    input.signature_id = Some(resolved.signature_id);
    input.signature_variant = Some(
        match resolved.variant {
            SignatureVariant::Long => "long",
            SignatureVariant::Short => "short",
        }
        .into(),
    );
    Ok(())
}

pub use satchel_core::drafts::outgoing::recipient_display_names;

pub fn normalize_recipients(values: &[String]) -> CommandResult<Vec<String>> {
    satchel_core::drafts::outgoing::normalize_recipients(values).map_err(CommandError::from)
}

fn attachment_decoded_size(content_base64: &str) -> i64 {
    content_base64
        .len()
        .checked_mul(3)
        .and_then(|length| i64::try_from(length / 4).ok())
        .unwrap_or(0)
}

fn log_perf(operation: &str, started_at: Instant, detail: String) {
    let enabled = std::env::var("SATCHEL_PERF_LOG")
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false);
    if !enabled {
        return;
    }

    tracing::info!(
        target: "perf",
        "{} {}ms {}",
        operation,
        started_at.elapsed().as_millis(),
        detail
    );
}

#[cfg(test)]
mod tests {
    #[tokio::test(flavor = "current_thread")]
    async fn smime_decode_yields_and_maps_failures_at_the_app_boundary() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let ran = AtomicBool::new(false);
        let html = format!(
            "Content-Type: text/html\r\n\r\n{}<script>bad()</script>",
            "<p>secret</p>".repeat(100_000)
        );
        let decode = async {
            let body = super::decode_smime_body(html.into_bytes()).await.unwrap();
            assert!(
                ran.load(Ordering::SeqCst),
                "decoding must yield the async worker"
            );
            assert!(!body.body.html.unwrap().contains("<script"));
        };
        tokio::join!(biased; decode, async {
            ran.store(true, Ordering::SeqCst);
        });
        let malformed = b"Content-Type: multipart/signed; protocol=\"application/pkcs7-signature\"; boundary=x\r\n\r\n--x--\r\n";
        let error = super::decode_smime_body(malformed.to_vec())
            .await
            .err()
            .unwrap();
        assert_eq!(error.key, "backend.errors.smimeMalformedSigned");
        assert!(error.params.is_empty());
    }

    #[test]
    fn newly_submitted_expired_schedule_is_rejected() {
        for raw in [
            "2001-01-01T10:00:00Z".to_string(),
            chrono::Utc::now().to_rfc3339(),
            "invalid".to_string(),
        ] {
            let error = super::parse_scheduled_for(Some(&raw)).unwrap_err();
            assert_eq!(
                super::CommandError::from(error).key,
                "backend.errors.scheduledTimeInvalid"
            );
        }
    }

    #[test]
    fn unscheduled_and_future_submissions_keep_their_meaning() {
        assert_eq!(super::parse_scheduled_for(None).unwrap(), None);
        assert_eq!(super::parse_scheduled_for(Some("  ")).unwrap(), None);
        let future = chrono::Utc::now() + chrono::Duration::hours(1);
        assert_eq!(
            super::parse_scheduled_for(Some(&future.to_rfc3339())).unwrap(),
            Some(future)
        );
    }

    #[tokio::test]
    async fn overdue_queued_schedule_is_still_claimed_for_execution() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        satchel_core::db::init(&pool).await.unwrap();
        let queue = crate::sync::queue::SyncQueue::new();
        let past = chrono::Utc::now() - chrono::Duration::minutes(1);
        let input = super::OutgoingMessageInput {
            to: vec!["recipient@example.com".into()],
            body: "Scheduled message".into(),
            scheduled_for: Some(past.to_rfc3339()),
            ..Default::default()
        };
        let action = super::build_send_action("account".into(), input, Some(past)).unwrap();
        assert_eq!(action.status, "scheduled");
        queue.enqueue(&pool, action).await.unwrap();
        let claimed = queue.claim_next(&pool).await.unwrap().expect("due send");
        let payload: crate::sync::outbox::SendActionPayload =
            serde_json::from_value(claimed.payload).unwrap();
        assert_eq!(payload.scheduled_for, Some(past.to_rfc3339()));
        assert_eq!(payload.input.scheduled_for, Some(past.to_rfc3339()));
    }

    #[test]
    fn opening_a_thread_loads_the_latest_bodies_and_every_draft() {
        let row = |id: &str, draft: i64| super::MessagePreviewRecord {
            id: id.to_string(),
            remote_uidvalidity: None,
            thread_id: Some("t".to_string()),
            subject: None,
            snippet: None,
            from_address: None,
            from_name: None,
            to_addresses: None,
            cc_addresses: None,
            reply_to: None,
            date: None,
            is_read: Some(1),
            is_draft: Some(draft),
        };
        let rows = vec![
            row("m1", 0),
            row("draft-old", 1),
            row("m2", 0),
            row("m3", 0),
        ];
        assert_eq!(super::opening_body_ids(&rows, 2), ["m2", "m3", "draft-old"]);
        assert_eq!(super::opening_body_ids(&rows[..1], 2), ["m1"]);
        assert!(super::opening_body_ids(&[], 2).is_empty());
    }

    #[test]
    fn mime_coverage_is_applied_after_each_verification_including_cache_hits() {
        let mut state = Some("signature_valid".to_string());
        super::apply_signature_scope(&mut state, true);
        assert_eq!(state.as_deref(), Some("signature_valid"));
        super::apply_signature_scope(&mut state, false);
        assert_eq!(state.as_deref(), Some("signature_partial"));
        for verdict in [
            "signature_invalid",
            "signature_identity_mismatch",
            "signature_unknown_key",
        ] {
            let mut state = Some(verdict.to_string());
            super::apply_signature_scope(&mut state, false);
            assert_eq!(state.as_deref(), Some(verdict));
        }
    }

    #[tokio::test]
    async fn cached_content_generation_regressions() {
        use crate::commands::attachments::{prepare_attachment_in_dir, DownloadAttachmentInput};
        use crate::db::remote_content::ContentReference;
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::db::init(&pool).await.unwrap();
        sqlx::raw_sql("INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
            VALUES ('a','A','me@example.test','imap','{}','','','');
            INSERT INTO folders (id, account_id, name, full_path, remote_id, folder_type, uidvalidity, created_at, updated_at)
            VALUES ('f','a','Inbox','INBOX','INBOX','inbox',7,'','');
            INSERT INTO threads (id, account_id, subject, last_message_date, created_at, updated_at) VALUES ('t','a','Subject','','','');
            INSERT INTO messages (id,account_id,folder_id,thread_id,remote_id,remote_uidvalidity,from_address,to_addresses,date,body_text,original_source,created_at,updated_at)
            VALUES ('m','a','f','t','42',7,'sender@example.test','[]','','cached body',CAST('From: sender@example.test' || char(13,10,13,10) || 'cached source' AS BLOB),'','');")
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO messages (id,account_id,folder_id,thread_id,remote_id,remote_uidvalidity,from_address,to_addresses,date,body_text,created_at,updated_at) VALUES ('neighbor','a','f','t','43',8,'sender@example.test','[]','','neighbor bytes','','')").execute(&pool).await.unwrap();
        sqlx::query("UPDATE messages SET snippet = 'cached snippet' WHERE id = 'm'")
            .execute(&pool)
            .await
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let reference = ContentReference::load(&pool, "m").await.unwrap();
        let part = crate::providers::traits::RemoteAttachment {
            remote_part_id: "inner".into(),
            filename: Some("inner.txt".into()),
            content_type: "text/plain".into(),
            size: Some(5),
            content_id: None,
            is_inline: false,
            content_base64: Some("aW5uZXI=".into()),
        };
        super::persist_decrypted_attachments_in_dir(
            dir.path(),
            &pool,
            &reference,
            std::slice::from_ref(&part),
        )
        .await
        .unwrap();
        let prepared = prepare_attachment_in_dir(
            dir.path(),
            &pool,
            DownloadAttachmentInput {
                attachment_id: "att-pgp-m-inner".into(),
            },
        )
        .await
        .unwrap();
        assert_eq!(
            tokio::fs::read(&prepared.cached_path).await.unwrap(),
            b"inner"
        );
        assert!(reference.owns_attachment_cache(&prepared.cached_path, "att-pgp-m-inner"));
        super::persist_decrypted_attachments_in_dir(
            dir.path(),
            &pool,
            &reference,
            std::slice::from_ref(&part),
        )
        .await
        .unwrap();
        let repeated: String =
            sqlx::query_scalar("SELECT cached_path FROM attachments WHERE id = 'att-pgp-m-inner'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(repeated, prepared.cached_path);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);

        // No provider config is usable in this fixture: success proves local cache use.
        for generation in [None, Some(7)] {
            sqlx::query("UPDATE messages SET remote_uidvalidity = ? WHERE id = 'm'")
                .bind(generation)
                .execute(&pool)
                .await
                .unwrap();
            sqlx::query("UPDATE folders SET uidvalidity = 8")
                .execute(&pool)
                .await
                .unwrap();
            let key = if generation.is_none() {
                "imapUidValidityMissing"
            } else {
                "imapUidValidityChanged"
            };
            let error = super::fetch_remote_message_source(&pool, &pool, "m")
                .await
                .unwrap_err();
            assert!(error.contains(key));
            let previews = super::validated_previews(
                &pool,
                satchel_core::db::messages::MessageReadRepository::new(&pool)
                    .list_messages_preview("t")
                    .await
                    .unwrap(),
            )
            .await
            .unwrap();
            let preview = previews.iter().find(|row| row.id == "m").unwrap();
            assert!(preview.content_error.as_deref().unwrap().contains(key));
            assert!(preview.body_text.is_empty());
            assert!(preview.snippet.is_empty());
            assert!(preview.body_html.is_none());
            assert!(preview.source_payload.is_none());
            let current = ContentReference::load(&pool, "m").await.unwrap();
            let repository = satchel_core::db::messages::MessageReadRepository::new(&pool);
            for mut row in repository
                .list_messages("t")
                .await
                .unwrap()
                .into_iter()
                .chain(repository.list_message_bodies(&["m".into()]).await.unwrap())
            {
                if row.id == "neighbor" {
                    let neighbor = ContentReference::load(&pool, "neighbor").await.unwrap();
                    assert!(super::gate_cached_body(&mut row, &neighbor).is_none());
                    assert_eq!(row.body_text.as_deref(), Some("neighbor bytes"));
                    continue;
                }
                assert!(super::gate_cached_body(&mut row, &current)
                    .unwrap()
                    .contains(key));
                assert!(row.body_text.is_none());
                assert!(row.body_html.is_none());
            }
            assert!(prepare_attachment_in_dir(
                dir.path(),
                &pool,
                DownloadAttachmentInput {
                    attachment_id: "att-pgp-m-inner".into()
                }
            )
            .await
            .unwrap_err()
            .contains(key));
        }
        let mut stale_part = part.clone();
        stale_part.content_base64 = Some("c3RhbGU=".into());
        assert!(super::persist_decrypted_attachments_in_dir(
            dir.path(),
            &pool,
            &reference,
            std::slice::from_ref(&stale_part)
        )
        .await
        .unwrap_err()
        .contains("imapUidValidityChanged"));
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            1,
            "rejected publication removes only its staged file"
        );
        // A verified new generation never accepts a prior generation's file.
        // Legacy decrypted rows must not send their inner part id to a provider.
        sqlx::query("UPDATE attachments SET remote_part_id = 'inner'")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("UPDATE messages SET remote_uidvalidity = 8")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            prepare_attachment_in_dir(
                dir.path(),
                &pool,
                DownloadAttachmentInput {
                    attachment_id: "att-pgp-m-inner".into()
                }
            )
            .await
            .unwrap_err(),
            "backend.errors.decryptedAttachmentUnavailable"
        );
        let current = ContentReference::load(&pool, "m").await.unwrap();
        super::persist_decrypted_attachments_in_dir(
            dir.path(),
            &pool,
            &current,
            std::slice::from_ref(&part),
        )
        .await
        .unwrap();
        let fresh = prepare_attachment_in_dir(
            dir.path(),
            &pool,
            DownloadAttachmentInput {
                attachment_id: "att-pgp-m-inner".into(),
            },
        )
        .await
        .unwrap();
        assert_ne!(prepared.cached_path, fresh.cached_path);
        assert_eq!(tokio::fs::read(fresh.cached_path).await.unwrap(), b"inner");
        for (provider, remote_id) in [
            ("imap", Some("import-550e8400-e29b-41d4-a716-446655440000")),
            ("imap", None),
            ("gmail", Some("gmail-id")),
            ("outlook", Some("outlook-id")),
            ("pop3", Some("pop-id")),
        ] {
            sqlx::query("UPDATE accounts SET provider = ?")
                .bind(provider)
                .execute(&pool)
                .await
                .unwrap();
            sqlx::query("UPDATE messages SET remote_id = ?, remote_uidvalidity = NULL")
                .bind(remote_id)
                .execute(&pool)
                .await
                .unwrap();
            let local = ContentReference::load(&pool, "m").await.unwrap();
            super::persist_decrypted_attachments_in_dir(
                dir.path(),
                &pool,
                &local,
                std::slice::from_ref(&part),
            )
            .await
            .unwrap();
            prepare_attachment_in_dir(
                dir.path(),
                &pool,
                DownloadAttachmentInput {
                    attachment_id: "att-pgp-m-inner".into(),
                },
            )
            .await
            .unwrap();
            assert!(super::fetch_remote_message_source(&pool, &pool, "m")
                .await
                .unwrap()
                .unwrap()
                .ends_with(b"cached source"));
        }
    }

    #[test]
    fn draft_attachment_names_are_cleaned_when_stored_and_loaded() {
        let input = super::OutgoingAttachmentInput {
            filename: "Rechnung\u{202e}fdp.html".into(),
            ..Default::default()
        };
        let stored = serde_json::to_value(&input).unwrap();
        assert_eq!(stored["filename"], "Rechnungfdp.html");
        let mut legacy = stored;
        legacy["filename"] = serde_json::json!("Rechnung\u{202e}fdp.html");
        let loaded: super::OutgoingAttachmentInput = serde_json::from_value(legacy).unwrap();
        assert_eq!(loaded.filename, "Rechnungfdp.html");
    }

    use satchel_core::db::messages::{MessageReadError, SentMessageError};

    use super::{is_owned_sender, normalize_recipients, parse_raw_message_body, raw_header_block};

    #[cfg(desktop)]
    #[test]
    fn a_failing_transform_explains_itself_and_the_next_plugin_still_runs() {
        use crate::plugins::runtime::PluginRuntime;
        use satchel_core::db::messages::MessagePreviewRecord;

        let empty = tempfile::tempdir().unwrap();
        let runtime = PluginRuntime::default();
        runtime
            .set_plugin_dirs(vec![empty.path().to_path_buf()])
            .unwrap();
        let mut dto = super::map_message_preview_row(MessagePreviewRecord {
            id: "m".into(),
            remote_uidvalidity: None,
            thread_id: Some("t".into()),
            subject: Some("Encrypted".into()),
            snippet: Some("-----BEGIN PGP MESSAGE-----".into()),
            from_address: Some("a@example.com".into()),
            from_name: None,
            to_addresses: None,
            cc_addresses: None,
            reply_to: None,
            date: None,
            is_read: None,
            is_draft: None,
        });

        let switched_off = super::apply_message_plugins(
            &runtime,
            &["first-plugin".to_string(), "second-plugin".to_string()],
            false,
            &mut dto,
        );

        assert!(switched_off.is_empty(), "not a timeout");
        let detail = dto.pgp_error_detail.expect("error detail on the message");
        assert!(detail.starts_with("first-plugin: "), "{detail}");
        let attempted: Vec<String> = runtime
            .metrics()
            .unwrap()
            .into_iter()
            .map(|metrics| metrics.plugin_id)
            .collect();
        assert_eq!(attempted, ["first-plugin", "second-plugin"]);
    }

    #[test]
    fn encrypted_mail_keeps_a_native_unlock_action_when_the_pgp_plugin_is_denied() {
        let runtime = crate::plugins::runtime::PluginRuntime::default();
        runtime.set_permission_grants(Default::default()).unwrap();
        for (has_key, expected) in [(true, "key_available_locked"), (false, "encrypted_no_key")] {
            let mut dto = super::map_stored_message_row(stored_body_record(
                Some("-----BEGIN PGP MESSAGE-----\nciphertext\n-----END PGP MESSAGE-----".into()),
                None,
            ));
            super::apply_message_plugins(
                &runtime,
                &["satchel-plugin-pgp".into()],
                has_key,
                &mut dto,
            );
            assert_eq!(dto.pgp_state.as_deref(), Some(expected));
            assert!(dto
                .pgp_error_detail
                .as_deref()
                .unwrap()
                .contains("read_threads"));
        }
    }

    #[test]
    fn the_primary_address_and_every_alias_may_appear_in_from() {
        let aliases = vec!["Sales@Firma.de".to_string(), " info@firma.de ".to_string()];

        assert!(is_owned_sender("max@firma.de", "max@firma.de", &aliases));
        assert!(is_owned_sender("sales@firma.de", "max@firma.de", &aliases));
        assert!(is_owned_sender(
            "  MAX@FIRMA.DE  ",
            "max@firma.de",
            &aliases
        ));
        assert!(is_owned_sender("info@firma.de", "max@firma.de", &aliases));
    }

    #[test]
    fn an_address_the_account_does_not_own_is_refused() {
        let aliases = vec!["sales@firma.de".to_string()];

        assert!(!is_owned_sender("chef@fremd.de", "max@firma.de", &aliases));
        assert!(!is_owned_sender("", "max@firma.de", &aliases));
        assert!(!is_owned_sender("   ", "max@firma.de", &aliases));
        // A deleted alias stops being owned the moment it leaves the list.
        assert!(!is_owned_sender("sales@firma.de", "max@firma.de", &[]));
    }

    #[test]
    fn keeps_both_addresses_of_one_recipient_entry() {
        assert_eq!(
            normalize_recipients(&["a@x.de b@y.de".to_string()]).unwrap(),
            vec!["a@x.de".to_string(), "b@y.de".to_string()]
        );
    }

    #[test]
    fn reduces_a_display_name_entry_to_its_address_and_deduplicates() {
        assert_eq!(
            normalize_recipients(&[
                "\"Morgan, Alex\" <a.morgan@example.com>".to_string(),
                "Alex <a.morgan@example.com>".to_string(),
                "  ".to_string(),
            ])
            .unwrap(),
            vec!["a.morgan@example.com".to_string()]
        );
    }

    #[test]
    fn keeps_the_first_display_name_of_a_deduplicated_recipient() {
        let to = vec![
            "Anna Meier <a.meier@kunde.de>".to_string(),
            "A. M. <a.meier@kunde.de>".to_string(),
        ];
        let cc = vec!["plain@kunde.de".to_string()];

        let names = super::recipient_display_names(&[&to, &cc]);

        assert_eq!(normalize_recipients(&to).unwrap().len(), 1);
        assert_eq!(
            names.into_iter().collect::<Vec<_>>(),
            [("a.meier@kunde.de".to_string(), "Anna Meier".to_string())]
        );
    }

    #[test]
    fn rejects_an_entry_without_an_address_instead_of_writing_it_into_the_header() {
        let error = normalize_recipients(&["\"Buck".to_string()]).unwrap_err();

        assert_eq!(error.key, "backend.errors.recipientAddressInvalid");
        assert_eq!(
            error.params.get("value"),
            Some(&serde_json::Value::from("\"Buck"))
        );
    }

    #[test]
    fn extracts_original_headers_without_the_mime_body() {
        let raw = b"From: sender@example.com\r\nAuthentication-Results: mx; dkim=pass\r\n\r\nBody";

        assert_eq!(
            raw_header_block(raw).as_deref(),
            Some("From: sender@example.com\r\nAuthentication-Results: mx; dkim=pass")
        );
    }

    #[test]
    fn maps_message_read_errors_to_stable_ipc_keys() {
        let not_found = super::CommandError::from(MessageReadError::NotFound);
        assert_eq!(not_found.key, "backend.errors.messageNotAvailableLocally");

        let detail = "no such column: auth_spf";
        let persistence = super::CommandError::from(MessageReadError::Persistence(
            sqlx::Error::Protocol(detail.into()),
        ));
        assert_eq!(persistence.key, "backend.errors.messageReadFailed");
        assert!(!persistence.to_string().contains(detail));
    }

    #[test]
    fn maps_sent_message_errors_to_stable_encoded_keys() {
        let detail = "database is locked";
        let persistence = super::CommandError::from(SentMessageError::Persistence(
            sqlx::Error::Protocol(detail.into()),
        ));
        assert_eq!(
            persistence.key,
            "backend.errors.sentMessagePersistenceFailed"
        );
        assert!(!persistence.encoded().contains(detail));

        let reply_context = super::CommandError::from(SentMessageError::ReplyContextLoad(
            sqlx::Error::Protocol(detail.into()),
        ));
        assert_eq!(reply_context.key, "backend.errors.replyContextLoadFailed");
        assert!(!reply_context.encoded().contains(detail));

        let missing = super::CommandError::from(SentMessageError::ReplyThreadNotFound {
            thread_id: "thread-1".into(),
        });
        assert_eq!(missing.key, "backend.errors.replyThreadNotFound");
        assert!(!missing.encoded().contains("thread-1"));
    }

    #[test]
    fn rejects_attachments_over_the_total_budget() {
        use super::{ensure_attachments_within_limit, OutgoingAttachmentInput};

        let big = |mb: usize| OutgoingAttachmentInput {
            filename: "x.bin".into(),
            mime_type: "application/octet-stream".into(),
            // 4 base64 chars per 3 decoded bytes.
            content_base64: "A".repeat(mb * 1024 * 1024 / 3 * 4),
            content_id: None,
            inline: false,
        };

        // Each file is individually acceptable; together they are not.
        assert!(ensure_attachments_within_limit(&[big(8), big(8)]).is_ok());
        let err = ensure_attachments_within_limit(&[big(8), big(8), big(8)])
            .expect_err("three 8 MB files must exceed the 20 MB budget");
        assert!(err.contains("limit is 20 MB"), "unexpected message: {err}");
    }

    #[test]
    fn allows_sending_without_attachments() {
        use super::ensure_attachments_within_limit;
        assert!(ensure_attachments_within_limit(&[]).is_ok());
    }

    #[test]
    fn old_cached_authentication_results_are_untrusted_until_refetched() {
        let mut row = stored_body_record(None, None);
        row.auth_spf = Some("pass".into());
        row.auth_dkim = Some("pass".into());
        row.auth_dmarc = Some("pass".into());
        let dto = super::map_stored_message_row(row.clone());
        assert!(dto.auth_spf.is_none() && dto.auth_dkim.is_none() && dto.auth_dmarc.is_none());
        row.trust_metadata = Some(r#"{"auth_verified":true}"#.into());
        assert_eq!(
            super::map_stored_message_row(row).auth_dmarc.as_deref(),
            Some("pass")
        );
    }

    /// Opening the source of a stored message must not queue for one of the
    /// two writer connections an ingest may be holding.
    #[tokio::test]
    async fn stored_message_source_is_read_without_a_writer_connection() {
        let dir = tempfile::tempdir().unwrap();
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(dir.path().join("source.db"))
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal);
        let write = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .acquire_timeout(std::time::Duration::from_secs(1))
            .connect_with(options.clone())
            .await
            .unwrap();
        crate::db::init(&write).await.unwrap();
        for sql in [
            "INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, created_at, updated_at) VALUES ('a', 'Account', 'me@example.test', 'imap', '{}', '', '', '')",
            "INSERT INTO folders (id, account_id, name, full_path, folder_type, created_at, updated_at) VALUES ('f', 'a', 'Inbox', 'Inbox', 'inbox', '', '')",
            "INSERT INTO messages (id, account_id, folder_id, thread_id, from_address, to_addresses, date, original_source, created_at, updated_at) VALUES ('m', 'a', 'f', 't', 'bob@example.test', '[]', '', CAST('From: bob@example.test' || char(13, 10, 13, 10) || 'Plain' AS BLOB), '', '')",
        ] {
            sqlx::query(sql).execute(&write).await.unwrap();
        }
        let read = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options.read_only(true))
            .await
            .unwrap();
        let _ingest = write.acquire().await.unwrap();
        let raw = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            super::fetch_remote_message_source(&read, &write, "m"),
        )
        .await
        .expect("a stored source needs no writer connection")
        .expect("stored source");
        assert_eq!(
            raw.as_deref(),
            Some(&b"From: bob@example.test\r\n\r\nPlain"[..])
        );
    }

    #[tokio::test]
    async fn unfixable_smime_candidates_are_not_refetched_on_every_open() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("../crates/satchel-core/src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
        // The stored source stands in for the provider: it has no signature
        // the parser can recover, so every attempt ends without a repair.
        for sql in [
            "INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, created_at, updated_at) VALUES ('a', 'Account', 'me@example.test', 'imap', '{}', '', '', '')",
            "INSERT INTO folders (id, account_id, name, full_path, folder_type, created_at, updated_at) VALUES ('f', 'a', 'Inbox', 'Inbox', 'inbox', '', '')",
            "INSERT INTO messages (id, account_id, folder_id, thread_id, from_address, to_addresses, date, original_source, created_at, updated_at) VALUES ('m', 'a', 'f', 't', 'bob@example.test', '[]', '', CAST('From: bob@example.test' || char(13, 10, 13, 10) || 'Plain' AS BLOB), '', '')",
            "INSERT INTO attachments (id, message_id, filename, content_type, size, created_at) VALUES ('att', 'm', 'smime.p7s', 'application/pkcs7-signature', 1, '')",
        ] {
            sqlx::query(sql).execute(&pool).await.unwrap();
        }
        let ids = vec!["m".to_string()];
        let raw_size = || async {
            sqlx::query_scalar::<_, Option<i64>>("SELECT raw_size FROM messages WHERE id = 'm'")
                .fetch_one(&pool)
                .await
                .unwrap()
        };

        assert!(super::repair_unparsed_smime_messages(&pool, &pool, &ids)
            .await
            .is_empty());
        assert!(raw_size().await.is_some(), "the first open refetches");
        sqlx::query("UPDATE messages SET raw_size = NULL WHERE id = 'm'")
            .execute(&pool)
            .await
            .unwrap();
        super::repair_unparsed_smime_messages(&pool, &pool, &ids).await;
        assert!(raw_size().await.is_none(), "the next open must not");

        // After the retry window the message gets another attempt.
        sqlx::query("UPDATE messages SET smime_repair_attempted_at = '2000-01-01T00:00:00+00:00'")
            .execute(&pool)
            .await
            .unwrap();
        super::repair_unparsed_smime_messages(&pool, &pool, &ids).await;
        assert!(raw_size().await.is_some());
    }

    #[test]
    fn a_message_in_smime_repair_is_not_claimed_twice() {
        let candidate = || vec![("claimed-twice".to_string(), "t".to_string())];
        let first = super::SmimeRepairClaim::new(candidate());
        assert_eq!(first.candidates.len(), 1);
        assert!(super::SmimeRepairClaim::new(candidate())
            .candidates
            .is_empty());
        drop(first);
        assert_eq!(
            super::SmimeRepairClaim::new(candidate()).candidates.len(),
            1
        );
    }

    #[tokio::test]
    async fn stale_source_fetch_cannot_write_into_a_reingested_generation() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("../crates/satchel-core/src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
        sqlx::raw_sql("INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
                VALUES ('a', 'Account', 'me@example.test', 'imap', '{}', '', '', '');
            INSERT INTO folders (id, account_id, name, full_path, remote_id, folder_type, uidvalidity, created_at, updated_at)
                VALUES ('f', 'a', 'Inbox', 'INBOX', 'INBOX', 'inbox', 7, '', '');
            INSERT INTO messages (id, account_id, folder_id, remote_id, remote_uidvalidity, from_address, to_addresses, date, created_at, updated_at)
                VALUES ('m', 'a', 'f', '42', 7, 'bob@example.test', '[]', '', '', '');")
            .execute(&pool).await.unwrap();
        let reference = crate::db::remote_content::ContentReference::load(&pool, "m")
            .await
            .unwrap();
        sqlx::raw_sql(
            "UPDATE folders SET uidvalidity = 8;
            UPDATE messages SET remote_uidvalidity = 8, body_text = 'replacement';",
        )
        .execute(&pool)
        .await
        .unwrap();
        let error = super::persist_raw_message_metadata(
            &pool,
            &reference,
            b"Content-Type: text/plain\r\n\r\nOld payload",
        )
        .await
        .unwrap_err();
        assert!(error.contains("imapUidValidityChanged"));
        let (source, body): (Option<Vec<u8>>, String) =
            sqlx::query_as("SELECT original_source, body_text FROM messages WHERE id = 'm'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(source, None);
        assert_eq!(body, "replacement");
    }

    /// A source read again can turn a row into root-signed mail. The rows an
    /// older parse listed beside it give way to the signed entity's parts.
    #[tokio::test]
    async fn a_source_reparse_rebuilds_older_attachment_rows_from_the_bundle() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::db::init(&pool).await.unwrap();
        sqlx::raw_sql("INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
                VALUES ('a', 'Account', 'me@example.test', 'gmail', '{}', '', '', '');
            INSERT INTO folders (id, account_id, name, full_path, folder_type, created_at, updated_at)
                VALUES ('f', 'a', 'Inbox', 'Inbox', 'inbox', '', '');
            INSERT INTO messages (id, account_id, folder_id, from_address, to_addresses, date, created_at, updated_at)
                VALUES ('m', 'a', 'f', 'sender@example.test', '[]', '', '', '');
            INSERT INTO attachments (id, message_id, filename, content_type, listing_rules, created_at)
                VALUES ('old-signature', 'm', 'Invoice.pdf', 'application/pgp-signature', 0, '');")
            .execute(&pool)
            .await
            .unwrap();
        let raw = include_bytes!(
            "../../../crates/satchel-core/tests/corpus/pgp-mime/thunderbird-mixed-attachment.eml"
        );
        let reference = crate::db::remote_content::ContentReference::load(&pool, "m")
            .await
            .unwrap();
        super::persist_raw_message_metadata(&pool, &reference, raw)
            .await
            .unwrap();
        let rows: Vec<(String, i64)> = sqlx::query_as(
            "SELECT filename, listing_rules FROM attachments WHERE message_id = 'm'",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(rows, [("figures.csv".to_string(), 1)]);
    }

    #[tokio::test]
    async fn source_reparse_clears_stale_html_and_forged_authentication() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("../crates/satchel-core/src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
        for sql in [
            "INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, created_at, updated_at) VALUES ('a', 'Account', 'me@example.test', 'gmail', '{}', '', '', '')",
            "INSERT INTO folders (id, account_id, name, full_path, folder_type, created_at, updated_at) VALUES ('f', 'a', 'Inbox', 'Inbox', 'inbox', '', '')",
            "INSERT INTO messages (id, account_id, folder_id, from_address, to_addresses, date, body_html, auth_dmarc, created_at, updated_at) VALUES ('m', 'a', 'f', 'bob@bank.example', '[]', '', '<p>Stale unsigned HTML</p>', 'pass', '', '')",
        ] {
            sqlx::query(sql).execute(&pool).await.unwrap();
        }
        let raw = b"From: bob@bank.example\r\nAuthentication-Results: attacker.example; dmarc=pass\r\nContent-Type: multipart/signed; protocol=\"application/pgp-signature\"; boundary=s\r\n\r\n--s\r\nContent-Type: text/plain\r\n\r\nSigned plain text\r\n--s\r\nContent-Type: application/pgp-signature\r\n\r\nSignature\r\n--s--\r\n";
        super::persist_raw_message_metadata(
            &pool,
            &crate::db::remote_content::ContentReference::load(&pool, "m")
                .await
                .unwrap(),
            raw,
        )
        .await
        .unwrap();
        let original_source: Vec<u8> =
            sqlx::query_scalar("SELECT original_source FROM messages WHERE id = 'm'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(original_source, raw);
        let (text, html, auth, trust): (String, Option<String>, Option<String>, String) =
            sqlx::query_as("SELECT body_text, body_html, auth_dmarc, trust_metadata FROM messages WHERE id = 'm'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(text.trim(), "Signed plain text");
        assert!(html.is_none());
        assert!(auth.is_none());
        let trust: crate::providers::traits::MessageTrust = serde_json::from_str(&trust).unwrap();
        assert!(trust.root_crypto);
        assert!(!trust.auth_verified);

        let provider_raw =
            String::from_utf8_lossy(raw).replace("attacker.example", "mx.google.com");
        super::persist_raw_message_metadata(
            &pool,
            &crate::db::remote_content::ContentReference::load(&pool, "m")
                .await
                .unwrap(),
            provider_raw.as_bytes(),
        )
        .await
        .unwrap();
        let provider_verdict: Option<String> =
            sqlx::query_scalar("SELECT auth_dmarc FROM messages WHERE id = 'm'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(provider_verdict.as_deref(), Some("pass"));

        sqlx::query("INSERT INTO messages (id, account_id, folder_id, from_address, to_addresses, date, created_at, updated_at) VALUES ('m-import-fixture', 'a', 'f', 'bob@bank.example', '[]', '', '', '')")
            .execute(&pool)
            .await
            .unwrap();
        super::persist_raw_message_metadata(
            &pool,
            &crate::db::remote_content::ContentReference::load(&pool, "m-import-fixture")
                .await
                .unwrap(),
            provider_raw.as_bytes(),
        )
        .await
        .unwrap();
        let imported_verdict: Option<String> =
            sqlx::query_scalar("SELECT auth_dmarc FROM messages WHERE id = 'm-import-fixture'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(imported_verdict.is_none());
    }

    #[test]
    fn covered_inline_pgp_reaches_decryption() {
        let parsed = crate::providers::imap::parse_raw_message_body(include_bytes!(
            "../../../crates/satchel-core/tests/corpus/pgp/inline-covered.eml"
        ));
        assert!(!parsed.trust.root_encryption);
        assert!(super::decrypts_inline_pgp(
            &parsed.trust,
            parsed.text.as_deref().unwrap()
        ));
    }

    #[test]
    fn only_a_whole_root_block_brings_its_own_html() {
        use crate::providers::traits::MessageTrust;
        let decrypted = || crate::commands::pgp::DecryptedIncomingPgp {
            prefix: String::new(),
            suffix: "\n".into(),
            plaintext: "Content-Type: text/html\r\n\r\n<p>Decrypted <b>HTML</b></p>".into(),
        };
        let root = MessageTrust {
            root_encryption: true,
            ..Default::default()
        };
        let body = super::inline_pgp_body(&root, decrypted()).unwrap();
        assert!(body.whole_body);
        assert!(body.html.unwrap().contains("<b>HTML</b>"));
        // The same block as the only text part of a multipart message: other
        // parts sit beside it, so it does not stand for the message.
        let body = super::inline_pgp_body(&MessageTrust::default(), decrypted()).unwrap();
        assert!(!body.whole_body);
        assert!(body.html.is_none());
    }

    /// A decrypted payload from `tests/corpus/decrypted`, as OpenPGP plaintext.
    macro_rules! decrypted_corpus {
        ($name:literal) => {
            include_str!(concat!(
                "../../../crates/satchel-core/tests/corpus/decrypted/",
                $name
            ))
        };
    }

    fn pgp_mime_plaintext(plaintext: &str) -> crate::commands::pgp::DecryptedIncomingPgp {
        crate::commands::pgp::DecryptedIncomingPgp {
            prefix: String::new(),
            suffix: String::new(),
            plaintext: plaintext.into(),
        }
    }

    /// PGP/MIME plaintext gets the completeness check S/MIME has: incomplete
    /// MIME is an OpenPGP error, never raw MIME shown as the decrypted text.
    #[test]
    fn pgp_mime_plaintexts_are_decoded_completely_or_refused() {
        use crate::providers::traits::MessageTrust;
        use satchel_core::email::decrypted::DecodeError;
        let root = MessageTrust {
            root_encryption: true,
            root_crypto: true,
            ..Default::default()
        };
        let mut failures = Vec::new();
        for (name, plaintext, expected) in [
            (
                "missing-multipart-boundary",
                decrypted_corpus!("missing-multipart-boundary.eml"),
                DecodeError::MalformedMime,
            ),
            (
                "nested-inline-envelope",
                decrypted_corpus!("nested-inline-envelope.eml"),
                DecodeError::RemainingEncryption,
            ),
            (
                "nested-malformed-signed",
                decrypted_corpus!("nested-malformed-signed.eml"),
                DecodeError::MalformedSigned,
            ),
            (
                "malformed-signed-three-parts",
                decrypted_corpus!("malformed-signed-three-parts.eml"),
                DecodeError::MalformedSigned,
            ),
            (
                "signed-part-labelled-by-description",
                decrypted_corpus!("signed-part-labelled-by-description.eml"),
                DecodeError::MalformedSigned,
            ),
        ] {
            match super::inline_pgp_body(&root, pgp_mime_plaintext(plaintext)) {
                Err(error) if error == expected => {}
                Err(error) => {
                    failures.push(format!("{name}: expected {expected:?}, got {error:?}"))
                }
                Ok(body) => failures.push(format!(
                    "{name}: expected {expected:?}, shown as {:?}",
                    body.decrypted_text
                )),
            }
        }
        // A file as the whole plaintext is listed, not shown as base64.
        match super::inline_pgp_body(
            &root,
            pgp_mime_plaintext(decrypted_corpus!("root-attachment.eml")),
        ) {
            Ok(body) => {
                if body.decrypted_text.contains("Content-")
                    || body.decrypted_text.contains("AP+AQQ")
                {
                    failures.push(format!(
                        "root-attachment: shown as {:?}",
                        body.decrypted_text
                    ));
                }
                if body.attachments.len() != 1 {
                    failures.push(format!(
                        "root-attachment: {} attachments",
                        body.attachments.len()
                    ));
                }
            }
            Err(error) => failures.push(format!("root-attachment: {error:?}")),
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
        for error in [
            DecodeError::NestingLimit,
            DecodeError::RemainingEncryption,
            DecodeError::MalformedSigned,
            DecodeError::MalformedMime,
            DecodeError::UndecodedPart,
            DecodeError::InvalidSignedContent("detail".into()),
        ] {
            assert!(super::pgp_decode_error_key(&error).starts_with("backend.errors.pgp"));
        }
    }

    /// The renderer only hydrates image bytes for inline images and for parts
    /// an image reference names. Bytes for any other part are a second copy
    /// of nothing it checks.
    #[test]
    fn provenance_carries_image_bytes_only_where_the_renderer_resolves_them() {
        let bytes = "data:x;base64,AAAA".to_string();
        let row = |content_type: &str, inline: bool, content_id: Option<&str>, filename: &str| {
            super::super::attachments::AttachmentDto {
                id: "att".into(),
                message_id: "msg".into(),
                filename: Some(filename.into()),
                content_type: content_type.into(),
                size: Some(3),
                content_id: content_id.map(str::to_string),
                is_inline: i64::from(inline),
                cached_path: None,
            }
        };
        let url = |row: &super::super::attachments::AttachmentDto, html: Option<&str>| {
            let referenced = html.map(super::referenced_cids).unwrap_or_default();
            super::provenance_data_url(row, Some(&bytes), &referenced)
        };
        let logo = row("image/png", false, Some("<logo@example>"), "logo.png");
        assert_eq!(
            url(
                &row("application/pdf", false, Some("a"), "a.pdf"),
                Some("<img src=\"cid:a\">")
            ),
            None
        );
        assert_eq!(
            url(&row("application/pdf", true, None, "a.pdf"), None),
            None
        );
        assert_eq!(url(&logo, None), None);
        assert_eq!(url(&logo, Some("<p>no reference</p>")), None);
        // Another image's reference does not make this one needed.
        assert_eq!(url(&logo, Some("<img src=\"cid:chart@example\">")), None);
        assert_eq!(
            url(&logo, Some("<a href=\"cid:logo@example\">logo</a>")),
            None
        );
        assert_eq!(
            url(&row("image/png", true, None, "inline.png"), None),
            Some(bytes.clone())
        );
        for reference in ["CID:Logo@Example", "cid:logo%40example", "cid:logo.png"] {
            assert_eq!(
                url(&logo, Some(&format!("<p><img src=\"{reference}\"></p>"))),
                Some(bytes.clone()),
                "{reference}"
            );
        }
        let referenced = super::referenced_cids("<img src=\"cid:logo@example\">");
        assert_eq!(super::provenance_data_url(&logo, None, &referenced), None);
    }

    /// Decrypted content that cannot be shown says so. A valid outer
    /// signature over the ciphertext does not turn that into "verified".
    #[test]
    fn a_decode_error_outranks_any_outer_signature() {
        let failed = super::PgpVerdict::failed("backend.errors.pgpMalformedSigned");
        assert_eq!(failed.state, "error");
        assert!(failed
            .error_detail
            .as_deref()
            .is_some_and(|detail| detail.contains("backend.errors.pgpMalformedSigned")));
        for outer in [
            "signature_valid",
            "signature_partial",
            "signature_unknown_key",
            "signature_invalid",
            "error",
        ] {
            let outer = super::PgpVerdict {
                state: outer.into(),
                error_detail: None,
                signer_name: Some("outer".into()),
                signer_fingerprint: Some("outer".into()),
            };
            assert_eq!(
                super::combine_pgp_verdicts(Some(failed.clone()), outer),
                failed
            );
        }
    }

    /// An outer signature does not overwrite what the decrypted content says
    /// about itself, unless the outer claim is the worse one.
    #[test]
    fn a_root_signature_never_hides_a_worse_inner_state() {
        let verdict = |state: &str, signer: &str| super::PgpVerdict {
            state: state.into(),
            error_detail: None,
            signer_name: Some(signer.into()),
            signer_fingerprint: Some(signer.into()),
        };
        let partial = verdict("signature_partial", "outer");
        for (inner, state, signer) in [
            ("signature_invalid", "signature_invalid", "inner"),
            ("signature_unknown_key", "signature_unknown_key", "inner"),
            // A tie keeps the decrypted content's own signature.
            ("signature_partial", "signature_partial", "inner"),
            ("decrypted", "signature_partial", "outer"),
        ] {
            let shown = super::combine_pgp_verdicts(Some(verdict(inner, "inner")), partial.clone());
            assert_eq!(shown.state, state, "inner {inner}");
            assert_eq!(shown.signer_name.as_deref(), Some(signer), "inner {inner}");
        }
        let invalid = verdict("signature_invalid", "outer");
        assert_eq!(
            super::combine_pgp_verdicts(Some(super::PgpVerdict::decrypted()), invalid.clone()),
            invalid
        );
    }

    #[test]
    fn a_valid_part_signature_does_not_stop_checking_the_next_one() {
        assert!(super::keeps_verifying_smime(Some("signature_valid")));
        assert!(super::keeps_verifying_smime(Some("signature_partial")));
        for state in ["signature_invalid", "signature_unchained", "error"] {
            assert!(!super::keeps_verifying_smime(Some(state)), "{state}");
        }
    }

    #[test]
    fn stored_marker_pairs_never_acquire_decryption_provenance() {
        for protocol in ["PGP", "SMIME"] {
            let text = format!("[[SATCHEL_{protocol}_DECRYPTED_BLOCK_START]]fake[[SATCHEL_{protocol}_DECRYPTED_BLOCK_END]]");
            let dto = super::map_stored_message_row(stored_body_record(Some(text.clone()), None));
            assert_eq!(dto.body_text, text);
            assert!(dto.body_segments.is_none());
            assert!(dto.pgp_state.is_none() && dto.smime_state.is_none());
        }
    }

    fn stored_body_record(
        body_text: Option<String>,
        body_html: Option<String>,
    ) -> super::MessageReadRecord {
        super::MessageReadRecord {
            id: "message".into(),
            remote_uidvalidity: None,
            thread_id: Some("thread".into()),
            subject: Some("Review".into()),
            snippet: Some("Preview".into()),
            from_address: Some("reviewer@example.test".into()),
            from_name: None,
            to_addresses: None,
            cc_addresses: None,
            reply_to: None,
            date: Some("2026-09-12T10:00:00Z".into()),
            body_text,
            body_html,
            is_read: Some(0),
            is_draft: Some(0),
            list_unsubscribe: None,
            list_unsubscribe_post: None,
            auth_spf: None,
            auth_dkim: None,
            auth_dmarc: None,
            smime_encrypted: None,
            smime_signed: None,
            pgp_signed: None,
            trust_metadata: None,
            source_payload: None,
        }
    }

    const FORGED_PGP_ROOT: &[u8] = include_bytes!(
        "../../../crates/satchel-core/tests/corpus/pgp-mime/forged-missing-protocol.eml"
    );
    const FORGED_SMIME_HTML_ROOT: &[u8] = include_bytes!(
        "../../../crates/satchel-core/tests/corpus/smime-signed/forged-html-link-swapped.eml"
    );
    const MUTT_SIGNED: &[u8] =
        include_bytes!("../../../crates/satchel-core/tests/corpus/pgp-mime/mutt-crlf.eml");

    /// The bundle an older parser kept for a forged root: the nested,
    /// genuinely signed part.
    fn legacy_bundle(forged: &[u8]) -> Vec<u8> {
        let text = std::str::from_utf8(forged).unwrap();
        let start = text
            .find("--OUT\r\nContent-Type: multipart/signed")
            .unwrap()
            + 7;
        text.as_bytes()[start..text.rfind("\r\n--OUT--").unwrap()].to_vec()
    }

    fn trust(json: &str) -> crate::providers::traits::MessageTrust {
        serde_json::from_str(json).unwrap()
    }

    /// Rows synced before the strict splitter keep the body an older parser
    /// chose. Whatever it is, the signature is shown over its own entity.
    #[tokio::test]
    async fn a_legacy_signed_row_shows_its_signed_entity_not_its_stored_body() {
        let legacy = trust(r#"{"root_crypto":true}"#);
        let padded = format!("Forged text {}", "x".repeat(65534 - 12));
        assert!(super::is_legacy_truncated_body(Some(&padded)));
        for stored in [
            "Forged text: wire the money to the new account.".to_string(),
            padded,
        ] {
            let bundle = legacy_bundle(FORGED_PGP_ROOT);
            let mut dto = super::map_stored_message_row(stored_body_record(Some(stored), None));
            dto.body_loaded = false;
            let shown = super::show_signed_body(&mut dto, &bundle, &legacy, true).await;
            assert_eq!(shown, super::SignedDisplay::Covered);
            assert!(
                dto.body_text.starts_with("The fix is on the branch."),
                "{}",
                dto.body_text
            );
            assert!(!dto.body_text.contains("Forged"));
            assert!(!dto.snippet.contains("Forged"));
            assert!(dto.body_loaded);
        }

        // A swapped link in stored HTML does not survive either.
        let bundle = legacy_bundle(FORGED_SMIME_HTML_ROOT);
        let mut dto = super::map_stored_message_row(stored_body_record(
            None,
            Some(r#"<p>Please review the invoice at <a href="https://evil.example/login">the bank portal</a> before Friday.</p>"#.into()),
        ));
        assert_eq!(
            super::show_signed_body(&mut dto, &bundle, &legacy, true).await,
            super::SignedDisplay::Covered
        );
        let html = dto.body_html.unwrap_or_default();
        assert!(html.contains("https://bank.example/invoice/42"), "{html}");
        assert!(!html.contains("evil.example"));
    }

    #[tokio::test]
    async fn a_signed_row_written_with_its_bundle_keeps_its_body_until_evicted() {
        let parsed = parse_raw_message_body(MUTT_SIGNED);
        assert!(parsed.trust.signed_binding);
        let bundle = parsed.pgp_signed.clone().unwrap();
        let stored = parsed.text.clone();

        let mut dto = super::map_stored_message_row(stored_body_record(stored.clone(), None));
        let shown = super::show_signed_body(&mut dto, &bundle, &parsed.trust, true).await;
        assert_eq!(shown, super::SignedDisplay::Covered);
        assert_eq!(Some(dto.body_text), stored);

        // Evicted: the snippet must not stand in for the body.
        let mut dto = super::map_stored_message_row(stored_body_record(None, None));
        assert_eq!(dto.body_text, "Preview");
        dto.body_loaded = false;
        assert_eq!(
            super::show_signed_body(&mut dto, &bundle, &parsed.trust, false).await,
            super::SignedDisplay::Covered
        );
        assert_eq!(Some(dto.body_text), stored);
        assert!(dto.body_loaded);
    }

    #[tokio::test]
    async fn opaque_smime_shows_the_verified_content_or_no_body_and_no_verdict() {
        let content = b"Content-Type: text/plain\r\n\r\nThe opaque contract text.\r\n".to_vec();
        let verified = |content: Option<Vec<u8>>| super::SmimeVerifyMessageResult {
            state: "signature_valid".into(),
            signer_email: Some("signer@example.test".into()),
            signer_cn: None,
            error_detail: None,
            content,
        };

        let mut dto = super::map_stored_message_row(stored_body_record(None, None));
        let result = super::show_encapsulated_body(&mut dto, verified(Some(content))).await;
        assert_eq!(result.state, "signature_valid");
        assert_eq!(dto.body_text, "The opaque contract text.");

        // The subject or snippet never stands in for content that is missing.
        let mut dto = super::map_stored_message_row(stored_body_record(None, None));
        let result = super::show_encapsulated_body(&mut dto, verified(None)).await;
        assert_eq!(result.state, "error");
        assert!(result
            .error_detail
            .as_deref()
            .is_some_and(|detail| detail.contains("backend.errors.signedContentUnavailable")));
        assert!(dto.body_text.is_empty());
        assert!(dto.body_html.is_none());
    }

    /// A signed entity that is itself encrypted displays nothing here.
    const SIGNED_ENCRYPTED_ENTITY: &[u8] = b"Content-Type: multipart/signed; micalg=pgp-sha256; protocol=\"application/pgp-signature\"; boundary=\"S\"\r\n\r\n\
        --S\r\nContent-Type: multipart/encrypted; protocol=\"application/pgp-encrypted\"; boundary=\"E\"\r\n\r\n\
        --E\r\nContent-Type: application/pgp-encrypted\r\n\r\nVersion: 1\r\n\
        --E\r\nContent-Type: application/octet-stream\r\n\r\n-----BEGIN PGP MESSAGE-----\r\nhQEMA\r\n-----END PGP MESSAGE-----\r\n--E--\r\n\
        --S\r\nContent-Type: application/pgp-signature\r\n\r\n-----BEGIN PGP SIGNATURE-----\r\nAAAA\r\n-----END PGP SIGNATURE-----\r\n--S--\r\n";

    /// A verdict needs a body it covers. A signed entity that displays nothing
    /// leaves the signature without one, whatever the row stored.
    #[tokio::test]
    async fn a_signed_entity_that_shows_nothing_carries_no_verdict() {
        let parsed = parse_raw_message_body(SIGNED_ENCRYPTED_ENTITY);
        assert!(parsed.trust.root_crypto && parsed.trust.signed_binding);
        let bundle = parsed.pgp_signed.clone().unwrap();
        let legacy = trust(r#"{"root_crypto":true}"#);
        for (stored, trust) in [
            (None, &parsed.trust),
            (Some("Forged text".to_string()), &legacy),
        ] {
            let has_stored_body = stored.is_some();
            let mut dto = super::map_stored_message_row(stored_body_record(stored, None));
            let shown = super::show_signed_body(&mut dto, &bundle, trust, has_stored_body).await;
            assert_eq!(shown, super::SignedDisplay::Unavailable);
            assert!(dto.body_text.is_empty(), "{}", dto.body_text);
            assert!(dto.body_html.is_none());
            assert!(dto.snippet.is_empty(), "{}", dto.snippet);
        }

        let mut dto = super::map_stored_message_row(stored_body_record(None, None));
        dto.pgp_state = Some("signature_valid".into());
        dto.pgp_signer_name = Some("Signer".into());
        super::PgpVerdict::failed("backend.errors.signedContentUnavailable").apply(&mut dto);
        assert_eq!(dto.pgp_state.as_deref(), Some("error"));
        assert!(dto
            .pgp_error_detail
            .as_deref()
            .is_some_and(|detail| detail.contains("backend.errors.signedContentUnavailable")));
        assert!(dto.pgp_signer_name.is_none());

        // Decrypted, the same structure is refused: its encrypted layer would
        // have to be shown in place, and nothing else stands in for it.
        let refused = super::inline_pgp_body(&decrypted_root(), decrypted(SIGNED_ENCRYPTED_ENTITY));
        assert_eq!(
            refused.err(),
            Some(satchel_core::email::decrypted::DecodeError::RemainingEncryption)
        );
        assert!(super::shows_nothing("", None));
        assert!(!super::shows_nothing("Signed text", None));
        assert!(!super::shows_nothing("", Some("<p>Signed</p>")));
    }

    /// The trust of a message whose root is one encrypted block.
    fn decrypted_root() -> crate::providers::traits::MessageTrust {
        trust(r#"{"root_crypto":true,"root_encryption":true}"#)
    }

    /// `plaintext` as the whole of an inline OpenPGP block's plaintext.
    fn decrypted(plaintext: &[u8]) -> crate::commands::pgp::DecryptedIncomingPgp {
        crate::commands::pgp::DecryptedIncomingPgp {
            prefix: String::new(),
            suffix: String::new(),
            plaintext: String::from_utf8(plaintext.to_vec()).unwrap(),
        }
    }

    /// The decrypted plaintext as a whole is not what a signature inside it
    /// covers, so it never stands in for the signed part. What is shown is
    /// the entity the bundle handed to the verifier yields.
    #[test]
    fn a_decrypted_signed_message_shows_only_its_signed_display() {
        let body = super::inline_pgp_body(&decrypted_root(), decrypted(MUTT_SIGNED)).unwrap();
        assert!(body.decrypted_text.starts_with("The fix is on the branch."));
        assert!(!body.decrypted_text.contains("BEGIN PGP SIGNATURE"));
        let [(bundle, true)] = &body.signatures[..] else {
            panic!("signatures {:?}", body.signatures.len());
        };
        let verified = crate::email::pgp_mime::parse_signed_message(bundle).unwrap();
        let stored = parse_raw_message_body(MUTT_SIGNED).pgp_signed.unwrap();
        assert_eq!(
            verified.signed_entity,
            crate::email::signed_mime::split_bundle(&stored)
                .unwrap()
                .entity
        );
        assert_eq!(
            crate::email::signed_mime::display_entity(&verified.signed_entity).text,
            parse_raw_message_body(MUTT_SIGNED).text
        );

        // A signed part inside the decrypted message signs only that part,
        // and a root the strict splitter does not take is refused.
        let forged = parse_raw_message_body(FORGED_PGP_ROOT);
        assert!(forged.pgp_signed.is_none());
        assert!(!forged.trust.root_crypto);
        assert_eq!(
            super::inline_pgp_body(&decrypted_root(), decrypted(FORGED_PGP_ROOT)).err(),
            Some(satchel_core::email::decrypted::DecodeError::MalformedSigned)
        );
    }

    /// A row synced before the binding mark, whose signed entity holds an
    /// armored block. The body the decryption reads is the entity's, put on
    /// the DTO first, never the stored one; segments from any earlier decode
    /// do not survive the entity replacing the body.
    #[tokio::test]
    async fn a_legacy_signed_row_decrypts_only_the_block_its_signature_covers() {
        let raw = include_bytes!(
            "../../../crates/satchel-core/tests/corpus/pgp/signed-covered-block.eml"
        );
        let parsed = parse_raw_message_body(raw);
        assert!(parsed.trust.signed_binding);
        let bundle = parsed.pgp_signed.clone().unwrap();
        let entity = crate::email::signed_mime::split_bundle(&bundle)
            .unwrap()
            .entity
            .to_vec();
        let legacy = trust(r#"{"root_crypto":true}"#);
        let forged =
            "Cover.\n\n-----BEGIN PGP MESSAGE-----\n\nhQEMAforged\n-----END PGP MESSAGE-----\n";

        let mut dto = super::map_stored_message_row(stored_body_record(Some(forged.into()), None));
        dto.body_segments = Some(vec![super::BodySegment::PgpDecrypted("Forged".into())]);
        assert_eq!(
            super::show_signed_body(&mut dto, &bundle, &legacy, true).await,
            super::SignedDisplay::Covered
        );
        assert!(dto.body_segments.is_none());
        assert_eq!(
            Some(dto.body_text.clone()),
            crate::email::signed_mime::display_entity(&entity).text
        );
        assert!(!dto.body_text.contains("hQEMAforged"));
        assert!(super::decrypts_inline_pgp(&legacy, &dto.body_text));
        // A decrypted block inside the shown entity only reaches partial scope.
        let segments = [
            super::BodySegment::Text("Cover".into()),
            super::BodySegment::PgpDecrypted("Plaintext".into()),
        ];
        assert!(!super::root_signature_covers_body(&legacy, Some(&segments)));

        // Written with its bundle, the stored body is the entity's and stays.
        let mut bound =
            super::map_stored_message_row(stored_body_record(parsed.text.clone(), None));
        assert_eq!(
            super::show_signed_body(&mut bound, &bundle, &parsed.trust, true).await,
            super::SignedDisplay::Covered
        );
        assert_eq!(Some(bound.body_text), parsed.text);
    }

    /// Decrypted content that is itself signed, with a file inside the signed
    /// entity, on a message beside a root signature whose older rows wait for
    /// their rebuild. The decrypted file is written under the current listing
    /// rules and the saved generation: it is listed, survives the repair and
    /// is served from the cache, with the signed entity's bytes.
    #[tokio::test]
    async fn attachments_under_a_signed_decrypted_entity_are_listed_and_downloadable() {
        use crate::commands::attachments::{
            attachments_for_messages, prepare_attachment_in_dir, DownloadAttachmentInput,
        };
        use base64::Engine as _;
        let raw = include_bytes!(
            "../../../crates/satchel-core/tests/corpus/pgp-mime/thunderbird-mixed-attachment.eml"
        );
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::db::init(&pool).await.unwrap();
        sqlx::raw_sql("INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
            VALUES ('a','A','me@example.test','imap','{}','','','');
            INSERT INTO folders (id, account_id, name, full_path, remote_id, folder_type, uidvalidity, created_at, updated_at)
            VALUES ('f','a','Inbox','INBOX','INBOX','inbox',7,'','');
            INSERT INTO threads (id, account_id, subject, last_message_date, created_at, updated_at) VALUES ('t','a','Subject','','','');")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO messages (id, account_id, folder_id, thread_id, remote_id, remote_uidvalidity,
                 from_address, to_addresses, date, trust_metadata, pgp_signed, created_at, updated_at)
             VALUES ('m', 'a', 'f', 't', '42', 7, 'gpg-fixture@example.test', '[]', '',
                 '{\"root_crypto\":true}', ?, '', '')",
        )
        .bind(
            parse_raw_message_body(include_bytes!(
                "../../../crates/satchel-core/tests/corpus/pgp-mime/apple-mail-gpgtools.eml"
            ))
            .pgp_signed
            .unwrap(),
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO attachments (id, message_id, filename, content_type, listing_rules, created_at)
             VALUES ('older', 'm', 'signature.asc', 'application/pgp-signature', 0, '')",
        )
        .execute(&pool)
        .await
        .unwrap();

        let body = super::inline_pgp_body(&decrypted_root(), decrypted(raw)).unwrap();
        assert_eq!(body.signatures.len(), 1);
        let reference = crate::db::remote_content::ContentReference::load(&pool, "m")
            .await
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let bindings = super::persist_decrypted_attachments_in_dir(
            dir.path(),
            &pool,
            &reference,
            &body.attachments,
        )
        .await
        .unwrap();
        let [binding] = &bindings[..] else {
            panic!("bindings {bindings:?}");
        };
        assert_eq!(binding.filename, "figures.csv");

        let listed = attachments_for_messages(&pool, &["m".into()])
            .await
            .unwrap();
        let ids: Vec<&str> = listed.iter().map(|row| row.id.as_str()).collect();
        assert_eq!(
            ids,
            [binding.id.as_str()],
            "the older row waits for its rebuild"
        );
        assert_eq!(listed[0].cached_path, binding.cached_path);

        satchel_core::db::data_repairs::run_repair(
            &pool,
            &satchel_core::db::signed_attachments::SIGNED_ENTITY_ATTACHMENTS,
            10,
            None,
            |_| {},
        )
        .await
        .unwrap();
        let listed = attachments_for_messages(&pool, &["m".into()])
            .await
            .unwrap();
        let ids: Vec<&str> = listed.iter().map(|row| row.id.as_str()).collect();
        assert_eq!(ids, [binding.id.as_str()]);

        let prepared = prepare_attachment_in_dir(
            dir.path(),
            &pool,
            DownloadAttachmentInput {
                attachment_id: binding.id.clone(),
            },
        )
        .await
        .unwrap();
        let entity_files = satchel_core::providers::imap::signed_entity_attachments(
            parse_raw_message_body(raw).pgp_signed.as_deref().unwrap(),
        )
        .unwrap();
        let expected = base64::engine::general_purpose::STANDARD
            .decode(entity_files[0].content_base64.as_deref().unwrap())
            .unwrap();
        assert_eq!(
            tokio::fs::read(&prepared.cached_path).await.unwrap(),
            expected
        );
        assert!(reference.owns_attachment_cache(&prepared.cached_path, &binding.id));
    }

    #[test]
    fn stored_review_body_reaches_the_detail_view_without_decoding_again() {
        for raw in [
            include_bytes!("../../tests/corpus/11-review-code-emoji.eml").as_slice(),
            include_bytes!("../../tests/corpus/12-review-code-emoji-quoted-printable.eml")
                .as_slice(),
        ] {
            let parsed = super::parse_raw_message_body(raw);
            let row = stored_body_record(parsed.text, parsed.html);
            let dto = super::map_stored_message_row(row.clone());
            assert!(dto.body_text.contains("🧹 Nitpick comments"));
            assert!(dto.body_html.as_ref().unwrap().contains("🪄 Autofix"));
            assert_eq!(
                Some(dto.body_text),
                row.body_text,
                "plain text changed after MIME decoding"
            );
            assert_eq!(
                dto.body_html, row.body_html,
                "HTML changed after MIME decoding"
            );
        }
    }

    #[test]
    fn stored_body_preserves_literal_encoding_examples_and_code() {
        let text = "  =?UTF-8?Q?=C3=B6?= and =C3=B6 are examples, as is Ã¶.\nconst a =\n b;\nconst c =\n d;\nconst e =\n f;\n";
        let html = format!("<pre>{text}</pre>");
        let dto = super::map_stored_message_row(stored_body_record(
            Some(text.into()),
            Some(html.clone()),
        ));
        assert_eq!(dto.body_text, text);
        assert_eq!(dto.body_html.as_deref(), Some(html.as_str()));
    }

    #[test]
    fn stored_body_preserves_a_quoted_mime_document() {
        let text = "Content-Type: text/html; charset=utf-8\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\n<p>Literal =C3=B6</p>";
        let dto = super::map_stored_message_row(stored_body_record(Some(text.into()), None));
        assert_eq!(dto.body_text, text);
        assert!(dto.body_html.is_none());
    }
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/messages/")
)]
pub struct MessageSourceDto {
    pub message_id: String,
    pub subject: Option<String>,
    /// Raw RFC 822 header block as received. Populated after an original-source
    /// fetch, or absent when the provider cannot supply the message anymore.
    pub raw_headers: Option<String>,
    pub body_text: Option<String>,
    pub body_html: Option<String>,
    pub raw_size: Option<i64>,
    /// Complete RFC 822/MIME source fetched explicitly from the provider.
    pub raw_source: Option<String>,
    /// `loaded`, `unavailable` or `unsupported`.
    pub raw_source_status: String,
}

#[derive(sqlx::FromRow)]
struct RemoteMessageSourceTarget {
    message_id_header: Option<String>,
    remote_id: Option<String>,
    account_email: String,
    provider: String,
    account_id: String,
    config: String,
}

/// Reads on `read_pool`. Only a refresh that must persist rotated OAuth
/// credentials takes one of the writer's connections.
pub(super) async fn fetch_remote_message_source(
    read_pool: &SqlitePool,
    write_pool: &SqlitePool,
    message_id: &str,
) -> Result<Option<Vec<u8>>, String> {
    let reference = ContentReference::load(read_pool, message_id).await?;
    reference.validate_cached_read()?;
    let original_source: Option<Vec<u8>> =
        sqlx::query_scalar("SELECT original_source FROM messages WHERE id = ?")
            .bind(message_id)
            .fetch_optional(read_pool)
            .await
            .map_err(|error| error.to_string())?
            .flatten();
    if let Some(raw) = original_source.filter(|raw| {
        !raw.starts_with(&[0xd0, 0xcf, 0x11, 0xe0]) && raw_header_block(raw).is_some()
    }) {
        let mut tx = read_pool.begin().await.map_err(|e| e.to_string())?;
        reference.ensure_current(&mut tx).await?;
        return Ok(Some(raw));
    }
    let target = sqlx::query_as::<_, RemoteMessageSourceTarget>(
        "SELECT m.message_id_header, m.remote_id,
                a.email AS account_email, a.provider, a.id AS account_id, a.config
         FROM messages m
         JOIN accounts a ON a.id = m.account_id
         WHERE m.id = ? LIMIT 1",
    )
    .bind(message_id)
    .fetch_optional(read_pool)
    .await
    .map_err(|err| err.to_string())?
    .ok_or_else(|| "message not found".to_string())?;
    let Some(remote_id) = target.remote_id.filter(|value| !value.trim().is_empty()) else {
        return Ok(None);
    };
    reference.validate_cached_read()?;
    if target.provider == "imap" && !reference.is_remote_imap() {
        return Ok(None);
    }
    let config = crate::utils::oauth::refresh_account_config(
        write_pool,
        &target.account_id,
        &target.provider,
        &target.config,
    )
    .await
    .map_err(|err| err.encoded())?;

    let raw = match target.provider.as_str() {
        "imap" => {
            crate::providers::imap::ImapProvider::from_account_config(
                &config,
                &target.account_email,
            )?
            .fetch_raw_message(&reference.message_ref()?)
            .await?
        }
        "pop3" => {
            crate::providers::pop3::Pop3Provider::from_account_config(
                &config,
                &target.account_email,
            )?
            .fetch_raw_message(&remote_id)
            .await?
        }
        "gmail" => crate::providers::gmail::GmailProvider::from_account_config(&config)
            .fetch_raw_message(&remote_id)
            .await
            .map_err(|error| error.to_string())?,
        "outlook" => crate::providers::outlook::OutlookProvider::from_account_config(&config)
            .fetch_raw_message(&remote_id)
            .await
            .map_err(|error| error.to_string())?,
        _ => return Ok(None),
    };
    if let Some(expected) = target.message_id_header.as_deref().filter(|id| {
        !id.trim().is_empty()
            && !id.ends_with(&format!("@{}.satchel.local>", target.provider))
            && !id.ends_with(&format!("@{}.flowmail.local>", target.provider))
    }) {
        let actual =
            raw_header_block(&raw).and_then(|headers| parse_header_value(&headers, "Message-ID"));
        if actual.as_deref().map(str::trim) != Some(expected.trim()) {
            return Err("backend.errors.exportOriginalUnavailable".into());
        }
    }
    let mut tx = read_pool.begin().await.map_err(|err| err.to_string())?;
    reference.ensure_current(&mut tx).await?;
    Ok(Some(raw))
}

fn raw_header_block(raw: &[u8]) -> Option<String> {
    let split_at = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .or_else(|| raw.windows(2).position(|window| window == b"\n\n"));
    split_at
        .map(|index| String::from_utf8_lossy(&raw[..index]).into_owned())
        .filter(|headers| !headers.trim().is_empty())
}

async fn persist_raw_message_metadata(
    pool: &SqlitePool,
    reference: &ContentReference,
    raw: &[u8],
) -> Result<bool, String> {
    let message_id = &reference.message_id;
    let headers = raw_header_block(raw);
    let mut parsed = parse_raw_message_body(raw);
    let mut tx = reference.begin_write(pool).await?;
    let (provider, config): (String, String) = sqlx::query_as("SELECT a.provider, a.config FROM accounts a JOIN messages m ON m.account_id = a.id WHERE m.id = ?").bind(message_id).fetch_one(&mut *tx).await.map_err(|error| error.to_string())?;
    let config: serde_json::Value = serde_json::from_str(&config).unwrap_or_default();
    // Assigning a local import to an account does not establish a receiving boundary.
    let receiver = if message_id.starts_with("m-import-") {
        ""
    } else if matches!(provider.as_str(), "gmail" | "outlook") {
        provider.as_str()
    } else {
        config
            .get(&provider)
            .and_then(|value| value.get("host"))
            .or_else(|| config.get("host"))
            .and_then(|value| value.as_str())
            .unwrap_or_default()
    };
    let repaired_smime = parsed.smime_signed.is_some();
    // The rows an older parse listed beside what is now a root signature are
    // settled with the bundle that signature comes with.
    let legacy_rows = parsed
        .pgp_signed
        .as_deref()
        .or(parsed.smime_signed.as_deref())
        .filter(|_| parsed.trust.root_crypto)
        .map(satchel_core::db::signed_attachments::LegacyRows::from_bundle);
    let (auth_spf, auth_dkim, auth_dmarc) = headers
        .as_deref()
        .and_then(|value| parse_header_value(value, "Authentication-Results"))
        .map(|value| crate::providers::traits::trusted_authentication_results(&value, receiver))
        .unwrap_or((None, None, None));
    parsed.trust.auth_verified = auth_spf.is_some() || auth_dkim.is_some() || auth_dmarc.is_some();
    let raw_size = i64::try_from(raw.len()).unwrap_or(i64::MAX);

    sqlx::query(
        "UPDATE messages
         SET original_source = ?,
             raw_headers = COALESCE(?, raw_headers),
             raw_size = ?,
             smime_encrypted = ?,
             smime_signed = ?,
             pgp_signed = ?,
             trust_metadata = ?,
             body_text = CASE WHEN ? THEN ? ELSE body_text END,
             body_html = CASE WHEN ? THEN ? ELSE body_html END,
             auth_spf = ?,
             auth_dkim = ?,
             auth_dmarc = ?,
             updated_at = ?
         WHERE id = ?",
    )
    .bind(raw)
    .bind(headers)
    .bind(raw_size)
    .bind(parsed.smime_encrypted)
    .bind(parsed.smime_signed)
    .bind(parsed.pgp_signed)
    .bind(serde_json::to_string(&parsed.trust).map_err(|err| err.to_string())?)
    .bind(parsed.trust.root_crypto)
    .bind(parsed.text)
    .bind(parsed.trust.root_crypto)
    .bind(parsed.html)
    .bind(auth_spf)
    .bind(auth_dkim)
    .bind(auth_dmarc)
    .bind(Utc::now().to_rfc3339())
    .bind(message_id)
    .execute(&mut *tx)
    .await
    .map_err(|err| err.to_string())?;
    if let Some(legacy_rows) = legacy_rows {
        satchel_core::db::signed_attachments::settle_legacy_rows(&mut tx, message_id, &legacy_rows)
            .await
            .map_err(|err| err.to_string())?;
    }
    tx.commit().await.map_err(|err| err.to_string())?;
    Ok(repaired_smime)
}

/// Read a single message's raw headers and bodies for the "show source" view.
///
/// Deliberately separate from `list_message_bodies`: raw headers are large, only
/// ever needed on explicit user request, and would otherwise be loaded for
/// every message in every thread.
#[tauri::command]
pub async fn get_message_source(
    pool: tauri::State<'_, crate::db::ReadPool>,
    write_pool: tauri::State<'_, SqlitePool>,
    message_id: String,
) -> CommandResult<MessageSourceDto> {
    let reference = ContentReference::load(pool.inner().as_ref(), &message_id).await?;
    reference.validate_cached_read()?;
    let source = MessageReadRepository::new(pool.inner().as_ref())
        .get_message_source(&message_id)
        .await
        .map_err(CommandError::from)?;

    let (raw_source, fetched_headers, fetched_size, raw_source_status) =
        match fetch_remote_message_source(pool.inner().as_ref(), write_pool.inner(), &message_id)
            .await
        {
            Ok(Some(raw)) => {
                let headers = raw_header_block(&raw);
                let size = i64::try_from(raw.len()).unwrap_or(i64::MAX);
                persist_raw_message_metadata(write_pool.inner(), &reference, &raw)
                    .await
                    .map_err(CommandError::from)?;
                (
                    Some(String::from_utf8_lossy(&raw).into_owned()),
                    headers,
                    Some(size),
                    "loaded",
                )
            }
            Ok(None) => (None, None, None, "unsupported"),
            Err(error) => {
                let command_error = CommandError::from(error.clone());
                if matches!(
                    command_error.key.as_str(),
                    "backend.errors.imapUidValidityMissing"
                        | "backend.errors.imapUidValidityChanged"
                ) {
                    return Err(command_error);
                }
                tracing::warn!(%message_id, %error, "could not fetch original message source");
                (None, None, None, "unavailable")
            }
        };

    let mut tx = pool
        .inner()
        .as_ref()
        .begin()
        .await
        .map_err(|e| e.to_string())?;
    reference.ensure_current(&mut tx).await?;
    Ok(MessageSourceDto {
        message_id: source.message_id,
        subject: source.subject,
        raw_headers: fetched_headers.or(source.raw_headers),
        body_text: source.body_text,
        body_html: source.body_html,
        raw_size: fetched_size.or(source.raw_size),
        raw_source,
        raw_source_status: raw_source_status.to_string(),
    })
}
