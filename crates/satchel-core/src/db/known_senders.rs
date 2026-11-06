//! Senders the user knows: everyone they have written to, and everyone in a
//! synced address book.
//!
//! Machine-shaped addresses and subject words are guesses. Having written to
//! someone, or keeping them in the address book, is not — so a known sender
//! outranks those guesses in the smart Inbox and skips the new-sender review.
//! Explicit choices (the provider's category, a sender or thread the user
//! filed by hand) still win over it.
//!
//! The match is always the exact address. A display name never makes a sender
//! known: anyone can call themselves by a contact's name.

use futures::future::BoxFuture;
use sqlx::{SqliteConnection, SqlitePool};

use super::data_repairs::{DataRepair, Prepared};

/// An address book whose entries make their senders known. Recipients of
/// sent mail are not one: triggers on `messages` keep those (`sent:<account>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressBook<'a> {
    /// The macOS Contacts store; it belongs to no account.
    MacOs,
    /// The contacts of one account (`kind` is `gmail`, `outlook`, `carddav`).
    Account { kind: &'a str, account_id: &'a str },
}

impl AddressBook<'_> {
    pub fn source(&self) -> String {
        match self {
            Self::MacOs => "macos".to_owned(),
            Self::Account { kind, account_id } => format!("{kind}:{account_id}"),
        }
    }

    fn account_id(&self) -> Option<&str> {
        match self {
            Self::MacOs => None,
            Self::Account { account_id, .. } => Some(account_id),
        }
    }
}

/// Replace everything `book` contributed with `emails`.
///
/// Call it only after the whole address book was read: a contact deleted
/// there must stop counting here, and a partial read would drop the rest.
pub async fn replace_address_book<I, S>(
    pool: &SqlitePool,
    book: AddressBook<'_>,
    emails: I,
) -> Result<usize, sqlx::Error>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let source = book.source();
    let account_id = book.account_id();
    let emails = emails
        .into_iter()
        .filter_map(|email| normalize(email.as_ref()))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let now = chrono::Utc::now().to_rfc3339();
    let mut transaction = pool.begin().await?;
    sqlx::query("DELETE FROM known_senders WHERE source = ?")
        .bind(&source)
        .execute(&mut *transaction)
        .await?;
    for chunk in emails.chunks(250) {
        let mut query = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
            "INSERT INTO known_senders (email, source, account_id, updated_at) ",
        );
        query.push_values(chunk, |mut row, email| {
            row.push_bind(email)
                .push_bind(&source)
                .push_bind(account_id)
                .push_bind(&now);
        });
        query.push(" ON CONFLICT (email, source) DO NOTHING");
        query.build().execute(&mut *transaction).await?;
    }
    transaction.commit().await?;
    Ok(emails.len())
}

fn normalize(email: &str) -> Option<String> {
    let email = email.trim().trim_matches(['<', '>']).to_ascii_lowercase();
    (email.find('@').is_some_and(|at| at > 0) && email.len() <= 320).then_some(email)
}

/// Recipients of mail sent before migration 0127. Later mail is covered by
/// the triggers.
pub const KNOWN_SENDERS_SENT: DataRepair = DataRepair {
    name: "known_senders_sent",
    block: known_senders_sent_block,
    prepare: None,
};

