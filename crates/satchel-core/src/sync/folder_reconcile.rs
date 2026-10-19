//! Reconcile remote folder state with Satchel's local message and workflow
//! projections.
//!
//! Mailbox sync decides when to fetch folders. This module owns what an
//! authoritative folder listing, message snapshot, or tombstone means locally,
//! including the queued-mutation exceptions that protect optimistic changes.

use std::collections::{HashMap, HashSet};

use sqlx::{Executor, Sqlite, SqlitePool};

use crate::providers::gmail::gmail_label_is_state_view;
use crate::providers::traits::{MessageRef, RemoteMessageRemoval, RemoteMessageState};
use crate::sync::remote_mutations::{remote_move_thread_id_from_json, RemoteMutationPayload};

use super::thread_resolution::refresh_provider_thread_projection;

#[derive(Debug, Default)]
pub struct ImapSnapshotChanges {
    pub relocated_threads: HashSet<String>,
    /// The snapshot named a different identity generation than the folder, so
    /// nothing was removed. Visible to the caller because a folder that keeps
    /// skipping its reconciliation is a fault worth seeing.
    pub skipped_stale_generation: bool,
}

/// Which queued mutations count as still owning a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingMutations {
    /// Everything that still has to reach the server. A message under any of
    /// them must not be relocated or removed locally.
    All,
    /// Only flag changes. A queued move says nothing about read or star state,
    /// and holding those back on its account would keep the provider's values
    /// out for as long as the move is stuck.
    FlagsOnly,
}

/// The `(source folder, remote id)` pairs a queued mutation still owns.
pub async fn pending_remote_message_refs<'e, E>(
    executor: E,
    account_id: &str,
    scope: PendingMutations,
) -> Result<HashSet<(String, String)>, String>
where
    E: Executor<'e, Database = Sqlite>,
{
    let payloads = sqlx::query_scalar::<_, String>(
        "SELECT payload
         FROM sync_queue
         WHERE account_id = ?
           AND action_type = 'remote_mutation'
           AND status IN ('pending', 'sending', 'scheduled', 'failed')",
    )
    .bind(account_id)
    .fetch_all(executor)
    .await
    .map_err(|err| err.to_string())?;

    let mut refs = HashSet::new();
    for payload in payloads {
        let Ok(mutation) = serde_json::from_str::<RemoteMutationPayload>(&payload) else {
            continue;
        };
        if scope == PendingMutations::FlagsOnly
            && !matches!(mutation, RemoteMutationPayload::Flags { .. })
        {
            continue;
        }
        let message_refs = match mutation {
            RemoteMutationPayload::Move { refs, .. }
            | RemoteMutationPayload::BlockedMove { refs, .. }
            | RemoteMutationPayload::Delete { refs }
            | RemoteMutationPayload::Flags { refs, .. } => refs,
            RemoteMutationPayload::WorkflowMove { refs, .. } => refs,
            // A draft waiting to be removed on the server must not be pulled
            // back in locally while that delete is still queued.
            RemoteMutationPayload::DeleteDraft {
                remote_message_id,
                source_folder,
                ..
            } => match remote_message_id {
                Some(remote_id) => vec![MessageRef {
                    remote_id,
                    source_folder,
                    source_uidvalidity: None,
                }],
                None => continue,
            },
            // Folder operations carry no message refs, so there is nothing to
            // reconcile here.
            RemoteMutationPayload::CreateFolder { .. }
            | RemoteMutationPayload::RenameFolder { .. }
            | RemoteMutationPayload::DeleteFolder { .. }
            | RemoteMutationPayload::AppendSent { .. }
            | RemoteMutationPayload::UndoTrash { .. } => continue,
        };
        for message_ref in message_refs {
            if let Some(folder) = message_ref.source_folder {
                refs.insert((folder.to_ascii_lowercase(), message_ref.remote_id));
            }
        }
    }
    Ok(refs)
}

fn normalized_remote_folder(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

async fn pending_remote_folder_paths<'e, E>(
    executor: E,
    account_id: &str,
) -> Result<HashSet<String>, String>
where
    E: Executor<'e, Database = Sqlite>,
{
    let payloads = sqlx::query_scalar::<_, String>(
        "SELECT payload FROM sync_queue
         WHERE account_id = ? AND action_type = 'remote_mutation'
           AND status IN ('pending', 'sending', 'scheduled')",
    )
    .bind(account_id)
    .fetch_all(executor)
    .await
    .map_err(|err| err.to_string())?;

    let mut paths = HashSet::new();
    let add_refs = |paths: &mut HashSet<String>, refs: &[MessageRef]| {
        paths.extend(
            refs.iter()
                .filter_map(|message| message.source_folder.as_deref())
                .map(normalized_remote_folder),
        );
    };
    for payload in payloads {
        let Ok(payload) = serde_json::from_str::<RemoteMutationPayload>(&payload) else {
            continue;
        };
        match payload {
            RemoteMutationPayload::Move {
                refs,
                target_remote_folder_id,
            } => {
                add_refs(&mut paths, &refs);
                paths.insert(normalized_remote_folder(&target_remote_folder_id));
            }
            RemoteMutationPayload::WorkflowMove {
                refs,
                target_remote_folder_id,
                ..
            } => {
                add_refs(&mut paths, &refs);
                paths.insert(normalized_remote_folder(&target_remote_folder_id));
            }
            RemoteMutationPayload::BlockedMove {
                refs,
                target_remote_folder_id,
                ..
            } => {
                add_refs(&mut paths, &refs);
                paths.insert(normalized_remote_folder(&target_remote_folder_id));
            }
            RemoteMutationPayload::Delete { refs } | RemoteMutationPayload::Flags { refs, .. } => {
                add_refs(&mut paths, &refs)
            }
            RemoteMutationPayload::DeleteDraft { source_folder, .. } => {
                if let Some(folder) = source_folder {
                    paths.insert(normalized_remote_folder(&folder));
                }
            }
            RemoteMutationPayload::CreateFolder { path }
            | RemoteMutationPayload::DeleteFolder { path } => {
                paths.insert(normalized_remote_folder(&path));
            }
            RemoteMutationPayload::RenameFolder { from_path, to_path } => {
                paths.insert(normalized_remote_folder(&from_path));
                paths.insert(normalized_remote_folder(&to_path));
            }
            // The forward move owns the folders until this dependent action
            // can enqueue its reverse move.
            RemoteMutationPayload::UndoTrash { .. } => {}
            RemoteMutationPayload::AppendSent { folder_hint, .. } => {
                if let Some(folder) = folder_hint {
                    paths.insert(normalized_remote_folder(&folder));
                }
            }
        }
    }
    Ok(paths)
}

fn complete_remote_folder_list_available(provider: &str) -> bool {
    if !provider.eq_ignore_ascii_case("imap") {
        return provider.eq_ignore_ascii_case("gmail") || provider.eq_ignore_ascii_case("outlook");
    }
    !matches!(
        std::env::var("SATCHEL_IMAP_SYNC_ALL_FOLDERS")
            .ok()
            .as_deref()
            .map(str::trim)
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("0" | "false" | "no" | "off")
    )
}

