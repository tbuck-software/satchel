use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use satchel_core::db::sync_quarantine::SyncQuarantineRepository;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};
use tauri::{AppHandle, Manager};

use crate::events::bus::{AppEvent, EventBus};
use crate::providers::{
    cursor::{backfill_pending, backfill_stats},
    folder_mapping::provider_folder_type,
    imap::imap_checkpoint,
    pop3::POP3_FOLDER,
    provider_for_account,
    traits::{MessageEnvelope, MessageRef, RemoteFolder, SyncCheckpoint},
};
use crate::sync::folder_reconcile::{
    folder_snapshot_due, reconcile_external_workflow_states, reconcile_imap_folder_rename,
    reconcile_imap_folder_snapshot, reconcile_missing_remote_folders,
    reconcile_remote_message_removals, reset_local_provider_folder_with_generation,
    should_snapshot_folder,
};
use crate::sync::gmail_reconcile::{ensure_gmail_label_folder, gmail_target_mailbox_ids};

#[cfg(mobile)]
use crate::sync::leases::acquire_mobile_mailbox_work_permit;
use crate::sync::leases::sync_run_lock;
#[cfg(target_os = "android")]
use crate::sync::leases::CrossProcessSyncLease;
use crate::sync::message_ingest::{replay_ingest_effects, upsert_provider_messages};
use crate::sync::outlook_migration::migrate_outlook_remote_ids;
use crate::sync::queue::{QueuedAction, SyncQueue};
use crate::sync::remote_mutations::{RemoteMutationPayload, ACTION_REMOTE_MUTATION};
use crate::sync::repairs::{repair_gmail_archive_duplicates, repair_stale_done_threads_in_inbox};
use crate::sync::thread_resolution::{
    deduplicate_cached_draft_autosaves, reconcile_sent_draft_autosaves,
    refresh_provider_thread_projections,
};
use crate::utils::oauth::{ensure_fresh_account_config, OAuthProvider};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AccountSelection<'a> {
    AllActive,
    One(&'a str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FolderSelection {
    All,
    Inbox,
}

pub(crate) struct MailboxSyncRequest<'a> {
    pub(crate) events: Option<&'a Arc<EventBus>>,
    pub(crate) app: Option<&'a AppHandle>,
    pub(crate) accounts: AccountSelection<'a>,
    pub(crate) folders: FolderSelection,
}

fn prepare_folders_for_sync(
    provider: &str,
    folders: &mut Vec<RemoteFolder>,
    selection: FolderSelection,
) {
    folders.sort_by_key(|folder| {
        provider_folder_type(provider, &folder.name, &folder.id, folder.kind.as_deref()) != "inbox"
    });
    if selection == FolderSelection::Inbox {
        folders.retain(|folder| {
            provider_folder_type(provider, &folder.name, &folder.id, folder.kind.as_deref())
                == "inbox"
        });
    }
}

/// `accountId` lets the UI open that account's settings, e.g. to replace a rejected password.
fn encoded_account_sync_failure(account: &str, account_id: &str, message: impl ToString) -> String {
    crate::commands::error::CommandError::new("backend.errors.accountSyncFailed")
        .with_param("account", account)
        .with_param("accountId", account_id)
        .with_param("message", message.to_string())
        .encoded()
}

fn encoded_folder_sync_failure(account: &str, folder: &str, message: impl ToString) -> String {
    crate::commands::error::CommandError::new("backend.errors.folderSyncFailed")
        .with_param("account", account)
        .with_param("folder", folder)
        .with_param("message", message.to_string())
        .encoded()
}

fn encoded_quarantined_messages(account: &str, folder: &str, count: usize) -> String {
    crate::commands::error::CommandError::new("backend.errors.messagesQuarantined")
        .with_param("account", account)
        .with_param("folder", folder)
        .with_param("count", count as i64)
        .encoded()
}

fn encoded_folder_ingest_failure(account: &str, folders: &[String], count: usize) -> String {
    crate::commands::error::CommandError::new("backend.errors.folderIngestBlocked")
        .with_param("account", account)
        .with_param("folders", folders.join(", "))
        .with_param("count", count as i64)
        .encoded()
}

/// How one folder's batch fared.
#[derive(Debug, Default)]
struct FolderIngestOutcome {
    ingested: usize,
    failed: usize,
    skipped: usize,
    /// Remote ids this batch committed, with no failure of theirs on record.
    ///
    /// The only evidence that a message is stored locally. The cursor is none:
    /// it also moves past messages that exhausted their quarantine attempts
    /// and never landed.
    persisted: Vec<String>,
}

impl FolderIngestOutcome {
    /// Whether the folder's cursor may move past this batch.
    fn cursor_may_advance(&self) -> bool {
        self.failed == 0
    }
}

/// Everything one folder's ingest needs besides the messages themselves.
struct FolderIngestContext<'a> {
    app: Option<&'a AppHandle>,
    events: Option<&'a Arc<EventBus>>,
    account: &'a MailAccountRow,
    config: &'a crate::commands::accounts::AccountConfig,
    folder_id: &'a str,
    folder_remote_id: &'a str,
    folder_type: &'a str,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct MailboxSyncSummary {
    pub(crate) accounts_synced: usize,
    pub(crate) accounts_failed: usize,
    pub(crate) folders_synced: usize,
    pub(crate) messages_synced: usize,
    pub(crate) backfill_pending_accounts: usize,
    pub(crate) backfill_remaining_messages: usize,
    pub(crate) backfill_total_messages: usize,
    pub(crate) failures: Vec<String>,
}

#[derive(Debug, FromRow)]
struct MailAccountRow {
    id: String,
    display_name: String,
    email: String,
    provider: String,
    config: String,
    sync_state: Option<String>,
    outlook_immutable_ids_migrated: i64,
    ingest_cutoff: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Default)]
struct ImapSyncState {
    #[serde(default)]
    cursors: HashMap<String, String>,
    #[serde(default)]
    folder_snapshot_at: HashMap<String, String>,
    /// Must survive a replacement cursor, failed snapshot, and process restart.
    /// Older state decodes as empty; older readers ignore the additional field.
    #[serde(default, skip_serializing_if = "HashSet::is_empty")]
    pending_snapshots: HashSet<String>,
}

impl ImapSyncState {
    fn request_snapshot(&mut self, folder: &str, snapshot_due: bool, required: bool) -> bool {
        if snapshot_due && required {
            self.pending_snapshots.insert(folder.to_string());
        }
        self.pending_snapshots.contains(folder)
    }
}

/// A provider returns Some only after every snapshot page has succeeded.
/// Keep the obligation on None, network failure, or a stale IMAP generation.
async fn reconcile_folder_snapshot(
    pool: &SqlitePool,
    provider: &dyn crate::providers::traits::MailProvider,
    account_id: &str,
    folder_id: &str,
    remote_folder_id: &str,
    state: &mut ImapSyncState,
    queue: &SyncQueue,
) -> Result<HashSet<String>, String> {
    let Some(snapshot) = provider
        .sync_message_state_snapshot(remote_folder_id)
        .await
        .map_err(|error| error.to_string())?
    else {
        return Ok(HashSet::new());
    };
    let changes = reconcile_imap_folder_snapshot(
        pool,
        account_id,
        folder_id,
        remote_folder_id,
        &snapshot.states,
        snapshot.remote_uidvalidity,
    )
    .await?;
    if !changes.skipped_stale_generation {
        state.folder_snapshot_at.insert(
            remote_folder_id.to_string(),
            chrono::Utc::now().to_rfc3339(),
        );
        state.pending_snapshots.remove(remote_folder_id);
        persist_sync_cursors(pool, account_id, state, queue, None).await?;
    } else {
        tracing::warn!(
            account_id,
            folder = remote_folder_id,
            snapshot_uidvalidity = ?snapshot.remote_uidvalidity,
            "folder snapshot named another generation, kept every message"
        );
    }
    Ok(changes.relocated_threads)
}