fn known_senders_sent_block<'c>(
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
        sqlx::query(include_str!("known_senders_backfill.sql"))
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
    use sqlx::sqlite::SqlitePoolOptions;

    async fn pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("pool");
        crate::db::init(&pool).await.expect("migrations");
        sqlx::query(
            "INSERT INTO accounts(id, display_name, email, provider, config, credentials_ref, is_active, created_at, updated_at)
             VALUES('acc-1', 'Me', 'me@example.test', 'imap', '{}', '', 1, '2026-01-01', '2026-01-01');
             INSERT INTO folders(id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at)
             VALUES('fld-sent', 'acc-1', 'Sent', 'Sent', 'sent', 'Sent', '2026-01-01', '2026-01-01');
             INSERT INTO threads(id, account_id, subject, last_message_date, message_count, unread_count, created_at, updated_at)
             VALUES('th-1', 'acc-1', 'Hello', '2026-01-01T00:00:00Z', 1, 0, '2026-01-01', '2026-01-01');",
        )
        .execute(&pool)
        .await
        .expect("seed");
        pool
    }

    async fn insert_message(pool: &SqlitePool, id: &str, is_sent: bool, is_draft: bool, to: &str) {
        sqlx::query(
            "INSERT INTO messages (id, account_id, folder_id, thread_id, subject, from_address,
                                   to_addresses, cc_addresses, bcc_addresses, date, is_sent, is_draft,
                                   remote_id, created_at, updated_at)
             VALUES (?, 'acc-1', 'fld-sent', 'th-1', 'Hello', 'me@example.test', ?,
                     '[\"Copy <Copy@Example.Test>\"]', '[]', '2026-01-01T00:00:00Z', ?, ?,
                     ?, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(id)
        .bind(to)
        .bind(is_sent)
        .bind(is_draft)
        .bind(id)
        .execute(pool)
        .await
        .expect("message");
    }

    async fn is_known<'e, E>(executor: E, email: &str) -> Result<bool, sqlx::Error>
    where
        E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
    {
        let Some(email) = normalize(email) else {
            return Ok(false);
        };
        sqlx::query_scalar::<_, i64>("SELECT EXISTS (SELECT 1 FROM known_senders WHERE email = ?)")
            .bind(email)
            .fetch_one(executor)
            .await
            .map(|known| known > 0)
    }

    async fn sources(pool: &SqlitePool, email: &str) -> Vec<String> {
        sqlx::query_scalar("SELECT source FROM known_senders WHERE email = ? ORDER BY source")
            .bind(email)
            .fetch_all(pool)
            .await
            .expect("sources")
    }

    #[tokio::test]
    async fn sent_mail_makes_its_recipients_known_but_drafts_and_inbox_mail_do_not() {
        let pool = pool().await;
        insert_message(&pool, "m-sent", true, false, "[\" Person@Example.Test \"]").await;
        insert_message(&pool, "m-draft", true, true, "[\"draft@example.test\"]").await;
        insert_message(
            &pool,
            "m-received",
            false,
            false,
            "[\"received@example.test\"]",
        )
        .await;

        assert_eq!(sources(&pool, "person@example.test").await, ["sent:acc-1"]);
        assert_eq!(sources(&pool, "copy@example.test").await, ["sent:acc-1"]);
        assert!(sources(&pool, "draft@example.test").await.is_empty());
        assert!(sources(&pool, "received@example.test").await.is_empty());

        // Sending the draft later counts.
        sqlx::query("UPDATE messages SET is_draft = 0 WHERE id = 'm-draft'")
            .execute(&pool)
            .await
            .expect("send draft");
        assert!(is_known(&pool, "DRAFT@example.test").await.expect("known"));
    }

    #[tokio::test]
    async fn the_repair_learns_recipients_of_mail_sent_before_the_triggers() {
        let pool = pool().await;
        insert_message(&pool, "m-old", true, false, "[\"old@example.test\"]").await;
        sqlx::query("DELETE FROM known_senders")
            .execute(&pool)
            .await
            .expect("forget");

        let mut connection = pool.acquire().await.expect("connection");
        let cursor = (KNOWN_SENDERS_SENT.block)(&mut connection, "", 100, None)
            .await
            .expect("block");
        assert!(cursor.is_some());
        let done =
            (KNOWN_SENDERS_SENT.block)(&mut connection, cursor.as_deref().unwrap(), 100, None)
                .await
                .expect("block");
        assert_eq!(done, None);
        drop(connection);

        assert_eq!(sources(&pool, "old@example.test").await, ["sent:acc-1"]);
        assert_eq!(sources(&pool, "copy@example.test").await, ["sent:acc-1"]);
    }

    #[tokio::test]
    async fn an_address_book_replaces_only_its_own_rows_and_leaves_with_its_account() {
        let pool = pool().await;
        insert_message(&pool, "m-sent", true, false, "[\"both@example.test\"]").await;
        let gmail = AddressBook::Account {
            kind: "gmail",
            account_id: "acc-1",
        };

        replace_address_book(
            &pool,
            gmail,
            ["Both@Example.Test", "gone@example.test", "no-at"],
        )
        .await
        .expect("first sync");
        replace_address_book(&pool, AddressBook::MacOs, ["mac@example.test"])
            .await
            .expect("mac sync");
        assert_eq!(
            sources(&pool, "both@example.test").await,
            ["gmail:acc-1", "sent:acc-1"]
        );
        assert!(is_known(&pool, "gone@example.test").await.expect("known"));
        assert!(!is_known(&pool, "no-at").await.expect("known"));

        // The contact was deleted in Google: it stops counting from there,
        // while the sent recipient and the other address book stay.
        replace_address_book(&pool, gmail, ["both@example.test"])
            .await
            .expect("second sync");
        assert!(!is_known(&pool, "gone@example.test").await.expect("known"));
        assert_eq!(
            sources(&pool, "both@example.test").await,
            ["gmail:acc-1", "sent:acc-1"]
        );

        // Removing the account takes what it contributed along.
        crate::db::accounts::AccountRepository::new(&pool)
            .delete("acc-1")
            .await
            .expect("delete account");
        assert!(!is_known(&pool, "both@example.test").await.expect("known"));
        assert!(is_known(&pool, "mac@example.test").await.expect("known"));
    }
}
