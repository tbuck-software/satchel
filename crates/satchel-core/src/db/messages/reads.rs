//! Message display/read projections and their persistence implementation.

use sqlx::{FromRow, QueryBuilder, Sqlite, SqlitePool};
use thiserror::Error;

#[derive(Debug, FromRow)]
struct StoredMessageRow {
    id: String,
    thread_id: Option<String>,
    subject: Option<String>,
    snippet: Option<String>,
    from_address: Option<String>,
    from_name: Option<String>,
    to_addresses: Option<String>,
    cc_addresses: Option<String>,
    reply_to: Option<String>,
    date: Option<String>,
    body_text: Option<String>,
    body_html: Option<String>,
    is_read: Option<i64>,
    is_draft: Option<i64>,
    list_unsubscribe: Option<String>,
    list_unsubscribe_post: Option<String>,
    auth_spf: Option<String>,
    auth_dkim: Option<String>,
    auth_dmarc: Option<String>,
    smime_encrypted: Option<Vec<u8>>,
    smime_signed: Option<Vec<u8>>,
    pgp_signed: Option<Vec<u8>>,
    trust_metadata: Option<String>,
    source_payload: Option<String>,
}

#[derive(Debug, FromRow)]
struct StoredMessagePreviewRow {
    id: String,
    thread_id: Option<String>,
    subject: Option<String>,
    snippet: Option<String>,
    from_address: Option<String>,
    from_name: Option<String>,
    to_addresses: Option<String>,
    cc_addresses: Option<String>,
    reply_to: Option<String>,
    date: Option<String>,
    is_read: Option<i64>,
    is_draft: Option<i64>,
}

/// Read data needed by the app to map and render a full message.
///
/// This is deliberately separate from `StoredMessageRow`: SQLx's row decoding
/// implementation and the database projection remain private to Core.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageReadRecord {
    pub id: String,
    pub thread_id: Option<String>,
    pub subject: Option<String>,
    pub snippet: Option<String>,
    pub from_address: Option<String>,
    pub from_name: Option<String>,
    pub to_addresses: Option<String>,
    pub cc_addresses: Option<String>,
    pub reply_to: Option<String>,
    pub date: Option<String>,
    pub body_text: Option<String>,
    pub body_html: Option<String>,
    pub is_read: Option<i64>,
    pub is_draft: Option<i64>,
    pub list_unsubscribe: Option<String>,
    pub list_unsubscribe_post: Option<String>,
    pub auth_spf: Option<String>,
    pub auth_dkim: Option<String>,
    pub auth_dmarc: Option<String>,
    pub smime_encrypted: Option<Vec<u8>>,
    pub smime_signed: Option<Vec<u8>>,
    pub pgp_signed: Option<Vec<u8>>,
    pub trust_metadata: Option<String>,
    pub source_payload: Option<String>,
}

/// Read data needed by the app to map and render a message preview.
///
/// Body and cryptographic payload columns intentionally do not belong to this
/// record or its SQL projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessagePreviewRecord {
    pub id: String,
    pub thread_id: Option<String>,
    pub subject: Option<String>,
    pub snippet: Option<String>,
    pub from_address: Option<String>,
    pub from_name: Option<String>,
    pub to_addresses: Option<String>,
    pub cc_addresses: Option<String>,
    pub reply_to: Option<String>,
    pub date: Option<String>,
    pub is_read: Option<i64>,
    pub is_draft: Option<i64>,
}

impl From<StoredMessageRow> for MessageReadRecord {
    fn from(row: StoredMessageRow) -> Self {
        Self {
            id: row.id,
            thread_id: row.thread_id,
            subject: row.subject,
            snippet: row.snippet,
            from_address: row.from_address,
            from_name: row.from_name,
            to_addresses: row.to_addresses,
            cc_addresses: row.cc_addresses,
            reply_to: row.reply_to,
            date: row.date,
            body_text: row.body_text,
            body_html: row.body_html,
            is_read: row.is_read,
            is_draft: row.is_draft,
            list_unsubscribe: row.list_unsubscribe,
            list_unsubscribe_post: row.list_unsubscribe_post,
            auth_spf: row.auth_spf,
            auth_dkim: row.auth_dkim,
            auth_dmarc: row.auth_dmarc,
            smime_encrypted: row.smime_encrypted,
            smime_signed: row.smime_signed,
            pgp_signed: row.pgp_signed,
            trust_metadata: row.trust_metadata,
            source_payload: row.source_payload,
        }
    }
}

