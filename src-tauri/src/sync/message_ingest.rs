//! Persist provider messages and their derived local projections.

use std::{collections::HashSet, sync::Arc};

use satchel_core::db::blocked_senders::BlockedSenderRepository;
use satchel_core::db::sender_preferences::{
    classify_message_with_provider_labels, provider_category, SenderPreferenceRepository,
};
use satchel_core::db::threads::{refresh_thread_projections_tx, ThreadRepository};
use satchel_core::sync::provider_identity::{provider_local_id, provider_message_db_id};
use satchel_core::sync::queue::SyncQueue;
use satchel_core::sync::remote_mutations::stable_queue_id;
use sqlx::{Acquire, SqlitePool};
use tauri::{AppHandle, Manager};

use crate::events::bus::EventBus;
use crate::providers::traits::MessageEnvelope;
use crate::sync::attachment_processing::process_special_attachment;
use crate::sync::folder_reconcile::{is_snooze_folder, reconcile_external_workflow_states};
use crate::sync::gmail_reconcile::{SnapshotDeferral, SnapshotResolution};
use crate::sync::thread_resolution::{
    deduplicate_spark_draft_versions, derive_message_id_header, derive_remote_draft_id,
    effective_from_name, message_is_sent, reconcile_sent_draft_autosaves,
    refresh_provider_thread_projections, remember_provider_thread_connection,
    resolve_provider_thread_tx, synthetic_classification_headers,
};

/// Read and star values protected by queued mutations in this write transaction.
#[derive(Debug, Default)]
struct PendingFlagLocks(HashSet<(String, String)>);

impl PendingFlagLocks {
    /// Read once per write transaction, after acquiring the ingest lock.
    async fn load(
        connection: &mut sqlx::SqliteConnection,
        account_id: &str,
    ) -> Result<Self, String> {
        crate::sync::folder_reconcile::pending_remote_message_refs(
            connection,
            account_id,
            crate::sync::folder_reconcile::PendingMutations::FlagsOnly,
        )
        .await
        .map(Self)
    }

    fn holds(&self, folder_remote_id: &str, remote_id: &str) -> bool {
        // Same normalisation the queue side applies when the set is built.
        self.0
            .contains(&(folder_remote_id.to_ascii_lowercase(), remote_id.to_string()))
    }
}

pub(crate) async fn upsert_provider_message(
    app: Option<&AppHandle>,
    pool: &SqlitePool,
    provider_kind: &str,
    account_id: &str,
    account_email: &str,
    account_display_name: &str,
    account_config: &crate::commands::accounts::AccountConfig,
    folder_id: &str,
    folder_remote_id: &str,
    folder_type: &str,
    message: &crate::providers::traits::MessageEnvelope,
    events: Option<&Arc<EventBus>>,
) -> Result<(), String> {
    let cutoff: Option<String> = sqlx::query_scalar(
        "SELECT COALESCE(NULLIF(last_sync_at, ''), NULLIF(created_at, '')) FROM accounts WHERE id = ?"
    ).bind(account_id).fetch_optional(pool).await.map_err(|error| error.to_string())?.flatten();
    upsert_provider_messages(
        app,
        pool,
        provider_kind,
        account_id,
        account_email,
        account_display_name,
        account_config,
        folder_id,
        folder_remote_id,
        folder_type,
        std::slice::from_ref(message),
        events,
        cutoff.as_deref(),
    )
    .await?
    .remove(0)
}

struct StoredMessage {
    message_id: String,
    thread_id: String,
    was_pre_existing: bool,
    message_is_draft: bool,
    message_is_sent: bool,
    attachment_ids: Vec<String>,
}

pub(crate) const INGEST_BATCH_SIZE: usize = 100;

pub(crate) async fn upsert_provider_messages(
    app: Option<&AppHandle>,
    pool: &SqlitePool,
    provider_kind: &str,
    account_id: &str,
    account_email: &str,
    account_display_name: &str,
    account_config: &crate::commands::accounts::AccountConfig,
    folder_id: &str,
    folder_remote_id: &str,
    folder_type: &str,
    messages: &[MessageEnvelope],
    events: Option<&Arc<EventBus>>,
    cutoff: Option<&str>,
) -> Result<Vec<Result<(), String>>, String> {
    let mut results = Vec::with_capacity(messages.len());
    for chunk in messages.chunks(INGEST_BATCH_SIZE) {
        let body_html = prepare_message_html(chunk).await?;
        let mut tx = pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(|error| error.to_string())?;
        let flag_locks = PendingFlagLocks::load(&mut tx, account_id).await?;
        let mut stored = Vec::new();
        for (message, body_html) in chunk.iter().zip(&body_html) {
            let mut savepoint = tx.begin().await.map_err(|error| error.to_string())?;
            let result = persist_provider_message(
                &mut savepoint,
                provider_kind,
                account_id,
                account_email,
                account_display_name,
                folder_id,
                folder_remote_id,
                folder_type,
                message,
                body_html.as_deref(),
                &flag_locks,
                cutoff,
            )
            .await;
            match result {
                Ok(value) => {
                    savepoint
                        .commit()
                        .await
                        .map_err(|error| error.to_string())?;
                    stored.push((message, value));
                    results.push(Ok(()));
                }
                Err(error) => {
                    savepoint
                        .rollback()
                        .await
                        .map_err(|error| error.to_string())?;
                    results.push(Err(error));
                }
            }
        }
        let mut contacts = Vec::new();
        for (message, _) in &stored {
            let sender_is_account = message
                .from_address
                .trim()
                .eq_ignore_ascii_case(account_email.trim());
            contacts.push((
                message.from_address.as_str(),
                effective_from_name(message, account_display_name, sender_is_account),
            ));
            contacts.extend(
                message
                    .to_addresses
                    .iter()
                    .chain(message.cc_addresses.iter())
                    .map(|address| (address.as_str(), None)),
            );
        }
        let mut contact_savepoint = tx.begin().await.map_err(|error| error.to_string())?;
        match satchel_core::db::contacts::touch_contacts_tx(&mut contact_savepoint, &contacts).await
        {
            Ok(()) => contact_savepoint
                .commit()
                .await
                .map_err(|error| error.to_string())?,
            Err(error) => {
                contact_savepoint
                    .rollback()
                    .await
                    .map_err(|error| error.to_string())?;
                tracing::warn!(%error, "contact persistence failed during message ingest");
            }
        }
        tx.commit().await.map_err(|error| error.to_string())?;
        replay_ingest_effects(pool, account_id).await?;
        for (message, stored) in stored {
            if let Err(error) = finish_provider_message(
                app,
                pool,
                provider_kind,
                account_id,
                account_email,
                account_config,
                folder_id,
                folder_remote_id,
                folder_type,
                message,
                stored,
                events,
            )
            .await
            {
                tracing::warn!(%error, remote_id = %message.remote_id, "post-ingest processing deferred");
            }
        }
    }
    if messages.is_empty() {
        replay_ingest_effects(pool, account_id).await?;
    }
    if messages.len() >= INGEST_BATCH_SIZE {
        for table in ["messages_fts", "message_cc_fts"] {
            if let Err(error) = sqlx::query(&format!(
                "INSERT INTO {table}({table}, rank) VALUES ('merge', 1000)"
            ))
            .execute(pool)
            .await
            {
                tracing::warn!(%error, table, "FTS merge deferred");
            }
        }
    }
    Ok(results)
}

/// Snapshot recovery must store every current label before losing the last
/// obsolete copy. Reuse the normal persistence/organization guards, but give
/// all targets of this delivery a single rollback boundary.
///
/// `Err` is reserved for failures that stop the whole snapshot run; anything
/// specific to this delivery defers only its ID.
pub(crate) async fn upsert_gmail_snapshot_message(
    app: Option<&AppHandle>,
    pool: &SqlitePool,
    account_id: &str,
    account_email: &str,
    account_display_name: &str,
    account_config: &crate::commands::accounts::AccountConfig,
    message: &MessageEnvelope,
    events: Option<&Arc<EventBus>>,
    cutoff: Option<&str>,
) -> Result<SnapshotResolution, String> {
    use satchel_core::db::sync_quarantine::SyncQuarantineRepository;
    let repository = SyncQuarantineRepository::new(pool);
    let mut targets = Vec::new();
    for label in crate::sync::gmail_reconcile::gmail_target_mailbox_ids(&message.mailbox_ids) {
        let (folder_id, folder_type) =
            crate::sync::gmail_reconcile::ensure_gmail_label_folder(pool, account_id, &label)
                .await?;
        if repository
            .load_folder(account_id, &folder_id)
            .await
            .map_err(|error| error.to_string())?
            .is_exhausted(&message.remote_id)
        {
            return Ok(SnapshotResolution::Deferred(SnapshotDeferral::Quarantined));
        }
        targets.push((folder_id, label, folder_type));
    }
    // Read before the replacement: obsolete copies may sit in other threads.
    let threads: HashSet<String> = sqlx::query_scalar(
        "SELECT DISTINCT thread_id FROM messages WHERE account_id = ? AND remote_id = ?",
    )
    .bind(account_id)
    .bind(&message.remote_id)
    .fetch_all(pool)
    .await
    .map_err(|error| error.to_string())?
    .into_iter()
    .collect();
    let html = prepare_message_html(std::slice::from_ref(message)).await?;
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|error| error.to_string())?;
    if crate::sync::gmail_reconcile::gmail_snapshot_message_pinned_tx(
        &mut tx,
        account_id,
        &message.remote_id,
    )
    .await?
    {
        tx.rollback().await.map_err(|error| error.to_string())?;
        return Ok(SnapshotResolution::Deferred(
            SnapshotDeferral::PendingMutation,
        ));
    }
    let flag_locks = PendingFlagLocks::load(&mut tx, account_id).await?;
    let mut stored = Vec::new();
    for (folder_id, label, folder_type) in &targets {
        match persist_provider_message(
            &mut tx,
            "gmail",
            account_id,
            account_email,
            account_display_name,
            folder_id,
            label,
            folder_type,
            message,
            html[0].as_deref(),
            &flag_locks,
            cutoff,
        )
        .await
        {
            Ok(value) => stored.push(value),
            Err(error) => {
                tx.rollback().await.map_err(|error| error.to_string())?;
                let attempts = repository
                    .record_failure(account_id, folder_id, &message.remote_id, &error)
                    .await
                    .map_err(|error| error.to_string())?;
                // The quarantine counts target failures; its last one holds
                // the ID like any other exhausted target.
                if attempts >= satchel_core::db::sync_quarantine::MAX_INGEST_ATTEMPTS {
                    tracing::warn!(account_id, folder_id, remote_id = %message.remote_id,
                        attempts, "snapshot target used up its attempts and is skipped: {error}");
                    return Ok(SnapshotResolution::Deferred(SnapshotDeferral::Quarantined));
                }
                return Ok(SnapshotResolution::Deferred(SnapshotDeferral::Failed(
                    crate::sync::errors::SyncError::Transient(error),
                )));
            }
        }
    }
    // Every actual target is safe now. Clear obsolete-label failures too,
    // otherwise they could keep rediscovering an already recovered delivery.
    sqlx::query("DELETE FROM sync_quarantine WHERE account_id = ? AND remote_id = ?")
        .bind(account_id)
        .bind(&message.remote_id)
        .execute(&mut *tx)
        .await
        .map_err(|error| error.to_string())?;
    tx.commit().await.map_err(|error| error.to_string())?;
    replay_ingest_effects(pool, account_id).await?;
    for ((folder_id, label, folder_type), stored) in targets.iter().zip(stored) {
        if let Err(error) = finish_provider_message(
            app,
            pool,
            "gmail",
            account_id,
            account_email,
            account_config,
            folder_id,
            label,
            folder_type,
            message,
            stored,
            events,
        )
        .await
        {
            tracing::warn!(%error, remote_id = %message.remote_id, "post-ingest processing deferred");
        }
    }
    Ok(SnapshotResolution::Resolved(threads))
}

/// Replay rules after a commit, including intent left by a previous process.
pub(crate) async fn replay_ingest_effects(
    pool: &SqlitePool,
    account_id: &str,
) -> Result<(), String> {
    let pending: Vec<(String, String)> = sqlx::query_as(
        "SELECT message_id, thread_id FROM ingest_effects WHERE account_id = ? ORDER BY created_at, message_id"
    ).bind(account_id).fetch_all(pool).await.map_err(|error| error.to_string())?;
    for chunk in pending.chunks(INGEST_BATCH_SIZE) {
        crate::commands::rules::run_pending_ingest_rules(pool, chunk).await?;
    }
    Ok(())
}

async fn prepare_message_html(messages: &[MessageEnvelope]) -> Result<Vec<Option<String>>, String> {
    let source = messages
        .iter()
        .map(|message| message.body_html.clone())
        .collect::<Vec<_>>();
    if source.iter().all(Option::is_none) {
        return Ok(source);
    }
    tokio::task::spawn_blocking(move || {
        source
            .into_iter()
            .map(|html| {
                html.and_then(|html| {
                    let cleaned = crate::email::sanitizer::sanitize_email_html(&html);
                    (!cleaned.trim().is_empty()).then_some(cleaned)
                })
            })
            .collect()
    })
    .await
    .map_err(|error| error.to_string())
}

