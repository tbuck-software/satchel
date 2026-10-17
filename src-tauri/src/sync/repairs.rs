use std::collections::HashSet;

use sqlx::{FromRow, SqlitePool};

use crate::sync::thread_resolution::{refresh_provider_thread_projection, sanitize_id};

#[derive(Debug, FromRow)]
struct ImapThreadRepairRow {
    message_id: String,
    account_id: String,
    old_thread_id: String,
    thread_key: String,
}

pub async fn repair_imap_thread_id_collisions(pool: &SqlitePool) -> Result<u64, String> {
    const REPAIR_KEY: &str = "repair.imap_thread_id_collisions_v2_at";
    let already_repaired =
        sqlx::query_scalar::<_, String>("SELECT value FROM app_settings WHERE key = ?")
            .bind(REPAIR_KEY)
            .fetch_optional(pool)
            .await
            .map_err(|err| err.to_string())?
            .is_some_and(|value| !value.trim().is_empty());
    if already_repaired {
        return Ok(0);
    }

    let rows = sqlx::query_as::<_, ImapThreadRepairRow>(
        "WITH imap_messages AS (
            SELECT
                m.id AS message_id,
                m.account_id AS account_id,
                m.thread_id AS old_thread_id,
                COALESCE(
                    NULLIF(TRIM(
                        CASE
                            WHEN LOWER(COALESCE(m.message_id_header, '')) LIKE '%@imap.satchel.local>' OR LOWER(COALESCE(m.message_id_header, '')) LIKE '%@imap.flowmail.local>' THEN ''
                            ELSE COALESCE(m.message_id_header, '')
                        END
                    ), ''),
                    COALESCE(NULLIF(TRIM(f.remote_id), ''), m.folder_id) || ':' || COALESCE(NULLIF(TRIM(m.remote_id), ''), m.id)
                ) AS thread_key
            FROM messages m
            JOIN accounts acc ON acc.id = m.account_id
            JOIN folders f ON f.id = m.folder_id
            WHERE acc.provider = 'imap'
              AND m.id LIKE 'm-imap-%'
              AND m.thread_id LIKE 'th-imap-%'
         ),
         colliding_threads AS (
            SELECT old_thread_id
            FROM imap_messages
            GROUP BY old_thread_id
            HAVING COUNT(DISTINCT thread_key) > 1
         )
         SELECT message_id, account_id, old_thread_id, thread_key
         FROM imap_messages
         WHERE old_thread_id IN (SELECT old_thread_id FROM colliding_threads)",
    )
    .fetch_all(pool)
    .await
    .map_err(|err| err.to_string())?;

    let mut moved = 0_u64;
    let mut old_thread_ids = HashSet::new();
    let mut touched_thread_ids = HashSet::new();

    for row in rows {
        let new_thread_id = format!(
            "th-imap-{}-{}",
            sanitize_id(&row.account_id),
            sanitize_id(&row.thread_key)
        );
        if new_thread_id == row.old_thread_id {
            continue;
        }

        sqlx::query(
            "INSERT OR IGNORE INTO threads (
                id, account_id, subject, last_message_date, message_count, unread_count,
                has_attachments, participants, snippet, workflow_state, workflow_changed_at,
                snooze_until, due_date, priority, board_column, board_position, category,
                created_at, updated_at, is_muted, is_starred
             )
             SELECT
                ?, account_id, subject, last_message_date, message_count, unread_count,
                has_attachments, participants, snippet, workflow_state, workflow_changed_at,
                snooze_until, due_date, priority, board_column, board_position, category,
                created_at, updated_at, is_muted, is_starred
             FROM threads
             WHERE id = ?",
        )
        .bind(&new_thread_id)
        .bind(&row.old_thread_id)
        .execute(pool)
        .await
        .map_err(|err| err.to_string())?;

        let result = sqlx::query("UPDATE messages SET thread_id = ?, updated_at = ? WHERE id = ?")
            .bind(&new_thread_id)
            .bind(chrono::Utc::now().to_rfc3339())
            .bind(&row.message_id)
            .execute(pool)
            .await
            .map_err(|err| err.to_string())?;

        if result.rows_affected() > 0 {
            moved += result.rows_affected();
            old_thread_ids.insert(row.old_thread_id);
            touched_thread_ids.insert(new_thread_id);
        }
    }

    for thread_id in touched_thread_ids.iter().chain(old_thread_ids.iter()) {
        refresh_provider_thread_projection(pool, thread_id).await?;
    }

    // This is a migration for thread ids created by old builds, not ongoing
    // reconciliation. Running it again after `merge_reply_split_threads`
    // mistakes a legitimate reply chain (naturally containing distinct
    // Message-IDs) for the old collision and splits it, after which the next
    // repair immediately joins it again. Mark a successful pass even when it
    // found nothing so startup remains idempotent.
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO app_settings (key, value, updated_at)
         VALUES (?, ?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
    )
    .bind(REPAIR_KEY)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|err| err.to_string())?;

    Ok(moved)
}

