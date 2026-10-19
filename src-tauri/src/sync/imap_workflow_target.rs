//! IMAP workflow-move machinery: executing a queued move against the real
//! IMAP target folder, then verifying and persisting the projection.
//!
//! Entry points are crate-visible because the queue worker, workflow commands,
//! and focused regression tests drive pieces of this pipeline directly.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use sqlx::SqlitePool;
use tauri::AppHandle;

use crate::providers::cursor::backfill_pending;
use crate::providers::{
    provider_for_account,
    traits::{
        MailProvider, MessageEnvelope, MessageRef, RemoteMessageRemoval, SyncBatch, SyncCheckpoint,
    },
};
use crate::AppState;

use super::folder_reconcile::{
    reconcile_imap_folder_snapshot, reconcile_remote_message_removals, reset_local_provider_folder,
};
use super::mailbox_sync::stored_body_window_days;
use super::message_ingest::upsert_provider_message;
use super::thread_resolution::refresh_provider_thread_projection;
use super::workflow_executor::set_workflow_payload_field;

pub(crate) async fn snapshot_exact_imap_target_ids(
    pool: &SqlitePool,
    account_id: &str,
    target_remote_folder_id: &str,
) -> crate::sync::errors::SyncResult<(Vec<String>, u32)> {
    let (email, encrypted_config): (String, String) = sqlx::query_as(
        "SELECT email, config FROM accounts
         WHERE id = ? AND provider = 'imap' AND is_active = 1 LIMIT 1",
    )
    .bind(account_id)
    .fetch_one(pool)
    .await
    .map_err(|err| format!("IMAP workflow account lookup failed: {err}"))?;
    let config = crate::commands::accounts::decrypt_account_config(account_id, &encrypted_config)?;
    let provider =
        provider_for_account("imap", &config, &email, stored_body_window_days(pool).await)?;
    let snapshot = provider
        .sync_message_state_snapshot(target_remote_folder_id)
        .await?
        .ok_or_else(|| "IMAP target folder did not provide a UID snapshot".to_string())?;
    let uidvalidity = snapshot.remote_uidvalidity.ok_or_else(|| {
        "IMAP target folder did not provide UIDVALIDITY for its UID snapshot".to_string()
    })?;
    let mut ids = snapshot
        .states
        .into_iter()
        .map(|state| state.remote_id)
        .collect::<Vec<_>>();
    ids.sort();
    ids.dedup();
    Ok((ids, uidvalidity))
}

/// Return only source refs that still exist after an interrupted IMAP MOVE.
/// UIDs are checked inside their original mailbox and UIDVALIDITY generation,
/// so a retry cannot accidentally apply a stale UID to another message.
pub(crate) async fn present_imap_workflow_sources(
    pool: &SqlitePool,
    account_id: &str,
    refs: &[MessageRef],
    expected_generations: &HashMap<String, u32>,
) -> crate::sync::errors::SyncResult<Vec<MessageRef>> {
    let (email, encrypted_config): (String, String) = sqlx::query_as(
        "SELECT email, config FROM accounts
         WHERE id = ? AND provider = 'imap' AND is_active = 1 LIMIT 1",
    )
    .bind(account_id)
    .fetch_one(pool)
    .await
    .map_err(|err| format!("IMAP workflow account lookup failed: {err}"))?;
    let config = crate::commands::accounts::decrypt_account_config(account_id, &encrypted_config)?;
    let provider =
        provider_for_account("imap", &config, &email, stored_body_window_days(pool).await)?;
    let mut refs_by_folder: HashMap<&str, Vec<&MessageRef>> = HashMap::new();
    for message in refs {
        let folder = message
            .source_folder
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("IMAP source folder is missing for {}", message.remote_id))?;
        refs_by_folder.entry(folder).or_default().push(message);
    }

    let mut remaining = Vec::new();
    for (folder, folder_refs) in refs_by_folder {
        let snapshot = provider
            .sync_message_state_snapshot(folder)
            .await?
            .ok_or_else(|| {
                format!("IMAP source folder `{folder}` did not provide a UID snapshot")
            })?;
        let current_generation = snapshot
            .remote_uidvalidity
            .ok_or_else(|| format!("IMAP source folder `{folder}` did not provide UIDVALIDITY"))?;
        if expected_generations.get(folder) != Some(&current_generation) {
            return Err(format!(
                "IMAP source folder `{folder}` changed UIDVALIDITY; refusing stale MOVE retry"
            )
            .into());
        }
        let present = snapshot
            .states
            .into_iter()
            .map(|state| state.remote_id)
            .collect::<HashSet<_>>();
        remaining.extend(
            folder_refs
                .into_iter()
                .filter(|message| present.contains(&message.remote_id))
                .cloned(),
        );
    }
    Ok(remaining)
}

/// Reconcile a legacy started move that has no persisted UIDVALIDITY. A
/// source UID that is still present is ambiguous and must not be submitted;
/// only source absence is safe to conclude from this weaker snapshot.
pub(crate) async fn present_imap_sources_without_generation(
    pool: &SqlitePool,
    account_id: &str,
    refs: &[MessageRef],
) -> crate::sync::errors::SyncResult<Vec<MessageRef>> {
    let (email, encrypted_config): (String, String) = sqlx::query_as(
        "SELECT email, config FROM accounts
         WHERE id = ? AND provider = 'imap' AND is_active = 1 LIMIT 1",
    )
    .bind(account_id)
    .fetch_one(pool)
    .await
    .map_err(|err| format!("IMAP workflow account lookup failed: {err}"))?;
    let config = crate::commands::accounts::decrypt_account_config(account_id, &encrypted_config)?;
    let provider =
        provider_for_account("imap", &config, &email, stored_body_window_days(pool).await)?;
    let mut refs_by_folder: HashMap<&str, Vec<&MessageRef>> = HashMap::new();
    for message in refs {
        let folder = message
            .source_folder
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("IMAP source folder is missing for {}", message.remote_id))?;
        refs_by_folder.entry(folder).or_default().push(message);
    }

    let mut remaining = Vec::new();
    for (folder, folder_refs) in refs_by_folder {
        let snapshot = provider
            .sync_message_state_snapshot(folder)
            .await?
            .ok_or_else(|| {
                format!("IMAP source folder `{folder}` did not provide a UID snapshot")
            })?;
        let present = snapshot
            .states
            .into_iter()
            .map(|state| state.remote_id)
            .collect::<HashSet<_>>();
        remaining.extend(
            folder_refs
                .into_iter()
                .filter(|message| present.contains(&message.remote_id))
                .cloned(),
        );
    }
    Ok(remaining)
}

