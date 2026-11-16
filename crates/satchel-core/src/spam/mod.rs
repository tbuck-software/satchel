//! A local spam filter that learns from the user: naive Bayes over header and
//! body tokens, kept per profile in SQLite, with no network access.
//!
//! - [`tokenizer`] turns a message into hashed tokens.
//! - [`classifier`] scores token counts (Robinson–Fisher, as in SpamBayes).
//! - [`store`] keeps counts, the training ledger and verdicts.
//! - [`bootstrap`] trains once from the Spam folder and from mail the user
//!   acted on.
//!
//! Training signals: Spam (and its Undo), Not spam (and its Undo), a move out
//! of Spam, and — weakly — starring or answering. One example per sender and
//! thread. New, unread IMAP and POP3 mail is classified at ingest; Gmail and
//! Microsoft accounts have their provider's filter and only train this one.
//!
//! The gates are in [`classifier`]: no verdict before 50 examples of each
//! class in at most 4:1 proportion and a finished bootstrap; Mark at 0.90;
//! Move at 0.99 on clues seen three times, with 200 of each class, never for
//! a sender the user corresponds with or rescued, never on a thread with mail
//! the user called wanted.

pub mod bootstrap;
pub mod classifier;
pub mod store;
pub mod tokenizer;

use serde::{Deserialize, Serialize};
use sqlx::{SqliteConnection, SqlitePool};

use crate::sync::queue::SyncQueue;
use classifier::Verdict;
pub use store::{Label, Signal};

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
    /// Whether the filter gives verdicts at all.
    pub ready: bool,
    /// The one-time pass over existing mail has not finished yet.
    pub bootstrap_pending: bool,
    /// More examples of each class it needs before it gives verdicts: the
    /// floor of [`classifier::MIN_TRAINED_PER_CLASS`] and the class ratio.
    pub spam_needed: i64,
    pub ham_needed: i64,
    /// Whether Move may move mail; until then Move behaves like Mark.
    pub move_ready: bool,
    pub move_spam_needed: i64,
    pub move_ham_needed: i64,
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
    let bootstrap_pending = store::bootstrap_pending(&mut conn).await?;
    let (spam_needed, ham_needed) = totals.needed(classifier::MIN_TRAINED_PER_CLASS);
    let (move_spam_needed, move_ham_needed) = totals.needed(classifier::MIN_MOVE_TRAINED_PER_CLASS);
    Ok(SpamFilterStatus {
        mode,
        spam_messages: totals.spam,
        ham_messages: totals.ham,
        tokens,
        ready: totals.ready() && !bootstrap_pending,
        bootstrap_pending,
        spam_needed,
        ham_needed,
        move_ready: totals.move_ready() && !bootstrap_pending,
        move_spam_needed,
        move_ham_needed,
    })
}

/// The examples these threads hold: one message per thread and sender, from
/// someone else, not a draft. A fifty-message thread from one sender is one
/// example, not fifty. A sender already trained in a thread keeps the
/// message it was trained with, so a relabel reaches the same row.
pub async fn incoming_messages(
    conn: &mut SqliteConnection,
    thread_ids: &[String],
) -> Result<Vec<String>, sqlx::Error> {
    let mut out = Vec::new();
    for thread_id in thread_ids {
        let rows: Vec<(String, String, Option<String>)> = sqlx::query_as(
            "SELECT m.id, LOWER(TRIM(COALESCE(m.from_address, ''))),
                    (SELECT t.message_id FROM spam_training t
                     JOIN messages trained ON trained.id = t.message_id
                     WHERE t.thread_id = m.thread_id
                       AND t.sender = LOWER(TRIM(COALESCE(m.from_address, '')))
                       AND trained.thread_id = m.thread_id
                     LIMIT 1)
             FROM messages m
             JOIN accounts a ON a.id = m.account_id
             LEFT JOIN folders f ON f.id = COALESCE(m.pending_folder_id, m.folder_id)
             WHERE m.thread_id = ?
               AND COALESCE(m.is_sent, 0) = 0 AND COALESCE(m.is_draft, 0) = 0
               AND LOWER(COALESCE(f.folder_type, '')) NOT IN ('sent', 'drafts')
               AND LOWER(TRIM(COALESCE(m.from_address, ''))) <> LOWER(TRIM(COALESCE(a.email, '')))
             ORDER BY m.date DESC, m.id DESC",
        )
        .bind(thread_id)
        .fetch_all(&mut *conn)
        .await?;
        let mut senders = std::collections::HashSet::new();
        for (id, sender, trained) in rows {
            if senders.insert(sender) {
                out.push(trained.unwrap_or(id));
            }
        }
    }
    Ok(out)
}