/// A completed remote snapshot is authoritative once no move for the account
/// is still queued. Older Satchel versions could persist `done` locally while
/// leaving the Gmail/IMAP INBOX copy in place; those threads then disappeared
/// only in Satchel. Resurface them locally instead of silently issuing a new
/// remote archive operation months later.
pub(crate) async fn repair_stale_done_threads_in_inbox(
    pool: &SqlitePool,
    account_id: &str,
) -> Result<u64, String> {
    let remote_mutation_pending: i64 = sqlx::query_scalar(
        "SELECT COUNT(1) FROM sync_queue
         WHERE account_id = ?
           AND action_type = 'remote_mutation'
           AND status IN ('pending', 'sending', 'scheduled')",
    )
    .bind(account_id)
    .fetch_one(pool)
    .await
    .map_err(|err| err.to_string())?;
    if remote_mutation_pending > 0 {
        return Ok(0);
    }

    let now = chrono::Utc::now().to_rfc3339();
    let changed = sqlx::query(
        "UPDATE threads
         SET workflow_state = 'inbox', board_column = 'inbox',
             snooze_until = NULL, workflow_changed_at = ?, updated_at = ?
         WHERE account_id = ?
           AND workflow_state = 'done'
           AND EXISTS (
             SELECT 1 FROM messages
             JOIN folders ON folders.id = messages.folder_id
             WHERE messages.thread_id = threads.id
               AND (
                 LOWER(TRIM(COALESCE(folders.remote_id, ''))) = 'inbox'
                 OR LOWER(TRIM(folders.name)) = 'inbox'
               )
           )",
    )
    .bind(&now)
    .bind(&now)
    .bind(account_id)
    .execute(pool)
    .await
    .map_err(|err| err.to_string())?
    .rows_affected();
    Ok(changed)
}

