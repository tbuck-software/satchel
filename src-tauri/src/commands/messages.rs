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
        verify_incoming_detached_signature, IncomingPgpDetachedSignature, IncomingPgpSignature,
        PGP_DECRYPTED_BLOCK_END, PGP_DECRYPTED_BLOCK_START,
    },
    commands::signatures::{
        resolve_signature_for_compose, ResolveSignatureInput, SignaturePreference, SignatureVariant,
    },
    commands::smime::{
        decrypt_message_with_unlocked_smime_certs, verify_smime_message, SMIME_DECRYPTED_BLOCK_END,
        SMIME_DECRYPTED_BLOCK_START,
    },
    email::sanitizer::sanitize_email_html,
    events::bus::AppEvent,
    plugins::runtime::{MessageTransformInput, PluginRuntime},
    providers::imap::{is_legacy_truncated_body, parse_header_value, parse_raw_message_body},
    providers::traits::MessageRef,
    sync::outbox::{build_send_action, execute_send_action},
    utils::mime::{
        decode_html_entities, decode_mime_encoded_words, extract_email_address,
        repair_likely_utf8_mojibake, resolve_sender_display_name,
    },
    AppState,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/messages/")
)]
pub struct MessageDto {
    pub id: String,
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
    pub snippet: String,
    pub body_text: String,
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

    let messages = rows
        .into_iter()
        .map(map_message_preview_row)
        .collect::<Vec<_>>();
    log_perf(
        "list_messages_preview",
        started_at,
        format!("thread_id={} rows={}", thread_id, messages.len()),
    );
    Ok(messages)
}