/// Persist the folder cursors reached so far, plus any remote mutation this
/// batch earned.
///
/// Written after every folder instead of once at the end of the account: a
/// single unusable message in the last folder used to discard the progress of
/// every folder before it, so the next run refetched everything and failed at
/// the same message again.
///
/// `mutation` shares the transaction because a queued server-side deletion is
/// only correct together with the cursor that recorded the download. An abort
/// before the commit leaves neither, and the next run fetches the batch again.
async fn persist_sync_cursors(
    pool: &SqlitePool,
    account_id: &str,
    state: &ImapSyncState,
    queue: &SyncQueue,
    mutation: Option<&QueuedAction>,
) -> Result<(), String> {
    let encoded = serde_json::to_string(state).map_err(|err| err.to_string())?;
    let now = chrono::Utc::now().to_rfc3339();
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|err| err.to_string())?;
    sqlx::query("UPDATE accounts SET sync_state = ?, updated_at = ? WHERE id = ?")
        .bind(encoded)
        .bind(&now)
        .bind(account_id)
        .execute(&mut *tx)
        .await
        .map_err(|err| err.to_string())?;
    if let Some(mutation) = mutation {
        queue.enqueue_in_transaction(&mut tx, mutation).await?;
    }
    tx.commit().await.map_err(|err| err.to_string())?;
    if mutation.is_some() {
        queue.wake_worker();
    }
    Ok(())
}

async fn reset_folder_checkpoint(
    pool: &SqlitePool,
    account_id: &str,
    folder_id: &str,
    folder_remote_id: &str,
    state: &mut ImapSyncState,
    queue: &SyncQueue,
) -> Result<(), String> {
    SyncQuarantineRepository::new(pool)
        .clear_folder(account_id, folder_id)
        .await
        .map_err(|error| error.to_string())?;
    state.cursors.remove(folder_remote_id);
    state.folder_snapshot_at.remove(folder_remote_id);
    persist_sync_cursors(pool, account_id, state, queue, None).await
}

/// Queue the server-side removal of exactly the UIDLs this run committed.
///
/// POP3 identifies mail by UIDL and has no folders, so the refs carry the one
/// mailbox the protocol offers. `Pop3Provider::delete_messages` maps them back
/// to session numbers when the queue worker runs.
fn pop3_delete_mutation(account_id: &str, uidls: &[String]) -> Result<QueuedAction, String> {
    let refs = uidls
        .iter()
        .map(|uidl| MessageRef {
            remote_id: uidl.clone(),
            source_folder: Some(POP3_FOLDER.to_string()),
            source_uidvalidity: None,
        })
        .collect::<Vec<_>>();
    let payload = serde_json::to_value(RemoteMutationPayload::Delete { refs })
        .map_err(|err| err.to_string())?;
    Ok(QueuedAction::new(
        account_id.to_string(),
        ACTION_REMOTE_MUTATION,
        payload,
    ))
}

/// Persist bounded batches, then quarantine failed savepoints.
/// Only committed messages without quarantine history authorize POP3 deletion.
async fn ingest_messages_into_folder(
    pool: &SqlitePool,
    context: &FolderIngestContext<'_>,
    folder_id: &str,
    folder_remote_id: &str,
    folder_type: &str,
    messages: &[MessageEnvelope],
    outcome: &mut FolderIngestOutcome,
) -> Result<(), String> {
    let account = context.account;
    let repository = SyncQuarantineRepository::new(pool);
    let quarantine = repository
        .load_folder(&account.id, folder_id)
        .await
        .map_err(|error| error.to_string())?;
    let mut candidates = Vec::new();
    for message in messages {
        if quarantine.is_exhausted(&message.remote_id) {
            outcome.skipped += 1;
            if account.provider.eq_ignore_ascii_case("gmail") {
                let labels = gmail_target_mailbox_ids(&message.mailbox_ids);
                let labels = serde_json::to_string(&labels).map_err(|error| error.to_string())?;
                let has_old_copy: bool = sqlx::query_scalar(
                    "SELECT EXISTS(
                       SELECT 1 FROM messages m JOIN folders f ON f.id = m.folder_id
                       WHERE m.account_id = ? AND m.remote_id = ?
                         AND NOT EXISTS (SELECT 1 FROM json_each(?) label
                             WHERE LOWER(TRIM(label.value)) = LOWER(TRIM(f.remote_id)))
                     )",
                )
                .bind(&account.id)
                .bind(&message.remote_id)
                .bind(labels)
                .fetch_one(pool)
                .await
                .map_err(|error| error.to_string())?;
                // Only an unfinished relabel holds progress after exhaustion.
                // New poison messages retain the normal quarantine skip policy.
                if has_old_copy {
                    outcome.failed += 1;
                }
            }
        } else {
            candidates.push(message.clone());
        }
    }
    let messages = candidates;
    let results = upsert_provider_messages(
        context.app,
        pool,
        &account.provider,
        &account.id,
        &account.email,
        &account.display_name,
        context.config,
        folder_id,
        folder_remote_id,
        folder_type,
        &messages,
        context.events,
        account.ingest_cutoff.as_deref(),
    )
    .await?;
    for (message, result) in messages.iter().zip(results) {
        match result {
            Ok(()) => {
                outcome.ingested += 1;
                if quarantine.contains(&message.remote_id) {
                    repository
                        .clear(&account.id, folder_id, &message.remote_id)
                        .await
                        .map_err(|error| error.to_string())?;
                } else {
                    outcome.persisted.push(message.remote_id.clone());
                }
            }
            Err(error) => {
                outcome.failed += 1;
                let attempts = repository
                    .record_failure(&account.id, folder_id, &message.remote_id, &error)
                    .await
                    .map_err(|error| error.to_string())?;
                tracing::warn!(account_id = %account.id, folder_id, remote_id = %message.remote_id,
                    attempts, "message ingest failed and was quarantined: {error}");
            }
        }
    }
    Ok(())
}

async fn ingest_folder_batch(
    pool: &SqlitePool,
    context: &FolderIngestContext<'_>,
    messages: Vec<MessageEnvelope>,
    externally_changed_threads: &mut HashSet<String>,
) -> Result<FolderIngestOutcome, String> {
    let account = context.account;
    let mut outcome = FolderIngestOutcome::default();
    if account.provider.eq_ignore_ascii_case("gmail") {
        let mut targets = std::collections::BTreeMap::<String, Vec<MessageEnvelope>>::new();
        for message in messages {
            // Account-wide history must land in actual labels, even when a
            // different folder's cursor observed it. Ingest owns stale-copy
            // cleanup in the target savepoint, never before it.
            let threads = sqlx::query_scalar::<_, String>(
                "SELECT DISTINCT thread_id FROM messages WHERE account_id = ?
                 AND remote_id = ? AND thread_id IS NOT NULL",
            )
            .bind(&account.id)
            .bind(&message.remote_id)
            .fetch_all(pool)
            .await
            .map_err(|error| error.to_string())?;
            externally_changed_threads.extend(threads);
            let labels = gmail_target_mailbox_ids(&message.mailbox_ids);
            for label in labels {
                targets.entry(label).or_default().push(message.clone());
            }
        }
        for (label, messages) in targets {
            let (folder_id, folder_type) =
                ensure_gmail_label_folder(pool, &account.id, &label).await?;
            ingest_messages_into_folder(
                pool,
                context,
                &folder_id,
                &label,
                &folder_type,
                &messages,
                &mut outcome,
            )
            .await?;
        }
    } else {
        ingest_messages_into_folder(
            pool,
            context,
            context.folder_id,
            context.folder_remote_id,
            context.folder_type,
            &messages,
            &mut outcome,
        )
        .await?;
    }
    Ok(outcome)
}

async fn stamp_inbox_fetch(
    pool: &SqlitePool,
    account_id: &str,
    started_at: &str,
) -> Result<(), String> {
    sqlx::query("UPDATE accounts SET last_sync_at = ?, updated_at = ? WHERE id = ?")
        .bind(started_at)
        .bind(chrono::Utc::now().to_rfc3339())
        .bind(account_id)
        .execute(pool)
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

const ACCOUNT_SYNC_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10 * 60);

async fn account_run_with_deadline<T>(
    duration: std::time::Duration,
    run: impl std::future::Future<Output = Result<T, String>>,
) -> Result<T, String> {
    tokio::time::timeout(duration, run).await.map_err(|_| {
        format!(
            "account sync timed out after {} seconds",
            duration.as_secs()
        )
    })?
}