/// Require consecutive complete listings before retiring a remote folder.
/// Cached mail stays in a local folder so thread metadata remains attached.
pub async fn reconcile_missing_remote_folders(
    pool: &SqlitePool,
    account_id: &str,
    provider: &str,
    remote_folder_ids: &HashSet<String>,
    sync_started_at: &str,
) -> Result<HashSet<String>, String> {
    if !complete_remote_folder_list_available(provider) {
        return Ok(HashSet::new());
    }
    let present = remote_folder_ids
        .iter()
        .map(|folder| normalized_remote_folder(folder))
        .collect::<HashSet<_>>();
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|err| err.to_string())?;
    let protected = pending_remote_folder_paths(&mut *tx, account_id).await?;
    let folders = sqlx::query_as::<_, (String, String, String, String, String, i64)>(
        "SELECT id, COALESCE(remote_id, ''), full_path, folder_type, updated_at, absent_runs
         FROM folders WHERE account_id = ? AND folder_type != 'local'",
    )
    .bind(account_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(|err| err.to_string())?;
    let eligible = folders
        .into_iter()
        .filter(|(_, remote, path, kind, _, _)| {
            let remote = if remote.trim().is_empty() {
                path
            } else {
                remote
            };
            !remote.trim().is_empty()
                && kind != "aside"
                && !(provider.eq_ignore_ascii_case("gmail")
                    && (remote.eq_ignore_ascii_case("archive")
                        || gmail_label_is_state_view(remote)))
        })
        .collect::<Vec<_>>();
    let missing = eligible
        .iter()
        .filter(|(_, remote, path, _, _, _)| {
            !present.contains(&normalized_remote_folder(if remote.is_empty() {
                path
            } else {
                remote
            }))
        })
        .count();
    // An Inbox fallback or loss of at least half the known folders is not a
    // trustworthy LIST. Reset the streak rather than confirming earlier gaps.
    if present.is_empty()
        || (provider.eq_ignore_ascii_case("imap")
            && present.len() == 1
            && present.contains("inbox")
            && eligible.len() > 1)
        || (missing >= 2 && missing * 2 >= eligible.len())
    {
        sqlx::query("UPDATE folders SET absent_runs = 0 WHERE account_id = ?")
            .bind(account_id)
            .execute(&mut *tx)
            .await
            .map_err(|err| err.to_string())?;
        tx.commit().await.map_err(|err| err.to_string())?;
        return Ok(HashSet::new());
    }
    let mut retained_threads = HashSet::new();
    for (folder_id, remote_id, path, _, updated_at, absent_runs) in eligible {
        let normalized = normalized_remote_folder(if remote_id.is_empty() {
            &path
        } else {
            &remote_id
        });
        if present.contains(&normalized)
            || protected.contains(&normalized)
            || updated_at.as_str() >= sync_started_at
        {
            sqlx::query("UPDATE folders SET absent_runs = 0 WHERE id = ?")
                .bind(&folder_id)
                .execute(&mut *tx)
                .await
                .map_err(|err| err.to_string())?;
            continue;
        }
        if absent_runs == 0 {
            sqlx::query("UPDATE folders SET absent_runs = 1 WHERE id = ?")
                .bind(&folder_id)
                .execute(&mut *tx)
                .await
                .map_err(|err| err.to_string())?;
            continue;
        }
        retained_threads.extend(sqlx::query_scalar::<_, String>(
            "SELECT DISTINCT thread_id FROM messages WHERE folder_id = ? AND thread_id IS NOT NULL"
        ).bind(&folder_id).fetch_all(&mut *tx).await.map_err(|err| err.to_string())?);
        let recovered = format!("recovered-{folder_id}");
        sqlx::query(
            "INSERT INTO folders (id, account_id, name, full_path, folder_type, remote_id, sync_enabled, created_at, updated_at)
             SELECT ?, account_id, name, ?, 'local', NULL, 0, created_at, ? FROM folders WHERE id = ?
             ON CONFLICT(id) DO UPDATE SET updated_at = excluded.updated_at"
        ).bind(&recovered).bind(format!("local/{recovered}")).bind(sync_started_at).bind(&folder_id).execute(&mut *tx).await.map_err(|err| err.to_string())?;
        sqlx::query("UPDATE messages SET folder_id = ?, remote_id = NULL, remote_uid = NULL, remote_draft_id = NULL WHERE folder_id = ?")
            .bind(&recovered).bind(&folder_id).execute(&mut *tx).await.map_err(|err| err.to_string())?;
        sqlx::query("UPDATE messages SET pending_folder_id = ? WHERE pending_folder_id = ?")
            .bind(&recovered)
            .bind(&folder_id)
            .execute(&mut *tx)
            .await
            .map_err(|err| err.to_string())?;
        sqlx::query("DELETE FROM folders WHERE id = ?")
            .bind(&folder_id)
            .execute(&mut *tx)
            .await
            .map_err(|err| err.to_string())?;
    }
    tx.commit().await.map_err(|err| err.to_string())?;
    Ok(retained_threads)
}

/// Match a renamed IMAP folder only when its generation and UID/header pairs
/// agree with exactly one absent folder. UIDVALIDITY alone is not unique.
pub async fn reconcile_imap_folder_rename(
    pool: &SqlitePool,
    account_id: &str,
    folder_id: &str,
    present: &HashSet<String>,
    batch: &crate::providers::traits::SyncBatch,
) -> Result<(), String> {
    let Some(generation) = batch.remote_uidvalidity else {
        return Ok(());
    };
    if !complete_remote_folder_list_available("imap") {
        return Ok(());
    }
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|err| err.to_string())?;
    let has_messages: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM messages WHERE folder_id = ?)")
            .bind(folder_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(|error| error.to_string())?;
    if has_messages {
        return Ok(());
    }
    let protected = pending_remote_folder_paths(&mut *tx, account_id).await?;
    let candidates: Vec<(String, String)> = sqlx::query_as(
        "SELECT id, COALESCE(remote_id, full_path) FROM folders
         WHERE account_id = ? AND id != ? AND uidvalidity = ? AND folder_type != 'local'",
    )
    .bind(account_id)
    .bind(folder_id)
    .bind(i64::from(generation))
    .fetch_all(&mut *tx)
    .await
    .map_err(|err| err.to_string())?;
    let mut matches = Vec::new();
    for (old_id, remote) in candidates {
        if present.contains(&remote) || protected.contains(&normalized_remote_folder(&remote)) {
            continue;
        }
        let rows: Vec<(String, Option<String>)> = sqlx::query_as(
            "SELECT remote_id, message_id_header FROM messages WHERE folder_id = ? AND remote_id IS NOT NULL"
        ).bind(&old_id).fetch_all(&mut *tx).await.map_err(|err| err.to_string())?;
        let mut overlap = 0;
        let mut conflict = false;
        for message in &batch.messages {
            if let Some((_, header)) = rows.iter().find(|(uid, _)| uid == &message.remote_id) {
                match (header.as_deref(), message.message_id_header.as_deref()) {
                    (Some(old), Some(new)) if old == new && !old.contains(".satchel.local>") => {
                        overlap += 1
                    }
                    _ => conflict = true,
                }
            }
        }
        if overlap > 0 && !conflict {
            matches.push(old_id);
        }
    }
    if matches.len() != 1 {
        return Ok(());
    }
    let old = &matches[0];
    for statement in [
        "UPDATE messages SET folder_id = ?1 WHERE folder_id = ?2",
        "UPDATE messages SET pending_folder_id = ?1 WHERE pending_folder_id = ?2",
        "UPDATE sync_quarantine SET folder_id = ?1 WHERE folder_id = ?2",
    ] {
        sqlx::query(statement)
            .bind(folder_id)
            .bind(old)
            .execute(&mut *tx)
            .await
            .map_err(|err| err.to_string())?;
    }
    sqlx::query("UPDATE folders SET uidvalidity = ? WHERE id = ?")
        .bind(i64::from(generation))
        .bind(folder_id)
        .execute(&mut *tx)
        .await
        .map_err(|err| err.to_string())?;
    sqlx::query("DELETE FROM folders WHERE id = ?")
        .bind(old)
        .execute(&mut *tx)
        .await
        .map_err(|err| err.to_string())?;
    tx.commit().await.map_err(|err| err.to_string())?;
    Ok(())
}

