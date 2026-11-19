//! Whether a message carries the paperclip: a part the reader lists as a file.
//!
//! Mirrors `listedAttachments` in `src/utils/messageAttachments.ts`; change
//! both together. Not files:
//! signature parts, calendar invites (the invite card shows them) and parts
//! the body shows at a `cid:` reference, such as signature logos. An inline
//! part nothing references is content the reader would otherwise never see,
//! so it counts. While no body is stored, an inline part with a Content-ID is
//! taken to be one the body shows.

use std::sync::LazyLock;

use futures::future::BoxFuture;
use sqlx::SqliteConnection;

use super::data_repairs::{DataRepair, Prepared};
use super::signed_attachments::HIDDEN_BESIDE_ROOT_SIGNATURE;

/// Whether attachment row `a` of message `m` is listed as a file.
const LISTED_PART: &str = "(COALESCE(a.content_type, '') NOT LIKE 'application/pgp-signature%'
      AND COALESCE(a.content_type, '') NOT LIKE '%pkcs7-signature%'
      AND COALESCE(a.filename, '') NOT LIKE '%.p7s'
      AND COALESCE(a.content_type, '') NOT LIKE 'text/calendar%'
      AND COALESCE(a.content_type, '') NOT LIKE 'application/ics%'
      AND COALESCE(a.filename, '') NOT LIKE '%.ics'
      AND NOT CASE
          WHEN m.body_html IS NULL AND m.body_text IS NULL
              THEN COALESCE(a.is_inline, 0) = 1
                   AND TRIM(COALESCE(a.content_id, ''), '<> ') != ''
          WHEN m.body_html IS NULL THEN 0
          ELSE (TRIM(COALESCE(a.content_id, ''), '<> ') != ''
                AND instr(lower(m.body_html), 'cid:' || lower(TRIM(a.content_id, '<> '))) > 0)
            OR (COALESCE(a.filename, '') != ''
                AND instr(lower(m.body_html), 'cid:' || lower(a.filename)) > 0)
      END)";

/// Sets the flag of every message matching `filter` that has attachment rows.
/// A message without rows keeps its flag: the provider listed no parts.
fn update_message_flags(filter: &str) -> String {
    format!(
        "UPDATE messages SET has_attachments = (
             SELECT EXISTS(SELECT 1 FROM attachments a
                           WHERE a.message_id = m.id
                             AND NOT {HIDDEN_BESIDE_ROOT_SIGNATURE}
                             AND {LISTED_PART})
             FROM messages m WHERE m.id = messages.id)
         WHERE {filter} AND EXISTS(SELECT 1 FROM attachments WHERE message_id = messages.id)"
    )
}

static UPDATE_ONE: LazyLock<String> = LazyLock::new(|| update_message_flags("id = ?"));
static UPDATE_RANGE: LazyLock<String> =
    LazyLock::new(|| update_message_flags("rowid > ? AND rowid <= ?"));

/// Recompute the flag of one message and of its thread from its stored parts
/// and body. Call it after either changed.
pub async fn refresh(
    connection: &mut SqliteConnection,
    message_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(&UPDATE_ONE)
        .bind(message_id)
        .execute(&mut *connection)
        .await?;
    sqlx::query(
        "UPDATE threads SET has_attachments =
             (SELECT COALESCE(MAX(has_attachments), 0) FROM messages WHERE thread_id = threads.id)
         WHERE id = (SELECT thread_id FROM messages WHERE id = ?)",
    )
    .bind(message_id)
    .execute(&mut *connection)
    .await?;
    Ok(())
}

/// Flags written while every stored part counted, inline logos included.
pub const LISTED_ATTACHMENT_FLAGS: DataRepair = DataRepair {
    name: "listed_attachment_flags",
    block: listed_attachment_flags_block,
    prepare: None,
};

