//! Links to a conversation from outside Satchel (`satchel://message/<id>`).
//!
//! The link carries the RFC 5322 Message-ID, not our thread id: thread ids are
//! minted locally and change with a rethreading or on another device, the
//! Message-ID is the same everywhere the mail exists.

use sqlx::SqlitePool;

use crate::db::threads::ThreadError;
use crate::utils::mime::decode_mime_encoded_words;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageLinkTarget {
    pub thread_id: String,
    pub subject: String,
    /// Without angle brackets. `None` when no message in the thread has a
    /// Message-ID that exists outside this database.
    pub message_id: Option<String>,
}

/// `<abc@host>` and `abc@host` name the same message.
pub fn normalize_message_id(value: &str) -> String {
    value
        .trim()
        .trim_start_matches('<')
        .trim_end_matches('>')
        .trim()
        .to_string()
}

/// Locally minted ids (`thread_resolution::derive_message_id_header`, draft
/// placeholders) mean nothing on another device. `@satchel.local` without a
/// subdomain is what we send out, so that one is real.
fn is_portable(header: &str) -> bool {
    let header = header.trim().to_ascii_lowercase();
    !normalize_message_id(&header).is_empty()
        && !header.ends_with(".satchel.local>")
        && !header.ends_with(".flowmail.local>")
        && !header.ends_with("@satchel.draft>")
}

/// The thread's first real message: it outlives replies and forwarded copies.
pub async fn link_target(
    pool: &SqlitePool,
    thread_id: &str,
) -> Result<MessageLinkTarget, ThreadError> {
    let subject =
        sqlx::query_scalar::<_, Option<String>>("SELECT subject FROM threads WHERE id = ?")
            .bind(thread_id)
            .fetch_optional(pool)
            .await?
            .ok_or(ThreadError::NotFound)?;

    let headers = sqlx::query_scalar::<_, String>(
        "SELECT message_id_header FROM messages
         WHERE thread_id = ? AND message_id_header IS NOT NULL AND COALESCE(is_draft, 0) = 0
         ORDER BY date ASC, id ASC",
    )
    .bind(thread_id)
    .fetch_all(pool)
    .await?;

    Ok(MessageLinkTarget {
        thread_id: thread_id.to_string(),
        subject: decode_mime_encoded_words(&subject.unwrap_or_default()),
        message_id: headers
            .iter()
            .find(|header| is_portable(header))
            .map(|header| normalize_message_id(header)),
    })
}

/// The thread holding `message_id`, preferring a copy outside trash and spam.
pub async fn resolve_message_id(
    pool: &SqlitePool,
    message_id: &str,
) -> Result<Option<String>, ThreadError> {
    let bare = normalize_message_id(message_id);
    if bare.is_empty() {
        return Ok(None);
    }
    // `account_id IN (…)` lets the lookup use idx_messages_account_msgid.
    let thread_id = sqlx::query_scalar::<_, String>(
        "SELECT m.thread_id FROM messages m
         JOIN threads t ON t.id = m.thread_id
         LEFT JOIN folders f ON f.id = m.folder_id
         WHERE m.account_id IN (SELECT id FROM accounts)
           AND m.message_id_header IN (?, ?)
         ORDER BY CASE WHEN LOWER(COALESCE(f.folder_type, '')) IN ('trash', 'spam', 'junk')
                       THEN 1 ELSE 0 END,
                  m.date DESC
         LIMIT 1",
    )
    .bind(format!("<{bare}>"))
    .bind(&bare)
    .fetch_optional(pool)
    .await?;
    Ok(thread_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
        let now = "2026-10-09T09:00:00Z";
        for statement in [
            "INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, kind, created_at, updated_at)
             VALUES ('acc', 'Work', 'me@example.com', 'imap', '{}', 'ref', 'email', ?1, ?1)",
            "INSERT INTO folders (id, account_id, name, full_path, folder_type, created_at, updated_at)
             VALUES ('inbox', 'acc', 'Inbox', 'INBOX', 'inbox', ?1, ?1),
                    ('trash', 'acc', 'Trash', 'Trash', 'trash', ?1, ?1)",
            "INSERT INTO threads (id, account_id, subject, last_message_date, workflow_state, created_at, updated_at)
             VALUES ('thr', 'acc', 'Invoice', ?1, 'inbox', ?1, ?1),
                    ('thr-trash', 'acc', 'Invoice', ?1, 'inbox', ?1, ?1),
                    ('thr-local', 'acc', 'No id', ?1, 'inbox', ?1, ?1)",
            "INSERT INTO messages (id, account_id, folder_id, thread_id, message_id_header, from_address, to_addresses, subject, date, created_at, updated_at)
             VALUES ('m1', 'acc', 'inbox', 'thr', '<root@mail.example.org>', 'a@example.org', '[]', 'Invoice', '2026-10-01T08:00:00Z', ?1, ?1),
                    ('m2', 'acc', 'inbox', 'thr', '<reply@mail.example.org>', 'a@example.org', '[]', 'Re: Invoice', '2026-10-02T08:00:00Z', ?1, ?1),
                    ('m3', 'acc', 'trash', 'thr-trash', '<root@mail.example.org>', 'a@example.org', '[]', 'Invoice', '2026-10-03T08:00:00Z', ?1, ?1),
                    ('m4', 'acc', 'inbox', 'thr-local', '<42@imap.satchel.local>', 'a@example.org', '[]', 'No id', '2026-10-01T08:00:00Z', ?1, ?1)",
        ] {
            sqlx::query(statement).bind(now).execute(&pool).await.unwrap();
        }
        pool
    }

    #[tokio::test]
    async fn links_to_the_first_message_without_brackets() {
        let pool = pool().await;
        let target = link_target(&pool, "thr").await.unwrap();
        assert_eq!(target.message_id.as_deref(), Some("root@mail.example.org"));
        assert_eq!(target.subject, "Invoice");
    }

    #[tokio::test]
    async fn locally_minted_ids_are_not_offered() {
        let pool = pool().await;
        assert_eq!(
            link_target(&pool, "thr-local").await.unwrap().message_id,
            None
        );
    }

    #[tokio::test]
    async fn unknown_thread_is_not_found() {
        let pool = pool().await;
        assert!(matches!(
            link_target(&pool, "nope").await,
            Err(ThreadError::NotFound)
        ));
    }

    #[tokio::test]
    async fn resolves_with_or_without_brackets_and_skips_trash() {
        let pool = pool().await;
        for input in [
            "root@mail.example.org",
            "<root@mail.example.org>",
            " <root@mail.example.org> ",
        ] {
            assert_eq!(
                resolve_message_id(&pool, input).await.unwrap().as_deref(),
                Some("thr"),
                "{input}"
            );
        }
        assert_eq!(resolve_message_id(&pool, "missing@x").await.unwrap(), None);
        assert_eq!(resolve_message_id(&pool, "<>").await.unwrap(), None);
    }
}