pub(crate) async fn snapshot_imap_source_generations(
    pool: &SqlitePool,
    account_id: &str,
    refs: &[MessageRef],
) -> crate::sync::errors::SyncResult<HashMap<String, u32>> {
    let (email, encrypted_config): (String, String) = sqlx::query_as(
        "SELECT email, config FROM accounts
         WHERE id = ? AND provider = 'imap' AND is_active = 1 LIMIT 1",
    )
    .bind(account_id)
    .fetch_one(pool)
    .await
    .map_err(|err| format!("IMAP workflow account lookup failed: {err}"))?;
    let config = crate::commands::accounts::decrypt_account_config(account_id, &encrypted_config)?;
    let provider =
        provider_for_account("imap", &config, &email, stored_body_window_days(pool).await)?;
    let folders = refs
        .iter()
        .map(|message| {
            message
                .source_folder
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| format!("IMAP source folder is missing for {}", message.remote_id))
        })
        .collect::<Result<HashSet<_>, _>>()?;
    let mut generations = HashMap::new();
    for folder in folders {
        let snapshot = provider
            .sync_message_state_snapshot(folder)
            .await?
            .ok_or_else(|| {
                format!("IMAP source folder `{folder}` did not provide a UID snapshot")
            })?;
        let generation = snapshot
            .remote_uidvalidity
            .ok_or_else(|| format!("IMAP source folder `{folder}` did not provide UIDVALIDITY"))?;
        generations.insert(folder.to_string(), generation);
    }
    Ok(generations)
}

/// Refuse a not-yet-started blocked move when its folder-scoped UID generation
/// no longer matches the selected IMAP mailboxes. The persisted generation is
/// part of the immutable operation identity, so a mismatch must fail rather
/// than risk applying the UID to a different message after rollover.
pub(crate) async fn validate_imap_source_generations(
    pool: &SqlitePool,
    account_id: &str,
    refs: &[MessageRef],
    expected_generations: &HashMap<String, u32>,
) -> crate::sync::errors::SyncResult<()> {
    let current_generations = snapshot_imap_source_generations(pool, account_id, refs).await?;
    if !source_generations_match(refs, expected_generations, &current_generations) {
        return Err("IMAP source folder UIDVALIDITY changed; refusing stale blocked move".into());
    }
    Ok(())
}

pub(crate) fn source_generations_match(
    refs: &[MessageRef],
    expected_generations: &HashMap<String, u32>,
    current_generations: &HashMap<String, u32>,
) -> bool {
    refs.iter().all(|message| {
        let Some(folder) = message
            .source_folder
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return false;
        };
        let Some(message_generation) = message.source_uidvalidity else {
            return false;
        };
        expected_generations.get(folder) == Some(&message_generation)
            && current_generations.get(folder) == Some(&message_generation)
    })
}

