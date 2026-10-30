//! Turn threads dragged onto the composer into `.eml` attachments.
//!
//! The original source is fetched the same way "show source" fetches it, so a
//! forwarded message keeps its headers, signatures and MIME parts byte for
//! byte. A thread contributes its newest delivered message: that one carries
//! the conversation's latest state, and a long thread would otherwise bury the
//! draft under a dozen near-identical files.

use crate::db::remote_content::ContentReference;
use base64::Engine as _;
use satchel_core::drafts::outgoing::OutgoingAttachmentInput;
use satchel_core::utils::filename::strip_filename_controls;
use sqlx::SqlitePool;

use crate::commands::error::{CommandError, CommandResult};

/// Longest subject kept in a file name; mail clients and file systems both
/// start truncating somewhere past this.
const MAX_STEM_CHARS: usize = 80;

/// Original sources of the newest message in each thread, as `message/rfc822`
/// attachments in the order the threads were given.
///
/// A thread whose source cannot be fetched (provider offline, local import
/// without the original bytes) is left out rather than failing the whole drop;
/// the caller compares the counts and says how many went missing.
#[tauri::command]
pub async fn thread_eml_attachments(
    pool: tauri::State<'_, crate::db::ReadPool>,
    write_pool: tauri::State<'_, SqlitePool>,
    thread_ids: Vec<String>,
) -> CommandResult<Vec<OutgoingAttachmentInput>> {
    let pool = pool.inner().as_ref();
    let write_pool = write_pool.inner();
    collect_thread_eml_attachments(pool, thread_ids, |id| async move {
        super::fetch_remote_message_source(pool, write_pool, &id).await
    })
    .await
}

async fn collect_thread_eml_attachments<F, Fut>(
    pool: &SqlitePool,
    thread_ids: Vec<String>,
    mut source: F,
) -> CommandResult<Vec<OutgoingAttachmentInput>>
where
    F: FnMut(String) -> Fut,
    Fut: std::future::Future<Output = Result<Option<Vec<u8>>, String>>,
{
    let mut references = Vec::new();
    let mut attachments = Vec::with_capacity(thread_ids.len());
    for thread_id in dedupe(thread_ids) {
        let Some(message) = newest_message(pool, &thread_id)
            .await
            .map_err(CommandError::unexpected)?
        else {
            continue;
        };
        let message_id = message.reference.message_id.clone();
        match source(message_id.clone()).await {
            Ok(Some(raw)) => {
                attachments.push(eml_attachment(message.subject.as_deref(), &raw));
                references.push(message.reference);
            }
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(%message_id, %error, "could not fetch message source for .eml attachment");
            }
        }
    }
    // One snapshot for the complete collection: a later download may have
    // yielded while an earlier message was reset or replaced.
    let mut tx = pool.begin().await.map_err(CommandError::unexpected)?;
    ContentReference::ensure_many_current(&mut tx, &references).await?;
    Ok(attachments)
}

fn dedupe(thread_ids: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    thread_ids
        .into_iter()
        .filter(|id| seen.insert(id.clone()))
        .collect()
}

#[derive(sqlx::FromRow)]
struct EmlMessage {
    subject: Option<String>,
    #[sqlx(flatten)]
    reference: ContentReference,
}

async fn newest_message(
    pool: &SqlitePool,
    thread_id: &str,
) -> Result<Option<EmlMessage>, sqlx::Error> {
    sqlx::query_as(
        "SELECT m.subject, m.id AS message_id, m.account_id, m.folder_id,
         m.remote_id, m.remote_uidvalidity, m.created_at, a.provider,
         COALESCE(f.remote_id, f.full_path) AS source_folder, f.uidvalidity AS folder_uidvalidity
         FROM messages m JOIN accounts a ON a.id = m.account_id
         LEFT JOIN folders f ON f.id = m.folder_id
         WHERE m.thread_id = ? AND COALESCE(m.is_draft, 0) = 0
         ORDER BY m.date DESC, m.id DESC
         LIMIT 1",
    )
    .bind(thread_id)
    .fetch_optional(pool)
    .await
}

fn eml_attachment(subject: Option<&str>, raw: &[u8]) -> OutgoingAttachmentInput {
    OutgoingAttachmentInput {
        filename: eml_filename(subject),
        mime_type: "message/rfc822".to_string(),
        content_base64: base64::engine::general_purpose::STANDARD.encode(raw),
        content_id: None,
        inline: false,
    }
}

