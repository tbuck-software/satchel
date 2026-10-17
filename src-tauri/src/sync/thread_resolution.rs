//! Thread identity, aggregate projection, and cached-draft reconciliation.
//!
//! How an incoming provider message is keyed into a thread (conversation id,
//! `Message-ID`, folder-scoped standalone keys), how replies are resolved to
//! existing threads, how the thread aggregate is refreshed from its messages,
//! and how superseded cached drafts (Spark autosaves, sent-mail copies) are
//! deduplicated after a sync.

use std::collections::{HashMap, HashSet};

use sqlx::{FromRow, SqlitePool};

use crate::db::threads::ThreadRepository;
use crate::providers::traits::MessageEnvelope;

pub(crate) async fn refresh_provider_thread_projection(
    pool: &SqlitePool,
    thread_id: &str,
) -> Result<(), String> {
    refresh_provider_thread_projections(pool, std::slice::from_ref(&thread_id.to_string())).await
}

/// Refresh a whole set of threads in one transaction.
///
/// `refresh_projection` has always taken a slice; every caller passed one id at
/// a time from inside a loop, so a reset folder or a batch of superseded drafts
/// paid a `BEGIN IMMEDIATE`, four aggregates and an UPDATE per thread instead
/// of once for all of them.
pub(crate) async fn refresh_provider_thread_projections(
    pool: &SqlitePool,
    thread_ids: &[String],
) -> Result<(), String> {
    if thread_ids.is_empty() {
        return Ok(());
    }
    ThreadRepository::new(pool)
        .refresh_projection(thread_ids)
        .await
        .map_err(|error| error.to_string())
}

pub(crate) fn sanitize_id(value: &str) -> String {
    value
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect::<String>()
}

pub fn derive_thread_key<'a>(message: &'a MessageEnvelope, standalone: &'a str) -> &'a str {
    message
        .remote_thread_id
        .as_deref()
        .or(message.message_id_header.as_deref())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(standalone)
}

/// The standalone grouping key, used when a message carries no conversation id
/// and no `Message-ID`. Scoped by folder for IMAP; see `derive_thread_key`.
pub fn standalone_thread_key(
    provider_kind: &str,
    folder_remote_id: &str,
    message: &MessageEnvelope,
) -> String {
    if provider_kind.eq_ignore_ascii_case("imap") {
        // The provider refuses a mailbox that reports no UIDVALIDITY, so the
        // generation is always known here. Keeping the slot unconditionally
        // means a recycled UID can never alias onto an older message's key.
        let uidvalidity = message.remote_uidvalidity.unwrap_or_default();
        format!("{folder_remote_id}:{uidvalidity}:{}", message.remote_id)
    } else {
        message.remote_id.clone()
    }
}

/// A message with no usable `Message-ID` still needs one: it is the join key
/// for replies and references. The synthetic form is marked `.satchel.local`
/// so it is recognisable as locally minted and cannot collide with a real one.
pub fn derive_message_id_header(
    provider_kind: &str,
    folder_remote_id: &str,
    message: &MessageEnvelope,
) -> String {
    message
        .message_id_header
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| {
            let native_id = standalone_thread_key(provider_kind, folder_remote_id, message);
            format!(
                "<{}@{}.satchel.local>",
                sanitize_id(&native_id),
                sanitize_id(provider_kind)
            )
        })
}

/// Which remote draft this message edits, if any.
///
/// Gmail supplies a separate draft id. Outlook uses its message id. IMAP must
/// supply a Message-ID based draft ref: its numeric UID is unsafe after a
/// UIDVALIDITY reset. Non-drafts never carry a draft id.
pub fn derive_remote_draft_id(
    message_is_draft: bool,
    provider_kind: &str,
    message: &MessageEnvelope,
) -> Option<String> {
    if !message_is_draft {
        return None;
    }
    message.remote_draft_id.clone().or_else(|| {
        provider_kind
            .eq_ignore_ascii_case("outlook")
            .then(|| message.remote_id.clone())
    })
}

/// Was this message sent by the account owner?
///
/// The folder alone is not enough: Gmail labels a sent message SENT wherever it
/// happens to be listed, so the labels have to be consulted too.
pub fn message_is_sent(folder_type: &str, mailbox_ids: &[String]) -> bool {
    folder_type == "sent"
        || mailbox_ids
            .iter()
            .any(|label| label.eq_ignore_ascii_case("SENT"))
}

/// Sender name for display, falling back to the account's own display name for
/// messages the owner sent — providers routinely omit the name on those.
pub fn effective_from_name<'a>(
    message: &'a MessageEnvelope,
    account_display_name: &'a str,
    sender_is_account: bool,
) -> Option<&'a str> {
    message
        .from_name
        .as_deref()
        .filter(|name| !name.trim().is_empty())
        .or_else(|| sender_is_account.then_some(account_display_name))
}