/// IMAP UIDs are mailbox-local. Keep the source projection in place while the
/// exact destination mailbox is fetched, then remove it only after the moved
/// message has landed locally with its new destination UID.
pub(crate) async fn reconcile_exact_imap_workflow_target(
    pool: &SqlitePool,
    app: Option<&AppHandle>,
    state: &AppState,
    account_id: &str,
    thread_id: &str,
    refs: &[MessageRef],
    target_folder_type: &str,
    target_remote_folder_id: &str,
    target_remote_ids_before: &[String],
    target_uidvalidity_before: u32,
    source_uidvalidity_before: &HashMap<String, u32>,
    action_id: &str,
    completed_payload: &serde_json::Value,
) -> crate::sync::errors::SyncResult<()> {
    let (display_name, email, encrypted_config): (String, String, String) = sqlx::query_as(
        "SELECT display_name, email, config FROM accounts
         WHERE id = ? AND provider = 'imap' AND is_active = 1 LIMIT 1",
    )
    .bind(account_id)
    .fetch_one(pool)
    .await
    .map_err(|err| format!("IMAP workflow account lookup failed: {err}"))?;
    let config = crate::commands::accounts::decrypt_account_config(account_id, &encrypted_config)?;
    let provider =
        provider_for_account("imap", &config, &email, stored_body_window_days(pool).await)?;
    let local_folder_id = crate::sync::remote_mutations::ensure_local_workflow_target_folder(
        pool,
        account_id,
        target_folder_type,
        target_remote_folder_id,
    )
    .await?;

    // No regular account filter is involved here: Done must fetch Archive even
    // when the user's ordinary IMAP mode intentionally syncs only Inbox/Aside.
    let initial_target_snapshot = provider
        .sync_message_state_snapshot(target_remote_folder_id)
        .await?
        .ok_or_else(|| "IMAP target folder did not provide a UID snapshot".to_string())?;
    let current_target_uidvalidity =
        initial_target_snapshot.remote_uidvalidity.ok_or_else(|| {
            "IMAP target folder did not provide UIDVALIDITY for reconciliation".to_string()
        })?;
    let generation_changed = current_target_uidvalidity != target_uidvalidity_before;
    let (complete_batch, target_snapshot) = if generation_changed {
        let provider_for_fetch = Arc::clone(&provider);
        let folder_for_fetch = target_remote_folder_id.to_string();
        let complete_batch = fetch_complete_imap_target(move |checkpoint| {
            let provider = Arc::clone(&provider_for_fetch);
            let folder = folder_for_fetch.clone();
            async move { provider.sync_messages(&folder, checkpoint).await }
        })
        .await?;
        let final_snapshot = provider
            .sync_message_state_snapshot(target_remote_folder_id)
            .await?
            .ok_or_else(|| "IMAP target folder did not provide a final UID snapshot".to_string())?;
        if final_snapshot.remote_uidvalidity != Some(current_target_uidvalidity) {
            return Err("IMAP target generation changed during its full rebuild".into());
        }
        (complete_batch, final_snapshot)
    } else {
        let new_remote_ids = proven_new_imap_target_ids(
            target_uidvalidity_before,
            current_target_uidvalidity,
            target_remote_ids_before,
            &initial_target_snapshot.states,
        );
        let batch = fetch_new_imap_target_messages(
            new_remote_ids,
            current_target_uidvalidity,
            |remote_ids| {
                let provider = Arc::clone(&provider);
                let folder = target_remote_folder_id.to_string();
                async move {
                    provider
                        .fetch_messages_by_remote_ids(&folder, &remote_ids)
                        .await
                }
            },
            || {
                let provider = Arc::clone(&provider);
                let folder = target_remote_folder_id.to_string();
                async move {
                    provider
                        .sync_messages(&folder, SyncCheckpoint { cursor: None })
                        .await
                }
            },
        )
        .await?;
        (batch, initial_target_snapshot)
    };
    let reset_threads = validate_generation_then_reset_imap_target(
        pool,
        account_id,
        &local_folder_id,
        &complete_batch.messages,
        current_target_uidvalidity,
        complete_batch.reset_folder || generation_changed,
    )
    .await?;
    reconcile_remote_message_removals(pool, account_id, &complete_batch.removals).await?;
    for message in complete_batch.messages {
        upsert_provider_message(
            app,
            pool,
            "imap",
            account_id,
            &email,
            &display_name,
            &config,
            &local_folder_id,
            target_remote_folder_id,
            target_folder_type,
            &message,
            Some(&state.events),
        )
        .await?;
    }
    // The batch above proved every message carries this generation. Recording
    // it lets the snapshot reconciliation below tell these UIDs apart from a
    // later generation's instead of reading a stale or empty value.
    sqlx::query("UPDATE folders SET uidvalidity = ?, updated_at = ? WHERE id = ?")
        .bind(i64::from(current_target_uidvalidity))
        .bind(chrono::Utc::now().to_rfc3339())
        .bind(&local_folder_id)
        .execute(pool)
        .await
        .map_err(|err| err.to_string())?;

    let states = target_snapshot.states;
    let new_target_remote_ids = proven_new_imap_target_ids(
        target_uidvalidity_before,
        current_target_uidvalidity,
        target_remote_ids_before,
        &states,
    );
    let snapshot_changes = reconcile_imap_folder_snapshot(
        pool,
        account_id,
        &local_folder_id,
        target_remote_folder_id,
        &states,
        Some(current_target_uidvalidity),
    )
    .await?;
    for affected_thread_id in reset_threads
        .iter()
        .chain(snapshot_changes.relocated_threads.iter())
    {
        refresh_provider_thread_projection(pool, affected_thread_id).await?;
    }

    let target_result = finalize_persisted_imap_workflow_target_projection(
        pool,
        account_id,
        thread_id,
        refs,
        &local_folder_id,
        new_target_remote_ids.as_ref(),
        action_id,
        completed_payload,
    )
    .await;
    let Err(target_error) = target_result else {
        return Ok(());
    };
    if !target_error.contains("IMAP target reconciliation did not find moved message") {
        return Err(target_error.into());
    }

    // Some IMAP servers accept MOVE but do not expose the destination copy in
    // the first reconciliation fetch. Once the original UIDs are definitely
    // absent from every source mailbox, keeping their local Inbox projection is
    // less correct than completing the already accepted operation. A missing
    // snapshot or a still-present source remains an error and is retried.
    if !imap_workflow_sources_are_absent(provider.as_ref(), refs, source_uidvalidity_before).await?
    {
        return Err(target_error.into());
    }
    tracing::warn!(
        action_id,
        thread_id,
        "IMAP move target was not identifiable; completing from verified source absence"
    );
    Ok(finalize_persisted_imap_workflow_without_target_projection(
        pool,
        account_id,
        thread_id,
        refs,
        action_id,
        completed_payload,
    )
    .await?)
}

