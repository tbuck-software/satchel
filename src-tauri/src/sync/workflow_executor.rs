//! Execute queued remote mutations, including crash-safe workflow moves.

use std::collections::{HashMap, HashSet};

use sqlx::SqlitePool;
use tauri::AppHandle;

use crate::sync::imap_workflow_target::{
    present_imap_sources_without_generation, present_imap_workflow_sources,
    reconcile_exact_imap_workflow_target, snapshot_exact_imap_target_ids,
    snapshot_imap_source_generations, validate_imap_source_generations,
};
use crate::sync::remote_mutations::{execute_remote_mutation, RemoteMutationPayload};
use crate::sync::thread_resolution::refresh_provider_thread_projection;
use crate::AppState;
use crate::{
    providers::traits::MessageRef,
    providers::{gmail::GmailProvider, provider_for_account},
};

/// Finish workflow moves before removing their durable queue row. IMAP assigns
/// a new UID in the destination mailbox, so its target folder must be synced
/// immediately. The completed bit makes reconciliation retries safe: once the
/// provider accepted the move, a later attempt never submits the stale source
/// UID again.
pub(crate) async fn execute_queued_remote_mutation(
    pool: &SqlitePool,
    app: Option<&AppHandle>,
    state: &AppState,
    action: &crate::sync::queue::QueuedAction,
) -> crate::sync::errors::SyncResult<()> {
    let _work = state.sync_activity.start_work();
    let mutation = serde_json::from_value::<RemoteMutationPayload>(action.payload.clone())
        .map_err(|err| format!("decode remote mutation: {err}"))?;
    if let RemoteMutationPayload::UndoTrash { receipt_id } = &mutation {
        satchel_core::mail_actions::undo::execute_queued_undo(pool, &state.queue, receipt_id)
            .await
            .map_err(|error| {
                if error == "backend.errors.mailUndoUnavailable" {
                    crate::sync::errors::SyncError::Permanent(error)
                } else {
                    error.into()
                }
            })?;
        return Ok(());
    }
    let is_blocked_move = matches!(&mutation, RemoteMutationPayload::BlockedMove { .. });
    let mut workflow_move = match mutation {
        RemoteMutationPayload::WorkflowMove {
            workflow_thread_id,
            target_folder_type,
            refs,
            target_remote_folder_id,
            provider_move_started,
            source_uidvalidity_before,
            provider_move_completed,
            target_remote_ids_before,
            target_uidvalidity_before,
            projection_completed,
            ..
        } => Some((
            workflow_thread_id,
            target_folder_type,
            refs,
            target_remote_folder_id,
            provider_move_started,
            source_uidvalidity_before,
            provider_move_completed,
            target_remote_ids_before,
            target_uidvalidity_before,
            projection_completed,
        )),
        RemoteMutationPayload::BlockedMove {
            blocked_thread_id,
            refs,
            target_remote_folder_id,
            provider_move_started,
            source_uidvalidity_before,
            provider_move_completed,
        } => Some((
            blocked_thread_id,
            String::new(),
            refs,
            target_remote_folder_id,
            provider_move_started,
            source_uidvalidity_before,
            provider_move_completed,
            // Blocked moves already committed their local thread projection.
            // Empty target metadata prevents the workflow-only target
            // reconciliation from running while retaining the same
            // crash-safe provider half and IMAP source reconciliation.
            Some(Vec::new()),
            Some(0),
            true,
        )),
        _ => None,
    };
    let provider_kind: Option<String> = if workflow_move.is_some() {
        sqlx::query_scalar("SELECT provider FROM accounts WHERE id = ? LIMIT 1")
            .bind(&action.account_id)
            .fetch_optional(pool)
            .await
            .map_err(|err| err.to_string())?
    } else {
        None
    };
    let mut execution_payload = action.payload.clone();
    if let Some((
        _,
        _,
        refs,
        target_remote_folder_id,
        was_started,
        source_validity,
        was_completed,
        before,
        validity,
        _,
    )) = workflow_move.as_mut()
    {
        if !*was_completed
            && (before.is_none() || validity.is_none())
            && provider_kind
                .as_deref()
                .is_some_and(|provider| provider.eq_ignore_ascii_case("imap"))
        {
            let (snapshot, uidvalidity) =
                snapshot_exact_imap_target_ids(pool, &action.account_id, target_remote_folder_id)
                    .await?;
            set_workflow_payload_field(
                &mut execution_payload,
                "target_remote_ids_before",
                serde_json::to_value(&snapshot).map_err(|err| err.to_string())?,
            )?;
            set_workflow_payload_field(
                &mut execution_payload,
                "target_uidvalidity_before",
                uidvalidity.into(),
            )?;
            persist_workflow_move_payload(pool, action, &execution_payload).await?;
            *before = Some(snapshot);
            *validity = Some(uidvalidity);
        }
        if !*was_completed
            && !*was_started
            && source_validity.is_none()
            && provider_kind
                .as_deref()
                .is_some_and(|provider| provider.eq_ignore_ascii_case("imap"))
        {
            let generations =
                snapshot_imap_source_generations(pool, &action.account_id, refs).await?;
            set_workflow_payload_field(
                &mut execution_payload,
                "source_uidvalidity_before",
                serde_json::to_value(&generations).map_err(|err| err.to_string())?,
            )?;
            persist_workflow_move_payload(pool, action, &execution_payload).await?;
            *source_validity = Some(generations);
        }
    }

    let mut skip_provider_call = false;
    let mut provider_payload = execution_payload.clone();
    if is_blocked_move
        && !workflow_move
            .as_ref()
            .is_some_and(|(_, _, _, _, _, _, completed, _, _, _)| *completed)
        && provider_kind
            .as_deref()
            .is_some_and(|provider| provider.eq_ignore_ascii_case("imap"))
    {
        let has_source_generation = workflow_move.as_ref().is_some_and(
            |(_, _, refs, _, _, source_validity, _, _, _, _)| {
                source_validity.is_some()
                    && refs
                        .iter()
                        .all(|message| message.source_uidvalidity.is_some())
            },
        );
        if !has_source_generation {
            return Err(
                "IMAP blocked move has no source UIDVALIDITY; refusing stale UID retry".into(),
            );
        }
    }
    if let Some((_, _, refs, _, was_started, source_validity, was_completed, _, _, _)) =
        workflow_move.as_ref()
    {
        if !*was_completed
            && !*was_started
            && provider_kind
                .as_deref()
                .is_some_and(|provider| provider.eq_ignore_ascii_case("imap"))
            && source_validity.is_some()
            && refs
                .iter()
                .all(|message| message.source_uidvalidity.is_some())
        {
            if let Some(expected_generations) = source_validity.as_ref() {
                validate_imap_source_generations(
                    pool,
                    &action.account_id,
                    refs,
                    expected_generations,
                )
                .await?;
            }
        }
    }
    if let Some((
        _,
        _,
        refs,
        target_remote_folder_id,
        was_started,
        source_validity,
        was_completed,
        _,
        _,
        _,
    )) = workflow_move.as_ref()
    {
        if !*was_completed && *was_started {
            let remaining = match provider_kind.as_deref() {
                Some(provider) if provider.eq_ignore_ascii_case("imap") => {
                    let remaining = if let Some(expected_generations) = source_validity.as_ref() {
                        present_imap_workflow_sources(
                            pool,
                            &action.account_id,
                            refs,
                            expected_generations,
                        )
                        .await?
                    } else {
                        present_imap_sources_without_generation(pool, &action.account_id, refs)
                            .await?
                    };
                    if source_validity.is_none() && !remaining.is_empty() {
                        return Err(
                            "legacy IMAP blocked move remains ambiguous; refusing stale UID retry"
                                .into(),
                        );
                    }
                    Some(remaining)
                }
                Some(provider)
                    if provider.eq_ignore_ascii_case("gmail")
                        || provider.eq_ignore_ascii_case("outlook") =>
                {
                    Some(
                        present_api_move_sources(
                            pool,
                            &action.account_id,
                            refs,
                            target_remote_folder_id,
                        )
                        .await?,
                    )
                }
                _ => None,
            };
            if let Some(remaining) = remaining {
                if should_skip_started_move(
                    provider_kind.as_deref(),
                    *was_started,
                    *was_completed,
                    remaining.is_empty(),
                ) {
                    skip_provider_call = true;
                } else {
                    set_workflow_payload_field(
                        &mut provider_payload,
                        "refs",
                        serde_json::to_value(remaining).map_err(|err| err.to_string())?,
                    )?;
                }
            }
        }
    }
    if let Some((_, _, _, _, was_started, _, was_completed, _, _, _)) = workflow_move.as_mut() {
        if !*was_completed && !*was_started {
            set_workflow_payload_field(
                &mut execution_payload,
                "provider_move_started",
                true.into(),
            )?;
            persist_workflow_move_payload(pool, action, &execution_payload).await?;
            *was_started = true;
        }
    }
    if !skip_provider_call {
        execute_remote_mutation(pool, &action.account_id, provider_payload).await?;
    }

    if let Some((
        thread_id,
        target_folder_type,
        refs,
        target_remote_folder_id,
        _,
        source_uidvalidity_before,
        was_completed,
        target_remote_ids_before,
        target_uidvalidity_before,
        projection_completed,
    )) = workflow_move
    {
        // This is intentionally the very first fallible operation after the
        // provider call. If projection or target reconciliation fails later,
        // the retry sees `true` and never submits the stale source UID again.
        if !was_completed {
            set_workflow_payload_field(
                &mut execution_payload,
                "provider_move_completed",
                true.into(),
            )?;
            persist_workflow_move_payload(pool, action, &execution_payload).await?;
        }

        if projection_completed {
            refresh_provider_thread_projection(pool, &thread_id).await?;
            return Ok(());
        }

        if provider_kind
            .as_deref()
            .is_some_and(|provider| provider.eq_ignore_ascii_case("imap"))
        {
            let legacy_empty_snapshot = Vec::new();
            let target_remote_ids_before = match target_remote_ids_before.as_deref() {
                Some(ids) => ids,
                None if was_completed => &legacy_empty_snapshot,
                None => {
                    return Err("IMAP workflow move has no pre-move target UID snapshot".into());
                }
            };
            reconcile_exact_imap_workflow_target(
                pool,
                app,
                state,
                &action.account_id,
                &thread_id,
                &refs,
                &target_folder_type,
                &target_remote_folder_id,
                target_remote_ids_before,
                // A legacy row may have reached provider completion before
                // UIDVALIDITY was persisted. Generation zero forces the safe,
                // unambiguous identity-reconciliation path without resubmitting.
                target_uidvalidity_before.unwrap_or(0),
                source_uidvalidity_before.as_ref().ok_or_else(|| {
                    "IMAP workflow move has no pre-move source UIDVALIDITY snapshot".to_string()
                })?,
                &action.id,
                &execution_payload,
            )
            .await?;
        } else {
            let target_id = crate::sync::remote_mutations::ensure_local_workflow_target_folder(
                pool,
                &action.account_id,
                &target_folder_type,
                &target_remote_folder_id,
            )
            .await?;
            crate::sync::remote_mutations::apply_completed_move_projection(
                pool,
                &action.account_id,
                &refs,
                &target_id,
            )
            .await?;
            set_workflow_payload_field(
                &mut execution_payload,
                "projection_completed",
                true.into(),
            )?;
            persist_workflow_move_payload(pool, action, &execution_payload).await?;
        }
    }

    Ok(())
}

