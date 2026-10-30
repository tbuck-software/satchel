use sqlx::{FromRow, SqlitePool};

use crate::providers::traits::RemoteAttachment;
use crate::sync::attachment_fetch::attachment_bytes;

#[derive(Debug, FromRow)]
struct CalendarAttachmentRow {
    attachment_id: String,
    account_id: String,
    account_email: String,
    provider: String,
    config: String,
    message_id: String,
    message_remote_id: String,
    folder_remote_id: Option<String>,
    folder_path: String,
    filename: Option<String>,
    content_type: String,
    size: Option<i64>,
    content_id: Option<String>,
    is_inline: i64,
    remote_part_id: String,
    cached_path: Option<String>,
}

pub async fn backfill_calendar_invites_from_attachments(
    pool: &SqlitePool,
    limit: i64,
) -> Result<u64, String> {
    let limit = if limit > 0 {
        limit
    } else {
        DEFAULT_CALENDAR_SCAN_BATCH
    };
    let rows = sqlx::query_as::<_, CalendarAttachmentRow>(
        "SELECT
            a.id AS attachment_id,
            acc.id AS account_id,
            acc.email AS account_email,
            acc.provider AS provider,
            acc.config AS config,
            m.id AS message_id,
            COALESCE(a.remote_uid, m.remote_id, '') AS message_remote_id,
            f.remote_id AS folder_remote_id,
            f.full_path AS folder_path,
            a.filename AS filename,
            a.content_type AS content_type,
            a.size AS size,
            a.content_id AS content_id,
            a.is_inline AS is_inline,
            COALESCE(a.remote_part_id, '') AS remote_part_id,
            a.cached_path AS cached_path
         FROM attachments a
         JOIN messages m ON m.id = a.message_id
         JOIN accounts acc ON acc.id = m.account_id
         JOIN folders f ON f.id = m.folder_id
         WHERE acc.is_active = 1
           AND (
             LOWER(a.content_type) LIKE 'text/calendar%'
             OR LOWER(a.content_type) = 'application/ics'
             OR LOWER(COALESCE(a.filename, '')) LIKE '%.ics'
           )
           AND a.calendar_scan_at IS NULL
           -- Decrypted content stays off disk beyond its attachment cache;
           -- an invite row would keep it after the message is gone.
           AND a.id NOT LIKE ?
           AND NOT EXISTS (
             SELECT 1 FROM calendar_invites ci WHERE ci.message_id = m.id
           )
         ORDER BY m.date DESC
         LIMIT ?",
    )
    .bind(crate::email::decrypted::attachment_id_pattern())
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(|err| err.to_string())?;

    let mut imported = 0_u64;
    let mut remote_fetches = 0_u32;
    for row in rows {
        let attachment = RemoteAttachment {
            remote_part_id: row.remote_part_id.clone(),
            filename: row.filename.clone(),
            content_type: row.content_type.clone(),
            size: row.size,
            content_id: row.content_id.clone(),
            is_inline: row.is_inline > 0,
            content_base64: None,
        };

        let cached_bytes = if let Some(path) = row.cached_path.as_deref() {
            tokio::fs::read(path).await.ok()
        } else {
            None
        };

        let bytes = match cached_bytes {
            Some(bytes) => Some(bytes),
            None if row.message_remote_id.trim().is_empty()
                || row.remote_part_id.trim().is_empty() =>
            {
                None
            }
            // Hunting for invites is background curiosity, and it is competing
            // for the same provider budget as fetching mail. Cached copies are
            // free; downloads are rationed and stop the pass the moment the
            // provider pushes back, rather than earning a longer block one
            // attachment at a time.
            None if remote_fetches >= MAX_CALENDAR_SCAN_FETCHES => break,
            None => {
                remote_fetches += 1;
                let account_config = crate::utils::oauth::refresh_account_config(
                    pool,
                    &row.account_id,
                    &row.provider,
                    &row.config,
                )
                .await
                .map_err(|err| err.encoded())?;
                match crate::providers::gmail::as_background(attachment_bytes(
                    &row.provider,
                    &account_config,
                    &row.account_email,
                    &row.message_remote_id,
                    row.folder_remote_id.as_deref().unwrap_or(&row.folder_path),
                    &attachment,
                ))
                .await
                {
                    Ok(bytes) => Some(bytes),
                    Err(err) if err.is_rate_limited() => {
                        tracing::info!(
                            "calendar invite scan stopped: provider is rate limited ({err})"
                        );
                        break;
                    }
                    Err(_) => None,
                }
            }
        };

        // Marked whatever the outcome: an attachment that yields no invite is
        // not going to start yielding one, and re-fetching it on every launch
        // is what kept the provider busy for nothing.
        mark_calendar_scanned(pool, &row.attachment_id).await;

        let Some(bytes) = bytes else {
            continue;
        };
        let raw = String::from_utf8_lossy(&bytes).into_owned();
        let Some(invite) = crate::email::calendar::parse_invite(&raw) else {
            continue;
        };
        crate::db::calendar::CalendarRepository::new(pool)
            .upsert_invite(&row.account_id, Some(&row.message_id), &invite)
            .await
            .map_err(|error| error.to_string())?;
        imported += 1;
    }

    Ok(imported)
}

/// How many attachments one pass looks at.
const DEFAULT_CALENDAR_SCAN_BATCH: i64 = 200;

