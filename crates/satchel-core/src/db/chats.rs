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
//! - A message from the user's own address is the user's only when it is in
//!   Sent or passed DMARC (Delta Chat on another device delivers to the
//!   Inbox). Anyone can write `From:` with the user's address.
//! - The chat is what its first message (or the user's last reply) says: a
//!   group if that one names a group, one-to-one otherwise. A later message
//!   cannot turn a one-to-one chat into someone's group.
//! - A group starts from the user's own last reply in it, or else from its
//!   first message, and grows only through messages whose `From` is already
//!   a member. The spec keeps the member list with
//!   `Chat-Group-Member-Added`/`-Removed` ("Add and remove members"); until
//!   Satchel tracks those, this is the closest list that a stranger cannot
//!   rewrite.
//! - A one-to-one chat keeps its peer: the one the user last replied to, or
//!   the counterpart of its first message. A newer sender never takes over.
//!
//! The quick reply asks before sending when the recipients differ from the
//! baseline — the user's last reply, or the chat's first message before there
//! is one — or the chat would fall back from encrypted to plain text after any
//! trusted message in it was encrypted (`recipients_changed`,
//! `encrypted_before`).
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
    /// What the recipients are compared with: who the user's last reply went
    /// to, or the members of the chat's first message before there is one.
    pub baseline_recipients: Vec<String>,
    /// `recipients` is not the set of `baseline_recipients`.
    pub recipients_changed: bool,
    /// A trusted message of the chat was end-to-end encrypted. Sticky: a
    /// later plain message does not lift it.
    pub encrypted_before: bool,
    /// Messages that count for nothing here: from neither the user nor a
    /// member. The chat view marks them.
    pub untrusted_message_ids: Vec<String>,
    /// The user has no trusted reply in this chat yet (none in Sent, none
    /// with a provider-vouched DMARC pass). Who the first reply goes to is
    /// read from mail anyone can send, so it always asks.
    pub first_reply: bool,
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
    is_sent: i64,
    auth_dmarc: Option<String>,
    /// The verdicts came through the receiving provider's trust boundary
    /// (`MessageTrust::auth_verified`); older rows have none.
    auth_verified: i64,
    /// In Trash or Spam: counts for nothing but can still be the origin.
    binned: i64,
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
        let mut conn = self.pool.acquire().await?;
        chat_thread_in(&mut conn, thread_id).await
    }
}

/// The user's addresses for an account: the primary first, then aliases.
async fn own_addresses(
    conn: &mut sqlx::SqliteConnection,
    account_id: &str,
    account_email: &str,
) -> Result<Vec<String>, sqlx::Error> {
    let mut own: Vec<String> = sqlx::query_scalar(
        "SELECT email FROM account_aliases WHERE account_id = ? ORDER BY is_default DESC, email",
    )
    .bind(account_id)
    .fetch_all(&mut *conn)
    .await?
    .into_iter()
    .map(|email: String| lower(&email))
    .collect();
    own.insert(0, lower(account_email));
    Ok(own)
}

/// [`ChatRepository::thread`] on a connection, so a transaction (the reply
/// merge) can decide on the same snapshot it changes.
pub async fn chat_thread_in(
    conn: &mut sqlx::SqliteConnection,
    thread_id: &str,
) -> Result<ChatThread, ChatError> {
    let account: Option<(String, String, String)> = sqlx::query_as(
        "SELECT accounts.id, accounts.email, accounts.display_name
         FROM threads JOIN accounts ON accounts.id = threads.account_id
         WHERE threads.id = ?",
    )
    .bind(thread_id)
    .fetch_optional(&mut *conn)
    .await?;
    let (account_id, account_email, account_name) = account.ok_or(ChatError::NotFound)?;
    let own = own_addresses(conn, &account_id, &account_email).await?;

    let trash = mailbox_section_predicate("trash").unwrap_or("0");
    let spam = mailbox_section_predicate("spam").unwrap_or("0");
    let rows: Vec<ChatMessageRow> = sqlx::query_as(&format!(
        "SELECT messages.id, messages.from_address, messages.from_name,
                messages.to_addresses, messages.cc_addresses,
                messages.chat_version, messages.chat_group_id, messages.chat_group_name,
                COALESCE(json_extract(messages.trust_metadata, '$.root_encryption'), 0) AS encrypted,
                COALESCE(messages.is_sent, 0) AS is_sent, messages.auth_dmarc,
                COALESCE(json_extract(messages.trust_metadata, '$.auth_verified'), 0) AS auth_verified,
                (COALESCE({trash}, 0) OR COALESCE({spam}, 0)) AS binned
         FROM messages
         LEFT JOIN folders mailbox_folders
           ON mailbox_folders.id = COALESCE(messages.pending_folder_id, messages.folder_id)
         WHERE messages.thread_id = ? AND messages.is_draft = 0
         ORDER BY julianday(COALESCE(messages.received_at, messages.created_at)) ASC,
                  messages.id ASC"
    ))
    .bind(thread_id)
    .fetch_all(&mut *conn)
    .await?;
    if !rows
        .iter()
        .any(|row| row.chat_version.is_some() && row.binned == 0)
    {
        return Err(ChatError::NotAChat);
    }
    Ok(summarize(thread_id, account_id, account_name, &own, &rows))
}