/// Run one account's sync under its lock (and, on mobile, the global mailbox
/// permit), with the deadline counted from the moment both are held.
///
/// Serialize one complete account snapshot, then release the permit so due
/// Outbox work gets a fair turn before the next account, avoiding the old
/// all-account lock convoy. The wait for the lock is not part of the run: a
/// poll queued behind a long full sync used to time out without doing any work
/// and mark the account as failed.
async fn account_run_exclusive<T>(
    account_id: &str,
    duration: std::time::Duration,
    run: impl std::future::Future<Output = Result<T, String>>,
) -> Result<T, String> {
    #[cfg(mobile)]
    let _mobile_work_guard = acquire_mobile_mailbox_work_permit().await;
    let run_lock = sync_run_lock(account_id);
    let _run_guard = run_lock.lock().await;
    account_run_with_deadline(duration, run).await
}

/// Execute one mailbox synchronization request.
///
/// `folders` controls whether the provider's complete folder set or only the
/// cached Inbox is visited. The optional app handle and event bus are adapters
/// for app-side observability; ingestion and all SQLite ordering stay shared.
pub(crate) async fn sync_mailboxes(
    pool: &SqlitePool,
    request: MailboxSyncRequest<'_>,
) -> Result<MailboxSyncSummary, String> {
    // IDLE watching itself is not work. Its triggered mailbox runs, foreground
    // requests, Dock actions and notification refreshes all enter here.
    let _work = request
        .app
        .map(|app| app.state::<crate::AppState>().sync_activity.start_work());
    let MailboxSyncRequest {
        events,
        app,
        accounts,
        folders: folder_selection,
    } = request;
    let account_ids = if let AccountSelection::One(account_id) = accounts {
        sqlx::query_scalar::<_, String>(
            "SELECT id FROM accounts WHERE provider IN ('imap', 'pop3', 'gmail', 'outlook') AND is_active = 1 AND id = ?",
        )
        .bind(account_id)
        .fetch_all(pool)
        .await
    } else {
        sqlx::query_scalar::<_, String>(
            "SELECT id FROM accounts WHERE provider IN ('imap', 'pop3', 'gmail', 'outlook') AND is_active = 1",
        )
        .fetch_all(pool)
        .await
    }
    .map_err(|err| err.to_string())?;

    let mut summary = MailboxSyncSummary::default();

    for account_id in account_ids {
        let result = account_run_exclusive(&account_id, ACCOUNT_SYNC_TIMEOUT, async {
        // Re-reading under the lock prevents a concurrent run from restoring a
        // stale cursor.
        let Some(mut account) = sqlx::query_as::<_, MailAccountRow>(
            "SELECT id, display_name, email, provider, config, sync_state, outlook_immutable_ids_migrated,
                    COALESCE(NULLIF(last_sync_at, ''), NULLIF(created_at, '')) AS ingest_cutoff
             FROM accounts
             WHERE provider IN ('imap', 'pop3', 'gmail', 'outlook')
               AND is_active = 1 AND id = ?",
        )
        .bind(&account_id)
        .fetch_optional(pool)
        .await
        .map_err(|err| err.to_string())?
        else {
            return Ok(());
        };
        // Seeded local demo account should not block or pollute real sync runs.
        if crate::sync::is_seeded_demo_account(&account.id, &account.email) {
            return Ok(());
        }

        #[cfg(target_os = "android")]
        let cross_process_lease = match CrossProcessSyncLease::acquire(pool, &account.id).await? {
            Some(lease) => lease,
            None => {
                tracing::debug!(
                    account_id = %account.id,
                    "mailbox sync is already running in another Android process"
                );
                return Ok(());
            }
        };

        let sync_started_at = chrono::Utc::now().to_rfc3339();
        let _ = sqlx::query(
            "UPDATE accounts
             SET sync_status = 'syncing',
                 last_sync_error = NULL,
                 last_sync_error_at = NULL,
                 sync_progress_json = ?,
                 updated_at = ?
             WHERE id = ?",
        )
        .bind(serde_json::json!({"phase": "starting"}).to_string())
        .bind(&sync_started_at)
        .bind(&account.id)
        .execute(pool)
        .await;

        let account_sync_result: Result<(usize, usize, bool, usize, usize), String> = async {
            replay_ingest_effects(pool, &account.id).await?;
            let mut config =
                crate::commands::accounts::decrypt_account_config(&account.id, &account.config)?;
            if let Ok(oauth_provider) = OAuthProvider::parse(&account.provider) {
                if let Some(updated_config) =
                    ensure_fresh_account_config(oauth_provider, &config).await?
                {
                    let now = chrono::Utc::now().to_rfc3339();
                    sqlx::query(
                        "UPDATE accounts
                         SET config = ?, updated_at = ?, last_oauth_error = NULL, last_oauth_error_at = NULL
                         WHERE id = ?",
                    )
                        .bind(crate::commands::accounts::encrypt_account_config(
                            &account.id,
                            &updated_config,
                        )?)
                        .bind(&now)
                        .bind(&account.id)
                        .execute(pool)
                        .await
                        .map_err(|err| err.to_string())?;
                    config = crate::commands::accounts::AccountConfig::from_plaintext(updated_config);
                }
            }

            let provider =
                provider_for_account(
                    &account.provider,
                    &config,
                    &account.email,
                    stored_body_window_days(pool).await,
                )?;

            // Durable queue of the running app when there is one, so a fresh
            // deletion wakes the worker instead of waiting for its next poll.
            let queue = app
                .and_then(|app| app.try_state::<crate::AppState>())
                .map(|state| state.queue.clone())
                .unwrap_or_else(|| Arc::new(SyncQueue::new()));
            let pop3_deletes_after_fetch = account.provider.eq_ignore_ascii_case("pop3")
                && crate::providers::pop3::deletes_after_fetch(&config)?;

            if account.provider.eq_ignore_ascii_case("outlook")
                && account.outlook_immutable_ids_migrated == 0
            {
                migrate_outlook_remote_ids(pool, &account.id, provider.as_ref()).await?;
                account.outlook_immutable_ids_migrated = 1;
            }

            let mut state: ImapSyncState = account
                .sync_state
                .as_deref()
                .and_then(|raw| serde_json::from_str(raw).ok())
                .unwrap_or_default();

            // The frequent Inbox path already has the provider's stable remote
            // folder id in SQLite after the first complete sync. Re-listing all
            // folders every 15 seconds is surprisingly expensive on Outlook
            // (the Graph adapter also resolves six well-known folders) and
            // defeats the point of a lightweight freshness check.
            let cached_inbox = if folder_selection == FolderSelection::Inbox {
                sqlx::query_as::<_, (String, String)>(
                    "SELECT COALESCE(NULLIF(remote_id, ''), full_path), name
                     FROM folders
                     WHERE account_id = ? AND folder_type = 'inbox'
                     LIMIT 1",
                )
                .bind(&account.id)
                .fetch_optional(pool)
                .await
                .map_err(|err| err.to_string())?
                .map(|(id, name)| RemoteFolder {
                    id,
                    name,
                    kind: Some("inbox".to_string()),
                })
            } else {
                None
            };
            let mut folders = match cached_inbox {
                Some(folder) => vec![folder],
                None => provider
                    .sync_folders()
                    .await
                    .map_err(|error| error.to_string())?,
            };
            if folder_selection == FolderSelection::All
                && !account.provider.eq_ignore_ascii_case("pop3")
                && !folders
                    .iter()
                    .any(|folder| crate::workflow::states::is_aside_folder(&folder.name))
            {
                match provider
                    .create_folder(crate::workflow::states::ASIDE_FOLDER_NAME)
                    .await
                {
                    Ok(()) => {
                        // Gmail and Outlook assign opaque remote ids at creation
                        // time. Refresh, but keep syncing the existing folders
                        // if the provider is only eventually consistent.
                        match provider.sync_folders().await {
                            Ok(refreshed) if !refreshed.is_empty() => folders = refreshed,
                            Ok(_) => summary.failures.push(
                                crate::commands::error::CommandError::new(
                                    "backend.errors.asideFolderCreatedRefreshPending",
                                )
                                .with_param("account", account.email.clone())
                                .encoded(),
                            ),
                            Err(err) => summary.failures.push(
                                crate::commands::error::CommandError::new(
                                    "backend.errors.asideFolderCreatedRefreshFailed",
                                )
                                .with_param("account", account.email.clone())
                                .with_param("message", err.to_string())
                                .encoded(),
                            ),
                        }
                    }
                    Err(err) => {
                        // A convenience workflow folder must never take the
                        // entire mailbox offline. The next sync retries; an
                        // already-existing race is also discovered then.
                        summary.failures.push(
                            crate::commands::error::CommandError::new(
                                "backend.errors.asideFolderUnavailable",
                            )
                            .with_param("account", account.email.clone())
                            .with_param("message", err.to_string())
                            .encoded(),
                        );
                    }
                }
            }
            if folders.is_empty() {
                if folder_selection == FolderSelection::All {
                    reconcile_missing_remote_folders(pool, &account.id, &account.provider,
                        &HashSet::new(), &sync_started_at).await?;
                }
                return Ok((0, 0, false, 0, 0));
            }

            // New mail is the latency-sensitive part of a complete sync too.
            // Keep Inbox first so a slower label/folder backfill cannot delay
            // its ingestion and notification.
            let available_folder_ids: HashSet<String> =
                folders.iter().map(|folder| folder.id.clone()).collect();
            prepare_folders_for_sync(&account.provider, &mut folders, folder_selection);
            if folder_selection == FolderSelection::Inbox && folders.is_empty() {
                return Err(format!("{}: inbox folder not found", account.email));
            }

            let mut folders_synced = 0usize;
            let mut messages_synced = 0usize;
            let mut has_backfill_pending = false;
            let mut backfill_remaining_messages = 0usize;
            let mut backfill_total_messages = 0usize;
            let mut snapshot_folders = Vec::<(String, String)>::new();
            let mut snapshot_sync_complete = true;
            let mut externally_changed_threads = HashSet::new();
            let mut folder_fetch_failures = Vec::<String>::new();
            let mut blocked_folders = Vec::<String>::new();
            let mut blocked_messages = 0usize;

            let total_folders = folders.len();
            for (folder_index, folder) in folders.into_iter().enumerate() {
                let folder_id = match satchel_core::db::folders::resolve_provider_folder_id(
                    pool,
                    &account.id,
                    &folder.id,
                )
                .await
                {
                    Ok(id) => id,
                    Err(err) => {
                        let failure = encoded_folder_sync_failure(&account.email, &folder.name, err);
                        folder_fetch_failures.push(failure.clone());
                        summary.failures.push(failure);
                        continue;
                    }
                };
                let now = chrono::Utc::now().to_rfc3339();
                let folder_type = provider_folder_type(
                    &account.provider,
                    &folder.name,
                    &folder.id,
                    folder.kind.as_deref(),
                );
                let should_snapshot_folder =
                    should_snapshot_folder(&account.provider, folder_type, &folder.name);
                let snapshot_due = should_snapshot_folder
                    && folder_snapshot_due(
                        &account.provider,
                        folder_type,
                        &folder.name,
                        state.folder_snapshot_at.get(&folder.id).map(String::as_str),
                        chrono::Utc::now(),
                    );
                let _ = sqlx::query(
                    "UPDATE accounts
                     SET sync_progress_json = ?, updated_at = ?
                     WHERE id = ?",
                )
                .bind(
                    serde_json::json!({
                        "phase": "folder",
                        "folder": folder.name,
                        "folder_index": folder_index + 1,
                        "folder_total": total_folders,
                    })
                    .to_string(),
                )
                .bind(&now)
                .bind(&account.id)
                .execute(pool)
                .await;

                sqlx::query(
                    "INSERT INTO folders (id, account_id, name, full_path, folder_type, remote_id, uidvalidity, message_count, unread_count, sync_enabled, sort_order, created_at, updated_at)
                     VALUES (?, ?, ?, ?, ?, ?, NULL, 0, 0, 1, 0, ?, ?)
                     ON CONFLICT(id) DO UPDATE SET
                        name = excluded.name,
                        full_path = excluded.full_path,
                        folder_type = excluded.folder_type,
                        remote_id = excluded.remote_id,
                        updated_at = excluded.updated_at",
                )
                .bind(&folder_id)
                .bind(&account.id)
                .bind(&folder.name)
                .bind(&folder.id)
                .bind(folder_type)
                .bind(&folder.id)
                .bind(&now)
                .bind(&now)
                .execute(pool)
                .await
                .map_err(|err| err.to_string())?;

                // Read before the sync writes the fresh generation below: a
                // cursor from before the cursor carried one has no generation
                // of its own, and this row is then the only witness.
                let stored_uidvalidity =
                    sqlx::query_scalar::<_, Option<i64>>("SELECT uidvalidity FROM folders WHERE id = ?")
                        .bind(&folder_id)
                        .fetch_optional(pool)
                        .await
                        .map_err(|err| err.to_string())?
                        .flatten()
                        .and_then(|value| u32::try_from(value).ok());
                let checkpoint = if account.provider.eq_ignore_ascii_case("imap") {
                    imap_checkpoint(state.cursors.get(&folder.id).cloned(), stored_uidvalidity)
                } else {
                    SyncCheckpoint {
                        cursor: state.cursors.get(&folder.id).cloned(),
                    }
                };
                let inbox_fetch_started_at = (folder_type == "inbox")
                    .then(|| crate::commands::notifications::notification_cutoff_at(chrono::Utc::now()));
                let mut batch = match provider.sync_messages(&folder.id, checkpoint).await
                {
                    Ok(batch) => batch,
                    Err(err) => {
                        if snapshot_due {
                            snapshot_sync_complete = false;
                        }
                        let failure = encoded_folder_sync_failure(&account.email, &folder.name, err);
                        folder_fetch_failures.push(failure.clone());
                        summary.failures.push(failure);
                        continue;
                    }
                };

                if account.provider.eq_ignore_ascii_case("imap") && folder_selection == FolderSelection::All {
                    reconcile_imap_folder_rename(pool, &account.id, &folder_id, &available_folder_ids, &batch).await?;
                }

                // IMAP UIDs are meaningful only inside the mailbox's current
                // UIDVALIDITY generation. Persist the generation even for an
                // empty batch so blocked moves never fall back to a stale
                // folder snapshot.
                if account.provider.eq_ignore_ascii_case("imap") && !batch.reset_folder {
                    sqlx::query(
                        "UPDATE folders SET uidvalidity = ?, updated_at = ? WHERE id = ?",
                    )
                    .bind(batch.remote_uidvalidity.map(i64::from))
                    .bind(&now)
                    .bind(&folder_id)
                    .execute(pool)
                    .await
                    .map_err(|err| err.to_string())?;
                }

                let reset_thread_ids = if batch.reset_folder {
                    let uidvalidity = batch.remote_uidvalidity.ok_or_else(|| {
                        format!("IMAP reset for {} has no UIDVALIDITY", folder.id)
                    })?;
                    let thread_ids = reset_local_provider_folder_with_generation(
                        pool, &account.id, &folder_id, uidvalidity, &now,
                    ).await?;
                    // Retrying a failed ingest must not reset this generation again.
                    reset_folder_checkpoint(pool, &account.id, &folder_id, &folder.id,
                        &mut state, queue.as_ref()).await?;
                    thread_ids
                } else {
                    HashSet::new()
                };
                externally_changed_threads.extend(reset_thread_ids.iter().cloned());

                if state.request_snapshot(&folder.id, snapshot_due, batch.snapshot_required) {
                    snapshot_folders.push((folder_id.clone(), folder.id.clone()));
                }

                let next_cursor = batch.next.cursor.take();

                externally_changed_threads.extend(
                    reconcile_remote_message_removals(pool, &account.id, &batch.removals).await?,
                );

                let ingest = ingest_folder_batch(
                    pool,
                    &FolderIngestContext {
                        app,
                        events,
                        account: &account,
                        config: &config,
                        folder_id: &folder_id,
                        folder_remote_id: &folder.id,
                        folder_type,
                    },
                    std::mem::take(&mut batch.messages),
                    &mut externally_changed_threads,
                )
                .await?;
                messages_synced += ingest.ingested;
                // A failed/quarantined replacement must retain its old copy.
                // Presence reconciliation cannot delete it before retry.
                if account.provider.eq_ignore_ascii_case("gmail") && ingest.failed > 0 {
                    snapshot_sync_complete = false;
                }

                // The cursor is written per folder, right after its own batch.
                // A failed savepoint retains the cursor until it succeeds or
                // exhausts quarantine, so healthy neighbors cannot hide it.
                let pop3_delete = (pop3_deletes_after_fetch && !ingest.persisted.is_empty())
                    .then(|| pop3_delete_mutation(&account.id, &ingest.persisted))
                    .transpose()?;
                if ingest.cursor_may_advance() {
                    if let Some(cursor) = next_cursor {
                        if let Some((remaining, total)) = backfill_stats(&cursor) {
                            backfill_remaining_messages += remaining;
                            backfill_total_messages += total;
                        }
                        if backfill_pending(&cursor) {
                            has_backfill_pending = true;
                        }
                        state.cursors.insert(folder.id.clone(), cursor);
                    }
                } else {
                    blocked_folders.push(folder.name.clone());
                    blocked_messages += ingest.failed;
                }
                // Healthy POP3 messages may be deleted even while the old
                // cursor is retained for a failing message.
                persist_sync_cursors(
                    pool, &account.id, &state, queue.as_ref(), pop3_delete.as_ref(),
                ).await?;
                if let Some(started_at) = inbox_fetch_started_at.as_deref()
                    .filter(|_| ingest.cursor_may_advance()) {
                    stamp_inbox_fetch(pool, &account.id, started_at).await?;
                }
                if ingest.failed > 0 {
                    summary.failures.push(encoded_quarantined_messages(
                        &account.email,
                        &folder.name,
                        ingest.failed,
                    ));
                }
                if ingest.skipped > 0 {
                    tracing::info!(
                        account_id = %account.id,
                        folder = %folder.name,
                        skipped = ingest.skipped,
                        "quarantined messages were not ingested again"
                    );
                }

                // Defer projection cleanup until the fresh batch has landed.
                // Threads whose real Message-ID survives keep their workflow,
                // tags and other local organization across UIDVALIDITY resets.
                refresh_provider_thread_projections(
                    pool,
                    &reset_thread_ids.iter().cloned().collect::<Vec<_>>(),
                )
                .await?;

                folders_synced += 1;
            }

            if folder_selection == FolderSelection::All {
                externally_changed_threads.extend(
                    reconcile_missing_remote_folders(
                        pool,
                        &account.id,
                        &account.provider,
                        &available_folder_ids,
                        &sync_started_at,
                    )
                    .await?,
                );
            }

            if snapshot_sync_complete {
                for (folder_id, remote_folder_id) in snapshot_folders {
                    match reconcile_folder_snapshot(
                        pool, provider.as_ref(), &account.id, &folder_id,
                        &remote_folder_id, &mut state, queue.as_ref(),
                    ).await {
                        Ok(changed) => externally_changed_threads.extend(changed),
                        Err(err) => summary.failures.push(encoded_folder_sync_failure(
                            &account.email, &remote_folder_id, err,
                        )),
                    }
                }
            }
            if account.provider.eq_ignore_ascii_case("gmail") {
                let removed = repair_gmail_archive_duplicates(pool, &account.id).await?;
                if removed > 0 {
                    tracing::info!(
                        account_id = %account.id,
                        removed,
                        "removed synthetic archive copies of labelled Gmail messages"
                    );
                }
            }
            if account.provider.eq_ignore_ascii_case("imap") {
                let collapsed = deduplicate_cached_draft_autosaves(pool, &account.id).await?;
                if collapsed > 0 {
                    tracing::info!(
                        account_id = %account.id,
                        collapsed,
                        "collapsed cached IMAP draft autosave revisions"
                    );
                }
                let superseded = reconcile_sent_draft_autosaves(pool, &account.id).await?;
                if !superseded.is_empty() {
                    refresh_provider_thread_projections(
                        pool,
                        &superseded.iter().cloned().collect::<Vec<_>>(),
                    )
                    .await?;
                    externally_changed_threads.extend(superseded);
                }
            }
            reconcile_external_workflow_states(pool, &externally_changed_threads).await?;
            if snapshot_sync_complete {
                repair_stale_done_threads_in_inbox(pool, &account.id).await?;
            }
            if let Some(events) = events {
                if !externally_changed_threads.is_empty() {
                    events.publish(AppEvent::ThreadsUpdated(externally_changed_threads.iter().cloned().collect()));
                }
            }

            // Forget cursors for folders the provider no longer returns. A left
            // behind cursor keeps reporting its backfill as pending, and nothing
            // will ever finish it — which pins the frontend to its 5-second
            // catch-up poll for good.
            if folder_selection == FolderSelection::All {
                let retained: HashSet<String> = sqlx::query_scalar(
                    "SELECT COALESCE(remote_id, full_path) FROM folders WHERE account_id = ? AND folder_type != 'local'"
                ).bind(&account.id).fetch_all(pool).await.map_err(|error| error.to_string())?
                    .into_iter().collect();
                state.cursors.retain(|folder_id, _| retained.contains(folder_id));
                state.folder_snapshot_at.retain(|folder_id, _| retained.contains(folder_id));
                state.pending_snapshots.retain(|folder_id| retained.contains(folder_id));
            }

            // The retained cursor set and the snapshot timestamps are the last
            // state change of this run; the status write below no longer carries
            // cursors, they are already durable per folder.
            persist_sync_cursors(pool, &account.id, &state, queue.as_ref(), None).await?;

            if !folder_fetch_failures.is_empty() {
                return Err(folder_fetch_failures.join("; "));
            }

            if !blocked_folders.is_empty() {
                let failure = encoded_folder_ingest_failure(
                    &account.email,
                    &blocked_folders,
                    blocked_messages,
                );
                // Also carried in the summary: the account error wraps this as
                // plain text, and the frontend needs the key and its parameters.
                summary.failures.push(failure.clone());
                return Err(failure);
            }

            let now = chrono::Utc::now().to_rfc3339();
            sqlx::query(
                "UPDATE accounts
                 SET sync_status = 'idle',
                     last_sync_error = NULL,
                     last_sync_error_at = NULL,
                     sync_progress_json = ?,
                     updated_at = ?
                 WHERE id = ?",
            )
                .bind(serde_json::json!({"phase": "complete"}).to_string())
                .bind(&now)
                .bind(&account.id)
                .execute(pool)
                .await
                .map_err(|err| err.to_string())?;

            Ok((
                folders_synced,
                messages_synced,
                has_backfill_pending,
                backfill_remaining_messages,
                backfill_total_messages,
            ))
        }
        .await;

        match account_sync_result {
            Ok((
                folders_synced,
                messages_synced,
                has_backfill_pending,
                backfill_remaining_messages,
                backfill_total_messages,
            )) => {
                summary.accounts_synced += 1;
                summary.folders_synced += folders_synced;
                summary.messages_synced += messages_synced;
                if has_backfill_pending {
                    summary.backfill_pending_accounts += 1;
                }
                summary.backfill_remaining_messages += backfill_remaining_messages;
                summary.backfill_total_messages += backfill_total_messages;
            }
            Err(error) => {
                let now = chrono::Utc::now().to_rfc3339();
                let failure = encoded_account_sync_failure(&account.email, &account.id, &error);
                let _ = sqlx::query(
                    "UPDATE accounts
                     SET sync_status = 'error',
                         last_sync_error = ?,
                         last_sync_error_at = ?,
                         sync_progress_json = ?,
                         updated_at = ?
                     WHERE id = ?",
                )
                .bind(&failure)
                .bind(&now)
                .bind(serde_json::json!({"phase": "error"}).to_string())
                .bind(&now)
                .bind(&account.id)
                .execute(pool)
                .await;
                summary.accounts_failed += 1;
                summary.failures.push(failure);
            }
        }

        #[cfg(target_os = "android")]
        cross_process_lease.release().await;
        Ok(())
        }).await;
        if let Err(error) = result {
            let now = chrono::Utc::now().to_rfc3339();
            let failure = encoded_account_sync_failure(&account_id, &account_id, &error);
            let _ = sqlx::query(
                "UPDATE accounts SET sync_status = 'error', last_sync_error = ?,
                last_sync_error_at = ?, sync_progress_json = ?, updated_at = ? WHERE id = ?",
            )
            .bind(&failure)
            .bind(&now)
            .bind(serde_json::json!({"phase": "error"}).to_string())
            .bind(&now)
            .bind(&account_id)
            .execute(pool)
            .await;
            summary.accounts_failed += 1;
            summary.failures.push(failure);
        }
    }

    // A block earned during this run has to outlive the process, or the next
    // start knocks on it and renews it.
    crate::sync::rate_limit_state::persist(pool).await;

    // Re-publish any durable intents left behind by a process exit between
    // message ingestion and native delivery. The renderer uses the message id,
    // so a concurrent retry updates the same system notification.
    if let Some(events) = events {
        if let Err(error) = crate::commands::notifications::publish_pending_mail_notifications(
            pool,
            events.as_ref(),
            None,
        )
        .await
        {
            tracing::warn!(%error, "pending mail notifications could not be published");
        }
    }

    Ok(summary)
}