/// Header block handed to the category classifier. The list headers are the
/// strongest newsletter signal available, and the folder name disambiguates
/// what the address alone cannot.
pub fn synthetic_classification_headers(
    message: &MessageEnvelope,
    folder_remote_id: &str,
) -> String {
    satchel_core::db::sender_preferences::classification_headers(
        message.list_unsubscribe.as_deref(),
        message.list_unsubscribe_post.as_deref(),
        Some(folder_remote_id),
    )
}

#[allow(clippy::too_many_arguments)]
/// The thread an already-synced ancestor of this message lives in, if any.
///
/// Checks the direct parent (`In-Reply-To`) first, then the `References`
/// chain from closest to root — the first hit wins so a reply lands next to
/// its nearest known relative.
pub(crate) async fn resolve_reply_thread_connection(
    connection: &mut sqlx::SqliteConnection,
    account_id: &str,
    message: &crate::providers::traits::MessageEnvelope,
) -> Result<Option<String>, String> {
    let mut candidates: Vec<&str> = Vec::new();
    if let Some(parent) = message.in_reply_to.as_deref() {
        if !parent.trim().is_empty() {
            candidates.push(parent);
        }
    }
    for reference in message.references.iter().rev() {
        if !reference.trim().is_empty() {
            candidates.push(reference);
        }
    }
    for candidate in candidates {
        let found = sqlx::query_scalar::<_, String>(
            "SELECT thread_id FROM messages
             WHERE account_id = ? AND message_id_header = ?
               AND NULLIF(TRIM(thread_id), '') IS NOT NULL
             LIMIT 1",
        )
        .bind(account_id)
        .bind(candidate.trim())
        .fetch_optional(&mut *connection)
        .await
        .map_err(|err| err.to_string())?;
        if let Some(thread_id) = found {
            return Ok(Some(thread_id));
        }
    }
    Ok(None)
}

pub use crate::email::subject::subject_key as normalized_reply_subject;

pub fn participant_addresses<'a>(
    from: &'a str,
    to: &'a [String],
    cc: &'a [String],
    bcc: &'a [String],
    account_email: &str,
) -> HashSet<String> {
    std::iter::once(from)
        .chain(to.iter().map(String::as_str))
        .chain(cc.iter().map(String::as_str))
        .chain(bcc.iter().map(String::as_str))
        .map(str::trim)
        .filter(|value| value.contains('@'))
        .filter(|value| !value.eq_ignore_ascii_case(account_email.trim()))
        .map(str::to_ascii_lowercase)
        .collect()
}

/// How far apart two messages may be dated and still be joined by subject
/// alone. A recurring invoice keeps its subject for years; a conversation does
/// not pause for a month and resume without reply headers.
const SUBJECT_FALLBACK_WINDOW_DAYS: i64 = 30;

/// Shortest normalized subject the fallback will act on. "Info", "Test",
/// "Termin" and "Rechnung" are what unrelated senders happen to write.
const SUBJECT_FALLBACK_MIN_LEN: usize = 12;

/// Last-resort IMAP grouping for servers or legacy rows without usable reply
/// headers. It is deliberately conservative, and every one of the four
/// conditions is load-bearing: the subject must be normalized-equal and long
/// enough to be specific, the candidate must be dated inside the window, and
/// From/To must overlap. Cc and Bcc are excluded on purpose — a shared
/// distribution list is not a conversation. Joining two threads cannot be
/// undone: `merge_reply_split_threads` has no counterpart, and the target
/// thread's workflow state, tags and topics win.
pub(crate) async fn resolve_imap_subject_thread_connection(
    connection: &mut sqlx::SqliteConnection,
    account_id: &str,
    account_email: &str,
    message: &MessageEnvelope,
) -> Result<Option<String>, String> {
    let has_real_message_id = message
        .message_id_header
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty());
    let has_reply_ancestry = message
        .in_reply_to
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
        || message
            .references
            .iter()
            .any(|value| !value.trim().is_empty());
    if has_real_message_id && !has_reply_ancestry {
        return Ok(None);
    }
    let subject = normalized_reply_subject(&message.subject);
    if subject.len() < SUBJECT_FALLBACK_MIN_LEN || subject == "(no subject)" {
        return Ok(None);
    }
    let participants =
        sender_and_recipient_addresses(&message.from_address, &message.to_addresses, account_email);
    if participants.is_empty() {
        return Ok(None);
    }
    // A message the window cannot be built around has no comparable position
    // in time, and joining on subject alone is what this guard exists to stop.
    let Some(window) = subject_fallback_window(&message.date_rfc3339) else {
        return Ok(None);
    };

    let candidates = sqlx::query_as::<_, (String, String, String, String)>(
        "SELECT thread_id, COALESCE(subject, ''), from_address,
                COALESCE(to_addresses, '[]')
         FROM messages
         WHERE account_id = ? AND LOWER(TRIM(COALESCE(subject, ''))) LIKE '%' || ?
           AND NULLIF(TRIM(thread_id), '') IS NOT NULL
           AND date >= ? AND date <= ?
         ORDER BY date DESC, id DESC
         LIMIT 200",
    )
    .bind(account_id)
    .bind(&subject)
    .bind(&window.0)
    .bind(&window.1)
    .fetch_all(&mut *connection)
    .await
    .map_err(|err| err.to_string())?;

    for (thread_id, candidate_subject, from, to) in candidates {
        if normalized_reply_subject(&candidate_subject) != subject {
            continue;
        }
        let to = serde_json::from_str::<Vec<String>>(&to).unwrap_or_default();
        let candidate_participants = sender_and_recipient_addresses(&from, &to, account_email);
        if !participants.is_disjoint(&candidate_participants) {
            return Ok(Some(thread_id));
        }
    }
    Ok(None)
}