impl From<StoredMessagePreviewRow> for MessagePreviewRecord {
    fn from(row: StoredMessagePreviewRow) -> Self {
        Self {
            id: row.id,
            thread_id: row.thread_id,
            subject: row.subject,
            snippet: row.snippet,
            from_address: row.from_address,
            from_name: row.from_name,
            to_addresses: row.to_addresses,
            cc_addresses: row.cc_addresses,
            reply_to: row.reply_to,
            date: row.date,
            is_read: row.is_read,
            is_draft: row.is_draft,
        }
    }
}

/// Column list for the full read projection. Keep it in sync with the private
/// stored row and use it for both thread and body reads.
const MESSAGE_READ_COLUMNS: &str = "id, thread_id, subject, snippet, from_address, from_name, \
     to_addresses, cc_addresses, reply_to, date, body_text, body_html, is_read, is_draft, \
     list_unsubscribe, list_unsubscribe_post, auth_spf, auth_dkim, auth_dmarc, \
     smime_encrypted, smime_signed, pgp_signed, trust_metadata, source_payload";

const MESSAGE_READ_PROJECTION: &str = "messages.id, messages.thread_id, messages.subject, messages.snippet, messages.from_address, messages.from_name, messages.to_addresses, messages.cc_addresses, messages.reply_to, messages.date, messages.body_text, messages.body_html, messages.is_read, CASE WHEN messages.is_draft = 1 AND LOWER(TRIM(folders.folder_type)) <> 'trash' THEN 1 ELSE 0 END AS is_draft, messages.list_unsubscribe, messages.list_unsubscribe_post, messages.auth_spf, messages.auth_dkim, messages.auth_dmarc, messages.smime_encrypted, messages.smime_signed, messages.pgp_signed, messages.trust_metadata, messages.source_payload";

/// The preview projection intentionally leaves out bodies and every unbounded
/// payload, `source_payload` included: the only consumer renders it next to a
/// loaded body, and it arrives with that body.
const MESSAGE_PREVIEW_COLUMNS: &str = "id, thread_id, subject, snippet, from_address, from_name, to_addresses, cc_addresses, reply_to, date, is_read, is_draft";

const MESSAGE_PREVIEW_PROJECTION: &str = "messages.id, messages.thread_id, messages.subject, messages.snippet, messages.from_address, messages.from_name, messages.to_addresses, messages.cc_addresses, messages.reply_to, messages.date, messages.is_read, CASE WHEN messages.is_draft = 1 AND LOWER(TRIM(folders.folder_type)) <> 'trash' THEN 1 ELSE 0 END AS is_draft";

/// Computes the identity used to collapse provider copies of one message.
/// Synthetic IMAP Message-IDs are local implementation details, so they fall
/// back to the folder and provider remote ID instead.
const MESSAGE_DEDUPE_KEY: &str = "CASE
    WHEN messages.id LIKE 'm-imap-%' THEN COALESCE(
      NULLIF(TRIM(CASE
        WHEN LOWER(COALESCE(messages.message_id_header, '')) LIKE '%@imap.satchel.local>'
          OR LOWER(COALESCE(messages.message_id_header, '')) LIKE '%@imap.flowmail.local>' THEN ''
        ELSE COALESCE(messages.message_id_header, '')
      END), ''),
      COALESCE(NULLIF(TRIM(messages.folder_id), ''), '') || ':'
        || COALESCE(NULLIF(TRIM(messages.remote_id), ''), messages.id)
    )
    ELSE COALESCE(
      NULLIF(TRIM(messages.message_id_header), ''),
      NULLIF(TRIM(messages.remote_id), ''),
      messages.id
    )
  END";

const MESSAGE_REPRESENTATIVE_ORDER: &str =
    "CASE WHEN messages.is_read = 0 THEN 0 ELSE 1 END, messages.date ASC, messages.id ASC";

