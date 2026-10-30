//! Persistent outbox: encapsulates persisted send actions and their execution.
//!
//! Flow:
//! 1. `commands::messages::send_message` calls `persist_send_action` BEFORE attempting
//!    the network send. The action is stored with status='sending'.
//! 2. Worker (or the command itself) calls `execute_send_action`. On success it deletes
//!    the row; on retryable failure it bumps retry_count and sets next_retry_at.
//! 3. On app start, `SyncQueue::reset_sending` retries orphaned remote mutations,
//!    but leaves ambiguous mail deliveries terminal so they cannot be duplicated.
//!    Ambiguous means `submission_started_at` is set: a send that crashed before
//!    the provider saw a byte goes back to `pending` instead of being lost.

use satchel_core::{
    db::{messages::SentMessageRepository, smime::SmimeRepositoryError},
    sync::errors::{SyncError, SyncResult},
};
use sqlx::SqlitePool;

use crate::{
    commands::{
        error::CommandError,
        messages::{persist_sent_message_record, OutgoingMessageInput, SendMessageResult},
        pgp, smime,
    },
    email::build::{build_rfc822, with_bcc_delivery_header},
    providers::{provider_for_stored_account, traits::OutgoingMessage},
    sync::queue::QueuedAction,
    AppState,
};

pub use satchel_core::sync::queue::ACTION_SEND_MESSAGE;

pub use satchel_core::drafts::send_queue::{build_send_action, SendActionPayload};