pub(crate) async fn imap_workflow_sources_are_absent(
    provider: &dyn MailProvider,
    refs: &[MessageRef],
    expected_generations: &HashMap<String, u32>,
) -> crate::sync::errors::SyncResult<bool> {
    let mut refs_by_folder: HashMap<&str, HashSet<&str>> = HashMap::new();
    for message in refs {
        let source_folder = message
            .source_folder
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                format!(
                    "IMAP source folder is missing for message {}",
                    message.remote_id
                )
            })?;
        refs_by_folder
            .entry(source_folder)
            .or_default()
            .insert(message.remote_id.as_str());
    }

    for (source_folder, remote_ids) in refs_by_folder {
        let snapshot = provider
            .sync_message_state_snapshot(source_folder)
            .await?
            .ok_or_else(|| {
                format!("IMAP source folder `{source_folder}` did not provide a UID snapshot")
            })?;
        let current_generation = snapshot.remote_uidvalidity.ok_or_else(|| {
            format!("IMAP source folder `{source_folder}` did not provide UIDVALIDITY")
        })?;
        if expected_generations.get(source_folder) != Some(&current_generation) {
            return Err(format!(
                "IMAP source folder `{source_folder}` changed UIDVALIDITY; refusing ambiguous move completion"
            )
            .into());
        }
        if snapshot
            .states
            .iter()
            .any(|state| remote_ids.contains(state.remote_id.as_str()))
        {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(crate) async fn validate_generation_then_reset_imap_target(
    pool: &SqlitePool,
    account_id: &str,
    local_folder_id: &str,
    messages: &[MessageEnvelope],
    current_uidvalidity: u32,
    should_reset: bool,
) -> Result<HashSet<String>, String> {
    if messages
        .iter()
        .any(|message| message.remote_uidvalidity != Some(current_uidvalidity))
    {
        return Err(
            "IMAP target generation changed while its messages were being reconciled".to_string(),
        );
    }
    if should_reset {
        reset_local_provider_folder(pool, account_id, local_folder_id).await
    } else {
        Ok(HashSet::new())
    }
}

#[derive(Default)]
pub(crate) struct CompleteImapTargetBatch {
    pub(crate) messages: Vec<MessageEnvelope>,
    pub(crate) removals: Vec<RemoteMessageRemoval>,
    pub(crate) reset_folder: bool,
}

impl From<SyncBatch> for CompleteImapTargetBatch {
    fn from(batch: SyncBatch) -> Self {
        Self {
            messages: batch.messages,
            removals: batch.removals,
            reset_folder: batch.reset_folder,
        }
    }
}

/// The messages a move added to its target folder, and only those.
///
/// A move into a folder of a thousand messages used to refetch the folder's
/// initial window from scratch. With both snapshots in the same generation
/// the new UIDs are known; the folder sync is the fallback when they are not,
/// or when the provider cannot fetch single messages.
pub(crate) async fn fetch_new_imap_target_messages<Targeted, TargetedFut, Full, FullFut>(
    new_remote_ids: Option<HashSet<String>>,
    expected_uidvalidity: u32,
    targeted: Targeted,
    full: Full,
) -> crate::sync::errors::SyncResult<CompleteImapTargetBatch>
where
    Targeted: FnOnce(Vec<String>) -> TargetedFut,
    TargetedFut: std::future::Future<Output = crate::sync::errors::SyncResult<Option<SyncBatch>>>,
    Full: FnOnce() -> FullFut,
    FullFut: std::future::Future<Output = crate::sync::errors::SyncResult<SyncBatch>>,
{
    let Some(new_remote_ids) = new_remote_ids else {
        return full().await.map(Into::into);
    };
    if new_remote_ids.is_empty() {
        return Ok(CompleteImapTargetBatch {
            messages: Vec::new(),
            removals: Vec::new(),
            reset_folder: false,
        });
    }
    let mut remote_ids: Vec<String> = new_remote_ids.into_iter().collect();
    remote_ids.sort();
    match targeted(remote_ids).await? {
        Some(batch) if batch.remote_uidvalidity == Some(expected_uidvalidity) => Ok(batch.into()),
        Some(_) => Err("IMAP target generation changed during its targeted fetch".into()),
        None => full().await.map(Into::into),
    }
}

pub(crate) async fn fetch_complete_imap_target<F, Fut>(
    mut fetch: F,
) -> crate::sync::errors::SyncResult<CompleteImapTargetBatch>
where
    F: FnMut(SyncCheckpoint) -> Fut,
    Fut: std::future::Future<Output = crate::sync::errors::SyncResult<SyncBatch>>,
{
    let mut checkpoint = SyncCheckpoint { cursor: None };
    let mut seen_cursors = HashSet::new();
    let mut complete = CompleteImapTargetBatch::default();
    loop {
        let batch = fetch(checkpoint).await?;
        complete.reset_folder |= batch.reset_folder;
        complete.messages.extend(batch.messages);
        complete.removals.extend(batch.removals);

        let Some(next_cursor) = batch.next.cursor else {
            break;
        };
        if !backfill_pending(&next_cursor) {
            break;
        }
        if !seen_cursors.insert(next_cursor.clone()) {
            return Err("IMAP target backfill cursor stopped making progress".into());
        }
        checkpoint = SyncCheckpoint {
            cursor: Some(next_cursor),
        };
    }
    Ok(complete)
}

pub(crate) fn proven_new_imap_target_ids(
    before_uidvalidity: u32,
    current_uidvalidity: u32,
    before_remote_ids: &[String],
    current_states: &[crate::providers::traits::RemoteMessageState],
) -> Option<HashSet<String>> {
    if before_uidvalidity != current_uidvalidity {
        return None;
    }
    let before = before_remote_ids.iter().collect::<HashSet<_>>();
    Some(
        current_states
            .iter()
            .filter(|state| !before.contains(&state.remote_id))
            .map(|state| state.remote_id.clone())
            .collect(),
    )
}

pub(crate) async fn verify_imap_workflow_target_projection(
    pool: &SqlitePool,
    account_id: &str,
    thread_id: &str,
    refs: &[MessageRef],
    local_target_folder_id: &str,
    eligible_target_remote_ids: Option<&HashSet<String>>,
) -> Result<Vec<(String, String)>, String> {
    let mut replacements = Vec::new();
    let mut matched_target_ids = HashSet::new();
    let mut all_eligible_target_ids = HashSet::new();
    for message in refs {
        let Some(source) = message.source_folder.as_deref() else {
            return Err("IMAP workflow source folder is missing".to_string());
        };
        let identity = sqlx::query_as::<
            _,
            (
                Option<String>,
                Option<String>,
                String,
                String,
                Option<i64>,
                String,
            ),
        >(
            "SELECT m.message_id_header, m.subject, m.from_address, m.date, m.raw_size, m.id
             FROM messages m
             JOIN folders f ON f.id = m.folder_id
             WHERE m.account_id = ? AND m.remote_id = ? AND (
               LOWER(TRIM(COALESCE(f.remote_id, ''))) = LOWER(TRIM(?))
               OR LOWER(TRIM(f.full_path)) = LOWER(TRIM(?))
             )
             LIMIT 1",
        )
        .bind(account_id)
        .bind(&message.remote_id)
        .bind(source)
        .bind(source)
        .fetch_optional(pool)
        .await
        .map_err(|err| err.to_string())?
        .ok_or_else(|| format!("IMAP workflow source {} disappeared", message.remote_id))?;

        let real_message_id = identity
            .0
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .filter(|value| !is_synthetic_imap_message_id(value));
        let target_candidates: Vec<(String, String)> = match real_message_id {
            Some(message_id) => sqlx::query_as(
                "SELECT id, remote_id FROM messages
                 WHERE account_id = ? AND folder_id = ? AND thread_id = ?
                   AND LOWER(TRIM(COALESCE(message_id_header, ''))) = LOWER(TRIM(?))
                   AND remote_id IS NOT NULL
                 ORDER BY updated_at DESC",
            )
            .bind(account_id)
            .bind(local_target_folder_id)
            .bind(thread_id)
            .bind(message_id)
            .fetch_all(pool)
            .await
            .map_err(|err| err.to_string())?,
            None => sqlx::query_as(
                "SELECT id, remote_id FROM messages
                 WHERE account_id = ? AND folder_id = ? AND thread_id = ?
                   AND COALESCE(subject, '') = COALESCE(?, '')
                   AND LOWER(TRIM(from_address)) = LOWER(TRIM(?))
                   AND date = ?
                   AND COALESCE(raw_size, -1) = COALESCE(?, -1)
                   AND remote_id IS NOT NULL
                 ORDER BY updated_at DESC",
            )
            .bind(account_id)
            .bind(local_target_folder_id)
            .bind(thread_id)
            .bind(&identity.1)
            .bind(&identity.2)
            .bind(&identity.3)
            .bind(identity.4)
            .fetch_all(pool)
            .await
            .map_err(|err| err.to_string())?,
        };
        let eligible_candidates = target_candidates
            .into_iter()
            .filter(|(_, remote_id)| {
                eligible_target_remote_ids.is_none_or(|remote_ids| remote_ids.contains(remote_id))
            })
            .collect::<Vec<_>>();
        all_eligible_target_ids.extend(
            eligible_candidates
                .iter()
                .map(|(candidate_id, _)| candidate_id.clone()),
        );
        let Some(target_id) = eligible_candidates
            .into_iter()
            .find(|(candidate_id, _)| !matched_target_ids.contains(candidate_id))
            .map(|(candidate_id, _)| candidate_id)
        else {
            return Err(format!(
                "IMAP target reconciliation did not find moved message {}",
                message.remote_id
            ));
        };
        replacements.push((identity.5, target_id.clone()));
        if !matched_target_ids.insert(target_id) {
            return Err("IMAP target reconciliation reused one copy for multiple messages".into());
        }
    }

    // Across a UIDVALIDITY reset there is no set-difference proof. Only a
    // complete, unambiguous one-to-one identity match is safe: an extra
    // preexisting duplicate must keep the source projection intact.
    if eligible_target_remote_ids.is_none() && all_eligible_target_ids.len() != refs.len() {
        return Err(
            "IMAP target reconciliation is ambiguous after UIDVALIDITY changed".to_string(),
        );
    }

    Ok(replacements)
}

#[cfg(test)]
pub(crate) async fn finalize_imap_workflow_target_projection(
    pool: &SqlitePool,
    account_id: &str,
    thread_id: &str,
    refs: &[MessageRef],
    local_target_folder_id: &str,
    new_target_remote_ids: &HashSet<String>,
) -> Result<(), String> {
    verify_imap_workflow_target_projection(
        pool,
        account_id,
        thread_id,
        refs,
        local_target_folder_id,
        Some(new_target_remote_ids),
    )
    .await?;
    delete_completed_imap_workflow_sources(pool, account_id, refs).await?;
    refresh_provider_thread_projection(pool, thread_id).await?;
    Ok(())
}

#[cfg(test)]
pub(crate) async fn finalize_imap_workflow_target_after_uidvalidity_change(
    pool: &SqlitePool,
    account_id: &str,
    thread_id: &str,
    refs: &[MessageRef],
    local_target_folder_id: &str,
) -> Result<(), String> {
    verify_imap_workflow_target_projection(
        pool,
        account_id,
        thread_id,
        refs,
        local_target_folder_id,
        None,
    )
    .await?;
    delete_completed_imap_workflow_sources(pool, account_id, refs).await?;
    refresh_provider_thread_projection(pool, thread_id).await?;
    Ok(())
}

pub(crate) async fn finalize_persisted_imap_workflow_target_projection(
    pool: &SqlitePool,
    account_id: &str,
    thread_id: &str,
    refs: &[MessageRef],
    local_target_folder_id: &str,
    eligible_target_remote_ids: Option<&HashSet<String>>,
    action_id: &str,
    completed_payload: &serde_json::Value,
) -> Result<(), String> {
    if persisted_imap_projection_is_completed(pool, action_id).await? {
        refresh_provider_thread_projection(pool, thread_id).await?;
        return Ok(());
    }

    let replacements = verify_imap_workflow_target_projection(
        pool,
        account_id,
        thread_id,
        refs,
        local_target_folder_id,
        eligible_target_remote_ids,
    )
    .await?;

    persist_completed_imap_workflow_projection_with_replacements(
        pool,
        account_id,
        thread_id,
        refs,
        action_id,
        completed_payload,
        &replacements,
    )
    .await
}

pub(crate) async fn finalize_persisted_imap_workflow_without_target_projection(
    pool: &SqlitePool,
    account_id: &str,
    thread_id: &str,
    refs: &[MessageRef],
    action_id: &str,
    completed_payload: &serde_json::Value,
) -> Result<(), String> {
    if persisted_imap_projection_is_completed(pool, action_id).await? {
        refresh_provider_thread_projection(pool, thread_id).await?;
        return Ok(());
    }
    persist_completed_imap_workflow_projection(
        pool,
        account_id,
        thread_id,
        refs,
        action_id,
        completed_payload,
    )
    .await
}

pub(crate) async fn persisted_imap_projection_is_completed(
    pool: &SqlitePool,
    action_id: &str,
) -> Result<bool, String> {
    // Deleting the source projection and recording this bit happen in one
    // transaction below. A process that dies after commit but before the queue
    // row is removed therefore resumes without requiring the deleted source
    // copy to prove the move a second time.
    let stored_payload: Option<String> = sqlx::query_scalar(
        "SELECT payload FROM sync_queue WHERE id = ? AND status = 'sending' LIMIT 1",
    )
    .bind(action_id)
    .fetch_optional(pool)
    .await
    .map_err(|err| err.to_string())?;
    let stored_payload = stored_payload
        .ok_or_else(|| "workflow move queue row disappeared during finalize".to_string())?;
    Ok(serde_json::from_str::<serde_json::Value>(&stored_payload)
        .map_err(|err| format!("decode persisted workflow move: {err}"))?
        .get("projection_completed")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false))
}

pub(crate) async fn persist_completed_imap_workflow_projection(
    pool: &SqlitePool,
    account_id: &str,
    thread_id: &str,
    refs: &[MessageRef],
    action_id: &str,
    completed_payload: &serde_json::Value,
) -> Result<(), String> {
    persist_completed_imap_workflow_projection_with_replacements(
        pool,
        account_id,
        thread_id,
        refs,
        action_id,
        completed_payload,
        &[],
    )
    .await
}

async fn persist_completed_imap_workflow_projection_with_replacements(
    pool: &SqlitePool,
    account_id: &str,
    thread_id: &str,
    refs: &[MessageRef],
    action_id: &str,
    completed_payload: &serde_json::Value,
    replacements: &[(String, String)],
) -> Result<(), String> {
    let mut projection_payload = completed_payload.clone();
    set_workflow_payload_field(&mut projection_payload, "projection_completed", true.into())?;
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|err| err.to_string())?;
    for (source, target) in replacements {
        satchel_core::mail_actions::undo::remap_message_in_transaction(&mut tx, source, target)
            .await?;
    }
    let deleted =
        delete_completed_imap_workflow_sources_in_transaction(&mut tx, account_id, refs).await?;
    if deleted != refs.len() as u64 {
        return Err(format!(
            "IMAP workflow finalize deleted {deleted} of {} source messages",
            refs.len()
        ));
    }
    let updated = sqlx::query(
        "UPDATE sync_queue SET payload = ?, updated_at_ms = ?
         WHERE id = ? AND status = 'sending'",
    )
    .bind(projection_payload.to_string())
    .bind(chrono::Utc::now().timestamp_millis())
    .bind(action_id)
    .execute(&mut *tx)
    .await
    .map_err(|err| err.to_string())?
    .rows_affected();
    if updated != 1 {
        return Err("workflow move queue row disappeared during finalize".into());
    }
    tx.commit().await.map_err(|err| err.to_string())?;
    refresh_provider_thread_projection(pool, thread_id).await?;
    Ok(())
}