/// The inclusive date range a subject-fallback candidate has to fall into.
fn subject_fallback_window(date_rfc3339: &str) -> Option<(String, String)> {
    let date = chrono::DateTime::parse_from_rfc3339(date_rfc3339.trim())
        .ok()?
        .with_timezone(&chrono::Utc);
    let span = chrono::TimeDelta::days(SUBJECT_FALLBACK_WINDOW_DAYS);
    Some(((date - span).to_rfc3339(), (date + span).to_rfc3339()))
}

/// The addresses that make two messages the same conversation: who wrote and
/// who was written to, minus the account owner.
fn sender_and_recipient_addresses(
    from: &str,
    to: &[String],
    account_email: &str,
) -> HashSet<String> {
    participant_addresses(from, to, &[], &[], account_email)
}

/// Spark saves a reply draft as a succession of IMAP messages with fresh
/// Message-IDs. Keep the newest local representation of that one logical draft
/// while leaving unrelated or independently composed drafts untouched.
pub async fn deduplicate_spark_draft_versions(
    pool: &SqlitePool,
    account_id: &str,
    folder_id: &str,
    message: &MessageEnvelope,
    message_id: &str,
    is_draft: bool,
) -> Result<HashSet<String>, String> {
    let Some(message_id_header) = message.message_id_header.as_deref() else {
        return Ok(HashSet::new());
    };
    let Some(in_reply_to) = message
        .in_reply_to
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(HashSet::new());
    };
    if !is_draft || !message_id_header.to_ascii_lowercase().ends_with("@spark>") {
        return Ok(HashSet::new());
    }

    let to_addresses =
        serde_json::to_string(&message.to_addresses).unwrap_or_else(|_| "[]".to_string());
    let rows = sqlx::query_as::<_, (String, String)>(
        "SELECT id, thread_id FROM messages
         WHERE account_id = ? AND folder_id = ? AND is_draft = 1
           AND LOWER(TRIM(COALESCE(message_id_header, ''))) LIKE '%@spark>'
           AND LOWER(TRIM(COALESCE(in_reply_to, ''))) = LOWER(?)
           AND LOWER(TRIM(COALESCE(subject, ''))) = LOWER(TRIM(?))
           AND LOWER(TRIM(from_address)) = LOWER(TRIM(?))
           AND to_addresses = ?
         ORDER BY date DESC, id DESC",
    )
    .bind(account_id)
    .bind(folder_id)
    .bind(in_reply_to)
    .bind(&message.subject)
    .bind(&message.from_address)
    .bind(to_addresses)
    .fetch_all(pool)
    .await
    .map_err(|err| err.to_string())?;
    if rows.len() <= 1 {
        return Ok(HashSet::new());
    }

    let mut touched = HashSet::new();
    let keep_id = &rows[0].0;
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|err| err.to_string())?;
    for (duplicate_id, thread_id) in rows.iter().skip(1) {
        if duplicate_id == keep_id {
            continue;
        }
        sqlx::query("DELETE FROM attachments WHERE message_id = ?")
            .bind(duplicate_id)
            .execute(&mut *tx)
            .await
            .map_err(|err| err.to_string())?;
        sqlx::query("DELETE FROM messages WHERE id = ? AND is_draft = 1")
            .bind(duplicate_id)
            .execute(&mut *tx)
            .await
            .map_err(|err| err.to_string())?;
        touched.insert(thread_id.clone());
    }
    tx.commit().await.map_err(|err| err.to_string())?;

    // The caller may have ingested an older autosave after the newest one.
    // Including its thread ensures its now-empty projection is removed.
    if message_id != keep_id {
        touched.extend(
            rows.iter()
                .filter(|row| row.0 == message_id)
                .map(|row| row.1.clone()),
        );
    }
    Ok(touched)
}

const DRAFT_AUTOSAVE_CLUSTER_GAP_SECONDS: i64 = 2 * 60 * 60;

