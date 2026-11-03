//! Last-write-wins for queued flags, including manually retried failed actions.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use sqlx::{Sqlite, Transaction};

use super::{queue::QueuedAction, remote_mutations::RemoteMutationPayload};
use crate::providers::traits::{FlagUpdate, MessageRef};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlagSupersession {
    pub reference_index: usize,
    pub read: bool,
    pub starred: bool,
}

/// At most three provider calls: read, star, or both. A newer read choice must
/// not discard the older action's star choice or flags for another message.
pub fn batches(
    refs: Vec<MessageRef>,
    read: Option<bool>,
    starred: Option<bool>,
    superseded: &[FlagSupersession],
) -> Vec<(Vec<MessageRef>, FlagUpdate)> {
    let masks: HashMap<_, _> = superseded
        .iter()
        .map(|mask| (mask.reference_index, (mask.read, mask.starred)))
        .collect();
    let mut groups = HashMap::<_, Vec<MessageRef>>::new();
    for (index, reference) in refs.into_iter().enumerate() {
        let (skip_read, skip_star) = masks.get(&index).copied().unwrap_or_default();
        let flags = (read.filter(|_| !skip_read), starred.filter(|_| !skip_star));
        if flags.0.is_some() || flags.1.is_some() {
            groups.entry(flags).or_default().push(reference);
        }
    }
    groups
        .into_iter()
        .map(|((read, starred), refs)| (refs, FlagUpdate { read, starred }))
        .collect()
}