/// Older parser versions stored the detached signature as an attachment but not
/// as a verifiable MIME bundle. Refresh only those explicit candidates, never
/// the rest of a thread. Returns the threads that changed.
async fn repair_unparsed_smime_messages(pool: &SqlitePool, message_ids: &[String]) -> Vec<String> {
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
    let Ok(candidates) = query.fetch_all(pool).await else {
        return Vec::new();
    };

    let mut repaired: Vec<String> = Vec::new();
    for (message_id, thread_id) in candidates {
        match fetch_remote_message_source(pool, &message_id).await {
            Ok(Some(raw)) => match persist_raw_message_metadata(pool, &message_id, &raw).await {
                Ok(true) => {
                    if !repaired.contains(&thread_id) {
                        repaired.push(thread_id);
                    }
                }
                Ok(false) => {}
                Err(error) => {
                    tracing::warn!(%message_id, %error, "could not persist repaired S/MIME metadata");
                }
            },
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(%message_id, %error, "could not refresh unparsed S/MIME message");
            }
        }
    }
    repaired
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
    let messages = rows
        .into_iter()
        .map(map_message_preview_row)
        .collect::<Vec<_>>();
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

    let repair_pool = write_pool.clone();
    let repair_ids = message_ids.to_vec();
    let repair_events = state.events.clone();
    tauri::async_runtime::spawn(async move {
        for thread_id in repair_unparsed_smime_messages(&repair_pool, &repair_ids).await {
            repair_events.publish(AppEvent::ThreadsUpdated(vec![thread_id]));
        }
    });

    let runtime = state.plugin_runtime.clone();
    let plugin_ids = runtime.enabled_message_transform_plugins()?;
    let has_pgp_private_key = has_any_private_pgp_key(pool).await;
    let has_smime_private_cert = has_any_private_smime_cert(pool).await;
    let rows = MessageReadRepository::new(pool)
        .list_message_bodies(message_ids)
        .await
        .map_err(CommandError::from)?;

    let mut bodies = Vec::with_capacity(rows.len());
    for row in rows {
        // Keep this state from the stored columns. `map_message_row` replaces
        // a missing body with the snippet so list views always have readable
        // text. Old IMAP bodies at the former 64-KiB cutoff likewise remain
        // readable as a cached prefix while the full body is fetched.
        let has_stored_body = row
            .body_text
            .as_deref()
            .is_some_and(|text| !text.trim().is_empty())
            || row
                .body_html
                .as_deref()
                .is_some_and(|html| !html.trim().is_empty());
        let body_loaded = has_stored_body
            && !(row.id.starts_with("m-imap-")
                && (is_legacy_truncated_body(row.body_text.as_deref())
                    || is_legacy_truncated_body(row.body_html.as_deref())));
        let dto = map_message_row(
            row,
            app,
            state,
            write_pool,
            &runtime,
            &plugin_ids,
            has_pgp_private_key,
            has_smime_private_cert,
        )
        .await;

        bodies.push(MessageBodyDto {
            id: dto.id,
            snippet: dto.snippet,
            body_text: dto.body_text,
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

    let mut dto = map_stored_message_row(row);
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
    if dto.body_text.contains("-----BEGIN PGP MESSAGE-----")
        || dto.body_text.contains(PGP_DECRYPTED_BLOCK_START)
        || dto.body_text.contains(SMIME_DECRYPTED_BLOCK_START)
    {
        dto.body_html = None;
    }

    if trust.root_encryption
        && original_body_text
            .trim_start()
            .starts_with("-----BEGIN PGP MESSAGE-----")
    {
        match decrypt_message_with_unlocked_keys(app, state, pool, &original_body_text).await {
            Ok(Some(decrypted)) if !decrypted.display_body.trim().is_empty() => {
                let parsed =
                    crate::providers::imap::parse_raw_message_body(decrypted.plaintext.as_bytes());
                if let Err(error) =
                    persist_decrypted_pgp_attachments(app, pool, &dto.id, &parsed.attachments).await
                {
                    tracing::warn!(message_id = %dto.id, %error, "could not cache decrypted PGP attachments");
                }
                let readable_text = parsed
                    .text
                    .filter(|text| !text.trim().is_empty())
                    .unwrap_or(decrypted.display_body);
                dto.snippet = first_non_empty_line(&readable_text).unwrap_or(dto.snippet.clone());
                dto.body_text = readable_text;
                dto.body_html = parsed.html.filter(|html| !html.trim().is_empty());
                if let Some(inner_signed) = parsed.pgp_signed.as_deref() {
                    apply_incoming_pgp_signed_status(&mut dto, app, pool, inner_signed).await;
                    apply_signature_scope(&mut dto.pgp_state, parsed.trust.root_crypto);
                } else {
                    dto.pgp_state = Some("decrypted".to_string());
                    dto.pgp_error_detail = None;
                }
            }
            Ok(_) => {}
            Err(_) => {
                dto.pgp_state = Some("error".to_string());
                dto.pgp_error_detail =
                    Some(CommandError::new("backend.errors.pgpDecryptFailed").encoded());
            }
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

    if let Some(signed_bundle) = pgp_signed.as_deref() {
        apply_incoming_pgp_signed_status(&mut dto, app, pool, signed_bundle).await;
        apply_signature_scope(&mut dto.pgp_state, trust.root_crypto);
    }

    if let Some(ciphertext) = smime_encrypted.as_deref() {
        dto.body_html = None;
        match decrypt_message_with_unlocked_smime_certs(app, state, pool, ciphertext).await {
            Ok(Some(decrypted_text)) if !decrypted_text.trim().is_empty() => {
                dto.snippet = first_non_empty_line(&decrypted_text).unwrap_or(dto.snippet.clone());
                dto.body_text = decrypted_text;
                dto.smime_state = Some("decrypted".to_string());
                dto.smime_error_detail = None;
            }
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
                dto.smime_error_detail = Some(err);
            }
        }
    }

    if let Some(signed_payload) = smime_signed.as_deref() {
        let verified =
            verify_smime_message(app, pool, &dto.id, signed_payload, &dto.from_address).await;
        dto.smime_state = Some(verified.state);
        apply_signature_scope(&mut dto.smime_state, trust.root_crypto);
        dto.smime_signer_email = verified.signer_email;
        dto.smime_signer_cn = verified.signer_cn;
        dto.smime_error_detail = verified.error_detail;
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
    dto
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

async fn apply_incoming_pgp_signed_status(
    dto: &mut MessageDto,
    app: &tauri::AppHandle,
    pool: &SqlitePool,
    signed_bundle: &[u8],
) {
    let from_address = dto.from_address.clone();
    let verification = match crate::email::pgp_mime::parse_signed_message(signed_bundle) {
        Ok(parsed) => {
            verify_incoming_detached_signature(
                app,
                pool,
                &dto.id,
                &parsed.signed_entity,
                &parsed.armored_signature,
                &from_address,
            )
            .await
        }
        Err(error) => Err(error),
    };

    match verification {
        Ok(IncomingPgpDetachedSignature::Valid {
            signer_name,
            signer_fingerprint,
        }) => {
            dto.pgp_state = Some("signature_valid".to_string());
            dto.pgp_error_detail = None;
            dto.pgp_signer_name = signer_name;
            dto.pgp_signer_fingerprint = Some(signer_fingerprint);
        }
        Ok(IncomingPgpDetachedSignature::IdentityMismatch {
            signer_name,
            signer_fingerprint,
        }) => {
            dto.pgp_state = Some("signature_identity_mismatch".to_string());
            dto.pgp_error_detail = None;
            dto.pgp_signer_name = signer_name;
            dto.pgp_signer_fingerprint = Some(signer_fingerprint);
        }
        Ok(IncomingPgpDetachedSignature::Invalid) => {
            dto.pgp_state = Some("signature_invalid".to_string());
            dto.pgp_error_detail = None;
            dto.pgp_signer_name = None;
            dto.pgp_signer_fingerprint = None;
        }
        Ok(IncomingPgpDetachedSignature::UnknownKey) => {
            dto.pgp_state = Some("signature_unknown_key".to_string());
            dto.pgp_error_detail = None;
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

async fn persist_decrypted_pgp_attachments(
    app: &tauri::AppHandle,
    pool: &SqlitePool,
    message_id: &str,
    attachments: &[crate::providers::traits::RemoteAttachment],
) -> Result<(), String> {
    if attachments.is_empty() {
        return Ok(());
    }

    let cache_dir = app
        .path()
        .resolve("satchel/attachments", tauri::path::BaseDirectory::AppData)
        .map_err(|error| error.to_string())?;
    tokio::fs::create_dir_all(&cache_dir)
        .await
        .map_err(|error| format!("create attachment cache: {error}"))?;
    let now = chrono::Utc::now().to_rfc3339();

    for attachment in attachments {
        let Some(encoded) = attachment.content_base64.as_deref() else {
            continue;
        };
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|error| format!("decode decrypted attachment: {error}"))?;
        let attachment_id = format!("att-pgp-{message_id}-{}", attachment.remote_part_id);
        let filename = attachment
            .filename
            .clone()
            .unwrap_or_else(|| "attachment".to_string());
        let path = crate::commands::attachments::attachment_cache_path(
            &cache_dir,
            &attachment_id,
            &filename,
        );
        tokio::fs::write(&path, &bytes)
            .await
            .map_err(|error| format!("write decrypted attachment cache: {error}"))?;
        let path = path.to_string_lossy().to_string();

        sqlx::query(
            "INSERT INTO attachments
             (id, message_id, filename, content_type, size, content_id, storage_path, is_inline,
              cached_path, remote_uid, remote_part_id, created_at)
             VALUES (?, ?, ?, ?, ?, ?, NULL, ?, ?, NULL, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
               filename = excluded.filename,
               content_type = excluded.content_type,
               size = excluded.size,
               content_id = excluded.content_id,
               is_inline = excluded.is_inline,
               cached_path = excluded.cached_path",
        )
        .bind(&attachment_id)
        .bind(message_id)
        .bind(&filename)
        .bind(&attachment.content_type)
        .bind(i64::try_from(bytes.len()).unwrap_or(i64::MAX))
        .bind(attachment.content_id.as_deref())
        .bind(if attachment.is_inline { 1 } else { 0 })
        .bind(&path)
        .bind(&attachment.remote_part_id)
        .bind(&now)
        .execute(pool)
        .await
        .map_err(|error| error.to_string())?;
    }

    sqlx::query("UPDATE messages SET has_attachments = 1 WHERE id = ?")
        .bind(message_id)
        .execute(pool)
        .await
        .map_err(|error| error.to_string())?;
    sqlx::query(
        "UPDATE threads SET has_attachments = 1
         WHERE id = (SELECT thread_id FROM messages WHERE id = ?)",
    )
    .bind(message_id)
    .execute(pool)
    .await
    .map_err(|error| error.to_string())?;
    Ok(())
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
        .find(|line| {
            !line.is_empty()
                && *line != PGP_DECRYPTED_BLOCK_START
                && *line != PGP_DECRYPTED_BLOCK_END
                && *line != SMIME_DECRYPTED_BLOCK_START
                && *line != SMIME_DECRYPTED_BLOCK_END
        })
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
    let normalized_to = normalize_recipients(&input.to)?;
    if normalized_to.is_empty() {
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

fn parse_scheduled_for(raw: Option<&str>) -> Result<Option<DateTime<Utc>>, String> {
    let Some(raw) = raw.map(str::trim).filter(|raw| !raw.is_empty()) else {
        return Ok(None);
    };
    let parsed = DateTime::parse_from_rfc3339(raw)
        .map_err(|err| format!("invalid scheduled_for timestamp: {err}"))?
        .with_timezone(&Utc);
    if parsed <= Utc::now() {
        return Ok(None);
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

    let message_body_text = input.body.clone();
    if !resolved.body_text.trim().is_empty() {
        input.body = format!("{}\n\n-- \n{}", input.body.trim_end(), resolved.body_text);
    }
    if resolved.body_html.is_some() || input.body_html.is_some() {
        let message_html = input
            .body_html
            .clone()
            .unwrap_or_else(|| format!("<div>{}</div>", plain_text_to_html(&message_body_text)));
        let signature_html = resolved
            .body_html
            .clone()
            .unwrap_or_else(|| plain_text_to_html(&resolved.body_text));
        input.body_html = Some(format!(
            "{message_html}<div data-satchel-signature=\"{}\">{signature_html}</div>",
            html_attribute_escape(&resolved.signature_id)
        ));
    }
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

fn plain_text_to_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\n', "<br>")
}

fn html_attribute_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
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
    #[test]
    fn opening_a_thread_loads_the_latest_bodies_and_every_draft() {
        let row = |id: &str, draft: i64| super::MessagePreviewRecord {
            id: id.to_string(),
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

    use super::{is_owned_sender, normalize_recipients, raw_header_block};

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
        super::persist_raw_message_metadata(&pool, "m", raw)
            .await
            .unwrap();
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
        super::persist_raw_message_metadata(&pool, "m", provider_raw.as_bytes())
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
        super::persist_raw_message_metadata(&pool, "m-import-fixture", provider_raw.as_bytes())
            .await
            .unwrap();
        let imported_verdict: Option<String> =
            sqlx::query_scalar("SELECT auth_dmarc FROM messages WHERE id = 'm-import-fixture'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(imported_verdict.is_none());
    }

    fn stored_body_record(
        body_text: Option<String>,
        body_html: Option<String>,
    ) -> super::MessageReadRecord {
        super::MessageReadRecord {
            id: "message".into(),
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
    remote_id: Option<String>,
    folder_remote_id: Option<String>,
    folder_uidvalidity: Option<i64>,
    account_email: String,
    provider: String,
    account_id: String,
    config: String,
}

async fn fetch_remote_message_source(
    pool: &SqlitePool,
    message_id: &str,
) -> Result<Option<Vec<u8>>, String> {
    let original_source: Option<Vec<u8>> =
        sqlx::query_scalar("SELECT original_source FROM messages WHERE id = ?")
            .bind(message_id)
            .fetch_optional(pool)
            .await
            .map_err(|error| error.to_string())?
            .flatten();
    if let Some(raw) = original_source.filter(|raw| !raw.starts_with(&[0xd0, 0xcf, 0x11, 0xe0])) {
        return Ok(Some(raw));
    }
    let target = sqlx::query_as::<_, RemoteMessageSourceTarget>(
        "SELECT m.remote_id, f.remote_id AS folder_remote_id, f.uidvalidity AS folder_uidvalidity,
                a.email AS account_email, a.provider, a.id AS account_id, a.config
         FROM messages m
         JOIN accounts a ON a.id = m.account_id
         LEFT JOIN folders f ON f.id = m.folder_id
         WHERE m.id = ? LIMIT 1",
    )
    .bind(message_id)
    .fetch_optional(pool)
    .await
    .map_err(|err| err.to_string())?
    .ok_or_else(|| "message not found".to_string())?;
    let Some(remote_id) = target.remote_id.filter(|value| !value.trim().is_empty()) else {
        return Ok(None);
    };
    let config =
        crate::commands::accounts::decrypt_account_config(&target.account_id, &target.config)?;

    let raw = match target.provider.as_str() {
        "imap" => {
            crate::providers::imap::ImapProvider::from_account_config(
                &config,
                &target.account_email,
            )?
            .fetch_raw_message(&MessageRef {
                remote_id,
                source_folder: target.folder_remote_id,
                source_uidvalidity: target
                    .folder_uidvalidity
                    .and_then(|value| u32::try_from(value).ok()),
            })
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
    message_id: &str,
    raw: &[u8],
) -> Result<bool, String> {
    let headers = raw_header_block(raw);
    let mut parsed = parse_raw_message_body(raw);
    let (provider, config): (String, String) = sqlx::query_as("SELECT a.provider, a.config FROM accounts a JOIN messages m ON m.account_id = a.id WHERE m.id = ?").bind(message_id).fetch_one(pool).await.map_err(|error| error.to_string())?;
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
    let (auth_spf, auth_dkim, auth_dmarc) = headers
        .as_deref()
        .and_then(|value| parse_header_value(value, "Authentication-Results"))
        .map(|value| crate::providers::traits::trusted_authentication_results(&value, receiver))
        .unwrap_or((None, None, None));
    parsed.trust.auth_verified = auth_spf.is_some() || auth_dkim.is_some() || auth_dmarc.is_some();
    let raw_size = i64::try_from(raw.len()).unwrap_or(i64::MAX);

    sqlx::query(
        "UPDATE messages
         SET raw_headers = COALESCE(?, raw_headers),
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
    .execute(pool)
    .await
    .map_err(|err| err.to_string())?;
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
    let source = MessageReadRepository::new(pool.inner().as_ref())
        .get_message_source(&message_id)
        .await
        .map_err(CommandError::from)?;

    let (raw_source, fetched_headers, fetched_size, raw_source_status) =
        match fetch_remote_message_source(pool.inner().as_ref(), &message_id).await {
            Ok(Some(raw)) => {
                let headers = raw_header_block(&raw);
                let size = i64::try_from(raw.len()).unwrap_or(i64::MAX);
                persist_raw_message_metadata(write_pool.inner(), &message_id, &raw)
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
                tracing::warn!(%message_id, %error, "could not fetch original message source");
                (None, None, None, "unavailable")
            }
        };

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