fn listed_attachment_flags_block<'c>(
    connection: &'c mut SqliteConnection,
    cursor: &'c str,
    limit: i64,
    _prepared: Option<Prepared>,
) -> BoxFuture<'c, Result<Option<String>, sqlx::Error>> {
    Box::pin(async move {
        let after: i64 = cursor.parse().unwrap_or(0);
        let last: Option<i64> = sqlx::query_scalar(
            "SELECT MAX(rowid) FROM (SELECT rowid FROM messages WHERE rowid > ? ORDER BY rowid LIMIT ?)",
        )
        .bind(after)
        .bind(limit)
        .fetch_one(&mut *connection)
        .await?;
        let Some(last) = last else {
            return Ok(None);
        };
        sqlx::query(&UPDATE_RANGE)
            .bind(after)
            .bind(last)
            .execute(&mut *connection)
            .await?;
        sqlx::query(
            "UPDATE threads SET has_attachments =
                 (SELECT COALESCE(MAX(has_attachments), 0) FROM messages WHERE thread_id = threads.id)
             WHERE id IN (SELECT thread_id FROM messages m
                          WHERE m.rowid > ? AND m.rowid <= ?
                            AND EXISTS(SELECT 1 FROM attachments WHERE message_id = m.id))",
        )
        .bind(after)
        .bind(last)
        .execute(&mut *connection)
        .await?;
        Ok(Some(last.to_string()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::data_repairs::{is_pending, run_repair};
    use sqlx::SqlitePool;

    async fn pool() -> SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::db::init(&pool).await.unwrap();
        sqlx::raw_sql("INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
                VALUES ('a', 'Account', 'me@example.test', 'imap', '{}', '', '', '');
            INSERT INTO folders (id, account_id, name, full_path, folder_type, created_at, updated_at)
                VALUES ('f', 'a', 'Inbox', 'INBOX', 'inbox', '', '');")
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    /// A message flagged the old way, every stored part counted, in a thread of its own.
    async fn message(pool: &SqlitePool, id: &str, html: Option<&str>, text: Option<&str>) {
        sqlx::query(
            "INSERT INTO threads (id, account_id, last_message_date, has_attachments, created_at, updated_at)
             VALUES (?, 'a', '', 1, '', '')",
        )
        .bind(format!("t-{id}"))
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO messages (id, account_id, folder_id, thread_id, from_address, to_addresses,
                 date, body_html, body_text, has_attachments, created_at, updated_at)
             VALUES (?, 'a', 'f', ?, 'sender@example.test', '[]', '', ?, ?, 1, '', '')",
        )
        .bind(id)
        .bind(format!("t-{id}"))
        .bind(html)
        .bind(text)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn part(
        pool: &SqlitePool,
        message: &str,
        name: &str,
        content_type: &str,
        cid: Option<&str>,
    ) {
        sqlx::query(
            "INSERT INTO attachments (id, message_id, filename, content_type, content_id, is_inline, listing_rules, created_at)
             VALUES (?, ?, ?, ?, ?, ?, 1, '')",
        )
        .bind(format!("{message}-{name}"))
        .bind(message)
        .bind(name)
        .bind(content_type)
        .bind(cid)
        .bind(cid.is_some())
        .execute(pool)
        .await
        .unwrap();
    }

    async fn flags(pool: &SqlitePool, message: &str) -> (i64, i64) {
        sqlx::query_as(
            "SELECT m.has_attachments, t.has_attachments
             FROM messages m JOIN threads t ON t.id = m.thread_id WHERE m.id = ?",
        )
        .bind(message)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn seed(pool: &SqlitePool) {
        // An Outlook signature logo, shown where the body references it.
        message(
            pool,
            "logo",
            Some(r#"<p>Hi</p><img src="cid:image001.png@01DB1234">"#),
            None,
        )
        .await;
        part(
            pool,
            "logo",
            "image001.png",
            "image/png",
            Some("<image001.png@01DB1234>"),
        )
        .await;
        // GroupWise: screenshots inline, referenced nowhere.
        message(
            pool,
            "groupwise",
            Some("<div>Liebe Kolleginnen</div>"),
            None,
        )
        .await;
        part(
            pool,
            "groupwise",
            "bild1.png",
            "image/png",
            Some("<1795.316@th-koeln.de>"),
        )
        .await;
        // A file beside a logo.
        message(pool, "file", Some(r#"<img src="cid:logo@x">"#), None).await;
        part(pool, "file", "logo.png", "image/png", Some("<logo@x>")).await;
        part(pool, "file", "report.pdf", "application/pdf", None).await;
        // Machinery only: a signature and an invite.
        message(pool, "machinery", None, Some("Hi")).await;
        part(
            pool,
            "machinery",
            "smime.p7s",
            "application/pkcs7-signature",
            None,
        )
        .await;
        part(
            pool,
            "machinery",
            "invite.ics",
            "text/calendar; method=REQUEST",
            None,
        )
        .await;
        // No body yet: an inline part with a Content-ID waits for it.
        message(pool, "pending", None, None).await;
        part(
            pool,
            "pending",
            "logo.png",
            "image/png",
            Some("<pending@x>"),
        )
        .await;
        // The provider listed no parts; nothing to decide from.
        message(pool, "unlisted", None, Some("Hi")).await;
    }

    async fn assert_flags(pool: &SqlitePool) {
        assert_eq!(flags(pool, "logo").await, (0, 0));
        assert_eq!(flags(pool, "groupwise").await, (1, 1));
        assert_eq!(flags(pool, "file").await, (1, 1));
        assert_eq!(flags(pool, "machinery").await, (0, 0));
        assert_eq!(flags(pool, "pending").await, (0, 0));
        assert_eq!(flags(pool, "unlisted").await, (1, 1));
    }

    #[tokio::test]
    async fn the_paperclip_marks_only_parts_the_reader_lists_as_files() {
        let pool = pool().await;
        seed(&pool).await;
        let mut connection = pool.acquire().await.unwrap();
        for id in [
            "logo",
            "groupwise",
            "file",
            "machinery",
            "pending",
            "unlisted",
        ] {
            refresh(&mut connection, id).await.unwrap();
        }
        drop(connection);
        assert_flags(&pool).await;
    }

    #[tokio::test]
    async fn a_body_that_arrives_without_the_reference_lists_the_part() {
        let pool = pool().await;
        seed(&pool).await;
        sqlx::query(
            "UPDATE messages SET body_html = '<p>Screenshot below</p>' WHERE id = 'pending'",
        )
        .execute(&pool)
        .await
        .unwrap();
        let mut connection = pool.acquire().await.unwrap();
        refresh(&mut connection, "pending").await.unwrap();
        drop(connection);
        assert_eq!(flags(&pool, "pending").await, (1, 1));
    }

    #[tokio::test]
    async fn the_repair_recomputes_stored_flags() {
        let pool = pool().await;
        // Migration 0194 planted the marker.
        assert!(is_pending(&pool, &LISTED_ATTACHMENT_FLAGS).await.unwrap());
        seed(&pool).await;
        run_repair(&pool, &LISTED_ATTACHMENT_FLAGS, 2, None, |_| {})
            .await
            .unwrap();
        assert_flags(&pool).await;
        assert!(!is_pending(&pool, &LISTED_ATTACHMENT_FLAGS).await.unwrap());
    }
}