pub(crate) fn is_synthetic_imap_message_id(value: &str) -> bool {
    let normalized = value.trim().to_ascii_lowercase();
    normalized.ends_with("@imap.satchel.local>") || normalized.ends_with("@imap.flowmail.local>")
}

#[cfg(test)]
pub(crate) async fn delete_completed_imap_workflow_sources(
    pool: &SqlitePool,
    account_id: &str,
    refs: &[MessageRef],
) -> Result<(), String> {
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|err| err.to_string())?;
    delete_completed_imap_workflow_sources_in_transaction(&mut tx, account_id, refs).await?;
    tx.commit().await.map_err(|err| err.to_string())?;
    Ok(())
}

pub(crate) async fn delete_completed_imap_workflow_sources_in_transaction(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    account_id: &str,
    refs: &[MessageRef],
) -> Result<u64, String> {
    let mut deleted = 0_u64;
    for message in refs {
        let Some(source) = message.source_folder.as_deref() else {
            continue;
        };
        sqlx::query(
            "DELETE FROM attachments WHERE message_id IN (
               SELECT m.id FROM messages m
               JOIN folders f ON f.id = m.folder_id
               WHERE m.account_id = ? AND m.remote_id = ?
                 AND (
                   LOWER(TRIM(COALESCE(f.remote_id, ''))) = LOWER(TRIM(?))
                   OR LOWER(TRIM(f.full_path)) = LOWER(TRIM(?))
                 )
             )",
        )
        .bind(account_id)
        .bind(&message.remote_id)
        .bind(source)
        .bind(source)
        .execute(&mut **tx)
        .await
        .map_err(|err| err.to_string())?;
        deleted += sqlx::query(
            "DELETE FROM messages
             WHERE account_id = ? AND remote_id = ? AND folder_id IN (
               SELECT id FROM folders WHERE account_id = ? AND (
                 LOWER(TRIM(COALESCE(remote_id, ''))) = LOWER(TRIM(?))
                 OR LOWER(TRIM(full_path)) = LOWER(TRIM(?))
               )
             )",
        )
        .bind(account_id)
        .bind(&message.remote_id)
        .bind(account_id)
        .bind(source)
        .bind(source)
        .execute(&mut **tx)
        .await
        .map_err(|err| err.to_string())?
        .rows_affected();
    }
    Ok(deleted)
}