fn thread_messages_query(projection: &str, columns: &str) -> String {
    format!(
        "WITH ranked_messages AS (
           SELECT {projection},
                  ROW_NUMBER() OVER (
                    PARTITION BY {MESSAGE_DEDUPE_KEY}
                    ORDER BY {MESSAGE_REPRESENTATIVE_ORDER}
                  ) AS rn
           FROM messages
           JOIN folders ON folders.id = messages.folder_id
           WHERE messages.thread_id = ?
         )
         SELECT {columns}
         FROM ranked_messages
         WHERE rn = 1
         ORDER BY date ASC"
    )
}

#[derive(Debug, Error)]
pub enum MessageReadError {
    #[error("message read persistence failed: {0}")]
    Persistence(#[from] sqlx::Error),
    #[error("message was not found")]
    NotFound,
}

/// Persistence boundary for the message display/read projections.
pub struct MessageReadRepository<'a> {
    pool: &'a SqlitePool,
}

impl<'a> MessageReadRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn list_messages(
        &self,
        thread_id: &str,
    ) -> Result<Vec<MessageReadRecord>, MessageReadError> {
        let rows = sqlx::query_as::<_, StoredMessageRow>(&thread_messages_query(
            MESSAGE_READ_PROJECTION,
            MESSAGE_READ_COLUMNS,
        ))
        .bind(thread_id)
        .fetch_all(self.pool)
        .await?;

        Ok(rows.into_iter().map(MessageReadRecord::from).collect())
    }

    pub async fn list_messages_preview(
        &self,
        thread_id: &str,
    ) -> Result<Vec<MessagePreviewRecord>, MessageReadError> {
        let rows = sqlx::query_as::<_, StoredMessagePreviewRow>(&thread_messages_query(
            MESSAGE_PREVIEW_PROJECTION,
            MESSAGE_PREVIEW_COLUMNS,
        ))
        .bind(thread_id)
        .fetch_all(self.pool)
        .await?;

        Ok(rows.into_iter().map(MessagePreviewRecord::from).collect())
    }

    pub async fn list_message_bodies(
        &self,
        message_ids: &[String],
    ) -> Result<Vec<MessageReadRecord>, MessageReadError> {
        if message_ids.is_empty() {
            return Ok(Vec::new());
        }

        let mut query = QueryBuilder::<Sqlite>::new(format!(
            "SELECT {MESSAGE_READ_COLUMNS} FROM messages WHERE id IN ("
        ));
        let mut separated = query.separated(", ");
        for message_id in message_ids {
            separated.push_bind(message_id);
        }
        separated.push_unseparated(") ORDER BY date ASC");

        let rows = query
            .build_query_as::<StoredMessageRow>()
            .fetch_all(self.pool)
            .await?;
        Ok(rows.into_iter().map(MessageReadRecord::from).collect())
    }

    pub async fn get_message_source(
        &self,
        message_id: &str,
    ) -> Result<MessageSourceRecord, MessageReadError> {
        let row: Option<(
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<i64>,
        )> = sqlx::query_as(
            "SELECT id, subject, raw_headers, body_text, COALESCE(original_body_html, body_html), raw_size
                 FROM messages WHERE id = ? LIMIT 1",
        )
        .bind(message_id)
        .fetch_optional(self.pool)
        .await?;

        let (message_id, subject, raw_headers, body_text, body_html, raw_size) =
            row.ok_or(MessageReadError::NotFound)?;

        Ok(MessageSourceRecord {
            message_id,
            subject,
            raw_headers: raw_headers.filter(|value| !value.trim().is_empty()),
            body_text,
            body_html,
            raw_size,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageSourceRecord {
    pub message_id: String,
    pub subject: Option<String>,
    /// Raw RFC822 header block. Empty or whitespace-only values are absent.
    pub raw_headers: Option<String>,
    pub body_text: Option<String>,
    pub body_html: Option<String>,
    pub raw_size: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// PERF-15: an unbounded column the doc comment already excluded.
    #[test]
    fn the_preview_projection_carries_no_payload_columns() {
        for projection in [MESSAGE_PREVIEW_COLUMNS, MESSAGE_PREVIEW_PROJECTION] {
            for column in [
                "source_payload",
                "body_text",
                "body_html",
                "smime_",
                "pgp_signed",
            ] {
                assert!(!projection.contains(column), "{column} in {projection}");
            }
        }
        assert!(MESSAGE_READ_COLUMNS.contains("source_payload"));
    }
}