/// Drop the synthetic ARCHIVE copy of a Gmail message that also lives in a
/// real label folder.
///
/// Until the synthetic archive was tied to "no real label left", a message
/// whose only label was one the user made counted as having no primary mailbox
/// and was stored twice — once under its label, once under ARCHIVE. Nothing
/// clears those rows on its own: the reconciler only deletes copies whose
/// folder is missing from the current label set, and ARCHIVE was in it.
///
/// Runs once per account; the marker is what keeps a full-sync loop from
/// walking every message again.
pub(crate) async fn repair_gmail_archive_duplicates(
    pool: &SqlitePool,
    account_id: &str,
) -> Result<u64, String> {
    let repair_key = format!("repair.gmail_archive_duplicates_at:{account_id}");
    let already_repaired =
        sqlx::query_scalar::<_, String>("SELECT value FROM app_settings WHERE key = ?")
            .bind(&repair_key)
            .fetch_optional(pool)
            .await
            .map_err(|err| err.to_string())?
            .is_some_and(|value| !value.trim().is_empty());
    if already_repaired {
        return Ok(0);
    }

    let duplicates = sqlx::query_as::<_, (String, String)>(
        "SELECT duplicate.id, duplicate.thread_id
         FROM messages duplicate
         JOIN folders archive
           ON archive.id = duplicate.folder_id
          AND archive.account_id = duplicate.account_id
          AND UPPER(TRIM(COALESCE(archive.remote_id, ''))) = 'ARCHIVE'
         WHERE duplicate.account_id = ?
           AND EXISTS (
             SELECT 1
             FROM messages other
             JOIN folders label ON label.id = other.folder_id
             WHERE other.account_id = duplicate.account_id
               AND other.remote_id = duplicate.remote_id
               AND other.id <> duplicate.id
               AND UPPER(TRIM(COALESCE(label.remote_id, ''))) <> 'ARCHIVE'
           )",
    )
    .bind(account_id)
    .fetch_all(pool)
    .await
    .map_err(|err| err.to_string())?;

    let now = chrono::Utc::now().to_rfc3339();
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|err| err.to_string())?;
    for (message_id, _) in &duplicates {
        sqlx::query("DELETE FROM attachments WHERE message_id = ?")
            .bind(message_id)
            .execute(&mut *tx)
            .await
            .map_err(|err| err.to_string())?;
        sqlx::query("DELETE FROM messages WHERE id = ?")
            .bind(message_id)
            .execute(&mut *tx)
            .await
            .map_err(|err| err.to_string())?;
    }
    sqlx::query(
        "INSERT INTO app_settings (key, value, updated_at) VALUES (?, ?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
    )
    .bind(&repair_key)
    .bind(&now)
    .bind(&now)
    .execute(&mut *tx)
    .await
    .map_err(|err| err.to_string())?;
    tx.commit().await.map_err(|err| err.to_string())?;

    let touched = duplicates
        .iter()
        .map(|(_, thread_id)| thread_id.clone())
        .collect::<HashSet<_>>();
    for thread_id in &touched {
        refresh_provider_thread_projection(pool, thread_id).await?;
    }
    Ok(duplicates.len() as u64)
}

#[cfg(test)]
mod tests {
    use super::{repair_gmail_archive_duplicates, repair_imap_thread_id_collisions};
    use sqlx::sqlite::SqlitePoolOptions;

    #[tokio::test]
    async fn legacy_imap_collision_repair_does_not_resplit_merged_replies() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("pool");
        sqlx::migrate!("../crates/satchel-core/src/db/migrations")
            .run(&pool)
            .await
            .expect("migrations");
        for statement in [
            "INSERT INTO accounts
             (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
             VALUES ('account', 'Me', 'me@example.com', 'imap', '{}', '', '2026-01-01', '2026-01-01')",
            "INSERT INTO folders
             (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at)
             VALUES ('inbox', 'account', 'INBOX', 'INBOX', 'inbox', 'INBOX', '2026-01-01', '2026-01-01')",
            "INSERT INTO threads
             (id, account_id, subject, last_message_date, created_at, updated_at)
             VALUES ('th-imap-account-collision', 'account', 'Subject', '2026-01-02', '2026-01-01', '2026-01-01')",
            "INSERT INTO messages
             (id, account_id, folder_id, thread_id, message_id_header, from_address,
              to_addresses, subject, date, created_at, updated_at)
             VALUES
             ('m-imap-account-INBOX-1', 'account', 'inbox', 'th-imap-account-collision', '<one@example.com>',
              'sender@example.com', '[]', 'Subject', '2026-01-01', '2026-01-01', '2026-01-01'),
             ('m-imap-account-INBOX-2', 'account', 'inbox', 'th-imap-account-collision', '<two@example.com>',
              'sender@example.com', '[]', 'Re: Subject', '2026-01-02', '2026-01-01', '2026-01-01')",
        ] {
            sqlx::query(statement).execute(&pool).await.expect("seed");
        }

        assert_eq!(
            repair_imap_thread_id_collisions(&pool)
                .await
                .expect("initial repair"),
            2
        );

        // Recreate the state produced by reply-chain merging. The legacy
        // migration must not split this conversation again on the next start.
        let merged_thread: String =
            sqlx::query_scalar("SELECT thread_id FROM messages ORDER BY id LIMIT 1")
                .fetch_one(&pool)
                .await
                .expect("surviving thread");
        sqlx::query("UPDATE messages SET thread_id = ?")
            .bind(&merged_thread)
            .execute(&pool)
            .await
            .expect("simulate merge");

        assert_eq!(
            repair_imap_thread_id_collisions(&pool)
                .await
                .expect("repeat repair"),
            0
        );
        let thread_count: i64 =
            sqlx::query_scalar("SELECT COUNT(DISTINCT thread_id) FROM messages")
                .fetch_one(&pool)
                .await
                .expect("thread count");
        assert_eq!(thread_count, 1);
    }