/// Whether the snapshot and the local folder speak of the same message ids.
///
/// IMAP UIDs mean nothing outside their UIDVALIDITY generation. One side
/// naming a generation the other does not know is enough of a mismatch:
/// providers with account-wide ids leave both empty and reconcile as before.
fn generations_agree(snapshot: Option<u32>, local: Option<u32>) -> bool {
    snapshot == local
}

/// Reconcile one folder against a provider snapshot.
///
/// `snapshot_uidvalidity` is the generation the snapshot's remote ids belong
/// to. If it disagrees with the folder's own, every local row would look
/// missing and be deleted along with its threads, tags and topics — so the
/// removal half is skipped and only matching ids have their flags updated.
pub async fn reconcile_imap_folder_snapshot(
    pool: &SqlitePool,
    account_id: &str,
    folder_id: &str,
    remote_folder_id: &str,
    remote_states: &[RemoteMessageState],
    snapshot_uidvalidity: Option<u32>,
) -> Result<ImapSnapshotChanges, String> {
    let remote_by_id = remote_states
        .iter()
        .map(|state| (state.remote_id.as_str(), state))
        .collect::<HashMap<_, _>>();
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|err| err.to_string())?;
    let local_uidvalidity =
        sqlx::query_scalar::<_, Option<i64>>("SELECT uidvalidity FROM folders WHERE id = ?")
            .bind(folder_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|err| err.to_string())?
            .flatten()
            .and_then(|value| u32::try_from(value).ok());
    let may_delete = generations_agree(snapshot_uidvalidity, local_uidvalidity);

    let pending_refs =
        pending_remote_message_refs(&mut *tx, account_id, PendingMutations::All).await?;
    let local_rows = sqlx::query_as::<_, (String, String, Option<String>, i64, i64)>(
        "SELECT id, thread_id, remote_id, is_read, is_starred
         FROM messages
         WHERE account_id = ? AND folder_id = ?",
    )
    .bind(account_id)
    .bind(folder_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(|err| err.to_string())?;

    let now = chrono::Utc::now().to_rfc3339();
    let normalized_folder = remote_folder_id.to_ascii_lowercase();
    let mut projection_threads = HashSet::new();
    let mut changes = ImapSnapshotChanges {
        skipped_stale_generation: !may_delete,
        ..ImapSnapshotChanges::default()
    };

    for (message_id, thread_id, remote_id, is_read, is_starred) in local_rows {
        let Some(remote_id) = remote_id else {
            continue;
        };
        if pending_refs.contains(&(normalized_folder.clone(), remote_id.clone())) {
            continue;
        }

        if let Some(remote) = remote_by_id.get(remote_id.as_str()) {
            let next_read = remote
                .is_read
                .map(|value| if value { 1 } else { 0 })
                .unwrap_or(is_read);
            let next_starred = remote
                .is_starred
                .map(|value| if value { 1 } else { 0 })
                .unwrap_or(is_starred);
            if is_read != next_read || is_starred != next_starred {
                sqlx::query(
                    "UPDATE messages
                     SET is_read = ?, is_starred = ?, updated_at = ?
                     WHERE id = ?",
                )
                .bind(next_read)
                .bind(next_starred)
                .bind(&now)
                .bind(&message_id)
                .execute(&mut *tx)
                .await
                .map_err(|err| err.to_string())?;
                projection_threads.insert(thread_id);
            }
            continue;
        }

        if !may_delete {
            continue;
        }

        sqlx::query("DELETE FROM attachments WHERE message_id = ?")
            .bind(&message_id)
            .execute(&mut *tx)
            .await
            .map_err(|err| err.to_string())?;
        sqlx::query("DELETE FROM messages WHERE id = ?")
            .bind(&message_id)
            .execute(&mut *tx)
            .await
            .map_err(|err| err.to_string())?;
        projection_threads.insert(thread_id.clone());
        changes.relocated_threads.insert(thread_id);
    }

    tx.commit().await.map_err(|err| err.to_string())?;
    for thread_id in projection_threads {
        refresh_provider_thread_projection(pool, &thread_id).await?;
    }
    Ok(changes)
}

/// Clear rows whose numeric IMAP UIDs became invalid after UIDVALIDITY changed.
/// Keeping them would let a reused UID overwrite an unrelated message and its
/// thread projection. Threads that still have messages in other folders live on.
pub async fn reset_local_provider_folder(
    pool: &SqlitePool,
    account_id: &str,
    folder_id: &str,
) -> Result<HashSet<String>, String> {
    reset_local_provider_folder_inner(pool, account_id, folder_id, None).await
}

/// Commit the new generation with the deletion of old UIDs. Otherwise an
/// action can read an old message joined to the new folder UIDVALIDITY and
/// mutate a different server message that reused its UID.
pub async fn reset_local_provider_folder_with_generation(
    pool: &SqlitePool,
    account_id: &str,
    folder_id: &str,
    uidvalidity: u32,
    now: &str,
) -> Result<HashSet<String>, String> {
    reset_local_provider_folder_inner(pool, account_id, folder_id, Some((uidvalidity, now))).await
}

async fn reset_local_provider_folder_inner(
    pool: &SqlitePool,
    account_id: &str,
    folder_id: &str,
    generation: Option<(u32, &str)>,
) -> Result<HashSet<String>, String> {
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|err| err.to_string())?;
    let thread_ids = sqlx::query_scalar::<_, String>(
        "SELECT DISTINCT thread_id FROM messages
         WHERE account_id = ? AND folder_id = ? AND NULLIF(TRIM(thread_id), '') IS NOT NULL",
    )
    .bind(account_id)
    .bind(folder_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(|err| err.to_string())?;

    if let Some((uidvalidity, now)) = generation {
        sqlx::query(
            "UPDATE folders SET uidvalidity = ?, updated_at = ? WHERE id = ? AND account_id = ?",
        )
        .bind(i64::from(uidvalidity))
        .bind(now)
        .bind(folder_id)
        .bind(account_id)
        .execute(&mut *tx)
        .await
        .map_err(|err| err.to_string())?;
    }
    sqlx::query(
        "DELETE FROM attachments
         WHERE message_id IN (SELECT id FROM messages WHERE account_id = ? AND folder_id = ?)",
    )
    .bind(account_id)
    .bind(folder_id)
    .execute(&mut *tx)
    .await
    .map_err(|err| err.to_string())?;
    sqlx::query("DELETE FROM messages WHERE account_id = ? AND folder_id = ?")
        .bind(account_id)
        .bind(folder_id)
        .execute(&mut *tx)
        .await
        .map_err(|err| err.to_string())?;
    tx.commit().await.map_err(|err| err.to_string())?;

    Ok(thread_ids.into_iter().collect())
}

