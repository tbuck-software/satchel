//! Fetch the body of a message the sync stored without one.
//!
//! A mailbox kept in full costs what a mailbox costs, which on a phone is the
//! wrong trade. The plan is to store envelopes for everything and bodies only
//! within a window, and this is the half that has to exist first: without a way
//! to fetch a body on demand, a window would turn old mail into blank pages.
//!
//! Only the two body columns are written. Subject, sender, date and threading
//! are already correct in the row — they came from the header the sync did
//! keep — so re-ingesting the message would be a second copy of work already
//! done, through the part of the pipeline that is hardest to get right.

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::commands::error::{CommandError, CommandResult};
use crate::providers::imap::is_legacy_truncated_body;
use crate::providers::traits::MessageRef;

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/message_body/", optional_fields = nullable))]
pub struct LoadMessageBodyInput {
    pub message_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/message_body/")
)]
pub struct LoadedMessageBody {
    pub body_text: String,
    pub body_html: Option<String>,
    /// False when the row already had a body and nothing was fetched.
    pub fetched: bool,
}

#[derive(sqlx::FromRow)]
struct BodyTarget {
    remote_id: Option<String>,
    folder_remote_id: Option<String>,
    body_text: Option<String>,
    body_html: Option<String>,
    provider: String,
    account_id: String,
    config: String,
}

#[tauri::command]
pub async fn load_message_body(
    pool: tauri::State<'_, crate::db::ReadPool>,
    write_pool: tauri::State<'_, SqlitePool>,
    input: LoadMessageBodyInput,
) -> CommandResult<LoadedMessageBody> {
    load_message_body_with_pools(pool.inner().as_ref(), write_pool.inner(), &input.message_id).await
}

pub(crate) async fn load_message_body_with_pools(
    read: &SqlitePool,
    write: &SqlitePool,
    message_id: &str,
) -> CommandResult<LoadedMessageBody> {
    let target = sqlx::query_as::<_, BodyTarget>(
        "SELECT m.remote_id, f.remote_id AS folder_remote_id,
                m.body_text, m.body_html, a.provider, a.id AS account_id, a.config
         FROM messages m
         JOIN accounts a ON a.id = m.account_id
         LEFT JOIN folders f ON f.id = m.folder_id
         WHERE m.id = ? LIMIT 1",
    )
    .bind(message_id)
    .fetch_optional(read)
    .await
    .map_err(|err| err.to_string())?
    .ok_or_else(|| CommandError::new("backend.errors.messageNotAvailableLocally"))?;

    if has_body(&target) {
        return Ok(LoadedMessageBody {
            body_text: target.body_text.unwrap_or_default(),
            body_html: target.body_html,
            fetched: false,
        });
    }

    let remote_id = target
        .remote_id
        .clone()
        .filter(|id| !id.is_empty())
        .ok_or_else(|| CommandError::new("backend.errors.messageNotAvailableRemotely"))?;
    let config =
        crate::commands::accounts::decrypt_account_config(&target.account_id, &target.config)?;

    let (body_text, body_html) = match target.provider.as_str() {
        "imap" | "pop3" => {
            let provider = crate::providers::imap::ImapProvider::from_account_config(&config, "")?;
            let parsed = provider
                .fetch_message_body(&MessageRef {
                    remote_id,
                    source_folder: target.folder_remote_id.clone(),
                    source_uidvalidity: None,
                })
                .await?;
            (parsed.text.unwrap_or_default(), parsed.html)
        }
        "gmail" => {
            let envelope = crate::providers::gmail::GmailProvider::from_account_config(&config)
                .fetch_message_by_id(&remote_id)
                .await?;
            (envelope.body_text.unwrap_or_default(), envelope.body_html)
        }
        "outlook" => {
            let (envelope, _folder) =
                crate::providers::outlook::OutlookProvider::from_account_config(&config)
                    .fetch_message_by_id(&remote_id)
                    .await?;
            (envelope.body_text.unwrap_or_default(), envelope.body_html)
        }
        other => {
            return Err(
                CommandError::new("backend.errors.messageBodyProviderUnsupported")
                    .with_param("provider", other),
            )
        }
    };

    // Sanitize at ingest; body reads repeat the pass as defence in depth.
    let original_body_html = body_html;
    let body_html =
        crate::email::sanitizer::sanitize_optional_email_html(original_body_html.clone())
            .await
            .map_err(|error| error.to_string())?;

    sqlx::query(
        "UPDATE messages
         SET body_text = ?, body_html = ?, original_body_html = ?, updated_at = ?
         WHERE id = ?",
    )
    .bind(&body_text)
    .bind(&body_html)
    .bind(&original_body_html)
    .bind(chrono::Utc::now().to_rfc3339())
    .bind(message_id)
    .execute(write)
    .await
    .map_err(|err| err.to_string())?;

    Ok(LoadedMessageBody {
        body_text,
        body_html,
        fetched: true,
    })
}

/// Whether the row already carries something worth showing.
///
/// A stored empty string counts as absent: the header-only path writes exactly
/// that, and treating it as a real body is how an empty page becomes permanent.
fn has_body(target: &BodyTarget) -> bool {
    let has_content = target
        .body_text
        .as_deref()
        .is_some_and(|text| !text.trim().is_empty())
        || target
            .body_html
            .as_deref()
            .is_some_and(|html| !html.trim().is_empty());
    let is_truncated_imap_body = target.provider == "imap"
        && (is_legacy_truncated_body(target.body_text.as_deref())
            || is_legacy_truncated_body(target.body_html.as_deref()));

    has_content && !is_truncated_imap_body
}

#[cfg(test)]
mod tests {
    use super::{has_body, BodyTarget};

    fn target(provider: &str, body_html: Option<String>) -> BodyTarget {
        BodyTarget {
            remote_id: Some("42".to_string()),
            folder_remote_id: Some("INBOX".to_string()),
            body_text: None,
            body_html,
            provider: provider.to_string(),
            account_id: "account".to_string(),
            config: String::new(),
        }
    }

    #[test]
    fn legacy_truncated_imap_body_is_refetched_but_kept_for_other_providers() {
        let at_old_limit = Some("x".repeat(64 * 1024));

        assert!(!has_body(&target("imap", at_old_limit.clone())));
        assert!(has_body(&target("gmail", at_old_limit)));
        assert!(has_body(&target("imap", Some("complete".to_string()))));
    }
}
