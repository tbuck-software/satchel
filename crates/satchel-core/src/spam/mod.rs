//! A local spam filter that learns from the user: naive Bayes over header and
//! body tokens, kept per profile in SQLite, with no network access.
//!
//! - [`tokenizer`] turns a message into hashed tokens.
//! - [`classifier`] scores token counts (Robinson–Fisher, as in SpamBayes).
//! - [`store`] keeps counts, the training ledger and verdicts.
//! - [`bootstrap`] trains once from the Spam folder and read Inbox mail.
//!
//! Training signals: Spam (and its Undo), Not spam, a move out of Spam, and
//! — weakly — starring or answering. New IMAP and POP3 mail is classified at
//! ingest; Gmail and Microsoft accounts have their provider's filter and only
//! train this one.

pub mod bootstrap;
pub mod classifier;
pub mod store;
pub mod tokenizer;

use serde::{Deserialize, Serialize};
use sqlx::{SqliteConnection, SqlitePool};

use crate::sync::queue::SyncQueue;
use classifier::Verdict;
pub use store::{Label, Signal};
use tokenizer::tokenize;

/// `app_settings` key of the filter mode.
pub const MODE_KEY: &str = "spam_filter.mode";

/// What the filter does with a message it judges to be spam.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "core/spam/"))]
#[serde(rename_all = "snake_case")]
pub enum SpamFilterMode {
    /// Neither classify nor mark. Training still happens: switching the filter
    /// on later should not start from nothing.
    Off,
    /// Show it as likely spam in the list and the reader.
    #[default]
    Mark,
    /// Mark it and move it to the Spam folder.
    Move,
}

impl SpamFilterMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Mark => "mark",
            Self::Move => "move",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "off" => Some(Self::Off),
            "mark" => Some(Self::Mark),
            "move" => Some(Self::Move),
            _ => None,
        }
    }
}

/// What the settings page shows about the filter.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "core/spam/"))]
pub struct SpamFilterStatus {
    pub mode: SpamFilterMode,
    pub spam_messages: i64,
    pub ham_messages: i64,
    pub tokens: i64,
    /// Whether the model has seen enough of both classes to classify.
    pub ready: bool,
    /// Messages of each class the model needs before it classifies.
    pub min_per_class: i64,
}

pub async fn mode(pool: &SqlitePool) -> Result<SpamFilterMode, sqlx::Error> {
    let stored: Option<String> = sqlx::query_scalar("SELECT value FROM app_settings WHERE key = ?")
        .bind(MODE_KEY)
        .fetch_optional(pool)
        .await?;
    Ok(stored
        .as_deref()
        .and_then(SpamFilterMode::parse)
        .unwrap_or_default())
}