#[cfg(test)]
mod tests {
    use super::refresh_provider_thread_projection;
    use super::{
        fetch_complete_imap_target, fetch_new_imap_target_messages, proven_new_imap_target_ids,
        reset_local_provider_folder, source_generations_match,
        validate_generation_then_reset_imap_target,
    };
    use crate::providers::traits::{
        MessageEnvelope, MessageRef, RemoteMessageState, SyncBatch, SyncCheckpoint,
    };
    use sqlx::sqlite::SqlitePoolOptions;
    use std::collections::{HashMap, HashSet};

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

    #[test]
    fn imap_target_uid_difference_is_scoped_to_uidvalidity_generation() {
        let states = vec![
            RemoteMessageState {
                remote_id: "808".into(),
                is_read: None,
                is_starred: None,
            },
            RemoteMessageState {
                remote_id: "909".into(),
                is_read: None,
                is_starred: None,
            },
        ];
        assert_eq!(
            proven_new_imap_target_ids(11, 11, &["808".into()], &states),
            Some(HashSet::from(["909".into()]))
        );
        assert_eq!(
            proven_new_imap_target_ids(11, 22, &["808".into()], &states),
            None,
            "the same numeric UIDs cannot be compared across generations"
        );
    }

    #[test]
    fn stale_blocked_move_generation_is_rejected_before_execution() {
        let refs = vec![MessageRef {
            remote_id: "42".into(),
            source_folder: Some("INBOX".into()),
            source_uidvalidity: Some(11),
        }];
        let expected = HashMap::from([(String::from("INBOX"), 11)]);
        assert!(source_generations_match(
            &refs,
            &expected,
            &HashMap::from([(String::from("INBOX"), 11)])
        ));
        assert!(!source_generations_match(
            &refs,
            &expected,
            &HashMap::from([(String::from("INBOX"), 22)])
        ));
        assert!(!source_generations_match(
            &[MessageRef {
                source_uidvalidity: None,
                ..refs[0].clone()
            }],
            &expected,
            &HashMap::from([(String::from("INBOX"), 11)])
        ));
    }

