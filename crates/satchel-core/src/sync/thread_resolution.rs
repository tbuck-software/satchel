//! Thread identity, aggregate projection, and cached-draft reconciliation.
//!
//! How an incoming provider message is keyed into a thread (conversation id,
//! `Message-ID`, folder-scoped standalone keys), how replies are resolved to
//! existing threads, how the thread aggregate is refreshed from its messages,
//! and how superseded cached drafts (Spark autosaves, sent-mail copies) are
//! deduplicated after a sync.

use std::collections::{HashMap, HashSet};

use sqlx::{FromRow, SqlitePool};

use super::provider_identity::provider_local_id;
use crate::db::threads::{merge_threads_tx, ThreadRepository};
use crate::providers::traits::MessageEnvelope;

pub async fn refresh_provider_thread_projection(
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
pub async fn refresh_provider_thread_projections(
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

/// Prefer the provider conversation ID, then Message-ID, then the standalone
/// identity. An explicitly empty conversation ID deliberately picks standalone.
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
        provider_local_id(
            "standalone",
            provider_kind,
            &[
                folder_remote_id,
                &uidvalidity.to_string(),
                &message.remote_id,
            ],
        )
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
                provider_local_id("header", provider_kind, &[&native_id]),
                provider_kind.to_ascii_lowercase()
            )
        })
}

/// A typed grouping identity keeps provider conversation IDs distinct from
/// Message-IDs and from folder-scoped standalone messages.
pub fn provider_thread_db_id(
    provider: &str,
    account_id: &str,
    folder_remote_id: &str,
    message: &MessageEnvelope,
) -> String {
    let standalone = standalone_thread_key(provider, folder_remote_id, message);
    let key = derive_thread_key(message, &standalone);
    let kind = match message
        .remote_thread_id
        .as_deref()
        .or(message.message_id_header.as_deref())
    {
        Some(value) if !value.trim().is_empty() && message.remote_thread_id.is_some() => {
            "conversation"
        }
        Some(value) if !value.trim().is_empty() => "message-id",
        _ => "standalone",
    };
    provider_local_id("th", provider, &[account_id, kind, key])
}

/// Resolve exact provider identities under the ingest write lock. Legacy rows
/// are adopted only through an exact message or Message-ID match, never through
/// their old punctuation-stripped IDs.
pub async fn resolve_provider_thread_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    provider: &str,
    account_id: &str,
    account_email: &str,
    folder_remote_id: &str,
    message: &MessageEnvelope,
    prior_thread_id: Option<&str>,
) -> Result<String, String> {
    let identity = provider_thread_db_id(provider, account_id, folder_remote_id, message);
    let mapped = sqlx::query_scalar::<_, String>(
        "SELECT m.thread_id FROM provider_thread_keys k
         JOIN messages m ON m.id = k.message_id
         WHERE k.identity = ? AND m.account_id = ?
           AND NULLIF(TRIM(m.thread_id), '') IS NOT NULL",
    )
    .bind(&identity)
    .bind(account_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|e| e.to_string())?;
    let header_thread = if let Some(header) = message
        .message_id_header
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        sqlx::query_scalar::<_, String>(
            "SELECT thread_id FROM messages WHERE account_id = ? AND message_id_header = ?
             AND NULLIF(TRIM(thread_id), '') IS NOT NULL ORDER BY id LIMIT 1",
        )
        .bind(account_id)
        .bind(header)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| e.to_string())?
    } else {
        None
    };
    let known = prior_thread_id.or(header_thread.as_deref());
    let mut thread_id = if let Some(known) = known
        .filter(|known| *known != identity)
        .filter(|_| mapped.as_deref().is_none_or(|mapped| mapped == identity))
    {
        // A new member may have arrived before the first legacy row was
        // refreshed. Fold that newly generated group into the existing thread
        // so its workflow, tags, topics and user-visible ID survive the upgrade.
        merge_threads_tx(tx, &identity, known)
            .await
            .map_err(|e| e.to_string())?;
        known.to_string()
    } else {
        mapped
            .or_else(|| known.map(str::to_string))
            .unwrap_or(identity)
    };
    if message.remote_thread_id.is_none() {
        if let Some(parent) = resolve_reply_thread_connection(&mut *tx, account_id, message).await?
        {
            thread_id = parent;
        } else if provider.eq_ignore_ascii_case("imap") {
            if let Some(subject) =
                resolve_imap_subject_thread_connection(&mut *tx, account_id, account_email, message)
                    .await?
            {
                thread_id = subject;
            }
        }
    }
    Ok(thread_id)
}