/// A move accepted before the process lost its response must not submit stale
/// source ids a second time when the provider exposes a source listing.
fn should_skip_started_move(
    provider_kind: Option<&str>,
    provider_move_started: bool,
    provider_move_completed: bool,
    source_refs_empty: bool,
) -> bool {
    provider_kind.is_some_and(|provider| {
        provider.eq_ignore_ascii_case("imap")
            || provider.eq_ignore_ascii_case("gmail")
            || provider.eq_ignore_ascii_case("outlook")
    }) && provider_move_started
        && !provider_move_completed
        && source_refs_empty
}

/// Gmail message ids and Outlook immutable ids are stable across moves. Their
/// folder listings therefore provide the same completion signal as IMAP's
/// UID snapshot without needing provider-specific target reconciliation.
async fn present_api_move_sources(
    pool: &SqlitePool,
    account_id: &str,
    refs: &[MessageRef],
    target_remote_folder_id: &str,
) -> crate::sync::errors::SyncResult<Vec<MessageRef>> {
    if refs.iter().any(|message| {
        message
            .source_folder
            .as_deref()
            .map(str::trim)
            .is_none_or(str::is_empty)
    }) {
        return Ok(refs.to_vec());
    }

    let (provider_kind, email, encrypted_config): (String, String, String) = sqlx::query_as(
        "SELECT provider, email, config FROM accounts
         WHERE id = ? AND is_active = 1 LIMIT 1",
    )
    .bind(account_id)
    .fetch_one(pool)
    .await
    .map_err(|err| format!("remote move account lookup failed: {err}"))?;
    let config = crate::commands::accounts::decrypt_account_config(account_id, &encrypted_config)?;
    let provider = provider_for_account(&provider_kind, &config, &email, None)?;

    let mut refs_by_folder: HashMap<&str, Vec<&MessageRef>> = HashMap::new();
    for message in refs {
        let Some(folder) = message.source_folder.as_deref().map(str::trim) else {
            return Ok(refs.to_vec());
        };
        refs_by_folder.entry(folder).or_default().push(message);
    }

    let gmail_provider = if provider_kind.eq_ignore_ascii_case("gmail")
        && refs_by_folder
            .keys()
            .any(|folder| folder.eq_ignore_ascii_case("ARCHIVE"))
    {
        Some(GmailProvider::from_account_config(&config))
    } else {
        None
    };

    let mut remaining = Vec::new();
    for (folder, folder_refs) in refs_by_folder {
        if folder.eq_ignore_ascii_case("ARCHIVE") && provider_kind.eq_ignore_ascii_case("gmail") {
            let gmail = gmail_provider
                .as_ref()
                .ok_or_else(|| "Gmail archive label lookup is unavailable".to_string())?;
            for message in folder_refs {
                let detail = gmail.fetch_message_by_id(&message.remote_id).await?;
                if !gmail_target_label_is_present(&detail.mailbox_ids, target_remote_folder_id) {
                    remaining.push(message.clone());
                }
            }
            continue;
        }
        let Some(snapshot) = provider.sync_message_state_snapshot(folder).await? else {
            // A provider without a presence snapshot retains its native
            // retry semantics; never infer completion from missing capability.
            return Ok(refs.to_vec());
        };
        let present = snapshot
            .states
            .into_iter()
            .map(|state| state.remote_id)
            .collect::<HashSet<_>>();
        remaining.extend(present_message_refs(folder_refs, &present));
    }
    Ok(remaining)
}