async fn persist_provider_message(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    provider_kind: &str,
    account_id: &str,
    account_email: &str,
    account_display_name: &str,
    folder_id: &str,
    folder_remote_id: &str,
    folder_type: &str,
    message: &crate::providers::traits::MessageEnvelope,
    body_html: Option<&str>,
    flag_locks: &PendingFlagLocks,
    cutoff: Option<&str>,
) -> Result<StoredMessage, String> {
    let flags_locked = flag_locks.holds(folder_remote_id, &message.remote_id);
    let now = chrono::Utc::now().to_rfc3339();
    let message_is_draft = provider_message_is_draft(folder_type, &message.mailbox_ids);
    let remote_draft_id = derive_remote_draft_id(message_is_draft, provider_kind, message);
    let mut provider_message_row = sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT id, thread_id FROM messages
         WHERE account_id = ? AND folder_id = ? AND remote_id = ?
         ORDER BY updated_at DESC LIMIT 1",
    )
    .bind(account_id)
    .bind(folder_id)
    .bind(&message.remote_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|err| err.to_string())?;
    if provider_message_row.is_none() && provider_kind.eq_ignore_ascii_case("gmail") {
        // Reuse an obsolete label copy rather than replacing its local ID.
        // Message topic links, cached attachments and other references survive
        // the move along with the thread, within this message's savepoint.
        let labels = crate::sync::gmail_reconcile::gmail_target_mailbox_ids(&message.mailbox_ids);
        let labels = serde_json::to_string(&labels).map_err(|err| err.to_string())?;
        provider_message_row = sqlx::query_as::<_, (String, Option<String>)>(
            "SELECT m.id, m.thread_id FROM messages m JOIN folders f ON f.id = m.folder_id
             WHERE m.account_id = ? AND m.remote_id = ?
               AND NOT EXISTS (SELECT 1 FROM json_each(?) label
                               WHERE LOWER(TRIM(label.value)) = LOWER(TRIM(f.remote_id)))
             ORDER BY m.id LIMIT 1",
        )
        .bind(account_id)
        .bind(&message.remote_id)
        .bind(&labels)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|err| err.to_string())?;
    }
    let provider_message_id = if let Some((id, _)) = &provider_message_row {
        id.clone()
    } else {
        let mut id = provider_message_db_id(provider_kind, account_id, folder_remote_id, message);
        // A prior relabel may have adopted this deterministic ID into another
        // still-current folder. Allocate a distinct copy under the write lock;
        // subsequent ingests find it by the exact account/folder/remote tuple.
        if provider_kind.eq_ignore_ascii_case("gmail") {
            while sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS(SELECT 1 FROM messages WHERE id = ?)",
            )
            .bind(&id)
            .fetch_one(&mut **tx)
            .await
            .map_err(|err| err.to_string())?
            {
                id = provider_local_id(
                    "m",
                    provider_kind,
                    &[
                        account_id,
                        folder_remote_id,
                        &message.remote_id,
                        &uuid::Uuid::new_v4().to_string(),
                    ],
                );
            }
        }
        id
    };
    let message_id = if let Some(remote_draft_id) = remote_draft_id.as_deref() {
        sqlx::query_scalar::<_, String>(
            "SELECT id FROM messages
             WHERE account_id = ? AND remote_draft_id = ?
             ORDER BY updated_at DESC LIMIT 1",
        )
        .bind(account_id)
        .bind(remote_draft_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|err| err.to_string())?
        .unwrap_or(provider_message_id)
    } else {
        provider_message_id
    };
    let prior_message = sqlx::query_as::<_, (Option<String>, Option<String>)>(
        "SELECT thread_id, message_id_header FROM messages WHERE id = ?",
    )
    .bind(&message_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|error| error.to_string())?;
    let prior_thread_id = prior_message
        .as_ref()
        .and_then(|row| row.0.as_deref())
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string);
    let thread_id = resolve_provider_thread_tx(
        tx,
        provider_kind,
        account_id,
        account_email,
        folder_remote_id,
        message,
        prior_thread_id.as_deref(),
    )
    .await?;
    let message_id_header = if message
        .message_id_header
        .as_deref()
        .is_none_or(|value| value.trim().is_empty())
    {
        // Locally composed replies may already reference this synthetic header.
        prior_message
            .as_ref()
            .and_then(|row| row.1.as_deref())
            .filter(|header| {
                header.ends_with(".satchel.local>") || header.ends_with(".flowmail.local>")
            })
            .map(str::to_string)
    } else {
        None
    }
    .unwrap_or_else(|| derive_message_id_header(provider_kind, folder_remote_id, message));

    // Pre-check whether this message already exists locally — drives whether we
    // fire a "new mail" notification after the upsert succeeds.
    let was_pre_existing = sqlx::query_scalar::<_, i64>(
        "SELECT 1 FROM messages WHERE id = ? OR (account_id = ? AND message_id_header = ? AND ? != '' AND ? NOT LIKE '%@%.satchel.local>') LIMIT 1"
    ).bind(&message_id).bind(account_id).bind(&message_id_header).bind(&message_id_header).bind(&message_id_header)
        .fetch_optional(&mut **tx).await.map_err(|error| error.to_string())?.is_some();

    let newly_seen = !was_pre_existing
        && cutoff.is_some_and(|cutoff| {
            // Provider receive time when known, like the notification cutoff.
            let seen = message
                .received_at_rfc3339
                .as_deref()
                .unwrap_or(&message.date_rfc3339);
            chrono::DateTime::parse_from_rfc3339(seen)
                .ok()
                .zip(chrono::DateTime::parse_from_rfc3339(cutoff).ok())
                .is_some_and(|(date, cutoff)| date >= cutoff)
        });

    let synthetic_headers = synthetic_classification_headers(message, folder_remote_id);
    let provider_category = provider_category(&message.mailbox_ids);
    let automatic_category = classify_message_with_provider_labels(
        &message.mailbox_ids,
        Some(&synthetic_headers),
        &message.from_address,
        Some(&message.subject),
    );
    // Sender preferences span accounts: the same person writing to a second
    // mailbox is still the person the user prioritised.
    // Provider data can contain an empty From address. It cannot match a
    // sender preference, so keep automatic classification for that fixture
    // shape instead of asking the lookup to validate an empty address.
    let sender_preference = if message.from_address.trim().is_empty() {
        None
    } else {
        Some(
            SenderPreferenceRepository::get_with_executor(&mut **tx, &message.from_address)
                .await
                .map_err(|error| error.to_string())?,
        )
    };
    let category = sender_preference
        .as_ref()
        .and_then(|preference| preference.category.as_deref())
        .unwrap_or(automatic_category)
        .to_string();
    let category_source = if sender_preference
        .as_ref()
        .is_some_and(|preference| preference.category.is_some())
    {
        "manual_sender"
    } else if provider_category.is_some() {
        "provider"
    } else {
        "heuristic"
    };
    let sender_is_priority = sender_preference
        .as_ref()
        .is_some_and(|preference| preference.is_priority);
    let provider_is_important = message
        .mailbox_ids
        .iter()
        .any(|label| label.eq_ignore_ascii_case("IMPORTANT"));
    let derived_priority = if sender_is_priority || provider_is_important {
        2_i64
    } else {
        1_i64
    };
    let priority_source = if sender_is_priority {
        "manual_sender"
    } else if provider_is_important {
        "provider"
    } else {
        "heuristic"
    };
    let provider_labels =
        serde_json::to_string(&message.mailbox_ids).unwrap_or_else(|_| "[]".to_string());

    let has_attachments = if message.attachments.is_empty() { 0 } else { 1 };

    let message_is_read = if message.is_read { 1 } else { 0 };
    let message_is_starred = if message.is_starred { 1 } else { 0 };
    let message_is_sent = message_is_sent(folder_type, &message.mailbox_ids);
    let sender_is_account = message
        .from_address
        .trim()
        .eq_ignore_ascii_case(account_email.trim());
    let effective_from_name = effective_from_name(message, account_display_name, sender_is_account);
    let unread_count = if message.is_read { 0 } else { 1 };

    // Display fields (subject/date/snippet/participants) only move forward:
    // backfill and multi-folder re-syncs deliver *older* messages of the same
    // thread, and without the WHERE guard they would drag the thread back in
    // time in the list and show a stale snippet. Dates are all UTC RFC3339, so
    // string comparison is chronological.
    sqlx::query(
        "INSERT INTO threads (id, account_id, subject, last_message_date, message_count, unread_count, has_attachments, participants, snippet, workflow_state, workflow_changed_at, snooze_until, due_date, priority, board_column, board_position, category, category_source, priority_source, created_at, updated_at)
         VALUES (?, ?, ?, ?, 1, ?, ?, ?, ?, 'inbox', ?, NULL, NULL, ?, 'inbox', 1, ?, ?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET
            subject = excluded.subject,
            last_message_date = excluded.last_message_date,
            participants = excluded.participants,
            snippet = excluded.snippet,
            category = CASE WHEN threads.category_source = 'manual_thread'
                            THEN threads.category ELSE excluded.category END,
            category_source = CASE WHEN threads.category_source = 'manual_thread'
                                   THEN threads.category_source ELSE excluded.category_source END,
            priority = CASE WHEN threads.priority_source = 'manual_thread'
                            THEN threads.priority ELSE excluded.priority END,
            priority_source = CASE WHEN threads.priority_source = 'manual_thread'
                                   THEN threads.priority_source ELSE excluded.priority_source END,
            updated_at = excluded.updated_at
         WHERE excluded.last_message_date >= threads.last_message_date",
    )
    .bind(&thread_id)
    .bind(account_id)
    .bind(&message.subject)
    .bind(&message.date_rfc3339)
    .bind(unread_count)
    .bind(has_attachments)
    .bind(
        serde_json::to_string(&vec![message.from_address.clone()])
            .unwrap_or_else(|_| "[]".to_string()),
    )
    .bind(&message.snippet)
    .bind(&now)
    .bind(derived_priority)
    .bind(&category)
    .bind(category_source)
    .bind(priority_source)
    .bind(&now)
    .bind(&now)
    .execute(&mut **tx)
    .await
    .map_err(|err| err.to_string())?;

    // Order-independent thread attributes, applied regardless of message age
    // (they used to live in the conflict clause above, which is now gated).
    sqlx::query(
        "UPDATE threads
         SET has_attachments = MAX(has_attachments, ?),
             category = CASE
               WHEN category_source = 'manual_thread' THEN category
               WHEN ? >= last_message_date THEN ?
               ELSE category
             END,
             category_source = CASE
               WHEN category_source = 'manual_thread' THEN category_source
               WHEN ? >= last_message_date THEN ?
               ELSE category_source
             END,
             priority = CASE
               WHEN priority_source = 'manual_thread' THEN priority
               WHEN ? >= last_message_date THEN ?
               ELSE priority
             END,
             priority_source = CASE
               WHEN priority_source = 'manual_thread' THEN priority_source
               WHEN ? >= last_message_date THEN ?
               ELSE priority_source
             END
         WHERE id = ?",
    )
    .bind(has_attachments)
    .bind(&message.date_rfc3339)
    .bind(&category)
    .bind(&message.date_rfc3339)
    .bind(category_source)
    .bind(&message.date_rfc3339)
    .bind(derived_priority)
    .bind(&message.date_rfc3339)
    .bind(priority_source)
    .bind(&thread_id)
    .execute(&mut **tx)
    .await
    .map_err(|err| err.to_string())?;

    if provider_kind == "imap" {
        // A legacy row can retain its local id when sync verifies it again.
        // Content from its unverified or earlier generation must not survive
        // COALESCE in the upsert, even when this envelope is header-only.
        sqlx::query(
            "UPDATE attachments SET cached_path = NULL, content_base64 = NULL, calendar_scan_at = NULL
            WHERE message_id = ? AND EXISTS (SELECT 1 FROM messages
                WHERE id = ? AND remote_uidvalidity IS NOT ?)",
        )
        .bind(&message_id)
        .bind(&message_id)
        .bind(message.remote_uidvalidity.map(i64::from))
        .execute(&mut **tx)
        .await
        .map_err(|error| error.to_string())?;
        sqlx::query(
            "UPDATE messages SET body_text = NULL, body_html = NULL,
                original_body_html = NULL, original_source = NULL, raw_headers = NULL, raw_size = NULL,
                draft_attachments_json = NULL,
                smime_repair_attempted_at = NULL
            WHERE id = ? AND remote_uidvalidity IS NOT ?",
        )
        .bind(&message_id)
        .bind(message.remote_uidvalidity.map(i64::from))
        .execute(&mut **tx)
        .await
        .map_err(|error| error.to_string())?;
    }

    // `has_attachments` only ever rises. A re-sync that returns the envelope
    // without its parts is missing information, not reporting a message that
    // lost its attachments — mail is immutable on the server.
    sqlx::query(
        "INSERT INTO messages (id, account_id, folder_id, thread_id, message_id_header, in_reply_to, references_header, from_address, from_name, to_addresses, cc_addresses, bcc_addresses, reply_to, subject, date, body_text, body_html, original_body_html, original_source, snippet, is_read, is_starred, is_draft, is_sent, has_attachments, remote_uid, remote_id, raw_size, raw_headers, remote_draft_id, category, provider_labels, list_unsubscribe, list_unsubscribe_post, auth_spf, auth_dkim, auth_dmarc, smime_encrypted, smime_signed, pgp_signed, trust_metadata, created_at, updated_at, received_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, NULL, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET
            folder_id = excluded.folder_id,
            thread_id = excluded.thread_id,
            remote_uid = excluded.remote_uid,
            remote_id = excluded.remote_id,
            message_id_header = excluded.message_id_header,
            in_reply_to = excluded.in_reply_to,
            references_header = excluded.references_header,
            from_address = excluded.from_address,
            from_name = excluded.from_name,
            to_addresses = excluded.to_addresses,
            cc_addresses = excluded.cc_addresses,
            bcc_addresses = excluded.bcc_addresses,
            reply_to = excluded.reply_to,
            subject = excluded.subject,
            date = excluded.date,
            received_at = COALESCE(excluded.received_at, messages.received_at),
            body_text = CASE WHEN json_extract(excluded.trust_metadata, '$.root_crypto') THEN excluded.body_text ELSE COALESCE(excluded.body_text, messages.body_text) END,
            body_html = CASE WHEN json_extract(excluded.trust_metadata, '$.root_crypto') THEN excluded.body_html ELSE COALESCE(excluded.body_html, messages.body_html) END,
            original_body_html = CASE WHEN json_extract(excluded.trust_metadata, '$.root_crypto') THEN excluded.original_body_html ELSE COALESCE(excluded.original_body_html, messages.original_body_html) END,
            original_source = COALESCE(excluded.original_source, messages.original_source),
            snippet = excluded.snippet,
            is_read = CASE WHEN ? THEN messages.is_read ELSE excluded.is_read END,
            is_starred = CASE WHEN ? THEN messages.is_starred ELSE excluded.is_starred END,
            is_draft = excluded.is_draft,
            is_sent = excluded.is_sent,
            has_attachments = MAX(messages.has_attachments, excluded.has_attachments),
            category = excluded.category,
            provider_labels = excluded.provider_labels,
            list_unsubscribe = excluded.list_unsubscribe,
            list_unsubscribe_post = excluded.list_unsubscribe_post,
            auth_spf = excluded.auth_spf,
            auth_dkim = excluded.auth_dkim,
            auth_dmarc = excluded.auth_dmarc,
            smime_encrypted = excluded.smime_encrypted,
            smime_signed = excluded.smime_signed,
            pgp_signed = excluded.pgp_signed,
            trust_metadata = excluded.trust_metadata,
            remote_draft_id = COALESCE(excluded.remote_draft_id, messages.remote_draft_id),
            draft_attachments_json = CASE
                WHEN excluded.is_draft = 1 THEN NULL
                ELSE messages.draft_attachments_json
            END,
            updated_at = excluded.updated_at",
    )
    .bind(&message_id)
    .bind(account_id)
    .bind(folder_id)
    .bind(&thread_id)
    .bind(&message_id_header)
    .bind(message.in_reply_to.as_deref())
    .bind(message.references.join(" "))
    .bind(&message.from_address)
    .bind(effective_from_name)
    .bind(
        serde_json::to_string(&message.to_addresses).unwrap_or_else(|_| "[]".to_string()),
    )
    .bind(
        serde_json::to_string(&message.cc_addresses).unwrap_or_else(|_| "[]".to_string()),
    )
    .bind(
        serde_json::to_string(&message.bcc_addresses).unwrap_or_else(|_| "[]".to_string()),
    )
    .bind(message.reply_to.as_deref())
    .bind(&message.subject)
    .bind(&message.date_rfc3339)
    .bind(message.body_text.as_deref())
    .bind(body_html)
    .bind(message.body_html.as_deref())
    .bind(message.original_source.as_deref())
    .bind(&message.snippet)
    .bind(message_is_read)
    .bind(message_is_starred)
    .bind(if message_is_draft { 1 } else { 0 })
    .bind(if message_is_sent { 1 } else { 0 })
    .bind(has_attachments)
    .bind(message.remote_id.parse::<i64>().ok())
    .bind(&message.remote_id)
    .bind(remote_draft_id.as_deref())
    .bind(&category)
    .bind(&provider_labels)
    .bind(message.list_unsubscribe.as_deref())
    .bind(message.list_unsubscribe_post.as_deref())
    .bind(message.auth_spf.as_deref())
    .bind(message.auth_dkim.as_deref())
    .bind(message.auth_dmarc.as_deref())
    .bind(message.smime_encrypted.as_deref())
    .bind(message.smime_signed.as_deref())
    .bind(message.pgp_signed.as_deref())
    .bind(serde_json::to_string(&message.trust).map_err(|err| err.to_string())?)
    .bind(&now)
    .bind(&now)
    .bind(message.received_at_rfc3339.as_deref())
    // The two guards of the conflict clause; its placeholders come after every
    // one of the inserted values.
    .bind(flags_locked)
    .bind(flags_locked)
    .execute(&mut **tx)
    .await
    .map_err(|err| err.to_string())?;

    // Stamp the generation from the fetched envelope, never from current folder state.
    sqlx::query("UPDATE messages SET remote_uidvalidity = ? WHERE id = ?")
        .bind(message.remote_uidvalidity.map(i64::from))
        .bind(&message_id)
        .execute(&mut **tx)
        .await
        .map_err(|error| error.to_string())?;

    remember_provider_thread_connection(
        &mut *tx,
        provider_kind,
        account_id,
        folder_remote_id,
        message,
        &message_id,
    )
    .await?;
    // An envelope that lists no attachments carries no statement about them.
    // Deleting on that emptied the stored metadata for every message a provider
    // hands over without its parts — S/MIME, multipart/signed, anything past the
    // parser's depth limit — and the next maintenance run then dropped the cache
    // files nothing pointed at any more.
    if !message.attachments.is_empty() {
        let remote_attachment_parts = serde_json::to_string(
            &message
                .attachments
                .iter()
                .map(|attachment| attachment.remote_part_id.as_str())
                .collect::<Vec<_>>(),
        )
        .map_err(|err| err.to_string())?;
        // Parts of decrypted content have no remote part, so no envelope
        // names them; they belong to the decrypted body, not the server's list.
        sqlx::query(
            "DELETE FROM attachments
             WHERE message_id = ?
               AND id NOT LIKE ?
               AND NOT EXISTS (
                 SELECT 1 FROM json_each(?) current_attachment
                 WHERE current_attachment.value = attachments.remote_part_id
               )",
        )
        .bind(&message_id)
        .bind(satchel_core::email::decrypted::attachment_id_pattern())
        .bind(remote_attachment_parts)
        .execute(&mut **tx)
        .await
        .map_err(|err| err.to_string())?;
    }

    let mut attachment_ids = Vec::with_capacity(message.attachments.len());
    for attachment in &message.attachments {
        let attachment_id = sqlx::query_scalar::<_, String>(
            "SELECT id FROM attachments WHERE message_id = ? AND remote_part_id = ? ORDER BY id LIMIT 1",
        ).bind(&message_id).bind(&attachment.remote_part_id)
            .fetch_optional(&mut **tx).await.map_err(|error| error.to_string())?
            .unwrap_or_else(|| provider_local_id("att", provider_kind, &[&message_id, &attachment.remote_part_id]));
        sqlx::query(
            "INSERT INTO attachments (id, message_id, filename, content_type, size, content_id, storage_path, is_inline, cached_path, remote_uid, remote_part_id, created_at, content_base64, listing_rules)
             VALUES (?, ?, ?, ?, ?, ?, NULL, ?, NULL, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
                filename = excluded.filename,
                content_type = excluded.content_type,
                size = excluded.size,
                content_id = excluded.content_id,
                is_inline = excluded.is_inline,
                cached_path = CASE
                    WHEN attachments.remote_uid IS NOT excluded.remote_uid THEN NULL
                    ELSE attachments.cached_path
                END,
                remote_uid = excluded.remote_uid,
                remote_part_id = excluded.remote_part_id,
                content_base64 = COALESCE(excluded.content_base64, attachments.content_base64),
                listing_rules = excluded.listing_rules",
        )
        .bind(&attachment_id)
        .bind(&message_id)
        .bind(attachment.filename.as_deref().map(satchel_core::utils::filename::strip_filename_controls))
        .bind(&attachment.content_type)
        .bind(attachment.size)
        .bind(attachment.content_id.as_deref())
        .bind(if attachment.is_inline { 1 } else { 0 })
        .bind(&message.remote_id)
        .bind(&attachment.remote_part_id)
        .bind(&now)
        .bind(attachment.content_base64.as_deref())
        .bind(satchel_core::db::signed_attachments::LISTING_RULES)
        .execute(&mut **tx)
        .await
        .map_err(|err| err.to_string())?;
        attachment_ids.push(attachment_id);
    }

    // Replacement, stale-copy cleanup and every affected thread projection
    // belong to the same savepoint.
    let mut committed_projection_threads = vec![thread_id.clone()];
    if provider_kind.eq_ignore_ascii_case("gmail") {
        let touched = crate::sync::gmail_reconcile::reconcile_gmail_message_mailboxes_tx(
            tx, account_id, message,
        )
        .await?;
        committed_projection_threads.extend(touched.into_iter().filter(|id| id != &thread_id));
    }
    if let Some(prior_thread_id) = prior_thread_id.as_ref() {
        if prior_thread_id != &thread_id {
            committed_projection_threads.push(prior_thread_id.clone());
        }
    }
    refresh_thread_projections_tx(tx, &committed_projection_threads)
        .await
        .map_err(|error| error.to_string())?;

    if newly_seen && !message_is_draft {
        if message_is_sent && sender_is_account {
            sqlx::query("UPDATE threads SET workflow_state = 'waiting', board_column = 'waiting', workflow_changed_at = ?, updated_at = ? WHERE id = ? AND workflow_state IN ('inbox', 'todo') AND last_message_date <= ?")
                .bind(&now).bind(&now).bind(&thread_id).bind(&message.date_rfc3339)
                .execute(&mut **tx).await.map_err(|error| error.to_string())?;
        } else if !sender_is_account {
            sqlx::query("UPDATE follow_up_reminders SET status = 'resolved', resolved_at = ?, updated_at = ? WHERE thread_id = ? AND status = 'pending'")
                .bind(&now).bind(&now).bind(&thread_id).execute(&mut **tx).await.map_err(|error| error.to_string())?;
            if folder_type == "inbox" {
                sqlx::query("UPDATE threads SET workflow_state = 'inbox', board_column = 'inbox', snooze_until = NULL, workflow_changed_at = ?, updated_at = ? WHERE id = ? AND workflow_state IN ('snoozed', 'aside', 'done', 'waiting')")
                    .bind(&now).bind(&now).bind(&thread_id).execute(&mut **tx).await.map_err(|error| error.to_string())?;
                sqlx::query("INSERT OR IGNORE INTO ingest_effects (message_id, thread_id, account_id, run_rules, created_at) VALUES (?, ?, ?, 1, ?)")
                    .bind(&message_id).bind(&thread_id).bind(account_id).bind(&now)
                    .execute(&mut **tx).await.map_err(|error| error.to_string())?;
            }
        }
    }
    Ok(StoredMessage {
        message_id,
        thread_id,
        was_pre_existing,
        message_is_draft,
        message_is_sent,
        attachment_ids,
    })
}

async fn finish_provider_message(
    app: Option<&AppHandle>,
    pool: &SqlitePool,
    provider_kind: &str,
    account_id: &str,
    account_email: &str,
    account_config: &crate::commands::accounts::AccountConfig,
    folder_id: &str,
    folder_remote_id: &str,
    folder_type: &str,
    message: &crate::providers::traits::MessageEnvelope,
    stored: StoredMessage,
    events: Option<&Arc<EventBus>>,
) -> Result<(), String> {
    let StoredMessage {
        message_id,
        thread_id,
        was_pre_existing,
        message_is_draft,
        message_is_sent,
        attachment_ids,
    } = stored;
    // Provider downloads and event publishing follow the committed rows.
    if folder_type == "archive" || folder_type == "aside" || is_snooze_folder(folder_remote_id) {
        reconcile_external_workflow_states(pool, &HashSet::from([thread_id.clone()])).await?;
    }

    // Calendar parts and inline images are downloaded from the provider, so
    // this runs on the committed attachment rows rather than inside the write
    // transaction.
    for (attachment_id, attachment) in attachment_ids.iter().zip(message.attachments.iter()) {
        if attachment.is_inline
            && attachment
                .content_type
                .trim()
                .to_ascii_lowercase()
                .starts_with("image/")
        {
            continue;
        }
        process_special_attachment(
            app,
            pool,
            provider_kind,
            account_id,
            account_config,
            account_email,
            &message_id,
            &message.remote_id,
            folder_remote_id,
            message.remote_uidvalidity,
            attachment_id,
            attachment,
        )
        .await;
    }

    // Only the threads the deduplication touched need another projection pass;
    // this message and the thread it left were refreshed inside the commit.
    let mut projection_threads = deduplicate_spark_draft_versions(
        pool,
        account_id,
        folder_id,
        message,
        &message_id,
        message_is_draft,
    )
    .await?;
    if message_is_sent && provider_kind.eq_ignore_ascii_case("imap") {
        projection_threads.extend(reconcile_sent_draft_autosaves(pool, account_id).await?);
    }
    refresh_provider_thread_projections(pool, &projection_threads.into_iter().collect::<Vec<_>>())
        .await?;

    // Auto-trash if sender is on the blocked list. Pre-existing messages are
    // retried while their local blocked projection is still open.
    // A lookup failure is fail-open for ingestion, but it must remain visible in
    // diagnostics rather than being silently treated as a non-match.
    // A headless ingest uses a durable queue too; the same queue is used for
    // diagnostics and the blocked move below.
    let blocked_sender_queue = app
        .and_then(|app| app.try_state::<crate::AppState>())
        .map(|state| state.queue.clone())
        .unwrap_or_else(|| Arc::new(SyncQueue::new()));
    let should_check_sender = should_check_blocked_sender(
        pool,
        blocked_sender_queue.as_ref(),
        was_pre_existing,
        account_id,
        &thread_id,
    )
    .await;
    let sender_is_blocked = if should_check_sender {
        match BlockedSenderRepository::new(pool)
            .is_blocked(account_id, &message.from_address)
            .await
        {
            Ok(blocked) => blocked,
            Err(error) => {
                tracing::warn!(
                    error = ?error,
                    account_id,
                    "blocked sender lookup failed during message ingest"
                );
                persist_blocked_sender_diagnostic(
                    pool,
                    blocked_sender_queue.as_ref(),
                    account_id,
                    "lookup",
                )
                .await;
                false
            }
        }
    } else {
        false
    };

    if sender_is_blocked {
        // Queue the provider move while the message still has its source
        // folder. IMAP UIDs are scoped to that folder, so local projection
        // must commit with the queue insertion.
        // A headless ingest still persists into the durable queue. It has no
        // attached worker to consume the wake, but the next normal sync can
        // process the pending row.
        match crate::commands::blocked_senders::prepare_blocked_thread_moves(
            pool,
            Some(account_id),
            &thread_id,
        )
        .await
        {
            Ok(actions) => {
                if let Err(error) = ThreadRepository::new(pool)
                    .commit_blocked_moves(
                        blocked_sender_queue.as_ref(),
                        std::slice::from_ref(&thread_id),
                        &actions,
                    )
                    .await
                {
                    tracing::warn!(
                        error = ?error,
                        account_id,
                        thread_id,
                        "blocked sender move and local projection commit failed during ingest"
                    );
                    persist_blocked_sender_diagnostic(
                        pool,
                        blocked_sender_queue.as_ref(),
                        account_id,
                        "commit",
                    )
                    .await;
                }
            }
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    account_id,
                    thread_id,
                    "blocked sender remote move preparation failed during ingest"
                );
                persist_blocked_sender_diagnostic(
                    pool,
                    blocked_sender_queue.as_ref(),
                    account_id,
                    "prepare",
                )
                .await;
                // Do not expose a local-only move when the queue could not
                // persist its provider operation. The projection remains open
                // for a subsequent ingest attempt.
            }
        }
    }

    // Detect calendar invitations: prefer the standalone text/calendar part if
    // the IMAP parser pulled one out, otherwise scan body text/html (Outlook/Gmail
    // often inline the invite).
    let invite_source = message.body_calendar.clone().or_else(|| {
        crate::email::calendar::extract_ics_block(
            message.body_text.as_deref(),
            message.body_html.as_deref(),
        )
    });
    if let Some(raw_ics) = invite_source {
        if let Some(invite) = crate::email::calendar::parse_invite(&raw_ics) {
            let _ = crate::db::calendar::CalendarRepository::new(pool)
                .upsert_invite(account_id, Some(&message_id), &invite)
                .await;
        }
    }

    // The insert trigger wrote the outbox row for eligible Inbox mail, and it
    // committed with the message above. Publish it immediately when a live
    // event loop exists; a later sync will replay the pending row after a crash.
    if !was_pre_existing {
        if let Some(bus) = events {
            let _ = crate::commands::notifications::publish_pending_mail_notifications(
                pool,
                bus.as_ref(),
                Some(&message_id),
            )
            .await;
        }
    }

    Ok(())
}