/// Messages of these threads that are in a Spam folder, or on their way
/// there.
pub async fn spam_folder_messages(
    pool: &SqlitePool,
    thread_ids: &[String],
) -> Result<Vec<String>, sqlx::Error> {
    let mut out = Vec::new();
    for thread_id in thread_ids {
        let ids: Vec<String> = sqlx::query_scalar(
            "SELECT m.id FROM messages m
             JOIN folders f ON f.id = COALESCE(m.pending_folder_id, m.folder_id)
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

/// Train the examples of these threads ([`incoming_messages`]).
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

/// Whether this address belongs to someone the user corresponds with: the
/// user's own addresses, anyone the user has written to, a sender sorted in
/// the screener, and contacts with details the user gave them.
///
/// The address alone proves nothing — From is whatever the sender typed. See
/// [`identity_unverified`] for when the protection does not hold.
pub async fn is_protected_sender(
    conn: &mut SqliteConnection,
    sender: &str,
) -> Result<bool, sqlx::Error> {
    let sender = store::normalize_sender(sender);
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

/// The receiving server says the From address is not the sender's: DMARC
/// failed, or SPF failed without a passing DKIM signature. Without
/// Authentication-Results (POP3, many IMAP servers) nothing is known and the
/// address is taken at its word.
pub fn identity_unverified(features: &tokenizer::SpamFeatures) -> bool {
    let verdict = |value: &Option<String>| value.as_deref().map(|v| v.trim().to_ascii_lowercase());
    let dmarc = verdict(&features.auth_dmarc);
    let spf = verdict(&features.auth_spf);
    let dkim = verdict(&features.auth_dkim);
    dmarc.as_deref() == Some("fail")
        || (spf.as_deref() == Some("fail") && dkim.as_deref() != Some("pass"))
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
    /// Kept its headers but gave no verdict: model not ready, or a verified
    /// protected sender.
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
/// filter set to Move, file it in Spam when every Move gate holds.
///
/// The caller passes only new mail (after the notification cutoff, unread):
/// a first sync, a re-sync or a UIDVALIDITY reset must not judge a backlog.
///
/// Move needs, beyond a spam verdict: [`classifier::Score::move_confident`],
/// [`classifier::ModelTotals::move_ready`], a sender who is neither protected
/// nor rescued with Not spam, and a thread of nothing but spam that nobody
/// called wanted. Otherwise Move marks, like Mark.
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
    let Some(mut features) = store::features_for_message(&mut conn, incoming.message_id)
        .await
        .map_err(|e| e.to_string())?
    else {
        return Ok(ClassifyOutcome::Skipped);
    };
    features.hint_headers = incoming.hint_headers.map(str::to_owned);
    let protected = is_protected_sender(&mut conn, incoming.from_address)
        .await
        .map_err(|e| e.to_string())?;
    let unverified = identity_unverified(&features);
    // A verified correspondent is left alone. One whose address the server
    // could not vouch for — sextortion "from" the user's own address — is
    // judged, but only ever marked.
    let scored = if protected && !unverified {
        None
    } else {
        store::score_message(&mut conn, &features)
            .await
            .map_err(|e| e.to_string())?
    };
    store::store_verdict(
        &mut conn,
        incoming.message_id,
        scored.map(|score| (score.score, score.verdict)),
        incoming.hint_headers,
    )
    .await
    .map_err(|e| e.to_string())?;
    let Some(score) = scored else {
        return Ok(ClassifyOutcome::Unscored);
    };
    if score.verdict != Verdict::Spam || mode != SpamFilterMode::Move {
        return Ok(ClassifyOutcome::Scored(score.verdict));
    }
    let model_may_move = score.move_confident
        && store::totals(&mut conn)
            .await
            .map_err(|e| e.to_string())?
            .move_ready();
    let rescued = store::rescued_sender(&mut conn, incoming.from_address)
        .await
        .map_err(|e| e.to_string())?;
    if !model_may_move || protected || rescued {
        return Ok(ClassifyOutcome::Scored(score.verdict));
    }
    // Only a thread made of nothing but spam moves: a spam-looking reply in a
    // conversation the user takes part in must not take the conversation
    // along, and mail the user once called wanted stays where it is.
    let movable: bool = sqlx::query_scalar(
        "SELECT NOT EXISTS (
            SELECT 1 FROM messages m
            LEFT JOIN spam_verdicts v ON v.message_id = m.id
            WHERE m.thread_id = ?
              AND (COALESCE(m.is_sent, 0) <> 0 OR COALESCE(m.is_draft, 0) <> 0
                   OR v.verdict IS NOT 'spam'
                   OR EXISTS (SELECT 1 FROM spam_training t
                              WHERE t.message_id = m.id AND t.label = 'ham'))
         )",
    )
    .bind(incoming.thread_id)
    .fetch_one(&mut *conn)
    .await
    .map_err(|e| e.to_string())?;
    drop(conn);
    if !movable {
        return Ok(ClassifyOutcome::Scored(score.verdict));
    }
    let receipt =
        crate::mail_actions::move_spam_with_receipt(pool, queue, &[incoming.thread_id.into()])
            .await?;
    sqlx::query(
        "UPDATE spam_verdicts SET receipt_id = ?, moved_at = ?
         WHERE message_id IN (SELECT id FROM messages WHERE thread_id = ?)",
    )
    .bind(receipt.undo_token.as_deref())
    .bind(chrono::Utc::now().to_rfc3339())
    .bind(incoming.thread_id)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(ClassifyOutcome::Moved {
        receipt_id: receipt.undo_token,
    })
}