fn gmail_target_label_is_present(labels: &[String], target_remote_folder_id: &str) -> bool {
    let target = target_remote_folder_id.trim();
    if target.eq_ignore_ascii_case("ARCHIVE") {
        return !labels
            .iter()
            .any(|label| label.eq_ignore_ascii_case("INBOX"));
    }
    labels
        .iter()
        .any(|label| label.eq_ignore_ascii_case(target))
}

fn present_message_refs(refs: Vec<&MessageRef>, present: &HashSet<String>) -> Vec<MessageRef> {
    refs.into_iter()
        .filter(|message| present.contains(&message.remote_id))
        .cloned()
        .collect()
}

pub(crate) fn set_workflow_payload_field(
    payload: &mut serde_json::Value,
    key: &str,
    value: serde_json::Value,
) -> Result<(), String> {
    payload
        .as_object_mut()
        .ok_or_else(|| "workflow move payload is not an object".to_string())?
        .insert(key.to_string(), value);
    Ok(())
}

async fn persist_workflow_move_payload(
    pool: &SqlitePool,
    action: &crate::sync::queue::QueuedAction,
    payload: &serde_json::Value,
) -> Result<(), String> {
    let changed = sqlx::query(
        "UPDATE sync_queue SET payload = ?, updated_at_ms = ?
         WHERE id = ? AND status = 'sending'",
    )
    .bind(payload.to_string())
    .bind(chrono::Utc::now().timestamp_millis())
    .bind(&action.id)
    .execute(pool)
    .await
    .map_err(|err| err.to_string())?
    .rows_affected();
    if changed != 1 {
        return Err("workflow move queue row disappeared before completion was recorded".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::{
        gmail_target_label_is_present, persist_workflow_move_payload, present_message_refs,
        set_workflow_payload_field, should_skip_started_move,
    };
    use crate::{
        providers::traits::MessageRef,
        sync::{queue::QueuedAction, remote_mutations::RemoteMutationPayload},
    };
    use sqlx::sqlite::SqlitePoolOptions;

    #[test]
    fn ambiguous_api_moves_are_not_submitted_again_when_sources_are_gone() {
        for provider in ["imap", "IMAP", "gmail", "GMAIL", "outlook", "OUTLOOK"] {
            assert!(should_skip_started_move(Some(provider), true, false, true));
        }
        assert!(!should_skip_started_move(Some("imap"), true, false, false));
        assert!(!should_skip_started_move(Some("gmail"), true, true, true));
        assert!(!should_skip_started_move(
            Some("outlook"),
            true,
            false,
            false
        ));
        assert!(!should_skip_started_move(Some("pop3"), true, false, true));
    }

    #[test]
    fn gmail_archived_source_to_custom_abuse_uses_target_label_membership() {
        let abuse = vec![String::from("Label_Abuse")];
        assert!(gmail_target_label_is_present(&abuse, "Label_Abuse"));
        assert!(!gmail_target_label_is_present(&abuse, "Label_Other"));
        assert!(!gmail_target_label_is_present(
            &[String::from("INBOX")],
            "Label_Abuse"
        ));
    }

    #[test]
    fn source_presence_snapshot_keeps_present_ids_and_drops_absent_ids() {
        let present = HashSet::from([String::from("present")]);
        let present_ref = MessageRef {
            remote_id: "present".into(),
            source_folder: Some("ARCHIVE".into()),
            source_uidvalidity: None,
        };
        let absent_ref = MessageRef {
            remote_id: "absent".into(),
            source_folder: Some("ARCHIVE".into()),
            source_uidvalidity: None,
        };
        let refs = vec![&present_ref, &absent_ref];
        let remaining = present_message_refs(refs, &present);
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].remote_id, "present");
    }

    #[test]
    fn blocked_move_carries_crash_safe_provider_state() {
        let payload = serde_json::to_value(RemoteMutationPayload::BlockedMove {
            blocked_thread_id: "thread".into(),
            refs: vec![MessageRef {
                remote_id: "101".into(),
                source_folder: Some("INBOX".into()),
                source_uidvalidity: None,
            }],
            target_remote_folder_id: "Spam".into(),
            provider_move_started: false,
            provider_move_completed: false,
            source_uidvalidity_before: None,
        })
        .expect("blocked move payload");
        let decoded: RemoteMutationPayload =
            serde_json::from_value(payload).expect("blocked move payload decode");
        match decoded {
            RemoteMutationPayload::BlockedMove {
                provider_move_started,
                provider_move_completed,
                source_uidvalidity_before,
                ..
            } => {
                assert!(!provider_move_started);
                assert!(!provider_move_completed);
                assert!(source_uidvalidity_before.is_none());
            }
            other => panic!("unexpected payload: {other:?}"),
        }
    }

    #[tokio::test]
    async fn provider_completion_survives_a_following_projection_failure() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("pool");
        sqlx::query(
            "CREATE TABLE sync_queue (
               id TEXT PRIMARY KEY, payload TEXT, status TEXT, updated_at_ms INTEGER
             );
             CREATE TABLE folders (
               id TEXT PRIMARY KEY, account_id TEXT, folder_type TEXT, updated_at TEXT
             );",
        )
        .execute(&pool)
        .await
        .expect("schema");
        let payload = serde_json::to_value(RemoteMutationPayload::WorkflowMove {
            workflow_thread_id: "thread".into(),
            source_folder_type: "inbox".into(),
            target_folder_type: "archive".into(),
            refs: vec![MessageRef {
                remote_id: "101".into(),
                source_folder: Some("INBOX".into()),
                source_uidvalidity: None,
            }],
            target_remote_folder_id: "Archive".into(),
            provider_move_started: false,
            source_uidvalidity_before: None,
            provider_move_completed: false,
            target_remote_ids_before: None,
            target_uidvalidity_before: None,
            projection_completed: false,
        })
        .expect("payload");
        let mut action = QueuedAction::new("account".into(), "remote_mutation", payload);
        action.id = "move".into();
        action.status = "sending".into();
        sqlx::query(
            "INSERT INTO sync_queue (id, payload, status, updated_at_ms)
             VALUES (?, ?, 'sending', 0)",
        )
        .bind(&action.id)
        .bind(action.payload.to_string())
        .execute(&pool)
        .await
        .expect("queue row");

        let mut completed_payload = action.payload.clone();
        set_workflow_payload_field(
            &mut completed_payload,
            "provider_move_completed",
            true.into(),
        )
        .expect("completed payload");
        persist_workflow_move_payload(&pool, &action, &completed_payload)
            .await
            .expect("persist provider completion");
        crate::sync::remote_mutations::apply_completed_workflow_move_projection(
            &pool,
            "account",
            &[MessageRef {
                remote_id: "101".into(),
                source_folder: Some("INBOX".into()),
                source_uidvalidity: None,
            }],
            "archive",
        )
        .await
        .expect_err("projection has no local target");

        let stored: String = sqlx::query_scalar("SELECT payload FROM sync_queue WHERE id = 'move'")
            .fetch_one(&pool)
            .await
            .expect("stored payload");
        match serde_json::from_str::<RemoteMutationPayload>(&stored).expect("stored mutation") {
            RemoteMutationPayload::WorkflowMove {
                provider_move_completed,
                ..
            } => assert!(provider_move_completed),
            other => panic!("unexpected payload: {other:?}"),
        }
    }
}