#[derive(Debug, Clone, FromRow)]
pub struct CachedDraftVersionRow {
    id: String,
    thread_id: String,
    folder_id: String,
    message_id_header: Option<String>,
    in_reply_to: Option<String>,
    from_address: Option<String>,
    to_addresses: Option<String>,
    cc_addresses: Option<String>,
    bcc_addresses: Option<String>,
    subject: Option<String>,
    date: Option<String>,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct DraftAutosaveKey {
    folder_id: String,
    in_reply_to: String,
    from_address: String,
    to_addresses: String,
    cc_addresses: String,
    bcc_addresses: String,
    subject: String,
}

impl DraftAutosaveKey {
    fn same_compose_session(&self, other: &Self) -> bool {
        self.in_reply_to == other.in_reply_to
            && self.from_address == other.from_address
            && self.to_addresses == other.to_addresses
            && self.cc_addresses == other.cc_addresses
            && self.bcc_addresses == other.bcc_addresses
            && self.subject == other.subject
    }
}

pub fn is_draft_autosave_header(value: Option<&str>) -> bool {
    let normalized = value.unwrap_or_default().trim().to_ascii_lowercase();
    normalized.ends_with("@spark>")
        || normalized.ends_with("@imap.satchel.local>")
        || normalized.ends_with("@imap.flowmail.local>")
}

pub fn normalized_draft_addresses(value: Option<&str>) -> String {
    let mut addresses = value
        .and_then(|raw| serde_json::from_str::<Vec<String>>(raw).ok())
        .unwrap_or_default()
        .into_iter()
        .map(|address| address.trim().to_ascii_lowercase())
        .filter(|address| !address.is_empty())
        .collect::<Vec<_>>();
    addresses.sort_unstable();
    addresses.dedup();
    addresses.join("\n")
}

pub fn draft_autosave_key(row: &CachedDraftVersionRow) -> Option<DraftAutosaveKey> {
    if !is_draft_autosave_header(row.message_id_header.as_deref()) {
        return None;
    }

    let subject = row
        .subject
        .as_deref()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let from_address = row
        .from_address
        .as_deref()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let to_addresses = normalized_draft_addresses(row.to_addresses.as_deref());
    let in_reply_to = row
        .in_reply_to
        .as_deref()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();

    // Empty new-message shells are common and may be independent. They are
    // not specific enough to collapse without a reply parent or recipient.
    if from_address.is_empty()
        || subject.is_empty()
        || ((subject == "(no subject)" || to_addresses.is_empty()) && in_reply_to.is_empty())
    {
        return None;
    }

    Some(DraftAutosaveKey {
        folder_id: row.folder_id.clone(),
        in_reply_to,
        from_address,
        to_addresses,
        cc_addresses: normalized_draft_addresses(row.cc_addresses.as_deref()),
        bcc_addresses: normalized_draft_addresses(row.bcc_addresses.as_deref()),
        subject,
    })
}

pub fn draft_autosave_timestamp(row: &CachedDraftVersionRow) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(row.date.as_deref()?)
        .ok()
        .map(|date| date.timestamp())
}

pub fn obsolete_cached_draft_versions(
    rows: &[CachedDraftVersionRow],
) -> Vec<CachedDraftVersionRow> {
    let mut groups = HashMap::<DraftAutosaveKey, Vec<(i64, CachedDraftVersionRow)>>::new();
    for row in rows {
        let (Some(key), Some(timestamp)) = (draft_autosave_key(row), draft_autosave_timestamp(row))
        else {
            continue;
        };
        groups
            .entry(key)
            .or_default()
            .push((timestamp, row.clone()));
    }

    let mut obsolete = Vec::new();
    for versions in groups.values_mut() {
        versions.sort_by(|left, right| {
            left.0
                .cmp(&right.0)
                .then_with(|| left.1.id.cmp(&right.1.id))
        });

        let mut cluster_start = 0usize;
        for index in 1..=versions.len() {
            let cluster_ended = index == versions.len()
                || versions[index].0 - versions[index - 1].0 > DRAFT_AUTOSAVE_CLUSTER_GAP_SECONDS;
            if !cluster_ended {
                continue;
            }
            if index - cluster_start > 1 {
                obsolete.extend(
                    versions[cluster_start..index - 1]
                        .iter()
                        .map(|(_, row)| row.clone()),
                );
            }
            cluster_start = index;
        }
    }
    obsolete
}