/// How much history the user wants kept with its body, if they have said.
///
/// `None` leaves the decision to the account config and the platform default —
/// three months on a phone, everything on a desktop. A stored 0 is the user
/// turning the limit off and has to survive as `Some(0)`, or the fallback would
/// quietly re-impose the very window they switched away from.
pub(crate) async fn stored_body_window_days(pool: &SqlitePool) -> Option<u32> {
    let raw: Option<String> =
        sqlx::query_scalar("SELECT value FROM app_settings WHERE key = 'sync.body_window_days'")
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
    raw?.trim().parse::<u32>().ok()
}

#[cfg(test)]
mod tests {
    use super::{
        encoded_account_sync_failure, ingest_folder_batch, persist_sync_cursors,
        prepare_folders_for_sync, FolderIngestContext, FolderSelection, ImapSyncState,
        MailAccountRow,
    };
    use crate::providers::traits::{MessageEnvelope, RemoteFolder};
    use satchel_core::db::sync_quarantine::MAX_INGEST_ATTEMPTS;
    use sqlx::sqlite::SqlitePoolOptions;
    use std::collections::{HashMap, HashSet};

    #[test]
    fn inbox_scope_keeps_only_inbox_while_full_sync_prioritizes_it() {
        let folders = || {
            vec![
                RemoteFolder {
                    id: "archive".into(),
                    name: "Archive".into(),
                    kind: Some("archive".into()),
                },
                RemoteFolder {
                    id: "inbox-id".into(),
                    name: "Posteingang".into(),
                    kind: Some("inbox".into()),
                },
                RemoteFolder {
                    id: "sent".into(),
                    name: "Gesendet".into(),
                    kind: Some("sent".into()),
                },
            ]
        };

        let mut full = folders();
        prepare_folders_for_sync("outlook", &mut full, FolderSelection::All);
        assert_eq!(full[0].kind.as_deref(), Some("inbox"));
        assert_eq!(full.len(), 3);

        let mut inbox = folders();
        prepare_folders_for_sync("outlook", &mut inbox, FolderSelection::Inbox);
        assert_eq!(inbox.len(), 1);
        assert_eq!(inbox[0].kind.as_deref(), Some("inbox"));
    }

