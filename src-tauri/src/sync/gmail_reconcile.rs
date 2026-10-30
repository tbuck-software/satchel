//! Gmail label semantics and mailbox reconciliation.
//!
//! Gmail exposes folders as labels: state views (IMPORTANT, STARRED, …) and
//! category views (CATEGORY_*) are not real folders, messages can carry many
//! labels at once, and a message without any real label belongs in Satchel's
//! synthetic ARCHIVE folder. Everything the sync loop needs to translate
//! between those two worlds lives here.

use std::collections::HashSet;

use crate::providers::traits::MessageEnvelope;
use sqlx::SqlitePool;

pub use crate::providers::gmail::gmail_label_is_state_view;

#[cfg(test)]
use super::folder_reconcile::reconcile_external_workflow_states;

pub fn gmail_labels_include(mailbox_ids: &[String], folder_id: &str) -> bool {
    mailbox_ids
        .iter()
        .any(|label| label.eq_ignore_ascii_case(folder_id.trim()))
}

pub fn gmail_label_is_category_view(label: &str) -> bool {
    matches!(
        label.trim().to_ascii_uppercase().as_str(),
        "CATEGORY_PERSONAL"
            | "CATEGORY_PROMOTIONS"
            | "CATEGORY_SOCIAL"
            | "CATEGORY_UPDATES"
            | "CATEGORY_FORUMS"
    )
}

pub fn gmail_message_belongs_to_folder(message: &MessageEnvelope, folder_id: &str) -> bool {
    gmail_labels_include(&message.mailbox_ids, folder_id)
}

/// Called only after a target copy has been stored, in the same savepoint.
/// A rollback retains the old copy, attachments and thread organization.
pub(crate) async fn reconcile_gmail_message_mailboxes_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    account_id: &str,
    message: &MessageEnvelope,
) -> Result<HashSet<String>, String> {
    let mut effective_mailbox_ids = gmail_effective_mailbox_ids(&message.mailbox_ids);
    if effective_mailbox_ids.is_empty() {
        effective_mailbox_ids.push("ARCHIVE".to_string());
    }
    let mailbox_ids =
        serde_json::to_string(&effective_mailbox_ids).map_err(|err| err.to_string())?;
    // Also guard standalone callers: observing a relabel is no evidence that
    // its target was successfully stored.
    let has_target: bool = sqlx::query_scalar(
        "SELECT EXISTS(
           SELECT 1 FROM messages m JOIN folders f ON f.id = m.folder_id
           WHERE m.account_id = ? AND m.remote_id = ?
             AND EXISTS (SELECT 1 FROM json_each(?) label
                         WHERE LOWER(TRIM(label.value)) = LOWER(TRIM(f.remote_id)))
         )",
    )
    .bind(account_id)
    .bind(&message.remote_id)
    .bind(&mailbox_ids)
    .fetch_one(&mut **tx)
    .await
    .map_err(|err| err.to_string())?;
    if !has_target {
        return Ok(HashSet::new());
    }
    let touched = sqlx::query_scalar::<_, String>(
        "SELECT DISTINCT thread_id FROM messages
         WHERE account_id = ? AND remote_id = ? AND thread_id IS NOT NULL",
    )
    .bind(account_id)
    .bind(&message.remote_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(|err| err.to_string())?
    .into_iter()
    .collect::<HashSet<_>>();

    sqlx::query(
        "DELETE FROM attachments
         WHERE message_id IN (
           SELECT stale.id
           FROM messages stale
           JOIN folders stale_folder ON stale_folder.id = stale.folder_id
           WHERE stale.account_id = ?
             AND stale.remote_id = ?
             AND NOT EXISTS (
               SELECT 1
               FROM json_each(?) current_label
               WHERE LOWER(TRIM(current_label.value)) = LOWER(TRIM(stale_folder.remote_id))
             )
         )",
    )
    .bind(account_id)
    .bind(&message.remote_id)
    .bind(&mailbox_ids)
    .execute(&mut **tx)
    .await
    .map_err(|err| err.to_string())?;

    let removed_stale_copies = sqlx::query(
        "DELETE FROM messages
         WHERE id IN (
           SELECT stale.id
           FROM messages stale
           JOIN folders stale_folder ON stale_folder.id = stale.folder_id
           WHERE stale.account_id = ?
             AND stale.remote_id = ?
             AND NOT EXISTS (
               SELECT 1
               FROM json_each(?) current_label
               WHERE LOWER(TRIM(current_label.value)) = LOWER(TRIM(stale_folder.remote_id))
             )
         )",
    )
    .bind(account_id)
    .bind(&message.remote_id)
    .bind(&mailbox_ids)
    .execute(&mut **tx)
    .await
    .map_err(|err| err.to_string())?
    .rows_affected();

    if removed_stale_copies > 0 {
        Ok(touched)
    } else {
        Ok(HashSet::new())
    }
}

