//! A Delta Chat thread as the chat view and the quick reply need it: who is
//! in it, which group it is, and which message the next one answers.
//!
//! Everything is read from the thread's messages; there is no chat table. The
//! member list follows the latest group message's `From`/`To`/`Cc`. The spec
//! keeps it with `Chat-Group-Member-Added`/`-Removed` ("Add and remove
//! members"); until Satchel tracks those, the latest message is the closest
//! view of who is in the group, and it is the list Delta Chat itself sends to.
use std::collections::BTreeMap;

use sqlx::SqlitePool;
use thiserror::Error;

use crate::utils::mime::extract_mailboxes;

#[derive(Debug, Error)]
pub enum ChatError {
    #[error("thread not found")]
    NotFound,
    #[error("thread holds no chat message")]
    NotAChat,
    #[error("chat persistence failed: {0}")]
    Persistence(#[from] sqlx::Error),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatThread {
    pub thread_id: String,
    pub account_id: String,
    pub account_name: String,
    /// The own address the chat runs on: an alias when the peer wrote to one.
    pub own_address: String,
    pub group_id: Option<String>,
    pub group_name: Option<String>,
    /// Who the next message goes to, own addresses left out, in header order.
    pub recipients: Vec<String>,
    /// Display names the messages gave, keyed by lowercased address.
    pub names: BTreeMap<String, String>,
    /// The newest message: the next one is its reply.
    pub reply_to_message_id: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
struct ChatMessageRow {
    id: String,
    from_address: String,
    from_name: Option<String>,
    to_addresses: Option<String>,
    cc_addresses: Option<String>,
    chat_version: Option<String>,
    chat_group_id: Option<String>,
    chat_group_name: Option<String>,
}

fn lower(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn mailboxes(json: Option<&str>) -> Vec<(Option<String>, String)> {
    let entries: Vec<String> = json
        .and_then(|raw| serde_json::from_str(raw).ok())
        .unwrap_or_default();
    entries
        .iter()
        .flat_map(|entry| extract_mailboxes(entry))
        .collect()
}

pub struct ChatRepository<'a> {
    pool: &'a SqlitePool,
}

impl<'a> ChatRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn thread(&self, thread_id: &str) -> Result<ChatThread, ChatError> {
        let account: Option<(String, String, String)> = sqlx::query_as(
            "SELECT accounts.id, accounts.email, accounts.display_name
             FROM threads JOIN accounts ON accounts.id = threads.account_id
             WHERE threads.id = ?",
        )
        .bind(thread_id)
        .fetch_optional(self.pool)
        .await?;
        let (account_id, account_email, account_name) = account.ok_or(ChatError::NotFound)?;
        let mut own: Vec<String> = sqlx::query_scalar(
            "SELECT email FROM account_aliases WHERE account_id = ? ORDER BY is_default DESC, email",
        )
        .bind(&account_id)
        .fetch_all(self.pool)
        .await?
        .into_iter()
        .map(|email: String| lower(&email))
        .collect();
        own.insert(0, lower(&account_email));

        let rows: Vec<ChatMessageRow> = sqlx::query_as(
            "SELECT id, from_address, from_name, to_addresses, cc_addresses,
                    chat_version, chat_group_id, chat_group_name
             FROM messages
             WHERE thread_id = ? AND is_draft = 0
             ORDER BY date DESC, id DESC",
        )
        .bind(thread_id)
        .fetch_all(self.pool)
        .await?;
        if !rows.iter().any(|row| row.chat_version.is_some()) {
            return Err(ChatError::NotAChat);
        }
        Ok(summarize(thread_id, account_id, account_name, &own, &rows))
    }
}