    async fn sync_pool() -> sqlx::SqlitePool {
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
        for (id, name) in [("inbox", "INBOX"), ("archive", "Archive")] {
            sqlx::query(
                "INSERT INTO folders
                 (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at)
                 VALUES (?, 'account', ?, ?, 'inbox', ?, '', '')",
            )
            .bind(id)
            .bind(name)
            .bind(name)
            .bind(name)
            .execute(&pool)
            .await
            .expect("folder");
        }
        pool
    }

    fn account_row() -> MailAccountRow {
        MailAccountRow {
            id: "account".into(),
            display_name: "Account".into(),
            email: "account@example.com".into(),
            provider: "imap".into(),
            config: "{}".into(),
            sync_state: None,
            outlook_immutable_ids_migrated: 1,
            ingest_cutoff: None,
        }
    }

    fn envelope(remote_id: &str) -> MessageEnvelope {
        serde_json::from_value(serde_json::json!({
            "remote_id": remote_id,
            "remote_thread_id": null,
            "subject": "Subject",
            "snippet": "",
            "from_address": "sender@example.com",
            "from_name": null,
            "to_addresses": [],
            "cc_addresses": [],
            "date_rfc3339": "2026-01-01T00:00:00Z",
            "body_text": null,
            "body_html": null,
        }))
        .expect("envelope")
    }

