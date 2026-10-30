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
use crate::db::remote_content::{saved_uidvalidity, ContentReference};
use crate::providers::imap::is_legacy_truncated_body;

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
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ipc-types", ts(optional = nullable))]
    pub remote_uidvalidity: Option<u32>,
    pub body_text: String,
    pub body_html: Option<String>,
    /// False when the row already had a body and nothing was fetched.
    pub fetched: bool,
}

#[derive(sqlx::FromRow)]
struct BodyTarget {
    remote_id: Option<String>,
    remote_uidvalidity: Option<i64>,
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
    let reference = ContentReference::load(read, message_id).await?;
    let target = sqlx::query_as::<_, BodyTarget>(
        "SELECT m.remote_id, m.remote_uidvalidity,
                m.body_text, m.body_html, a.provider, a.id AS account_id, a.config
         FROM messages m
         JOIN accounts a ON a.id = m.account_id
         WHERE m.id = ? LIMIT 1",
    )
    .bind(message_id)
    .fetch_optional(read)
    .await
    .map_err(|err| err.to_string())?
    .ok_or_else(|| CommandError::new("backend.errors.messageNotAvailableLocally"))?;

    reference.validate_cached_read()?;
    let mut tx = read.begin().await.map_err(|e| e.to_string())?;
    reference.ensure_current(&mut tx).await?;
    tx.rollback().await.map_err(|e| e.to_string())?;
    if has_body(&target) {
        return Ok(LoadedMessageBody {
            remote_uidvalidity: saved_uidvalidity(target.remote_uidvalidity),
            body_text: target.body_text.unwrap_or_default(),
            body_html: target.body_html,
            fetched: false,
        });
    }
    reference.validate_cached_read()?;

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
                .fetch_message_body(&reference.message_ref()?)
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

    persist_loaded_body(
        write,
        &reference,
        &body_text,
        body_html.as_deref(),
        original_body_html.as_deref(),
    )
    .await?;

    Ok(LoadedMessageBody {
        remote_uidvalidity: reference.uidvalidity(),
        body_text,
        body_html,
        fetched: true,
    })
}

async fn persist_loaded_body(
    write: &SqlitePool,
    reference: &ContentReference,
    body_text: &str,
    body_html: Option<&str>,
    original_body_html: Option<&str>,
) -> Result<(), String> {
    let mut tx = reference.begin_write(write).await?;
    sqlx::query(
        "UPDATE messages
         SET body_text = ?, body_html = ?, original_body_html = ?, updated_at = ?
         WHERE id = ?",
    )
    .bind(body_text)
    .bind(body_html)
    .bind(original_body_html)
    .bind(chrono::Utc::now().to_rfc3339())
    .bind(&reference.message_id)
    .execute(&mut *tx)
    .await
    .map_err(|err| err.to_string())?;

    tx.commit().await.map_err(|err| err.to_string())?;
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
    let is_truncated_imap_body =
        crate::db::remote_content::is_remote_imap(&target.provider, target.remote_id.as_deref())
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
            remote_uidvalidity: Some(7),
            body_text: None,
            body_html,
            provider: provider.to_string(),
            account_id: "account".to_string(),
            config: String::new(),
        }
    }

    #[tokio::test]
    async fn body_writeback_requires_the_generation_that_was_fetched() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::db::init(&pool).await.unwrap();
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
        super::persist_loaded_body(
            &pool,
            &reference,
            "matching",
            Some("<p>matching</p>"),
            Some("<p>matching</p>"),
        )
        .await
        .unwrap();
        let body: String = sqlx::query_scalar("SELECT body_text FROM messages WHERE id = 'm'")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(body, "matching");
        let cached = super::load_message_body_with_pools(&pool, &pool, "m")
            .await
            .unwrap();
        assert_eq!(cached.body_text, "matching");
        assert_eq!(cached.remote_uidvalidity, Some(7));
        assert!(!cached.fetched);
        sqlx::raw_sql(
            "UPDATE folders SET uidvalidity = 8;
            UPDATE messages SET remote_uidvalidity = 8, body_text = 'replacement',
                body_html = '<p>replacement</p>', original_body_html = '<p>replacement</p>';",
        )
        .execute(&pool)
        .await
        .unwrap();
        let error = super::persist_loaded_body(
            &pool,
            &reference,
            "stale",
            Some("<p>stale</p>"),
            Some("<p>stale</p>"),
        )
        .await
        .unwrap_err();
        assert!(error.contains("imapUidValidityChanged"));
        let bodies: (String, String, String) = sqlx::query_as(
            "SELECT body_text, body_html, original_body_html FROM messages WHERE id = 'm'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            bodies,
            (
                "replacement".into(),
                "<p>replacement</p>".into(),
                "<p>replacement</p>".into()
            )
        );
        sqlx::query(
            "UPDATE messages SET body_text = 'stored stale bytes', body_html = '<p>stale</p>', remote_uidvalidity = NULL",
        )
        .execute(&pool)
        .await
        .unwrap();
        let error = super::load_message_body_with_pools(&pool, &pool, "m")
            .await
            .unwrap_err();
        assert!(error.to_string().contains("imapUidValidityMissing"));
        sqlx::query("UPDATE messages SET remote_uidvalidity = 7")
            .execute(&pool)
            .await
            .unwrap();
        let error = super::load_message_body_with_pools(&pool, &pool, "m")
            .await
            .unwrap_err();
        assert!(error.to_string().contains("imapUidValidityChanged"));
        for (provider, remote_id) in [
            ("imap", None),
            ("imap", Some("import-550e8400-e29b-41d4-a716-446655440000")),
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
            let cached = super::load_message_body_with_pools(&pool, &pool, "m")
                .await
                .unwrap();
            assert_eq!(cached.body_text, "stored stale bytes");
            assert!(!cached.fetched);
        }
    }

    #[test]
    fn legacy_truncated_imap_body_is_refetched_but_kept_for_other_providers() {
        let at_old_limit = Some("x".repeat(64 * 1024));

        assert!(!has_body(&target("imap", at_old_limit.clone())));
        assert!(has_body(&target("gmail", at_old_limit.clone())));
        for remote_id in [
            None,
            Some("import-550e8400-e29b-41d4-a716-446655440000".to_string()),
        ] {
            let mut local = target("imap", at_old_limit.clone());
            local.remote_id = remote_id;
            assert!(has_body(&local));
        }
        assert!(has_body(&target("imap", Some("complete".to_string()))));
    }
}