pub async fn set_mode(pool: &SqlitePool, mode: SpamFilterMode) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO app_settings (key, value, updated_at) VALUES (?, ?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
    )
    .bind(MODE_KEY)
    .bind(mode.as_str())
    .bind(chrono::Utc::now().to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn status(pool: &SqlitePool) -> Result<SpamFilterStatus, sqlx::Error> {
    let mode = mode(pool).await?;
    let mut conn = pool.acquire().await?;
    let totals = store::totals(&mut conn).await?;
    let tokens = store::token_count(&mut conn).await?;
    Ok(SpamFilterStatus {
        mode,
        spam_messages: totals.spam,
        ham_messages: totals.ham,
        tokens,
        ready: totals.ready(),
        min_per_class: classifier::MIN_TRAINED_PER_CLASS,
    })
}

/// Messages of these threads that someone else sent: not drafts, not the
/// user's own copies. Only those say anything about spam.
pub async fn incoming_messages(
    conn: &mut SqliteConnection,
    thread_ids: &[String],
) -> Result<Vec<String>, sqlx::Error> {
    let mut out = Vec::new();
    for thread_id in thread_ids {
        let ids: Vec<String> = sqlx::query_scalar(
            "SELECT m.id FROM messages m
             JOIN accounts a ON a.id = m.account_id
             LEFT JOIN folders f ON f.id = COALESCE(m.pending_folder_id, m.folder_id)
             WHERE m.thread_id = ?
               AND COALESCE(m.is_sent, 0) = 0 AND COALESCE(m.is_draft, 0) = 0
               AND LOWER(COALESCE(f.folder_type, '')) NOT IN ('sent', 'drafts')
               AND LOWER(TRIM(COALESCE(m.from_address, ''))) <> LOWER(TRIM(COALESCE(a.email, '')))
             ORDER BY m.id",
        )
        .bind(thread_id)
        .fetch_all(&mut *conn)
        .await?;
        out.extend(ids);
    }
    Ok(out)
}

/// Messages of these threads that sit in a Spam folder now.
pub async fn spam_folder_messages(
    pool: &SqlitePool,
    thread_ids: &[String],
) -> Result<Vec<String>, sqlx::Error> {
    let mut out = Vec::new();
    for thread_id in thread_ids {
        let ids: Vec<String> = sqlx::query_scalar(
            "SELECT m.id FROM messages m JOIN folders f ON f.id = m.folder_id
             WHERE m.thread_id = ? AND LOWER(COALESCE(f.folder_type, '')) = 'spam'
             ORDER BY m.id",
        )
        .bind(thread_id)
        .fetch_all(pool)
        .await?;
        out.extend(ids);
    }
    Ok(out)
}

/// Train messages in one write transaction, then keep the model bounded.
pub async fn train_messages(
    pool: &SqlitePool,
    message_ids: &[String],
    label: Label,
    signal: Signal,
    receipt_id: Option<&str>,
) -> Result<usize, sqlx::Error> {
    if message_ids.is_empty() {
        return Ok(0);
    }
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let mut changed = 0;
    for message_id in message_ids {
        if store::train_message(&mut tx, message_id, label, signal, receipt_id).await? {
            changed += 1;
        }
    }
    if changed > 0 {
        store::prune(&mut tx, store::MAX_MODEL_TOKENS, store::PRUNE_TARGET).await?;
    }
    tx.commit().await?;
    Ok(changed)
}

/// Train every incoming message of these threads.
pub async fn train_threads(
    pool: &SqlitePool,
    thread_ids: &[String],
    label: Label,
    signal: Signal,
    receipt_id: Option<&str>,
) -> Result<usize, sqlx::Error> {
    let message_ids = {
        let mut conn = pool.acquire().await?;
        incoming_messages(&mut conn, thread_ids).await?
    };
    train_messages(pool, &message_ids, label, signal, receipt_id).await
}

/// Reverse the training an undone action caused.
pub async fn untrain_receipt(pool: &SqlitePool, receipt_id: &str) -> Result<usize, sqlx::Error> {
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let changed = store::untrain_receipt(&mut tx, receipt_id).await?;
    tx.commit().await?;
    Ok(changed)
}

/// Whether the filter must leave mail from this address alone: the user's own
/// addresses, anyone the user has written to, a sender the user has sorted in
/// the screener, and contacts with details the user gave them.
pub async fn is_protected_sender(
    conn: &mut SqliteConnection,
    sender: &str,
) -> Result<bool, sqlx::Error> {
    let sender = sender.trim().trim_matches(['<', '>']).to_lowercase();
    if sender.is_empty() || !sender.contains('@') {
        return Ok(false);
    }
    sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS (SELECT 1 FROM accounts WHERE LOWER(TRIM(email)) = ?1)
             OR EXISTS (SELECT 1 FROM account_aliases WHERE LOWER(TRIM(email)) = ?1)
             OR EXISTS (
                 SELECT 1 FROM message_addresses ma
                 JOIN messages sent ON sent.id = ma.message_id
                 LEFT JOIN folders sent_folder ON sent_folder.id = sent.folder_id
                 WHERE ma.email = ?1
                   AND (COALESCE(sent.is_sent, 0) <> 0 OR LOWER(COALESCE(sent_folder.folder_type, '')) = 'sent'))
             OR EXISTS (SELECT 1 FROM sender_preferences WHERE LOWER(TRIM(sender_email)) = ?1)
             OR EXISTS (
                 SELECT 1 FROM contacts WHERE email = ?1
                   AND (avatar_source IN ('manual', 'local_contact', 'provider_contact')
                        OR TRIM(COALESCE(notes, '')) <> ''))",
    )
    .bind(&sender)
    .fetch_one(&mut *conn)
    .await
    .map(|protected| protected != 0)
}