/// `<subject>.eml`, with path separators and characters Windows refuses
/// replaced, so the recipient can save the file under the name it arrives with.
fn eml_filename(subject: Option<&str>) -> String {
    // Folded subjects carry tabs and line breaks; they separate words, so they
    // become spaces before the control filter would drop them outright.
    let spaced: String = subject
        .unwrap_or_default()
        .chars()
        .map(|ch| if ch.is_whitespace() { ' ' } else { ch })
        .collect();
    let cleaned: String = strip_filename_controls(&spaced)
        .chars()
        .map(|ch| match ch {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            other => other,
        })
        .collect();
    // Leading dots hide the file; Windows drops trailing ones.
    let stem: String = cleaned
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_matches('.')
        .chars()
        .take(MAX_STEM_CHARS)
        .collect();
    let stem = stem.trim_end();
    if stem.is_empty() {
        "message.eml".to_string()
    } else {
        format!("{stem}.eml")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn collection_revalidates_earlier_sources_after_later_downloads() {
        for change in [
            "reset",
            "replacement",
            "missing",
            "local",
            "imported",
            "matching",
        ] {
            let pool = sqlx::sqlite::SqlitePoolOptions::new()
                .max_connections(1)
                .connect("sqlite::memory:")
                .await
                .unwrap();
            crate::db::init(&pool).await.unwrap();
            sqlx::raw_sql("INSERT INTO accounts (id, email, display_name, provider, config, credentials_ref, created_at, updated_at)
                VALUES ('a', 'me@example.test', 'Me', 'imap', '{}', '', '', '');
                INSERT INTO folders (id, account_id, name, full_path, folder_type, uidvalidity, created_at, updated_at)
                VALUES ('f', 'a', 'INBOX', 'INBOX', 'inbox', 7, '', '');
                INSERT INTO threads (id, account_id, subject, last_message_date, created_at, updated_at)
                VALUES ('t1', 'a', 'One', '', '', ''), ('t2', 'a', 'Two', '', '', '');
                INSERT INTO messages (id, account_id, folder_id, thread_id, remote_id, remote_uidvalidity, from_address, to_addresses, date, created_at, updated_at)
                VALUES ('m1', 'a', 'f', 't1', '41', 7, '', '[]', '', '', ''), ('m2', 'a', 'f', 't2', '42', 7, '', '[]', '', '', '');")
                .execute(&pool).await.unwrap();
            let raw = b"Subject: Original\r\n\r\nOriginal content\r\n";
            sqlx::query("UPDATE messages SET original_source = ?")
                .bind(raw.as_slice())
                .execute(&pool)
                .await
                .unwrap();
            if matches!(change, "local" | "imported") {
                sqlx::query("UPDATE messages SET remote_id = ?, remote_uidvalidity = NULL")
                    .bind(if change == "local" {
                        None
                    } else {
                        Some("import-550e8400-e29b-41d4-a716-446655440000")
                    })
                    .execute(&pool)
                    .await
                    .unwrap();
            }
            let result = collect_thread_eml_attachments(&pool, vec!["t1".into(), "t2".into(), "t1".into()], |id| {
                let pool = &pool;
                async move {
                    let source = super::super::fetch_remote_message_source(pool, pool, &id).await?;
                    if id == "m2" {
                        let sql = match change {
                            "reset" | "local" | "imported" => "UPDATE folders SET uidvalidity = 8",
                            "replacement" => "UPDATE messages SET remote_id = '43', created_at = 'replacement' WHERE id = 'm1'",
                            "missing" => "DELETE FROM messages WHERE id = 'm1'",
                            "matching" => "SELECT 1",
                            _ => unreachable!(),
                        };
                        sqlx::query(sql).execute(pool).await.unwrap();
                    }
                    Ok(source)
                }
            }).await;
            if matches!(change, "matching" | "local" | "imported") {
                let attachments = result.unwrap();
                assert_eq!(attachments.len(), 2);
                for attachment in attachments {
                    assert_eq!(
                        base64::engine::general_purpose::STANDARD
                            .decode(attachment.content_base64)
                            .unwrap(),
                        raw
                    );
                }
            } else {
                assert_eq!(
                    result.unwrap_err().key,
                    "backend.errors.imapUidValidityChanged"
                );
            }
        }
    }

    #[test]
    fn filename_keeps_the_subject_and_drops_path_characters() {
        assert_eq!(
            eml_filename(Some("Re: Rechnung 10/2026 <dringend>")),
            "Re_ Rechnung 10_2026 _dringend_.eml"
        );
    }

    #[test]
    fn filename_falls_back_when_the_subject_is_empty() {
        assert_eq!(eml_filename(None), "message.eml");
        assert_eq!(eml_filename(Some("  \u{200b} ")), "message.eml");
        assert_eq!(eml_filename(Some("...")), "message.eml");
    }

    #[test]
    fn filename_collapses_whitespace_and_caps_its_length() {
        assert_eq!(eml_filename(Some("a\t\tb\r\nc")), "a b c.eml");
        let long = "x".repeat(200);
        assert_eq!(
            eml_filename(Some(&long)).chars().count(),
            MAX_STEM_CHARS + 4
        );
    }

    #[test]
    fn attachment_carries_the_original_bytes() {
        let raw = b"Subject: hi\r\nContent-Transfer-Encoding: 8bit\r\n\r\n\xc3\xa4\r\n";
        let attachment = eml_attachment(Some("hi"), raw);
        assert_eq!(attachment.mime_type, "message/rfc822");
        assert!(!attachment.inline);
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(attachment.content_base64)
            .unwrap();
        assert_eq!(decoded, raw);
    }

    #[test]
    fn repeated_threads_attach_once() {
        assert_eq!(
            dedupe(vec!["a".into(), "b".into(), "a".into()]),
            vec!["a".to_string(), "b".to_string()]
        );
    }
}