fn summarize(
    thread_id: &str,
    account_id: String,
    account_name: String,
    own: &[String],
    rows: &[ChatMessageRow],
) -> ChatThread {
    let is_own = |addr: &str| own.contains(&lower(addr));
    let mut names = BTreeMap::new();
    for row in rows.iter().rev() {
        if let Some(name) = row.from_name.as_deref().filter(|n| !n.trim().is_empty()) {
            names.insert(lower(&row.from_address), name.trim().to_string());
        }
        for (name, addr) in mailboxes(row.to_addresses.as_deref())
            .into_iter()
            .chain(mailboxes(row.cc_addresses.as_deref()))
        {
            if let Some(name) = name.filter(|n| !n.trim().is_empty()) {
                names
                    .entry(lower(&addr))
                    .or_insert_with(|| name.trim().to_string());
            }
        }
    }

    let group_row = rows.iter().find(|row| row.chat_group_id.is_some());
    let group_id = group_row.and_then(|row| row.chat_group_id.clone());
    let group_name = rows
        .iter()
        .filter(|row| row.chat_group_id == group_id)
        .find_map(|row| row.chat_group_name.clone());

    let addresses_of = |row: &ChatMessageRow| {
        std::iter::once(row.from_address.clone())
            .chain(
                mailboxes(row.to_addresses.as_deref())
                    .into_iter()
                    .map(|(_, a)| a),
            )
            .chain(
                mailboxes(row.cc_addresses.as_deref())
                    .into_iter()
                    .map(|(_, a)| a),
            )
            .collect::<Vec<_>>()
    };
    let candidates = match group_row {
        Some(row) => addresses_of(row),
        None => match rows.iter().find(|row| !is_own(&row.from_address)) {
            Some(row) => vec![row.from_address.clone()],
            None => rows.first().map(addresses_of).unwrap_or_default(),
        },
    };
    let mut recipients: Vec<String> = Vec::new();
    for addr in candidates {
        let addr = lower(&addr);
        if addr.contains('@') && !is_own(&addr) && !recipients.contains(&addr) {
            recipients.push(addr);
        }
    }

    // The address the peer wrote to; the primary otherwise.
    let own_address = rows
        .iter()
        .filter(|row| !is_own(&row.from_address))
        .flat_map(|row| {
            mailboxes(row.to_addresses.as_deref())
                .into_iter()
                .chain(mailboxes(row.cc_addresses.as_deref()))
                .map(|(_, addr)| lower(&addr))
                .collect::<Vec<_>>()
        })
        .find(|addr| is_own(addr))
        .unwrap_or_else(|| own.first().cloned().unwrap_or_default());

    ChatThread {
        thread_id: thread_id.to_string(),
        account_id,
        account_name,
        own_address,
        group_id,
        group_name,
        recipients,
        names,
        reply_to_message_id: rows.first().map(|row| row.id.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn pool() -> SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::db::init(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
             VALUES ('acc', 'Tom', 'tom@example.test', 'imap', ?, '', '', '')",
        )
        .bind(crate::accounts::encrypt_account_config("acc", "{}").unwrap())
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO account_aliases (id, account_id, email, created_at, updated_at)
             VALUES ('alias', 'acc', 'chat@example.test', '', '')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO folders(id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at)
             VALUES('inbox', 'acc', 'INBOX', 'INBOX', 'inbox', 'INBOX', '', '')",
        )
        .execute(&pool)
        .await
        .unwrap();
        for thread in ["one", "group", "mail"] {
            sqlx::query(
                "INSERT INTO threads(id, account_id, subject, last_message_date, message_count, unread_count, created_at, updated_at)
                 VALUES(?, 'acc', 'Chat', '2026-08-01T00:00:00Z', 1, 0, '', '')",
            )
            .bind(thread)
            .execute(&pool)
            .await
            .unwrap();
        }
        pool
    }

    async fn message(
        pool: &SqlitePool,
        id: &str,
        thread: &str,
        date: &str,
        from: (&str, &str),
        to: &[&str],
        chat: Option<(&str, &str)>,
    ) {
        sqlx::query(
            "INSERT INTO messages(id, account_id, folder_id, thread_id, from_address, from_name,
                to_addresses, subject, date, remote_id, created_at, updated_at,
                chat_version, chat_group_id, chat_group_name)
             VALUES(?, 'acc', 'inbox', ?, ?, ?, ?, 'Chat', ?, ?, '', '', '1.0', ?, ?)",
        )
        .bind(id)
        .bind(thread)
        .bind(from.1)
        .bind(from.0)
        .bind(serde_json::to_string(to).unwrap())
        .bind(date)
        .bind(id)
        .bind(chat.map(|c| c.0))
        .bind(chat.map(|c| c.1))
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn a_one_to_one_chat_answers_the_peer_from_the_address_it_wrote_to() {
        let pool = pool().await;
        message(
            &pool,
            "m1",
            "one",
            "2026-08-01T10:00:00Z",
            ("Lena", "lena@example.test"),
            &["chat@example.test"],
            None,
        )
        .await;
        message(
            &pool,
            "m2",
            "one",
            "2026-08-01T11:00:00Z",
            ("Tom", "chat@example.test"),
            &["Lena <lena@example.test>"],
            None,
        )
        .await;
        let chat = ChatRepository::new(&pool).thread("one").await.unwrap();
        assert_eq!(chat.recipients, vec!["lena@example.test".to_string()]);
        assert_eq!(chat.own_address, "chat@example.test");
        assert_eq!(chat.group_id, None);
        assert_eq!(chat.reply_to_message_id.as_deref(), Some("m2"));
        assert_eq!(
            chat.names.get("lena@example.test").map(String::as_str),
            Some("Lena")
        );
    }

    #[tokio::test]
    async fn a_group_goes_to_every_member_of_its_latest_message() {
        let pool = pool().await;
        let group = Some(("abcDEF123_-x", "Family"));
        message(
            &pool,
            "g1",
            "group",
            "2026-08-01T10:00:00Z",
            ("Lena", "lena@example.test"),
            &["tom@example.test", "jonas@example.test"],
            group,
        )
        .await;
        message(
            &pool,
            "g2",
            "group",
            "2026-08-01T11:00:00Z",
            ("Jonas", "jonas@example.test"),
            &["tom@example.test", "lena@example.test", "mia@example.test"],
            group,
        )
        .await;
        let chat = ChatRepository::new(&pool).thread("group").await.unwrap();
        assert_eq!(chat.group_id.as_deref(), Some("abcDEF123_-x"));
        assert_eq!(chat.group_name.as_deref(), Some("Family"));
        assert_eq!(
            chat.recipients,
            vec![
                "jonas@example.test",
                "lena@example.test",
                "mia@example.test"
            ]
        );
        assert_eq!(chat.own_address, "tom@example.test");
    }

    #[tokio::test]
    async fn plain_mail_is_not_a_chat() {
        let pool = pool().await;
        sqlx::query(
            "INSERT INTO messages(id, account_id, folder_id, thread_id, from_address, to_addresses, subject, date, remote_id, created_at, updated_at)
             VALUES('p', 'acc', 'inbox', 'mail', 'a@example.test', '[]', 'Hi', '2026-08-01', 'p', '', '')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let repo = ChatRepository::new(&pool);
        assert!(matches!(
            repo.thread("mail").await,
            Err(ChatError::NotAChat)
        ));
        assert!(matches!(
            repo.thread("nope").await,
            Err(ChatError::NotFound)
        ));
    }
}