/// A Spark send is a new IMAP message, not an update of its last autosave.
/// If the Drafts-folder snapshot lags behind, both rows otherwise remain
/// visible indefinitely. A sent copy with the same reply parent, recipients
/// and subject supersedes an older autosave from the same short compose
/// session.
pub async fn reconcile_sent_draft_autosaves(
    pool: &SqlitePool,
    account_id: &str,
) -> Result<HashSet<String>, String> {
    let drafts = sqlx::query_as::<_, CachedDraftVersionRow>(
        "SELECT id, thread_id, folder_id, message_id_header, in_reply_to,
                from_address, to_addresses, cc_addresses, bcc_addresses,
                subject, date
         FROM messages
         WHERE account_id = ? AND is_draft = 1
           AND LOWER(TRIM(COALESCE(message_id_header, ''))) LIKE '%@spark>'",
    )
    .bind(account_id)
    .fetch_all(pool)
    .await
    .map_err(|err| err.to_string())?;

    let mut sent_by_thread = HashMap::<String, Vec<CachedDraftVersionRow>>::new();
    let mut obsolete = Vec::new();
    for draft in drafts {
        let (Some(draft_key), Some(draft_at)) =
            (draft_autosave_key(&draft), draft_autosave_timestamp(&draft))
        else {
            continue;
        };
        if !sent_by_thread.contains_key(&draft.thread_id) {
            let sent = sqlx::query_as::<_, CachedDraftVersionRow>(
                "SELECT id, thread_id, folder_id, message_id_header, in_reply_to,
                        from_address, to_addresses, cc_addresses, bcc_addresses,
                        subject, date
                 FROM messages
                 WHERE account_id = ? AND thread_id = ? AND is_sent = 1
                   AND LOWER(TRIM(COALESCE(message_id_header, ''))) LIKE '%@spark>'",
            )
            .bind(account_id)
            .bind(&draft.thread_id)
            .fetch_all(pool)
            .await
            .map_err(|err| err.to_string())?;
            sent_by_thread.insert(draft.thread_id.clone(), sent);
        }
        let superseded = sent_by_thread
            .get(&draft.thread_id)
            .into_iter()
            .flatten()
            .any(|sent| {
                let (Some(sent_key), Some(sent_at)) =
                    (draft_autosave_key(sent), draft_autosave_timestamp(sent))
                else {
                    return false;
                };
                sent_key.same_compose_session(&draft_key)
                    && sent_at >= draft_at
                    && sent_at - draft_at <= DRAFT_AUTOSAVE_CLUSTER_GAP_SECONDS
            });
        if superseded {
            obsolete.push(draft);
        }
    }

    let mut touched_threads = HashSet::new();
    if obsolete.is_empty() {
        return Ok(touched_threads);
    }
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|err| err.to_string())?;
    for draft in obsolete {
        sqlx::query("DELETE FROM attachments WHERE message_id = ?")
            .bind(&draft.id)
            .execute(&mut *tx)
            .await
            .map_err(|err| err.to_string())?;
        let removed = sqlx::query("DELETE FROM messages WHERE id = ? AND is_draft = 1")
            .bind(&draft.id)
            .execute(&mut *tx)
            .await
            .map_err(|err| err.to_string())?
            .rows_affected();
        if removed > 0 {
            touched_threads.insert(draft.thread_id);
        }
    }
    tx.commit().await.map_err(|err| err.to_string())?;
    Ok(touched_threads)
}

/// Collapse cached IMAP draft revisions after a sync pass. Spark writes each
/// autosave as a fresh server message; older Satchel versions additionally
/// synthesized Message-IDs when IMAP omitted the header. Both are cache rows,
/// so retaining only the newest revision in a short, otherwise-identical
/// compose session is reproducible on every full resync without mutating the
/// provider's Drafts folder.
pub async fn deduplicate_cached_draft_autosaves(
    pool: &SqlitePool,
    account_id: &str,
) -> Result<u64, String> {
    let rows = sqlx::query_as::<_, CachedDraftVersionRow>(
        "SELECT id, thread_id, folder_id, message_id_header, in_reply_to,
                from_address, to_addresses, cc_addresses, bcc_addresses,
                subject, date
         FROM messages
         WHERE account_id = ? AND is_draft = 1
           AND (
             LOWER(TRIM(COALESCE(message_id_header, ''))) LIKE '%@spark>'
             OR LOWER(TRIM(COALESCE(message_id_header, ''))) LIKE '%@imap.satchel.local>'
             OR LOWER(TRIM(COALESCE(message_id_header, ''))) LIKE '%@imap.flowmail.local>'
           )",
    )
    .bind(account_id)
    .fetch_all(pool)
    .await
    .map_err(|err| err.to_string())?;
    let obsolete = obsolete_cached_draft_versions(&rows);
    if obsolete.is_empty() {
        return Ok(0);
    }

    let mut touched_threads = HashSet::new();
    let mut removed = 0u64;
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|err| err.to_string())?;
    for row in obsolete {
        sqlx::query("DELETE FROM attachments WHERE message_id = ?")
            .bind(&row.id)
            .execute(&mut *tx)
            .await
            .map_err(|err| err.to_string())?;
        removed += sqlx::query("DELETE FROM messages WHERE id = ? AND is_draft = 1")
            .bind(&row.id)
            .execute(&mut *tx)
            .await
            .map_err(|err| err.to_string())?
            .rows_affected();
        touched_threads.insert(row.thread_id);
    }
    tx.commit().await.map_err(|err| err.to_string())?;

    for thread_id in touched_threads {
        refresh_provider_thread_projection(pool, &thread_id).await?;
    }
    Ok(removed)
}