#[cfg(test)]
pub async fn reconcile_gmail_message_mailboxes(
    pool: &SqlitePool,
    account_id: &str,
    message: &MessageEnvelope,
) -> Result<HashSet<String>, String> {
    let threads = sqlx::query_scalar::<_, String>(
        "SELECT DISTINCT thread_id FROM messages WHERE account_id = ? AND remote_id = ? AND thread_id IS NOT NULL",
    ).bind(account_id).bind(&message.remote_id).fetch_all(pool).await
        .map_err(|err| err.to_string())?.into_iter().collect();
    let mut tx = pool.begin().await.map_err(|err| err.to_string())?;
    let touched = reconcile_gmail_message_mailboxes_tx(&mut tx, account_id, message).await?;
    satchel_core::db::threads::refresh_thread_projections_tx(
        &mut tx,
        &touched.iter().cloned().collect::<Vec<_>>(),
    )
    .await
    .map_err(|err| err.to_string())?;
    tx.commit().await.map_err(|err| err.to_string())?;
    reconcile_external_workflow_states(pool, &threads).await?;
    Ok(touched)
}

/// The labels of a message that stand for a real local folder.
///
/// Category and state views are Gmail's own filters, not places a message
/// lives. Everything else is — INBOX, SENT, DRAFT, TRASH, SPAM, the Aside
/// label and every label the user made.
pub fn gmail_effective_mailbox_ids(mailbox_ids: &[String]) -> Vec<String> {
    mailbox_ids
        .iter()
        .filter(|label| !gmail_label_is_category_view(label) && !gmail_label_is_state_view(label))
        .cloned()
        .collect()
}

/// Whether a message has no real folder left and needs the synthetic ARCHIVE.
///
/// One definition for both callers. Naming the primary mailboxes separately
/// put a message whose only label was the user's own into ARCHIVE *and* into
/// that label — two `messages` rows for one message, which only the thread
/// view ever collapsed again.
pub fn gmail_message_needs_synthetic_archive(message: &MessageEnvelope) -> bool {
    gmail_effective_mailbox_ids(&message.mailbox_ids).is_empty()
}

/// History can mention a target before its turn in folder discovery/ingest.
pub async fn ensure_gmail_label_folder(
    pool: &SqlitePool,
    account_id: &str,
    remote_id: &str,
) -> Result<(String, String), String> {
    if remote_id == "ARCHIVE" {
        return Ok((
            ensure_gmail_archive_folder(pool, account_id).await?,
            "archive".into(),
        ));
    }
    if let Some(folder) = sqlx::query_as::<_, (String, String)>(
        "SELECT id, folder_type FROM folders WHERE account_id = ? AND remote_id = ?",
    )
    .bind(account_id)
    .bind(remote_id)
    .fetch_optional(pool)
    .await
    .map_err(|err| err.to_string())?
    {
        return Ok(folder);
    }
    let id = satchel_core::db::folders::resolve_provider_folder_id(pool, account_id, remote_id)
        .await
        .map_err(|err| err.to_string())?;
    let kind =
        crate::providers::folder_mapping::provider_folder_type("gmail", remote_id, remote_id, None);
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO folders
         (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT DO NOTHING",
    )
    .bind(&id)
    .bind(account_id)
    .bind(remote_id)
    .bind(remote_id)
    .bind(kind)
    .bind(remote_id)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|err| err.to_string())?;
    Ok((id, kind.into()))
}

