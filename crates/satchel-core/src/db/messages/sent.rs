//! Persistence for locally-created sent messages and reply headers.

use chrono::Utc;
use sqlx::{FromRow, Sqlite, SqlitePool};
use thiserror::Error;

/// Metadata needed to show an outgoing attachment on its persisted message.
///
/// The attachment bytes remain an app/provider concern. Core only stores the
/// metadata that belongs to the message record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SentAttachmentMetadata {
    pub filename: String,
    pub mime_type: String,
    /// Approximate decoded byte size calculated at the compose boundary.
    pub size: i64,
    pub content_id: Option<String>,
    pub inline: bool,
}

/// Input to the sent-message persistence boundary.
///
/// Recipient normalization is intentionally performed by the app compose
/// boundary. Core receives the normalized lists and persists them as-is.
#[derive(Debug, Clone, Default)]
pub struct SentMessageWriteRequest {
    pub account_id: String,
    pub account_email: String,
    pub account_display_name: String,
    pub thread_id: Option<String>,
    pub subject: String,
    pub body: String,
    pub body_html: Option<String>,
    pub from_address: Option<String>,
    pub from_name: Option<String>,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub reply_to: Option<String>,
    /// Local id of the message being answered, as sent. Keeps the persisted
    /// Sent record's threading headers identical to the ones on the wire.
    pub reply_to_message_id: Option<String>,
    pub message_id_header: Option<String>,
    pub signature_id: Option<String>,
    pub signature_variant: Option<String>,
    pub attachments: Vec<SentAttachmentMetadata>,
}

/// Identifiers produced by a successful sent-message write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SentMessageRecord {
    pub message_id: String,
    pub thread_id: String,
}

/// RFC 5322 reply headers derived from the newest message in a thread.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReplyThreadContext {
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
}