pub async fn reconcile_remote_message_removals(
    pool: &SqlitePool,
    account_id: &str,
    removals: &[RemoteMessageRemoval],
) -> Result<HashSet<String>, String> {
    if removals.is_empty() {
        return Ok(HashSet::new());
    }

    let mut touched_threads = HashSet::new();
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|err| err.to_string())?;
    for removal in removals {
        let rows = sqlx::query_as::<_, (String, String)>(
            "SELECT m.id, m.thread_id
             FROM messages m
             JOIN folders f ON f.id = m.folder_id
             WHERE m.account_id = ? AND m.remote_id = ?
               AND (? IS NULL
                    OR LOWER(TRIM(f.remote_id)) = LOWER(TRIM(?))
                    OR LOWER(TRIM(f.id)) = LOWER(TRIM(?)))",
        )
        .bind(account_id)
        .bind(&removal.remote_id)
        .bind(removal.source_folder.as_deref())
        .bind(removal.source_folder.as_deref())
        .bind(removal.source_folder.as_deref())
        .fetch_all(&mut *tx)
        .await
        .map_err(|err| err.to_string())?;

        for (message_id, thread_id) in rows {
            sqlx::query("DELETE FROM attachments WHERE message_id = ?")
                .bind(&message_id)
                .execute(&mut *tx)
                .await
                .map_err(|err| err.to_string())?;
            sqlx::query("DELETE FROM messages WHERE id = ?")
                .bind(&message_id)
                .execute(&mut *tx)
                .await
                .map_err(|err| err.to_string())?;
            touched_threads.insert(thread_id);
        }
    }
    tx.commit().await.map_err(|err| err.to_string())?;

    for thread_id in &touched_threads {
        refresh_provider_thread_projection(pool, thread_id).await?;
    }
    Ok(touched_threads)
}

pub async fn reconcile_external_workflow_states(
    pool: &SqlitePool,
    thread_ids: &HashSet<String>,
) -> Result<(), String> {
    if thread_ids.is_empty() {
        return Ok(());
    }
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|err| err.to_string())?;
    let now = chrono::Utc::now().to_rfc3339();
    let queued_payloads = sqlx::query_scalar::<_, String>(
        "SELECT payload FROM sync_queue
         WHERE action_type = 'remote_mutation'
           AND status IN ('pending', 'sending', 'scheduled')",
    )
    .fetch_all(&mut *tx)
    .await
    .map_err(|err| err.to_string())?;
    let pending_workflow_thread_ids = queued_payloads
        .iter()
        .filter_map(|payload| remote_move_thread_id_from_json(payload))
        .collect::<HashSet<_>>();
    for thread_id in thread_ids {
        // During the Undo window the provider intentionally still exposes the
        // old location. The durable workflow move is the authoritative intent;
        // do not let a concurrent sync immediately revert the optimistic state.
        if pending_workflow_thread_ids.contains(thread_id) {
            continue;
        }
        let row = sqlx::query_as::<_, (String, Option<String>, i64, i64, i64, i64)>(
            "SELECT
                workflow_state,
                snooze_until,
                EXISTS (
                    SELECT 1 FROM messages m
                    JOIN folders f ON f.id = m.folder_id
                    WHERE m.thread_id = threads.id
                      AND LOWER(TRIM(f.folder_type)) = 'inbox'
                ) AS has_inbox,
                EXISTS (
                    SELECT 1 FROM messages m
                    JOIN folders f ON f.id = m.folder_id
                    WHERE m.thread_id = threads.id
                      AND LOWER(TRIM(f.folder_type)) = 'aside'
                ) AS has_aside,
                EXISTS (
                    SELECT 1 FROM messages m
                    JOIN folders f ON f.id = m.folder_id
                    WHERE m.thread_id = threads.id
                      AND (
                        LOWER(TRIM(COALESCE(f.remote_id, f.full_path, f.name))) = 'later'
                        OR LOWER(TRIM(COALESCE(f.remote_id, f.full_path, f.name))) LIKE '%.later'
                        OR LOWER(TRIM(COALESCE(f.remote_id, f.full_path, f.name))) LIKE '%/later'
                      )
                ) AS has_later,
                EXISTS (
                    SELECT 1 FROM messages m
                    JOIN folders f ON f.id = m.folder_id
                    WHERE m.thread_id = threads.id
                      AND LOWER(TRIM(f.folder_type)) = 'archive'
                ) AS has_archive
             FROM threads
             WHERE id = ?",
        )
        .bind(thread_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|err| err.to_string())?;
        let Some((current, snooze_until, has_inbox, has_aside, has_later, has_archive)) = row
        else {
            continue;
        };

        // Pending Satchel moves were filtered above. Without one, a real Inbox
        // copy is authoritative: it can be a new reply to a conversation whose
        // older messages are still in Aside, and that reply must resurface the
        // thread on every device and regardless of folder sync order.
        let target = if has_inbox > 0 {
            Some("inbox")
        } else if has_aside > 0 {
            Some("aside")
        } else if has_later > 0 {
            Some("snoozed")
        } else if has_archive > 0 {
            Some("done")
        } else {
            None
        };
        let Some(target) = target else {
            continue;
        };
        if current == target && (target == "snoozed" || snooze_until.is_none()) {
            continue;
        }

        sqlx::query(
            "UPDATE threads
             SET workflow_state = ?, board_column = ?, snooze_until = NULL,
                 workflow_changed_at = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(target)
        .bind(target)
        .bind(&now)
        .bind(&now)
        .bind(thread_id)
        .execute(&mut *tx)
        .await
        .map_err(|err| err.to_string())?;
    }
    tx.commit().await.map_err(|err| err.to_string())?;
    Ok(())
}

pub fn is_snooze_folder(name: &str) -> bool {
    let normalized = name.trim().to_ascii_lowercase();
    normalized == "later" || normalized.ends_with(".later") || normalized.ends_with("/later")
}

pub fn should_snapshot_folder(provider: &str, folder_type: &str, folder_name: &str) -> bool {
    let provider = provider.trim().to_ascii_lowercase();
    matches!(provider.as_str(), "imap" | "gmail" | "outlook")
        && (!folder_name.trim().is_empty() || !folder_type.trim().is_empty())
}

const SECONDARY_FOLDER_SNAPSHOT_INTERVAL_HOURS: i64 = 24;

pub fn folder_snapshot_due(
    provider: &str,
    folder_type: &str,
    folder_name: &str,
    last_snapshot_at: Option<&str>,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    if !provider.eq_ignore_ascii_case("imap")
        || folder_type == "inbox"
        || folder_type == "archive"
        || folder_type == "drafts"
        || folder_type == "aside"
        || is_snooze_folder(folder_name)
    {
        return true;
    }
    let Some(last_snapshot_at) =
        last_snapshot_at.and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
    else {
        return true;
    };
    now.signed_duration_since(last_snapshot_at.with_timezone(&chrono::Utc))
        >= chrono::Duration::hours(SECONDARY_FOLDER_SNAPSHOT_INTERVAL_HOURS)
}

#[cfg(test)]
mod tests {
    use super::{
        folder_snapshot_due, is_snooze_folder, reconcile_external_workflow_states,
        reconcile_imap_folder_snapshot, reconcile_missing_remote_folders, should_snapshot_folder,
    };
    use crate::providers::{folder_mapping::provider_folder_type, traits::RemoteMessageState};
    use crate::sync::remote_mutations::RemoteMutationPayload;
    use sqlx::sqlite::SqlitePoolOptions;
    use std::collections::HashSet;

