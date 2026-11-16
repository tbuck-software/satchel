//! Local copies of sent mail, and the Sent folder they go into.
//!
//! A send stores a local copy right away (`m-<uuid>`, no remote id) while the
//! server copy arrives with a later sync. Both carry the same Message-ID; the
//! server copy replaces the local one once it is ingested.

use futures::future::BoxFuture;
use sqlx::{Connection, Executor, Sqlite, SqliteConnection};

use super::data_repairs::{DataRepair, Prepared};
use super::folders::FolderChoice;

/// The Sent folder both the APPEND and the local copy use.
pub async fn preferred_sent_folder<'e>(
    executor: impl Executor<'e, Database = Sqlite>,
    account_id: &str,
) -> Result<Option<FolderChoice>, sqlx::Error> {
    super::folders::preferred_folder_of_type(executor, account_id, "sent").await
}

/// A server copy of this Message-ID in one of the account's Sent folders.
/// Returns `(message id, thread id)`.
pub async fn synced_sent_copy<'e>(
    executor: impl Executor<'e, Database = Sqlite>,
    account_id: &str,
    message_id_header: &str,
) -> Result<Option<(String, Option<String>)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT m.id, m.thread_id FROM messages m JOIN folders f ON f.id = m.folder_id
         WHERE m.account_id = ? AND m.message_id_header = ?
           AND m.remote_id IS NOT NULL AND f.folder_type = 'sent'
         ORDER BY m.id LIMIT 1",
    )
    .bind(account_id)
    .bind(message_id_header)
    .fetch_optional(executor)
    .await
}

/// Replace the local copies of a sent mail with its server copy `keep_id`.
///
/// Only rows the send path wrote are touched: in a Sent folder, sent, not a
/// draft, with neither remote id nor UID. Reminders, invites and tasks move
/// to the server copy. Returns the threads that lost a message; their
/// projections are the caller's to refresh.
pub async fn retire_local_sent_copies(
    connection: &mut SqliteConnection,
    account_id: &str,
    message_id_header: &str,
    keep_id: &str,
) -> Result<Vec<String>, sqlx::Error> {
    let header = message_id_header.trim();
    if header.is_empty() {
        return Ok(Vec::new());
    }
    let copies: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT m.id, m.thread_id FROM messages m JOIN folders f ON f.id = m.folder_id
         WHERE m.account_id = ? AND m.message_id_header = ? AND m.id != ?
           AND m.remote_id IS NULL AND m.remote_uid IS NULL
           AND m.is_sent = 1 AND m.is_draft = 0 AND f.folder_type = 'sent'",
    )
    .bind(account_id)
    .bind(header)
    .bind(keep_id)
    .fetch_all(&mut *connection)
    .await?;

    let mut threads = Vec::new();
    for (copy_id, thread_id) in copies {
        for table in ["follow_up_reminders", "calendar_invites", "tasks"] {
            sqlx::query(&format!(
                "UPDATE {table} SET message_id = ? WHERE message_id = ?"
            ))
            .bind(keep_id)
            .bind(&copy_id)
            .execute(&mut *connection)
            .await?;
        }
        sqlx::query("DELETE FROM attachments WHERE message_id = ?")
            .bind(&copy_id)
            .execute(&mut *connection)
            .await?;
        sqlx::query("DELETE FROM messages WHERE id = ?")
            .bind(&copy_id)
            .execute(&mut *connection)
            .await?;
        if let Some(thread_id) = thread_id.filter(|id| !id.trim().is_empty()) {
            if !threads.contains(&thread_id) {
                threads.push(thread_id);
            }
        }
    }
    Ok(threads)
}

/// Local copies left behind while the send path and the APPEND chose
/// different Sent folders: each one that has a server copy is retired.
pub const SENT_LOCAL_COPIES: DataRepair = DataRepair {
    name: "sent_local_copies",
    block: sent_local_copies_block,
    prepare: None,
};