    #[tokio::test]
    async fn a_labelled_gmail_message_loses_its_synthetic_archive_copy_once() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("pool");
        sqlx::migrate!("../crates/satchel-core/src/db/migrations")
            .run(&pool)
            .await
            .expect("migrations");
        let now = chrono::Utc::now().to_rfc3339();
        for statement in [
            "INSERT INTO accounts
             (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
             VALUES ('account', 'Me', 'me@example.com', 'gmail', '{}', '', '', '')",
            "INSERT INTO folders
             (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at)
             VALUES ('kunden', 'account', 'Kunden', 'Label_7', 'custom', 'Label_7', '', '')",
            "INSERT INTO folders
             (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at)
             VALUES ('archive', 'account', 'Archiviert', 'Archive', 'archive', 'ARCHIVE', '', '')",
            "INSERT INTO threads
             (id, account_id, subject, last_message_date, workflow_state, created_at, updated_at)
             VALUES ('thread', 'account', 'Subject', '', 'inbox', '', '')",
        ] {
            sqlx::query(statement).execute(&pool).await.expect("seed");
        }
        for (id, folder) in [("m-label", "kunden"), ("m-archive", "archive")] {
            sqlx::query(
                "INSERT INTO messages
                 (id, account_id, folder_id, thread_id, from_address, to_addresses, subject,
                  date, remote_id, created_at, updated_at)
                 VALUES (?, 'account', ?, 'thread', 'sender@example.com', '[]', 'Subject', ?, 'remote', ?, ?)",
            )
            .bind(id)
            .bind(folder)
            .bind(&now)
            .bind(&now)
            .bind(&now)
            .execute(&pool)
            .await
            .expect("message");
        }
        sqlx::query(
            "INSERT INTO attachments
             (id, message_id, filename, content_type, size, created_at)
             VALUES ('attachment', 'm-archive', 'report.pdf', 'application/pdf', 12, ?)",
        )
        .bind(&now)
        .execute(&pool)
        .await
        .expect("attachment");

        assert_eq!(
            repair_gmail_archive_duplicates(&pool, "account")
                .await
                .expect("repair"),
            1
        );
        let remaining: Vec<String> = sqlx::query_scalar("SELECT id FROM messages ORDER BY id")
            .fetch_all(&pool)
            .await
            .expect("remaining");
        assert_eq!(remaining, vec!["m-label".to_string()]);
        let attachments: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM attachments")
            .fetch_one(&pool)
            .await
            .expect("attachments");
        assert_eq!(attachments, 0);

        // The marker is what keeps every later full sync from walking the
        // account's messages again.
        assert_eq!(
            repair_gmail_archive_duplicates(&pool, "account")
                .await
                .expect("repeat repair"),
            0
        );
    }
}