/// A message the sync just stored for the first time.
#[derive(Debug, Clone)]
pub struct IncomingMessage<'a> {
    pub provider: &'a str,
    pub message_id: &'a str,
    pub thread_id: &'a str,
    pub from_address: &'a str,
    pub hint_headers: Option<&'a str>,
}

/// What classifying a new message did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClassifyOutcome {
    /// Not a message the filter looks at (mode off, provider with its own
    /// filter, already classified).
    Skipped,
    /// Kept its headers but gave no verdict: model not ready, or a protected
    /// sender.
    Unscored,
    Scored(Verdict),
    /// Judged spam and moved to the Spam folder under an undo receipt.
    Moved {
        receipt_id: Option<String>,
    },
}

/// Providers whose servers already filter spam. The filter trains on their
/// mail but does not act on it.
pub fn provider_filters_spam(provider: &str) -> bool {
    matches!(
        provider.to_ascii_lowercase().as_str(),
        "gmail" | "outlook" | "graph" | "microsoft"
    )
}

/// Classify a message the sync stored for the first time, and with the
/// filter set to Move, file it in Spam.
///
/// The move goes through [`crate::mail_actions::move_spam_with_receipt`]: the
/// provider move is queued before the local one, and the receipt lets Not
/// spam restore the message through the ordinary Undo path.
pub async fn classify_incoming(
    pool: &SqlitePool,
    queue: &SyncQueue,
    incoming: IncomingMessage<'_>,
) -> Result<ClassifyOutcome, String> {
    let mode = mode(pool).await.map_err(|e| e.to_string())?;
    if mode == SpamFilterMode::Off || provider_filters_spam(incoming.provider) {
        return Ok(ClassifyOutcome::Skipped);
    }
    let mut conn = pool.acquire().await.map_err(|e| e.to_string())?;
    let classified: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM spam_verdicts WHERE message_id = ?)")
            .bind(incoming.message_id)
            .fetch_one(&mut *conn)
            .await
            .map_err(|e| e.to_string())?;
    if classified {
        return Ok(ClassifyOutcome::Skipped);
    }
    let protected = is_protected_sender(&mut conn, incoming.from_address)
        .await
        .map_err(|e| e.to_string())?;
    let Some(mut features) = store::features_for_message(&mut conn, incoming.message_id)
        .await
        .map_err(|e| e.to_string())?
    else {
        return Ok(ClassifyOutcome::Skipped);
    };
    features.hint_headers = incoming.hint_headers.map(str::to_owned);
    let scored = if protected {
        None
    } else {
        store::score_tokens(&mut conn, &tokenize(&features))
            .await
            .map_err(|e| e.to_string())?
    };
    store::store_verdict(
        &mut conn,
        incoming.message_id,
        scored,
        incoming.hint_headers,
    )
    .await
    .map_err(|e| e.to_string())?;
    let Some((_, verdict)) = scored else {
        return Ok(ClassifyOutcome::Unscored);
    };
    if verdict != Verdict::Spam || mode != SpamFilterMode::Move {
        return Ok(ClassifyOutcome::Scored(verdict));
    }
    // Only a thread made of nothing but spam moves: a spam-looking reply in a
    // conversation the user takes part in must not take the conversation along.
    let movable: bool = sqlx::query_scalar(
        "SELECT NOT EXISTS (
            SELECT 1 FROM messages m
            LEFT JOIN spam_verdicts v ON v.message_id = m.id
            WHERE m.thread_id = ?
              AND (COALESCE(m.is_sent, 0) <> 0 OR COALESCE(m.is_draft, 0) <> 0
                   OR v.verdict IS NOT 'spam')
         )",
    )
    .bind(incoming.thread_id)
    .fetch_one(&mut *conn)
    .await
    .map_err(|e| e.to_string())?;
    drop(conn);
    if !movable {
        return Ok(ClassifyOutcome::Scored(verdict));
    }
    let receipt =
        crate::mail_actions::move_spam_with_receipt(pool, queue, &[incoming.thread_id.into()])
            .await?;
    if let Some(receipt_id) = receipt.undo_token.as_deref() {
        sqlx::query(
            "UPDATE spam_verdicts SET receipt_id = ?
             WHERE message_id IN (SELECT id FROM messages WHERE thread_id = ?)",
        )
        .bind(receipt_id)
        .bind(incoming.thread_id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    }
    Ok(ClassifyOutcome::Moved {
        receipt_id: receipt.undo_token,
    })
}