fn sent_local_copies_block<'c>(
    connection: &'c mut SqliteConnection,
    cursor: &'c str,
    limit: i64,
    _prepared: Option<Prepared>,
) -> BoxFuture<'c, Result<Option<String>, sqlx::Error>> {
    Box::pin(async move {
        // Walks only Sent folders' messages (indexed by folder), not the table.
        let candidates: Vec<(String, String, String)> = sqlx::query_as(
            "SELECT m.id, m.account_id, m.message_id_header
             FROM folders f JOIN messages m ON m.folder_id = f.id
             WHERE f.folder_type = 'sent' AND m.id > ?
               AND m.remote_id IS NULL AND m.remote_uid IS NULL
               AND m.is_sent = 1 AND m.is_draft = 0
               AND TRIM(COALESCE(m.message_id_header, '')) != ''
             ORDER BY m.id LIMIT ?",
        )
        .bind(cursor)
        .bind(limit)
        .fetch_all(&mut *connection)
        .await?;
        let Some(last) = candidates.last().map(|row| row.0.clone()) else {
            return Ok(None);
        };

        let mut savepoint = connection.begin().await?;
        let mut threads = Vec::new();
        for (_, account_id, header) in &candidates {
            let Some((keep_id, keep_thread)) =
                synced_sent_copy(&mut *savepoint, account_id, header).await?
            else {
                continue;
            };
            for thread_id in
                retire_local_sent_copies(&mut savepoint, account_id, header, &keep_id).await?
            {
                if !threads.contains(&thread_id) {
                    threads.push(thread_id);
                }
            }
            if let Some(keep_thread) = keep_thread.filter(|id| !threads.contains(id)) {
                threads.push(keep_thread);
            }
        }
        crate::db::threads::refresh_thread_projections_tx(&mut savepoint, &threads)
            .await
            .map_err(|error| sqlx::Error::Io(std::io::Error::other(error.to_string())))?;
        savepoint.commit().await?;
        Ok(Some(last))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::data_repairs::{run_repair, RepairRun};
    use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

    const SENT: &str = "fld-sent";
    const LEGACY: &str = "fld-sent-messages";

    async fn pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("pool");
        crate::db::init(&pool).await.expect("migrations");
        // The repair this module adds is planted by the migration; tests
        // start it on purpose.
        sqlx::query("DELETE FROM app_settings WHERE key = 'data_repair.sent_local_copies'")
            .execute(&pool)
            .await
            .expect("marker");
        // Apple Mail's legacy folder was listed first, as on the reported account.
        sqlx::query(
            "INSERT INTO accounts(id, display_name, email, provider, config, credentials_ref, is_active, created_at, updated_at)
             VALUES('acc', 'Me', 'me@example.test', 'imap', '{}', '', 1, '2026-01-01', '2026-01-01');
             INSERT INTO folders(id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at)
             VALUES('fld-sent-messages', 'acc', 'INBOX.Sent Messages', 'INBOX.Sent Messages', 'sent', 'INBOX.Sent Messages', '2026-03-08T16:35:37Z', '2026-03-08T16:35:37Z'),
                   ('fld-sent', 'acc', 'INBOX.Sent', 'INBOX.Sent', 'sent', 'INBOX.Sent', '2026-03-08T16:35:38Z', '2026-03-08T16:35:38Z');",
        )
        .execute(&pool)
        .await
        .expect("seed");
        pool
    }

    async fn insert_message(
        pool: &SqlitePool,
        id: &str,
        folder: &str,
        thread: &str,
        header: &str,
        remote_uid: Option<i64>,
    ) {
        sqlx::query(
            "INSERT OR IGNORE INTO threads(id, account_id, subject, last_message_date, message_count, unread_count, created_at, updated_at)
             VALUES(?, 'acc', 'Hello', '2026-11-14T00:00:00Z', 1, 0, '2026-11-14', '2026-11-14')",
        )
        .bind(thread)
        .execute(pool)
        .await
        .expect("thread");
        sqlx::query(
            "INSERT INTO messages (id, account_id, folder_id, thread_id, message_id_header, subject,
                                   from_address, to_addresses, date, is_sent, is_draft,
                                   remote_uid, remote_id, created_at, updated_at)
             VALUES (?, 'acc', ?, ?, ?, 'Hello', 'me@example.test', '[]',
                     '2026-11-14T00:00:00Z', 1, 0, ?, ?, '2026-11-14', '2026-11-14')",
        )
        .bind(id)
        .bind(folder)
        .bind(thread)
        .bind(header)
        .bind(remote_uid)
        .bind(remote_uid.map(|uid| uid.to_string()))
        .execute(pool)
        .await
        .expect("message");
    }

    async fn ids(pool: &SqlitePool) -> Vec<String> {
        sqlx::query_scalar("SELECT id FROM messages ORDER BY id")
            .fetch_all(pool)
            .await
            .expect("ids")
    }

    async fn preferred(pool: &SqlitePool) -> String {
        preferred_sent_folder(pool, "acc")
            .await
            .expect("query")
            .expect("folder")
            .id
    }

    #[tokio::test]
    async fn two_sent_folders_resolve_to_the_one_holding_the_most_mail() {
        let pool = pool().await;
        // As reported: the legacy folder is older and holds a few old mails
        // plus stranded local copies; the live one holds the rest.
        insert_message(&pool, "m-old", LEGACY, "th-old", "<old@x>", Some(1)).await;
        insert_message(&pool, "m-local", LEGACY, "th-local", "<local@x>", None).await;
        for uid in 1..=3 {
            let id = format!("m-sent-{uid}");
            insert_message(&pool, &id, SENT, &id, &format!("<{id}@x>"), Some(uid)).await;
        }
        assert_eq!(preferred(&pool).await, SENT);
    }

    #[tokio::test]
    async fn two_empty_sent_folders_prefer_the_canonical_name_over_the_older_row() {
        let pool = pool().await;
        assert_eq!(preferred(&pool).await, SENT);
    }

    #[tokio::test]
    async fn a_server_declared_sent_folder_wins_over_everything_else() {
        let pool = pool().await;
        insert_message(&pool, "m-a", SENT, "th-a", "<a@x>", Some(1)).await;
        sqlx::query("UPDATE folders SET special_use = 1 WHERE id = ?")
            .bind(LEGACY)
            .execute(&pool)
            .await
            .expect("flag");
        let folder = preferred_sent_folder(&pool, "acc").await.unwrap().unwrap();
        assert_eq!(folder.id, LEGACY);
        assert_eq!(folder.remote_path, "INBOX.Sent Messages");
    }

    #[tokio::test]
    async fn retiring_keeps_the_server_copy_and_moves_the_reminder_over() {
        let pool = pool().await;
        insert_message(&pool, "m-local", LEGACY, "th-1", "<sent@x>", None).await;
        insert_message(&pool, "m-server", SENT, "th-1", "<sent@x>", Some(398)).await;
        sqlx::query(
            "INSERT INTO follow_up_reminders (id, account_id, thread_id, message_id, due_at, status, created_at, updated_at)
             VALUES ('r', 'acc', 'th-1', 'm-local', '2026-11-20T00:00:00Z', 'pending', '', '')",
        )
        .execute(&pool)
        .await
        .expect("reminder");

        let mut connection = pool.acquire().await.unwrap();
        let threads = retire_local_sent_copies(&mut connection, "acc", "<sent@x>", "m-server")
            .await
            .unwrap();
        drop(connection);

        assert_eq!(threads, ["th-1"]);
        assert_eq!(ids(&pool).await, ["m-server"]);
        let reminder: String =
            sqlx::query_scalar("SELECT message_id FROM follow_up_reminders WHERE id = 'r'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(reminder, "m-server");
    }

    #[tokio::test]
    async fn retiring_leaves_server_mail_and_unsynced_copies_alone() {
        let pool = pool().await;
        // The old Apple Mail copy is server mail with the same Message-ID.
        insert_message(&pool, "m-legacy", LEGACY, "th-1", "<sent@x>", Some(4)).await;
        insert_message(&pool, "m-server", SENT, "th-1", "<sent@x>", Some(398)).await;
        insert_message(&pool, "m-other", LEGACY, "th-2", "<other@x>", None).await;

        let mut connection = pool.acquire().await.unwrap();
        retire_local_sent_copies(&mut connection, "acc", "<sent@x>", "m-server")
            .await
            .unwrap();
        drop(connection);

        assert_eq!(ids(&pool).await, ["m-legacy", "m-other", "m-server"]);
    }

    #[tokio::test]
    async fn the_repair_removes_stranded_copies_and_their_empty_threads() {
        let pool = pool().await;
        // As reported: local copy in the legacy folder, server copy in INBOX.Sent.
        insert_message(&pool, "m-1", LEGACY, "th-local", "<one@x>", None).await;
        insert_message(&pool, "m-imap-1", SENT, "th-server", "<one@x>", Some(397)).await;
        insert_message(&pool, "m-2", LEGACY, "th-2", "<two@x>", None).await;
        insert_message(&pool, "m-imap-2", SENT, "th-2", "<two@x>", Some(398)).await;
        // Not synced yet: stays until its server copy arrives.
        insert_message(&pool, "m-3", LEGACY, "th-3", "<three@x>", None).await;
        sqlx::query(
            "INSERT INTO app_settings (key, value, updated_at)
             VALUES ('data_repair.sent_local_copies', '', '')",
        )
        .execute(&pool)
        .await
        .unwrap();

        let run = run_repair(&pool, &SENT_LOCAL_COPIES, 1, None, |_| {})
            .await
            .unwrap();

        assert!(matches!(run, RepairRun::Completed { .. }));
        assert_eq!(ids(&pool).await, ["m-3", "m-imap-1", "m-imap-2"]);
        let threads: Vec<String> = sqlx::query_scalar("SELECT id FROM threads ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(threads, ["th-2", "th-3", "th-server"]);
    }
}