    async fn ingest(
        pool: &sqlx::SqlitePool,
        account: &MailAccountRow,
        folder_id: &str,
        messages: Vec<MessageEnvelope>,
    ) -> super::FolderIngestOutcome {
        let mut externally_changed_threads = HashSet::new();
        ingest_folder_batch(
            pool,
            &FolderIngestContext {
                app: None,
                events: None,
                account,
                config: &crate::commands::accounts::AccountConfig::from_plaintext("{}"),
                folder_id,
                folder_remote_id: "INBOX",
                folder_type: "inbox",
            },
            messages,
            &mut externally_changed_threads,
        )
        .await
        .expect("ingest")
    }

    async fn stored_cursors(pool: &sqlx::SqlitePool) -> HashMap<String, String> {
        let raw: Option<String> = sqlx::query_scalar("SELECT sync_state FROM accounts")
            .fetch_one(pool)
            .await
            .expect("sync state");
        serde_json::from_str::<ImapSyncState>(raw.as_deref().unwrap_or("{}"))
            .expect("sync state json")
            .cursors
    }

    /// One unusable message used to abort the account before any cursor was
    /// written — including the cursors of every folder that had synced fine.
    #[tokio::test]
    async fn a_failing_message_does_not_discard_other_folder_cursors() {
        let pool = sync_pool().await;
        let account = account_row();
        let mut state = ImapSyncState::default();

        let healthy = ingest(&pool, &account, "inbox", vec![envelope("1")]).await;
        assert_eq!(healthy.ingested, 1);
        assert!(healthy.cursor_may_advance());
        state.cursors.insert("INBOX".into(), "cursor-1".into());
        persist_sync_cursors(
            &pool,
            &account.id,
            &state,
            &crate::sync::queue::SyncQueue::new(),
            None,
        )
        .await
        .expect("persist cursor");

        // Second folder: its row is missing, so the message insert trips the
        // foreign key on `folder_id`.
        let broken = ingest(&pool, &account, "missing-folder", vec![envelope("2")]).await;
        assert_eq!(broken.ingested, 0);
        assert_eq!(broken.failed, 1);
        assert!(!broken.cursor_may_advance());

        assert_eq!(
            stored_cursors(&pool).await.get("INBOX").map(String::as_str),
            Some("cursor-1")
        );
        let quarantined: Vec<(String, i64)> =
            sqlx::query_as("SELECT remote_id, attempts FROM sync_quarantine")
                .fetch_all(&pool)
                .await
                .expect("quarantine");
        assert_eq!(quarantined, vec![("2".to_string(), 1)]);
    }

    #[tokio::test]
    async fn an_exhausted_quarantine_entry_is_skipped_and_the_cursor_advances() {
        let pool = sync_pool().await;
        let account = account_row();
        sqlx::query(
            "INSERT INTO sync_quarantine
             (account_id, folder_id, remote_id, attempts, last_error, first_failed_at, last_failed_at)
             VALUES ('account', 'missing-folder', '2', ?, 'broken', '', '')",
        )
        .bind(MAX_INGEST_ATTEMPTS)
        .execute(&pool)
        .await
        .expect("quarantine seed");

        let outcome = ingest(&pool, &account, "missing-folder", vec![envelope("2")]).await;

        assert_eq!(outcome.skipped, 1);
        assert_eq!(outcome.failed, 0);
        assert!(outcome.cursor_may_advance());
        let attempts: i64 = sqlx::query_scalar("SELECT attempts FROM sync_quarantine")
            .fetch_one(&pool)
            .await
            .expect("attempts");
        assert_eq!(attempts, MAX_INGEST_ATTEMPTS);
    }

    #[tokio::test]
    async fn a_batch_with_unexhausted_failures_keeps_its_cursor() {
        let pool = sync_pool().await;
        let account = account_row();
        sqlx::query(
            "CREATE TRIGGER reject_broken_messages BEFORE INSERT ON messages
             WHEN new.remote_id LIKE 'broken-%'
             BEGIN SELECT RAISE(ABORT, 'message rejected'); END",
        )
        .execute(&pool)
        .await
        .expect("trigger");
        let messages = (0..10)
            .map(|index| {
                envelope(&if index < 8 {
                    format!("broken-{index}")
                } else {
                    format!("{index}")
                })
            })
            .collect::<Vec<_>>();

        let outcome = ingest(&pool, &account, "inbox", messages).await;

        assert_eq!(outcome.ingested, 2);
        assert_eq!(outcome.failed, 8);
        assert!(!outcome.cursor_may_advance());
    }