    /// What the provider was asked for: `Some(uids)` for a targeted fetch,
    /// `None` for a folder sync from an empty cursor.
    type Requests = std::sync::Arc<std::sync::Mutex<Vec<Option<Vec<String>>>>>;

    fn batch_of(uids: &[String], uidvalidity: u32) -> SyncBatch {
        SyncBatch {
            messages: uids.iter().map(|uid| envelope(uid)).collect(),
            removals: Vec::new(),
            remote_uidvalidity: Some(uidvalidity),
            reset_folder: false,
            snapshot_required: false,
            next: SyncCheckpoint { cursor: None },
        }
    }

    async fn run_target_fetch(
        before: &[String],
        after: &[String],
        served_uidvalidity: u32,
    ) -> (
        crate::sync::errors::SyncResult<super::CompleteImapTargetBatch>,
        Vec<Option<Vec<String>>>,
    ) {
        let states = after
            .iter()
            .map(|uid| crate::providers::traits::RemoteMessageState {
                remote_id: uid.clone(),
                is_read: None,
                is_starred: None,
            })
            .collect::<Vec<_>>();
        let requests: Requests = Default::default();
        let (targeted_log, full_log) = (requests.clone(), requests.clone());
        let result = fetch_new_imap_target_messages(
            proven_new_imap_target_ids(7, 7, before, &states),
            7,
            |uids: Vec<String>| async move {
                targeted_log.lock().unwrap().push(Some(uids.clone()));
                Ok(Some(batch_of(&uids, served_uidvalidity)))
            },
            || async move {
                full_log.lock().unwrap().push(None);
                Ok(batch_of(&[], 7))
            },
        )
        .await;
        let log = requests.lock().unwrap().clone();
        (result, log)
    }

    #[tokio::test]
    async fn a_workflow_move_fetches_only_the_new_target_uids() {
        let known: Vec<String> = (1..=1000).map(|uid| uid.to_string()).collect();
        let mut after = known.clone();
        after.push("1001".into());

        let (batch, requests) = run_target_fetch(&known, &after, 7).await;

        assert_eq!(requests, [Some(vec!["1001".to_string()])]);
        let batch = batch.expect("targeted fetch");
        assert_eq!(batch.messages.len(), 1);
        assert!(!batch.reset_folder);
    }

    #[tokio::test]
    async fn a_workflow_move_without_new_uids_fetches_nothing() {
        let known: Vec<String> = (1..=10).map(|uid| uid.to_string()).collect();

        let (batch, requests) = run_target_fetch(&known, &known, 7).await;

        assert!(requests.is_empty(), "{requests:?}");
        assert!(batch.expect("no fetch").messages.is_empty());
    }

    #[tokio::test]
    async fn a_generation_change_between_snapshot_and_fetch_is_refused() {
        let (batch, _) = run_target_fetch(&["1".into()], &["1".into(), "2".into()], 8).await;

        assert!(batch.is_err());
    }

    #[tokio::test]
    async fn unprovable_new_uids_fall_back_to_the_folder_sync() {
        let requests: Requests = Default::default();
        let full_log = requests.clone();

        fetch_new_imap_target_messages(
            None,
            7,
            |_uids: Vec<String>| async { Ok(None) },
            || async move {
                full_log.lock().unwrap().push(None);
                Ok(batch_of(&[], 7))
            },
        )
        .await
        .expect("fallback");

        assert_eq!(*requests.lock().unwrap(), [None]);
    }

