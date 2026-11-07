//! A Delta Chat thread as the chat view and the quick reply need it: who is
//! in it, which group it is, and which message the next one answers.
//!
//! Everything is read from the thread's messages; there is no chat table.
//! Any mail can thread into a chat and say anything in its headers, so who a
//! reply goes to comes only from trusted sources:
//!
//! - Messages in Trash or Spam do not count.
//! - Order is the time Satchel received a message, never its `Date`, which
//!   the sender writes (a `Date: 2099` would otherwise always be "newest").
//! - A group starts from the user's own last reply in it, or else from the
//!   oldest group message, and grows only through messages whose `From` is
//!   already a member. The spec keeps the member list with
//!   `Chat-Group-Member-Added`/`-Removed` ("Add and remove members"); until
//!   Satchel tracks those, this is the closest list that a stranger cannot
//!   rewrite.
//! - A one-to-one chat keeps its peer: the one the user last replied to, or
//!   the counterpart of its first message. A newer sender never takes over.
//!
//! The quick reply asks before sending when the recipients differ from the
//! user's last reply or the chat would fall back from encrypted to plain text
//! (`recipients_changed`, `encrypted_before`).
use std::collections::BTreeMap;

use sqlx::SqlitePool;
use thiserror::Error;

use crate::db::threads::mailbox_section_predicate;
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
    /// Who the user's last reply in this chat went to, if there is one.
    pub last_reply_recipients: Option<Vec<String>>,
    /// `recipients` is not the set of `last_reply_recipients`.
    pub recipients_changed: bool,
    /// The latest trusted message of the chat was end-to-end encrypted.
    pub encrypted_before: bool,
    /// Display names the trusted messages gave, keyed by lowercased address.
    pub names: BTreeMap<String, String>,
    /// The newest trusted message: the next one is its reply.
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
    encrypted: i64,
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

        let trash = mailbox_section_predicate("trash").unwrap_or("0");
        let spam = mailbox_section_predicate("spam").unwrap_or("0");
        let rows: Vec<ChatMessageRow> = sqlx::query_as(&format!(
            "SELECT messages.id, messages.from_address, messages.from_name,
                    messages.to_addresses, messages.cc_addresses,
                    messages.chat_version, messages.chat_group_id, messages.chat_group_name,
                    COALESCE(json_extract(messages.trust_metadata, '$.root_encryption'), 0) AS encrypted
             FROM messages
             LEFT JOIN folders mailbox_folders
               ON mailbox_folders.id = COALESCE(messages.pending_folder_id, messages.folder_id)
             WHERE messages.thread_id = ? AND messages.is_draft = 0
               AND NOT COALESCE({trash}, 0) AND NOT COALESCE({spam}, 0)
             ORDER BY julianday(COALESCE(messages.received_at, messages.created_at)) ASC,
                      messages.id ASC"
        ))
        .bind(thread_id)
        .fetch_all(self.pool)
        .await?;
        if !rows.iter().any(|row| row.chat_version.is_some()) {
            return Err(ChatError::NotAChat);
        }
        Ok(summarize(thread_id, account_id, account_name, &own, &rows))
    }
}

/// `From`, `To` and `Cc` of a message, lowercased, in header order.
fn addresses_of(row: &ChatMessageRow) -> Vec<String> {
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
        .map(|addr| lower(&addr))
        .filter(|addr| addr.contains('@'))
        .collect()
}

fn push_unique(list: &mut Vec<String>, addr: String) {
    if !list.contains(&addr) {
        list.push(addr);
    }
}