/// Build a send action whose queue id and Message-ID are stable for one
/// logical operation. Re-enqueuing the same operation can therefore neither
/// create a second pending row nor resend after the first row was completed:
/// `execute_send_action` recognizes the persisted Message-ID on recovery.
pub fn build_idempotent_send_action(
    account_id: String,
    input: OutgoingMessageInput,
    idempotency_key: &str,
) -> Result<QueuedAction, String> {
    use sha2::{Digest, Sha256};

    let digest = Sha256::digest(idempotency_key.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let payload = SendActionPayload {
        account_id: account_id.clone(),
        input,
        draft_reference: None,
        scheduled_for: None,
        message_id_header: Some(format!("<calendar-rsvp-{digest}@satchel.local>")),
    };
    let payload_value =
        serde_json::to_value(&payload).map_err(|err| format!("encode send payload: {err}"))?;
    let mut action = QueuedAction::new(account_id, ACTION_SEND_MESSAGE, payload_value);
    action.id = format!("calendar-rsvp-{digest}");
    // Unlike send_message, this caller does not execute the action inline.
    action.status = "pending".into();
    Ok(action)
}

#[derive(Debug, sqlx::FromRow)]
struct AccountRow {
    pub id: String,
    pub display_name: String,
    pub provider: String,
    pub email: String,
    pub config: String,
}

pub async fn execute_send_action(
    pool: &SqlitePool,
    app: &tauri::AppHandle,
    state: &AppState,
    action: &QueuedAction,
) -> crate::sync::errors::SyncResult<SendMessageResult> {
    let _work = state.sync_activity.start_work();
    let payload: SendActionPayload = serde_json::from_value(action.payload.clone())
        .map_err(|err| format!("decode send payload: {err}"))?;

    // Idempotency pre-check: if a message with this stable Message-ID already exists
    // locally as `is_sent=1`, the previous attempt actually succeeded — recover the
    // bookkeeping and don't re-send.
    if let Some(mid) = payload.message_id_header.as_deref() {
        if let Some(existing) = sqlx::query_as::<_, (String, String)>(
            "SELECT id, thread_id FROM messages WHERE message_id_header = ? AND is_sent = 1 LIMIT 1",
        )
        .bind(mid)
        .fetch_optional(pool)
        .await
        .map_err(|err| err.to_string())?
        {
            return Ok(SendMessageResult {
                status: "sent".into(),
                thread_id: existing.1,
                message_id: existing.0,
                queued_id: Some(action.id.clone()),
                undo_until: None,
            });
        }
    }

    let account = sqlx::query_as::<_, AccountRow>(
        "SELECT id, display_name, provider, email, config FROM accounts WHERE id = ? LIMIT 1",
    )
    .bind(&payload.account_id)
    .fetch_one(pool)
    .await
    .map_err(|err| format!("account lookup failed: {err}"))?;
    let config = crate::utils::oauth::refresh_account_config(
        pool,
        &account.id,
        &account.provider,
        &account.config,
    )
    .await?;

    let provider = provider_for_stored_account(
        pool,
        &account.id,
        &account.provider,
        &config,
        &account.email,
        None,
    )?;

    let mut outgoing = build_outgoing_message(
        pool,
        &payload.input,
        &payload.account_id,
        payload.message_id_header.clone(),
    )
    .await?;
    let request_rr = sqlx::query_scalar::<_, String>(
        "SELECT value FROM app_settings WHERE key = 'compose.send_read_receipts'",
    )
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    .map(|v| {
        matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
    .unwrap_or(false);
    if request_rr {
        let rr_addr = outgoing
            .reply_to
            .clone()
            .or_else(|| outgoing.from_address.clone())
            .unwrap_or_else(|| account.email.clone());
        outgoing.request_read_receipt_to = Some(rr_addr);
    }
    apply_pgp_send_transform(
        pool,
        app,
        state,
        &mut outgoing,
        &account.email,
        &account.display_name,
    )
    .await?;
    apply_smime_send_transform(
        pool,
        app,
        state,
        &mut outgoing,
        &account.email,
        &account.display_name,
    )
    .await?;
    apply_bcc_delivery_header(&mut outgoing, recipient_source(&account.provider));
    // Resolve the sender identity before handing the message to a provider.
    // Providers that submit MIME (Outlook, and Gmail via messages.send) build
    // the From header from these fields; leaving them None produced an empty
    // "From:" and relied on the API to fill it in.
    if outgoing
        .from_address
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_none()
    {
        outgoing.from_address = Some(account.email.clone());
    }
    if outgoing
        .from_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_none()
        && !account.display_name.trim().is_empty()
    {
        outgoing.from_name = Some(account.display_name.clone());
    }

    // Decide where the Sent copy comes from before send consumes the message —
    // SMTP-only accounts need it APPENDed to the remote Sent folder below.
    let sent_copy = (account.provider == "imap").then(|| sent_copy_source(&outgoing));

    // Point of no return. Before this line a crash cannot have produced a
    // delivery, so recovery may send the row again; after it, it may not.
    state
        .queue
        .mark_submission_started(pool, &action.id)
        .await?;
    let wire_copy = provider.send_returning_wire_copy(outgoing).await?;

    // SMTP doesn't file a Sent copy; without this the mail exists only in the
    // local DB and is invisible in webmail/other clients. A failure must not
    // fail the send — the mail IS out, and retrying this action would resend
    // it — so the copy gets a queue row of its own that only ever appends.
    if let Some(raw) = sent_copy.and_then(|source| source.resolve(wire_copy)) {
        let folder_hint = sqlx::query_scalar::<_, Option<String>>(
            "SELECT COALESCE(remote_id, full_path) FROM folders
             WHERE account_id = ? AND folder_type = 'sent' LIMIT 1",
        )
        .bind(&account.id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .flatten();
        if let Err(err) = provider.append_sent(&raw, folder_hint.as_deref()).await {
            tracing::warn!(target: "outbox", "append to Sent failed, queued for retry: {err}");
            if let Err(queue_err) = satchel_core::sync::remote_mutations::enqueue_append_sent(
                pool,
                &state.queue,
                &account.id,
                &raw,
                folder_hint.as_deref(),
            )
            .await
            {
                tracing::error!(target: "outbox", "could not queue the Sent copy: {queue_err}");
            }
        }
    }

    let result = persist_sent_message_record(
        pool,
        &account.id,
        &account.email,
        &account.display_name,
        &payload.input,
        payload.message_id_header.as_deref(),
    )
    .await
    .map_err(|error| crate::sync::errors::SyncError::Delivered(error.encoded()));

    // The default send path goes through the undo-send delay, i.e. through this
    // worker — the draft cleanup in the send_message command's immediate branch
    // never runs for it. Without this, every sent mail leaves a stale draft.
    if result.is_ok() {
        if let Some(draft_id) = payload.input.draft_id.as_deref() {
            if let Err(err) =
                satchel_core::drafts::delete_after_send(pool, &state.queue, draft_id).await
            {
                tracing::warn!("sent queued message but failed to delete remote draft: {err}");
            }
        }
    }

    result
}

/// Where the copy filed in the remote Sent folder comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
enum SentCopy {
    /// A crypto transform already produced the wire form. It is the only
    /// version that exists in ciphertext — rebuilding would file the plaintext.
    Prebuilt(String),
    /// Nothing was transformed: the transport's own bytes are the copy.
    Transport,
}

impl SentCopy {
    fn resolve(self, wire_copy: Option<Vec<u8>>) -> Option<Vec<u8>> {
        match self {
            SentCopy::Prebuilt(raw) => Some(raw.into_bytes()),
            SentCopy::Transport => wire_copy,
        }
    }
}

fn sent_copy_source(outgoing: &OutgoingMessage) -> SentCopy {
    match outgoing
        .raw_rfc822_override
        .as_deref()
        .filter(|raw| !raw.trim().is_empty())
    {
        Some(raw) => SentCopy::Prebuilt(raw.to_string()),
        None => SentCopy::Transport,
    }
}

async fn build_outgoing_message(
    pool: &SqlitePool,
    input: &OutgoingMessageInput,
    account_id: &str,
    message_id_header: Option<String>,
) -> Result<OutgoingMessage, String> {
    let normalize = |values: &[String]| {
        crate::commands::messages::normalize_recipients(values).map_err(|error| error.encoded())
    };
    let to = normalize(&input.to)?;
    let cc = normalize(&input.cc)?;
    let bcc = normalize(&input.bcc)?;
    if to.is_empty() && cc.is_empty() && bcc.is_empty() {
        return Err("missing recipient address".to_string());
    }

    let reply_context = if let Some(thread_id) = input.thread_id.as_deref() {
        Some(
            SentMessageRepository::new(pool)
                .load_reply_context(account_id, thread_id, input.reply_to_message_id.as_deref())
                .await
                .map_err(|error| CommandError::from(error).encoded())?,
        )
    } else {
        None
    };

    let attachments = input
        .attachments
        .iter()
        .map(|att| crate::providers::traits::OutgoingAttachment {
            filename: att.filename.clone(),
            mime_type: att.mime_type.clone(),
            content_base64: att.content_base64.clone(),
            content_id: att.content_id.clone(),
            inline: att.inline,
        })
        .collect::<Vec<_>>();

    Ok(OutgoingMessage {
        from_address: input.from_address.clone(),
        from_name: input.from_name.clone(),
        reply_to: input.reply_to.clone(),
        to,
        cc,
        bcc,
        recipient_names: crate::commands::messages::recipient_display_names(&[
            &input.to, &input.cc, &input.bcc,
        ]),
        subject: input.subject.clone(),
        body: input.body.clone(),
        body_html: input.body_html.clone(),
        in_reply_to: reply_context.as_ref().and_then(|c| c.in_reply_to.clone()),
        references: reply_context
            .as_ref()
            .map(|c| c.references.clone())
            .unwrap_or_default(),
        attachments,
        message_id_header,
        request_read_receipt_to: None,
        raw_rfc822_override: None,
        smime_sign_with_cert_id: input.smime_sign_with_cert_id.clone(),
        smime_encrypt_to_recipients: input.smime_encrypt_to_recipients,
        pgp_sign_with_key_id: input.pgp_sign_with_key_id.clone(),
        pgp_encrypt_to_recipients: input.pgp_encrypt_to_recipients,
        priority: input.priority.clone(),
    })
}

/// Apply PGP/MIME signing and/or encryption (RFC 3156).
///
/// Runs before the S/MIME transform and bails if both are requested: layering
/// the two produces a message almost nothing can read, and silently dropping
/// one of them would be worse.
async fn apply_pgp_send_transform(
    pool: &SqlitePool,
    app: &tauri::AppHandle,
    state: &AppState,
    outgoing: &mut OutgoingMessage,
    fallback_from_address: &str,
    fallback_from_name: &str,
) -> Result<(), String> {
    let sign_key = outgoing.pgp_sign_with_key_id.clone();
    let encrypt = outgoing.pgp_encrypt_to_recipients;
    if sign_key.is_none() && !encrypt {
        return Ok(());
    }

    if outgoing.smime_sign_with_cert_id.is_some() || outgoing.smime_encrypt_to_recipients {
        return Err("backend.errors.pgpSmimeConflict".to_string());
    }

    let original = build_rfc822(
        outgoing,
        Some(fallback_from_address),
        Some(fallback_from_name),
    )
    .map_err(|error| error.encoded())?;

    let signature = match sign_key.as_deref() {
        Some(key_id) => {
            let (armored_key, passphrase) = pgp::load_private_signing_key(app, state, pool, key_id)
                .await
                .map_err(|_| "backend.errors.pgpSigningKeyUnavailable".to_string())?;
            let entity = crate::email::pgp_mime::signable_entity(&original);
            Some(
                crate::utils::pgp_engine::sign_detached(
                    &armored_key,
                    &passphrase,
                    entity.as_bytes(),
                )
                .map_err(|_| "backend.errors.pgpSignFailed".to_string())?,
            )
        }
        None => None,
    };

    if encrypt {
        let recipients = crypto_recipients(outgoing);
        let mut keys = pgp::resolve_recipient_public_keys(pool, app, &recipients)
            .await
            .map_err(|_| "backend.errors.pgpRecipientKeyMissing".to_string())?;

        // Encrypt to self as well. The Sent copy is the ciphertext that went on
        // the wire; without our own key it is an archive we cannot read.
        let own_address = sender_identity(outgoing, fallback_from_address);
        let own_keys = pgp::resolve_recipient_public_keys(pool, app, &[own_address])
            .await
            .map_err(|_| "backend.errors.pgpOwnKeyMissing".to_string())?;
        for key in own_keys {
            if !keys.contains(&key) {
                keys.push(key);
            }
        }

        // Sign-then-encrypt per RFC 3156 §6.2: the signature travels inside the
        // ciphertext, so it is not exposed to anyone who cannot decrypt.
        let plaintext = match signature.as_deref() {
            Some(sig) => crate::email::pgp_mime::signed_entity_for_encryption(&original, sig),
            None => crate::email::pgp_mime::signable_entity(&original),
        };
        let ciphertext = crate::utils::pgp_engine::encrypt_to(&keys, plaintext.as_bytes())
            .map_err(|_| "backend.errors.pgpEncryptFailed".to_string())?;
        outgoing.raw_rfc822_override = Some(crate::email::pgp_mime::build_encrypted(
            &original,
            &ciphertext,
        ));
        return Ok(());
    }

    if let Some(sig) = signature {
        outgoing.raw_rfc822_override = Some(crate::email::pgp_mime::build_signed(&original, &sig));
    }
    Ok(())
}

async fn apply_smime_send_transform(
    pool: &SqlitePool,
    app: &tauri::AppHandle,
    state: &AppState,
    outgoing: &mut OutgoingMessage,
    fallback_from_address: &str,
    fallback_from_name: &str,
) -> SyncResult<()> {
    if outgoing.smime_sign_with_cert_id.is_none() && !outgoing.smime_encrypt_to_recipients {
        return Ok(());
    }

    let original = build_rfc822(
        outgoing,
        Some(fallback_from_address),
        Some(fallback_from_name),
    )?;
    let mut crypto_payload = original.clone().into_bytes();

    if let Some(cert_id) = outgoing.smime_sign_with_cert_id.as_deref() {
        let (key, cert, passphrase) =
            smime::load_private_signing_material(app, state, pool, cert_id).await?;
        crypto_payload =
            crate::utils::smime_engine::sign(&key, &cert, passphrase.as_deref(), &crypto_payload)?;
    }

    if outgoing.smime_encrypt_to_recipients {
        let recipients = crypto_recipients(outgoing);
        let mut certs = smime::resolve_recipient_certs(pool, &recipients)
            .await
            .map_err(smime_recipient_resolution_error)?
            .into_iter()
            .map(|(_, pem)| pem)
            .collect::<Vec<_>>();

        // Encrypt to self as well. The Sent copy is the ciphertext that went on
        // the wire; without our own certificate it is an archive we cannot read.
        let own_address = sender_identity(outgoing, fallback_from_address);
        let own_certs = smime::resolve_recipient_certs(pool, &[own_address])
            .await
            .map_err(own_smime_certificate_error)?;
        for (_, pem) in own_certs {
            if !certs.contains(&pem) {
                certs.push(pem);
            }
        }
        crypto_payload = crate::utils::smime_engine::encrypt(&certs, &crypto_payload)?;
    }

    let crypto = String::from_utf8_lossy(&crypto_payload).to_string();
    outgoing.raw_rfc822_override = Some(merge_delivery_headers(&original, &crypto));
    Ok(())
}

/// How the send endpoint learns who the recipients are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecipientSource {
    /// Gmail `messages.send` and Graph `/me/sendMail` read them from the
    /// headers and strip Bcc before delivery.
    Headers,
    /// SMTP takes them from the envelope. A Bcc header on this path would
    /// reach every To and Cc recipient.
    Envelope,
}

fn recipient_source(provider: &str) -> RecipientSource {
    match provider {
        "gmail" | "outlook" => RecipientSource::Headers,
        _ => RecipientSource::Envelope,
    }
}

/// The crypto transforms build their base message without a `Bcc:` header, so
/// a header-addressed endpoint would drop those recipients without a word —
/// after having encrypted for them. Add the header to the delivery headers
/// only, which keeps it outside the signed or encrypted entity.
fn apply_bcc_delivery_header(outgoing: &mut OutgoingMessage, source: RecipientSource) {
    if source != RecipientSource::Headers || outgoing.bcc.is_empty() {
        return;
    }
    let Some(raw) = outgoing
        .raw_rfc822_override
        .as_deref()
        .filter(|raw| !raw.trim().is_empty())
    else {
        // No crypto override: the provider builds the headers itself and
        // already uses `build_rfc822_with_bcc`.
        return;
    };
    let with_bcc = with_bcc_delivery_header(raw, &outgoing.bcc);
    outgoing.raw_rfc822_override = Some(with_bcc);
}

/// Every address a crypto transform has to encrypt for, in header order.
fn crypto_recipients(outgoing: &OutgoingMessage) -> Vec<String> {
    let mut recipients = Vec::new();
    recipients.extend(outgoing.to.iter().cloned());
    recipients.extend(outgoing.cc.iter().cloned());
    recipients.extend(outgoing.bcc.iter().cloned());
    recipients
}

/// The address this message is sent from, resolved the way `build_rfc822`
/// resolves the `From` header.
fn sender_identity(outgoing: &OutgoingMessage, fallback_from_address: &str) -> String {
    outgoing
        .from_address
        .as_deref()
        .map(str::trim)
        .filter(|address| !address.is_empty())
        .unwrap_or(fallback_from_address)
        .to_string()
}

/// A missing *own* certificate needs its own key: telling the user a recipient
/// is missing when the gap is on their own side sends them to the wrong place.
fn own_smime_certificate_error(error: SmimeRepositoryError) -> SyncError {
    match error {
        SmimeRepositoryError::MissingRecipients { .. } => {
            SyncError::Permanent("backend.errors.smimeOwnCertificateMissing".to_string())
        }
        other => smime_recipient_resolution_error(other),
    }
}

fn smime_recipient_resolution_error(error: SmimeRepositoryError) -> SyncError {
    // A gap or a tie in the certificate store is the user's to resolve;
    // retrying the send would only repeat the same question.
    let is_permanent = matches!(
        &error,
        SmimeRepositoryError::MissingRecipients { .. }
            | SmimeRepositoryError::AmbiguousRecipients { .. }
    );
    let encoded = CommandError::from(error).encoded();
    if is_permanent {
        SyncError::Permanent(encoded)
    } else {
        SyncError::Transient(encoded)
    }
}

fn merge_delivery_headers(original_rfc822: &str, crypto_rfc822: &str) -> String {
    let original_headers = original_rfc822
        .split_once("\r\n\r\n")
        .or_else(|| original_rfc822.split_once("\n\n"))
        .map(|(headers, _)| headers)
        .unwrap_or(original_rfc822);
    let crypto_trimmed = crypto_rfc822.trim_start_matches(['\r', '\n']);
    // Per logical header, not per line: a folded Content-Type would otherwise
    // lose its first line and leave its parameters attached to the header above.
    let delivery_headers = crate::email::build::split_logical_headers(original_headers)
        .into_iter()
        .filter(|header| !crate::email::build::is_content_header(header))
        .collect::<Vec<_>>()
        .join("\r\n");
    format!("{delivery_headers}\r\n{crypto_trimmed}")
}

#[cfg(test)]
mod tests {
    use sqlx::sqlite::SqlitePoolOptions;

    use super::*;

    #[tokio::test]
    async fn missing_smime_recipient_is_structured_and_permanent_through_the_outbox_path() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory db");
        satchel_core::db::init(&pool).await.expect("migrations");

        let error = smime::resolve_recipient_certs(&pool, &["missing@example.test".to_owned()])
            .await
            .expect_err("recipient certificate should be missing");
        let sync_error = smime_recipient_resolution_error(error);

        assert!(sync_error.is_permanent());
        let stored = sync_error.to_string();
        let command_error = CommandError::from(sync_error);
        assert_eq!(
            command_error.key,
            "backend.errors.smimeRecipientCertificateMissing"
        );
        assert_eq!(
            command_error.params.get("recipients"),
            Some(&serde_json::json!("missing@example.test"))
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&stored).expect("structured stored error"),
            serde_json::json!({
                "key": "backend.errors.smimeRecipientCertificateMissing",
                "params": { "recipients": "missing@example.test" }
            })
        );
    }

    fn encrypted_message(plaintext: &str) -> OutgoingMessage {
        OutgoingMessage {
            to: vec!["her@example.test".to_owned()],
            subject: "quarterly figures".to_owned(),
            body: plaintext.to_owned(),
            raw_rfc822_override: Some(
                "To: her@example.test\r\nSubject: quarterly figures\r\n\
                 Content-Type: multipart/encrypted; \
                 protocol=\"application/pgp-encrypted\"; boundary=\"b\"\r\n\r\n\
                 --b--\r\n"
                    .to_owned(),
            ),
            ..Default::default()
        }
    }

    #[test]
    fn an_encrypted_send_files_the_ciphertext_instead_of_a_rebuilt_plaintext() {
        const PLAINTEXT: &str = "salary numbers for Q3";
        let outgoing = encrypted_message(PLAINTEXT);

        let copy = sent_copy_source(&outgoing)
            .resolve(Some(b"unused transport bytes".to_vec()))
            .expect("a Sent copy");
        let copy = String::from_utf8(copy).expect("utf-8 copy");

        assert!(copy.contains("multipart/encrypted"));
        assert!(
            !copy.contains(PLAINTEXT),
            "the plaintext body must never reach the remote Sent folder"
        );
        // Rebuilding is what leaked it: build_rfc822 does not know the override.
        assert!(build_rfc822(&outgoing, Some("me@example.test"), None)
            .unwrap()
            .contains(PLAINTEXT));
    }

    #[test]
    fn the_sent_copy_is_the_transport_wire_form_unless_crypto_prebuilt_one() {
        let mut outgoing = OutgoingMessage {
            to: vec!["her@example.test".to_owned()],
            body: "plain text".to_owned(),
            ..Default::default()
        };
        assert_eq!(sent_copy_source(&outgoing), SentCopy::Transport);
        assert_eq!(
            sent_copy_source(&outgoing).resolve(Some(b"WIRE".to_vec())),
            Some(b"WIRE".to_vec())
        );

        // A blank override is not a wire form.
        outgoing.raw_rfc822_override = Some("   \r\n".to_owned());
        assert_eq!(sent_copy_source(&outgoing), SentCopy::Transport);

        outgoing.raw_rfc822_override = Some("RAW".to_owned());
        assert_eq!(
            sent_copy_source(&outgoing).resolve(Some(b"WIRE".to_vec())),
            Some(b"RAW".to_vec())
        );
    }

    #[test]
    fn the_sender_is_not_among_the_recipients_and_needs_its_own_encrypt_to_self_step() {
        let outgoing = OutgoingMessage {
            to: vec!["her@example.test".to_owned()],
            cc: vec!["team@example.test".to_owned()],
            bcc: vec!["chef@example.test".to_owned()],
            ..Default::default()
        };

        let recipients = crypto_recipients(&outgoing);
        assert_eq!(
            recipients,
            vec!["her@example.test", "team@example.test", "chef@example.test"]
        );
        let sender = sender_identity(&outgoing, "me@example.test");
        assert_eq!(sender, "me@example.test");
        assert!(!recipients.contains(&sender));

        let aliased = OutgoingMessage {
            from_address: Some("  alias@example.test  ".to_owned()),
            ..outgoing
        };
        assert_eq!(
            sender_identity(&aliased, "me@example.test"),
            "alias@example.test"
        );
    }

    #[test]
    fn a_crypto_override_carries_bcc_only_where_the_api_reads_recipients_from_headers() {
        for (provider, expect_header) in [
            ("gmail", true),
            ("outlook", true),
            ("imap", false),
            ("pop3", false),
        ] {
            let mut outgoing = OutgoingMessage {
                to: vec!["her@example.test".to_owned()],
                bcc: vec!["chef@example.test".to_owned()],
                raw_rfc822_override: Some(
                    "From: me@example.test\r\nTo: her@example.test\r\n\r\nsigned".to_owned(),
                ),
                ..Default::default()
            };

            apply_bcc_delivery_header(&mut outgoing, recipient_source(provider));

            let raw = outgoing.raw_rfc822_override.as_deref().expect("override");
            assert_eq!(
                raw.contains("Bcc: chef@example.test"),
                expect_header,
                "{provider}: header-addressed APIs need the Bcc header, raw SMTP must never see it"
            );
            // SMTP still reaches the address: the envelope is built from here.
            assert_eq!(outgoing.bcc, vec!["chef@example.test".to_owned()]);
        }
    }

    #[test]
    fn a_send_without_crypto_leaves_the_bcc_header_to_the_provider() {
        let mut outgoing = OutgoingMessage {
            to: vec!["her@example.test".to_owned()],
            bcc: vec!["chef@example.test".to_owned()],
            ..Default::default()
        };

        apply_bcc_delivery_header(&mut outgoing, RecipientSource::Headers);

        assert_eq!(outgoing.raw_rfc822_override, None);
    }

    /// A `Content-Type` long enough to fold, with the `References` header in
    /// front of it so a stray continuation line has somewhere wrong to land.
    fn folded_content_type_message() -> String {
        concat!(
            "From: me@example.test\r\n",
            "References: <a@example.test>\r\n <b@example.test>\r\n",
            "MIME-Version: 1.0\r\n",
            "Content-Type: multipart/mixed;\r\n",
            " boundary=\"=_satchel_mixed_0123456789abcdef0123456789abcdef\";\r\n",
            " charset=UTF-8\r\n",
            "\r\n",
            "--=_satchel_mixed_0123456789abcdef0123456789abcdef--\r\n",
        )
        .to_owned()
    }

    #[test]
    fn a_folded_content_type_is_dropped_whole_from_the_delivery_headers() {
        let merged = merge_delivery_headers(
            &folded_content_type_message(),
            "Content-Type: application/pkcs7-mime\r\n\r\nciphertext\r\n",
        );
        let (headers, _) = merged.split_once("\r\n\r\n").expect("header/body split");

        assert!(
            !headers.contains("boundary=\"=_satchel_mixed_"),
            "a continuation of the dropped Content-Type survived: {headers:?}"
        );
        assert!(!headers.contains("charset=UTF-8"));
        assert_eq!(
            headers,
            "From: me@example.test\r\nReferences: <a@example.test>\r\n <b@example.test>\r\n\
             Content-Type: application/pkcs7-mime"
        );
    }

    #[test]
    fn a_folded_header_that_is_kept_survives_byte_for_byte() {
        let original = folded_content_type_message();
        let merged = merge_delivery_headers(&original, "Content-Type: text/plain\r\n\r\nbody\r\n");

        assert!(
            merged.contains("References: <a@example.test>\r\n <b@example.test>\r\n"),
            "the folded References header has to reappear unchanged: {merged:?}"
        );
    }

    #[test]
    fn a_missing_own_certificate_is_reported_separately_from_a_missing_recipient() {
        let error = own_smime_certificate_error(SmimeRepositoryError::MissingRecipients {
            recipients: vec!["me@example.test".to_owned()],
        });

        assert!(error.is_permanent());
        assert_eq!(
            CommandError::from(error).key,
            "backend.errors.smimeOwnCertificateMissing"
        );
    }

    #[test]
    fn smime_repository_failure_stays_redacted_and_retryable() {
        let detail = "near SELECT: private certificate detail";
        let sync_error = smime_recipient_resolution_error(SmimeRepositoryError::Persistence(
            sqlx::Error::Protocol(detail.into()),
        ));

        assert!(!sync_error.is_permanent());
        assert!(matches!(&sync_error, SyncError::Transient(_)));
        let stored = sync_error.to_string();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&stored).expect("structured stored error"),
            serde_json::json!({ "key": "backend.errors.smimeCertificatePersistenceFailed" })
        );
        assert!(!stored.contains(detail));
    }
}