/// Whether the thread holds a chat message.
pub async fn holds_chat(
    conn: &mut sqlx::SqliteConnection,
    thread_id: &str,
) -> Result<bool, sqlx::Error> {
    let found: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM messages WHERE thread_id = ? AND chat_version IS NOT NULL LIMIT 1",
    )
    .bind(thread_id)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(found.is_some())
}

/// Whether the thread `child` may be merged into `parent` (the reply merge).
///
/// Mail threads merge freely. A chat merges only to heal one that split
/// because a reply was fetched before its parent: into a chat, and only when
/// every sender in `child` is the user or already a trusted member of
/// `parent`. A stranger's thread is never pulled into a chat, and a chat is
/// never folded into plain mail.
pub async fn join_allowed(
    conn: &mut sqlx::SqliteConnection,
    child: &str,
    parent: &str,
) -> Result<bool, ChatError> {
    let child_chat = holds_chat(conn, child).await?;
    let parent_chat = holds_chat(conn, parent).await?;
    if !child_chat && !parent_chat {
        return Ok(true);
    }
    if !parent_chat {
        return Ok(false);
    }
    let chat = match chat_thread_in(conn, parent).await {
        Ok(chat) => chat,
        Err(ChatError::NotAChat) => return Ok(false),
        Err(error) => return Err(error),
    };
    let account_email: String = sqlx::query_scalar("SELECT email FROM accounts WHERE id = ?")
        .bind(&chat.account_id)
        .fetch_one(&mut *conn)
        .await?;
    let own = own_addresses(conn, &chat.account_id, &account_email).await?;
    let senders: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT LOWER(TRIM(from_address)) FROM messages WHERE thread_id = ?",
    )
    .bind(child)
    .fetch_all(&mut *conn)
    .await?;
    Ok(senders
        .iter()
        .all(|sender| own.contains(sender) || chat.recipients.contains(sender)))
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