    #[tokio::test]
    async fn one_failed_message_in_a_small_batch_is_retried_until_exhausted() {
        let server = Pop3Fake::spawn(vec![(1, "uidl-1"), (2, "broken-2"), (3, "uidl-3")]).await;
        let pool = pop3_pool(server.port, true).await;
        sqlx::query(
            "CREATE TRIGGER reject_broken_messages BEFORE INSERT ON messages
            WHEN new.remote_id = 'broken-2' BEGIN SELECT RAISE(ABORT, 'busy'); END",
        )
        .execute(&pool)
        .await
        .unwrap();
        for attempt in 1..=MAX_INGEST_ATTEMPTS {
            sync_pop3_account(&pool).await;
            let attempts: i64 = sqlx::query_scalar(
                "SELECT attempts FROM sync_quarantine WHERE remote_id = 'broken-2'",
            )
            .fetch_one(&pool)
            .await
            .unwrap();
            assert_eq!(attempts, attempt);
        }
        sync_pop3_account(&pool).await;
        let attempts: i64 =
            sqlx::query_scalar("SELECT attempts FROM sync_quarantine WHERE remote_id = 'broken-2'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(attempts, MAX_INGEST_ATTEMPTS);
        assert!(!stored_cursors(&pool).await.is_empty());
    }

    #[tokio::test]
    async fn a_failed_inbox_fetch_keeps_the_notification_cutoff() {
        let server = Pop3Fake::spawn(vec![(1, "fail-listing")]).await;
        let pool = pop3_pool(server.port, true).await;
        let cutoff = "2026-01-01T00:00:00Z";
        sqlx::query("UPDATE accounts SET last_sync_at = ?")
            .bind(cutoff)
            .execute(&pool)
            .await
            .unwrap();
        let summary = sync_pop3_account(&pool).await;
        assert!(!summary.failures.is_empty());
        assert_eq!(summary.accounts_failed, 1);
        let actual: String = sqlx::query_scalar("SELECT last_sync_at FROM accounts")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(actual, cutoff);
    }

    #[tokio::test]
    async fn account_deadline_releases_the_run_lock() {
        let lock = crate::sync::leases::sync_run_lock("deadline-test");
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            super::account_run_with_deadline(std::time::Duration::from_millis(50), async {
                let _guard = lock.lock().await;
                std::future::pending::<Result<(), String>>().await
            }),
        )
        .await;
        assert!(result.is_ok(), "account run escaped its deadline");
        assert!(result.unwrap().unwrap_err().contains("timed out"));
        assert!(lock.try_lock().is_ok());
    }

