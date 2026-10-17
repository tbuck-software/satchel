//! Sidebar projection with trigger-maintained distinct thread counts.
use super::data_repairs::{is_pending, THREAD_FOLDERS};
use sqlx::{FromRow, SqlitePool};

#[derive(Debug, FromRow)]
pub struct MailboxRecord {
    pub id: String,
    pub account_id: String,
    pub account_email: String,
    pub name: String,
    pub folder_type: String,
    pub thread_count: i64,
    pub unread_count: i64,
}

pub const MAILBOX_SELECT: &str = "SELECT
            folders.id,
            folders.account_id,
            accounts.email AS account_email,
            folders.name,
            folders.folder_type,
            folders.thread_count,
            folders.thread_unread_count AS unread_count
         FROM folders
         JOIN accounts ON accounts.id = folders.account_id
         WHERE accounts.is_active = 1 AND accounts.kind = 'email'
           AND UPPER(TRIM(COALESCE(folders.remote_id, ''))) NOT IN (
             'CATEGORY_PERSONAL', 'CATEGORY_PROMOTIONS', 'CATEGORY_SOCIAL',
             'CATEGORY_UPDATES', 'CATEGORY_FORUMS',
             'IMPORTANT', 'STARRED', 'UNREAD', 'CHAT'
           )
         ORDER BY accounts.email ASC, folders.name ASC";

const MAILBOX_SELECT_PENDING: &str = "SELECT
            folders.id,
            folders.account_id,
            accounts.email AS account_email,
            folders.name,
            folders.folder_type,
            COUNT(DISTINCT CASE
              WHEN NOT (
                LOWER(TRIM(folders.folder_type)) = 'inbox'
                AND threads.workflow_state IN ('aside', 'done', 'snoozed')
              )
              THEN messages.thread_id
            END) AS thread_count,
            COUNT(DISTINCT CASE
              WHEN messages.is_read = 0
               AND NOT (
                 LOWER(TRIM(folders.folder_type)) = 'inbox'
                 AND threads.workflow_state IN ('aside', 'done', 'snoozed')
               )
              THEN messages.thread_id
            END) AS unread_count
         FROM folders
         JOIN accounts ON accounts.id = folders.account_id
         LEFT JOIN messages ON
           (messages.pending_folder_id IS NULL AND messages.folder_id = folders.id)
           OR messages.pending_folder_id = folders.id
         LEFT JOIN threads ON threads.id = messages.thread_id
         WHERE accounts.is_active = 1 AND accounts.kind = 'email'
           AND UPPER(TRIM(COALESCE(folders.remote_id, ''))) NOT IN (
             'CATEGORY_PERSONAL', 'CATEGORY_PROMOTIONS', 'CATEGORY_SOCIAL',
             'CATEGORY_UPDATES', 'CATEGORY_FORUMS',
             'IMPORTANT', 'STARRED', 'UNREAD', 'CHAT'
           )
         GROUP BY folders.id, folders.account_id, accounts.email, folders.name, folders.folder_type
         ORDER BY accounts.email ASC, folders.name ASC";

pub async fn list_mailboxes(pool: &SqlitePool) -> Result<Vec<MailboxRecord>, sqlx::Error> {
    let sql = if is_pending(pool, &THREAD_FOLDERS).await? {
        MAILBOX_SELECT_PENDING
    } else {
        MAILBOX_SELECT
    };
    sqlx::query_as(sql).fetch_all(pool).await
}
