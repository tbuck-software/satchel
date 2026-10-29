//! Durable draft editing and provider synchronization, shared by app and headless callers.
//! Local writes and queue transitions commit together; provider I/O never delays saving.

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, Sqlite, SqlitePool, Transaction};
use std::sync::OnceLock;
use tokio::sync::Mutex;

use self::error::DraftResult;
use self::outgoing::{
    normalize_recipients, recipient_display_names, OutgoingAttachmentInput, OutgoingMessageInput,
};
#[cfg(test)]
use self::send_queue::build_send_action;
use self::send_queue::SendActionPayload;
mod error;
pub mod outgoing;
pub mod send_queue;
use crate::db::messages::SentMessageRepository;
use crate::db::signatures::SignatureRepository;
use crate::providers::{
    provider_for_account,
    traits::{OutgoingAttachment, OutgoingMessage},
};
use crate::sync::queue::{QueuedAction, SyncQueue, ACTION_SYNC_DRAFT};
use crate::sync::remote_mutations::{RemoteMutationPayload, ACTION_REMOTE_MUTATION};
use crate::utils::oauth::{ensure_fresh_account_config, OAuthProvider};
pub use error::DraftError;

static REMOTE_DRAFT_SYNC_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Debug, Serialize, Deserialize)]
struct QueuedDraftSync {
    input: DraftInput,
    revision: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/drafts/"))]
pub struct DraftInput {
    /// Optional id; if provided, an existing draft is updated.
    pub id: Option<String>,
    pub account_id: String,
    #[serde(default)]
    pub thread_id: Option<String>,
    #[serde(default)]
    pub to: Vec<String>,
    #[serde(default)]
    pub cc: Vec<String>,
    #[serde(default)]
    pub bcc: Vec<String>,
    #[serde(default)]
    pub subject: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub body_html: Option<String>,
    #[serde(default)]
    pub from_address: Option<String>,
    #[serde(default)]
    pub from_name: Option<String>,
    #[serde(default)]
    pub reply_to: Option<String>,
    /// RFC3339; for Send Later draft.
    #[serde(default)]
    pub scheduled_for: Option<String>,
    #[serde(default)]
    pub attachments: Vec<OutgoingAttachmentInput>,
    #[serde(default)]
    pub signature_id: Option<String>,
    #[serde(default)]
    pub signature_variant: Option<String>,
    /// Per-message edit of the resolved signature; None keeps the stored one.
    #[serde(default)]
    pub signature_override_text: Option<String>,
    #[serde(default)]
    pub signature_override_html: Option<String>,
    #[serde(default)]
    pub send_options: Option<serde_json::Value>,
    /// The `revision` this editor last saw. A mismatch means another window
    /// changed, sent or discarded the draft, and the save is refused.
    #[serde(default, skip_serializing)]
    pub expected_revision: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/drafts/"))]
pub struct DraftDto {
    pub id: String,
    pub account_id: String,
    pub thread_id: Option<String>,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub subject: String,
    pub body: String,
    pub body_html: Option<String>,
    pub from_address: Option<String>,
    pub from_name: Option<String>,
    pub reply_to: Option<String>,
    pub scheduled_for: Option<String>,
    pub attachments: Vec<OutgoingAttachmentInput>,
    pub signature_id: Option<String>,
    pub signature_variant: Option<String>,
    pub signature_override_text: Option<String>,
    pub signature_override_html: Option<String>,
    pub updated_at: String,
    /// Bumped by every draft save; editors send it back as `expected_revision`.
    #[serde(default)]
    pub revision: i64,
    /// Set when the draft is on disk but the provider copy is not. The draft
    /// itself is valid — this only says the remote half has to wait.
    #[serde(default)]
    pub remote_error: Option<String>,
    #[serde(default)]
    pub send_cancelled: bool,
    #[serde(default)]
    pub send_options: Option<serde_json::Value>,
}

#[derive(Debug, FromRow)]
struct DraftRow {
    id: String,
    account_id: String,
    thread_id: Option<String>,
    to_addresses: Option<String>,
    cc_addresses: Option<String>,
    bcc_addresses: Option<String>,
    subject: Option<String>,
    body_text: Option<String>,
    body_html: Option<String>,
    from_address: Option<String>,
    from_name: Option<String>,
    reply_to: Option<String>,
    scheduled_for: Option<String>,
    draft_attachments_json: Option<String>,
    signature_id: Option<String>,
    signature_variant: Option<String>,
    signature_override_text: Option<String>,
    signature_override_html: Option<String>,
    updated_at: String,
    draft_send_options_json: Option<String>,
    draft_revision: i64,
}

#[derive(Debug, FromRow)]
struct ImportedDraftAttachmentRow {
    id: String,
    filename: Option<String>,
    content_type: String,
    content_id: Option<String>,
    is_inline: i64,
    cached_path: Option<String>,
    remote_part_id: Option<String>,
    provider: String,
    account_id: String,
    account_email: String,
    config: String,
}

impl From<DraftRow> for DraftDto {
    fn from(row: DraftRow) -> Self {
        Self {
            id: row.id,
            account_id: row.account_id,
            thread_id: row.thread_id,
            to: parse_json_array(row.to_addresses.as_deref()),
            cc: parse_json_array(row.cc_addresses.as_deref()),
            bcc: parse_json_array(row.bcc_addresses.as_deref()),
            subject: row.subject.unwrap_or_default(),
            body: row.body_text.unwrap_or_default(),
            body_html: row.body_html,
            from_address: row.from_address,
            from_name: row.from_name,
            reply_to: row.reply_to,
            scheduled_for: row.scheduled_for,
            attachments: row
                .draft_attachments_json
                .as_deref()
                .and_then(|raw| serde_json::from_str(raw).ok())
                .unwrap_or_default(),
            signature_id: row.signature_id,
            signature_variant: row.signature_variant,
            signature_override_text: row.signature_override_text,
            signature_override_html: row.signature_override_html,
            updated_at: row.updated_at,
            revision: row.draft_revision,
            send_options: row
                .draft_send_options_json
                .as_deref()
                .and_then(|raw| serde_json::from_str(raw).ok()),
            remote_error: None,
            send_cancelled: false,
        }
    }
}

fn parse_json_array(raw: Option<&str>) -> Vec<String> {
    raw.and_then(|raw| serde_json::from_str::<Vec<String>>(raw).ok())
        .unwrap_or_default()
}

async fn ensure_drafts_folder(
    connection: &mut sqlx::SqliteConnection,
    account_id: &str,
    now: &str,
) -> Result<String, String> {
    let existing = sqlx::query_scalar::<_, String>(
        "SELECT id FROM folders WHERE account_id = ? AND folder_type = 'drafts' LIMIT 1",
    )
    .bind(account_id)
    .fetch_optional(&mut *connection)
    .await
    .map_err(|err| err.to_string())?;
    if let Some(id) = existing {
        return Ok(id);
    }
    let id = crate::db::folders::resolve_provider_folder_id(&mut *connection, account_id, "Drafts")
        .await
        .map_err(|err| err.to_string())?;
    sqlx::query(
        "INSERT INTO folders (id, account_id, name, full_path, folder_type, remote_id, uidvalidity, message_count, unread_count, sync_enabled, sort_order, created_at, updated_at)
         VALUES (?, ?, 'Drafts', 'Drafts', 'drafts', 'Drafts', NULL, 0, 0, 1, 0, ?, ?)
         ON CONFLICT DO NOTHING",
    )
    .bind(&id)
    .bind(account_id)
    .bind(now)
    .bind(now)
    .execute(&mut *connection)
    .await
    .map_err(|err| err.to_string())?;
    Ok(id)
}

pub async fn save(
    pool: &SqlitePool,
    queue: &SyncQueue,
    input: &DraftInput,
) -> DraftResult<(DraftDto, String)> {
    if let Some(variant) = input.signature_variant.as_deref() {
        if !matches!(variant, "auto" | "long" | "short" | "none") {
            return Err(DraftError::SignatureVariantInvalid);
        }
    }
    if let Some(signature_id) = input.signature_id.as_deref() {
        let owned = SignatureRepository::new(pool)
            .contains_owned(signature_id, &input.account_id)
            .await
            .map_err(DraftError::Signature)?;
        if !owned {
            return Err(DraftError::SignatureNotFound);
        }
    }
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(DraftError::unexpected)?;
    if let (Some(id), Some(expected)) = (input.id.as_deref(), input.expected_revision) {
        ensure_draft_revision(&mut tx, id, expected).await?;
    }
    let cancelled = if let Some(id) = input.id.as_deref() {
        ensure_draft_not_queued(&mut tx, id).await?
    } else {
        false
    };
    let mut result = save_draft_in_transaction(&mut tx, queue, input).await?;
    result.0.send_cancelled = cancelled;
    tx.commit().await.map_err(DraftError::unexpected)?;
    queue.wake_worker();
    Ok(result)
}

async fn save_draft_in_transaction(
    tx: &mut Transaction<'_, Sqlite>,
    queue: &SyncQueue,
    input: &DraftInput,
) -> DraftResult<(DraftDto, String)> {
    let now = chrono::Utc::now().to_rfc3339();
    let id = input
        .id
        .clone()
        .unwrap_or_else(|| format!("draft-{}", uuid::Uuid::new_v4()));
    let folder_id = ensure_drafts_folder(tx, &input.account_id, &now).await?;

    let to_json = serde_json::to_string(&input.to).unwrap_or_else(|_| "[]".into());
    let cc_json = serde_json::to_string(&input.cc).unwrap_or_else(|_| "[]".into());
    let bcc_json = serde_json::to_string(&input.bcc).unwrap_or_else(|_| "[]".into());
    let attachments_json =
        serde_json::to_string(&input.attachments).unwrap_or_else(|_| "[]".into());
    let has_attachments = if input.attachments.is_empty() { 0 } else { 1 };

    let thread_id: Option<String> = if let Some(thread_id) = input.thread_id.as_deref() {
        sqlx::query_scalar("SELECT id FROM threads WHERE id = ? AND account_id = ?")
            .bind(thread_id)
            .bind(&input.account_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(DraftError::unexpected)?
    } else {
        None
    };
    let existing_remote: Option<(
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    )> = sqlx::query_as(
        "SELECT m.account_id, m.remote_draft_id, m.remote_id, f.remote_id, m.thread_id
         FROM messages m JOIN folders f ON f.id = m.folder_id
         WHERE m.id = ? AND m.is_draft = 1",
    )
    .bind(&id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(DraftError::unexpected)?;
    let mut old_thread_to_refresh = None;
    if let Some((old_account, old_draft_id, old_message_id, old_folder, old_thread_id)) =
        existing_remote
    {
        let account_changed = old_account != input.account_id;
        if account_changed && (old_draft_id.is_some() || old_message_id.is_some()) {
            let payload = serde_json::to_value(RemoteMutationPayload::DeleteDraft {
                local_draft_id: Some(id.clone()),
                remote_draft_id: old_draft_id,
                remote_message_id: old_message_id,
                source_folder: old_folder,
            })
            .map_err(DraftError::unexpected)?;
            queue
                .enqueue_in_transaction(
                    tx,
                    &QueuedAction::new(old_account, ACTION_REMOTE_MUTATION, payload),
                )
                .await
                .map_err(DraftError::unexpected)?;
        }
        if account_changed {
            old_thread_to_refresh = old_thread_id;
        }
    }
    cancel_obsolete_draft_deletions(tx, &id, &input.account_id)
        .await
        .map_err(DraftError::unexpected)?;

    sqlx::query(
        "INSERT INTO messages (id, account_id, folder_id, thread_id, message_id_header, in_reply_to, references_header, from_address, from_name, to_addresses, cc_addresses, bcc_addresses, reply_to, subject, date, body_text, body_html, snippet, is_read, is_starred, is_draft, is_sent, has_attachments, remote_uid, remote_id, raw_size, raw_headers, scheduled_for, remote_draft_id, draft_attachments_json, signature_id, signature_variant, signature_override_text, signature_override_html, created_at, updated_at, draft_revision)
         VALUES (?, ?, ?, ?, ?, NULL, NULL, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 1, 0, 1, 0, ?, NULL, NULL, NULL, NULL, ?, ?, ?, ?, ?, ?, ?, ?, ?, 1)
         ON CONFLICT(id) DO UPDATE SET
            account_id = excluded.account_id,
            folder_id = excluded.folder_id,
            thread_id = excluded.thread_id,
            from_address = excluded.from_address,
            from_name = excluded.from_name,
            to_addresses = excluded.to_addresses,
            cc_addresses = excluded.cc_addresses,
            bcc_addresses = excluded.bcc_addresses,
            reply_to = excluded.reply_to,
            subject = excluded.subject,
            body_text = excluded.body_text,
            body_html = excluded.body_html,
            snippet = excluded.snippet,
            has_attachments = excluded.has_attachments,
            scheduled_for = excluded.scheduled_for,
            remote_draft_id = CASE WHEN messages.account_id = excluded.account_id THEN messages.remote_draft_id ELSE NULL END,
            remote_id = CASE WHEN messages.account_id = excluded.account_id THEN messages.remote_id ELSE NULL END,
            remote_uid = CASE WHEN messages.account_id = excluded.account_id THEN messages.remote_uid ELSE NULL END,
            draft_attachments_json = excluded.draft_attachments_json,
            signature_id = excluded.signature_id,
            signature_variant = excluded.signature_variant,
            signature_override_text = excluded.signature_override_text,
            signature_override_html = excluded.signature_override_html,
            draft_revision = messages.draft_revision + 1,
            updated_at = excluded.updated_at",
    )
    .bind(&id)
    .bind(&input.account_id)
    .bind(&folder_id)
    .bind(thread_id.as_deref())
    .bind(format!("<{id}@satchel.draft>"))
    .bind(input.from_address.as_deref())
    .bind(input.from_name.as_deref())
    .bind(&to_json)
    .bind(&cc_json)
    .bind(&bcc_json)
    .bind(input.reply_to.as_deref())
    .bind(&input.subject)
    .bind(&now)
    .bind(&input.body)
    .bind(input.body_html.as_deref())
    .bind(snippet(&input.body, &input.subject))
    .bind(has_attachments)
    .bind(input.scheduled_for.as_deref())
    .bind(Option::<&str>::None)
    .bind(&attachments_json)
    .bind(input.signature_id.as_deref())
    .bind(input.signature_variant.as_deref())
    .bind(input.signature_override_text.as_deref())
    .bind(input.signature_override_html.as_deref())
    .bind(&now)
    .bind(&now)
    .execute(&mut **tx)
    .await
    .map_err(DraftError::unexpected)?;

    sqlx::query("UPDATE messages SET draft_send_options_json = ? WHERE id = ?")
        .bind(
            input
                .send_options
                .as_ref()
                .map(serde_json::Value::to_string),
        )
        .bind(&id)
        .execute(&mut **tx)
        .await
        .map_err(DraftError::unexpected)?;

    let row = sqlx::query_as::<_, DraftRow>(
        "SELECT id, account_id, thread_id, to_addresses, cc_addresses, bcc_addresses, subject, body_text, body_html, from_address, from_name, reply_to, scheduled_for, draft_attachments_json, signature_id, signature_variant, signature_override_text, signature_override_html, updated_at, draft_send_options_json, draft_revision
         FROM messages WHERE id = ?",
    )
    .bind(&id)
    .fetch_one(&mut **tx)
    .await
    .map_err(DraftError::unexpected)?;

    let mut remote_input = input.clone();
    remote_input.id = Some(id.clone());
    remote_input.thread_id = thread_id;
    let payload = serde_json::to_value(QueuedDraftSync {
        input: remote_input,
        revision: now.clone(),
    })
    .map_err(DraftError::unexpected)?;
    // Only the newest not-yet-started upload matters. This also removes an
    // older upload in backoff so it cannot hold the account's queue head.
    sqlx::query(
        "DELETE FROM sync_queue
         WHERE action_type = ? AND status IN ('pending', 'scheduled', 'failed')
           AND json_extract(payload, '$.input.id') = ?",
    )
    .bind(ACTION_SYNC_DRAFT)
    .bind(&id)
    .execute(&mut **tx)
    .await
    .map_err(DraftError::unexpected)?;
    queue
        .enqueue_in_transaction(
            tx,
            &QueuedAction::new(input.account_id.clone(), ACTION_SYNC_DRAFT, payload),
        )
        .await
        .map_err(DraftError::unexpected)?;
    if let Some(thread_id) = old_thread_to_refresh {
        crate::db::threads::refresh_thread_projections_tx(tx, &[thread_id])
            .await
            .map_err(DraftError::from)?;
    }

    Ok((row.into(), now))
}

fn draft_input_for_send(
    account_id: &str,
    input: &OutgoingMessageInput,
    scheduled_for: Option<&str>,
) -> DraftResult<DraftInput> {
    let mut value = serde_json::to_value(input).map_err(DraftError::unexpected)?;
    value["id"] = serde_json::json!(input.draft_id);
    value["account_id"] = serde_json::json!(account_id);
    value["scheduled_for"] = serde_json::json!(scheduled_for);
    // The queue body already includes the resolved signature and inline assets.
    value["signature_variant"] = serde_json::json!("none");
    value["send_options"] = draft_send_options(input);
    serde_json::from_value(value).map_err(DraftError::unexpected)
}

fn draft_send_options(input: &OutgoingMessageInput) -> serde_json::Value {
    serde_json::json!({
        "reply_to_message_id": input.reply_to_message_id, "follow_up_at": input.follow_up_at,
        "smime_sign_with_cert_id": input.smime_sign_with_cert_id,
        "smime_encrypt_to_recipients": input.smime_encrypt_to_recipients,
        "pgp_sign_with_key_id": input.pgp_sign_with_key_id,
        "pgp_encrypt_to_recipients": input.pgp_encrypt_to_recipients, "priority": input.priority,
    })
}

/// Refuses a save from an editor that did not see the latest revision. Runs
/// before `ensure_draft_not_queued`, so a stale editor cannot cancel a failed send.
async fn ensure_draft_revision(
    tx: &mut Transaction<'_, Sqlite>,
    id: &str,
    expected: i64,
) -> DraftResult<()> {
    let current: Option<i64> = sqlx::query_scalar(
        "SELECT m.draft_revision FROM messages m JOIN folders f ON f.id = m.folder_id
         WHERE m.id = ? AND m.is_draft = 1 AND LOWER(TRIM(f.folder_type)) <> 'trash'",
    )
    .bind(id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(DraftError::unexpected)?;
    match current {
        None => Err(DraftError::GoneElsewhere),
        Some(revision) if revision != expected => Err(DraftError::ChangedElsewhere),
        Some(_) => Ok(()),
    }
}

async fn ensure_draft_not_queued(tx: &mut Transaction<'_, Sqlite>, id: &str) -> DraftResult<bool> {
    // Known rejections can return to editing; no worker may claim a failed row.
    let cancelled = sqlx::query("DELETE FROM sync_queue WHERE action_type = 'send_message' AND status = 'failed' AND json_valid(payload) AND json_extract(payload, '$.input.draft_id') = ?")
        .bind(id).execute(&mut **tx).await.map_err(DraftError::unexpected)?.rows_affected() > 0;
    let queued: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sync_queue WHERE action_type = 'send_message'
         AND json_valid(payload) AND json_extract(payload, '$.input.draft_id') = ?)",
    )
    .bind(id)
    .fetch_one(&mut **tx)
    .await
    .map_err(DraftError::unexpected)?;
    if queued {
        return Err(DraftError::SendPending);
    }
    Ok(cancelled)
}

pub async fn restore_queued_draft(
    pool: &SqlitePool,
    queue: &SyncQueue,
    id: &str,
) -> DraftResult<DraftDto> {
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(DraftError::unexpected)?;
    let draft = restore_queued_draft_tx(&mut tx, queue, id).await?;
    tx.commit().await.map_err(DraftError::unexpected)?;
    queue.wake_worker();
    Ok(draft)
}

async fn restore_queued_draft_tx(
    tx: &mut Transaction<'_, Sqlite>,
    queue: &SyncQueue,
    id: &str,
) -> DraftResult<DraftDto> {
    let row: Option<(String, String)> = sqlx::query_as(
        "DELETE FROM sync_queue WHERE id = ? AND action_type = 'send_message'
         AND status IN ('pending', 'scheduled', 'failed') RETURNING account_id, payload",
    )
    .bind(id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(DraftError::unexpected)?;
    let (account_id, payload) = row.ok_or_else(|| DraftError::OutboxActionCannotBeDiscarded)?;
    let payload: SendActionPayload =
        serde_json::from_str(&payload).map_err(DraftError::unexpected)?;
    let input = draft_input_for_send(&account_id, &payload.input, None)?;
    let (draft, _) = save_draft_in_transaction(tx, queue, &input).await?;
    Ok(draft)
}

pub async fn enqueue_send_with_draft(
    pool: &SqlitePool,
    queue: &SyncQueue,
    action: &QueuedAction,
) -> DraftResult<()> {
    let payload: SendActionPayload =
        serde_json::from_value(action.payload.clone()).map_err(DraftError::unexpected)?;
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(DraftError::unexpected)?;
    if let Some(id) = payload.input.draft_id.as_deref() {
        ensure_draft_not_queued(&mut tx, id).await?;
        let input = draft_input_for_send(
            &action.account_id,
            &payload.input,
            payload.scheduled_for.as_deref(),
        )?;
        save_draft_in_transaction(&mut tx, queue, &input).await?;
        // The queued send owns this snapshot; uploading an older draft adds no value.
        sqlx::query("DELETE FROM sync_queue WHERE action_type = 'sync_draft' AND status <> 'sending' AND json_extract(payload, '$.input.id') = ?")
            .bind(id).execute(&mut *tx).await.map_err(DraftError::unexpected)?;
    }
    queue.enqueue_in_transaction(&mut tx, action).await?;
    tx.commit().await.map_err(DraftError::unexpected)?;
    queue.wake_worker();
    Ok(())
}

pub async fn load(
    pool: &SqlitePool,
    read: &SqlitePool,
    queue: &SyncQueue,
    id: &str,
) -> DraftResult<Option<DraftDto>> {
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(DraftError::unexpected)?;
    let queued: Option<String> = sqlx::query_scalar(
        "SELECT id FROM sync_queue WHERE action_type = 'send_message' AND json_valid(payload) AND json_extract(payload, '$.input.draft_id') = ? LIMIT 1",
    ).bind(id).fetch_optional(&mut *tx).await.map_err(DraftError::unexpected)?;
    if let Some(queued) = queued {
        let mut draft = restore_queued_draft_tx(&mut tx, queue, &queued).await?;
        draft.send_cancelled = true;
        tx.commit().await.map_err(DraftError::unexpected)?;
        queue.wake_worker();
        return Ok(Some(draft));
    }
    tx.commit().await.map_err(DraftError::unexpected)?;
    load_draft_inner(read, pool, id)
        .await
        .map_err(DraftError::from)
}

async fn cancel_obsolete_draft_deletions(
    tx: &mut Transaction<'_, Sqlite>,
    draft_id: &str,
    account_id: &str,
) -> Result<u64, String> {
    // A cleanup from an earlier account switch must not delete the active
    // account's draft after a later switch back. SQL write serialization makes
    // this atomic with a worker claim or a user's retry of a failed action.
    let removed = sqlx::query(
        "DELETE FROM sync_queue
         WHERE account_id = ? AND action_type = ?
           AND status IN ('pending', 'scheduled', 'failed')
           AND json_valid(payload)
           AND json_extract(payload, '$.kind') = 'delete_draft'
           AND json_extract(payload, '$.local_draft_id') = ?",
    )
    .bind(account_id)
    .bind(ACTION_REMOTE_MUTATION)
    .bind(draft_id)
    .execute(&mut **tx)
    .await
    .map_err(|error| error.to_string())?
    .rows_affected();
    Ok(removed)
}

pub async fn execute_queued_draft_sync(
    pool: &SqlitePool,
    queue: &SyncQueue,
    action: &QueuedAction,
) -> crate::sync::errors::SyncResult<()> {
    let payload: QueuedDraftSync =
        serde_json::from_value(action.payload.clone()).map_err(|error| {
            tracing::warn!(action_id = %action.id, %error, "invalid queued draft payload");
            crate::sync::errors::SyncError::Permanent("backend.errors.draftSyncQueueInvalid".into())
        })?;
    let id = payload.input.id.clone().ok_or_else(|| {
        crate::sync::errors::SyncError::Permanent("backend.errors.draftSyncQueueInvalid".into())
    })?;
    sync_draft_remotely(pool, queue, payload.input, id, payload.revision)
        .await
        .map_err(Into::into)
}

async fn sync_draft_remotely(
    pool: &SqlitePool,
    queue: &SyncQueue,
    input: DraftInput,
    id: String,
    revision: String,
) -> Result<(), String> {
    // Provider calls are serialized, but this lock never delays a local write.
    // A newer local revision supersedes a queued upload before it starts.
    let _remote_guard = REMOTE_DRAFT_SYNC_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .await;
    let current: Option<(String, String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT account_id, updated_at, remote_draft_id, remote_id
         FROM messages WHERE id = ? AND is_draft = 1",
    )
    .bind(&id)
    .fetch_optional(pool)
    .await
    .map_err(|error| error.to_string())?;
    let Some((account_id, updated_at, existing_remote_draft_id, existing_remote_message_id)) =
        current
    else {
        return Ok(());
    };
    if account_id != input.account_id || updated_at != revision {
        return Ok(());
    }
    // A delete that was already sending during A -> B -> A can fail after the
    // switch-back transaction. This account's queue runs one action at a time,
    // so the old delete has settled before this upload is dispatched.
    let mut tx = pool.begin().await.map_err(|error| error.to_string())?;
    cancel_obsolete_draft_deletions(&mut tx, &id, &input.account_id).await?;
    tx.commit().await.map_err(|error| error.to_string())?;

    let remote_result = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        push_draft_to_remote(
            pool,
            &input,
            &id,
            existing_remote_draft_id.as_deref(),
            existing_remote_message_id.as_deref(),
        ),
    )
    .await;
    match remote_result {
        Ok(Ok(Some(remote_id))) => {
            record_remote_draft_id(
                pool,
                queue,
                &id,
                &input.account_id,
                remote_id,
                existing_remote_message_id.as_deref(),
            )
            .await?;
        }
        Ok(Ok(None)) => {}
        Ok(Err(err)) => return Err(err),
        Err(_) => return Err("backend.errors.draftSyncTimedOut".into()),
    }
    Ok(())
}

async fn record_remote_draft_id(
    pool: &SqlitePool,
    queue: &SyncQueue,
    id: &str,
    account_id: &str,
    remote_id: String,
    previous_message_id: Option<&str>,
) -> Result<(), String> {
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|err| err.to_string())?;
    // A -> B -> A can happen while A's earlier upload is in flight. Keep the
    // queued deletion ahead of the new A upload; publishing that doomed id on
    // the current row would make the new upload update a draft about to vanish.
    let pending_deletion: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sync_queue
         WHERE account_id = ? AND action_type = ?
           AND status IN ('pending', 'scheduled', 'sending', 'failed')
           AND json_extract(payload, '$.kind') = 'delete_draft'
           AND (json_extract(payload, '$.remote_draft_id') = ?
                OR json_extract(payload, '$.remote_message_id') = ?)",
    )
    .bind(account_id)
    .bind(ACTION_REMOTE_MUTATION)
    .bind(&remote_id)
    .bind(previous_message_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|err| err.to_string())?;
    if pending_deletion > 0 {
        tx.commit().await.map_err(|err| err.to_string())?;
        return Ok(());
    }
    let updated = sqlx::query(
        "UPDATE messages SET remote_draft_id = ? WHERE id = ? AND account_id = ? AND is_draft = 1",
    )
    .bind(&remote_id)
    .bind(id)
    .bind(account_id)
    .execute(&mut *tx)
    .await
    .map_err(|err| err.to_string())?;
    if updated.rows_affected() == 0 {
        let payload = serde_json::to_value(RemoteMutationPayload::DeleteDraft {
            local_draft_id: Some(id.to_string()),
            remote_draft_id: Some(remote_id),
            remote_message_id: None,
            source_folder: None,
        })
        .map_err(|err| err.to_string())?;
        queue
            .enqueue_in_transaction(
                &mut tx,
                &QueuedAction::new(account_id.to_string(), ACTION_REMOTE_MUTATION, payload),
            )
            .await?;
    }
    tx.commit().await.map_err(|err| err.to_string())?;
    if updated.rows_affected() == 0 {
        queue.wake_worker();
    }
    Ok(())
}

pub async fn list(pool: &SqlitePool) -> Result<Vec<DraftDto>, String> {
    let rows = sqlx::query_as::<_, DraftRow>(
        "SELECT m.id, m.account_id, m.thread_id, m.to_addresses, m.cc_addresses, m.bcc_addresses, m.subject, m.body_text, m.body_html, m.from_address, m.from_name, m.reply_to, m.scheduled_for, m.draft_attachments_json, m.signature_id, m.signature_variant, m.signature_override_text, m.signature_override_html, m.updated_at, m.draft_send_options_json, m.draft_revision
         FROM messages m
         JOIN folders f ON f.id = m.folder_id
         WHERE m.is_draft = 1 AND LOWER(TRIM(f.folder_type)) <> 'trash'
         ORDER BY m.updated_at DESC",
    )
    .fetch_all(pool)
    .await
    .map_err(|error| error.to_string())?;
    let mut drafts = Vec::with_capacity(rows.len());
    for row in rows {
        drafts.push(sanitize_draft_html(row.into()).await?);
    }
    Ok(drafts)
}

async fn sanitize_draft_html(mut draft: DraftDto) -> Result<DraftDto, String> {
    if let Some(html) = draft.body_html.take() {
        draft.body_html = Some(
            crate::email::processing::sanitize_message_html(draft.id.clone(), html)
                .await
                .map_err(|error| error.to_string())?,
        );
    }
    Ok(draft)
}

async fn load_draft_inner(
    read: &SqlitePool,
    write: &SqlitePool,
    id: &str,
) -> Result<Option<DraftDto>, String> {
    let row = sqlx::query_as::<_, DraftRow>(
        "SELECT m.id, m.account_id, m.thread_id, m.to_addresses, m.cc_addresses, m.bcc_addresses, m.subject, m.body_text, m.body_html, m.from_address, m.from_name, m.reply_to, m.scheduled_for, m.draft_attachments_json, m.signature_id, m.signature_variant, m.signature_override_text, m.signature_override_html, m.updated_at, m.draft_send_options_json, m.draft_revision
         FROM messages m
         JOIN folders f ON f.id = m.folder_id
         WHERE m.id = ? AND m.is_draft = 1 AND LOWER(TRIM(f.folder_type)) <> 'trash'",
    )
    .bind(id)
    .fetch_optional(read)
    .await
    .map_err(|err| err.to_string())?;
    let Some(row) = row else { return Ok(None) };
    let had_local_attachment_payload = row
        .draft_attachments_json
        .as_deref()
        .is_some_and(|raw| !raw.trim().is_empty());
    let mut draft: DraftDto = row.into();
    if !had_local_attachment_payload {
        hydrate_imported_draft_attachments(read, write, id, &mut draft).await?;
    }
    Ok(Some(sanitize_draft_html(draft).await?))
}

async fn hydrate_imported_draft_attachments(
    read: &SqlitePool,
    write: &SqlitePool,
    draft_id: &str,
    draft: &mut DraftDto,
) -> Result<(), String> {
    let reference = crate::db::remote_content::ContentReference::load(read, draft_id).await?;
    let rows = sqlx::query_as::<_, ImportedDraftAttachmentRow>(
        "SELECT a.id, a.filename, a.content_type, a.content_id, a.is_inline, a.cached_path,
                a.remote_part_id, acc.provider, acc.id AS account_id, acc.email AS account_email,
                acc.config
         FROM attachments a
         JOIN messages m ON m.id = a.message_id
         JOIN accounts acc ON acc.id = m.account_id
         JOIN folders f ON f.id = m.folder_id
         WHERE m.id = ? AND m.is_draft = 1
         ORDER BY a.id",
    )
    .bind(draft_id)
    .fetch_all(read)
    .await
    .map_err(|err| err.to_string())?;
    if rows.is_empty() {
        return Ok(());
    }
    reference.validate_cached_read()?;

    let mut attachments = Vec::with_capacity(rows.len());
    for row in rows {
        let cached = if let Some(path) = row.cached_path.as_deref() {
            if reference.owns_attachment_cache(path, &row.id) {
                tokio::fs::read(path).await.ok()
            } else {
                None
            }
        } else {
            None
        };
        let (bytes, fetched_filename, fetched_type) = if let Some(bytes) = cached {
            (bytes, None, None)
        } else {
            let message = reference.message_ref()?;
            let remote_part_id = row.remote_part_id.as_deref().ok_or_else(|| {
                format!(
                    "draft attachment {} has no remote part id",
                    row.filename.as_deref().unwrap_or("(unnamed)")
                )
            })?;
            let config = crate::accounts::decrypt_account_config(&row.account_id, &row.config)?;
            let provider = provider_for_account(&row.provider, &config, &row.account_email, None)?;
            let fetched = tokio::time::timeout(
                std::time::Duration::from_secs(60),
                provider.fetch_attachment(&message, remote_part_id),
            )
            .await
            .map_err(|_| "draft attachment fetch timed out".to_string())?
            .map_err(|error| error.to_string())?;
            (fetched.bytes, fetched.filename, Some(fetched.content_type))
        };
        let mime_type = fetched_type
            .filter(|value| {
                !value.trim().is_empty()
                    && (!value.eq_ignore_ascii_case("application/octet-stream")
                        || row
                            .content_type
                            .eq_ignore_ascii_case("application/octet-stream"))
            })
            .unwrap_or(row.content_type);
        attachments.push(OutgoingAttachmentInput {
            filename: crate::utils::filename::strip_filename_controls(
                &fetched_filename
                    .or(row.filename)
                    .unwrap_or_else(|| "attachment".to_string()),
            ),
            mime_type,
            content_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
            content_id: row.content_id,
            inline: row.is_inline > 0,
        });
    }

    let mut tx = reference.begin_write(write).await?;
    let serialized = serde_json::to_string(&attachments).map_err(|err| err.to_string())?;
    sqlx::query(
        "UPDATE messages SET draft_attachments_json = ?, updated_at = ?
         WHERE id = ? AND is_draft = 1",
    )
    .bind(serialized)
    .bind(chrono::Utc::now().to_rfc3339())
    .bind(draft_id)
    .execute(&mut *tx)
    .await
    .map_err(|err| err.to_string())?;
    tx.commit().await.map_err(|err| err.to_string())?;
    draft.attachments = attachments;
    Ok(())
}

pub async fn delete_after_send(
    pool: &SqlitePool,
    queue: &SyncQueue,
    id: &str,
) -> Result<(), String> {
    delete_draft_with_queue(pool, queue, id, false)
        .await
        .map_err(|error| error.encoded())
}

pub async fn discard(pool: &SqlitePool, queue: &SyncQueue, id: &str) -> DraftResult<()> {
    delete_draft_with_queue(pool, queue, id, true).await
}

async fn delete_draft_with_queue(
    pool: &SqlitePool,
    queue: &SyncQueue,
    id: &str,
    cancel_send: bool,
) -> DraftResult<()> {
    // Lock before reading the identity: a concurrent save may have just created
    // or replaced the remote draft and must publish that id before deletion.
    let _remote_guard = REMOTE_DRAFT_SYNC_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .await;
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(DraftError::unexpected)?;
    let queued: Vec<String> = sqlx::query_scalar("SELECT id FROM sync_queue WHERE action_type = 'send_message' AND json_valid(payload) AND json_extract(payload, '$.input.draft_id') = ?")
        .bind(id).fetch_all(&mut *tx).await.map_err(DraftError::unexpected)?;
    for queued_id in queued.into_iter().filter(|_| cancel_send) {
        restore_queued_draft_tx(&mut tx, queue, &queued_id).await?;
    }
    let remote: Option<(String, Option<String>, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT m.account_id, m.remote_draft_id, m.remote_id, f.remote_id
         FROM messages m
         JOIN folders f ON f.id = m.folder_id
         WHERE m.id = ? AND m.is_draft = 1",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|err| err.to_string())?;

    // The provider copy is queued, never awaited here. Deleting it inline made
    // the whole discard fail whenever the provider was rate limited — with the
    // draft still in the conversation and an error the user cannot act on. The
    // queue owns the retry; the local row goes now.
    if let Some((account_id, stored_draft_id, message_remote_id, folder_remote_id)) = remote {
        if stored_draft_id.is_some() || message_remote_id.is_some() {
            let payload = serde_json::to_value(RemoteMutationPayload::DeleteDraft {
                local_draft_id: Some(id.into()),
                remote_draft_id: stored_draft_id,
                remote_message_id: message_remote_id,
                source_folder: folder_remote_id,
            })
            .map_err(DraftError::unexpected)?;
            queue
                .enqueue_in_transaction(
                    &mut tx,
                    &QueuedAction::new(account_id, ACTION_REMOTE_MUTATION, payload),
                )
                .await?;
        }
    }
    // Read the conversation before the row is gone: a draft written inside a
    // thread leaves message count, snippet and date behind, and the thread
    // itself has to disappear when the draft was all it held.
    let thread_id: Option<String> =
        sqlx::query_scalar("SELECT thread_id FROM messages WHERE id = ? AND is_draft = 1")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|err| err.to_string())?
            .flatten();

    // attachments.message_id has no ON DELETE clause — the rows must go first
    // or the message delete fails with a foreign key error.
    sqlx::query(
        "DELETE FROM attachments
         WHERE message_id IN (SELECT id FROM messages WHERE id = ? AND is_draft = 1)",
    )
    .bind(id)
    .execute(&mut *tx)
    .await
    .map_err(|err| err.to_string())?;

    sqlx::query("DELETE FROM messages WHERE id = ? AND is_draft = 1")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(|err| err.to_string())?;

    sqlx::query("DELETE FROM sync_queue WHERE action_type = 'sync_draft' AND status <> 'sending' AND json_extract(payload, '$.input.id') = ?")
        .bind(id).execute(&mut *tx).await.map_err(DraftError::unexpected)?;
    if let Some(thread_id) = thread_id {
        crate::db::threads::refresh_thread_projections_tx(&mut tx, &[thread_id])
            .await
            .map_err(DraftError::from)?;
    }
    tx.commit().await.map_err(DraftError::unexpected)?;
    queue.wake_worker();
    Ok(())
}

async fn push_draft_to_remote(
    pool: &SqlitePool,
    input: &DraftInput,
    local_draft_id: &str,
    remote_draft_id: Option<&str>,
    remote_message_id: Option<&str>,
) -> Result<Option<String>, String> {
    let row: Option<(String, String, String, String, String, String)> = sqlx::query_as(
        "SELECT id, display_name, provider, email, config,
                COALESCE((
                  SELECT COALESCE(NULLIF(TRIM(remote_id), ''), full_path)
                  FROM folders
                  WHERE account_id = accounts.id AND folder_type = 'drafts'
                  ORDER BY updated_at DESC LIMIT 1
                ), 'Drafts')
         FROM accounts WHERE id = ? LIMIT 1",
    )
    .bind(&input.account_id)
    .fetch_optional(pool)
    .await
    .map_err(|err| err.to_string())?;
    let Some((account_id, display_name, provider_kind, email, stored_config, drafts_folder)) = row
    else {
        return Ok(None);
    };
    let mut config = crate::accounts::decrypt_account_config(&account_id, &stored_config)?;

    if let Ok(oauth_provider) = OAuthProvider::parse(&provider_kind) {
        if let Some(updated) = ensure_fresh_account_config(oauth_provider, &config).await? {
            let now = chrono::Utc::now().to_rfc3339();
            sqlx::query(
                "UPDATE accounts
                 SET config = ?, updated_at = ?, last_oauth_error = NULL, last_oauth_error_at = NULL
                 WHERE id = ?",
            )
            .bind(crate::accounts::encrypt_account_config(
                &account_id,
                &updated,
            )?)
            .bind(&now)
            .bind(&input.account_id)
            .execute(pool)
            .await
            .map_err(|err| err.to_string())?;
            config = crate::accounts::AccountConfig::from_plaintext(updated);
        }
    }

    let provider = provider_for_account(&provider_kind, &config, &email, None)?;
    let resolved_remote_draft_id = if remote_draft_id.is_none() {
        if let Some(message_remote_id) = remote_message_id {
            provider
                .resolve_draft_id(message_remote_id, Some(&drafts_folder))
                .await
                .map_err(|error| error.to_string())?
        } else {
            None
        }
    } else {
        remote_draft_id.map(str::to_string)
    };

    let attachments = input
        .attachments
        .iter()
        .map(|att| OutgoingAttachment {
            filename: att.filename.clone(),
            mime_type: att.mime_type.clone(),
            content_base64: att.content_base64.clone(),
            content_id: att.content_id.clone(),
            inline: att.inline,
        })
        .collect::<Vec<_>>();
    // Same treatment the send path gives them. A draft uploaded with a raw chip
    // ("Morgan, Alex <t@x.de>") carries an invalid recipient list, and whoever
    // opens it in webmail sends to that list.
    let normalize =
        |values: &[String]| normalize_recipients(values).map_err(|error| error.encoded());

    // A draft reply that carries no threading headers starts a new thread for
    // the recipient when it is sent from the provider's own web interface.
    let reply_context = match input.thread_id.as_deref() {
        Some(thread_id) => Some(
            SentMessageRepository::new(pool)
                .load_reply_context(&input.account_id, thread_id, None)
                .await
                .map_err(|error| DraftError::from(error).encoded())?,
        ),
        None => None,
    };

    let outgoing = OutgoingMessage {
        from_address: input.from_address.clone(),
        from_name: input.from_name.clone().or(Some(display_name)),
        reply_to: input.reply_to.clone(),
        to: normalize(&input.to)?,
        cc: normalize(&input.cc)?,
        bcc: normalize(&input.bcc)?,
        recipient_names: recipient_display_names(&[&input.to, &input.cc, &input.bcc]),
        subject: input.subject.clone(),
        body: input.body.clone(),
        body_html: input.body_html.clone(),
        in_reply_to: reply_context.as_ref().and_then(|c| c.in_reply_to.clone()),
        references: reply_context
            .as_ref()
            .map(|c| c.references.clone())
            .unwrap_or_default(),
        attachments,
        message_id_header: Some(format!("<{local_draft_id}@satchel.draft>")),
        request_read_receipt_to: None,
        raw_rfc822_override: None,
        smime_sign_with_cert_id: None,
        smime_encrypt_to_recipients: false,
        // Drafts are stored as plaintext; crypto is applied on send.
        pgp_sign_with_key_id: None,
        pgp_encrypt_to_recipients: false,
        priority: None,
    };
    provider
        .upsert_draft(
            outgoing,
            resolved_remote_draft_id.as_deref(),
            Some(&drafts_folder),
        )
        .await
        .map_err(|error| error.to_string())
}

fn snippet(body: &str, fallback_subject: &str) -> String {
    let normalized = body
        .replace("\r\n", "\n")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if normalized.is_empty() {
        return fallback_subject.to_string();
    }
    const MAX: usize = 180;
    let mut out = normalized.chars().take(MAX).collect::<String>();
    if normalized.chars().count() > MAX {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{
        cancel_obsolete_draft_deletions, delete_after_send, discard, execute_queued_draft_sync,
        load_draft_inner, record_remote_draft_id, save, DraftInput, ACTION_SYNC_DRAFT,
        REMOTE_DRAFT_SYNC_LOCK,
    };
    use sqlx::sqlite::SqlitePoolOptions;
    use tokio::sync::Mutex;

    fn draft_input(account_id: &str, body: &str) -> DraftInput {
        DraftInput {
            id: Some("draft".into()),
            account_id: account_id.into(),
            thread_id: None,
            to: vec![],
            cc: vec![],
            bcc: vec![],
            subject: "Subject".into(),
            body: body.into(),
            body_html: None,
            from_address: Some(format!("{account_id}@example.test")),
            from_name: None,
            reply_to: None,
            scheduled_for: None,
            attachments: vec![],
            signature_id: None,
            signature_variant: None,
            signature_override_text: None,
            signature_override_html: None,
            send_options: None,
            expected_revision: None,
        }
    }

    async fn draft_test_pool() -> sqlx::SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("pool");
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .expect("migrations");
        sqlx::raw_sql(
            "INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
             VALUES ('A', 'A', 'a@example.test', 'imap', '{}', '', '2026-01-01', '2026-01-01'),
                    ('B', 'B', 'b@example.test', 'imap', '{}', '', '2026-01-01', '2026-01-01');",
        ).execute(&pool).await.expect("accounts");
        pool
    }

    #[tokio::test]
    async fn draft_fallback_preserves_an_exact_remote_folder_with_a_legacy_id() {
        let pool = draft_test_pool().await;
        sqlx::query("INSERT INTO folders (id, account_id, name, full_path, remote_id, folder_type, created_at, updated_at) VALUES ('drafts-legacy', 'A', 'Drafts', 'Drafts', 'Drafts', 'custom', '', '')").execute(&pool).await.unwrap();
        let (draft, _) = save(
            &pool,
            &crate::sync::queue::SyncQueue::new(),
            &draft_input("A", "Body"),
        )
        .await
        .unwrap();
        let folder: String = sqlx::query_scalar("SELECT folder_id FROM messages WHERE id = ?")
            .bind(&draft.id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(folder, "drafts-legacy");
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM folders WHERE account_id = 'A'")
                .fetch_one(&pool)
                .await
                .unwrap(),
            1
        );
    }

    #[tokio::test]
    async fn draft_reads_sanitize_legacy_html_before_the_repair_finishes() {
        let pool = draft_test_pool().await;
        let queue = crate::sync::queue::SyncQueue::new();
        let (draft, _) = save(&pool, &queue, &draft_input("A", "Body"))
            .await
            .unwrap();
        sqlx::query("UPDATE messages SET body_html = '<p onclick=bad()>Hi<script>bad()</script></p>' WHERE id = ?")
            .bind(&draft.id).execute(&pool).await.unwrap();
        let listed = super::list(&pool).await.unwrap();
        assert_eq!(listed[0].body_html.as_deref(), Some("<p>Hi</p>"));
        let loaded = load_draft_inner(&pool, &pool, &draft.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(loaded.body_html.as_deref(), Some("<p>Hi</p>"));
    }

    #[tokio::test]
    async fn cached_draft_load_does_not_wait_for_the_write_pool() {
        let dir = tempfile::tempdir().unwrap();
        let pools = crate::db::connect_pools(dir.path().join("satchel.db"))
            .await
            .unwrap();
        sqlx::query("INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, created_at, updated_at) VALUES ('A', 'A', 'a@example.test', 'imap', '{}', '', '2026-01-01', '2026-01-01')").execute(&pools.write).await.unwrap();
        save(
            &pools.write,
            &crate::sync::queue::SyncQueue::new(),
            &draft_input("A", "Cached body"),
        )
        .await
        .unwrap();
        let first = pools.write.acquire().await.unwrap();
        let second = pools.write.acquire().await.unwrap();
        let draft = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            load_draft_inner(pools.read.as_ref(), &pools.write, "draft"),
        )
        .await
        .expect("cached draft uses read pool")
        .unwrap()
        .unwrap();
        assert_eq!(draft.body, "Cached body");
        drop((first, second));
    }

    #[tokio::test]
    async fn local_save_completes_while_remote_sync_is_blocked() {
        let pool = draft_test_pool().await;
        let queue = crate::sync::queue::SyncQueue::new();
        let lock = REMOTE_DRAFT_SYNC_LOCK.get_or_init(|| Mutex::new(()));
        let _blocked_remote = lock.lock().await;

        let (first, _) = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            save(&pool, &queue, &draft_input("A", "first revision")),
        )
        .await
        .expect("local save must not wait for remote")
        .expect("first save");
        let first_action: String =
            sqlx::query_scalar("SELECT id FROM sync_queue WHERE action_type = ?")
                .bind(ACTION_SYNC_DRAFT)
                .fetch_one(&pool)
                .await
                .expect("first queued upload");
        sqlx::query("UPDATE sync_queue SET status = 'sending' WHERE id = ?")
            .bind(&first_action)
            .execute(&pool)
            .await
            .expect("start first upload");
        let (latest, _) = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            save(&pool, &queue, &draft_input("A", "latest revision")),
        )
        .await
        .expect("latest local revision must not wait for remote")
        .expect("latest save");

