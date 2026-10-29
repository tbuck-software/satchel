//! Turn threads dragged onto the composer into `.eml` attachments.
//!
//! The original source is fetched the same way "show source" fetches it, so a
//! forwarded message keeps its headers, signatures and MIME parts byte for
//! byte. A thread contributes its newest delivered message: that one carries
//! the conversation's latest state, and a long thread would otherwise bury the
//! draft under a dozen near-identical files.

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
    thread_ids: Vec<String>,
) -> CommandResult<Vec<OutgoingAttachmentInput>> {
    let pool = pool.inner().as_ref();
    let mut attachments = Vec::with_capacity(thread_ids.len());
    for thread_id in dedupe(thread_ids) {
        let Some((message_id, subject)) = newest_message(pool, &thread_id)
            .await
            .map_err(CommandError::unexpected)?
        else {
            continue;
        };
        match super::fetch_remote_message_source(pool, &message_id).await {
            Ok(Some(raw)) => attachments.push(eml_attachment(subject.as_deref(), &raw)),
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(%message_id, %error, "could not fetch message source for .eml attachment");
            }
        }
    }
    Ok(attachments)
}

fn dedupe(thread_ids: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    thread_ids
        .into_iter()
        .filter(|id| seen.insert(id.clone()))
        .collect()
}

async fn newest_message(
    pool: &SqlitePool,
    thread_id: &str,
) -> Result<Option<(String, Option<String>)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT id, subject FROM messages
         WHERE thread_id = ? AND COALESCE(is_draft, 0) = 0
         ORDER BY date DESC, id DESC
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