    #[tokio::test]
    async fn waiting_for_the_account_lock_does_not_count_against_the_deadline() {
        let lock = crate::sync::leases::sync_run_lock("deadline-wait-test");
        let held = lock.clone().lock_owned().await;
        let release = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
            drop(held);
        });
        let result = super::account_run_exclusive(
            "deadline-wait-test",
            std::time::Duration::from_millis(50),
            async { Ok::<_, String>("ran") },
        )
        .await;
        release.await.unwrap();
        assert_eq!(result, Ok("ran"));
    }

    #[tokio::test]
    async fn receive_time_filters_notifications_without_changing_the_displayed_date() {
        let pool = sync_pool().await;
        let account = account_row();
        let now = chrono::Utc::now();
        let cutoff = (now - chrono::TimeDelta::minutes(5)).to_rfc3339();
        sqlx::query("UPDATE accounts SET last_sync_at = ?")
            .bind(cutoff)
            .execute(&pool)
            .await
            .unwrap();
        let mut message = envelope("received");
        message.date_rfc3339 = (now - chrono::TimeDelta::minutes(10)).to_rfc3339();
        message.received_at_rfc3339 = Some(now.to_rfc3339());
        let display_date = message.date_rfc3339.clone();
        let outcome = ingest(&pool, &account, "inbox", vec![message]).await;
        assert_eq!(outcome.ingested, 1);
        let (date, received_at, state): (String, String, String) = sqlx::query_as(
            "SELECT m.date, m.received_at, o.state FROM messages m JOIN notification_outbox o ON o.message_id = m.id")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(date, display_date);
        assert_eq!(received_at, now.to_rfc3339());
        assert_eq!(state, "pending");
    }

    #[tokio::test]
    async fn successful_inbox_fetch_uses_its_start_time_as_the_cutoff() {
        let pool = sync_pool().await;
        let started_at = "2026-10-12T10:00:00Z";
        super::stamp_inbox_fetch(&pool, "account", started_at)
            .await
            .unwrap();
        let actual: String =
            sqlx::query_scalar("SELECT last_sync_at FROM accounts WHERE id = 'account'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(actual, started_at);
    }

    #[tokio::test]
    async fn a_transient_ingest_failure_is_stored_on_the_next_run() {
        let server = Pop3Fake::spawn(vec![(1, "uidl-1"), (2, "broken-2"), (3, "uidl-3")]).await;
        let pool = pop3_pool(server.port, true).await;
        sqlx::query(
            "CREATE TRIGGER reject_one_message BEFORE INSERT ON messages
            WHEN new.remote_id = 'broken-2' BEGIN SELECT RAISE(ABORT, 'busy'); END",
        )
        .execute(&pool)
        .await
        .unwrap();
        let first = sync_pop3_account(&pool).await;
        assert_eq!(first.accounts_failed, 1);
        assert!(stored_cursors(&pool).await.is_empty());
        sqlx::query("DROP TRIGGER reject_one_message")
            .execute(&pool)
            .await
            .unwrap();
        let second = sync_pop3_account(&pool).await;
        assert_eq!(second.accounts_failed, 0);
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 3);
        let quarantined: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_quarantine")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(quarantined, 0);
        assert!(!stored_cursors(&pool).await.is_empty());
    }

    #[tokio::test]
    async fn reset_retries_do_not_keep_a_cursor_from_the_previous_generation() {
        let pool = sync_pool().await;
        let mut state = ImapSyncState::default();
        state
            .cursors
            .insert("INBOX".into(), "old-generation".into());
        state
            .cursors
            .insert("Archive".into(), "archive-cursor".into());
        state
            .folder_snapshot_at
            .insert("INBOX".into(), "old-snapshot".into());
        let quarantine = satchel_core::db::sync_quarantine::SyncQuarantineRepository::new(&pool);
        quarantine
            .record_failure("account", "inbox", "42", "busy")
            .await
            .unwrap();
        let queue = super::SyncQueue::new();
        super::reset_folder_checkpoint(&pool, "account", "inbox", "INBOX", &mut state, &queue)
            .await
            .unwrap();
        let stored = stored_cursors(&pool).await;
        assert!(!stored.contains_key("INBOX"));
        assert_eq!(
            stored.get("Archive").map(String::as_str),
            Some("archive-cursor")
        );
        assert!(!state.folder_snapshot_at.contains_key("INBOX"));
        assert!(!quarantine
            .load_folder("account", "inbox")
            .await
            .unwrap()
            .contains("42"));
    }

    #[tokio::test]
    async fn one_failed_savepoint_keeps_the_cursor_until_a_retry_succeeds() {
        let pool = sync_pool().await;
        let account = account_row();
        sqlx::query("CREATE TRIGGER reject_one BEFORE INSERT ON messages WHEN new.remote_id = 'broken' BEGIN SELECT RAISE(ABORT, 'busy'); END").execute(&pool).await.unwrap();
        let first = ingest(
            &pool,
            &account,
            "inbox",
            vec![
                envelope("healthy-1"),
                envelope("broken"),
                envelope("healthy-2"),
            ],
        )
        .await;
        assert_eq!(first.ingested, 2);
        assert!(!first.cursor_may_advance());
        sqlx::query("DROP TRIGGER reject_one")
            .execute(&pool)
            .await
            .unwrap();
        let second = ingest(
            &pool,
            &account,
            "inbox",
            vec![
                envelope("healthy-1"),
                envelope("broken"),
                envelope("healthy-2"),
            ],
        )
        .await;
        assert!(second.cursor_may_advance());
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM messages")
                .fetch_one(&pool)
                .await
                .unwrap(),
            3
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_quarantine")
                .fetch_one(&pool)
                .await
                .unwrap(),
            0
        );
    }

    /// A scriptable POP3 server that records every command it is sent.
    ///
    /// Deletion is the one POP3 operation that cannot be taken back, so the
    /// tests below assert on the wire rather than on our own bookkeeping.
    struct Pop3Fake {
        port: u16,
        commands: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    }

    impl Pop3Fake {
        async fn spawn(messages: Vec<(u32, &'static str)>) -> Self {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind");
            let port = listener.local_addr().expect("address").port();
            let commands = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let log = commands.clone();
            tokio::spawn(async move {
                while let Ok((socket, _)) = listener.accept().await {
                    let log = log.clone();
                    let messages = messages.clone();
                    tokio::spawn(async move { serve_pop3(socket, messages, log).await });
                }
            });
            Self { port, commands }
        }

        fn commands(&self) -> Vec<String> {
            self.commands.lock().expect("commands").clone()
        }
    }

    async fn serve_pop3(
        socket: tokio::net::TcpStream,
        messages: Vec<(u32, &'static str)>,
        log: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    ) {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

        let (reader, mut writer) = socket.into_split();
        let mut reader = tokio::io::BufReader::new(reader);
        if writer.write_all(b"+OK fake ready\r\n").await.is_err() {
            return;
        }
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line).await {
                Ok(0) | Err(_) => return,
                Ok(_) => {}
            }
            let command = line.trim().to_string();
            log.lock().expect("commands").push(command.clone());
            if command.starts_with("UIDL") && messages.iter().any(|(_, id)| *id == "fail-listing") {
                return;
            }
            let response = if command.starts_with("UIDL") {
                let mut body = String::from("+OK listing\r\n");
                for (number, uidl) in &messages {
                    body.push_str(&format!("{number} {uidl}\r\n"));
                }
                body.push_str(".\r\n");
                body
            } else if let Some(number) = command.strip_prefix("RETR ") {
                match number.trim().parse::<u32>().ok().and_then(|number| {
                    messages
                        .iter()
                        .find(|(candidate, _)| *candidate == number)
                        .map(|(_, uidl)| *uidl)
                }) {
                    Some(uidl) => format!(
                        "+OK message\r\nMessage-ID: <{uidl}@example.test>\r\n\
                         From: Sender <sender@example.test>\r\n\
                         To: me@example.test\r\n\
                         Subject: {uidl}\r\n\
                         Date: Thu, 01 Jan 2026 00:00:00 +0000\r\n\r\nBody\r\n.\r\n"
                    ),
                    None => "-ERR no such message\r\n".to_string(),
                }
            } else {
                "+OK\r\n".to_string()
            };
            if writer.write_all(response.as_bytes()).await.is_err() {
                return;
            }
            if command.starts_with("QUIT") {
                return;
            }
        }
    }

    async fn pop3_pool(port: u16, leave_on_server: bool) -> sqlx::SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("pool");
        sqlx::migrate!("../crates/satchel-core/src/db/migrations")
            .run(&pool)
            .await
            .expect("migrations");
        let config = serde_json::json!({
            "pop3": {
                "host": "127.0.0.1",
                "port": port,
                "tls": false,
                "username": "me@example.test",
                "password": "secret",
                "leave_on_server": leave_on_server,
            }
        })
        .to_string();
        satchel_core::utils::crypto::seed_master_key_for_tests();
        let config =
            crate::commands::accounts::encrypt_account_config("pop3-account", &config).unwrap();
        sqlx::query(
            "INSERT INTO accounts
             (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
             VALUES ('pop3-account', 'Pop', 'me@example.test', 'pop3', ?, '', '', '')",
        )
        .bind(config)
        .execute(&pool)
        .await
        .expect("account");
        pool
    }

    async fn sync_pop3_account(pool: &sqlx::SqlitePool) -> super::MailboxSyncSummary {
        super::sync_mailboxes(
            pool,
            super::MailboxSyncRequest {
                events: None,
                app: None,
                accounts: super::AccountSelection::One("pop3-account"),
                folders: FolderSelection::All,
            },
        )
        .await
        .expect("sync")
    }

    async fn queued_delete_refs(pool: &sqlx::SqlitePool) -> Vec<Vec<String>> {
        let payloads: Vec<String> =
            sqlx::query_scalar("SELECT payload FROM sync_queue WHERE action_type = ?")
                .bind(super::ACTION_REMOTE_MUTATION)
                .fetch_all(pool)
                .await
                .expect("queue rows");
        payloads
            .iter()
            .filter_map(|payload| {
                match serde_json::from_str::<super::RemoteMutationPayload>(payload).ok()? {
                    super::RemoteMutationPayload::Delete { refs } => Some(
                        refs.into_iter()
                            .map(|message| message.remote_id)
                            .collect::<Vec<_>>(),
                    ),
                    _ => None,
                }
            })
            .collect()
    }

    /// The fetch session must never delete: only the ingest knows whether a
    /// message survived the trip into SQLite.
    #[tokio::test]
    async fn pop3_deletes_only_what_was_persisted() {
        let server = Pop3Fake::spawn(vec![(1, "uidl-1"), (2, "broken-2"), (3, "uidl-3")]).await;
        let pool = pop3_pool(server.port, false).await;
        sqlx::query(
            "CREATE TRIGGER reject_broken_messages BEFORE INSERT ON messages
             WHEN new.remote_id LIKE 'broken-%'
             BEGIN SELECT RAISE(ABORT, 'message rejected'); END",
        )
        .execute(&pool)
        .await
        .expect("trigger");

        sync_pop3_account(&pool).await;

        assert!(!server
            .commands()
            .iter()
            .any(|command| command.starts_with("DELE")));
        assert_eq!(
            queued_delete_refs(&pool).await,
            vec![vec!["uidl-1".to_string(), "uidl-3".to_string()]]
        );
        let quarantined: Vec<String> = sqlx::query_scalar("SELECT remote_id FROM sync_quarantine")
            .fetch_all(&pool)
            .await
            .expect("quarantine");
        assert_eq!(quarantined, vec!["broken-2".to_string()]);
    }

    #[tokio::test]
    async fn pop3_leaves_mail_alone_when_configured() {
        let server = Pop3Fake::spawn(vec![(1, "uidl-1")]).await;
        let pool = pop3_pool(server.port, true).await;

        sync_pop3_account(&pool).await;

        assert!(queued_delete_refs(&pool).await.is_empty());
        assert!(!server
            .commands()
            .iter()
            .any(|command| command.starts_with("DELE")));
    }

    /// The queue row and the cursor are one write. Losing it means the mail is
    /// fetched again, which is harmless; keeping half of it would delete mail
    /// nobody can prove arrived.
    #[tokio::test]
    async fn pop3_delete_and_cursor_commit_together() {
        let server = Pop3Fake::spawn(vec![(1, "uidl-1"), (2, "uidl-2")]).await;
        let pool = pop3_pool(server.port, false).await;
        sqlx::query(
            "CREATE TRIGGER reject_remote_mutations BEFORE INSERT ON sync_queue
             WHEN new.action_type = 'remote_mutation'
             BEGIN SELECT RAISE(ABORT, 'queue rejected'); END",
        )
        .execute(&pool)
        .await
        .expect("trigger");

        let summary = super::sync_mailboxes(
            &pool,
            super::MailboxSyncRequest {
                events: None,
                app: None,
                accounts: super::AccountSelection::One("pop3-account"),
                folders: FolderSelection::All,
            },
        )
        .await
        .expect("sync");
        assert_eq!(summary.accounts_failed, 1);
        assert!(queued_delete_refs(&pool).await.is_empty());
        assert!(stored_cursors(&pool).await.is_empty());

        sqlx::query("DROP TRIGGER reject_remote_mutations")
            .execute(&pool)
            .await
            .expect("drop trigger");
        sync_pop3_account(&pool).await;

        let retrieved = server
            .commands()
            .iter()
            .filter(|command| command.starts_with("RETR "))
            .count();
        assert_eq!(retrieved, 4);
        assert_eq!(
            queued_delete_refs(&pool).await,
            vec![vec!["uidl-1".to_string(), "uidl-2".to_string()]]
        );
    }

    #[test]
    fn account_failures_cross_the_ui_boundary_as_message_keys() {
        let failure =
            encoded_account_sync_failure("person@example.test", "account-1", "provider refused");

        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&failure).unwrap(),
            serde_json::json!({
                "key": "backend.errors.accountSyncFailed",
                "params": {
                    "account": "person@example.test",
                    "accountId": "account-1",
                    "message": "provider refused"
                }
            })
        );
    }
}

#[cfg(test)]
#[path = "mailbox_sync/consistency_tests.rs"]
mod consistency_tests;