/// Failures at the sent-message persistence boundary.
///
/// The database error remains available as a source for diagnostics, while the
/// display text deliberately does not expose SQL details at an app boundary.
#[derive(Debug, Error)]
pub enum SentMessageError {
    #[error("sent message persistence failed")]
    Persistence(#[source] sqlx::Error),
    #[error("reply context could not be loaded")]
    ReplyContextLoad(#[source] sqlx::Error),
    #[error("thread not found for reply: {thread_id}")]
    ReplyThreadNotFound { thread_id: String },
}

impl From<sqlx::Error> for SentMessageError {
    fn from(error: sqlx::Error) -> Self {
        Self::Persistence(error)
    }
}

#[derive(Debug, FromRow)]
struct ExistingThreadRow {
    subject: Option<String>,
    workflow_state: Option<String>,
    participants: Option<String>,
}

#[derive(Debug, FromRow)]
struct ReplyContextRow {
    message_id_header: Option<String>,
    references_header: Option<String>,
}

#[derive(Debug)]
struct SentThreadContext {
    id: String,
    subject: String,
    snippet: String,
    workflow_transition_from: Option<String>,
}

/// Persistence boundary for locally-created sent messages.
pub struct SentMessageRepository<'a> {
    pool: &'a SqlitePool,
}

impl<'a> SentMessageRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    /// Persist a sent message, its thread transition, workflow event, and
    /// attachment metadata in one message transaction.
    pub async fn persist(
        &self,
        request: SentMessageWriteRequest,
    ) -> Result<SentMessageRecord, SentMessageError> {
        let now = Utc::now().to_rfc3339();
        let message_id = format!("m-{}", uuid::Uuid::new_v4());
        let message_id_header = request
            .message_id_header
            .clone()
            .unwrap_or_else(|| format!("<{message_id}@satchel.local>"));
        let reply_context = if let Some(thread_id) = request.thread_id.as_deref() {
            Some(
                self.load_reply_context(
                    &request.account_id,
                    thread_id,
                    request.reply_to_message_id.as_deref(),
                )
                .await?,
            )
        } else {
            None
        };

        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        // Folder repair belongs to the same transaction: a failed sent-message
        // write must not leave a synthetic folder behind.
        let sent_folder_id = ensure_sent_folder(&mut tx, &request.account_id, &now).await?;

        let thread_context = upsert_sent_thread(
            &mut tx,
            &request.account_id,
            &request.account_email,
            request.thread_id.as_deref(),
            &request.subject,
            &request.body,
            &request.to,
            &now,
        )
        .await?;

        let from_address = request
            .from_address
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(&request.account_email);
        let from_name = request
            .from_name
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(&request.account_display_name);
        let has_attachments = if request.attachments.is_empty() { 0 } else { 1 };

        sqlx::query(
            "INSERT INTO messages (id, account_id, folder_id, thread_id, message_id_header, in_reply_to, references_header, from_address, from_name, to_addresses, cc_addresses, bcc_addresses, reply_to, subject, date, body_text, body_html, snippet, is_read, is_starred, is_draft, is_sent, has_attachments, remote_uid, remote_id, raw_size, raw_headers, signature_id, signature_variant, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 1, 0, 0, 1, ?, NULL, NULL, NULL, NULL, ?, ?, ?, ?)",
        )
        .bind(&message_id)
        .bind(&request.account_id)
        .bind(&sent_folder_id)
        .bind(&thread_context.id)
        .bind(&message_id_header)
        .bind(reply_context
            .as_ref()
            .and_then(|context| context.in_reply_to.as_deref()))
        .bind(reply_context.as_ref().map(|context| {
            if context.references.is_empty() {
                String::new()
            } else {
                context.references.join(" ")
            }
        }))
        .bind(from_address)
        .bind(from_name)
        .bind(serde_json::to_string(&request.to).unwrap_or_else(|_| "[]".into()))
        .bind(serde_json::to_string(&request.cc).unwrap_or_else(|_| "[]".into()))
        .bind(serde_json::to_string(&request.bcc).unwrap_or_else(|_| "[]".into()))
        .bind(request.reply_to.as_deref())
        .bind(&thread_context.subject)
        .bind(&now)
        .bind(&request.body)
        .bind(request.body_html.as_deref())
        .bind(&thread_context.snippet)
        .bind(has_attachments)
        .bind(request.signature_id.as_deref())
        .bind(request.signature_variant.as_deref())
        .bind(&now)
        .bind(&now)
        .execute(&mut *tx)
        .await?;

        if let Some(from_state) = thread_context.workflow_transition_from.as_deref() {
            sqlx::query(
                "INSERT INTO workflow_events (id, thread_id, from_state, to_state, reason, created_at)
                 VALUES (?, ?, ?, 'waiting', 'manual', ?)",
            )
            .bind(format!("wf-{}", uuid::Uuid::new_v4()))
            .bind(&thread_context.id)
            .bind(from_state)
            .bind(&now)
            .execute(&mut *tx)
            .await?;
        }

        for attachment in &request.attachments {
            let attachment_id = format!("att-{}", uuid::Uuid::new_v4());
            sqlx::query(
                "INSERT INTO attachments (id, message_id, filename, content_type, size, content_id, storage_path, is_inline, created_at)
                 VALUES (?, ?, ?, ?, ?, ?, NULL, ?, ?)",
            )
            .bind(&attachment_id)
            .bind(&message_id)
            .bind(crate::utils::filename::strip_filename_controls(&attachment.filename))
            .bind(&attachment.mime_type)
            .bind(attachment.size)
            .bind(attachment.content_id.as_deref())
            .bind(if attachment.inline { 1 } else { 0 })
            .bind(&now)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;

        Ok(SentMessageRecord {
            message_id,
            thread_id: thread_context.id,
        })
    }

    /// Load the RFC 5322 reply headers a new message inherits.
    ///
    /// `reply_to_message_id` is the message the user actually answered. Without
    /// it the newest message in the thread has to stand in, which points the
    /// reply at the wrong branch as soon as the user scrolls up — or as soon as
    /// a new message arrives while they are typing. A thread with no messages
    /// has an empty context.
    pub async fn load_reply_context(
        &self,
        account_id: &str,
        thread_id: &str,
        reply_to_message_id: Option<&str>,
    ) -> Result<ReplyThreadContext, SentMessageError> {
        let mut last_message = match reply_to_message_id
            .map(str::trim)
            .filter(|id| !id.is_empty())
        {
            Some(message_id) => sqlx::query_as::<_, ReplyContextRow>(
                "SELECT message_id_header, references_header
                 FROM messages
                 WHERE id = ? AND account_id = ? AND COALESCE(is_draft, 0) = 0
                 LIMIT 1",
            )
            .bind(message_id)
            .bind(account_id)
            .fetch_optional(self.pool)
            .await
            .map_err(SentMessageError::ReplyContextLoad)?,
            None => None,
        };

        if last_message.is_none() {
            // A draft is not something anyone replied to, and it sits in the
            // same thread with a newer date. `id DESC` only decides ties, but
            // two messages can share a `date` and the pick has to survive a retry.
            last_message = sqlx::query_as::<_, ReplyContextRow>(
                "SELECT message_id_header, references_header
                 FROM messages
                 WHERE thread_id = ? AND account_id = ? AND COALESCE(is_draft, 0) = 0
                 ORDER BY date DESC, id DESC
                 LIMIT 1",
            )
            .bind(thread_id)
            .bind(account_id)
            .fetch_optional(self.pool)
            .await
            .map_err(SentMessageError::ReplyContextLoad)?;
        }

        let Some(last_message) = last_message else {
            return Ok(ReplyThreadContext::default());
        };

        let in_reply_to = last_message
            .message_id_header
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let mut references = split_reference_header(last_message.references_header.as_deref());
        if let Some(message_id) = &in_reply_to {
            if !references.iter().any(|value| value == message_id) {
                references.push(message_id.clone());
            }
        }

        Ok(ReplyThreadContext {
            in_reply_to,
            references: cap_references(references),
        })
    }
}

/// How many entries a `References` header keeps.
///
/// RFC 5322 §3.6.4 permits trimming; the convention every client follows is to
/// keep the head — it names the thread's root — plus the most recent entries.
/// A 60-message ticket thread otherwise writes a header of 3000 octets.
const MAX_REFERENCES: usize = 20;

fn cap_references(mut references: Vec<String>) -> Vec<String> {
    if references.len() <= MAX_REFERENCES {
        return references;
    }
    let head = references.remove(0);
    let tail = references.split_off(references.len() - (MAX_REFERENCES - 1));
    let mut capped = Vec::with_capacity(MAX_REFERENCES);
    capped.push(head);
    capped.extend(tail);
    capped
}

async fn upsert_sent_thread(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    account_id: &str,
    account_email: &str,
    thread_id: Option<&str>,
    raw_subject: &str,
    body: &str,
    recipients: &[String],
    now: &str,
) -> Result<SentThreadContext, SentMessageError> {
    let subject = normalize_subject(raw_subject);
    let snippet = build_snippet(body, &subject);

    if let Some(thread_id) = thread_id {
        let existing = sqlx::query_as::<_, ExistingThreadRow>(
            "SELECT subject, workflow_state, participants
             FROM threads
             WHERE id = ? AND account_id = ?
             LIMIT 1",
        )
        .bind(thread_id)
        .bind(account_id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(|| SentMessageError::ReplyThreadNotFound {
            thread_id: thread_id.to_string(),
        })?;

        let merged_participants =
            merge_participants_json(existing.participants.as_deref(), recipients, account_email);
        let next_subject = if subject.is_empty() {
            existing
                .subject
                .clone()
                .unwrap_or_else(|| "(no subject)".to_string())
        } else {
            subject.clone()
        };
        let workflow_transition_from = existing
            .workflow_state
            .as_deref()
            .filter(|state| *state != "waiting")
            .map(str::to_string);

        sqlx::query(
            "UPDATE threads
             SET subject = ?, last_message_date = ?, message_count = COALESCE(message_count, 0) + 1,
                 unread_count = 0, participants = ?, snippet = ?, workflow_state = 'waiting',
                 workflow_changed_at = ?, board_column = 'waiting', updated_at = ?
             WHERE id = ?",
        )
        .bind(&next_subject)
        .bind(now)
        .bind(merged_participants)
        .bind(&snippet)
        .bind(now)
        .bind(now)
        .bind(thread_id)
        .execute(&mut **tx)
        .await?;

        return Ok(SentThreadContext {
            id: thread_id.to_string(),
            subject: next_subject,
            snippet,
            workflow_transition_from,
        });
    }

    let thread_id = format!("th-{}", uuid::Uuid::new_v4());
    sqlx::query(
        "INSERT INTO threads (id, account_id, subject, last_message_date, message_count, unread_count, has_attachments, participants, snippet, workflow_state, workflow_changed_at, snooze_until, due_date, priority, board_column, board_position, created_at, updated_at)
         VALUES (?, ?, ?, ?, 1, 0, 0, ?, ?, 'waiting', ?, NULL, NULL, 1, 'waiting', 1, ?, ?)",
    )
    .bind(&thread_id)
    .bind(account_id)
    .bind(&subject)
    .bind(now)
    .bind(merge_participants_json(None, recipients, account_email))
    .bind(&snippet)
    .bind(now)
    .bind(now)
    .bind(now)
    .execute(&mut **tx)
    .await?;

    Ok(SentThreadContext {
        id: thread_id,
        subject,
        snippet,
        workflow_transition_from: None,
    })
}

async fn ensure_sent_folder(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    account_id: &str,
    now: &str,
) -> Result<String, SentMessageError> {
    let candidate_id = crate::db::folders::folder_db_id(account_id, "Sent");
    // The INSERT and its existence check are one SQLite write statement. This
    // serializes concurrent first sends without requiring provider folders to
    // have a global uniqueness constraint.
    sqlx::query(
        "INSERT INTO folders (id, account_id, name, full_path, folder_type, remote_id, uidvalidity, message_count, unread_count, sync_enabled, sort_order, created_at, updated_at)
         SELECT ?, ?, 'Sent', 'Sent', 'sent', 'Sent', NULL, 0, 0, 1, 0, ?, ?
         WHERE NOT EXISTS (
           SELECT 1 FROM folders WHERE account_id = ? AND folder_type = 'sent'
         )
         ON CONFLICT DO NOTHING",
    )
    .bind(&candidate_id)
    .bind(account_id)
    .bind(now)
    .bind(now)
    .bind(account_id)
    .execute(&mut **tx)
    .await?;

    sqlx::query_scalar::<_, String>(
        "SELECT id FROM folders
         WHERE account_id = ? AND folder_type = 'sent'
         ORDER BY created_at ASC, id ASC
         LIMIT 1",
    )
    .bind(account_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(Into::into)
}

fn normalize_subject(subject: &str) -> String {
    let trimmed = subject.trim();
    if trimmed.is_empty() {
        "(no subject)".to_string()
    } else {
        trimmed.to_string()
    }
}

fn merge_participants_json(
    existing_raw: Option<&str>,
    recipients: &[String],
    account_email: &str,
) -> String {
    let mut participants = existing_raw
        .and_then(|raw| serde_json::from_str::<Vec<String>>(raw).ok())
        .unwrap_or_default();
    let account_email = account_email.trim().to_lowercase();

    for recipient in recipients {
        let lower = recipient.to_lowercase();
        if lower.is_empty() || lower == account_email {
            continue;
        }
        if participants
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(recipient))
        {
            continue;
        }
        participants.push(recipient.clone());
    }

    serde_json::to_string(&participants).unwrap_or_else(|_| "[]".to_string())
}

fn build_snippet(body: &str, fallback_subject: &str) -> String {
    let normalized = body
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    if normalized.is_empty() {
        return fallback_subject.to_string();
    }

    const MAX_CHARS: usize = 180;
    let mut snippet = normalized.chars().take(MAX_CHARS).collect::<String>();
    if normalized.chars().count() > MAX_CHARS {
        snippet.push('…');
    }
    snippet
}

fn split_reference_header(raw: Option<&str>) -> Vec<String> {
    let Some(raw) = raw else {
        return Vec::new();
    };

    raw.split_whitespace()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .fold(Vec::new(), |mut references, value| {
            let item = value.to_string();
            if !references.iter().any(|existing| existing == &item) {
                references.push(item);
            }
            references
        })
}
