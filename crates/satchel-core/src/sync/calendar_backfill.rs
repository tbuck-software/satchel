use sqlx::{FromRow, SqlitePool};

use crate::db::remote_content::ContentReference;
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
    remote_uidvalidity: Option<i64>,
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
            COALESCE(m.remote_id, '') AS message_remote_id,
            f.remote_id AS folder_remote_id,
            f.full_path AS folder_path,
            m.remote_uidvalidity,
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
           AND NOT EXISTS (
             SELECT 1 FROM calendar_invites ci WHERE ci.message_id = m.id
           )
         ORDER BY m.date DESC
         LIMIT ?",
    )
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(|err| err.to_string())?;

    let mut imported = 0_u64;
    let mut remote_fetches = 0_u32;
    for row in rows {
        let Ok(reference) = ContentReference::load(pool, &row.message_id).await else {
            continue;
        };
        if reference.account_id != row.account_id
            || reference.provider != row.provider
            || reference.remote_id.as_deref().unwrap_or_default() != row.message_remote_id
            || reference.remote_uidvalidity != row.remote_uidvalidity
            || (row.provider == "imap"
                && reference.source_folder.as_deref()
                    != Some(row.folder_remote_id.as_deref().unwrap_or(&row.folder_path)))
        {
            continue;
        }
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
            if reference.owns_attachment_cache(path, &row.attachment_id) {
                tokio::fs::read(path).await.ok()
            } else {
                None
            }
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
                let account_config =
                    crate::accounts::decrypt_account_config(&row.account_id, &row.config)?;
                match crate::providers::gmail::as_background(attachment_bytes(
                    &row.provider,
                    &account_config,
                    &row.account_email,
                    &row.message_remote_id,
                    row.folder_remote_id.as_deref().unwrap_or(&row.folder_path),
                    row.remote_uidvalidity
                        .and_then(|value| u32::try_from(value).ok()),
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

        let invite = bytes.as_deref().and_then(|bytes| {
            crate::email::calendar::parse_invite(&String::from_utf8_lossy(bytes))
        });
        // Marked whatever the outcome: an attachment that yields no invite is
        // not going to start yielding one, and re-fetching it on every launch
        // is what kept the provider busy for nothing.
        match mark_calendar_scanned(pool, &row.attachment_id, &reference, invite.as_ref()).await {
            Ok(true) => {}
            Ok(false) => continue,
            Err(error) => {
                tracing::debug!(%error, "skipping invite scan after message reference changed");
                continue;
            }
        }

        imported += u64::from(invite.is_some());
    }

    Ok(imported)
}

/// How many attachments one pass looks at.
const DEFAULT_CALENDAR_SCAN_BATCH: i64 = 200;

/// How many of those may be downloaded from the provider.
const MAX_CALENDAR_SCAN_FETCHES: u32 = 20;

async fn mark_calendar_scanned(
    pool: &SqlitePool,
    attachment_id: &str,
    reference: &ContentReference,
    invite: Option<&crate::email::calendar::ParsedInvite>,
) -> Result<bool, String> {
    let mut tx = reference.begin_write(pool).await?;
    let result =
        sqlx::query("UPDATE attachments SET calendar_scan_at = ? WHERE id = ? AND message_id = ?")
            .bind(chrono::Utc::now().to_rfc3339())
            .bind(attachment_id)
            .bind(&reference.message_id)
            .execute(&mut *tx)
            .await
            .map_err(|error| error.to_string())?;
    if result.rows_affected() != 1 {
        return Ok(false);
    }
    if let Some(invite) = invite {
        crate::db::calendar::CalendarRepository::upsert_invite_on_connection(
            &mut tx,
            &reference.account_id,
            Some(&reference.message_id),
            invite,
        )
        .await
        .map_err(|error| error.to_string())?;
    }
    tx.commit().await.map_err(|error| error.to_string())?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::backfill_calendar_invites_from_attachments;
    use sqlx::sqlite::SqlitePoolOptions;

    #[tokio::test]
    async fn an_old_scan_cannot_mark_a_reingested_generation() {
        let pool = SqlitePoolOptions::new()
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
                VALUES ('m', 'a', 'f', '42', 7, 'bob@example.test', '[]', '', '', '');
            INSERT INTO attachments (id, message_id, content_type, created_at) VALUES ('att', 'm', 'text/calendar', '');")
            .execute(&pool).await.unwrap();
        let reference = crate::db::remote_content::ContentReference::load(&pool, "m")
            .await
            .unwrap();
        let invite = crate::email::calendar::parse_invite("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:generation-test\r\nDTSTART:20261029T120000Z\r\nDTEND:20261029T130000Z\r\nSUMMARY:Original invite\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n").unwrap();
        assert!(super::mark_calendar_scanned(&pool, "att", &reference, None)
            .await
            .unwrap());
        sqlx::raw_sql(
            "UPDATE folders SET uidvalidity = 8; UPDATE messages SET remote_uidvalidity = 8;
            UPDATE attachments SET calendar_scan_at = NULL;",
        )
        .execute(&pool)
        .await
        .unwrap();
        assert!(
            super::mark_calendar_scanned(&pool, "att", &reference, Some(&invite))
                .await
                .unwrap_err()
                .contains("imapUidValidityChanged")
        );
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM calendar_invites")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(
            count, 0,
            "an old attachment cannot publish against a replacement message"
        );
        let marker: Option<String> =
            sqlx::query_scalar("SELECT calendar_scan_at FROM attachments WHERE id = 'att'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(marker, None);
        let current = crate::db::remote_content::ContentReference::load(&pool, "m")
            .await
            .unwrap();
        assert!(
            super::mark_calendar_scanned(&pool, "att", &current, Some(&invite))
                .await
                .unwrap()
        );
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM calendar_invites WHERE message_id = 'm'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(count, 1);
    }

    /// The invite scan has to converge. It used to select every .ics
    /// attachment that had not produced an invite and re-fetch it from the
    /// provider on every launch — an attachment that yields nothing never
    /// stops qualifying, so the app opened each session by downloading the
    /// same attachments again, which is how it walked into a rate limit four
    /// seconds after starting.
    #[tokio::test]
    async fn the_invite_scan_looks_at_an_attachment_once() {
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
        sqlx::query(
            "INSERT INTO messages
             (id, account_id, folder_id, thread_id, from_address, to_addresses, subject, date,
              created_at, updated_at)
             VALUES ('msg', 'acc', 'inbox', 'th', 'you@example.com', '[]', 'Termin', ?, ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("message");
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
}