/// Merge threads that were split because a reply synced before its parent
/// (or predates reply-aware threading). The parent's thread is canonical;
/// the reply's whole thread moves over, topic links included. Chains
/// converge across iterations.
pub async fn merge_reply_split_threads(pool: &SqlitePool) -> Result<u64, String> {
    merge_reply_split_threads_with_budget(pool, None).await
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
struct ReplyMergeCursor {
    after: String,
    round: u8,
    moved_in_round: u64,
}

static REPLY_MERGE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

const REPLY_MERGE_CURSOR: &str = "maintenance.reply_merge_cursor";

async fn merge_reply_split_threads_with_budget(
    pool: &SqlitePool,
    max_blocks: Option<u64>,
) -> Result<u64, String> {
    let _guard = REPLY_MERGE_LOCK.lock().await;
    let saved: Option<String> = sqlx::query_scalar("SELECT value FROM app_settings WHERE key = ?")
        .bind(REPLY_MERGE_CURSOR)
        .fetch_optional(pool)
        .await
        .map_err(|err| err.to_string())?;
    let mut cursor: ReplyMergeCursor = saved
        .map(|v| serde_json::from_str(&v))
        .transpose()
        .map_err(|err| err.to_string())?
        .unwrap_or_default();
    let mut moved_total = 0;
    let mut blocks = 0;
    while max_blocks.is_none_or(|max| blocks < max) {
        // Page every message by its stable primary key, including non-replies.
        let ids: Vec<String> =
            sqlx::query_scalar("SELECT id FROM messages WHERE id > ? ORDER BY id LIMIT 256")
                .bind(&cursor.after)
                .fetch_all(pool)
                .await
                .map_err(|err| err.to_string())?;
        if ids.is_empty() {
            if cursor.moved_in_round == 0 || cursor.round >= 9 {
                break;
            }
            cursor.after.clear();
            cursor.round += 1;
            cursor.moved_in_round = 0;
        } else {
            for id in &ids {
                let pair = sqlx::query_as::<_, (String, String)>(
                    "SELECT child.thread_id, parent.thread_id
                     FROM messages child
                     JOIN accounts acc ON acc.id = child.account_id AND acc.provider = 'imap'
                     JOIN messages parent ON parent.id = (
                       SELECT id FROM messages
                       WHERE account_id = child.account_id AND message_id_header = child.in_reply_to
                         AND thread_id LIKE 'th-imap-%' ORDER BY id LIMIT 1
                     )
                     WHERE child.id = ? AND NULLIF(TRIM(child.in_reply_to), '') IS NOT NULL
                       AND child.thread_id LIKE 'th-imap-%' AND child.thread_id != parent.thread_id",
                ).bind(id).fetch_optional(pool).await.map_err(|err| err.to_string())?;
                if let Some((child, parent)) = pair {
                    let moved = ThreadRepository::new(pool)
                        .merge_into(&child, &parent)
                        .await
                        .map_err(|err| err.to_string())?;
                    moved_total += moved;
                    cursor.moved_in_round += moved;
                }
            }
            cursor.after = ids.last().expect("nonempty block").clone();
        }
        // Merges are idempotent. A crash before this commit repeats only this block.
        sqlx::query("INSERT INTO app_settings (key, value, updated_at) VALUES (?, ?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at")
            .bind(REPLY_MERGE_CURSOR).bind(serde_json::to_string(&cursor).map_err(|err| err.to_string())?)
            .bind(chrono::Utc::now().to_rfc3339()).execute(pool).await.map_err(|err| err.to_string())?;
        blocks += 1;
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    if max_blocks.is_none_or(|max| blocks < max) {
        sqlx::query("DELETE FROM app_settings WHERE key = ?")
            .bind(REPLY_MERGE_CURSOR)
            .execute(pool)
            .await
            .map_err(|err| err.to_string())?;
    }
    Ok(moved_total)
}

#[cfg(test)]
mod tests {
    use super::{
        deduplicate_cached_draft_autosaves, derive_message_id_header, derive_remote_draft_id,
        derive_thread_key, effective_from_name, message_is_sent, standalone_thread_key,
        synthetic_classification_headers,
    };
    use crate::providers::traits::MessageEnvelope;
    use sqlx::sqlite::SqlitePoolOptions;

    /// A minimal envelope; each test sets only the fields it cares about.
    fn envelope(remote_id: &str) -> MessageEnvelope {
        MessageEnvelope {
            remote_id: remote_id.to_string(),
            remote_uidvalidity: None,
            remote_thread_id: None,
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

    #[tokio::test]
    async fn reply_merge_resumes_a_saved_page_and_converges_across_passes() {
        use super::{merge_reply_split_threads_with_budget, REPLY_MERGE_CURSOR};
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::db::init(&pool).await.unwrap();
        sqlx::query("INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, sync_state, created_at, updated_at) VALUES ('a', 'A', 'a@b.test', 'imap', '{}', '', '{}', '', '')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO folders (id, account_id, name, full_path, folder_type, created_at, updated_at) VALUES ('f', 'a', 'INBOX', 'INBOX', 'inbox', '', '')").execute(&pool).await.unwrap();
        for (id, parent) in [
            ("a-child", Some("<z-parent>")),
            ("m-reply", Some("<a-child>")),
            ("z-parent", None),
        ] {
            let thread = format!("th-imap-{id}");
            sqlx::query("INSERT INTO threads (id, account_id, subject, last_message_date, workflow_state, message_count, created_at, updated_at) VALUES (?, 'a', 'S', '2020-01-01', 'inbox', 1, '', '')").bind(&thread).execute(&pool).await.unwrap();
            sqlx::query("INSERT INTO messages (id, account_id, folder_id, thread_id, from_address, to_addresses, subject, date, message_id_header, in_reply_to, created_at, updated_at) VALUES (?, 'a', 'f', ?, 'a@b.test', '[]', 'S', '2020-01-01', ?, ?, '', '')")
                .bind(id).bind(thread).bind(format!("<{id}>")).bind(parent).execute(&pool).await.unwrap();
        }
        // Enough non-replies to put the parent on a later page.
        sqlx::query("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<300) INSERT INTO messages (id, account_id, folder_id, from_address, to_addresses, subject, date, created_at, updated_at) SELECT printf('b%04d', x), 'a', 'f', 'a@b.test', '[]', 'Other', '', '', '' FROM n").execute(&pool).await.unwrap();
        assert_eq!(
            merge_reply_split_threads_with_budget(&pool, Some(1))
                .await
                .unwrap(),
            1
        );
        let cursor: String = sqlx::query_scalar("SELECT value FROM app_settings WHERE key = ?")
            .bind(REPLY_MERGE_CURSOR)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(cursor.contains("b0255"), "{cursor}");
        assert_eq!(
            merge_reply_split_threads_with_budget(&pool, None)
                .await
                .unwrap(),
            1
        );
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(DISTINCT thread_id) FROM messages WHERE thread_id IS NOT NULL",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(count, 1);
        assert!(
            sqlx::query_scalar::<_, String>("SELECT value FROM app_settings WHERE key = ?")
                .bind(REPLY_MERGE_CURSOR)
                .fetch_optional(&pool)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn the_provider_conversation_id_decides_the_thread() {
        let mut message = envelope("m-1");
        message.remote_thread_id = Some("thread-42".to_string());
        message.message_id_header = Some("<abc@example.com>".to_string());
        let standalone = standalone_thread_key("gmail", "INBOX", &message);
        assert_eq!(derive_thread_key(&message, &standalone), "thread-42");
    }

    #[test]
    fn message_id_groups_a_thread_when_the_provider_offers_none() {
        let mut message = envelope("m-1");
        message.message_id_header = Some("<abc@example.com>".to_string());
        let standalone = standalone_thread_key("gmail", "INBOX", &message);
        assert_eq!(
            derive_thread_key(&message, &standalone),
            "<abc@example.com>"
        );
    }

    #[tokio::test]
    async fn historical_imap_autosaves_collapse_per_compose_session() {
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
             VALUES ('drafts', 'account', 'Drafts', 'Drafts', 'drafts', 'Drafts', '2026-01-01', '2026-01-01')",
        ] {
            sqlx::query(statement).execute(&pool).await.expect("seed");
        }

        let versions = [
            (
                "compose-1",
                "<1@imap.flowmail.local>",
                "",
                "Compose",
                "[\"mac@example.com\"]",
                "2026-01-01T10:00:00Z",
            ),
            (
                "compose-2",
                "<2@imap.flowmail.local>",
                "",
                "Compose",
                "[\"mac@example.com\"]",
                "2026-01-01T10:05:00Z",
            ),
            (
                "compose-3",
                "<3@Spark>",
                "",
                "Compose",
                "[\"mac@example.com\"]",
                "2026-01-01T10:10:00Z",
            ),
            // Same participants and subject, but a later compose session.
            (
                "compose-later",
                "<4@Spark>",
                "",
                "Compose",
                "[\"mac@example.com\"]",
                "2026-01-01T14:30:00Z",
            ),
            (
                "reply-1",
                "<5@Spark>",
                "<parent@example.com>",
                "Re: Parent",
                "[\"tim@example.com\"]",
                "2026-01-01T11:00:00Z",
            ),
            (
                "reply-2",
                "<6@Spark>",
                "<parent@example.com>",
                "Re: Parent",
                "[\"tim@example.com\"]",
                "2026-01-01T11:01:00Z",
            ),
            // A different reply target must remain independent.
            (
                "reply-other",
                "<7@Spark>",
                "<other@example.com>",
                "Re: Parent",
                "[\"tim@example.com\"]",
                "2026-01-01T11:02:00Z",
            ),
            // Empty shells are not specific enough for automatic cleanup.
            (
                "empty-1",
                "<8@Spark>",
                "",
                "(no subject)",
                "[]",
                "2026-01-01T12:00:00Z",
            ),
            (
                "empty-2",
                "<9@Spark>",
                "",
                "(no subject)",
                "[]",
                "2026-01-01T12:01:00Z",
            ),
        ];
        for (id, header, parent, subject, to, date) in versions {
            let thread_id = format!("thread-{id}");
            sqlx::query(
                "INSERT INTO threads
                 (id, account_id, subject, last_message_date, participants, snippet, created_at, updated_at)
                 VALUES (?, 'account', ?, ?, '[]', ?, ?, ?)",
            )
            .bind(&thread_id)
            .bind(subject)
            .bind(date)
            .bind(subject)
            .bind(date)
            .bind(date)
            .execute(&pool)
            .await
            .expect("thread");
            sqlx::query(
                "INSERT INTO messages
                 (id, account_id, folder_id, thread_id, message_id_header, in_reply_to,
                  from_address, to_addresses, cc_addresses, bcc_addresses, subject, date,
                  body_text, snippet, is_draft, remote_id, created_at, updated_at)
                 VALUES (?, 'account', 'drafts', ?, ?, NULLIF(?, ''),
                         'me@example.com', ?, '[]', '[]', ?, ?, ?, ?, 1, ?, ?, ?)",
            )
            .bind(id)
            .bind(&thread_id)
            .bind(header)
            .bind(parent)
            .bind(to)
            .bind(subject)
            .bind(date)
            .bind(format!("body {id}"))
            .bind(format!("body {id}"))
            .bind(id)
            .bind(date)
            .bind(date)
            .execute(&pool)
            .await
            .expect("message");
        }
        sqlx::query(
            "INSERT INTO attachments
             (id, message_id, filename, content_type, created_at)
             VALUES ('old-attachment', 'compose-1', 'old.txt', 'text/plain', '2026-01-01')",
        )
        .execute(&pool)
        .await
        .expect("attachment");

        let removed = deduplicate_cached_draft_autosaves(&pool, "account")
            .await
            .expect("deduplicate");
        assert_eq!(removed, 3);

        let remaining: Vec<String> =
            sqlx::query_scalar("SELECT id FROM messages WHERE is_draft = 1 ORDER BY id")
                .fetch_all(&pool)
                .await
                .expect("remaining drafts");
        assert_eq!(
            remaining,
            vec![
                "compose-3",
                "compose-later",
                "empty-1",
                "empty-2",
                "reply-2",
                "reply-other",
            ]
        );
        let stale_attachments: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM attachments WHERE id = 'old-attachment'")
                .fetch_one(&pool)
                .await
                .expect("attachment count");
        assert_eq!(stale_attachments, 0);
    }

    #[test]
    fn an_empty_conversation_id_falls_through_to_the_standalone_key() {
        let mut message = envelope("m-1");
        message.remote_thread_id = Some("   ".to_string());
        message.message_id_header = Some("<abc@example.com>".to_string());
        let standalone = standalone_thread_key("gmail", "INBOX", &message);
        assert_eq!(derive_thread_key(&message, &standalone), "m-1");
    }

    #[test]
    fn a_missing_message_id_is_replaced_by_a_recognisably_local_one() {
        let message = envelope("m-1");
        let header = derive_message_id_header("gmail", "INBOX", &message);
        assert_eq!(header, "<m-1@gmail.satchel.local>");

        let mut with_header = envelope("m-1");
        with_header.message_id_header = Some("<real@example.com>".to_string());
        assert_eq!(
            derive_message_id_header("gmail", "INBOX", &with_header),
            "<real@example.com>"
        );
    }

    /// IMAP requires its stable Message-ID ref and cannot fall back to a UID.
    #[test]
    fn draft_identity_follows_what_the_provider_supplies() {
        let message = envelope("m-1");
        assert_eq!(
            derive_remote_draft_id(true, "outlook", &message).as_deref(),
            Some("m-1")
        );
        assert_eq!(derive_remote_draft_id(true, "imap", &message), None);
        assert_eq!(derive_remote_draft_id(true, "gmail", &message), None);
        assert_eq!(derive_remote_draft_id(false, "outlook", &message), None);

        let mut supplied = envelope("m-1");
        supplied.remote_draft_id = Some("draft-9".to_string());
        assert_eq!(
            derive_remote_draft_id(true, "gmail", &supplied).as_deref(),
            Some("draft-9")
        );
    }

    /// Gmail labels a sent message SENT wherever it is listed, so the folder
    /// alone does not settle it.
    #[test]
    fn a_sent_message_is_recognised_by_folder_or_label() {
        assert!(message_is_sent("sent", &[]));
        assert!(message_is_sent("inbox", &["SENT".to_string()]));
        assert!(message_is_sent("inbox", &["sent".to_string()]));
        assert!(!message_is_sent(
            "inbox",
            &["CATEGORY_PERSONAL".to_string()]
        ));
    }

    #[test]
    fn the_account_name_stands_in_only_for_the_owners_own_mail() {
        let mut message = envelope("m-1");
        assert_eq!(effective_from_name(&message, "Ich", true), Some("Ich"));
        assert_eq!(effective_from_name(&message, "Ich", false), None);

        message.from_name = Some("Lena Becker".to_string());
        assert_eq!(
            effective_from_name(&message, "Ich", true),
            Some("Lena Becker")
        );

        message.from_name = Some("   ".to_string());
        assert_eq!(effective_from_name(&message, "Ich", true), Some("Ich"));
    }

    #[test]
    fn list_headers_reach_the_classifier_with_the_folder() {
        let mut message = envelope("m-1");
        message.list_unsubscribe = Some("<https://example.com/u>".to_string());
        let headers = synthetic_classification_headers(&message, "INBOX");
        assert!(headers.contains("List-Unsubscribe: <https://example.com/u>"));
        assert!(headers.ends_with("INBOX"));

        let plain = synthetic_classification_headers(&envelope("m-1"), "INBOX");
        assert_eq!(plain, "INBOX");
    }
}