pub async fn ensure_gmail_archive_folder(
    pool: &SqlitePool,
    account_id: &str,
) -> Result<String, String> {
    if let Some(id) = sqlx::query_scalar::<_, String>(
        "SELECT id FROM folders
         WHERE account_id = ? AND folder_type = 'archive'
         ORDER BY updated_at DESC LIMIT 1",
    )
    .bind(account_id)
    .fetch_optional(pool)
    .await
    .map_err(|err| err.to_string())?
    {
        return Ok(id);
    }
    let id = satchel_core::db::folders::resolve_provider_folder_id(pool, account_id, "ARCHIVE")
        .await
        .map_err(|err| err.to_string())?;
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO folders
         (id, account_id, name, full_path, folder_type, remote_id,
          message_count, unread_count, sync_enabled, sort_order, created_at, updated_at)
         VALUES (?, ?, 'Archiviert', 'Archive', 'archive', 'ARCHIVE', 0, 0, 1, 0, ?, ?)
         ON CONFLICT DO NOTHING",
    )
    .bind(&id)
    .bind(account_id)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|err| err.to_string())?;
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::{gmail_message_needs_synthetic_archive, reconcile_gmail_message_mailboxes};
    use crate::providers::traits::MessageEnvelope;
    use sqlx::sqlite::SqlitePoolOptions;

    /// A minimal envelope; each test sets only the fields it cares about.
    fn envelope(remote_id: &str) -> MessageEnvelope {
        MessageEnvelope {
            remote_id: remote_id.to_string(),
            remote_uidvalidity: None,
            remote_thread_id: None,
            original_source: None,
            mailbox_ids: Vec::new(),
            message_id_header: None,
            in_reply_to: None,
            references: Vec::new(),
            subject: String::new(),
            snippet: String::new(),
            from_address: String::new(),
            from_name: None,
            to_addresses: Vec::new(),
            cc_addresses: Vec::new(),
            bcc_addresses: Vec::new(),
            reply_to: None,
            remote_draft_id: None,
            received_at_rfc3339: None,
            date_rfc3339: String::new(),
            body_text: None,
            body_html: None,
            is_read: true,
            is_starred: false,
            body_calendar: None,
            smime_encrypted: None,
            smime_signed: None,
            pgp_signed: None,
            trust: Default::default(),
            list_unsubscribe: None,
            list_unsubscribe_post: None,
            auth_spf: None,
            auth_dkim: None,
            auth_dmarc: None,
            attachments: Vec::new(),
        }
    }

    /// One definition of "needs the synthetic archive", checked directly.
    #[test]
    fn only_a_message_without_any_real_label_needs_the_synthetic_archive() {
        for labels in [
            vec![],
            vec!["CATEGORY_PROMOTIONS".to_string()],
            vec!["IMPORTANT".to_string(), "CATEGORY_UPDATES".to_string()],
        ] {
            let mut message = envelope("remote");
            message.mailbox_ids = labels.clone();
            assert!(
                gmail_message_needs_synthetic_archive(&message),
                "{labels:?}"
            );
        }

        for labels in [
            vec!["Label_7".to_string()],
            vec!["INBOX".to_string()],
            vec!["Label_7".to_string(), "CATEGORY_PROMOTIONS".to_string()],
        ] {
            let mut message = envelope("remote");
            message.mailbox_ids = labels.clone();
            assert!(
                !gmail_message_needs_synthetic_archive(&message),
                "{labels:?}"
            );
        }
    }

    /// A message archived in Gmail Web keeps the label the user gave it. It
    /// belongs in that label's folder, and nowhere else.
    #[tokio::test]
    async fn gmail_custom_label_only_message_is_not_synthesized_as_archive() {
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
        sqlx::query(
            "INSERT INTO accounts
             (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
             VALUES ('account', 'Me', 'me@example.com', 'gmail', '{}', '', ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("account");
        sqlx::query(
            "INSERT INTO folders
             (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at)
             VALUES ('kunden', 'account', 'Kunden', 'Label_7', 'custom', 'Label_7', ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("label folder");
        sqlx::query(
            "INSERT INTO threads
             (id, account_id, subject, last_message_date, workflow_state, created_at, updated_at)
             VALUES ('thread', 'account', 'Subject', ?, 'inbox', ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("thread");
        sqlx::query(
            "INSERT INTO messages
             (id, account_id, folder_id, thread_id, from_address, to_addresses, subject, date, remote_id, created_at, updated_at)
             VALUES ('message', 'account', 'kunden', 'thread', 'sender@example.com', '[]', 'Subject', ?, 'remote', ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("message");

        let mut message = envelope("remote");
        message.mailbox_ids = vec!["Label_7".to_string()];
        reconcile_gmail_message_mailboxes(&pool, "account", &message)
            .await
            .expect("reconcile custom label");

        let (rows, folder_id, archive_folders): (i64, String, i64) = sqlx::query_as(
            "SELECT (SELECT COUNT(*) FROM messages),
                    (SELECT folder_id FROM messages),
                    (SELECT COUNT(*) FROM folders WHERE folder_type = 'archive')",
        )
        .fetch_one(&pool)
        .await
        .expect("result");
        assert_eq!(
            (rows, folder_id.as_str(), archive_folders),
            (1, "kunden", 0)
        );
    }

    #[tokio::test]
    async fn gmail_aside_only_message_is_not_synthesized_as_archive() {
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
        sqlx::query(
            "INSERT INTO accounts
             (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
             VALUES ('account', 'Me', 'me@example.com', 'gmail', '{}', '', ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("account");
        sqlx::query(
            "INSERT INTO folders
             (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at)
             VALUES ('aside', 'account', 'Aside', 'Label_123', 'aside', 'Label_123', ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("aside folder");
        sqlx::query(
            "INSERT INTO threads
             (id, account_id, subject, last_message_date, workflow_state, created_at, updated_at)
             VALUES ('thread', 'account', 'Subject', ?, 'inbox', ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("thread");
        sqlx::query(
            "INSERT INTO messages
             (id, account_id, folder_id, thread_id, from_address, to_addresses, subject, date, remote_id, created_at, updated_at)
             VALUES ('message', 'account', 'aside', 'thread', 'sender@example.com', '[]', 'Subject', ?, 'remote', ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("message");
        let mut message = envelope("remote");
        message.mailbox_ids = vec!["Label_123".to_string()];

        reconcile_gmail_message_mailboxes(&pool, "account", &message)
            .await
            .expect("reconcile aside label");

        let result: (String, i64) = sqlx::query_as(
            "SELECT workflow_state,
                    (SELECT COUNT(*) FROM folders WHERE folder_type = 'archive')
             FROM threads WHERE id = 'thread'",
        )
        .fetch_one(&pool)
        .await
        .expect("result");
        assert_eq!(result, ("aside".to_string(), 0));
    }

    #[tokio::test]
    async fn gmail_reconcile_removes_stale_folder_copies_and_attachments() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("pool");
        sqlx::query(
            "CREATE TABLE folders (
               id TEXT PRIMARY KEY,
               account_id TEXT,
               remote_id TEXT,
               folder_type TEXT,
               updated_at TEXT
             )",
        )
        .execute(&pool)
        .await
        .expect("folders");
        sqlx::query(
            "CREATE TABLE messages (
               id TEXT PRIMARY KEY,
               account_id TEXT,
               folder_id TEXT,
               thread_id TEXT,
               remote_id TEXT
             )",
        )
        .execute(&pool)
        .await
        .expect("messages");
        sqlx::query("CREATE TABLE attachments (id TEXT PRIMARY KEY, message_id TEXT)")
            .execute(&pool)
            .await
            .expect("attachments");
        for (id, remote_id) in [("inbox", "INBOX"), ("sent", "SENT")] {
            sqlx::query(
                "INSERT INTO folders (id, account_id, remote_id, folder_type, updated_at)
                 VALUES (?, 'account', ?, 'custom', '')",
            )
            .bind(id)
            .bind(remote_id)
            .execute(&pool)
            .await
            .expect("folder");
        }
        for (id, folder_id) in [("copy-inbox", "inbox"), ("copy-sent", "sent")] {
            sqlx::query(
                "INSERT INTO messages (id, account_id, folder_id, remote_id)
                 VALUES (?, 'account', ?, 'remote-message')",
            )
            .bind(id)
            .bind(folder_id)
            .execute(&pool)
            .await
            .expect("message");
        }
        sqlx::query("INSERT INTO attachments (id, message_id) VALUES ('attachment', 'copy-inbox')")
            .execute(&pool)
            .await
            .expect("attachment");

        let message = MessageEnvelope {
            remote_id: "remote-message".to_string(),
            remote_uidvalidity: None,
            remote_thread_id: None,
            original_source: None,
            mailbox_ids: vec!["SENT".to_string(), "CATEGORY_PERSONAL".to_string()],
            message_id_header: None,
            in_reply_to: None,
            references: Vec::new(),
            subject: String::new(),
            snippet: String::new(),
            from_address: String::new(),
            from_name: None,
            to_addresses: Vec::new(),
            cc_addresses: Vec::new(),
            bcc_addresses: Vec::new(),
            reply_to: None,
            remote_draft_id: None,
            received_at_rfc3339: None,
            date_rfc3339: String::new(),
            body_text: None,
            body_html: None,
            is_read: true,
            is_starred: false,
            body_calendar: None,
            smime_encrypted: None,
            smime_signed: None,
            pgp_signed: None,
            trust: Default::default(),
            list_unsubscribe: None,
            list_unsubscribe_post: None,
            auth_spf: None,
            auth_dkim: None,
            auth_dmarc: None,
            attachments: Vec::new(),
        };

        reconcile_gmail_message_mailboxes(&pool, "account", &message)
            .await
            .expect("reconcile");

        let remaining: Vec<String> = sqlx::query_scalar("SELECT id FROM messages ORDER BY id")
            .fetch_all(&pool)
            .await
            .expect("remaining messages");
        let attachments: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM attachments")
            .fetch_one(&pool)
            .await
            .expect("remaining attachments");
        assert_eq!(remaining, vec!["copy-sent"]);
        assert_eq!(attachments, 0);
    }
}