async fn should_check_blocked_sender(
    pool: &SqlitePool,
    queue: &SyncQueue,
    was_pre_existing: bool,
    account_id: &str,
    thread_id: &str,
) -> bool {
    if !was_pre_existing {
        return true;
    }
    match ThreadRepository::new(pool)
        .blocked_projection_pending(thread_id)
        .await
    {
        Ok(pending) => pending,
        Err(error) => {
            tracing::warn!(
                error = ?error,
                account_id,
                thread_id,
                "blocked sender projection state lookup failed during message ingest"
            );
            persist_blocked_sender_diagnostic(pool, queue, account_id, "projection-state").await;
            false
        }
    }
}

async fn persist_blocked_sender_diagnostic(
    pool: &SqlitePool,
    queue: &SyncQueue,
    account_id: &str,
    error_kind: &str,
) {
    let mut action = satchel_core::sync::queue::QueuedAction::new(
        account_id.to_string(),
        "blocked_sender_diagnostic",
        serde_json::json!({ "kind": error_kind }),
    );
    action.id = stable_queue_id("blocked-sender-diagnostic", error_kind, account_id);
    action.status = "failed".to_string();
    action.last_error = Some("backend.errors.blockedSenderPersistenceFailed".to_string());
    if let Err(error) = queue.enqueue(pool, action).await {
        tracing::error!(
            error = %error,
            account_id,
            error_kind,
            "could not persist blocked sender diagnostic"
        );
    }
}

fn provider_message_is_draft(folder_type: &str, mailbox_ids: &[String]) -> bool {
    folder_type == "drafts"
        || mailbox_ids
            .iter()
            .any(|label| label.eq_ignore_ascii_case("DRAFT"))
}

#[cfg(test)]
mod tests {
    use super::{
        persist_blocked_sender_diagnostic, provider_message_db_id, provider_message_is_draft,
        should_check_blocked_sender, upsert_provider_message,
    };
    use crate::providers::traits::{MessageEnvelope, RemoteAttachment, RemoteMessageState};
    use crate::sync::folder_reconcile::reconcile_imap_folder_snapshot;
    use crate::sync::gmail_reconcile::{gmail_labels_include, reconcile_gmail_message_mailboxes};
    use crate::sync::queue::SyncQueue;
    use crate::sync::thread_resolution::{merge_reply_split_threads, standalone_thread_key};
    use sqlx::sqlite::SqlitePoolOptions;

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