        assert_eq!(first.id, latest.id);
        assert_eq!(latest.body, "latest revision");
        let persisted: String =
            sqlx::query_scalar("SELECT body_text FROM messages WHERE id = 'draft'")
                .fetch_one(&pool)
                .await
                .expect("persisted body");
        assert_eq!(persisted, "latest revision");

        // The latest upload survives a worker restart, even when the old one
        // was already claimed before the process stopped.
        let actions: Vec<(String, String)> = sqlx::query_as(
            "SELECT id, status FROM sync_queue WHERE action_type = ? ORDER BY rowid",
        )
        .bind(ACTION_SYNC_DRAFT)
        .fetch_all(&pool)
        .await
        .expect("queued uploads");
        assert_eq!(actions.len(), 2);
        assert_eq!(
            queue.reset_sending(&pool).await.expect("startup recovery"),
            1
        );
        let status: String = sqlx::query_scalar("SELECT status FROM sync_queue WHERE id = ?")
            .bind(&actions[0].0)
            .fetch_one(&pool)
            .await
            .expect("recovered upload");
        assert_eq!(status, "pending");
        drop(_blocked_remote);
        let old_action = queue
            .claim_next(&pool)
            .await
            .expect("claim")
            .expect("old upload");
        assert_eq!(old_action.id, actions[0].0);
        execute_queued_draft_sync(&pool, &queue, &old_action)
            .await
            .expect("superseded upload skips the provider");
    }

    #[tokio::test]
    async fn a_save_from_a_stale_editor_is_refused() {
        let pool = draft_test_pool().await;
        let queue = crate::sync::queue::SyncQueue::new();
        let (first, _) = save(&pool, &queue, &draft_input("A", "first"))
            .await
            .expect("first save");
        assert_eq!(first.revision, 1);

        let mut window_a = draft_input("A", "edit from window A");
        window_a.expected_revision = Some(first.revision);
        let (second, _) = save(&pool, &queue, &window_a)
            .await
            .expect("current editor saves");
        assert_eq!(second.revision, 2);

        // Thread-wide updates touch updated_at; they must not read as an edit.
        sqlx::query("UPDATE messages SET is_read = 1, updated_at = 'later' WHERE id = 'draft'")
            .execute(&pool)
            .await
            .expect("thread-wide update");
        let mut window_a_again = draft_input("A", "still window A");
        window_a_again.expected_revision = Some(second.revision);
        let (third, _) = save(&pool, &queue, &window_a_again)
            .await
            .expect("unrelated bump is not a conflict");

        let mut window_b = draft_input("A", "stale window B");
        window_b.expected_revision = Some(first.revision);
        let error = save(&pool, &queue, &window_b)
            .await
            .expect_err("stale editor is refused");
        assert_eq!(error.message_key(), "backend.errors.draftChangedElsewhere");
        let body: String = sqlx::query_scalar("SELECT body_text FROM messages WHERE id = 'draft'")
            .fetch_one(&pool)
            .await
            .expect("body");
        assert_eq!(body, "still window A");

        discard(&pool, &queue, "draft")
            .await
            .expect("sent or discarded elsewhere");
        let mut after_delete = draft_input("A", "would resurrect");
        after_delete.expected_revision = Some(third.revision);
        let error = save(&pool, &queue, &after_delete)
            .await
            .expect_err("a vanished draft is not recreated");
        assert_eq!(error.message_key(), "backend.errors.draftGoneElsewhere");
    }

    #[tokio::test]
    async fn account_change_moves_local_draft_and_queues_old_remote_identity() {
        let pool = draft_test_pool().await;
        let queue = crate::sync::queue::SyncQueue::new();
        save(&pool, &queue, &draft_input("A", "body"))
            .await
            .expect("draft under A");
        sqlx::query("UPDATE messages SET remote_draft_id = 'remote-A', remote_id = 'message-A', remote_uid = '9' WHERE id = 'draft'")
            .execute(&pool).await.expect("remote identity");

        let (draft, _) = save(&pool, &queue, &draft_input("B", "body for B"))
            .await
            .expect("move to B");
        assert_eq!(draft.id, "draft");
        assert_eq!(draft.account_id, "B");
        assert_eq!(draft.body, "body for B");
        let identity: (String, String, Option<String>, Option<String>, Option<String>) =
            sqlx::query_as("SELECT m.account_id, f.account_id, m.remote_draft_id, m.remote_id, m.remote_uid FROM messages m JOIN folders f ON f.id = m.folder_id WHERE m.id = 'draft'")
                .fetch_one(&pool).await.expect("moved identity");
        assert_eq!(identity, ("B".into(), "B".into(), None, None, None));
        let (queue_account, payload): (String, String) = sqlx::query_as(
            "SELECT account_id, payload FROM sync_queue WHERE action_type = 'remote_mutation'",
        )
        .fetch_one(&pool)
        .await
        .expect("old remote cleanup");
        assert_eq!(queue_account, "A");
        assert!(payload.contains("remote-A"), "{payload}");
        assert!(payload.contains("message-A"), "{payload}");
    }

    #[tokio::test]
    async fn remote_save_finishing_after_account_change_cleans_up_its_old_copy() {
        let pool = draft_test_pool().await;
        let queue = crate::sync::queue::SyncQueue::new();
        save(&pool, &queue, &draft_input("A", "body"))
            .await
            .expect("draft under A");
        save(&pool, &queue, &draft_input("B", "body for B"))
            .await
            .expect("move to B while A upload is in flight");

        record_remote_draft_id(&pool, &queue, "draft", "A", "late-remote-A".into(), None)
            .await
            .expect("record late upload");

        let current: (String, Option<String>) =
            sqlx::query_as("SELECT account_id, remote_draft_id FROM messages WHERE id = 'draft'")
                .fetch_one(&pool)
                .await
                .expect("current draft");
        assert_eq!(current, ("B".into(), None));
        let (account_id, payload): (String, String) = sqlx::query_as(
            "SELECT account_id, payload FROM sync_queue WHERE action_type = 'remote_mutation'",
        )
        .fetch_one(&pool)
        .await
        .expect("cleanup for in-flight upload");
        assert_eq!(account_id, "A");
        assert!(payload.contains("late-remote-A"), "{payload}");
    }

    #[tokio::test]
    async fn switching_back_cancels_a_failed_old_remote_delete() {
        let pool = draft_test_pool().await;
        let queue = crate::sync::queue::SyncQueue::new();
        save(&pool, &queue, &draft_input("A", "first A"))
            .await
            .expect("first A save");
        sqlx::query("DELETE FROM sync_queue WHERE action_type = ?")
            .bind(ACTION_SYNC_DRAFT)
            .execute(&pool)
            .await
            .expect("first A upload completed");
        sqlx::query("UPDATE messages SET remote_draft_id = 'remote-A' WHERE id = 'draft'")
            .execute(&pool)
            .await
            .expect("remote A identity");
        save(&pool, &queue, &draft_input("B", "B revision"))
            .await
            .expect("switch to B");
        sqlx::query("UPDATE sync_queue SET status = 'failed' WHERE account_id = 'A' AND action_type = 'remote_mutation'")
            .execute(&pool)
            .await
            .expect("old A delete failed");
        save(&pool, &queue, &draft_input("A", "latest A"))
            .await
            .expect("switch back to A");

        let failed_cleanup: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sync_queue WHERE account_id = 'A' AND action_type = 'remote_mutation'",
        )
        .fetch_one(&pool)
        .await
        .expect("obsolete cleanup count");
        assert_eq!(failed_cleanup, 0);
        assert_eq!(
            queue
                .requeue_failed_remote_mutations(&pool)
                .await
                .expect("retry old failures"),
            0
        );
        let ordered: Vec<String> = sqlx::query_scalar(
            "SELECT action_type FROM sync_queue WHERE account_id = 'A' ORDER BY rowid",
        )
        .fetch_all(&pool)
        .await
        .expect("A action order");
        assert_eq!(ordered, vec![ACTION_SYNC_DRAFT]);
    }

    #[tokio::test]
    async fn an_inflight_delete_that_fails_after_switch_back_is_removed_before_upload() {
        let pool = draft_test_pool().await;
        let queue = crate::sync::queue::SyncQueue::new();
        save(&pool, &queue, &draft_input("A", "first A"))
            .await
            .expect("first A save");
        sqlx::query("DELETE FROM sync_queue WHERE action_type = ?")
            .bind(ACTION_SYNC_DRAFT)
            .execute(&pool)
            .await
            .expect("first A upload completed");
        sqlx::query("UPDATE messages SET remote_draft_id = 'remote-A' WHERE id = 'draft'")
            .execute(&pool)
            .await
            .expect("remote A identity");
        save(&pool, &queue, &draft_input("B", "B revision"))
            .await
            .expect("switch to B");
        sqlx::query("UPDATE sync_queue SET status = 'sending' WHERE account_id = 'A' AND action_type = 'remote_mutation'")
            .execute(&pool)
            .await
            .expect("claim old A deletion");
        save(&pool, &queue, &draft_input("A", "latest A"))
            .await
            .expect("switch back to A");
        let sending: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sync_queue WHERE account_id = 'A' AND status = 'sending'",
        )
        .fetch_one(&pool)
        .await
        .expect("sending cleanup");
        assert_eq!(
            sending, 1,
            "an already claimed deletion cannot be cancelled by save"
        );

        // The account's queue worker settles this action before starting the
        // later A upload; its executor then removes the failed obsolete row.
        sqlx::query("UPDATE sync_queue SET status = 'failed' WHERE account_id = 'A' AND action_type = 'remote_mutation'")
            .execute(&pool)
            .await
            .expect("old A deletion failed");
        let mut tx = pool.begin().await.expect("cleanup transaction");
        assert_eq!(
            cancel_obsolete_draft_deletions(&mut tx, "draft", "A")
                .await
                .expect("cancel obsolete failure"),
            1
        );
        tx.commit().await.expect("commit cancellation");
        assert_eq!(
            queue
                .requeue_failed_remote_mutations(&pool)
                .await
                .expect("retry old failures"),
            0
        );
    }

    #[tokio::test]
    async fn retrying_failed_cleanup_before_switch_back_still_cancels_it() {
        let pool = draft_test_pool().await;
        let queue = crate::sync::queue::SyncQueue::new();
        save(&pool, &queue, &draft_input("A", "first A"))
            .await
            .expect("first A save");
        sqlx::query("DELETE FROM sync_queue WHERE action_type = ?")
            .bind(ACTION_SYNC_DRAFT)
            .execute(&pool)
            .await
            .expect("first A upload completed");
        sqlx::query("UPDATE messages SET remote_draft_id = 'remote-A' WHERE id = 'draft'")
            .execute(&pool)
            .await
            .expect("remote A identity");
        save(&pool, &queue, &draft_input("B", "B revision"))
            .await
            .expect("switch to B");
        sqlx::query("UPDATE sync_queue SET status = 'failed' WHERE account_id = 'A' AND action_type = 'remote_mutation'")
            .execute(&pool)
            .await
            .expect("old A deletion failed");
        assert_eq!(
            queue
                .requeue_failed_remote_mutations(&pool)
                .await
                .expect("user retry"),
            1
        );
        save(&pool, &queue, &draft_input("A", "latest A"))
            .await
            .expect("switch back to A");
        let old_cleanup: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sync_queue WHERE account_id = 'A' AND action_type = 'remote_mutation'",
        )
        .fetch_one(&pool)
        .await
        .expect("old cleanup count");
        assert_eq!(old_cleanup, 0);
    }

    #[tokio::test]
    async fn discard_projection_failure_uses_thread_persistence_key_without_sql_detail() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("pool");
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .expect("migrations");
        sqlx::raw_sql(
            "INSERT INTO accounts
             (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
             VALUES('account', 'Account', 'me@example.test', 'imap', '{}', '', '2026-01-01', '2026-01-01');
             INSERT INTO folders
             (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at)
             VALUES('drafts', 'account', 'Drafts', 'Drafts', 'drafts', 'Drafts',
                    '2026-01-01', '2026-01-01'),
                   ('inbox', 'account', 'Inbox', 'INBOX', 'inbox', 'INBOX',
                    '2026-01-01', '2026-01-01');
             INSERT INTO threads
             (id, account_id, subject, last_message_date, message_count, unread_count,
              created_at, updated_at)
             VALUES('thread', 'account', 'Subject', '2026-01-01', 2, 0,
                    '2026-01-01', '2026-01-01');
             INSERT INTO messages
             (id, account_id, folder_id, thread_id, from_address, to_addresses, subject,
              date, is_read, is_draft, created_at, updated_at)
             VALUES('message', 'account', 'inbox', 'thread', 'sender@example.test', '[]',
                    'Subject', '2026-01-01', 1, 0, '2026-01-01', '2026-01-01'),
                   ('draft', 'account', 'drafts', 'thread', 'me@example.test', '[]',
                    'Subject', '2026-01-02', 1, 1, '2026-01-01', '2026-01-01');
             CREATE TRIGGER fail_thread_projection BEFORE UPDATE ON threads
             BEGIN SELECT RAISE(ABORT, 'private draft SQL detail'); END;",
        )
        .execute(&pool)
        .await
        .expect("fixture");
        let queue = crate::sync::queue::SyncQueue::new();

        let error = discard(&pool, &queue, "draft")
            .await
            .expect_err("projection failure");
        let serialized: serde_json::Value = serde_json::from_str(&error.encoded()).unwrap();

        assert_eq!(
            serialized,
            serde_json::json!({ "key": "backend.errors.threadPersistenceFailed" })
        );
        assert!(!serialized.to_string().contains("private draft SQL detail"));
    }

    #[tokio::test]
    async fn a_trashed_draft_cannot_be_reopened_as_an_active_draft() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("pool");
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .expect("migrations");
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO accounts
             (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
             VALUES ('account', 'Me', 'me@example.com', 'imap', '{}', '', ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("account");
        sqlx::query(
            "INSERT INTO folders
             (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at)
             VALUES ('trash', 'account', 'Trash', 'Trash', 'trash', 'Trash', ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("folder");
        sqlx::query(
            "INSERT INTO messages
             (id, account_id, folder_id, from_address, to_addresses, subject, date,
              body_text, is_read, is_draft, created_at, updated_at)
             VALUES ('draft', 'account', 'trash', 'me@example.com', '[]', 'Discarded', ?,
                     'Body', 1, 1, ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("draft");

        assert!(load_draft_inner(&pool, &pool, "draft")
            .await
            .expect("load")
            .is_none());
    }

    #[tokio::test]
    async fn imported_draft_hydrates_cached_remote_attachments_before_editing() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("pool");
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .expect("migrations");
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO accounts
             (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
             VALUES ('account', 'Me', 'me@example.com', 'imap', '{}', '', ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("account");
        sqlx::query(
            "INSERT INTO folders
             (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at)
             VALUES ('drafts', 'account', 'Drafts', 'Drafts', 'drafts', 'Drafts', ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("folder");
        sqlx::query(
            "INSERT INTO messages
             (id, account_id, folder_id, thread_id, message_id_header, from_address,
              to_addresses, subject, date, body_text, is_read, is_draft, remote_id,
              created_at, updated_at)
             VALUES ('draft', 'account', 'drafts', NULL, '<draft@example.com>',
                     'me@example.com', '[]', 'Draft', ?, 'Body', 1, 1, '42', ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("draft");
        let cache_path =
            std::env::temp_dir().join(format!("satchel-draft-attachment-{}", uuid::Uuid::new_v4()));
        tokio::fs::write(&cache_path, b"attachment body")
            .await
            .expect("cache");
        sqlx::query(
            "INSERT INTO attachments
             (id, message_id, filename, content_type, content_id, is_inline, cached_path,
              remote_uid, remote_part_id, created_at)
             VALUES ('attachment', 'draft', 'note.txt', 'text/plain', NULL, 0, ?,
                     '42', 'part-1', ?)",
        )
        .bind(cache_path.to_string_lossy().to_string())
        .bind(&now)
        .execute(&pool)
        .await
        .expect("attachment");

        let error = load_draft_inner(&pool, &pool, "draft").await.unwrap_err();
        assert!(error.contains("imapUidValidityMissing"));
        sqlx::raw_sql(
            "UPDATE messages SET remote_uidvalidity = 7;
            UPDATE folders SET uidvalidity = 8;",
        )
        .execute(&pool)
        .await
        .unwrap();
        let error = load_draft_inner(&pool, &pool, "draft").await.unwrap_err();
        assert!(error.contains("imapUidValidityChanged"));

        // A verified cache remains usable without downloading the draft again.
        use sha2::{Digest, Sha256};
        let prefix: String = Sha256::digest(b"attachment:imap:7")
            .iter()
            .take(12)
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let verified_path =
            cache_path.with_file_name(format!("{prefix}-{}-note.txt", uuid::Uuid::new_v4()));
        tokio::fs::rename(&cache_path, &verified_path)
            .await
            .unwrap();
        sqlx::query("UPDATE attachments SET cached_path = ?")
            .bind(verified_path.to_string_lossy().as_ref())
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("UPDATE folders SET uidvalidity = 7")
            .execute(&pool)
            .await
            .unwrap();
        let draft = load_draft_inner(&pool, &pool, "draft")
            .await
            .expect("load")
            .expect("draft dto");
        assert_eq!(draft.attachments.len(), 1);
        assert_eq!(draft.attachments[0].filename, "note.txt");
        assert_eq!(draft.attachments[0].mime_type, "text/plain");
        assert_eq!(draft.attachments[0].content_base64, "YXR0YWNobWVudCBib2R5");
        let stored: String =
            sqlx::query_scalar("SELECT draft_attachments_json FROM messages WHERE id = 'draft'")
                .fetch_one(&pool)
                .await
                .expect("stored attachments");
        assert!(stored.contains("note.txt"));
        let _ = tokio::fs::remove_file(verified_path).await;
    }

    /// attachments.message_id carries no ON DELETE clause, so discarding a
    /// draft that has attachment rows used to die on the foreign key.
    #[tokio::test]
    async fn discarding_a_draft_with_attachments_survives_the_foreign_key() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("pool");
        // The app connects with foreign_keys(true); the test must too, or the
        // regression this guards against cannot fire at all.
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .expect("fk pragma");
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .expect("migrations");
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO accounts
             (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
             VALUES ('account', 'Me', 'me@example.com', 'imap', '{}', '', ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("account");
        sqlx::query(
            "INSERT INTO folders
             (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at)
             VALUES ('drafts', 'account', 'Drafts', 'Drafts', 'drafts', 'Drafts', ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("folder");
        sqlx::query(
            "INSERT INTO messages
             (id, account_id, folder_id, thread_id, from_address, to_addresses, subject, date,
              body_text, is_read, is_draft, created_at, updated_at)
             VALUES ('draft', 'account', 'drafts', NULL, 'me@example.com', '[]', 'Betreff', ?,
                     'Text', 1, 1, ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("draft");
        sqlx::query(
            "INSERT INTO attachments
             (id, message_id, filename, content_type, is_inline, created_at)
             VALUES ('attachment', 'draft', 'liste.xlsx', 'application/vnd.ms-excel', 0, ?)",
        )
        .bind(&now)
        .execute(&pool)
        .await
        .expect("attachment");

        let queue = crate::sync::queue::SyncQueue::new();
        delete_after_send(&pool, &queue, "draft")
            .await
            .expect("discard with attachment");

        let messages: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages")
            .fetch_one(&pool)
            .await
            .expect("messages");
        let attachments: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM attachments")
            .fetch_one(&pool)
            .await
            .expect("attachments");
        assert_eq!((messages, attachments), (0, 0));
    }

    /// The provider half of a discard goes to the queue. Doing it inline meant
    /// a rate-limited Gmail — blocks run up to an hour — could refuse the
    /// delete outright, leaving the draft in the conversation with an error
    /// nobody can act on.
    #[tokio::test]
    async fn discarding_queues_the_remote_delete_instead_of_awaiting_it() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("pool");
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .expect("migrations");
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO accounts
             (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
             VALUES ('account', 'Me', 'me@example.com', 'gmail', '{}', '', ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("account");
        sqlx::query(
            "INSERT INTO folders
             (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at)
             VALUES ('drafts', 'account', 'Drafts', 'Drafts', 'drafts', 'DRAFT', ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("folder");
        sqlx::query(
            "INSERT INTO messages
             (id, account_id, folder_id, thread_id, from_address, to_addresses, subject, date,
              body_text, is_read, is_draft, remote_id, remote_draft_id, created_at, updated_at)
             VALUES ('draft', 'account', 'drafts', NULL, 'me@example.com', '[]', 'Betreff', ?,
                     'Text', 1, 1, 'msg-42', 'r-7', ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("draft");

        let queue = crate::sync::queue::SyncQueue::new();
        delete_after_send(&pool, &queue, "draft")
            .await
            .expect("discard");

        let gone: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE id = 'draft'")
            .fetch_one(&pool)
            .await
            .expect("count");
        assert_eq!(gone, 0, "the local draft goes immediately");
        let queued: String = sqlx::query_scalar(
            "SELECT payload FROM sync_queue WHERE action_type = 'remote_mutation'",
        )
        .fetch_one(&pool)
        .await
        .expect("queued remote delete");
        assert!(queued.contains("delete_draft"), "{queued}");
        assert!(queued.contains("r-7"), "{queued}");
    }

    /// A draft written inside a conversation is a message of that thread.
    /// Discarding it has to leave the thread's projection correct — and take
    /// the thread with it when the draft was the only message left.
    #[tokio::test]
    async fn discarding_a_thread_draft_refreshes_the_conversation() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("pool");
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .expect("migrations");
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO accounts
             (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
             VALUES ('account', 'Me', 'me@example.com', 'gmail', '{}', '', ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("account");
        sqlx::query(
            "INSERT INTO folders
             (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at)
             VALUES ('drafts', 'account', 'Drafts', 'Drafts', 'drafts', 'Drafts', ?, ?),
                    ('inbox', 'account', 'INBOX', 'INBOX', 'inbox', 'INBOX', ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("folders");
        for (thread_id, subject) in [("th-kept", "Mit Antwort"), ("th-lonely", "Nur Entwurf")] {
            sqlx::query(
                "INSERT INTO threads
                 (id, account_id, subject, participants, message_count, unread_count,
                  has_attachments, workflow_state, board_column, priority, last_message_date,
                  created_at, updated_at)
                 VALUES (?, 'account', ?, '[]', 2, 0, 0, 'inbox', 'inbox', 0, ?, ?, ?)",
            )
            .bind(thread_id)
            .bind(subject)
            .bind(&now)
            .bind(&now)
            .bind(&now)
            .execute(&pool)
            .await
            .expect("thread");
        }
        sqlx::query(
            "INSERT INTO messages
             (id, account_id, folder_id, thread_id, from_address, to_addresses, subject, date,
              body_text, is_read, is_draft, created_at, updated_at)
             VALUES ('incoming', 'account', 'inbox', 'th-kept', 'you@example.com', '[]',
                     'Mit Antwort', ?, 'Hallo', 1, 0, ?, ?),
                    ('draft-kept', 'account', 'drafts', 'th-kept', 'me@example.com', '[]',
                     'Mit Antwort', ?, 'Entwurf', 1, 1, ?, ?),
                    ('draft-lonely', 'account', 'drafts', 'th-lonely', 'me@example.com', '[]',
                     'Nur Entwurf', ?, 'Entwurf', 1, 1, ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .bind(&now)
        .bind(&now)
        .bind(&now)
        .bind(&now)
        .bind(&now)
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("messages");

        let queue = crate::sync::queue::SyncQueue::new();
        delete_after_send(&pool, &queue, "draft-kept")
            .await
            .expect("discard");
        delete_after_send(&pool, &queue, "draft-lonely")
            .await
            .expect("discard");

        let kept: i64 =
            sqlx::query_scalar("SELECT message_count FROM threads WHERE id = 'th-kept'")
                .fetch_one(&pool)
                .await
                .expect("kept thread");
        assert_eq!(kept, 1, "the remaining message still counts");
        let lonely: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM threads WHERE id = 'th-lonely'")
            .fetch_one(&pool)
            .await
            .expect("lonely thread");
        assert_eq!(lonely, 0, "a thread without messages must not survive");
    }
}

#[cfg(test)]
mod queued_send_tests {
    use super::*;

    async fn fixture() -> (SqlitePool, SyncQueue) {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, created_at, updated_at) VALUES ('a', 'A', 'a@example.test', 'imap', '{}', 'test', '2026-01-01', '2026-01-01')")
            .execute(&pool).await.unwrap();
        (pool, SyncQueue::new())
    }

    fn action(draft_id: Option<&str>) -> QueuedAction {
        let input: OutgoingMessageInput = serde_json::from_value(serde_json::json!({
            "draft_id": draft_id, "to": ["you@example.test"], "cc": ["cc@example.test"], "bcc": ["bcc@example.test"],
            "subject": "Latest subject", "body": "Latest edits before autosave", "body_html": "<p>Latest edits</p>",
            "from_address": "alias@example.test", "from_name": "Alias", "reply_to": "reply@example.test",
            "reply_to_message_id": "reply-branch", "priority": "high", "follow_up_at": "2026-12-01T12:00:00Z",
            "pgp_sign_with_key_id": "test-fingerprint", "pgp_encrypt_to_recipients": true,
            "attachments": [{"filename": "offer.pdf", "mime_type": "application/pdf", "content_base64": "aGVsbG8=", "content_id": "offer", "inline": true}]
        })).unwrap();
        build_send_action(
            "a".into(),
            input,
            Some(chrono::Utc::now() + chrono::Duration::days(1)),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn undo_restores_latest_content_and_full_attachments_with_or_without_autosave() {
        for id in [None, Some("old-draft")] {
            let (pool, queue) = fixture().await;
            let action = action(id);
            enqueue_send_with_draft(&pool, &queue, &action)
                .await
                .unwrap();
            if let Some(id) = id {
                sqlx::query("UPDATE messages SET body_text = 'Old autosave' WHERE id = ?")
                    .bind(id)
                    .execute(&pool)
                    .await
                    .unwrap();
            }
            let draft = restore_queued_draft(&pool, &queue, &action.id)
                .await
                .unwrap();
            if let Some(id) = id {
                assert_eq!(draft.id, id);
            }
            let expected = &action.payload["input"];
            for field in [
                "to",
                "cc",
                "bcc",
                "subject",
                "body",
                "body_html",
                "from_address",
                "from_name",
                "reply_to",
                "attachments",
            ] {
                assert_eq!(
                    serde_json::to_value(&draft).unwrap()[field],
                    expected[field],
                    "{field}"
                );
            }
            assert_eq!(
                draft.send_options.as_ref().unwrap()["reply_to_message_id"],
                "reply-branch"
            );
            assert_eq!(draft.send_options.as_ref().unwrap()["priority"], "high");
            assert_eq!(
                draft.send_options.as_ref().unwrap()["pgp_encrypt_to_recipients"],
                true
            );
            assert_eq!(draft.scheduled_for, None);
            assert!(queue.list_mail_outbox(&pool).await.unwrap().is_empty());
            assert!(restore_queued_draft(&pool, &queue, &action.id)
                .await
                .is_err());
        }
    }

    #[tokio::test]
    async fn failed_restore_rolls_back_the_conditional_delete() {
        let (pool, queue) = fixture().await;
        let action = action(None);
        queue.enqueue(&pool, action.clone()).await.unwrap();
        sqlx::query("CREATE TRIGGER reject_draft BEFORE INSERT ON messages BEGIN SELECT RAISE(ABORT, 'write failed'); END")
            .execute(&pool).await.unwrap();
        assert!(restore_queued_draft(&pool, &queue, &action.id)
            .await
            .is_err());
        assert_eq!(
            queue.list_mail_outbox(&pool).await.unwrap()[0].id,
            action.id
        );
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE is_draft = 1")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn undo_and_claim_preserve_the_winners_record() {
        let (pool, queue) = fixture().await;
        let mut action = action(None);
        action.status = "pending".into();
        action.scheduled_for = None;
        queue.enqueue(&pool, action.clone()).await.unwrap();
        queue.claim_ready(&pool, &action.id).await.unwrap().unwrap();
        assert!(restore_queued_draft(&pool, &queue, &action.id)
            .await
            .is_err());
        assert_eq!(
            queue.list_mail_outbox(&pool).await.unwrap()[0].status,
            "sending"
        );
        let mut undone = action.clone();
        undone.id = "undo-first".into();
        undone.status = "pending".into();
        queue.enqueue(&pool, undone.clone()).await.unwrap();
        restore_queued_draft(&pool, &queue, &undone.id)
            .await
            .unwrap();
        assert!(queue
            .claim_ready(&pool, &undone.id)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn opening_scheduled_draft_cancels_before_editing_and_rescheduling() {
        let (pool, queue) = fixture().await;
        let action = action(Some("scheduled-draft"));
        enqueue_send_with_draft(&pool, &queue, &action)
            .await
            .unwrap();
        assert!(enqueue_send_with_draft(&pool, &queue, &action)
            .await
            .is_err());
        let projected = load_draft_inner(&pool, &pool, "scheduled-draft")
            .await
            .unwrap()
            .unwrap();
        let mut input: DraftInput =
            serde_json::from_value(serde_json::to_value(&projected).unwrap()).unwrap();
        input.body = "Stale composer edit".into();
        assert!(save(&pool, &queue, &input).await.is_err());
        let opened = load(&pool, &pool, &queue, "scheduled-draft")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(opened.body, "Latest edits before autosave");
        assert_eq!(opened.scheduled_for, None);
        assert!(opened.send_cancelled);
        assert!(queue.list_mail_outbox(&pool).await.unwrap().is_empty());
        input.body = "Corrected version".into();
        input.scheduled_for = None;
        save(&pool, &queue, &input).await.unwrap();
        assert_eq!(
            load_draft_inner(&pool, &pool, "scheduled-draft")
                .await
                .unwrap()
                .unwrap()
                .body,
            "Corrected version"
        );
        let mut replacement = action.clone();
        replacement.id = "replacement".into();
        replacement.payload["input"]["body"] = serde_json::json!("Corrected version");
        enqueue_send_with_draft(&pool, &queue, &replacement)
            .await
            .unwrap();
        assert_eq!(queue.list_mail_outbox(&pool).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn known_rejection_can_be_edited_and_resubmitted_but_uncertain_delivery_stays_locked() {
        let (pool, queue) = fixture().await;
        let mut original = action(Some("rejected-draft"));
        original.status = "sending".into();
        enqueue_send_with_draft(&pool, &queue, &original)
            .await
            .unwrap();
        queue
            .mark_failed_permanent(&pool, &original.id, "recipient rejected")
            .await
            .unwrap();
        let draft = load_draft_inner(&pool, &pool, "rejected-draft")
            .await
            .unwrap()
            .unwrap();
        let mut input: DraftInput =
            serde_json::from_value(serde_json::to_value(draft).unwrap()).unwrap();
        input.body = "Corrected text".into();
        let (saved, _) = save(&pool, &queue, &input).await.unwrap();
        assert!(saved.send_cancelled);
        assert!(queue.list_mail_outbox(&pool).await.unwrap().is_empty());
        let mut replacement = action(Some("rejected-draft"));
        replacement.payload["input"]["body"] = serde_json::json!("Corrected text");
        enqueue_send_with_draft(&pool, &queue, &replacement)
            .await
            .unwrap();
        queue
            .mark_failed_permanent(&pool, &replacement.id, "rejected again")
            .await
            .unwrap();
        let mut direct_retry = replacement.clone();
        direct_retry.id = "explicit-resend".into();
        enqueue_send_with_draft(&pool, &queue, &direct_retry)
            .await
            .unwrap();
        assert_eq!(queue.list_mail_outbox(&pool).await.unwrap().len(), 1);
        queue
            .mark_submission_started(&pool, &direct_retry.id)
            .await
            .unwrap();
        queue
            .mark_delivery_unknown(&pool, &direct_retry.id)
            .await
            .unwrap();
        assert!(save(&pool, &queue, &input).await.is_err());
        assert!(enqueue_send_with_draft(&pool, &queue, &replacement)
            .await
            .is_err());
        assert_eq!(
            queue.list_mail_outbox(&pool).await.unwrap()[0].status,
            "delivery_unknown"
        );
    }

    #[tokio::test]
    async fn deleting_scheduled_draft_cancels_send_but_worker_owned_draft_is_protected() {
        let (pool, queue) = fixture().await;
        let action = action(Some("scheduled-draft"));
        enqueue_send_with_draft(&pool, &queue, &action)
            .await
            .unwrap();
        discard(&pool, &queue, "scheduled-draft").await.unwrap();
        assert!(queue.list_mail_outbox(&pool).await.unwrap().is_empty());
        assert!(load_draft_inner(&pool, &pool, "scheduled-draft")
            .await
            .unwrap()
            .is_none());
        let mut sending = action.clone();
        sending.id = "worker-owned".into();
        sending.status = "sending".into();
        enqueue_send_with_draft(&pool, &queue, &sending)
            .await
            .unwrap();
        assert!(discard(&pool, &queue, "scheduled-draft").await.is_err());
        assert!(load(&pool, &pool, &queue, "scheduled-draft").await.is_err());
        assert_eq!(
            queue.list_mail_outbox(&pool).await.unwrap()[0].status,
            "sending"
        );
    }
}