    #[tokio::test]
    async fn failed_imap_generation_reset_keeps_the_old_generation_with_its_uids() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("pool");
        sqlx::query("CREATE TABLE folders (id TEXT PRIMARY KEY, account_id TEXT, uidvalidity INTEGER, updated_at TEXT)")
            .execute(&pool).await.expect("folders");
        sqlx::query("CREATE TABLE messages (id TEXT PRIMARY KEY, account_id TEXT, folder_id TEXT, thread_id TEXT)")
            .execute(&pool).await.expect("messages");
        sqlx::query("CREATE TABLE attachments (message_id TEXT)")
            .execute(&pool)
            .await
            .expect("attachments");
        sqlx::query("INSERT INTO folders VALUES ('inbox', 'account', 17, 'old')")
            .execute(&pool)
            .await
            .expect("folder");
        sqlx::query("INSERT INTO messages VALUES ('message', 'account', 'inbox', 'thread')")
            .execute(&pool)
            .await
            .expect("message");
        sqlx::query("CREATE TRIGGER reject_reset BEFORE DELETE ON messages BEGIN SELECT RAISE(ABORT, 'reset failed'); END")
            .execute(&pool).await.expect("trigger");

        super::reset_local_provider_folder_with_generation(&pool, "account", "inbox", 18, "new")
            .await
            .expect_err("reset must fail");

        let generation: i64 =
            sqlx::query_scalar("SELECT uidvalidity FROM folders WHERE id = 'inbox'")
                .fetch_one(&pool)
                .await
                .expect("generation");
        let old_uid_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE id = 'message'")
                .fetch_one(&pool)
                .await
                .expect("message count");
        assert_eq!((generation, old_uid_count), (17, 1));