/// `rows` are oldest first, by receipt time; rows in Trash or Spam are
/// flagged `binned` and count only as the chat's stored origin.
fn summarize(
    thread_id: &str,
    account_id: String,
    account_name: String,
    own: &[String],
    all_rows: &[ChatMessageRow],
) -> ChatThread {
    let is_own = |addr: &str| own.contains(&lower(addr));
    // The user's own: sent from here, or delivered with a DMARC pass the
    // receiving provider vouched for.
    let own_trusted = |row: &ChatMessageRow| {
        is_own(&row.from_address)
            && (row.is_sent != 0
                || (row.auth_verified != 0
                    && row
                        .auth_dmarc
                        .as_deref()
                        .is_some_and(|verdict| verdict.trim().eq_ignore_ascii_case("pass"))))
    };
    let spoofed_own = |row: &ChatMessageRow| is_own(&row.from_address) && !own_trusted(row);
    let others = |row: &ChatMessageRow| {
        addresses_of(row)
            .into_iter()
            .filter(|addr| !is_own(addr))
            .collect::<Vec<_>>()
    };
    let visible = |row: &ChatMessageRow| row.binned == 0;
    let rows = all_rows;
    let last_own = rows
        .iter()
        .rposition(|row| visible(row) && own_trusted(row));
    // Where the chat started: its earliest received chat message, even if it
    // is in the Trash now. Plain mail never starts a chat. This is a hint for
    // the first reply only, which always asks (`first_reply`): any mail that
    // threads in can be older than the real start.
    let origin = rows
        .iter()
        .position(|row| row.chat_version.is_some() && !spoofed_own(row));

    // A group if the user's last reply or the origin says so.
    let group_id = last_own
        .and_then(|index| rows[index].chat_group_id.clone())
        .or_else(|| origin.and_then(|index| rows[index].chat_group_id.clone()));

    let mut recipients: Vec<String> = Vec::new();
    let mut baseline: Vec<String> = Vec::new();
    match group_id.as_deref() {
        Some(group) => {
            let in_group = |row: &ChatMessageRow| row.chat_group_id.as_deref() == Some(group);
            let base = last_own
                .filter(|index| in_group(&rows[*index]))
                .or_else(|| origin.filter(|index| in_group(&rows[*index])));
            if let Some(base) = base {
                for addr in others(&rows[base]) {
                    push_unique(&mut recipients, addr);
                }
                baseline = recipients.clone();
                for row in rows[base + 1..]
                    .iter()
                    .filter(|row| visible(row) && in_group(row))
                {
                    // Only a member can bring others in.
                    if !spoofed_own(row) && recipients.contains(&lower(&row.from_address)) {
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
                None => origin.and_then(|index| {
                    let first = &rows[index];
                    if is_own(&first.from_address) {
                        others(first).into_iter().next()
                    } else {
                        Some(lower(&first.from_address))
                    }
                }),
            };
            recipients.extend(peer);
            baseline = recipients.clone();
        }
    }
    if let Some(index) = last_own {
        baseline = others(&rows[index]);
    }
    let recipients_changed = {
        let mut before = baseline.clone();
        let mut now = recipients.clone();
        before.sort();
        now.sort();
        before.dedup();
        now.dedup();
        before != now
    };
    // Owner decision (ADR 0020): until the user has a trusted reply in the
    // chat, every reply asks first, showing its recipients.
    let first_reply = last_own.is_none();

    let trusted = |row: &ChatMessageRow| {
        visible(row)
            && (own_trusted(row)
                || (!is_own(&row.from_address) && recipients.contains(&lower(&row.from_address))))
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
        .find_map(|row| row.chat_group_name.clone())
        .or_else(|| origin.and_then(|index| rows[index].chat_group_name.clone()));

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

    let encrypted_before = trusted_rows.iter().any(|row| row.encrypted != 0);
    let reply_to_message_id = latest_trusted.map(|row| row.id.clone());
    let untrusted_message_ids = rows
        .iter()
        .filter(|row| visible(row) && !trusted(row))
        .map(|row| row.id.clone())
        .collect();
    ChatThread {
        thread_id: thread_id.to_string(),
        account_id,
        account_name,
        own_address,
        group_id,
        group_name,
        recipients,
        baseline_recipients: baseline,
        recipients_changed,
        encrypted_before,
        untrusted_message_ids,
        first_reply,
        names,
        reply_to_message_id,
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
        for (id, kind) in [
            ("inbox", "inbox"),
            ("sent", "sent"),
            ("trash", "trash"),
            ("spam", "spam"),
        ] {
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
                    chat_version, chat_group_id, chat_group_name, trust_metadata, is_sent)
                 VALUES(?, 'acc', ?, ?, ?, ?, ?, 'Chat', ?, ?, ?, '', '', '1.0', ?, ?, ?, ?)",
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
            .bind(i64::from(folder == "sent"))
            .execute(pool)
            .await
            .unwrap();
        }

        async fn inbox(&self, pool: &SqlitePool) {
            self.store(pool, "inbox", self.received, false).await;
        }

        /// The user's own reply, as Satchel keeps it: in Sent.
        async fn sent(&self, pool: &SqlitePool) {
            self.store(pool, "sent", self.received, false).await;
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
        .sent(&pool)
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
        .store(&pool, "sent", "2026-08-01T11:00:00Z", true)
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
        .sent(&pool)
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
            chat.baseline_recipients,
            vec!["lena@example.test".to_string()]
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

    // ---- review repro (expected-safe behaviour; each FAILS on 1429a83) ----
    /// Unsafe: a stranger is a recipient and nobody is asked. `_case` only
    /// names the scenario in the call.
    fn silent(chat: &ChatThread, _case: &str) -> bool {
        chat.recipients.iter().any(|a| a.ends_with("@evil.test")) && !chat.recipients_changed
    }

    #[tokio::test]
    async fn repro_spoofed_own_address_must_not_set_the_baseline() {
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
        Msg {
            id: "selfspoof",
            thread: "group",
            received: "2026-08-01T11:00:00Z",
            from: ("Tom", "tom@example.test"),
            to: &["attacker@evil.test", "victim@evil.test"],
            chat: GROUP,
        }
        .inbox(&pool)
        .await;
        let chat = ChatRepository::new(&pool).thread("group").await.unwrap();
        assert!(
            !silent(&chat, "own"),
            "recipients={:?} changed={}",
            chat.recipients,
            chat.recipients_changed
        );
    }

    #[tokio::test]
    async fn repro_first_reply_after_a_member_message_adds_a_stranger_must_ask() {
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
        Msg {
            id: "spoof",
            thread: "group",
            received: "2026-08-01T11:00:00Z",
            from: ("Lena", "lena@example.test"),
            to: &["tom@example.test", "jonas@example.test", "victim@evil.test"],
            chat: GROUP,
        }
        .inbox(&pool)
        .await;
        let chat = ChatRepository::new(&pool).thread("group").await.unwrap();
        assert!(
            !silent(&chat, "member"),
            "recipients={:?} changed={}",
            chat.recipients,
            chat.recipients_changed
        );
    }

    #[tokio::test]
    async fn repro_a_one_to_one_must_not_become_a_stranger_group_silently() {
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
        Msg {
            id: "evil",
            thread: "one",
            received: "2026-08-01T11:00:00Z",
            from: ("Lena", "attacker@evil.test"),
            to: &["tom@example.test", "victim@evil.test"],
            chat: GROUP,
        }
        .inbox(&pool)
        .await;
        let chat = ChatRepository::new(&pool).thread("one").await.unwrap();
        assert!(
            !silent(&chat, "stranger"),
            "recipients={:?} changed={}",
            chat.recipients,
            chat.recipients_changed
        );
    }

    #[tokio::test]
    async fn repro_a_plain_message_from_a_member_must_not_lift_the_downgrade_guard() {
        let pool = pool().await;
        Msg {
            id: "m1",
            thread: "one",
            received: "2026-08-01T10:00:00Z",
            from: ("Lena", "lena@example.test"),
            to: &["tom@example.test"],
            chat: None,
        }
        .store(&pool, "inbox", "2026-08-01T10:00:00Z", true)
        .await;
        Msg {
            id: "spoof",
            thread: "one",
            received: "2026-08-01T11:00:00Z",
            from: ("Lena", "lena@example.test"),
            to: &["tom@example.test"],
            chat: None,
        }
        .inbox(&pool)
        .await;
        let chat = ChatRepository::new(&pool).thread("one").await.unwrap();
        assert!(
            chat.encrypted_before,
            "an unauthenticated plain message reset encrypted_before"
        );
    }

    #[tokio::test]
    async fn repro_ordering_follows_receipt_time_not_date() {
        let pool = pool().await;
        // Lena's real first message is received at 10:00 with an honest Date.
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
        // Jonas (a member) is received later but carries a Date from 2020: with Date ordering
        // his message would count as the oldest and seed the group.
        Msg {
            id: "g2",
            thread: "group",
            received: "2026-08-01T11:00:00Z",
            from: ("Jonas", "jonas@example.test"),
            to: &["tom@example.test", "lena@example.test", "mia@example.test"],
            chat: GROUP,
        }
        .store(&pool, "inbox", "2020-01-01T00:00:00Z", false)
        .await;
        let chat = ChatRepository::new(&pool).thread("group").await.unwrap();
        // Receipt order: seed {lena, jonas}, then jonas adds mia.
        assert_eq!(
            chat.recipients,
            vec![
                "lena@example.test",
                "jonas@example.test",
                "mia@example.test"
            ]
        );
    }

    #[tokio::test]
    async fn an_own_message_counts_when_sent_or_dmarc_passed_and_strangers_are_marked() {
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
        // Delta Chat on another device: in the Inbox, but DMARC passed.
        Msg {
            id: "g2",
            thread: "group",
            received: "2026-08-01T11:00:00Z",
            from: ("Tom", "tom@example.test"),
            to: &["lena@example.test"],
            chat: GROUP,
        }
        .inbox(&pool)
        .await;
        sqlx::query("UPDATE messages SET auth_dmarc = 'pass', trust_metadata = '{\"auth_verified\":true}' WHERE id = 'g2'")
            .execute(&pool)
            .await
            .unwrap();
        Msg {
            id: "x",
            thread: "group",
            received: "2026-08-01T12:00:00Z",
            from: ("Mia", "spam@evil.test"),
            to: &["tom@example.test"],
            chat: GROUP,
        }
        .inbox(&pool)
        .await;
        let chat = ChatRepository::new(&pool).thread("group").await.unwrap();
        assert_eq!(chat.recipients, vec!["lena@example.test"]);
        assert_eq!(chat.baseline_recipients, vec!["lena@example.test"]);
        assert!(!chat.recipients_changed);
        assert_eq!(chat.untrusted_message_ids, vec!["x".to_string()]);
    }

    // ---- third review: probe cases (zz_review_chat_probe.rs) ----

    /// A row with Message-ID and In-Reply-To, in any thread, for merge cases.
    #[allow(clippy::too_many_arguments)]
    async fn linked(
        pool: &SqlitePool,
        id: &str,
        thread: &str,
        mid: &str,
        irt: Option<&str>,
        received: &str,
        from: &str,
        to: &[&str],
        chat: Option<Option<&str>>,
    ) {
        sqlx::query(
            "INSERT INTO threads(id, account_id, subject, last_message_date, message_count, unread_count, created_at, updated_at)
             VALUES(?, 'acc', 'Chat', '2026-08-01T00:00:00Z', 1, 0, '', '') ON CONFLICT(id) DO NOTHING",
        )
        .bind(thread)
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO messages(id, account_id, folder_id, thread_id, message_id_header, in_reply_to,
                from_address, from_name, to_addresses, subject, date, received_at, remote_id,
                created_at, updated_at, chat_version, chat_group_id, chat_group_name, trust_metadata)
             VALUES(?, 'acc', 'inbox', ?, ?, ?, ?, ?, ?, 'Message from Lena', ?, ?, ?, ?, '', ?, ?, ?, '{}')",
        )
        .bind(id)
        .bind(thread)
        .bind(mid)
        .bind(irt)
        .bind(from)
        .bind(from)
        .bind(serde_json::to_string(to).unwrap())
        .bind(received)
        .bind(received)
        .bind(id)
        .bind(received)
        .bind(chat.map(|_| "1.0"))
        .bind(chat.flatten())
        .bind(chat.flatten().map(|_| "G"))
        .execute(pool)
        .await
        .unwrap();
    }

    async fn thread_of(pool: &SqlitePool, id: &str) -> String {
        sqlx::query_scalar("SELECT thread_id FROM messages WHERE id = ?")
            .bind(id)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn a_reply_merge_never_pulls_a_strangers_older_thread_into_a_chat() {
        let pool = pool().await;
        // The attacker's older chat message, "replying" to an invented <x>.
        linked(
            &pool,
            "a1",
            "th-imap-A",
            "<a1>",
            Some("<x>"),
            "2026-07-01T09:00:00Z",
            "attacker@evil.test",
            &["tom@example.test", "victim@evil.test"],
            Some(Some("EVILGROUP123")),
        )
        .await;
        linked(
            &pool,
            "y",
            "th-imap-B",
            "<y>",
            None,
            "2026-08-01T10:00:00Z",
            "lena@example.test",
            &["tom@example.test"],
            Some(None),
        )
        .await;
        // M carries Message-ID <x> and answers Lena's <y>.
        linked(
            &pool,
            "m",
            "th-imap-B",
            "<x>",
            Some("<y>"),
            "2026-08-01T11:00:00Z",
            "attacker@evil.test",
            &["tom@example.test"],
            Some(None),
        )
        .await;
        crate::sync::thread_resolution::merge_reply_split_threads(&pool)
            .await
            .unwrap();
        assert_eq!(thread_of(&pool, "a1").await, "th-imap-A");
        let chat = ChatRepository::new(&pool)
            .thread("th-imap-B")
            .await
            .unwrap();
        assert_eq!(chat.group_id, None);
        assert_eq!(chat.recipients, vec!["lena@example.test"]);
        assert_eq!(chat.untrusted_message_ids, vec!["m".to_string()]);
    }

    #[tokio::test]
    async fn a_reply_merge_never_pulls_older_plain_mail_into_a_chat() {
        let pool = pool().await;
        linked(
            &pool,
            "a1",
            "th-imap-A",
            "<a1>",
            Some("<x>"),
            "2026-07-01T09:00:00Z",
            "attacker@evil.test",
            &["tom@example.test"],
            None,
        )
        .await;
        linked(
            &pool,
            "y",
            "th-imap-B",
            "<y>",
            None,
            "2026-08-01T10:00:00Z",
            "lena@example.test",
            &["tom@example.test"],
            Some(None),
        )
        .await;
        linked(
            &pool,
            "m",
            "th-imap-B",
            "<x>",
            Some("<y>"),
            "2026-08-01T11:00:00Z",
            "attacker@evil.test",
            &["tom@example.test"],
            Some(None),
        )
        .await;
        crate::sync::thread_resolution::merge_reply_split_threads(&pool)
            .await
            .unwrap();
        assert_eq!(thread_of(&pool, "a1").await, "th-imap-A");
        let chat = ChatRepository::new(&pool)
            .thread("th-imap-B")
            .await
            .unwrap();
        assert_eq!(chat.recipients, vec!["lena@example.test"]);
    }

    #[tokio::test]
    async fn after_the_users_reply_a_merge_attempt_changes_nothing() {
        let pool = pool().await;
        linked(
            &pool,
            "a1",
            "th-imap-A",
            "<a1>",
            Some("<x>"),
            "2026-07-01T09:00:00Z",
            "attacker@evil.test",
            &["tom@example.test", "victim@evil.test"],
            Some(Some("EVILGROUP123")),
        )
        .await;
        linked(
            &pool,
            "y",
            "th-imap-B",
            "<y>",
            None,
            "2026-08-01T10:00:00Z",
            "lena@example.test",
            &["tom@example.test"],
            Some(None),
        )
        .await;
        linked(
            &pool,
            "r",
            "th-imap-B",
            "<r>",
            Some("<y>"),
            "2026-08-01T10:30:00Z",
            "tom@example.test",
            &["lena@example.test"],
            Some(None),
        )
        .await;
        sqlx::query("UPDATE messages SET folder_id = 'sent', is_sent = 1 WHERE id = 'r'")
            .execute(&pool)
            .await
            .unwrap();
        linked(
            &pool,
            "m",
            "th-imap-B",
            "<x>",
            Some("<y>"),
            "2026-08-01T11:00:00Z",
            "attacker@evil.test",
            &["tom@example.test"],
            Some(None),
        )
        .await;
        crate::sync::thread_resolution::merge_reply_split_threads(&pool)
            .await
            .unwrap();
        let chat = ChatRepository::new(&pool)
            .thread("th-imap-B")
            .await
            .unwrap();
        assert_eq!(chat.group_id, None);
        assert_eq!(chat.recipients, vec!["lena@example.test"]);
        assert!(!chat.recipients_changed);
    }

    #[tokio::test]
    async fn an_older_strangers_chat_message_in_the_thread_needs_a_confirmed_first_reply() {
        let pool = pool().await;
        // However the stranger's message got into the thread (an older
        // provider conversation, a merge before the guard), it is older than
        // the chat's real start. Satchel cannot tell, so the first reply asks.
        linked(
            &pool,
            "a1",
            "th-imap-B",
            "<a1>",
            None,
            "2026-07-01T09:00:00Z",
            "attacker@evil.test",
            &["tom@example.test", "victim@evil.test"],
            Some(Some("EVILGROUP123")),
        )
        .await;
        linked(
            &pool,
            "y",
            "th-imap-B",
            "<y>",
            None,
            "2026-08-01T10:00:00Z",
            "lena@example.test",
            &["tom@example.test"],
            Some(None),
        )
        .await;
        let chat = ChatRepository::new(&pool)
            .thread("th-imap-B")
            .await
            .unwrap();
        assert!(chat.first_reply, "confirmation required");
    }

    #[tokio::test]
    async fn a_one_to_one_first_reply_asks_and_then_follows_the_trusted_reply() {
        let pool = pool().await;
        linked(
            &pool,
            "y",
            "th-imap-B",
            "<y>",
            None,
            "2026-08-01T10:00:00Z",
            "lena@example.test",
            &["tom@example.test"],
            Some(None),
        )
        .await;
        let chat = ChatRepository::new(&pool)
            .thread("th-imap-B")
            .await
            .unwrap();
        assert!(chat.first_reply);
        linked(
            &pool,
            "r",
            "th-imap-B",
            "<r>",
            Some("<y>"),
            "2026-08-01T11:00:00Z",
            "tom@example.test",
            &["lena@example.test"],
            Some(None),
        )
        .await;
        sqlx::query("UPDATE messages SET folder_id = 'sent', is_sent = 1 WHERE id = 'r'")
            .execute(&pool)
            .await
            .unwrap();
        let chat = ChatRepository::new(&pool)
            .thread("th-imap-B")
            .await
            .unwrap();
        assert!(!chat.first_reply);
        assert_eq!(chat.recipients, vec!["lena@example.test"]);
    }

    #[tokio::test]
    async fn a_chat_heals_only_with_its_own_members() {
        let pool = pool().await;
        // Lena's reply was fetched before her first message: two threads.
        linked(
            &pool,
            "m1",
            "th-imap-R",
            "<m1>",
            Some("<m0>"),
            "2026-08-02T10:00:00Z",
            "lena@example.test",
            &["tom@example.test"],
            Some(None),
        )
        .await;
        linked(
            &pool,
            "m0",
            "th-imap-P",
            "<m0>",
            None,
            "2026-08-01T10:00:00Z",
            "lena@example.test",
            &["tom@example.test"],
            Some(None),
        )
        .await;
        let mut conn = pool.acquire().await.unwrap();
        assert!(join_allowed(&mut conn, "th-imap-R", "th-imap-P")
            .await
            .unwrap());
        // A stranger's chat thread does not join, nor does a chat fold into mail.
        drop(conn);
        linked(
            &pool,
            "s",
            "th-imap-S",
            "<s>",
            Some("<m0>"),
            "2026-08-02T11:00:00Z",
            "attacker@evil.test",
            &["tom@example.test"],
            Some(None),
        )
        .await;
        linked(
            &pool,
            "plain",
            "th-imap-M",
            "<plain>",
            None,
            "2026-08-01T09:00:00Z",
            "lena@example.test",
            &["tom@example.test"],
            None,
        )
        .await;
        let mut conn = pool.acquire().await.unwrap();
        assert!(!join_allowed(&mut conn, "th-imap-S", "th-imap-P")
            .await
            .unwrap());
        assert!(!join_allowed(&mut conn, "th-imap-R", "th-imap-M")
            .await
            .unwrap());
        // Plain mail merges as before.
        drop(conn);
        linked(
            &pool,
            "plain2",
            "th-imap-N",
            "<plain2>",
            Some("<plain>"),
            "2026-08-01T09:30:00Z",
            "x@example.test",
            &["tom@example.test"],
            None,
        )
        .await;
        let mut conn = pool.acquire().await.unwrap();
        assert!(join_allowed(&mut conn, "th-imap-N", "th-imap-M")
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn older_plain_mail_in_the_same_conversation_does_not_start_the_chat() {
        let pool = pool().await;
        // Gmail/Graph conversation: plain mail with the same subject is older.
        linked(
            &pool,
            "p",
            "th-gmail-c",
            "<p>",
            None,
            "2026-07-01T09:00:00Z",
            "attacker@evil.test",
            &["tom@example.test"],
            None,
        )
        .await;
        linked(
            &pool,
            "y",
            "th-gmail-c",
            "<y>",
            None,
            "2026-08-01T10:00:00Z",
            "lena@example.test",
            &["tom@example.test"],
            Some(None),
        )
        .await;
        let chat = ChatRepository::new(&pool)
            .thread("th-gmail-c")
            .await
            .unwrap();
        assert_eq!(chat.recipients, vec!["lena@example.test"]);
        assert_eq!(chat.untrusted_message_ids, vec!["p".to_string()]);
    }

    #[tokio::test]
    async fn a_trashed_origin_still_defines_the_chat() {
        let pool = pool().await;
        linked(
            &pool,
            "g1",
            "th-imap-G",
            "<g1>",
            None,
            "2026-08-01T10:00:00Z",
            "lena@example.test",
            &["tom@example.test", "jonas@example.test"],
            Some(Some("abcDEF123_-x")),
        )
        .await;
        sqlx::query("UPDATE messages SET folder_id = 'trash' WHERE id = 'g1'")
            .execute(&pool)
            .await
            .unwrap();
        linked(
            &pool,
            "e",
            "th-imap-G",
            "<e>",
            Some("<g1>"),
            "2026-08-01T11:00:00Z",
            "attacker@evil.test",
            &["tom@example.test", "victim@evil.test"],
            Some(Some("abcDEF123_-x")),
        )
        .await;
        let chat = ChatRepository::new(&pool)
            .thread("th-imap-G")
            .await
            .unwrap();
        assert_eq!(
            chat.recipients,
            vec!["lena@example.test", "jonas@example.test"]
        );
        assert_eq!(chat.untrusted_message_ids, vec!["e".to_string()]);
    }

    #[tokio::test]
    async fn an_unsolicited_one_to_one_goes_back_to_its_sender_only() {
        let pool = pool().await;
        linked(
            &pool,
            "f1",
            "th-imap-B",
            "<f1>",
            None,
            "2026-08-01T10:00:00Z",
            "attacker@evil.test",
            &["tom@example.test"],
            Some(None),
        )
        .await;
        sqlx::query("UPDATE messages SET cc_addresses = '[\"victim@evil.test\"]' WHERE id = 'f1'")
            .execute(&pool)
            .await
            .unwrap();
        let chat = ChatRepository::new(&pool)
            .thread("th-imap-B")
            .await
            .unwrap();
        assert_eq!(chat.recipients, vec!["attacker@evil.test"]);
        // The first reply to someone who wrote first still asks.
        assert!(chat.first_reply, "confirmation required");
    }

    #[tokio::test]
    async fn the_first_reply_in_a_group_always_asks() {
        let pool = pool().await;
        linked(
            &pool,
            "f1",
            "th-imap-B",
            "<f1>",
            None,
            "2026-08-01T10:00:00Z",
            "attacker@evil.test",
            &["tom@example.test", "victim@evil.test"],
            Some(Some("EVILGROUP123")),
        )
        .await;
        let chat = ChatRepository::new(&pool)
            .thread("th-imap-B")
            .await
            .unwrap();
        assert_eq!(
            chat.recipients,
            vec!["attacker@evil.test", "victim@evil.test"]
        );
        assert!(chat.first_reply);
        // Once the user has replied in the group, the next reply does not.
        linked(
            &pool,
            "r",
            "th-imap-B",
            "<r>",
            Some("<f1>"),
            "2026-08-01T11:00:00Z",
            "tom@example.test",
            &["attacker@evil.test", "victim@evil.test"],
            Some(Some("EVILGROUP123")),
        )
        .await;
        sqlx::query("UPDATE messages SET folder_id = 'sent', is_sent = 1 WHERE id = 'r'")
            .execute(&pool)
            .await
            .unwrap();
        let chat = ChatRepository::new(&pool)
            .thread("th-imap-B")
            .await
            .unwrap();
        assert!(!chat.first_reply);
    }

    #[tokio::test]
    async fn a_dmarc_pass_counts_only_when_the_provider_vouched_for_it() {
        let pool = pool().await;
        linked(
            &pool,
            "y",
            "th-imap-B",
            "<y>",
            None,
            "2026-08-01T10:00:00Z",
            "lena@example.test",
            &["tom@example.test"],
            Some(None),
        )
        .await;
        // An old row: auth_dmarc says pass, trust_metadata says nothing.
        linked(
            &pool,
            "s",
            "th-imap-B",
            "<s>",
            Some("<y>"),
            "2026-08-01T11:00:00Z",
            "tom@example.test",
            &["attacker@evil.test"],
            None,
        )
        .await;
        sqlx::query("UPDATE messages SET auth_dmarc = 'pass' WHERE id = 's'")
            .execute(&pool)
            .await
            .unwrap();
        let chat = ChatRepository::new(&pool)
            .thread("th-imap-B")
            .await
            .unwrap();
        assert_eq!(chat.recipients, vec!["lena@example.test"]);
        assert_eq!(chat.baseline_recipients, vec!["lena@example.test"]);
        assert_eq!(chat.untrusted_message_ids, vec!["s".to_string()]);
        // The same verdict through the provider's trust boundary counts.
        sqlx::query(
            "UPDATE messages SET trust_metadata = '{\"auth_verified\":true}' WHERE id = 's'",
        )
        .execute(&pool)
        .await
        .unwrap();
        let chat = ChatRepository::new(&pool)
            .thread("th-imap-B")
            .await
            .unwrap();
        assert_eq!(chat.recipients, vec!["attacker@evil.test"]);
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