    #[tokio::test]
    async fn exact_imap_target_fetch_completes_every_backfill_page() {
        let first_messages = (1..=100)
            .map(|uid| envelope(&uid.to_string()))
            .collect::<Vec<_>>();
        let batches =
            std::sync::Arc::new(std::sync::Mutex::new(std::collections::VecDeque::from([
                SyncBatch {
                    messages: first_messages,
                    removals: Vec::new(),
                    remote_uidvalidity: Some(7),
                    reset_folder: true,
                    snapshot_required: true,
                    next: SyncCheckpoint {
                        cursor: Some(
                            serde_json::json!({
                                "backfill_remaining_messages": 1,
                                "backfill_total_messages": 101
                            })
                            .to_string(),
                        ),
                    },
                },
                SyncBatch {
                    messages: vec![envelope("101")],
                    removals: Vec::new(),
                    remote_uidvalidity: Some(7),
                    reset_folder: false,
                    snapshot_required: true,
                    next: SyncCheckpoint {
                        cursor: Some(
                            serde_json::json!({
                                "backfill_remaining_messages": 0,
                                "backfill_total_messages": 101
                            })
                            .to_string(),
                        ),
                    },
                },
            ])));
        let source = std::sync::Arc::clone(&batches);
        let complete = fetch_complete_imap_target(move |_| {
            let batch = source.lock().expect("batch lock").pop_front();
            async move {
                batch.ok_or_else(|| crate::sync::errors::SyncError::from("unexpected extra page"))
            }
        })
        .await
        .expect("complete target fetch");

        assert!(complete.reset_folder);
        assert_eq!(complete.messages.len(), 101);
        assert_eq!(
            complete
                .messages
                .last()
                .map(|message| message.remote_id.as_str()),
            Some("101")
        );
        assert!(batches.lock().expect("remaining batch lock").is_empty());
    }

    #[tokio::test]
    async fn uidvalidity_reset_removes_stale_rows_and_repairs_shared_thread_projection() {
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
             VALUES ('sent', 'account', 'Sent', 'Sent', 'sent', 'Sent', '2026-01-01', '2026-01-01')",
            "INSERT INTO threads
             (id, account_id, subject, snippet, last_message_date, created_at, updated_at)
             VALUES ('thread', 'account', 'stale', 'stale', '2026-12-01', '2026-01-01', '2026-01-01')",
            "INSERT INTO messages
             (id, account_id, folder_id, thread_id, from_address, to_addresses, subject, snippet, date, created_at, updated_at)
             VALUES ('inbox-message', 'account', 'inbox', 'thread', 'old@example.com', '[]', 'stale', 'stale', '2026-12-01', '2026-01-01', '2026-01-01')",
            "INSERT INTO messages
             (id, account_id, folder_id, thread_id, from_address, to_addresses, subject, snippet, date, created_at, updated_at)
             VALUES ('sent-message', 'account', 'sent', 'thread', 'me@example.com', '[]', 'kept', 'kept body', '2026-01-02', '2026-01-01', '2026-01-01')",
            "INSERT INTO threads
             (id, account_id, subject, snippet, last_message_date, created_at, updated_at)
             VALUES ('orphan', 'account', 'orphan', 'orphan', '2026-01-03', '2026-01-01', '2026-01-01')",
            "INSERT INTO messages
             (id, account_id, folder_id, thread_id, from_address, to_addresses, subject, snippet, date, created_at, updated_at)
             VALUES ('orphan-message', 'account', 'inbox', 'orphan', 'old@example.com', '[]', 'orphan', 'orphan', '2026-01-03', '2026-01-01', '2026-01-01')",
            "INSERT INTO tags (id, name) VALUES ('tag', 'Tag')",
            "INSERT INTO thread_tags (thread_id, tag_id) VALUES ('orphan', 'tag')",
            "INSERT INTO workflow_events
             (id, thread_id, from_state, to_state, reason, created_at)
             VALUES ('event', 'orphan', 'inbox', 'done', 'test', '2026-01-01')",
        ] {
            sqlx::query(statement).execute(&pool).await.expect("seed");
        }

        let before_mismatch: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages")
            .fetch_one(&pool)
            .await
            .expect("message count before mismatch");
        for invalid_uidvalidity in [Some(11), None] {
            let mut wrong_generation = envelope("999");
            wrong_generation.remote_uidvalidity = invalid_uidvalidity;
            validate_generation_then_reset_imap_target(
                &pool,
                "account",
                "inbox",
                &[wrong_generation],
                22,
                true,
            )
            .await
            .expect_err("unproven fetch must abort before reset");
        }
        let after_mismatch: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages")
            .fetch_one(&pool)
            .await
            .expect("message count after mismatch");
        assert_eq!(after_mismatch, before_mismatch);

        let reset_threads = reset_local_provider_folder(&pool, "account", "inbox")
            .await
            .expect("reset");
        for thread_id in reset_threads {
            refresh_provider_thread_projection(&pool, &thread_id)
                .await
                .expect("projection cleanup");
        }
        let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages")
            .fetch_one(&pool)
            .await
            .expect("remaining");
        assert_eq!(remaining, 1);
        let projection: (String, String) =
            sqlx::query_as("SELECT subject, snippet FROM threads WHERE id = 'thread'")
                .fetch_one(&pool)
                .await
                .expect("projection");
        assert_eq!(projection, ("kept".to_string(), "kept body".to_string()));
        let orphan_exists: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM threads WHERE id = 'orphan'")
                .fetch_one(&pool)
                .await
                .expect("orphan count");
        assert_eq!(orphan_exists, 0);
    }
}