/// How many of those may be downloaded from the provider.
const MAX_CALENDAR_SCAN_FETCHES: u32 = 20;

async fn mark_calendar_scanned(pool: &SqlitePool, attachment_id: &str) {
    let _ = sqlx::query("UPDATE attachments SET calendar_scan_at = ? WHERE id = ?")
        .bind(chrono::Utc::now().to_rfc3339())
        .bind(attachment_id)
        .execute(pool)
        .await;
}

#[cfg(test)]
mod tests {
    use super::backfill_calendar_invites_from_attachments;
    use sqlx::sqlite::SqlitePoolOptions;

    /// The invite scan has to converge. It used to select every .ics
    /// attachment that had not produced an invite and re-fetch it from the
    /// provider on every launch — an attachment that yields nothing never
    /// stops qualifying, so the app opened each session by downloading the
    /// same attachments again, which is how it walked into a rate limit four
    /// seconds after starting.
    /// A migrated database with one account and its inbox.
    async fn pool_with_inbox() -> sqlx::SqlitePool {
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
             (id, display_name, email, provider, config, credentials_ref, is_active, created_at, updated_at)
             VALUES ('acc', 'Me', 'me@example.com', 'gmail', '{}', '', 1, ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("account");
        sqlx::query(
            "INSERT INTO folders
             (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at)
             VALUES ('inbox', 'acc', 'INBOX', 'INBOX', 'inbox', 'INBOX', ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("folder");
        pool
    }

    async fn insert_message(pool: &sqlx::SqlitePool, id: &str) {
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO messages
             (id, account_id, folder_id, thread_id, from_address, to_addresses, subject, date,
              created_at, updated_at)
             VALUES (?, 'acc', 'inbox', ?, 'you@example.com', '[]', 'Termin', ?, ?, ?)",
        )
        .bind(id)
        .bind(format!("th-{id}"))
        .bind(&now)
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await
        .expect("message");
    }

    #[tokio::test]
    async fn the_invite_scan_looks_at_an_attachment_once() {
        let pool = pool_with_inbox().await;
        let now = chrono::Utc::now().to_rfc3339();
        insert_message(&pool, "msg").await;
        // No cached copy and no remote id: nothing to read, nothing to import.
        sqlx::query(
            "INSERT INTO attachments
             (id, message_id, filename, content_type, is_inline, remote_part_id, created_at)
             VALUES ('att', 'msg', 'termin.ics', 'text/calendar', 0, '', ?)",
        )
        .bind(&now)
        .execute(&pool)
        .await
        .expect("attachment");

        let imported = backfill_calendar_invites_from_attachments(&pool, 0)
            .await
            .expect("scan");
        assert_eq!(imported, 0);

        let scanned: Option<String> =
            sqlx::query_scalar("SELECT calendar_scan_at FROM attachments WHERE id = 'att'")
                .fetch_one(&pool)
                .await
                .expect("marker");
        assert!(scanned.is_some(), "the attachment is marked as looked at");

        // Second pass: nothing left to consider, so nothing is fetched again.
        let again = backfill_calendar_invites_from_attachments(&pool, 0)
            .await
            .expect("second scan");
        assert_eq!(again, 0);
        let remaining: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM attachments WHERE calendar_scan_at IS NULL")
                .fetch_one(&pool)
                .await
                .expect("count");
        assert_eq!(remaining, 0);
    }

    /// Decrypted content is not kept at rest, and `calendar_invites` rows
    /// outlive their message. An invitation found in decrypted content stays
    /// a file of that message.
    #[tokio::test]
    async fn the_invite_scan_skips_parts_of_decrypted_content() {
        let pool = pool_with_inbox().await;
        let now = chrono::Utc::now().to_rfc3339();
        let cache = tempfile::tempdir().expect("cache dir");
        let ics = |uid: &str| {
            format!(
                "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Satchel//Test//EN\r\nMETHOD:REQUEST\r\n\
                 BEGIN:VEVENT\r\nUID:{uid}\r\nDTSTAMP:20261030T100000Z\r\n\
                 DTSTART:20261105T100000Z\r\nDTEND:20261105T110000Z\r\nSUMMARY:{uid}\r\n\
                 ORGANIZER;CN=Alice:mailto:alice@example.test\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
            )
        };
        for (message, attachment) in [
            ("plain", "att-plain".to_string()),
            (
                "encrypted",
                crate::email::decrypted::attachment_id("encrypted", "mime-1"),
            ),
        ] {
            insert_message(&pool, message).await;
            let path = cache.path().join(format!("{message}.ics"));
            std::fs::write(&path, ics(&format!("{message}@example.test"))).expect("cache file");
            sqlx::query(
                "INSERT INTO attachments
                 (id, message_id, filename, content_type, is_inline, cached_path, created_at)
                 VALUES (?, ?, 'invite.ics', 'text/calendar', 0, ?, ?)",
            )
            .bind(&attachment)
            .bind(message)
            .bind(path.to_string_lossy().to_string())
            .bind(&now)
            .execute(&pool)
            .await
            .expect("attachment");
        }

        let imported = backfill_calendar_invites_from_attachments(&pool, 0)
            .await
            .expect("scan");
        let invites: Vec<Option<String>> =
            sqlx::query_scalar("SELECT message_id FROM calendar_invites ORDER BY message_id")
                .fetch_all(&pool)
                .await
                .expect("invites");
        assert_eq!((imported, invites), (1, vec![Some("plain".to_string())]));
    }
}