    #[tokio::test]
    async fn retains_download_only_original_bytes_across_envelope_refreshes() {
        let pool = ingest_pool().await;
        let mut message = envelope("pop-original");
        let raw = b"Subject: Original\r\n\r\n\xff\0\r\n".to_vec();
        message.original_source = Some(raw.clone());
        ingest(&pool, "inbox", &message).await.unwrap();
        message.original_source = None;
        ingest(&pool, "inbox", &message).await.unwrap();
        let stored: Vec<u8> = sqlx::query_scalar(
            "SELECT original_source FROM messages WHERE remote_id = 'pop-original'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(stored, raw);
    }

    #[tokio::test]
    async fn sanitizes_stored_html_and_retains_the_original() {
        let pool = ingest_pool().await;
        let mut message = envelope("html-sanitize");
        message.body_html = Some("<p onclick=bad()>Hi<script>bad()</script></p>".into());
        ingest(&pool, "inbox", &message).await.unwrap();
        let (cleaned, original): (String, String) = sqlx::query_as(
            "SELECT body_html, original_body_html FROM messages WHERE remote_id = 'html-sanitize'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(cleaned, "<p>Hi</p>");
        assert_eq!(original, message.body_html.unwrap());
    }

    #[tokio::test]
    async fn root_coverage_replaces_stale_unsigned_body_representations_atomically() {
        let pool = ingest_pool().await;
        let mut message = envelope("covered-body");
        message.from_address = "bob@bank.example".into();
        message.body_text = Some("Old body".into());
        message.body_html = Some("<p>Unsigned stale HTML</p>".into());
        ingest(&pool, "inbox", &message).await.unwrap();
        message.trust.root_crypto = true;
        message.body_text = Some("Signed plain text".into());
        message.body_html = None;
        ingest(&pool, "inbox", &message).await.unwrap();
        let (text, html, trust): (String, Option<String>, String) = sqlx::query_as("SELECT body_text, body_html, trust_metadata FROM messages WHERE remote_id = 'covered-body'").fetch_one(&pool).await.unwrap();
        assert_eq!(text, "Signed plain text");
        assert!(html.is_none());
        assert!(
            serde_json::from_str::<crate::providers::traits::MessageTrust>(&trust)
                .unwrap()
                .root_crypto
        );
    }

    #[tokio::test]
    async fn ingest_rechecks_preexisting_blocked_sender_until_projection_is_done() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("pool");
        sqlx::migrate!("../crates/satchel-core/src/db/migrations")
            .run(&pool)
            .await
            .expect("migrations");
        sqlx::query(
            "INSERT INTO accounts
             (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
             VALUES ('account', 'Account', 'account@example.com', 'imap', '{}', '', '', '')",
        )
        .execute(&pool)
        .await
        .expect("account");
        sqlx::query(
            "INSERT INTO threads
             (id, account_id, subject, last_message_date, workflow_state, created_at, updated_at)
             VALUES ('thread', 'account', 'Subject', '', 'inbox', '', '')",
        )
        .execute(&pool)
        .await
        .expect("thread");

        let queue = SyncQueue::new();
        assert!(should_check_blocked_sender(&pool, &queue, true, "account", "thread").await);
        assert!(should_check_blocked_sender(&pool, &queue, false, "account", "thread").await);
        sqlx::query(
            "UPDATE threads SET workflow_state = 'done', board_column = 'done' WHERE id = 'thread'",
        )
        .execute(&pool)
        .await
        .expect("completed projection");
        assert!(!should_check_blocked_sender(&pool, &queue, true, "account", "thread").await);
    }

    #[tokio::test]
    async fn blocked_sender_diagnostic_is_stable_terminal_and_redacted() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("pool");
        sqlx::migrate!("../crates/satchel-core/src/db/migrations")
            .run(&pool)
            .await
            .expect("migrations");
        sqlx::query(
            "INSERT INTO accounts
             (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
             VALUES ('account', 'Account', 'account@example.com', 'imap', '{}', '', '', '')",
        )
        .execute(&pool)
        .await
        .expect("account");

        let queue = SyncQueue::new();
        persist_blocked_sender_diagnostic(&pool, &queue, "account", "lookup").await;
        persist_blocked_sender_diagnostic(&pool, &queue, "account", "lookup").await;
        let row: (String, String, String, String) = sqlx::query_as(
            "SELECT id, action_type, status, last_error
             FROM sync_queue WHERE account_id = 'account'",
        )
        .fetch_one(&pool)
        .await
        .expect("diagnostic row");
        assert!(row.0.starts_with("blocked-sender-diagnostic:lookup:"));
        assert_eq!(row.1, "blocked_sender_diagnostic");
        assert_eq!(row.2, "failed");
        assert_eq!(row.3, "backend.errors.blockedSenderPersistenceFailed");
        assert!(queue
            .claim_next(&pool)
            .await
            .expect("claim diagnostic")
            .is_none());
    }

    async fn ingest_pool() -> sqlx::SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("pool");
        seed_ingest_pool(&pool).await;
        pool
    }

    async fn seed_ingest_pool(pool: &sqlx::SqlitePool) {
        sqlx::migrate!("../crates/satchel-core/src/db/migrations")
            .run(pool)
            .await
            .expect("migrations");
        sqlx::query(
            "INSERT INTO accounts
             (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
             VALUES ('account', 'Account', 'account@example.com', 'imap', '{}', '', '', '')",
        )
        .execute(pool)
        .await
        .expect("account");
        sqlx::query(
            "INSERT INTO folders
             (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at)
             VALUES ('inbox', 'account', 'INBOX', 'INBOX', 'inbox', 'INBOX', '', '')",
        )
        .execute(pool)
        .await
        .expect("folder");
    }

    async fn ingest_many(
        pool: &sqlx::SqlitePool,
        messages: &[MessageEnvelope],
    ) -> Vec<Result<(), String>> {
        let cutoff: Option<String> = sqlx::query_scalar(
        "SELECT COALESCE(NULLIF(last_sync_at, ''), NULLIF(created_at, '')) FROM accounts WHERE id = ?"
    ).bind("account").fetch_optional(pool).await.unwrap().flatten();
        super::upsert_provider_messages(
            None,
            pool,
            "imap",
            "account",
            "account@example.com",
            "Account",
            &crate::commands::accounts::AccountConfig::from_plaintext("{}"),
            "inbox",
            "INBOX",
            "inbox",
            messages,
            None,
            cutoff.as_deref(),
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn automatic_rules_run_before_notification_publication() {
        let pool = ingest_pool().await;
        let cutoff = (chrono::Utc::now() - chrono::TimeDelta::minutes(1)).to_rfc3339();
        sqlx::query("UPDATE accounts SET last_sync_at = ?")
            .bind(&cutoff)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO rules (id, name, trigger_type, trigger_config, actions, is_active, priority, created_at, updated_at) VALUES ('rule', 'Read incoming mail', 'on_receive', '{\"from_contains\":\"external\"}', '[\"mark_read\"]', 1, 1, '', '')")
            .execute(&pool).await.unwrap();
        crate::commands::notifications::notification_session_started_at();
        let mut message = envelope("7");
        message.is_read = false;
        message.from_address = "external@example.test".into();
        message.date_rfc3339 = chrono::Utc::now().to_rfc3339();
        let bus = std::sync::Arc::new(crate::events::bus::EventBus::new(16));
        let mut received = bus.subscribe();
        super::upsert_provider_message(
            None,
            &pool,
            "imap",
            "account",
            "account@example.com",
            "Account",
            &crate::commands::accounts::AccountConfig::from_plaintext("{}"),
            "inbox",
            "INBOX",
            "inbox",
            &message,
            Some(&bus),
        )
        .await
        .unwrap();
        let state: String = sqlx::query_scalar("SELECT state FROM notification_outbox")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(state, "dismissed");
        assert!(
            received.try_recv().is_err(),
            "read mail must not publish NewMail"
        );
    }

    #[tokio::test]
    async fn batch_preparation_sanitizes_html_before_persistence() {
        let pool = ingest_pool().await;
        let mut message = envelope("unsafe-html");
        message.body_html = Some("<p onclick=bad()>Hi<script>bad()</script></p>".into());
        assert!(ingest_many(&pool, &[message]).await[0].is_ok());
        let stored: String = sqlx::query_scalar("SELECT body_html FROM messages")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(stored, "<p>Hi</p>");
    }

    #[tokio::test]
    async fn inline_images_wait_for_the_attachment_backfill() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let requests = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let recorded = requests.clone();
        let server = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0; 8192];
                if socket.read(&mut request).await.unwrap() == 0 {
                    continue;
                }
                recorded.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let body = r#"{"contentBytes":"AQ==","contentType":"image/png"}"#;
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            }
        });
        std::env::set_var("SATCHEL_GRAPH_API_BASE", format!("http://{address}"));
        let pool = ingest_pool().await;
        let mut message = envelope("inline");
        message.attachments.push(RemoteAttachment {
            remote_part_id: "image".into(),
            filename: Some("image.png".into()),
            content_type: "image/png".into(),
            size: Some(1),
            is_inline: true,
            content_id: Some("image".into()),
            content_base64: None,
        });
        let config = crate::commands::accounts::AccountConfig::from_plaintext(
            r#"{"oauth":{"access_token":"test-token"}}"#,
        );
        let results = super::upsert_provider_messages(
            None,
            &pool,
            "outlook",
            "account",
            "account@example.com",
            "Account",
            &config,
            "inbox",
            "INBOX",
            "inbox",
            &[message],
            None,
            None,
        )
        .await
        .unwrap();
        assert!(results[0].is_ok());
        assert_eq!(requests.load(std::sync::atomic::Ordering::Relaxed), 0);
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM attachments WHERE is_inline = 1 AND cached_path IS NULL"
            )
            .fetch_one(&pool)
            .await
            .unwrap(),
            1
        );
        std::env::remove_var("SATCHEL_GRAPH_API_BASE");
        server.abort();
    }

    #[tokio::test]
    async fn embedded_inline_payloads_survive_headless_ingest() {
        let pool = ingest_pool().await;
        let mut message = envelope("embedded");
        message.attachments.push(RemoteAttachment {
            remote_part_id: "image".into(),
            filename: Some("image.png".into()),
            content_type: "image/png".into(),
            size: Some(1),
            is_inline: true,
            content_id: Some("image".into()),
            content_base64: Some("AQ==".into()),
        });
        assert!(ingest_many(&pool, &[message]).await[0].is_ok());
        let payload: String = sqlx::query_scalar("SELECT content_base64 FROM attachments")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(
            crate::sync::attachment_fetch::decode_attachment_base64(&payload).unwrap(),
            vec![1]
        );
    }

    #[tokio::test]
    async fn a_bad_message_rolls_back_only_its_savepoint_and_contacts_are_combined() {
        let pool = ingest_pool().await;
        sqlx::query("CREATE TRIGGER reject_broken BEFORE INSERT ON messages WHEN new.remote_id = 'broken' BEGIN SELECT RAISE(ABORT, 'broken'); END")
            .execute(&pool).await.unwrap();
        let messages = ["first", "broken", "last"].map(|id| {
            let mut message = envelope(id);
            message.from_address = "same@example.test".into();
            message.to_addresses = vec!["recipient@example.test".into()];
            message
        });
        let results = ingest_many(&pool, &messages).await;
        assert!(results[0].is_ok());
        assert!(results[1].is_err());
        assert!(results[2].is_ok());
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM messages")
                .fetch_one(&pool)
                .await
                .unwrap(),
            2
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM threads")
                .fetch_one(&pool)
                .await
                .unwrap(),
            2
        );
        let frequencies: Vec<i64> =
            sqlx::query_scalar("SELECT frequency FROM contacts ORDER BY email")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(frequencies, vec![2, 2]);
    }

    #[tokio::test]
    async fn replies_see_parents_inserted_in_the_same_batch() {
        let pool = ingest_pool().await;
        let mut parent = envelope("parent");
        parent.message_id_header = Some("<parent@example.test>".into());
        let mut reply = envelope("reply");
        reply.in_reply_to = parent.message_id_header.clone();
        assert!(ingest_many(&pool, &[parent, reply])
            .await
            .iter()
            .all(Result::is_ok));
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(DISTINCT thread_id) FROM messages")
                .fetch_one(&pool)
                .await
                .unwrap(),
            1
        );
    }

    #[tokio::test]
    async fn overnight_reply_wakes_waiting_and_snoozed_threads_and_resolves_followups() {
        for state in ["waiting", "snoozed"] {
            let pool = ingest_pool().await;
            let mut parent = envelope("parent");
            parent.message_id_header = Some("<parent@example.test>".into());
            parent.date_rfc3339 = "2026-01-01T08:00:00Z".into();
            ingest(&pool, "inbox", &parent).await.unwrap();
            let thread: String = sqlx::query_scalar("SELECT thread_id FROM messages")
                .fetch_one(&pool)
                .await
                .unwrap();
            sqlx::query(
                "UPDATE threads SET workflow_state = ?, snooze_until = '2026-12-01T00:00:00Z'",
            )
            .bind(state)
            .execute(&pool)
            .await
            .unwrap();
            sqlx::query("UPDATE accounts SET last_sync_at = '2026-01-01T12:00:00Z'")
                .execute(&pool)
                .await
                .unwrap();
            sqlx::query("INSERT INTO follow_up_reminders (id, account_id, thread_id, due_at, status, created_at, updated_at) VALUES ('reminder', 'account', ?, '2026-01-03T00:00:00Z', 'pending', '', '')")
                .bind(&thread).execute(&pool).await.unwrap();
            sqlx::query("INSERT INTO rules (id, name, trigger_type, trigger_config, actions, is_active, priority, created_at, updated_at) VALUES ('rule', 'Reply rule', 'on_receive', '{\"from_contains\":\"external\"}', '[\"set_priority:3\"]', 1, 1, '', '')")
                .execute(&pool).await.unwrap();
            let mut reply = envelope("overnight");
            reply.in_reply_to = parent.message_id_header.clone();
            reply.from_address = "external@example.test".into();
            reply.date_rfc3339 = "2026-01-02T03:00:00Z".into();
            ingest(&pool, "inbox", &reply).await.unwrap();
            let result: (String, Option<String>, i64) = sqlx::query_as(
                "SELECT workflow_state, snooze_until, priority FROM threads WHERE id = ?",
            )
            .bind(&thread)
            .fetch_one(&pool)
            .await
            .unwrap();
            assert_eq!(result, ("inbox".into(), None, 3));
            assert_eq!(
                sqlx::query_scalar::<_, String>("SELECT status FROM follow_up_reminders")
                    .fetch_one(&pool)
                    .await
                    .unwrap(),
                "resolved"
            );
            ingest(&pool, "inbox", &reply).await.unwrap();
            assert_eq!(
                sqlx::query_scalar::<_, i64>("SELECT trigger_count FROM rules")
                    .fetch_one(&pool)
                    .await
                    .unwrap(),
                1
            );
        }
    }

    #[tokio::test]
    async fn historical_backfill_and_folder_copies_do_not_run_reply_effects() {
        let pool = ingest_pool().await;
        let mut parent = envelope("parent");
        parent.message_id_header = Some("<parent@example.test>".into());
        parent.date_rfc3339 = "2026-01-01T08:00:00Z".into();
        ingest(&pool, "inbox", &parent).await.unwrap();
        sqlx::query("UPDATE threads SET workflow_state = 'waiting'")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("UPDATE accounts SET last_sync_at = '2026-01-03T00:00:00Z'")
            .execute(&pool)
            .await
            .unwrap();
        let mut old = envelope("backfill");
        old.in_reply_to = parent.message_id_header.clone();
        old.date_rfc3339 = "2026-01-02T00:00:00Z".into();
        old.from_address = "external@example.test".into();
        ingest(&pool, "inbox", &old).await.unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, String>("SELECT workflow_state FROM threads LIMIT 1")
                .fetch_one(&pool)
                .await
                .unwrap(),
            "waiting"
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM ingest_effects")
                .fetch_one(&pool)
                .await
                .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn committed_rule_intent_replays_after_a_crash_and_failed_rules_stay_pending() {
        let pool = ingest_pool().await;
        sqlx::query("UPDATE accounts SET last_sync_at = '2026-01-01T00:00:00Z'")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO rules (id, name, trigger_type, trigger_config, actions, is_active, priority, created_at, updated_at) VALUES ('rule', 'Reply rule', 'on_receive', '{\"from_contains\":\"external\"}', '[\"set_state:todo\",\"unsupported\"]', 1, 1, '', '')")
            .execute(&pool).await.unwrap();
        let mut message = envelope("crash");
        message.from_address = "external@example.test".into();
        message.date_rfc3339 = "2026-01-02T03:00:00Z".into();
        let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
        super::persist_provider_message(
            &mut tx,
            "imap",
            "account",
            "account@example.com",
            "Account",
            "inbox",
            "INBOX",
            "inbox",
            &message,
            None,
            &super::PendingFlagLocks::default(),
            Some("2026-01-01T00:00:00Z"),
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM ingest_effects")
                .fetch_one(&pool)
                .await
                .unwrap(),
            1
        );
        super::replay_ingest_effects(&pool, "account")
            .await
            .unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, String>("SELECT workflow_state FROM threads")
                .fetch_one(&pool)
                .await
                .unwrap(),
            "inbox"
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM ingest_effects")
                .fetch_one(&pool)
                .await
                .unwrap(),
            1
        );
        sqlx::query("UPDATE rules SET actions = '[\"set_state:todo\"]'")
            .execute(&pool)
            .await
            .unwrap();
        super::replay_ingest_effects(&pool, "account")
            .await
            .unwrap();
        super::replay_ingest_effects(&pool, "account")
            .await
            .unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM ingest_effects")
                .fetch_one(&pool)
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT trigger_count FROM rules")
                .fetch_one(&pool)
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM workflow_events WHERE reason = 'rule'"
            )
            .fetch_one(&pool)
            .await
            .unwrap(),
            1
        );
    }

    #[tokio::test]
    #[ignore = "synthetic ingest measurement"]
    async fn synthetic_ingest_measurement() {
        let messages = (0..200)
            .map(|index| {
                let mut message = envelope(&format!("bench-{index}"));
                message.from_address = "sender@example.test".into();
                message.to_addresses = vec!["one@example.test".into(), "two@example.test".into()];
                message.cc_addresses = vec!["three@example.test".into()];
                message
            })
            .collect::<Vec<_>>();
        let directory = tempfile::tempdir_in(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../target"),
        )
        .unwrap();
        let file = directory.path().join("ingest.db");
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                sqlx::sqlite::SqliteConnectOptions::new()
                    .filename(&file)
                    .create_if_missing(true)
                    .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
                    .synchronous(sqlx::sqlite::SqliteSynchronous::Normal)
                    .foreign_keys(true),
            )
            .await
            .unwrap();
        seed_ingest_pool(&pool).await;
        sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("PRAGMA wal_autocheckpoint = 0")
            .execute(&pool)
            .await
            .unwrap();
        let before = wal_commit_count(&file);
        let config = crate::commands::accounts::AccountConfig::from_plaintext("{}");
        let started = std::time::Instant::now();
        let results = super::upsert_provider_messages(
            None,
            &pool,
            "imap",
            "account",
            "account@example.com",
            "Account",
            &config,
            "inbox",
            "INBOX",
            "inbox",
            &messages,
            None,
            None,
        )
        .await
        .unwrap();
        assert!(results.iter().all(Result::is_ok));
        let elapsed = started.elapsed();
        let commits = wal_commit_count(&file) - before;
        println!("synthetic WAL batch ingest: 200 messages, {commits} commits, {elapsed:?}");
        assert!(commits <= 4, "unbounded ingest commits: {commits}");
        pool.close().await;
    }

    fn wal_commit_count(database: &std::path::Path) -> usize {
        let wal = database.with_file_name(format!(
            "{}-wal",
            database.file_name().unwrap().to_str().unwrap()
        ));
        let Ok(bytes) = std::fs::read(wal) else {
            return 0;
        };
        if bytes.len() < 32 {
            return 0;
        }
        let page_size = u32::from_be_bytes(bytes[8..12].try_into().unwrap()) as usize;
        bytes[32..]
            .chunks_exact(24 + page_size)
            .filter(|frame| u32::from_be_bytes(frame[4..8].try_into().unwrap()) != 0)
            .count()
    }

    async fn ingest(
        pool: &sqlx::SqlitePool,
        folder_id: &str,
        message: &MessageEnvelope,
    ) -> Result<(), String> {
        upsert_provider_message(
            None,
            pool,
            "imap",
            "account",
            "account@example.com",
            "Account",
            &crate::commands::accounts::AccountConfig::from_plaintext("{}"),
            folder_id,
            "INBOX",
            "inbox",
            message,
            None,
        )
        .await
    }

    async fn queue_mutation(pool: &sqlx::SqlitePool, status: &str, payload: serde_json::Value) {
        sqlx::query(
            "INSERT INTO sync_queue (id, account_id, action_type, payload, status, created_at, updated_at)
             VALUES ('queued', 'account', 'remote_mutation', ?, ?, '', '')",
        )
        .bind(payload.to_string())
        .bind(status)
        .execute(pool)
        .await
        .expect("queue row");
    }

    fn flags_payload() -> serde_json::Value {
        serde_json::json!({
            "kind": "flags",
            "refs": [{"remote_id": "42", "source_folder": "INBOX"}],
            "read": true,
            "starred": null,
        })
    }

    async fn read_state_after_provider_says_unread(pool: &sqlx::SqlitePool) -> i64 {
        let mut message = envelope("42");
        message.date_rfc3339 = "2026-01-01T00:00:00Z".to_string();
        message.from_address = "sender@example.com".to_string();
        message.is_read = false;
        ingest(pool, "inbox", &message)
            .await
            .expect("second ingest");
        sqlx::query_scalar("SELECT is_read FROM messages")
            .fetch_one(pool)
            .await
            .expect("is_read")
    }

    /// The user marked the thread read; the mutation is queued and has not
    /// reached the server. The next provider batch must not undo it.
    #[tokio::test]
    async fn an_open_flag_mutation_protects_the_local_read_state() {
        for status in ["pending", "sending", "scheduled", "failed"] {
            let pool = ingest_pool().await;
            let mut message = envelope("42");
            message.date_rfc3339 = "2026-01-01T00:00:00Z".to_string();
            message.from_address = "sender@example.com".to_string();
            message.is_read = true;
            ingest(&pool, "inbox", &message)
                .await
                .expect("first ingest");
            queue_mutation(&pool, status, flags_payload()).await;

            assert_eq!(
                read_state_after_provider_says_unread(&pool).await,
                1,
                "status {status}"
            );
        }
    }

    #[tokio::test]
    async fn without_a_queue_row_the_provider_wins() {
        let pool = ingest_pool().await;
        let mut message = envelope("42");
        message.date_rfc3339 = "2026-01-01T00:00:00Z".to_string();
        message.from_address = "sender@example.com".to_string();
        message.is_read = true;
        ingest(&pool, "inbox", &message)
            .await
            .expect("first ingest");

        assert_eq!(read_state_after_provider_says_unread(&pool).await, 0);
    }

    #[tokio::test]
    async fn a_completed_queue_row_no_longer_protects() {
        let pool = ingest_pool().await;
        let mut message = envelope("42");
        message.date_rfc3339 = "2026-01-01T00:00:00Z".to_string();
        message.from_address = "sender@example.com".to_string();
        message.is_read = true;
        ingest(&pool, "inbox", &message)
            .await
            .expect("first ingest");
        queue_mutation(&pool, "done", flags_payload()).await;

        assert_eq!(read_state_after_provider_says_unread(&pool).await, 0);
    }

    /// A queued move says where the message goes, not whether it was read.
    #[tokio::test]
    async fn an_open_move_does_not_protect_flags() {
        let pool = ingest_pool().await;
        let mut message = envelope("42");
        message.date_rfc3339 = "2026-01-01T00:00:00Z".to_string();
        message.from_address = "sender@example.com".to_string();
        message.is_read = true;
        ingest(&pool, "inbox", &message)
            .await
            .expect("first ingest");
        queue_mutation(
            &pool,
            "pending",
            serde_json::json!({
                "kind": "move",
                "refs": [{"remote_id": "42", "source_folder": "INBOX"}],
                "target_remote_folder_id": "Archive",
            }),
        )
        .await;

        assert_eq!(read_state_after_provider_says_unread(&pool).await, 0);
    }

    /// A provider that hands over an envelope without its parts is not saying
    /// the message lost its attachments.
    #[tokio::test]
    async fn an_envelope_without_attachments_keeps_the_stored_rows() {
        let pool = ingest_pool().await;
        let mut message = envelope("42");
        message.date_rfc3339 = "2026-01-01T00:00:00Z".to_string();
        message.from_address = "sender@example.com".to_string();
        message.attachments = vec![
            RemoteAttachment {
                remote_part_id: "2".into(),
                filename: Some("re\u{202e}port.pdf".into()),
                content_type: "application/pdf".into(),
                size: Some(12),
                content_id: None,
                is_inline: false,
                content_base64: None,
            },
            RemoteAttachment {
                remote_part_id: "3".into(),
                filename: Some("sheet.csv".into()),
                content_type: "text/csv".into(),
                size: Some(34),
                content_id: None,
                is_inline: false,
                content_base64: None,
            },
        ];
        ingest(&pool, "inbox", &message)
            .await
            .expect("first ingest");
        let stored_name: String =
            sqlx::query_scalar("SELECT filename FROM attachments WHERE remote_part_id = '2'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(stored_name, "report.pdf");
        // Sync lists by today's rules, so a root signature never hides these.
        let rules: Vec<i64> = sqlx::query_scalar("SELECT listing_rules FROM attachments")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(
            rules,
            [satchel_core::db::signed_attachments::LISTING_RULES; 2]
        );
        sqlx::query("UPDATE attachments SET cached_path = '/cache/' || id")
            .execute(&pool)
            .await
            .expect("cache attachments");

        // The same message again, this time as an envelope that names no parts.
        message.attachments.clear();
        ingest(&pool, "inbox", &message)
            .await
            .expect("re-ingest without attachments");

        let cached: Vec<(String, Option<String>)> =
            sqlx::query_as("SELECT remote_part_id, cached_path FROM attachments ORDER BY id")
                .fetch_all(&pool)
                .await
                .expect("attachments");
        assert_eq!(cached.len(), 2);
        assert!(cached.iter().all(|(_, cached_path)| cached_path.is_some()));
        let message_has_attachments: i64 =
            sqlx::query_scalar("SELECT has_attachments FROM messages")
                .fetch_one(&pool)
                .await
                .expect("message flag");
        assert_eq!(message_has_attachments, 1);
        let thread_has_attachments: i64 = sqlx::query_scalar("SELECT has_attachments FROM threads")
            .fetch_one(&pool)
            .await
            .expect("thread flag");
        assert_eq!(thread_has_attachments, 1);
    }

    /// Rows for parts of decrypted content carry no remote part, so the
    /// envelope's part list never names them. Dropping them on every resync
    /// left the message without its decrypted images until it was opened
    /// again.
    #[tokio::test]
    async fn a_resync_keeps_the_parts_of_decrypted_content() {
        let pool = ingest_pool().await;
        let mut message = envelope("43");
        message.date_rfc3339 = "2026-01-01T00:00:00Z".to_string();
        message.attachments = vec![RemoteAttachment {
            remote_part_id: "2".into(),
            filename: Some("encrypted.asc".into()),
            content_type: "application/octet-stream".into(),
            size: Some(12),
            content_id: None,
            is_inline: false,
            content_base64: None,
        }];
        ingest(&pool, "inbox", &message)
            .await
            .expect("first ingest");
        let message_id: String = sqlx::query_scalar("SELECT id FROM messages")
            .fetch_one(&pool)
            .await
            .unwrap();
        let decrypted = satchel_core::email::decrypted::attachment_id(&message_id, "mime-2");
        sqlx::query(
            "INSERT INTO attachments
             (id, message_id, filename, content_type, is_inline, cached_path, remote_part_id,
              created_at)
             VALUES (?, ?, 'logo.png', 'image/png', 1, '/cache/logo.png', NULL, '')",
        )
        .bind(&decrypted)
        .bind(&message_id)
        .execute(&pool)
        .await
        .expect("decrypted part");

        ingest(&pool, "inbox", &message).await.expect("resync");

        let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM attachments ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(ids.len(), 2, "{ids:?}");
        assert!(ids.contains(&decrypted), "{ids:?}");
    }

    /// A `Date` far in the future used to win the `last_message_date >= …`
    /// guard of the thread projection forever, so no later message could ever
    /// update the thread's subject or snippet again.
    #[tokio::test]
    async fn a_future_dated_message_does_not_freeze_its_thread() {
        let pool = ingest_pool().await;
        let now = chrono::Utc::now();

        let mut spam = envelope("1");
        spam.remote_thread_id = Some("conversation".into());
        spam.from_address = "spam@example.com".to_string();
        spam.subject = "Gewinnbenachrichtigung".to_string();
        spam.snippet = "Sie haben gewonnen".to_string();
        spam.date_rfc3339 = crate::providers::imap::parse_message_date(
            "Date: Fri, 1 Jan 2038 00:00:00 +0000\r\n",
            None,
            now,
        );
        assert_eq!(spam.date_rfc3339, now.to_rfc3339());
        ingest(&pool, "inbox", &spam).await.expect("spam");

        let mut later = envelope("2");
        later.remote_thread_id = Some("conversation".into());
        later.from_address = "kollege@example.com".to_string();
        later.subject = "Protokoll der Sitzung".to_string();
        later.snippet = "Anbei das Protokoll".to_string();
        later.date_rfc3339 = (now + chrono::TimeDelta::minutes(1)).to_rfc3339();
        ingest(&pool, "inbox", &later).await.expect("later");

        let thread: (String, String) =
            sqlx::query_as("SELECT subject, COALESCE(snippet, '') FROM threads")
                .fetch_one(&pool)
                .await
                .expect("thread");
        assert_eq!(
            thread,
            (
                "Protokoll der Sitzung".to_string(),
                "Anbei das Protokoll".to_string()
            )
        );
    }

    #[tokio::test]
    async fn a_failing_message_insert_rolls_back_the_thread_upsert() {
        let pool = ingest_pool().await;
        let mut message = envelope("42");
        message.date_rfc3339 = "2026-01-01T00:00:00Z".to_string();

        // The folder does not exist, so the message insert trips its foreign
        // key after the thread row has already been written.
        let error = ingest(&pool, "missing-folder", &message)
            .await
            .expect_err("foreign key violation");
        assert!(error.to_lowercase().contains("foreign key"), "{error}");

        let threads: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM threads")
            .fetch_one(&pool)
            .await
            .expect("threads");
        assert_eq!(threads, 0);
        let messages: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages")
            .fetch_one(&pool)
            .await
            .expect("messages");
        assert_eq!(messages, 0);
    }

    /// The whole reply-threading story in one run: a reply that arrives before
    /// its parent splits off, the merge stitches the threads (moving topic
    /// links along), and later replies join the thread directly at upsert.
    #[tokio::test]
    async fn imap_replies_join_and_merge_into_one_thread() {
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
             VALUES ('account', 'Me', 'me@example.com', 'imap', '{}', '', ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("account");
        sqlx::query(
            "INSERT INTO folders
             (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at)
             VALUES ('inbox', 'account', 'INBOX', 'INBOX', 'inbox', 'INBOX', ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("folder");

        let upsert = |message: crate::providers::traits::MessageEnvelope,
                      pool: sqlx::SqlitePool| async move {
            upsert_provider_message(
                None,
                &pool,
                "imap",
                "account",
                "me@example.com",
                "Me",
                &crate::commands::accounts::AccountConfig::from_plaintext("{}"),
                "inbox",
                "INBOX",
                "inbox",
                &message,
                None,
            )
            .await
            .expect("upsert");
        };

        // The reply syncs first — its parent is unknown, so it starts alone.
        let mut reply = envelope("uid-2");
        reply.message_id_header = Some("<reply@example.com>".to_string());
        reply.in_reply_to = Some("<start@example.com>".to_string());
        reply.date_rfc3339 = now.clone();
        reply.pgp_signed = Some(b"signed MIME".to_vec());
        upsert(reply, pool.clone()).await;

        let stored_pgp_signed: Option<Vec<u8>> =
            sqlx::query_scalar("SELECT pgp_signed FROM messages WHERE remote_id = 'uid-2'")
                .fetch_one(&pool)
                .await
                .expect("stored PGP signed MIME");
        assert_eq!(
            stored_pgp_signed.as_deref(),
            Some(b"signed MIME".as_slice())
        );

        let mut parent = envelope("uid-1");
        parent.message_id_header = Some("<start@example.com>".to_string());
        parent.date_rfc3339 = now.clone();
        upsert(parent, pool.clone()).await;

        let split: i64 = sqlx::query_scalar("SELECT COUNT(DISTINCT thread_id) FROM messages")
            .fetch_one(&pool)
            .await
            .expect("split count");
        assert_eq!(split, 2, "reply-before-parent must split first");

        // A topic pinned to the reply's thread must survive the merge.
        let reply_thread: String = sqlx::query_scalar(
            "SELECT thread_id FROM messages WHERE message_id_header = '<reply@example.com>'",
        )
        .fetch_one(&pool)
        .await
        .expect("reply thread");
        sqlx::query(
            "INSERT INTO topics (id, name, created_at, updated_at) VALUES ('topic-1', 'T', ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("topic");
        sqlx::query(
            "INSERT INTO topic_items (topic_id, item_type, item_id, added_at) VALUES ('topic-1', 'thread', ?, ?)",
        )
        .bind(&reply_thread)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("topic link");

        let moved = merge_reply_split_threads(&pool).await.expect("merge");
        assert!(moved >= 1);
        let merged: i64 = sqlx::query_scalar("SELECT COUNT(DISTINCT thread_id) FROM messages")
            .fetch_one(&pool)
            .await
            .expect("merged count");
        assert_eq!(merged, 1, "merge must leave one conversation");

        let surviving: String = sqlx::query_scalar(
            "SELECT thread_id FROM messages WHERE message_id_header = '<start@example.com>'",
        )
        .fetch_one(&pool)
        .await
        .expect("surviving thread");
        let linked: String =
            sqlx::query_scalar("SELECT item_id FROM topic_items WHERE topic_id = 'topic-1'")
                .fetch_one(&pool)
                .await
                .expect("moved link");
        assert_eq!(linked, surviving, "topic link must follow the merge");

        // With both sides present, a later reply joins directly at upsert.
        let mut reply2 = envelope("uid-3");
        reply2.message_id_header = Some("<reply2@example.com>".to_string());
        reply2.in_reply_to = Some("<reply@example.com>".to_string());
        reply2.date_rfc3339 = now.clone();
        upsert(reply2, pool.clone()).await;
        let still_one: i64 = sqlx::query_scalar("SELECT COUNT(DISTINCT thread_id) FROM messages")
            .fetch_one(&pool)
            .await
            .expect("direct join count");
        assert_eq!(still_one, 1, "a new reply must join the existing thread");
    }

    async fn subject_fallback_pool() -> sqlx::SqlitePool {
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
             VALUES ('inbox', 'account', 'INBOX', 'INBOX', 'inbox', 'INBOX', '2026-01-01', '2026-01-01')",
        ] {
            sqlx::query(statement).execute(&pool).await.expect("seed");
        }
        pool
    }

    async fn distinct_threads(pool: &sqlx::SqlitePool) -> i64 {
        sqlx::query_scalar("SELECT COUNT(DISTINCT thread_id) FROM messages")
            .fetch_one(pool)
            .await
            .expect("thread count")
    }

    /// Four characters were enough to join "Info", "Test" and "Termin" across
    /// unrelated correspondents.
    #[tokio::test]
    async fn imap_subject_fallback_keeps_generic_subjects_apart() {
        let pool = subject_fallback_pool().await;
        for (index, sender) in ["anna@example.com", "bernd@example.com"]
            .into_iter()
            .enumerate()
        {
            let mut message = envelope(&format!("{index}"));
            message.subject = "Termin".to_string();
            message.from_address = sender.to_string();
            message.to_addresses = vec!["me@example.com".to_string()];
            message.cc_addresses = vec!["liste@example.com".to_string()];
            message.date_rfc3339 = "2026-03-02T09:00:00Z".to_string();
            ingest(&pool, "inbox", &message).await.expect("ingest");
        }

        assert_eq!(distinct_threads(&pool).await, 2);
    }

    /// Twelve monthly invoices carry the same subject and the same sender;
    /// they are twelve conversations, not one.
    #[tokio::test]
    async fn imap_subject_fallback_ignores_candidates_outside_the_window() {
        let pool = subject_fallback_pool().await;
        for (index, date) in ["2026-01-02T09:00:00Z", "2026-04-02T09:00:00Z"]
            .into_iter()
            .enumerate()
        {
            let mut message = envelope(&format!("{index}"));
            message.subject = "Rechnung Nr. 4711".to_string();
            message.from_address = "buchhaltung@example.com".to_string();
            message.to_addresses = vec!["me@example.com".to_string()];
            message.date_rfc3339 = date.to_string();
            ingest(&pool, "inbox", &message).await.expect("ingest");
        }

        assert_eq!(distinct_threads(&pool).await, 2);
    }

    /// A shared distribution list in Cc is not a conversation.
    #[tokio::test]
    async fn imap_subject_fallback_ignores_a_shared_cc_only() {
        let pool = subject_fallback_pool().await;
        for (index, (sender, recipient)) in [
            ("anna@example.com", "carla@example.com"),
            ("bernd@example.com", "dora@example.com"),
        ]
        .into_iter()
        .enumerate()
        {
            let mut message = envelope(&format!("{index}"));
            message.subject = "Wartungsfenster am Samstag".to_string();
            message.from_address = sender.to_string();
            message.to_addresses = vec![recipient.to_string()];
            message.cc_addresses = vec!["liste@example.com".to_string()];
            message.date_rfc3339 = "2026-03-02T09:00:00Z".to_string();
            ingest(&pool, "inbox", &message).await.expect("ingest");
        }

        assert_eq!(distinct_threads(&pool).await, 2);
    }

    #[tokio::test]
    async fn imap_subject_fallback_groups_legacy_messages_by_participant() {
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
             VALUES ('account', 'Me', 'me@example.com', 'outlook', '{}', '', '2026-01-01', '2026-01-01')",
            "INSERT INTO folders
             (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at)
             VALUES ('inbox', 'account', 'INBOX', 'INBOX', 'inbox', 'INBOX', '2026-01-01', '2026-01-01')",
            "INSERT INTO folders
             (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at)
             VALUES ('sent', 'account', 'Sent', 'Sent', 'sent', 'Sent', '2026-01-01', '2026-01-01')",
        ] {
            sqlx::query(statement).execute(&pool).await.expect("seed");
        }

        let mut original = envelope("1");
        original.subject = "Messdaten Q3 2025".to_string();
        original.from_address = "tim@example.com".to_string();
        original.to_addresses = vec!["me@example.com".to_string()];
        original.date_rfc3339 = "2025-05-09T06:35:48Z".to_string();
        upsert_provider_message(
            None,
            &pool,
            "outlook",
            "account",
            "me@example.com",
            "Me",
            &crate::commands::accounts::AccountConfig::from_plaintext("{}"),
            "inbox",
            "INBOX",
            "inbox",
            &original,
            None,
        )
        .await
        .expect("original");

        let mut reply = envelope("2");
        reply.subject = "AW: Messdaten Q3 2025".to_string();
        reply.from_address = "me@example.com".to_string();
        reply.to_addresses = vec!["tim@example.com".to_string()];
        reply.date_rfc3339 = "2025-05-12T10:57:47Z".to_string();
        upsert_provider_message(
            None,
            &pool,
            "imap",
            "account",
            "me@example.com",
            "Me",
            &crate::commands::accounts::AccountConfig::from_plaintext("{}"),
            "sent",
            "Sent",
            "sent",
            &reply,
            None,
        )
        .await
        .expect("reply");

        let thread_count: i64 =
            sqlx::query_scalar("SELECT COUNT(DISTINCT thread_id) FROM messages")
                .fetch_one(&pool)
                .await
                .expect("thread count");
        assert_eq!(thread_count, 1);

        let mut independent = envelope("3");
        independent.message_id_header = Some("<new-root@example.com>".to_string());
        independent.subject = "Messdaten Q3 2025".to_string();
        independent.from_address = "tim@example.com".to_string();
        independent.to_addresses = vec!["me@example.com".to_string()];
        independent.date_rfc3339 = "2026-08-04T12:00:00Z".to_string();
        upsert_provider_message(
            None,
            &pool,
            "imap",
            "account",
            "me@example.com",
            "Me",
            &crate::commands::accounts::AccountConfig::from_plaintext("{}"),
            "inbox",
            "INBOX",
            "inbox",
            &independent,
            None,
        )
        .await
        .expect("independent root");
        let thread_count: i64 =
            sqlx::query_scalar("SELECT COUNT(DISTINCT thread_id) FROM messages")
                .fetch_one(&pool)
                .await
                .expect("thread count after independent root");
        assert_eq!(
            thread_count, 2,
            "a real root Message-ID starts a new thread"
        );
    }

    #[tokio::test]
    async fn spark_reply_autosaves_keep_only_the_newest_draft_in_the_parent_thread() {
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
             VALUES ('inbox', 'account', 'INBOX', 'INBOX', 'inbox', 'INBOX', '2026-01-01', '2026-01-01')",
            "INSERT INTO folders
             (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at)
             VALUES ('drafts', 'account', 'Drafts', 'Drafts', 'drafts', 'Drafts', '2026-01-01', '2026-01-01')",
            "INSERT INTO folders
             (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at)
             VALUES ('sent', 'account', 'Sent', 'Sent', 'sent', 'Sent', '2026-01-01', '2026-01-01')",
        ] {
            sqlx::query(statement).execute(&pool).await.expect("seed");
        }

        let mut parent = envelope("1");
        parent.message_id_header = Some("<parent@example.com>".to_string());
        parent.subject = "Messdaten".to_string();
        parent.from_address = "tim@example.com".to_string();
        parent.to_addresses = vec!["me@example.com".to_string()];
        parent.date_rfc3339 = "2026-08-04T18:00:00Z".to_string();
        upsert_provider_message(
            None,
            &pool,
            "imap",
            "account",
            "me@example.com",
            "Me",
            &crate::commands::accounts::AccountConfig::from_plaintext("{}"),
            "inbox",
            "INBOX",
            "inbox",
            &parent,
            None,
        )
        .await
        .expect("parent");

        for (remote_id, header, date, body) in [
            ("10", "<first@Spark>", "2026-08-04T19:00:00Z", "first"),
            ("11", "<second@Spark>", "2026-08-04T19:10:00Z", "latest"),
        ] {
            let mut draft = envelope(remote_id);
            draft.message_id_header = Some(header.to_string());
            draft.in_reply_to = Some("<parent@example.com>".to_string());
            draft.subject = "Re: Messdaten".to_string();
            draft.from_address = "me@example.com".to_string();
            draft.to_addresses = vec!["tim@example.com".to_string()];
            draft.date_rfc3339 = date.to_string();
            draft.body_text = Some(body.to_string());
            upsert_provider_message(
                None,
                &pool,
                "imap",
                "account",
                "me@example.com",
                "Me",
                &crate::commands::accounts::AccountConfig::from_plaintext("{}"),
                "drafts",
                "Drafts",
                "drafts",
                &draft,
                None,
            )
            .await
            .expect("draft");
        }

        let drafts: Vec<(String, String)> =
            sqlx::query_as("SELECT body_text, thread_id FROM messages WHERE is_draft = 1")
                .fetch_all(&pool)
                .await
                .expect("draft rows");
        assert_eq!(drafts.len(), 1);
        assert_eq!(drafts[0].0, "latest");
        let parent_thread: String = sqlx::query_scalar(
            "SELECT thread_id FROM messages WHERE message_id_header = '<parent@example.com>'",
        )
        .fetch_one(&pool)
        .await
        .expect("parent thread");
        assert_eq!(drafts[0].1, parent_thread);

        let mut sent = envelope("12");
        sent.message_id_header = Some("<sent@Spark>".to_string());
        sent.in_reply_to = Some("<parent@example.com>".to_string());
        sent.subject = "Re: Messdaten".to_string();
        sent.from_address = "me@example.com".to_string();
        sent.to_addresses = vec!["tim@example.com".to_string()];
        sent.date_rfc3339 = "2026-08-04T19:10:30Z".to_string();
        sent.body_text = Some("latest".to_string());
        upsert_provider_message(
            None,
            &pool,
            "imap",
            "account",
            "me@example.com",
            "Me",
            &crate::commands::accounts::AccountConfig::from_plaintext("{}"),
            "sent",
            "Sent",
            "sent",
            &sent,
            None,
        )
        .await
        .expect("sent copy");

        let remaining_drafts: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE is_draft = 1")
                .fetch_one(&pool)
                .await
                .expect("remaining drafts");
        assert_eq!(
            remaining_drafts, 0,
            "the sent copy supersedes its Spark autosave"
        );
    }

    /// IMAP identifies messages per folder: the same numeric id means different
    /// messages in different folders. An unscoped standalone key would merge
    /// unrelated mail into one thread.
    #[test]
    fn imap_scopes_a_standalone_thread_key_by_folder() {
        let mut message = envelope("1234");
        message.remote_uidvalidity = Some(77);
        assert_ne!(
            standalone_thread_key("imap", "INBOX", &message),
            standalone_thread_key("imap", "Archive", &message)
        );
        assert!(
            provider_message_db_id("imap", "account", "INBOX", &message).starts_with("m-imap-v2-")
        );
        // Account-wide ids need no scoping.
        assert_eq!(standalone_thread_key("gmail", "INBOX", &message), "1234");

        // The provider refuses a mailbox without a UIDVALIDITY, so the
        // generation slot is always filled — never dropped, which would let a
        // recycled UID alias onto the previous generation's key.
        let other_generation = envelope("1234");
        assert_ne!(
            standalone_thread_key("imap", "INBOX", &other_generation),
            standalone_thread_key("imap", "INBOX", &message)
        );
    }

    async fn ingest_identity_fixture(
        pool: &sqlx::SqlitePool,
        provider: &str,
        folder_id: &str,
        folder_remote_id: &str,
        message: &MessageEnvelope,
    ) {
        upsert_provider_message(
            None,
            pool,
            provider,
            "account",
            "account@example.com",
            "Account",
            &crate::commands::accounts::AccountConfig::from_plaintext("{}"),
            folder_id,
            folder_remote_id,
            "inbox",
            message,
            None,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn punctuation_distinguishes_folders_threads_and_attachment_parts() {
        let pool = ingest_pool().await;
        for (folder, remote) in [
            ("project-slash", "Projects/2026"),
            ("project-dash", "Projects-2026"),
        ] {
            sqlx::query("INSERT INTO folders (id, account_id, name, full_path, remote_id, folder_type, created_at, updated_at) VALUES (?, 'account', ?, ?, ?, 'custom', '', '')")
                .bind(folder).bind(remote).bind(remote).bind(remote).execute(&pool).await.unwrap();
        }
        let mut message = envelope("42");
        message.remote_uidvalidity = Some(7);
        message.attachments = ["part.1", "part-1"]
            .into_iter()
            .map(|part| RemoteAttachment {
                remote_part_id: part.to_string(),
                filename: Some(format!("{part}.txt")),
                content_type: "text/plain".to_string(),
                size: Some(1),
                content_id: None,
                is_inline: false,
                content_base64: None,
            })
            .collect();
        for (folder, remote) in [
            ("project-slash", "Projects/2026"),
            ("project-dash", "Projects-2026"),
        ] {
            ingest_identity_fixture(&pool, "imap", folder, remote, &message).await;
        }
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM messages")
                .fetch_one(&pool)
                .await
                .unwrap(),
            2
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM threads")
                .fetch_one(&pool)
                .await
                .unwrap(),
            2
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM attachments")
                .fetch_one(&pool)
                .await
                .unwrap(),
            4
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(DISTINCT message_id_header) FROM messages")
                .fetch_one(&pool)
                .await
                .unwrap(),
            2
        );

        let pool = ingest_pool().await;
        for (remote, header) in [("42", "<a.b@example.com>"), ("43", "<a-b@example.com>")] {
            let mut message = envelope(remote);
            message.message_id_header = Some(header.to_string());
            ingest_identity_fixture(&pool, "imap", "inbox", "INBOX", &message).await;
        }
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM threads")
                .fetch_one(&pool)
                .await
                .unwrap(),
            2
        );
    }

    #[tokio::test]
    async fn verifying_a_new_generation_discards_old_bodies_and_attachment_caches() {
        let pool = ingest_pool().await;
        let mut message = envelope("42");
        message.remote_uidvalidity = Some(7);
        message.body_text = Some("old body".into());
        message.attachments.push(RemoteAttachment {
            remote_part_id: "part.1".into(),
            filename: Some("file.bin".into()),
            content_type: "application/octet-stream".into(),
            size: Some(3),
            content_id: None,
            is_inline: false,
            content_base64: Some("b2xk".into()),
        });
        ingest_identity_fixture(&pool, "imap", "inbox", "INBOX", &message).await;
        let original_id: String = sqlx::query_scalar("SELECT id FROM messages")
            .fetch_one(&pool)
            .await
            .unwrap();
        sqlx::query("UPDATE attachments SET cached_path = '/cache/old', calendar_scan_at = 'old generation'")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("UPDATE messages SET raw_headers = 'old headers', raw_size = 99")
            .execute(&pool)
            .await
            .unwrap();
        message.remote_uidvalidity = Some(8);
        message.body_text = None;
        message.body_html = None;
        message.original_source = None;
        message.attachments.clear();
        ingest_identity_fixture(&pool, "imap", "inbox", "INBOX", &message).await;
        let (id, generation, body): (String, i64, Option<String>) =
            sqlx::query_as("SELECT id, remote_uidvalidity, body_text FROM messages")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            id, original_id,
            "retaining a local id must still invalidate its content"
        );
        assert_eq!(generation, 8);
        assert_eq!(body, None);
        let source: (Option<String>, Option<i64>) =
            sqlx::query_as("SELECT raw_headers, raw_size FROM messages")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(source, (None, None));
        let (path, payload, scan): (Option<String>, Option<String>, Option<String>) =
            sqlx::query_as("SELECT cached_path, content_base64, calendar_scan_at FROM attachments")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(path, None);
        assert_eq!(payload, None);
        assert_eq!(scan, None);
    }

    #[tokio::test]
    async fn legacy_ids_memberships_and_attachment_cache_survive_resync_and_folder_copies() {
        let pool = ingest_pool().await;
        sqlx::query("INSERT INTO threads (id, account_id, subject, last_message_date, workflow_state, board_column, category, category_source, priority, priority_source, created_at, updated_at) VALUES ('th-imap-legacy', 'account', 'Legacy', '', 'todo', 'todo', 'personal', 'manual_thread', 3, 'manual_thread', '', '')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO messages (id, account_id, folder_id, thread_id, message_id_header, remote_id, from_address, to_addresses, date, created_at, updated_at) VALUES ('m-imap-legacy', 'account', 'inbox', 'th-imap-legacy', '<legacy@example.com>', '42', '', '[]', '', '', '')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO attachments (id, message_id, remote_part_id, remote_uid, content_type, cached_path, created_at) VALUES ('att-legacy', 'm-imap-legacy', 'part.1', '42', 'text/plain', '/legacy-cache', '')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO tags (id, name) VALUES ('tag', 'Tag')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO thread_tags (thread_id, tag_id) VALUES ('th-imap-legacy', 'tag')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO topics (id, name, created_at, updated_at) VALUES ('topic', 'Topic', '', '')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO topic_items (topic_id, item_type, item_id, added_at) VALUES ('topic', 'thread', 'th-imap-legacy', '')").execute(&pool).await.unwrap();
        let mut message = envelope("42");
        message.message_id_header = Some("<legacy@example.com>".to_string());
        message.attachments.push(RemoteAttachment {
            remote_part_id: "part.1".into(),
            filename: None,
            content_type: "text/plain".into(),
            size: Some(1),
            content_id: None,
            is_inline: false,
            content_base64: None,
        });
        ingest_identity_fixture(&pool, "imap", "inbox", "INBOX", &message).await;
        assert_eq!(
            sqlx::query_as::<_, (String, String)>("SELECT id, thread_id FROM messages")
                .fetch_one(&pool)
                .await
                .unwrap(),
            ("m-imap-legacy".into(), "th-imap-legacy".into())
        );
        assert_eq!(
            sqlx::query_as::<_, (String, String)>("SELECT id, cached_path FROM attachments")
                .fetch_one(&pool)
                .await
                .unwrap(),
            ("att-legacy".into(), "/legacy-cache".into())
        );
        let state = sqlx::query_as::<_, (String, i64, String)>(
            "SELECT workflow_state, priority, category FROM threads",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(state, ("todo".into(), 3, "personal".into()));
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM thread_tags WHERE thread_id = 'th-imap-legacy'"
            )
            .fetch_one(&pool)
            .await
            .unwrap(),
            1
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM topic_items WHERE item_id = 'th-imap-legacy'"
            )
            .fetch_one(&pool)
            .await
            .unwrap(),
            1
        );
        sqlx::query("INSERT INTO folders (id, account_id, name, full_path, remote_id, folder_type, created_at, updated_at) VALUES ('archive', 'account', 'Archive', 'Archive', 'Archive', 'archive', '', '')").execute(&pool).await.unwrap();
        message.remote_id = "99".into();
        ingest_identity_fixture(&pool, "imap", "archive", "Archive", &message).await;
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM messages WHERE thread_id = 'th-imap-legacy'"
            )
            .fetch_one(&pool)
            .await
            .unwrap(),
            2
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM threads")
                .fetch_one(&pool)
                .await
                .unwrap(),
            1
        );
    }

    #[tokio::test]
    async fn a_legacy_synthetic_header_remains_a_valid_local_reply_reference() {
        let pool = ingest_pool().await;
        sqlx::query("INSERT INTO threads (id, account_id, last_message_date, created_at, updated_at) VALUES ('th-imap-legacy', 'account', '', '', '')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO messages (id, account_id, folder_id, thread_id, message_id_header, remote_id, from_address, to_addresses, date, created_at, updated_at) VALUES ('m-imap-legacy', 'account', 'inbox', 'th-imap-legacy', '<INBOX-42@imap.flowmail.local>', '42', '', '[]', '', '', '')").execute(&pool).await.unwrap();
        ingest_identity_fixture(&pool, "imap", "inbox", "INBOX", &envelope("42")).await;
        assert_eq!(
            sqlx::query_scalar::<_, String>("SELECT message_id_header FROM messages")
                .fetch_one(&pool)
                .await
                .unwrap(),
            "<INBOX-42@imap.flowmail.local>"
        );
    }

    #[tokio::test]
    async fn first_new_conversation_member_is_folded_into_the_later_resynced_legacy_thread() {
        let pool = ingest_pool().await;
        sqlx::query("INSERT INTO threads (id, account_id, subject, last_message_date, workflow_state, board_column, created_at, updated_at) VALUES ('th-gmail-legacy', 'account', 'Legacy', '', 'todo', 'todo', '', '')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO messages (id, account_id, folder_id, thread_id, message_id_header, remote_id, from_address, to_addresses, date, created_at, updated_at) VALUES ('m-gmail-legacy', 'account', 'inbox', 'th-gmail-legacy', '<legacy@example.com>', '41', '', '[]', '', '', '')").execute(&pool).await.unwrap();
        let mut message = envelope("42");
        message.remote_thread_id = Some("conversation/raw-key".into());
        message.message_id_header = Some("<new@example.com>".into());
        ingest_identity_fixture(&pool, "gmail", "inbox", "INBOX", &message).await;
        sqlx::query("INSERT INTO tags (id, name) VALUES ('new-tag', 'New tag')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO thread_tags (thread_id, tag_id) SELECT thread_id, 'new-tag' FROM messages WHERE remote_id = '42'").execute(&pool).await.unwrap();
        message.remote_id = "41".into();
        message.message_id_header = Some("<legacy@example.com>".into());
        ingest_identity_fixture(&pool, "gmail", "inbox", "INBOX", &message).await;
        message.remote_id = "43".into();
        message.message_id_header = Some("<newer@example.com>".into());
        ingest_identity_fixture(&pool, "gmail", "inbox", "INBOX", &message).await;
        assert_eq!(sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM thread_tags WHERE thread_id = 'th-gmail-legacy' AND tag_id = 'new-tag'").fetch_one(&pool).await.unwrap(), 1);
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM threads")
                .fetch_one(&pool)
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM messages WHERE thread_id = 'th-gmail-legacy'"
            )
            .fetch_one(&pool)
            .await
            .unwrap(),
            3
        );
        assert_eq!(
            sqlx::query_scalar::<_, String>("SELECT workflow_state FROM threads")
                .fetch_one(&pool)
                .await
                .unwrap(),
            "todo"
        );
    }

    #[tokio::test]
    async fn provider_thread_alias_survives_anchor_deletion_until_the_last_member_is_removed() {
        let pool = ingest_pool().await;
        sqlx::query("INSERT INTO threads (id, account_id, subject, last_message_date, workflow_state, board_column, created_at, updated_at) VALUES ('th-gmail-legacy', 'account', 'Legacy', '', 'todo', 'todo', '', '')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO messages (id, account_id, folder_id, thread_id, message_id_header, remote_id, from_address, to_addresses, date, created_at, updated_at) VALUES ('m-gmail-legacy', 'account', 'inbox', 'th-gmail-legacy', '<legacy@example.com>', '41', '', '[]', '', '', '')").execute(&pool).await.unwrap();
        let mut message = envelope("41");
        message.remote_thread_id = Some("conversation/raw-key".into());
        message.message_id_header = Some("<legacy@example.com>".into());
        ingest_identity_fixture(&pool, "gmail", "inbox", "INBOX", &message).await;
        message.remote_id = "42".into();
        message.message_id_header = Some("<anchor@example.com>".into());
        ingest_identity_fixture(&pool, "gmail", "inbox", "INBOX", &message).await;
        let identity = crate::sync::thread_resolution::provider_thread_db_id(
            "gmail", "account", "INBOX", &message,
        );
        let anchor: String =
            sqlx::query_scalar("SELECT message_id FROM provider_thread_keys WHERE identity = ?")
                .bind(&identity)
                .fetch_one(&pool)
                .await
                .unwrap();
        let expected_anchor = provider_message_db_id("gmail", "account", "INBOX", &message);
        assert_eq!(anchor, expected_anchor);
        sqlx::query("DELETE FROM messages WHERE id = ?")
            .bind(&anchor)
            .execute(&pool)
            .await
            .unwrap();
        let replacement: String =
            sqlx::query_scalar("SELECT message_id FROM provider_thread_keys WHERE identity = ?")
                .bind(&identity)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(replacement, "m-gmail-legacy");

        // Neither the provider message ID nor its Message-ID has been seen.
        // Only the surviving conversation alias can recover the legacy thread.
        message.remote_id = "43".into();
        message.message_id_header = Some("<new-member@example.com>".into());
        ingest_identity_fixture(&pool, "gmail", "inbox", "INBOX", &message).await;
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM messages WHERE thread_id = 'th-gmail-legacy'"
            )
            .fetch_one(&pool)
            .await
            .unwrap(),
            2
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM threads")
                .fetch_one(&pool)
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            sqlx::query_scalar::<_, String>("SELECT workflow_state FROM threads")
                .fetch_one(&pool)
                .await
                .unwrap(),
            "todo"
        );

        // A bulk delete may reanchor repeatedly within the statement. Its last
        // member must still cascade the alias instead of leaving a dangling key.
        sqlx::query("DELETE FROM messages WHERE thread_id = 'th-gmail-legacy'")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM provider_thread_keys WHERE identity = ?"
            )
            .bind(&identity)
            .fetch_one(&pool)
            .await
            .unwrap(),
            0
        );
    }

    #[test]
    fn provider_identity_keeps_uidvalidity_and_all_components() {
        let mut message = envelope("42");
        message.remote_uidvalidity = Some(9);
        let id = provider_message_db_id("imap", "account@example.com", "INBOX", &message);
        assert!(id.starts_with("m-imap-v2-"));
        message.remote_uidvalidity = Some(10);
        assert_ne!(
            id,
            provider_message_db_id("imap", "account@example.com", "INBOX", &message)
        );
        assert_ne!(
            provider_message_db_id("imap", "account@example.com", "Projects/2026", &message),
            provider_message_db_id("imap", "account@example.com", "Projects-2026", &message),
        );
    }

    #[test]
    fn gmail_history_messages_are_only_stored_in_current_labels() {
        let labels = vec!["SENT".to_string(), "CATEGORY_PERSONAL".to_string()];

        assert!(gmail_labels_include(&labels, "SENT"));
        assert!(gmail_labels_include(&labels, "category_personal"));
        assert!(!gmail_labels_include(&labels, "INBOX"));
        assert!(!provider_message_is_draft("sent", &labels));
        assert!(provider_message_is_draft("drafts", &[]));
        assert!(provider_message_is_draft("custom", &["DRAFT".to_string()]));
    }

    #[tokio::test]
    async fn gmail_provider_labels_drive_category_and_priority() {
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
             (id, display_name, email, provider, config, credentials_ref, is_active, created_at, updated_at)
             VALUES ('acc', 'Me', 'me@example.com', 'gmail', '{}', '', 1, ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("account");
        sqlx::query(
            "INSERT INTO folders
             (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at)
             VALUES ('inbox', 'acc', 'INBOX', 'INBOX', 'inbox', 'INBOX', ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("folder");

        let mut message = envelope("amazon-message");
        message.remote_thread_id = Some("amazon-thread".to_string());
        message.mailbox_ids = vec![
            "INBOX".to_string(),
            "CATEGORY_UPDATES".to_string(),
            "IMPORTANT".to_string(),
        ];
        message.subject = "Unterwegs zur Zustellung".to_string();
        message.snippet = "Dein Paket kommt heute".to_string();
        message.from_address = "shipment-tracking@amazon.de".to_string();
        message.from_name = Some("Amazon.de".to_string());
        message.to_addresses = vec!["me@example.com".to_string()];
        message.date_rfc3339 = now;
        message.list_unsubscribe = Some("<https://amazon.example/unsubscribe>".to_string());

        upsert_provider_message(
            None,
            &pool,
            "gmail",
            "acc",
            "me@example.com",
            "Me",
            &crate::commands::accounts::AccountConfig::from_plaintext("{}"),
            "inbox",
            "INBOX",
            "inbox",
            &message,
            None,
        )
        .await
        .expect("upsert");

        let thread: (String, String, i64, String) = sqlx::query_as(
            "SELECT category, category_source, priority, priority_source
             FROM threads WHERE account_id = 'acc'",
        )
        .fetch_one(&pool)
        .await
        .expect("thread classification");
        assert_eq!(
            thread,
            ("updates".into(), "provider".into(), 2, "provider".into())
        );
        let labels: String = sqlx::query_scalar(
            "SELECT provider_labels FROM messages WHERE remote_id = 'amazon-message'",
        )
        .fetch_one(&pool)
        .await
        .expect("provider labels");
        assert!(labels.contains("CATEGORY_UPDATES"));
        assert!(labels.contains("IMPORTANT"));
    }

    #[tokio::test]
    async fn malformed_sender_address_does_not_abort_ingest_or_preference_policy() {
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
             VALUES ('account', 'Me', 'me@example.com', 'imap', '{}', '', ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("account");
        sqlx::query(
            "INSERT INTO folders
             (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at)
             VALUES ('inbox', 'account', 'INBOX', 'INBOX', 'inbox', 'INBOX', ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("folder");

        let mut message = envelope("malformed-sender");
        message.from_address = "not-an-email".to_string();
        message.subject = "Status".to_string();
        message.snippet = "Sale invoice offer".to_string();
        message.date_rfc3339 = now;
        upsert_provider_message(
            None,
            &pool,
            "imap",
            "account",
            "me@example.com",
            "Me",
            &crate::commands::accounts::AccountConfig::from_plaintext("{}"),
            "inbox",
            "INBOX",
            "inbox",
            &message,
            None,
        )
        .await
        .expect("malformed sender must not abort ingest");

        let projection: (String, String) = sqlx::query_as(
            "SELECT threads.category, threads.category_source
             FROM threads JOIN messages ON messages.thread_id = threads.id
             WHERE messages.remote_id = 'malformed-sender'",
        )
        .fetch_one(&pool)
        .await
        .expect("thread projection");
        assert_eq!(projection, ("personal".into(), "heuristic".into()));
    }

    #[tokio::test]
    async fn gmail_archive_move_and_upsert_reuse_the_same_local_row() {
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
             VALUES ('inbox', 'account', 'Inbox', 'INBOX', 'inbox', 'INBOX', ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("inbox");
        sqlx::query(
            "INSERT INTO rules
             (id, name, description, is_active, priority, trigger_type, trigger_config, actions,
              last_triggered, trigger_count, created_at, updated_at)
             VALUES ('rule', 'Priority', NULL, 1, 0, 'on_receive',
                     '{\"subject_contains\":\"subject\"}', '[\"set_priority:3\"]',
                     NULL, 0, ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("rule");
        let mut message = MessageEnvelope {
            remote_id: "gmail-message".to_string(),
            remote_uidvalidity: None,
            remote_thread_id: Some("gmail-thread".to_string()),
            original_source: None,
            mailbox_ids: vec!["INBOX".to_string()],
            message_id_header: Some("<gmail@example.com>".to_string()),
            in_reply_to: None,
            references: Vec::new(),
            subject: "Subject".to_string(),
            snippet: "Body".to_string(),
            from_address: "sender@example.com".to_string(),
            from_name: Some("Sender".to_string()),
            to_addresses: vec!["me@example.com".to_string()],
            cc_addresses: Vec::new(),
            bcc_addresses: Vec::new(),
            reply_to: None,
            remote_draft_id: None,
            received_at_rfc3339: None,
            date_rfc3339: now.clone(),
            body_text: Some("Body".to_string()),
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
        upsert_provider_message(
            None,
            &pool,
            "gmail",
            "account",
            "me@example.com",
            "Me",
            &crate::commands::accounts::AccountConfig::from_plaintext("{}"),
            "inbox",
            "INBOX",
            "inbox",
            &message,
            None,
        )
        .await
        .expect("inbox upsert");

        let priority: i64 = sqlx::query_scalar("SELECT priority FROM threads LIMIT 1")
            .fetch_one(&pool)
            .await
            .expect("priority");
        let trigger_count: i64 =
            sqlx::query_scalar("SELECT trigger_count FROM rules WHERE id = 'rule'")
                .fetch_one(&pool)
                .await
                .expect("trigger count");
        assert_eq!(priority, 3);
        assert_eq!(trigger_count, 1);

        upsert_provider_message(
            None,
            &pool,
            "gmail",
            "account",
            "me@example.com",
            "Me",
            &crate::commands::accounts::AccountConfig::from_plaintext("{}"),
            "inbox",
            "INBOX",
            "inbox",
            &message,
            None,
        )
        .await
        .expect("repeat inbox upsert");
        let trigger_count: i64 =
            sqlx::query_scalar("SELECT trigger_count FROM rules WHERE id = 'rule'")
                .fetch_one(&pool)
                .await
                .expect("repeat trigger count");
        assert_eq!(trigger_count, 1);

        // Archived in Gmail Web: every real label is gone and only the
        // category view is left, which is what earns the synthetic ARCHIVE.
        message.mailbox_ids = vec!["CATEGORY_PERSONAL".to_string()];
        // Merely observing the relabel cannot remove the only stored copy.
        assert!(
            reconcile_gmail_message_mailboxes(&pool, "account", &message)
                .await
                .expect("defer archive cleanup")
                .is_empty()
        );
        let archive_folder_id =
            crate::sync::gmail_reconcile::ensure_gmail_archive_folder(&pool, "account")
                .await
                .expect("archive folder");
        upsert_provider_message(
            None,
            &pool,
            "gmail",
            "account",
            "me@example.com",
            "Me",
            &crate::commands::accounts::AccountConfig::from_plaintext("{}"),
            &archive_folder_id,
            "ARCHIVE",
            "archive",
            &message,
            None,
        )
        .await
        .expect("archive upsert");

        let copies: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM messages m
             JOIN folders f ON f.id = m.folder_id
             WHERE m.account_id = 'account' AND m.remote_id = 'gmail-message'
               AND f.folder_type = 'archive'",
        )
        .fetch_one(&pool)
        .await
        .expect("archive copies");
        assert_eq!(copies, 1);
    }

    #[tokio::test]
    async fn outlook_delta_reuses_the_translated_legacy_row() {
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
             VALUES ('account', 'Me', 'me@example.com', 'outlook', '{}', '', ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("account");
        sqlx::query(
            "INSERT INTO folders
             (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at)
             VALUES ('inbox', 'account', 'Inbox', 'inbox', 'inbox', 'inbox', ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("folder");
        sqlx::query(
            "INSERT INTO threads
             (id, account_id, subject, last_message_date, message_count, unread_count,
              workflow_state, created_at, updated_at)
             VALUES ('legacy-thread', 'account', 'Subject', ?, 1, 0, 'inbox', ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("thread");
        sqlx::query(
            "INSERT INTO messages
             (id, account_id, folder_id, thread_id, message_id_header, from_address,
              to_addresses, subject, date, body_text, is_read, remote_id, created_at, updated_at)
             VALUES ('legacy-local-id', 'account', 'inbox', 'legacy-thread',
                     '<message@example.com>', 'sender@example.com', '[]', 'Subject', ?,
                     'Old body', 1, 'immutable-id', ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("legacy message");
        let message = MessageEnvelope {
            remote_id: "immutable-id".to_string(),
            remote_uidvalidity: None,
            remote_thread_id: Some("conversation".to_string()),
            original_source: None,
            mailbox_ids: Vec::new(),
            message_id_header: Some("<message@example.com>".to_string()),
            in_reply_to: None,
            references: Vec::new(),
            subject: "Subject".to_string(),
            snippet: "New body".to_string(),
            from_address: "sender@example.com".to_string(),
            from_name: Some("Sender".to_string()),
            to_addresses: vec!["me@example.com".to_string()],
            cc_addresses: Vec::new(),
            bcc_addresses: Vec::new(),
            reply_to: None,
            remote_draft_id: None,
            received_at_rfc3339: None,
            date_rfc3339: now.clone(),
            body_text: Some("New body".to_string()),
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
        upsert_provider_message(
            None,
            &pool,
            "outlook",
            "account",
            "me@example.com",
            "Me",
            &crate::commands::accounts::AccountConfig::from_plaintext("{}"),
            "inbox",
            "inbox",
            "inbox",
            &message,
            None,
        )
        .await
        .expect("delta upsert");

        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT id, body_text FROM messages WHERE account_id = 'account'")
                .fetch_all(&pool)
                .await
                .expect("messages");
        assert_eq!(rows, vec![("legacy-local-id".into(), "New body".into())]);
    }

    #[tokio::test]
    async fn provider_upsert_persists_draft_identity_and_reply_metadata() {
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
             VALUES ('drafts', 'account', 'Drafts', 'Drafts', 'drafts', 'DRAFT', ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("folder");
        let message = MessageEnvelope {
            remote_id: "remote-message".to_string(),
            remote_uidvalidity: None,
            remote_thread_id: Some("remote-thread".to_string()),
            original_source: None,
            mailbox_ids: vec!["DRAFT".to_string()],
            message_id_header: Some("<draft@example.com>".to_string()),
            in_reply_to: Some("<parent@example.com>".to_string()),
            references: vec!["<root@example.com>".to_string()],
            subject: "Draft".to_string(),
            snippet: "Body".to_string(),
            from_address: "me@example.com".to_string(),
            from_name: Some("Me".to_string()),
            to_addresses: vec!["recipient@example.com".to_string()],
            cc_addresses: Vec::new(),
            bcc_addresses: vec!["hidden@example.com".to_string()],
            reply_to: Some("replies@example.net".to_string()),
            remote_draft_id: Some("stable-draft".to_string()),
            received_at_rfc3339: None,
            date_rfc3339: now.clone(),
            body_text: Some("Body".to_string()),
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
            attachments: vec![RemoteAttachment {
                remote_part_id: "mime-1".to_string(),
                filename: Some("note.txt".to_string()),
                content_type: "text/plain".to_string(),
                size: Some(8),
                content_id: None,
                is_inline: false,
                content_base64: Some("b2xkIGJvZHk=".to_string()),
            }],
        };
        upsert_provider_message(
            None,
            &pool,
            "gmail",
            "account",
            "me@example.com",
            "Me",
            &crate::commands::accounts::AccountConfig::from_plaintext("{}"),
            "drafts",
            "DRAFT",
            "drafts",
            &message,
            None,
        )
        .await
        .expect("upsert");

        let row: (i64, String, String, String) = sqlx::query_as(
            "SELECT is_draft, remote_draft_id, reply_to, bcc_addresses FROM messages LIMIT 1",
        )
        .fetch_one(&pool)
        .await
        .expect("message row");
        assert_eq!(row.0, 1);
        assert_eq!(row.1, "stable-draft");
        assert_eq!(row.2, "replies@example.net");
        assert_eq!(row.3, "[\"hidden@example.com\"]");
        sqlx::query("UPDATE attachments SET cached_path = '/tmp/stale-draft-cache'")
            .execute(&pool)
            .await
            .expect("seed stale cache");

        let mut replacement = message.clone();
        replacement.remote_id = "replacement-message".to_string();
        replacement.body_text = Some("Replaced body".to_string());
        upsert_provider_message(
            None,
            &pool,
            "gmail",
            "account",
            "me@example.com",
            "Me",
            &crate::commands::accounts::AccountConfig::from_plaintext("{}"),
            "drafts",
            "DRAFT",
            "drafts",
            &replacement,
            None,
        )
        .await
        .expect("replace draft snapshot");

        let changes = reconcile_imap_folder_snapshot(
            &pool,
            "account",
            "drafts",
            "DRAFT",
            &[RemoteMessageState {
                remote_id: "replacement-message".to_string(),
                is_read: None,
                is_starred: None,
            }],
            // Gmail: neither side names a generation.
            None,
        )
        .await
        .expect("draft snapshot reconciliation");
        assert!(changes.relocated_threads.is_empty());
        let replacement_row: (i64, String, String, String) = sqlx::query_as(
            "SELECT COUNT(*), remote_id, remote_draft_id, body_text
             FROM messages WHERE remote_draft_id = 'stable-draft'",
        )
        .fetch_one(&pool)
        .await
        .expect("replacement row");
        assert_eq!(replacement_row.0, 1);
        assert_eq!(replacement_row.1, "replacement-message");
        assert_eq!(replacement_row.2, "stable-draft");
        assert_eq!(replacement_row.3, "Replaced body");
        let attachment_state: (String, Option<String>) = sqlx::query_as(
            "SELECT remote_uid, cached_path FROM attachments WHERE message_id IN (
               SELECT id FROM messages WHERE remote_draft_id = 'stable-draft'
             )",
        )
        .fetch_one(&pool)
        .await
        .expect("replacement attachment");
        assert_eq!(attachment_state, ("replacement-message".into(), None));
    }

    /// A queued send saves its draft's identity. Ordinary sync then adopts the
    /// draft row for the server copy, before or after the enqueue, and assigns
    /// it a UID (first ingest) or a new one (autosave re-upload) in the same
    /// mailbox generation. Undo, scheduled-draft Edit and Discard must survive
    /// that for every provider; only a new UIDVALIDITY may refuse the snapshot.
    mod queued_draft_identity {
        use super::*;
        use satchel_core::drafts::{self, send_queue::build_send_action, DraftError};

        const SNAPSHOT_BODY: &str = "Queued text";

        async fn draft_pool(provider: &str) -> sqlx::SqlitePool {
            let pool = ingest_pool().await;
            sqlx::query("UPDATE accounts SET provider = ?")
                .bind(provider)
                .execute(&pool)
                .await
                .unwrap();
            sqlx::query(
                "INSERT INTO folders (id, account_id, name, full_path, folder_type, remote_id, uidvalidity, created_at, updated_at)
                 VALUES ('drafts', 'account', 'Drafts', 'Drafts', 'drafts', ?, ?, '', '')",
            )
            .bind(if provider == "gmail" { "DRAFT" } else { "Drafts" })
            .bind((provider == "imap").then_some(7_i64))
            .execute(&pool)
            .await
            .unwrap();
            pool
        }

        fn server_copy(provider: &str, version: u32, generation: u32) -> MessageEnvelope {
            let mut message = envelope(&match provider {
                "imap" => (40 + version).to_string(),
                "gmail" => format!("gmail-message-{version}"),
                // Outlook updates a draft in place; its id is the draft id.
                _ => "outlook-draft".to_string(),
            });
            message.remote_uidvalidity = (provider == "imap").then_some(generation);
            message.remote_draft_id = (provider != "outlook").then(|| "stable-draft".to_string());
            message.mailbox_ids = vec!["DRAFT".to_string()];
            message.message_id_header = Some("<draft@example.com>".to_string());
            message.subject = "Subject".to_string();
            message.body_text = Some(format!("Server copy {version}"));
            message.from_address = "account@example.com".to_string();
            message.date_rfc3339 = format!("2026-01-01T00:00:0{version}Z");
            message
        }

        async fn ingest(pool: &sqlx::SqlitePool, provider: &str, message: &MessageEnvelope) {
            upsert_provider_message(
                None,
                pool,
                provider,
                "account",
                "account@example.com",
                "Account",
                &crate::commands::accounts::AccountConfig::from_plaintext("{}"),
                "drafts",
                if provider == "gmail" {
                    "DRAFT"
                } else {
                    "Drafts"
                },
                "drafts",
                message,
                None,
            )
            .await
            .unwrap();
        }

        async fn identity(pool: &sqlx::SqlitePool) -> (String, Option<String>, Option<i64>) {
            sqlx::query_as(
                "SELECT id, remote_id, remote_uidvalidity FROM messages WHERE is_draft = 1",
            )
            .fetch_one(pool)
            .await
            .unwrap()
        }

        /// Saves draft `d1`, records its provider id as an upload would, then
        /// queues a scheduled send of `SNAPSHOT_BODY` from it.
        async fn queue_send(
            pool: &sqlx::SqlitePool,
            queue: &SyncQueue,
            provider: &str,
            ingest_before: bool,
        ) -> String {
            let input: drafts::DraftInput = serde_json::from_value(serde_json::json!({
                "id": "d1", "account_id": "account", "to": ["you@example.test"],
                "subject": "Subject", "body": "Autosaved text", "from_address": "account@example.com",
            }))
            .unwrap();
            drafts::save(pool, queue, &input).await.unwrap();
            sqlx::query("UPDATE messages SET remote_draft_id = ? WHERE id = 'd1'")
                .bind(if provider == "outlook" {
                    "outlook-draft"
                } else {
                    "stable-draft"
                })
                .execute(pool)
                .await
                .unwrap();
            if ingest_before {
                ingest(pool, provider, &server_copy(provider, 1, 7)).await;
            }
            let outgoing = serde_json::from_value(serde_json::json!({
                "draft_id": "d1", "to": ["you@example.test"], "subject": "Subject",
                "body": SNAPSHOT_BODY, "from_address": "account@example.com",
            }))
            .unwrap();
            let action = build_send_action(
                "account".into(),
                outgoing,
                Some(chrono::Utc::now() + chrono::Duration::days(1)),
            )
            .unwrap();
            drafts::enqueue_send_with_draft(pool, queue, &action)
                .await
                .unwrap();
            action.id
        }

        async fn queued_sends(pool: &sqlx::SqlitePool) -> i64 {
            sqlx::query_scalar("SELECT COUNT(*) FROM sync_queue WHERE action_type = 'send_message'")
                .fetch_one(pool)
                .await
                .unwrap()
        }

        #[tokio::test]
        async fn sync_adopting_the_server_copy_keeps_undo_edit_and_discard() {
            for provider in ["imap", "gmail", "outlook"] {
                for ordering in ["first_ingest_after_enqueue", "reupload_after_enqueue"] {
                    for operation in ["undo", "edit", "discard"] {
                        let case = format!("{provider}/{ordering}/{operation}");
                        let pool = draft_pool(provider).await;
                        let queue = SyncQueue::new();
                        let reupload = ordering == "reupload_after_enqueue";
                        let queued = queue_send(&pool, &queue, provider, reupload).await;
                        let before = identity(&pool).await;
                        ingest(
                            &pool,
                            provider,
                            &server_copy(provider, 1 + u32::from(reupload), 7),
                        )
                        .await;
                        let after = identity(&pool).await;
                        assert_eq!(after.0, "d1", "{case}: sync adopts the local draft row");
                        assert!(after.1.is_some(), "{case}");
                        if reupload && provider != "outlook" {
                            assert_ne!(before.1, after.1, "{case}: the re-upload replaces the UID");
                        } else if !reupload {
                            assert_eq!(before.1, None, "{case}: the draft was local when queued");
                        }
                        match operation {
                            "undo" | "edit" => {
                                let draft = if operation == "undo" {
                                    drafts::restore_queued_draft(&pool, &queue, &queued).await
                                } else {
                                    drafts::load(&pool, &pool, &queue, "d1")
                                        .await
                                        .map(|draft| draft.expect("draft"))
                                }
                                .unwrap_or_else(|error| panic!("{case}: {error:?}"));
                                assert_eq!(draft.id, "d1", "{case}");
                                assert_eq!(draft.body, SNAPSHOT_BODY, "{case}");
                            }
                            _ => {
                                drafts::discard(&pool, &queue, "d1")
                                    .await
                                    .unwrap_or_else(|error| panic!("{case}: {error:?}"));
                                let remaining: i64 = sqlx::query_scalar(
                                    "SELECT COUNT(*) FROM messages WHERE id = 'd1'",
                                )
                                .fetch_one(&pool)
                                .await
                                .unwrap();
                                assert_eq!(remaining, 0, "{case}");
                            }
                        }
                        assert_eq!(queued_sends(&pool).await, 0, "{case}");
                    }
                }
            }
        }

        #[tokio::test]
        async fn a_new_mailbox_generation_still_refuses_the_queued_snapshot() {
            for ingest_before in [false, true] {
                for operation in ["undo", "edit"] {
                    let pool = draft_pool("imap").await;
                    let queue = SyncQueue::new();
                    let queued = queue_send(&pool, &queue, "imap", ingest_before).await;
                    // The server reset Drafts; sync adopts the same draft row
                    // under the new UIDVALIDITY.
                    sqlx::query("UPDATE folders SET uidvalidity = 8 WHERE id = 'drafts'")
                        .execute(&pool)
                        .await
                        .unwrap();
                    ingest(&pool, "imap", &server_copy("imap", 1, 8)).await;
                    assert_eq!(
                        identity(&pool).await,
                        ("d1".to_string(), Some("41".to_string()), Some(8))
                    );
                    let result = if operation == "undo" {
                        drafts::restore_queued_draft(&pool, &queue, &queued).await
                    } else {
                        drafts::load(&pool, &pool, &queue, "d1")
                            .await
                            .map(|draft| draft.expect("draft"))
                    };
                    if ingest_before {
                        assert!(
                            matches!(result, Err(DraftError::QueuedDraftNotRestorable)),
                            "{operation}: {result:?}"
                        );
                        let body: Option<String> =
                            sqlx::query_scalar("SELECT body_text FROM messages WHERE id = 'd1'")
                                .fetch_one(&pool)
                                .await
                                .unwrap();
                        assert_eq!(body.as_deref(), Some("Server copy 1"));
                        assert_eq!(queued_sends(&pool).await, 1);
                        // Discard needs no snapshot: it stops the send and removes the draft.
                        drafts::discard(&pool, &queue, "d1").await.unwrap();
                        assert_eq!(queued_sends(&pool).await, 0);
                    } else {
                        // Queued while still local, the snapshot never had a generation.
                        assert_eq!(result.unwrap().body, SNAPSHOT_BODY, "{operation}");
                    }
                }
            }
        }
    }
}