/// `rows` are oldest first, by receipt time, without Trash and Spam.
fn summarize(
    thread_id: &str,
    account_id: String,
    account_name: String,
    own: &[String],
    rows: &[ChatMessageRow],
) -> ChatThread {
    let is_own = |addr: &str| own.contains(&lower(addr));
    let others = |row: &ChatMessageRow| {
        addresses_of(row)
            .into_iter()
            .filter(|addr| !is_own(addr))
            .collect::<Vec<_>>()
    };
    let last_own = rows.iter().rposition(|row| is_own(&row.from_address));

    // The group: the one the user last replied in, or the oldest one named.
    let group_id = last_own
        .and_then(|index| rows[index].chat_group_id.clone())
        .or_else(|| rows.iter().find_map(|row| row.chat_group_id.clone()));

    let mut recipients: Vec<String> = Vec::new();
    match group_id.as_deref() {
        Some(group) => {
            let in_group = |row: &ChatMessageRow| row.chat_group_id.as_deref() == Some(group);
            let base = last_own
                .filter(|index| in_group(&rows[*index]))
                .or_else(|| rows.iter().position(in_group));
            if let Some(base) = base {
                for addr in others(&rows[base]) {
                    push_unique(&mut recipients, addr);
                }
                for row in rows[base + 1..].iter().filter(|row| in_group(row)) {
                    // Only a member can bring others in.
                    if recipients.contains(&lower(&row.from_address)) {
                        for addr in others(row) {
                            push_unique(&mut recipients, addr);
                        }
                    }
                }
            }
        }
        None => {
            let peer = match last_own {
                Some(index) => others(&rows[index]).into_iter().next(),
                None => rows.first().and_then(|first| {
                    if is_own(&first.from_address) {
                        others(first).into_iter().next()
                    } else {
                        Some(lower(&first.from_address))
                    }
                }),
            };
            recipients.extend(peer);
        }
    }

    let last_reply_recipients = last_own.map(|index| others(&rows[index]));
    let recipients_changed = last_reply_recipients.as_ref().is_some_and(|last| {
        let mut before = last.clone();
        let mut now = recipients.clone();
        before.sort();
        now.sort();
        before.dedup();
        now.dedup();
        before != now
    });

    let trusted = |row: &ChatMessageRow| {
        is_own(&row.from_address) || recipients.contains(&lower(&row.from_address))
    };
    let trusted_rows = rows.iter().filter(|row| trusted(row)).collect::<Vec<_>>();
    let latest_trusted = trusted_rows.last().copied();

    let mut names = BTreeMap::new();
    for row in &trusted_rows {
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

    let group_name = trusted_rows
        .iter()
        .rev()
        .filter(|row| row.chat_group_id == group_id)
        .find_map(|row| row.chat_group_name.clone());

    // The address the user last wrote from, or the one the peer wrote to.
    let own_address = last_own
        .map(|index| lower(&rows[index].from_address))
        .or_else(|| {
            trusted_rows
                .iter()
                .flat_map(|row| addresses_of(row))
                .find(|addr| is_own(addr))
        })
        .unwrap_or_else(|| own.first().cloned().unwrap_or_default());

    ChatThread {
        thread_id: thread_id.to_string(),
        account_id,
        account_name,
        own_address,
        group_id,
        group_name,
        recipients,
        last_reply_recipients,
        recipients_changed,
        encrypted_before: latest_trusted.is_some_and(|row| row.encrypted != 0),
        names,
        reply_to_message_id: latest_trusted.map(|row| row.id.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GROUP: Option<(&str, &str)> = Some(("abcDEF123_-x", "Family"));

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
        for (id, kind) in [("inbox", "inbox"), ("trash", "trash"), ("spam", "spam")] {
            sqlx::query(
                "INSERT INTO folders(id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at)
                 VALUES(?, 'acc', ?, ?, ?, ?, '', '')",
            )
            .bind(id)
            .bind(kind)
            .bind(kind)
            .bind(kind)
            .bind(kind)
            .execute(&pool)
            .await
            .unwrap();
        }
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

    struct Msg<'a> {
        id: &'a str,
        thread: &'a str,
        received: &'a str,
        from: (&'a str, &'a str),
        to: &'a [&'a str],
        chat: Option<(&'a str, &'a str)>,
    }

    impl Msg<'_> {
        async fn store(&self, pool: &SqlitePool, folder: &str, date: &str, encrypted: bool) {
            sqlx::query(
                "INSERT INTO messages(id, account_id, folder_id, thread_id, from_address, from_name,
                    to_addresses, subject, date, received_at, remote_id, created_at, updated_at,
                    chat_version, chat_group_id, chat_group_name, trust_metadata)
                 VALUES(?, 'acc', ?, ?, ?, ?, ?, 'Chat', ?, ?, ?, '', '', '1.0', ?, ?, ?)",
            )
            .bind(self.id)
            .bind(folder)
            .bind(self.thread)
            .bind(self.from.1)
            .bind(self.from.0)
            .bind(serde_json::to_string(self.to).unwrap())
            .bind(date)
            .bind(self.received)
            .bind(self.id)
            .bind(self.chat.map(|c| c.0))
            .bind(self.chat.map(|c| c.1))
            .bind(if encrypted {
                r#"{"root_encryption":true}"#
            } else {
                "{}"
            })
            .execute(pool)
            .await
            .unwrap();
        }

        async fn inbox(&self, pool: &SqlitePool) {
            self.store(pool, "inbox", self.received, false).await;
        }
    }

    #[tokio::test]
    async fn a_one_to_one_chat_answers_the_peer_from_the_address_it_wrote_to() {
        let pool = pool().await;
        Msg {
            id: "m1",
            thread: "one",
            received: "2026-08-01T10:00:00Z",
            from: ("Lena", "lena@example.test"),
            to: &["chat@example.test"],
            chat: None,
        }
        .inbox(&pool)
        .await;
        Msg {
            id: "m2",
            thread: "one",
            received: "2026-08-01T11:00:00Z",
            from: ("Tom", "chat@example.test"),
            to: &["Lena <lena@example.test>"],
            chat: None,
        }
        .inbox(&pool)
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
        assert!(!chat.recipients_changed);
    }

    #[tokio::test]
    async fn a_newer_sender_never_takes_over_a_one_to_one_chat() {
        let pool = pool().await;
        Msg {
            id: "m1",
            thread: "one",
            received: "2026-08-01T10:00:00Z",
            from: ("Lena", "lena@example.test"),
            to: &["tom@example.test"],
            chat: None,
        }
        .inbox(&pool)
        .await;
        // Threads in later, claims to be newest of all.
        Msg {
            id: "evil",
            thread: "one",
            received: "2026-08-01T12:00:00Z",
            from: ("Lena", "spam@evil.test"),
            to: &["tom@example.test"],
            chat: None,
        }
        .store(&pool, "inbox", "2099-01-01T00:00:00Z", false)
        .await;
        let chat = ChatRepository::new(&pool).thread("one").await.unwrap();
        assert_eq!(chat.recipients, vec!["lena@example.test".to_string()]);
        assert_eq!(chat.reply_to_message_id.as_deref(), Some("m1"));
    }

    #[tokio::test]
    async fn a_group_grows_only_through_its_members() {
        let pool = pool().await;
        Msg {
            id: "g1",
            thread: "group",
            received: "2026-08-01T10:00:00Z",
            from: ("Lena", "lena@example.test"),
            to: &["tom@example.test", "jonas@example.test"],
            chat: GROUP,
        }
        .inbox(&pool)
        .await;
        // A member adds Mia.
        Msg {
            id: "g2",
            thread: "group",
            received: "2026-08-01T11:00:00Z",
            from: ("Jonas", "jonas@example.test"),
            to: &["tom@example.test", "lena@example.test", "mia@example.test"],
            chat: GROUP,
        }
        .inbox(&pool)
        .await;
        let chat = ChatRepository::new(&pool).thread("group").await.unwrap();
        assert_eq!(chat.group_id.as_deref(), Some("abcDEF123_-x"));
        assert_eq!(chat.group_name.as_deref(), Some("Family"));
        assert_eq!(
            chat.recipients,
            vec![
                "lena@example.test",
                "jonas@example.test",
                "mia@example.test"
            ]
        );
        assert_eq!(chat.own_address, "tom@example.test");
    }

    #[tokio::test]
    async fn a_crafted_group_message_cannot_redirect_the_reply() {
        let pool = pool().await;
        Msg {
            id: "g1",
            thread: "group",
            received: "2026-08-01T10:00:00Z",
            from: ("Lena", "lena@example.test"),
            to: &["tom@example.test", "jonas@example.test"],
            chat: GROUP,
        }
        .store(&pool, "inbox", "2026-08-01T10:00:00Z", true)
        .await;
        Msg {
            id: "g2",
            thread: "group",
            received: "2026-08-01T11:00:00Z",
            from: ("Tom", "tom@example.test"),
            to: &["lena@example.test", "jonas@example.test"],
            chat: GROUP,
        }
        .store(&pool, "inbox", "2026-08-01T11:00:00Z", true)
        .await;
        // The reviewer's reproduction: same group id, a Date far in the
        // future, a stranger as sender and a new recipient list.
        let evil = Msg {
            id: "evil",
            thread: "group",
            received: "2026-08-01T12:00:00Z",
            from: ("Lena", "spam@evil.test"),
            to: &[
                "tom@example.test",
                "victim1@example.test",
                "victim2@example.test",
            ],
            chat: Some(("abcDEF123_-x", "Family (new)")),
        };
        evil.store(&pool, "inbox", "2099-01-01T00:00:00Z", false)
            .await;
        let chat = ChatRepository::new(&pool).thread("group").await.unwrap();
        assert_eq!(
            chat.recipients,
            vec!["lena@example.test", "jonas@example.test"]
        );
        assert_eq!(chat.group_name.as_deref(), Some("Family"));
        assert_eq!(chat.reply_to_message_id.as_deref(), Some("g2"));
        assert!(!chat.recipients_changed);
        assert!(chat.encrypted_before);
        assert!(!chat.names.contains_key("spam@evil.test"));
    }

    #[tokio::test]
    async fn a_member_who_adds_people_after_the_last_reply_is_a_change_to_confirm() {
        let pool = pool().await;
        Msg {
            id: "g1",
            thread: "group",
            received: "2026-08-01T10:00:00Z",
            from: ("Tom", "tom@example.test"),
            to: &["lena@example.test"],
            chat: GROUP,
        }
        .inbox(&pool)
        .await;
        Msg {
            id: "g2",
            thread: "group",
            received: "2026-08-01T11:00:00Z",
            from: ("Lena", "lena@example.test"),
            to: &["tom@example.test", "mia@example.test"],
            chat: GROUP,
        }
        .inbox(&pool)
        .await;
        let chat = ChatRepository::new(&pool).thread("group").await.unwrap();
        assert_eq!(
            chat.recipients,
            vec!["lena@example.test", "mia@example.test"]
        );
        assert_eq!(
            chat.last_reply_recipients,
            Some(vec!["lena@example.test".to_string()])
        );
        assert!(chat.recipients_changed);
    }

    #[tokio::test]
    async fn trash_and_spam_do_not_count() {
        let pool = pool().await;
        Msg {
            id: "g1",
            thread: "group",
            received: "2026-08-01T10:00:00Z",
            from: ("Lena", "lena@example.test"),
            to: &["tom@example.test"],
            chat: GROUP,
        }
        .inbox(&pool)
        .await;
        Msg {
            id: "g2",
            thread: "group",
            received: "2026-08-01T11:00:00Z",
            from: ("Lena", "lena@example.test"),
            to: &["tom@example.test", "trash@example.test"],
            chat: GROUP,
        }
        .store(&pool, "trash", "2026-08-01T11:00:00Z", false)
        .await;
        Msg {
            id: "g3",
            thread: "group",
            received: "2026-08-01T12:00:00Z",
            from: ("Lena", "lena@example.test"),
            to: &["tom@example.test", "spam@example.test"],
            chat: GROUP,
        }
        .store(&pool, "spam", "2026-08-01T12:00:00Z", false)
        .await;
        let chat = ChatRepository::new(&pool).thread("group").await.unwrap();
        assert_eq!(chat.recipients, vec!["lena@example.test"]);
        assert_eq!(chat.reply_to_message_id.as_deref(), Some("g1"));
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