        sqlx::query("DROP TRIGGER reject_reset")
            .execute(&pool)
            .await
            .expect("remove trigger");
        let affected = super::reset_local_provider_folder_with_generation(
            &pool, "account", "inbox", 18, "new",
        )
        .await
        .expect("reset");
        assert_eq!(affected, HashSet::from(["thread".to_string()]));
        let generation: i64 =
            sqlx::query_scalar("SELECT uidvalidity FROM folders WHERE id = 'inbox'")
                .fetch_one(&pool)
                .await
                .expect("new generation");
        let old_uid_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE id = 'message'")
                .fetch_one(&pool)
                .await
                .expect("old message count");
        assert_eq!((generation, old_uid_count), (18, 0));
    }

    #[test]
    fn inbox_descendants_are_custom_folders() {
        assert_eq!(
            provider_folder_type("imap", "INBOX", "INBOX", None),
            "inbox"
        );
        assert_eq!(
            provider_folder_type(
                "imap",
                "INBOX.Abuse - ExampleMap",
                "INBOX.Abuse - ExampleMap",
                None,
            ),
            "custom"
        );
        assert_eq!(
            provider_folder_type("imap", "INBOX/Clients", "INBOX/Clients", None),
            "custom"
        );
        assert!(is_snooze_folder("INBOX.Later"));
        assert!(is_snooze_folder("Later"));
        assert!(!is_snooze_folder("Newsletter"));
        assert!(should_snapshot_folder("gmail", "drafts", "DRAFT"));
        assert!(should_snapshot_folder("gmail", "inbox", "INBOX"));
        assert!(should_snapshot_folder("outlook", "archive", "Archive"));
        assert_eq!(
            provider_folder_type("gmail", "Aside", "Label_123", Some("user"),),
            "aside"
        );
        assert!(should_snapshot_folder("imap", "aside", "Aside"));
        assert!(should_snapshot_folder("imap", "drafts", "Entwürfe"));
        assert!(should_snapshot_folder("imap", "sent", "Gesendet"));
        assert!(should_snapshot_folder("imap", "custom", "Kunden"));
    }

    #[test]
    fn secondary_imap_snapshots_run_daily_while_drafts_remain_immediate() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-08-11T12:00:00Z")
            .expect("now")
            .with_timezone(&chrono::Utc);
        assert!(!folder_snapshot_due(
            "imap",
            "sent",
            "Sent",
            Some("2026-08-11T11:00:00Z"),
            now,
        ));
        assert!(folder_snapshot_due(
            "imap",
            "sent",
            "Sent",
            Some("2026-08-10T11:00:00Z"),
            now,
        ));
        assert!(folder_snapshot_due(
            "imap",
            "drafts",
            "Drafts",
            Some("2026-08-11T11:59:59Z"),
            now,
        ));
    }

    #[tokio::test]
    async fn a_confirmed_folder_removal_keeps_cached_mail() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("pool");
        crate::db::init(&pool).await.expect("migrations");
        sqlx::query(
            "INSERT INTO accounts
             (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
             VALUES ('account', 'Me', 'me@example.com', 'imap', '{}', '', '2026-01-01', '2026-01-01')",
        )
        .execute(&pool)
        .await
        .expect("account");
        for (id, remote_id) in [
            ("present", "INBOX"),
            ("missing", "Old.Folder"),
            ("pending", "New.Folder"),
            ("present-2", "Archive"),
            ("present-3", "Sent"),
        ] {
            sqlx::query(
                "INSERT INTO folders
                 (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at)
                 VALUES (?, 'account', ?, ?, 'custom', ?, '2026-01-01', '2026-01-01')",
            )
            .bind(id)
            .bind(remote_id)
            .bind(remote_id)
            .bind(remote_id)
            .execute(&pool)
            .await
            .expect("folder");
        }
        sqlx::query(
            "INSERT INTO messages
             (id, account_id, folder_id, from_address, to_addresses, subject, date, is_draft, created_at, updated_at)
             VALUES ('stale-message', 'account', 'missing', 'a@example.com', '[]', 'Old',
                     '2026-01-01', 0, '2026-01-01', '2026-01-01')",
        )
        .execute(&pool)
        .await
        .expect("message");
        let payload = serde_json::to_string(&RemoteMutationPayload::CreateFolder {
            path: "New.Folder".into(),
        })
        .expect("payload");
        sqlx::query(
            "INSERT INTO sync_queue
             (id, account_id, action_type, payload, status, created_at, updated_at)
             VALUES ('create', 'account', 'remote_mutation', ?, 'pending', '2026-01-01', '2026-01-01')",
        )
        .bind(payload)
        .execute(&pool)
        .await
        .expect("queue");

        for statement in [
            "INSERT INTO threads (id, account_id, subject, last_message_date, created_at, updated_at) VALUES ('kept-thread', 'account', 'Old', '2026-01-01', '', '')",
            "UPDATE messages SET thread_id = 'kept-thread' WHERE id = 'stale-message'",
            "INSERT INTO tags (id, name) VALUES ('kept-tag', 'Important')",
            "INSERT INTO thread_tags (thread_id, tag_id) VALUES ('kept-thread', 'kept-tag')",
            "INSERT INTO topics (id, name, created_at, updated_at) VALUES ('kept-topic', 'Project', '', '')",
            "INSERT INTO topic_items (topic_id, item_type, item_id, added_at) VALUES ('kept-topic', 'thread', 'kept-thread', '')",
            "INSERT INTO workflow_events (id, thread_id, from_state, to_state, reason, created_at) VALUES ('kept-event', 'kept-thread', 'todo', 'waiting', 'user', '')",
        ] { sqlx::query(statement).execute(&pool).await.unwrap(); }
        let listing = HashSet::from([
            "INBOX".to_string(),
            "Archive".to_string(),
            "Sent".to_string(),
        ]);
        reconcile_missing_remote_folders(
            &pool,
            "account",
            "imap",
            &listing,
            "2026-08-11T12:00:00Z",
        )
        .await
        .unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM folders WHERE id = 'missing'")
                .fetch_one(&pool)
                .await
                .unwrap(),
            1
        );
        reconcile_missing_remote_folders(
            &pool,
            "account",
            "imap",
            &listing,
            "2026-08-11T13:00:00Z",
        )
        .await
        .unwrap();
        let remaining: Vec<String> = sqlx::query_scalar("SELECT id FROM folders ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(
            remaining,
            vec![
                "pending",
                "present",
                "present-2",
                "present-3",
                "recovered-missing"
            ]
        );
        let folder: String =
            sqlx::query_scalar("SELECT folder_id FROM messages WHERE id = 'stale-message'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(folder, "recovered-missing");
        for table in ["thread_tags", "topic_items", "workflow_events"] {
            assert_eq!(
                sqlx::query_scalar::<_, i64>(&format!("SELECT COUNT(*) FROM {table}"))
                    .fetch_one(&pool)
                    .await
                    .unwrap(),
                1
            );
        }
    }

    #[tokio::test]
    async fn truncated_and_empty_listings_preserve_mail_and_reset_absence_streaks() {
        let pool = snapshot_pool(Some(111)).await;
        for name in ["Archive", "Sent", "Projects"] {
            sqlx::query("INSERT INTO folders (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at) VALUES (?, 'account', ?, ?, 'custom', ?, '', '')")
                .bind(name).bind(name).bind(name).bind(name).execute(&pool).await.unwrap();
        }
        for listing in [HashSet::from(["INBOX".into()]), HashSet::new()] {
            for _ in 0..3 {
                reconcile_missing_remote_folders(
                    &pool,
                    "account",
                    "imap",
                    &listing,
                    "2026-08-11T12:00:00Z",
                )
                .await
                .unwrap();
            }
            assert_eq!(snapshot_counts(&pool).await, (3, 3));
            assert_eq!(
                sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM folders")
                    .fetch_one(&pool)
                    .await
                    .unwrap(),
                4
            );
        }
        let absent = HashSet::from(["INBOX".into(), "Archive".into(), "Sent".into()]);
        let full = HashSet::from([
            "INBOX".into(),
            "Archive".into(),
            "Sent".into(),
            "Projects".into(),
        ]);
        for listing in [&absent, &full, &absent] {
            reconcile_missing_remote_folders(
                &pool,
                "account",
                "imap",
                listing,
                "2026-08-11T12:00:00Z",
            )
            .await
            .unwrap();
        }
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM folders WHERE id = 'Projects'")
                .fetch_one(&pool)
                .await
                .unwrap(),
            1
        );
    }

    #[tokio::test]
    async fn an_imap_rename_moves_mail_without_losing_local_thread_data() {
        let pool = snapshot_pool(Some(111)).await;
        sqlx::query("INSERT INTO folders (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at) VALUES ('renamed', 'account', 'Renamed', 'Renamed', 'custom', 'Renamed', '', '')").execute(&pool).await.unwrap();
        let message = serde_json::from_value(serde_json::json!({
            "remote_id":"1", "remote_thread_id":null, "message_id_header":"<1@example.com>",
            "subject":"Subject", "snippet":"", "from_address":"a@example.com", "from_name":null,
            "to_addresses":[], "cc_addresses":[], "date_rfc3339":"2026-01-01T00:00:00Z",
            "body_text":null, "body_html":null, "is_read":false, "is_starred":false, "attachments":[]
        })).unwrap();
        let batch = crate::providers::traits::SyncBatch {
            messages: vec![message],
            removals: vec![],
            remote_uidvalidity: Some(111),
            reset_folder: false,
            snapshot_required: false,
            next: Default::default(),
        };
        super::reconcile_imap_folder_rename(
            &pool,
            "account",
            "renamed",
            &HashSet::from(["Renamed".into()]),
            &batch,
        )
        .await
        .unwrap();
        assert_eq!(snapshot_counts(&pool).await, (3, 3));
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM messages WHERE folder_id = 'renamed'"
            )
            .fetch_one(&pool)
            .await
            .unwrap(),
            3
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM folders WHERE id = 'folder'")
                .fetch_one(&pool)
                .await
                .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn remote_aside_is_reconstructed_and_a_later_inbox_reply_resurfaces_it() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("pool");
        for statement in [
            "CREATE TABLE folders (id TEXT PRIMARY KEY, name TEXT, full_path TEXT, folder_type TEXT, remote_id TEXT)",
            "CREATE TABLE threads (id TEXT PRIMARY KEY, workflow_state TEXT, board_column TEXT, snooze_until TEXT, workflow_changed_at TEXT, updated_at TEXT)",
            "CREATE TABLE messages (id TEXT PRIMARY KEY, folder_id TEXT, thread_id TEXT)",
            "CREATE TABLE sync_queue (payload TEXT, action_type TEXT, status TEXT)",
        ] {
            sqlx::query(statement).execute(&pool).await.expect("schema");
        }
        sqlx::query(
            "INSERT INTO folders (id, name, full_path, folder_type, remote_id) VALUES
             ('inbox', 'Posteingang', 'Posteingang', 'inbox', 'inbox-id'),
             ('aside', 'Aside', 'Aside', 'aside', 'Label_123')",
        )
        .execute(&pool)
        .await
        .expect("folders");
        sqlx::query(
            "INSERT INTO threads (id, workflow_state, board_column, snooze_until, workflow_changed_at, updated_at)
             VALUES ('thread', 'inbox', 'inbox', NULL, '', '')",
        )
        .execute(&pool)
        .await
        .expect("thread");
        sqlx::query(
            "INSERT INTO messages (id, folder_id, thread_id) VALUES
             ('aside-copy', 'aside', 'thread')",
        )
        .execute(&pool)
        .await
        .expect("messages");

        reconcile_external_workflow_states(&pool, &HashSet::from(["thread".to_string()]))
            .await
            .expect("aside reconciliation");
        let aside: String =
            sqlx::query_scalar("SELECT workflow_state FROM threads WHERE id = 'thread'")
                .fetch_one(&pool)
                .await
                .expect("aside state");
        assert_eq!(aside, "aside");

        // A newer reply arrives in Inbox while old conversation messages keep
        // the Aside label/folder. Inbox must resurface the thread.
        sqlx::query(
            "INSERT INTO messages (id, folder_id, thread_id) VALUES
             ('inbox-copy', 'inbox', 'thread')",
        )
        .execute(&pool)
        .await
        .expect("inbox reply");
        reconcile_external_workflow_states(&pool, &HashSet::from(["thread".to_string()]))
            .await
            .expect("inbox reconciliation");
        let inbox: String =
            sqlx::query_scalar("SELECT workflow_state FROM threads WHERE id = 'thread'")
                .fetch_one(&pool)
                .await
                .expect("inbox state");
        assert_eq!(inbox, "inbox");
    }

    #[tokio::test]
    async fn pending_blocked_move_protects_done_projection_from_inbox_reconcile() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("pool");
        for statement in [
            "CREATE TABLE folders (id TEXT PRIMARY KEY, name TEXT, full_path TEXT, folder_type TEXT, remote_id TEXT)",
            "CREATE TABLE threads (id TEXT PRIMARY KEY, workflow_state TEXT, board_column TEXT, snooze_until TEXT, workflow_changed_at TEXT, updated_at TEXT)",
            "CREATE TABLE messages (id TEXT PRIMARY KEY, folder_id TEXT, thread_id TEXT)",
            "CREATE TABLE sync_queue (payload TEXT, action_type TEXT, status TEXT)",
        ] {
            sqlx::query(statement).execute(&pool).await.expect("schema");
        }
        sqlx::query(
            "INSERT INTO folders (id, name, full_path, folder_type, remote_id)
             VALUES ('inbox', 'Inbox', 'Inbox', 'inbox', 'INBOX'),
                    ('spam', 'Spam', 'Spam', 'spam', 'Spam')",
        )
        .execute(&pool)
        .await
        .expect("folders");
        sqlx::query(
            "INSERT INTO threads (id, workflow_state, board_column, snooze_until, workflow_changed_at, updated_at)
             VALUES ('thread', 'done', 'done', NULL, '', '')",
        )
        .execute(&pool)
        .await
        .expect("thread");
        sqlx::query(
            "INSERT INTO messages (id, folder_id, thread_id) VALUES ('message', 'inbox', 'thread')",
        )
        .execute(&pool)
        .await
        .expect("message");
        let payload = serde_json::to_string(&RemoteMutationPayload::BlockedMove {
            blocked_thread_id: "thread".into(),
            refs: vec![crate::providers::traits::MessageRef {
                remote_id: "42".into(),
                source_folder: Some("INBOX".into()),
                source_uidvalidity: None,
            }],
            target_remote_folder_id: "Spam".into(),
            provider_move_started: false,
            provider_move_completed: false,
            source_uidvalidity_before: None,
        })
        .expect("payload");
        sqlx::query(
            "INSERT INTO sync_queue (payload, action_type, status)
             VALUES (?, 'remote_mutation', 'pending')",
        )
        .bind(payload)
        .execute(&pool)
        .await
        .expect("queue");

        reconcile_external_workflow_states(&pool, &HashSet::from(["thread".to_string()]))
            .await
            .expect("reconcile");
        let state: String =
            sqlx::query_scalar("SELECT workflow_state FROM threads WHERE id = 'thread'")
                .fetch_one(&pool)
                .await
                .expect("state");
        assert_eq!(state, "done");
    }

    /// One folder, three messages, each with a tag of its own.
    async fn snapshot_pool(folder_uidvalidity: Option<u32>) -> sqlx::SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("pool");
        crate::db::init(&pool).await.expect("migrations");
        sqlx::query(
            "INSERT INTO accounts
             (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
             VALUES ('account', 'Me', 'me@example.com', 'imap', '{}', '', '2026-01-01', '2026-01-01')",
        )
        .execute(&pool)
        .await
        .expect("account");
        sqlx::query(
            "INSERT INTO folders
             (id, account_id, name, full_path, folder_type, remote_id, uidvalidity, created_at, updated_at)
             VALUES ('folder', 'account', 'INBOX', 'INBOX', 'inbox', 'INBOX', ?, '2026-01-01', '2026-01-01')",
        )
        .bind(folder_uidvalidity.map(i64::from))
        .execute(&pool)
        .await
        .expect("folder");
        sqlx::query("INSERT INTO tags (id, name) VALUES ('tag', 'Important')")
            .execute(&pool)
            .await
            .expect("tag");
        for uid in ["1", "2", "3"] {
            sqlx::query(
                "INSERT INTO threads
                 (id, account_id, subject, last_message_date, created_at, updated_at)
                 VALUES (?, 'account', 'Subject', '2026-01-01', '2026-01-01', '2026-01-01')",
            )
            .bind(format!("thread-{uid}"))
            .execute(&pool)
            .await
            .expect("thread");
            sqlx::query(
                "INSERT INTO messages
                 (id, account_id, folder_id, thread_id, message_id_header, remote_id,
                  from_address, to_addresses, subject, date, created_at, updated_at)
                 VALUES (?, 'account', 'folder', ?, ?, ?, 'a@example.com', '[]', 'Subject',
                         '2026-01-01', '2026-01-01', '2026-01-01')",
            )
            .bind(format!("message-{uid}"))
            .bind(format!("thread-{uid}"))
            .bind(format!("<{uid}@example.com>"))
            .bind(uid)
            .execute(&pool)
            .await
            .expect("message");
            sqlx::query("INSERT INTO thread_tags (thread_id, tag_id) VALUES (?, 'tag')")
                .bind(format!("thread-{uid}"))
                .execute(&pool)
                .await
                .expect("thread tag");
        }
        pool
    }

    async fn snapshot_counts(pool: &sqlx::SqlitePool) -> (i64, i64) {
        let messages: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages")
            .fetch_one(pool)
            .await
            .expect("messages");
        let tags: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM thread_tags")
            .fetch_one(pool)
            .await
            .expect("thread tags");
        (messages, tags)
    }

    fn snapshot_states(remote_ids: &[&str]) -> Vec<RemoteMessageState> {
        remote_ids
            .iter()
            .map(|remote_id| RemoteMessageState {
                remote_id: (*remote_id).to_string(),
                is_read: Some(false),
                is_starred: Some(false),
            })
            .collect()
    }

    /// A snapshot from another UIDVALIDITY names UIDs that mean nothing here.
    /// Treating it as authoritative wiped the folder — and with it the tags,
    /// topics and workflow events of every thread that lived only there.
    #[tokio::test]
    async fn imap_snapshot_with_a_changed_generation_deletes_nothing() {
        let pool = snapshot_pool(Some(111)).await;

        let changes = reconcile_imap_folder_snapshot(
            &pool,
            "account",
            "folder",
            "INBOX",
            &snapshot_states(&["70", "71"]),
            Some(222),
        )
        .await
        .expect("reconcile");

        assert!(changes.skipped_stale_generation);
        assert!(changes.relocated_threads.is_empty());
        assert_eq!(snapshot_counts(&pool).await, (3, 3));
    }

    #[tokio::test]
    async fn imap_snapshot_without_a_generation_deletes_nothing() {
        let pool = snapshot_pool(Some(111)).await;

        let changes = reconcile_imap_folder_snapshot(
            &pool,
            "account",
            "folder",
            "INBOX",
            &snapshot_states(&["1", "2"]),
            None,
        )
        .await
        .expect("reconcile");

        assert!(changes.skipped_stale_generation);
        assert_eq!(snapshot_counts(&pool).await, (3, 3));
    }

    #[tokio::test]
    async fn imap_snapshot_still_reconciles_within_one_generation() {
        let pool = snapshot_pool(Some(111)).await;

        let changes = reconcile_imap_folder_snapshot(
            &pool,
            "account",
            "folder",
            "INBOX",
            &snapshot_states(&["1", "3"]),
            Some(111),
        )
        .await
        .expect("reconcile");

        assert!(!changes.skipped_stale_generation);
        assert_eq!(
            changes.relocated_threads,
            HashSet::from(["thread-2".into()])
        );
        let remaining: Vec<String> =
            sqlx::query_scalar("SELECT remote_id FROM messages ORDER BY remote_id")
                .fetch_all(&pool)
                .await
                .expect("messages");
        assert_eq!(remaining, vec!["1".to_string(), "3".to_string()]);
    }

    #[tokio::test]
    async fn imap_snapshot_reconciles_flags_moves_and_workflow_states() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("pool");
        for statement in [
            "CREATE TABLE folders (
               id TEXT PRIMARY KEY,
               name TEXT,
               full_path TEXT,
               folder_type TEXT,
               remote_id TEXT,
               uidvalidity INTEGER
             )",
            "CREATE TABLE threads (
               id TEXT PRIMARY KEY,
               workflow_state TEXT,
               board_column TEXT,
               snooze_until TEXT,
               workflow_changed_at TEXT,
               updated_at TEXT,
               message_count INTEGER,
               unread_count INTEGER,
               is_starred INTEGER,
               has_attachments INTEGER,
               subject TEXT,
               snippet TEXT,
               last_message_date TEXT
             )",
            "CREATE TABLE messages (
               id TEXT PRIMARY KEY,
               account_id TEXT,
               folder_id TEXT,
               thread_id TEXT,
               message_id_header TEXT,
               remote_id TEXT,
               is_read INTEGER,
               is_starred INTEGER,
               is_draft INTEGER DEFAULT 0,
               has_attachments INTEGER DEFAULT 0,
               subject TEXT,
               snippet TEXT,
               date TEXT,
               updated_at TEXT
             )",
            "CREATE TABLE attachments (id TEXT PRIMARY KEY, message_id TEXT)",
            "CREATE TABLE sync_queue (
               account_id TEXT,
               action_type TEXT,
               payload TEXT,
               status TEXT
             )",
        ] {
            sqlx::query(statement).execute(&pool).await.expect("schema");
        }

        for (id, name, kind) in [
            ("inbox", "INBOX", "inbox"),
            ("archive", "INBOX.Archive", "archive"),
            ("later", "INBOX.Later", "custom"),
        ] {
            sqlx::query(
                "INSERT INTO folders (id, name, full_path, folder_type, remote_id, uidvalidity)
                 VALUES (?, ?, ?, ?, ?, 111)",
            )
            .bind(id)
            .bind(name)
            .bind(name)
            .bind(kind)
            .bind(name)
            .execute(&pool)
            .await
            .expect("folder");
        }
        for (id, state) in [
            ("thread-flags", "inbox"),
            ("thread-pending", "inbox"),
            ("thread-completed-move", "done"),
            ("thread-done", "inbox"),
            ("thread-snooze", "inbox"),
        ] {
            sqlx::query(
                "INSERT INTO threads
                 (id, workflow_state, board_column, snooze_until, workflow_changed_at,
                  updated_at, message_count, unread_count, is_starred)
                 VALUES (?, ?, ?, NULL, '', '', 1, 1, 0)",
            )
            .bind(id)
            .bind(state)
            .bind(state)
            .execute(&pool)
            .await
            .expect("thread");
        }
        for (id, folder, thread, header, remote_id) in [
            ("m-imap-flags", "inbox", "thread-flags", "<flags>", "1"),
            (
                "m-imap-pending",
                "inbox",
                "thread-pending",
                "<pending>",
                "5",
            ),
            (
                "m-imap-completed-move",
                "inbox",
                "thread-completed-move",
                "<completed-move>",
                "6",
            ),
            ("m-imap-done-old", "inbox", "thread-done", "<done>", "2"),
            ("m-imap-done-new", "archive", "thread-done", "<done>", "20"),
            (
                "m-imap-snooze-old",
                "inbox",
                "thread-snooze",
                "<snooze>",
                "3",
            ),
            (
                "m-imap-snooze-new",
                "later",
                "thread-snooze",
                "<snooze>",
                "30",
            ),
        ] {
            sqlx::query(
                "INSERT INTO messages
                 (id, account_id, folder_id, thread_id, message_id_header, remote_id,
                  is_read, is_starred, updated_at)
                 VALUES (?, 'account', ?, ?, ?, ?, 0, 0, '')",
            )
            .bind(id)
            .bind(folder)
            .bind(thread)
            .bind(header)
            .bind(remote_id)
            .execute(&pool)
            .await
            .expect("message");
        }
        sqlx::query(
            "INSERT INTO attachments (id, message_id) VALUES ('attachment', 'm-imap-done-old')",
        )
        .execute(&pool)
        .await
        .expect("attachment");
        sqlx::query(
            "INSERT INTO sync_queue (account_id, action_type, payload, status)
             VALUES (
               'account',
               'remote_mutation',
               '{\"kind\":\"flags\",\"refs\":[{\"remote_id\":\"5\",\"source_folder\":\"INBOX\"}],\"read\":true,\"starred\":null}',
               'pending'
             )",
        )
        .execute(&pool)
        .await
        .expect("pending mutation");
        sqlx::query(
            r#"INSERT INTO sync_queue (account_id, action_type, payload, status)
             VALUES (
               'account',
               'remote_mutation',
               '{"kind":"workflow_move","workflow_thread_id":"thread-completed-move","source_folder_type":"inbox","target_folder_type":"archive","refs":[{"remote_id":"6","source_folder":"INBOX"}],"target_remote_folder_id":"INBOX.Archive","provider_move_completed":true}',
               'failed'
             )"#,
        )
        .execute(&pool)
        .await
        .expect("completed workflow move");

        let changes = reconcile_imap_folder_snapshot(
            &pool,
            "account",
            "inbox",
            "INBOX",
            &[
                RemoteMessageState {
                    remote_id: "1".to_string(),
                    is_read: Some(true),
                    is_starred: Some(true),
                },
                RemoteMessageState {
                    remote_id: "5".to_string(),
                    is_read: Some(true),
                    is_starred: Some(false),
                },
            ],
            Some(111),
        )
        .await
        .expect("reconcile inbox");
        reconcile_external_workflow_states(&pool, &changes.relocated_threads)
            .await
            .expect("workflow reconciliation");

        let flags = sqlx::query_as::<_, (i64, i64)>(
            "SELECT is_read, is_starred FROM messages WHERE id = 'm-imap-flags'",
        )
        .fetch_one(&pool)
        .await
        .expect("flags");
        let pending_read: i64 =
            sqlx::query_scalar("SELECT is_read FROM messages WHERE id = 'm-imap-pending'")
                .fetch_one(&pool)
                .await
                .expect("pending read state");
        let completed_source_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE id = 'm-imap-completed-move'")
                .fetch_one(&pool)
                .await
                .expect("completed move source");
        let workflows = sqlx::query_as::<_, (String, String, String)>(
            "SELECT
               (SELECT workflow_state FROM threads WHERE id = 'thread-flags'),
               (SELECT workflow_state FROM threads WHERE id = 'thread-done'),
               (SELECT workflow_state FROM threads WHERE id = 'thread-snooze')",
        )
        .fetch_one(&pool)
        .await
        .expect("workflows");
        let attachments: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM attachments")
            .fetch_one(&pool)
            .await
            .expect("attachments");
        assert_eq!(flags, (1, 1));
        assert_eq!(pending_read, 0);
        assert_eq!(completed_source_count, 1);
        assert_eq!(workflows, ("inbox".into(), "done".into(), "snoozed".into()));
        assert_eq!(attachments, 0);

        sqlx::query(
            "INSERT INTO messages
             (id, account_id, folder_id, thread_id, message_id_header, remote_id,
              is_read, is_starred, updated_at)
             VALUES ('m-imap-done-returned', 'account', 'inbox', 'thread-done',
                     '<done>', '4', 1, 0, '')",
        )
        .execute(&pool)
        .await
        .expect("returned inbox copy");
        let changes = reconcile_imap_folder_snapshot(
            &pool,
            "account",
            "archive",
            "INBOX.Archive",
            &[],
            Some(111),
        )
        .await
        .expect("reconcile archive");
        reconcile_external_workflow_states(&pool, &changes.relocated_threads)
            .await
            .expect("returned workflow");
        let returned: String =
            sqlx::query_scalar("SELECT workflow_state FROM threads WHERE id = 'thread-done'")
                .fetch_one(&pool)
                .await
                .expect("returned state");
        assert_eq!(returned, "inbox");
    }
}