pub async fn remember_provider_thread_connection(
    connection: &mut sqlx::SqliteConnection,
    provider: &str,
    account_id: &str,
    folder_remote_id: &str,
    message: &MessageEnvelope,
    message_id: &str,
) -> Result<(), String> {
    let identity = provider_thread_db_id(provider, account_id, folder_remote_id, message);
    sqlx::query("INSERT INTO provider_thread_keys (identity, message_id) VALUES (?, ?) ON CONFLICT(identity) DO UPDATE SET message_id = excluded.message_id")
        .bind(identity).bind(message_id).execute(connection).await.map_err(|e| e.to_string())?;
    Ok(())
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
    crate::db::sender_preferences::classification_headers(
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
pub async fn resolve_reply_thread_connection(
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
pub async fn resolve_imap_subject_thread_connection(
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
/// the reply's whole thread moves over, topic links included.
///
/// Only IMAP threads split this way; the other providers name the
/// conversation themselves. Each pass checks the messages added since the
/// last one, as replies and as parents, and remembers how far it got.
pub async fn merge_reply_split_threads(pool: &SqlitePool) -> Result<u64, String> {
    merge_reply_split_threads_with_budget(pool, None).await
}

/// Progress of the reply merge across passes.
#[derive(Default, serde::Serialize, serde::Deserialize)]
struct ReplyMergeState {
    /// Every message up to this rowid has been checked.
    checked_rowid: i64,
    /// The pass in progress: the highest rowid it covers ...
    pass_end: Option<i64>,
    /// ... and the rowid it has paged up to.
    after: i64,
    /// Replies whose merge was deferred (a queued remote move holds the
    /// thread), tried again on the next pass.
    deferred: Vec<String>,
}

static REPLY_MERGE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

const REPLY_MERGE_STATE: &str = "maintenance.reply_merge_state";
/// The cursor of the former full-table pass, which restarted on every launch.
const LEGACY_REPLY_MERGE_CURSOR: &str = "maintenance.reply_merge_cursor";
const REPLY_MERGE_PAGE: i64 = 256;
/// Deferred replies kept for retry; a merge that never becomes possible must
/// not grow the state without bound.
const MAX_DEFERRED_REPLIES: usize = 1_000;

/// Replies among, or answering, the messages in `(after, last]`.
const REPLY_MERGE_CANDIDATES: &str = "
    SELECT child.id FROM messages child
    JOIN accounts acc ON acc.id = child.account_id AND acc.provider = 'imap'
    WHERE child.rowid > ?1 AND child.rowid <= ?2
      AND NULLIF(TRIM(child.in_reply_to), '') IS NOT NULL
      AND child.thread_id LIKE 'th-imap-%'
    UNION
    SELECT child.id FROM messages parent
    JOIN accounts acc ON acc.id = parent.account_id AND acc.provider = 'imap'
    JOIN messages child ON child.account_id = parent.account_id
      AND child.in_reply_to = parent.message_id_header
    WHERE parent.rowid > ?1 AND parent.rowid <= ?2
      AND NULLIF(TRIM(parent.message_id_header), '') IS NOT NULL
      AND parent.thread_id LIKE 'th-imap-%'
      AND child.in_reply_to IS NOT NULL
      AND child.thread_id LIKE 'th-imap-%' AND child.thread_id != parent.thread_id";

/// Merge one reply's thread into its parent's. `Ok(None)` when there is
/// nothing to merge, `Ok(Some(0))` when the merge was deferred.
async fn merge_reply_into_parent(pool: &SqlitePool, reply_id: &str) -> Result<Option<u64>, String> {
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
    )
    .bind(reply_id)
    .fetch_optional(pool)
    .await
    .map_err(|err| err.to_string())?;
    let Some((child, parent)) = pair else {
        return Ok(None);
    };
    ThreadRepository::new(pool)
        .merge_into(&child, &parent)
        .await
        .map(Some)
        .map_err(|err| err.to_string())
}

async fn save_reply_merge_state(pool: &SqlitePool, state: &ReplyMergeState) -> Result<(), String> {
    sqlx::query("INSERT INTO app_settings (key, value, updated_at) VALUES (?, ?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at")
        .bind(REPLY_MERGE_STATE)
        .bind(serde_json::to_string(state).map_err(|err| err.to_string())?)
        .bind(chrono::Utc::now().to_rfc3339())
        .execute(pool)
        .await
        .map_err(|err| err.to_string())?;
    Ok(())
}

async fn merge_reply_split_threads_with_budget(
    pool: &SqlitePool,
    max_blocks: Option<u64>,
) -> Result<u64, String> {
    let _guard = REPLY_MERGE_LOCK.lock().await;
    sqlx::query("DELETE FROM app_settings WHERE key = ?")
        .bind(LEGACY_REPLY_MERGE_CURSOR)
        .execute(pool)
        .await
        .map_err(|err| err.to_string())?;
    let saved: Option<String> = sqlx::query_scalar("SELECT value FROM app_settings WHERE key = ?")
        .bind(REPLY_MERGE_STATE)
        .fetch_optional(pool)
        .await
        .map_err(|err| err.to_string())?;
    let mut state: ReplyMergeState = saved
        .and_then(|value| serde_json::from_str(&value).ok())
        .unwrap_or_default();
    let mut moved_total = 0;
    let mut deferred = Vec::new();
    let pass_end = match state.pass_end {
        Some(end) => end,
        None => {
            // A new pass: retry what the last one had to leave.
            for reply_id in std::mem::take(&mut state.deferred) {
                match merge_reply_into_parent(pool, &reply_id).await? {
                    Some(0) => deferred.push(reply_id),
                    Some(moved) => moved_total += moved,
                    None => {}
                }
            }
            let end: Option<i64> = sqlx::query_scalar("SELECT MAX(rowid) FROM messages")
                .fetch_one(pool)
                .await
                .map_err(|err| err.to_string())?;
            let end = end.unwrap_or(0);
            state.after = state.checked_rowid;
            state.pass_end = Some(end);
            end
        }
    };
    let mut blocks = 0;
    while max_blocks.is_none_or(|max| blocks < max) {
        let last: Option<i64> = sqlx::query_scalar(
            "SELECT MAX(rowid) FROM (SELECT rowid FROM messages WHERE rowid > ? AND rowid <= ? ORDER BY rowid LIMIT ?)",
        )
        .bind(state.after)
        .bind(pass_end)
        .bind(REPLY_MERGE_PAGE)
        .fetch_one(pool)
        .await
        .map_err(|err| err.to_string())?;
        let Some(last) = last else {
            state.checked_rowid = pass_end.max(state.checked_rowid);
            state.pass_end = None;
            state.after = state.checked_rowid;
            break;
        };
        let candidates: Vec<String> = sqlx::query_scalar(REPLY_MERGE_CANDIDATES)
            .bind(state.after)
            .bind(last)
            .fetch_all(pool)
            .await
            .map_err(|err| err.to_string())?;
        for reply_id in candidates {
            match merge_reply_into_parent(pool, &reply_id).await? {
                Some(0) => deferred.push(reply_id),
                Some(moved) => moved_total += moved,
                None => {}
            }
        }
        state.after = last;
        // Merges are idempotent. A crash before this commit repeats only this page.
        state.deferred.append(&mut deferred);
        save_reply_merge_state(pool, &state).await?;
        blocks += 1;
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    state.deferred.append(&mut deferred);
    state.deferred.sort();
    state.deferred.dedup();
    if state.deferred.len() > MAX_DEFERRED_REPLIES {
        let excess = state.deferred.len() - MAX_DEFERRED_REPLIES;
        state.deferred.drain(..excess);
    }
    save_reply_merge_state(pool, &state).await?;
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

    async fn reply_merge_pool() -> sqlx::SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::db::init(&pool).await.unwrap();
        sqlx::query("INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, sync_state, created_at, updated_at) VALUES ('a', 'A', 'a@b.test', 'imap', '{}', '', '{}', '', ''), ('g', 'G', 'g@b.test', 'gmail', '{}', '', '{}', '', '')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO folders (id, account_id, name, full_path, folder_type, created_at, updated_at) VALUES ('f', 'a', 'INBOX', 'INBOX', 'inbox', '', ''), ('gf', 'g', 'INBOX', 'INBOX', 'inbox', '', '')").execute(&pool).await.unwrap();
        pool
    }

    /// One message in its own thread, `th-imap-<id>` (or `th-gmail-<id>`).
    async fn reply_merge_message(
        pool: &sqlx::SqlitePool,
        account: &str,
        id: &str,
        parent: Option<&str>,
    ) {
        let (folder, prefix) = if account == "a" {
            ("f", "th-imap")
        } else {
            ("gf", "th-gmail")
        };
        let thread = format!("{prefix}-{id}");
        sqlx::query("INSERT INTO threads (id, account_id, subject, last_message_date, workflow_state, message_count, created_at, updated_at) VALUES (?, ?, 'S', '2020-01-01', 'inbox', 1, '', '')").bind(&thread).bind(account).execute(pool).await.unwrap();
        sqlx::query("INSERT INTO messages (id, account_id, folder_id, thread_id, from_address, to_addresses, subject, date, message_id_header, in_reply_to, created_at, updated_at) VALUES (?, ?, ?, ?, 'a@b.test', '[]', 'S', '2020-01-01', ?, ?, '', '')")
            .bind(id).bind(account).bind(folder).bind(thread).bind(format!("<{id}>")).bind(parent.map(|p| format!("<{p}>"))).execute(pool).await.unwrap();
    }

    async fn thread_of(pool: &sqlx::SqlitePool, id: &str) -> String {
        sqlx::query_scalar("SELECT thread_id FROM messages WHERE id = ?")
            .bind(id)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn reply_merge_resumes_a_saved_page_and_converges_a_chain() {
        use super::{merge_reply_split_threads_with_budget, REPLY_MERGE_STATE};
        let pool = reply_merge_pool().await;
        reply_merge_message(&pool, "a", "a-child", Some("z-parent")).await;
        // Enough non-replies to put the rest of the chain on a later page.
        sqlx::query("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<300) INSERT INTO messages (id, account_id, folder_id, from_address, to_addresses, subject, date, created_at, updated_at) SELECT printf('b%04d', x), 'a', 'f', 'a@b.test', '[]', 'Other', '', '', '' FROM n").execute(&pool).await.unwrap();
        reply_merge_message(&pool, "a", "m-reply", Some("a-child")).await;
        reply_merge_message(&pool, "a", "z-parent", None).await;

        assert_eq!(
            merge_reply_split_threads_with_budget(&pool, Some(1))
                .await
                .unwrap(),
            2,
            "the first page holds a reply and the parent of another"
        );
        let state: String = sqlx::query_scalar("SELECT value FROM app_settings WHERE key = ?")
            .bind(REPLY_MERGE_STATE)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(state.contains("\"after\":256"), "{state}");
        assert_eq!(
            merge_reply_split_threads_with_budget(&pool, None)
                .await
                .unwrap(),
            0
        );
        assert_eq!(thread_of(&pool, "a-child").await, "th-imap-z-parent");
        assert_eq!(thread_of(&pool, "m-reply").await, "th-imap-z-parent");
        let state: String = sqlx::query_scalar("SELECT value FROM app_settings WHERE key = ?")
            .bind(REPLY_MERGE_STATE)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(state.contains("\"checked_rowid\":303"), "{state}");
        assert!(state.contains("\"pass_end\":null"), "{state}");
    }

    #[tokio::test]
    async fn later_reply_merge_passes_check_only_new_messages_as_replies_and_parents() {
        use super::merge_reply_split_threads;
        let pool = reply_merge_pool().await;
        reply_merge_message(&pool, "a", "old-parent", None).await;
        reply_merge_message(&pool, "a", "early-reply", Some("late-parent")).await;
        reply_merge_message(&pool, "a", "settled", None).await;
        reply_merge_message(&pool, "a", "settled-reply", Some("settled")).await;
        assert_eq!(merge_reply_split_threads(&pool).await.unwrap(), 1);
        assert_eq!(thread_of(&pool, "settled-reply").await, "th-imap-settled");

        // Split a checked pair by hand: a later pass must not page back over it.
        sqlx::query("INSERT INTO threads (id, account_id, subject, last_message_date, workflow_state, message_count, created_at, updated_at) VALUES ('th-imap-resplit', 'a', 'S', '2020-01-01', 'inbox', 1, '', '')").execute(&pool).await.unwrap();
        sqlx::query("UPDATE messages SET thread_id = 'th-imap-resplit' WHERE id = 'settled-reply'")
            .execute(&pool)
            .await
            .unwrap();
        // A new reply to an old parent, and a new parent of an old reply.
        reply_merge_message(&pool, "a", "new-reply", Some("old-parent")).await;
        reply_merge_message(&pool, "a", "late-parent", None).await;
        // Other providers name their conversations; their replies stay put.
        reply_merge_message(&pool, "g", "gmail-parent", None).await;
        reply_merge_message(&pool, "g", "gmail-reply", Some("gmail-parent")).await;

        assert_eq!(merge_reply_split_threads(&pool).await.unwrap(), 2);
        assert_eq!(thread_of(&pool, "new-reply").await, "th-imap-old-parent");
        assert_eq!(thread_of(&pool, "early-reply").await, "th-imap-late-parent");
        assert_eq!(thread_of(&pool, "settled-reply").await, "th-imap-resplit");
        assert_eq!(
            thread_of(&pool, "gmail-reply").await,
            "th-gmail-gmail-reply"
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
    fn conversation_and_message_id_namespaces_remain_distinct() {
        let mut message = envelope("42");
        message.remote_thread_id = Some("<same@example.com>".to_string());
        let conversation = super::provider_thread_db_id("imap", "account", "INBOX", &message);
        message.remote_thread_id = None;
        message.message_id_header = Some("<same@example.com>".to_string());
        assert_ne!(
            conversation,
            super::provider_thread_db_id("imap", "account", "INBOX", &message)
        );
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
        sqlx::migrate!("./src/db/migrations")
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
        assert!(header.starts_with("<header-gmail-v2-"));
        assert!(header.ends_with("@gmail.satchel.local>"));
        assert_ne!(
            header,
            derive_message_id_header("gmail", "INBOX", &envelope("m.1"))
        );

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