/// The write reservation covers supersession and the new queue row together.
/// The running request keeps its already cloned payload. Its durable retry
/// payload records supersession too, in case that request fails after a newer
/// choice was queued. The account worker runs the newer choice afterwards.
pub async fn supersede(
    tx: &mut Transaction<'_, Sqlite>,
    action: &QueuedAction,
) -> Result<(), String> {
    if action.action_type != super::remote_mutations::ACTION_REMOTE_MUTATION
        || !matches!(action.status.as_str(), "pending" | "scheduled")
    {
        return Ok(());
    }
    let Ok(RemoteMutationPayload::Flags {
        refs,
        read,
        starred,
        ..
    }) = serde_json::from_value(action.payload.clone())
    else {
        return Ok(());
    };
    let provider: Option<String> = sqlx::query_scalar("SELECT provider FROM accounts WHERE id = ?")
        .bind(&action.account_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| e.to_string())?;
    let global_ids = matches!(provider.as_deref(), Some("gmail" | "outlook"));
    let identity = |reference: &MessageRef| {
        (
            reference.remote_id.clone(),
            if global_ids {
                None
            } else {
                reference.source_folder.as_ref().map(|folder| {
                    // Only INBOX is case-insensitive in IMAP. "Work" and
                    // "work" can contain unrelated messages with the same UID.
                    if folder.eq_ignore_ascii_case("INBOX") {
                        "INBOX".into()
                    } else {
                        folder.clone()
                    }
                })
            },
            (!global_ids).then_some(reference.source_uidvalidity),
        )
    };
    let incoming: HashSet<_> = refs.iter().map(&identity).collect();
    let already_sending: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sync_queue WHERE id = ? AND status = 'sending')",
    )
    .bind(&action.id)
    .fetch_one(&mut **tx)
    .await
    .map_err(|e| e.to_string())?;
    if already_sending {
        return Ok(());
    }
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT id, payload, status FROM sync_queue WHERE account_id = ?
         AND action_type = 'remote_mutation' AND id <> ?
         AND status IN ('pending', 'scheduled', 'failed', 'sending')
         AND CASE WHEN json_valid(payload) THEN json_extract(payload, '$.kind') END = 'flags'",
    )
    .bind(&action.account_id)
    .bind(&action.id)
    .fetch_all(&mut **tx)
    .await
    .map_err(|e| e.to_string())?;
    for (id, payload, status) in rows {
        let Ok(mut previous) = serde_json::from_str::<RemoteMutationPayload>(&payload) else {
            continue;
        };
        let RemoteMutationPayload::Flags {
            refs,
            read: old_read,
            starred: old_starred,
            superseded,
        } = &mut previous
        else {
            continue;
        };
        let mut masks: HashMap<_, _> = superseded
            .iter()
            .cloned()
            .map(|mask| (mask.reference_index, mask))
            .collect();
        let mut changed = false;
        for (index, reference) in refs.iter().enumerate() {
            if !incoming.contains(&identity(reference)) {
                continue;
            }
            let skip_read = read.is_some() && old_read.is_some();
            let skip_star = starred.is_some() && old_starred.is_some();
            if !skip_read && !skip_star {
                continue;
            }
            let mask = masks.entry(index).or_insert(FlagSupersession {
                reference_index: index,
                read: false,
                starred: false,
            });
            mask.read |= skip_read;
            mask.starred |= skip_star;
            changed = true;
        }
        if !changed {
            continue;
        }
        *superseded = masks.into_values().collect();
        superseded.sort_by_key(|mask| mask.reference_index);
        if status != "sending"
            && batches(refs.clone(), *old_read, *old_starred, superseded).is_empty()
        {
            sqlx::query("DELETE FROM sync_queue WHERE id = ?")
                .bind(&id)
                .execute(&mut **tx)
                .await
                .map_err(|e| e.to_string())?;
        } else {
            sqlx::query("UPDATE sync_queue SET payload = ? WHERE id = ?")
                .bind(serde_json::to_string(&previous).map_err(|e| e.to_string())?)
                .bind(&id)
                .execute(&mut **tx)
                .await
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::queue::SyncQueue;

    fn reference(id: &str, folder: &str, generation: u32) -> MessageRef {
        MessageRef {
            remote_id: id.into(),
            source_folder: Some(folder.into()),
            source_uidvalidity: Some(generation),
        }
    }

    fn action(
        id: &str,
        refs: Vec<MessageRef>,
        read: Option<bool>,
        starred: Option<bool>,
    ) -> QueuedAction {
        let mut action = QueuedAction::new(
            "a".into(),
            "remote_mutation",
            serde_json::to_value(RemoteMutationPayload::Flags {
                refs,
                read,
                starred,
                superseded: Vec::new(),
            })
            .unwrap(),
        );
        action.id = id.into();
        action
    }

    async fn pool() -> sqlx::SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::db::init(&pool).await.unwrap();
        for id in ["a", "b"] {
            sqlx::query("INSERT INTO accounts(id, display_name, email, provider, config, credentials_ref, created_at, updated_at) VALUES (?, '', 'a@example.test', 'imap', '{}', '', '', '')")
                .bind(id).execute(&pool).await.unwrap();
        }
        pool
    }

    async fn planned(pool: &sqlx::SqlitePool, id: &str) -> Vec<(Vec<MessageRef>, FlagUpdate)> {
        let raw: String = sqlx::query_scalar("SELECT payload FROM sync_queue WHERE id = ?")
            .bind(id)
            .fetch_one(pool)
            .await
            .unwrap();
        match serde_json::from_str(&raw).unwrap() {
            RemoteMutationPayload::Flags {
                refs,
                read,
                starred,
                superseded,
            } => batches(refs, read, starred, &superseded),
            _ => panic!("expected flags"),
        }
    }

    #[tokio::test]
    async fn newer_read_intent_preserves_older_stars_and_other_messages() {
        let pool = pool().await;
        let queue = SyncQueue::new();
        let mut older = action(
            "old",
            vec![reference("1", "INBOX", 7), reference("2", "INBOX", 7)],
            Some(true),
            Some(true),
        );
        older.status = "failed".into();
        queue.enqueue(&pool, older).await.unwrap();
        queue
            .enqueue(
                &pool,
                action("new", vec![reference("1", "inbox", 7)], Some(false), None),
            )
            .await
            .unwrap();
        let old = planned(&pool, "old").await;
        assert_eq!(old.iter().map(|(refs, _)| refs.len()).sum::<usize>(), 2);
        for (refs, flags) in &old {
            assert_eq!(flags.starred, Some(true));
            assert_eq!(
                flags.read,
                if refs[0].remote_id == "1" {
                    None
                } else {
                    Some(true)
                }
            );
        }
        let newest = planned(&pool, "new").await;
        assert_eq!(newest[0].1.read, Some(false));
        assert_eq!(newest[0].1.starred, None);
        // Once the newer request has succeeded and left the queue, retrying
        // the failed request must still not restore the superseded read value.
        sqlx::query("DELETE FROM sync_queue WHERE id = 'new'")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(planned(&pool, "old").await.len(), 2);
    }

    #[tokio::test]
    async fn supersession_is_atomic_and_scoped_to_the_remote_identity() {
        let pool = pool().await;
        let queue = SyncQueue::new();
        for (id, folder, generation, account, status) in [
            ("same", "INBOX", 7, "a", "failed"),
            ("folder", "Other", 7, "a", "failed"),
            ("generation", "INBOX", 8, "a", "failed"),
            ("account", "INBOX", 7, "b", "failed"),
            ("sending", "INBOX", 7, "a", "sending"),
        ] {
            let mut old = action(
                id,
                vec![reference("1", folder, generation)],
                Some(true),
                None,
            );
            old.account_id = account.into();
            old.status = status.into();
            queue.enqueue(&pool, old).await.unwrap();
        }
        let new = action("new", vec![reference("1", "INBOX", 7)], Some(false), None);
        let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
        queue.enqueue_in_transaction(&mut tx, &new).await.unwrap();
        tx.rollback().await.unwrap();
        assert_eq!(planned(&pool, "same").await[0].1.read, Some(true));
        queue.enqueue(&pool, new).await.unwrap();
        let left: Vec<String> = sqlx::query_scalar("SELECT id FROM sync_queue ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(left, ["account", "folder", "generation", "new", "sending"]);
        assert!(
            planned(&pool, "sending").await.is_empty(),
            "an in-flight failure must retain supersession for its eventual retry"
        );
        queue
            .mark_failed_permanent(&pool, "sending", "offline")
            .await
            .unwrap();
        let retry: String =
            sqlx::query_scalar("SELECT payload FROM sync_queue WHERE id = 'sending'")
                .fetch_one(&pool)
                .await
                .unwrap();
        super::super::remote_mutations::execute_remote_mutation(
            &pool,
            "a",
            serde_json::from_str(&retry).unwrap(),
        )
        .await
        .expect("a superseded retry needs no provider connection");
    }

    #[tokio::test]
    async fn imap_folder_names_other_than_inbox_keep_their_case() {
        let pool = pool().await;
        let queue = SyncQueue::new();
        let mut older = action("old", vec![reference("1", "Work", 7)], Some(true), None);
        older.status = "failed".into();
        queue.enqueue(&pool, older).await.unwrap();
        queue
            .enqueue(
                &pool,
                action("new", vec![reference("1", "work", 7)], Some(false), None),
            )
            .await
            .unwrap();
        assert_eq!(planned(&pool, "old").await[0].1.read, Some(true));
        assert_eq!(planned(&pool, "new").await[0].1.read, Some(false));
    }

    #[tokio::test]
    async fn gmail_flags_follow_the_message_across_label_views() {
        let pool = pool().await;
        sqlx::query("UPDATE accounts SET provider = 'gmail' WHERE id = 'a'")
            .execute(&pool)
            .await
            .unwrap();
        let queue = SyncQueue::new();
        let mut older = action(
            "old",
            vec![reference("global-id", "INBOX", 7)],
            None,
            Some(true),
        );
        older.status = "failed".into();
        queue.enqueue(&pool, older).await.unwrap();
        queue
            .enqueue(
                &pool,
                action(
                    "new",
                    vec![reference("global-id", "STARRED", 0)],
                    None,
                    Some(false),
                ),
            )
            .await
            .unwrap();
        let left: Vec<String> = sqlx::query_scalar("SELECT id FROM sync_queue")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(left, ["new"]);
    }
}
