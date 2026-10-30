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
    let config = crate::utils::oauth::refresh_account_config(
        write,
        &target.account_id,
        &target.provider,
        &target.config,
    )
    .await?;

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

    store_fetched_body(
        write,
        message_id,
        &body_text,
        body_html.as_deref(),
        original_body_html.as_deref(),
    )
    .await
    .map_err(|err| err.to_string())?;

    Ok(LoadedMessageBody {
        body_text,
        body_html,
        fetched: true,
    })
}

/// Write a body fetched on its own into the existing row.
///
/// The row's signature bundle is not refetched with it, so the body is no
/// longer the one the strict splitter wrote with that bundle: the binding mark
/// goes, and a signature verdict shows the bundle's own entity from now on.
async fn store_fetched_body(
    write: &SqlitePool,
    message_id: &str,
    body_text: &str,
    body_html: Option<&str>,
    original_body_html: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE messages
         SET body_text = ?, body_html = ?, original_body_html = ?, updated_at = ?,
             trust_metadata = CASE WHEN json_valid(trust_metadata)
                 THEN json_set(trust_metadata, '$.signed_binding', json('false'))
                 ELSE trust_metadata END
         WHERE id = ?",
    )
    .bind(body_text)
    .bind(body_html)
    .bind(original_body_html)
    .bind(chrono::Utc::now().to_rfc3339())
    .bind(message_id)
    .execute(write)
    .await?;
    Ok(())
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
    use super::{has_body, store_fetched_body, BodyTarget};

    #[tokio::test]
    async fn a_body_fetched_on_its_own_loses_the_signed_binding() {
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
            "INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, created_at, updated_at) VALUES ('a', 'Account', 'me@example.test', 'imap', '{}', '', '', '')",
            "INSERT INTO folders (id, account_id, name, full_path, folder_type, created_at, updated_at) VALUES ('f', 'a', 'Inbox', 'Inbox', 'inbox', '', '')",
            r#"INSERT INTO messages (id, account_id, folder_id, from_address, to_addresses, date, trust_metadata, created_at, updated_at) VALUES ('signed', 'a', 'f', 'a@example.test', '[]', '', '{"root_crypto":true,"signed_binding":true}', '', ''), ('plain', 'a', 'f', 'a@example.test', '[]', '', NULL, '', '')"#,
        ] {
            sqlx::query(sql).execute(&pool).await.unwrap();
        }

        for id in ["signed", "plain"] {
            store_fetched_body(&pool, id, "Fetched", None, None)
                .await
                .unwrap();
        }
        let signed: String =
            sqlx::query_scalar("SELECT trust_metadata FROM messages WHERE id = 'signed'")
                .fetch_one(&pool)
                .await
                .unwrap();
        let trust: crate::providers::traits::MessageTrust = serde_json::from_str(&signed).unwrap();
        assert!(trust.root_crypto);
        assert!(!trust.signed_binding);
        let plain: (String, Option<String>) =
            sqlx::query_as("SELECT body_text, trust_metadata FROM messages WHERE id = 'plain'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(plain, ("Fetched".to_string(), None));
    }

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
