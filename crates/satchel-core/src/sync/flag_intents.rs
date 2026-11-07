//! Last-write-wins for queued flags, including manually retried failed actions.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use sqlx::{Sqlite, Transaction};

use super::{
    queue::QueuedAction,
    remote_mutations::{RemoteMutationPayload, QUEUED_KIND},
};
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

/// Where a flag reference points. Gmail and Outlook ids are global; IMAP UIDs
/// are only unique per folder and generation.
type Identity = (String, Option<String>, Option<Option<u32>>);

fn identity(global_ids: bool, reference: &MessageRef) -> Identity {
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
}

fn has_global_ids(provider: Option<&str>) -> bool {
    matches!(provider, Some("gmail" | "outlook"))
}

/// The flags choice a new queue row makes, if it can supersede older rows.
struct Intent {
    refs: Vec<MessageRef>,
    read: Option<bool>,
    starred: Option<bool>,
}

impl Intent {
    fn of(action: &QueuedAction) -> Option<Self> {
        if action.action_type != super::remote_mutations::ACTION_REMOTE_MUTATION
            || !matches!(action.status.as_str(), "pending" | "scheduled")
        {
            return None;
        }
        match serde_json::from_value(action.payload.clone()) {
            Ok(RemoteMutationPayload::Flags {
                refs,
                read,
                starred,
                ..
            }) => Some(Self {
                refs,
                read,
                starred,
            }),
            _ => None,
        }
    }

    fn identities(&self, global_ids: bool) -> HashSet<Identity> {
        self.refs
            .iter()
            .map(|reference| identity(global_ids, reference))
            .collect()
    }

    /// Mask what this choice replaces in an older flags payload. Returns
    /// whether the older payload changed.
    fn supersede(
        &self,
        incoming: &HashSet<Identity>,
        global_ids: bool,
        previous: &mut RemoteMutationPayload,
    ) -> bool {
        let RemoteMutationPayload::Flags {
            refs,
            read: old_read,
            starred: old_starred,
            superseded,
        } = previous
        else {
            return false;
        };
        let mut masks: HashMap<_, _> = superseded
            .iter()
            .cloned()
            .map(|mask| (mask.reference_index, mask))
            .collect();
        let mut changed = false;
        for (index, reference) in refs.iter().enumerate() {
            if !incoming.contains(&identity(global_ids, reference)) {
                continue;
            }
            let skip_read = self.read.is_some() && old_read.is_some();
            let skip_star = self.starred.is_some() && old_starred.is_some();
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
        if changed {
            *superseded = masks.into_values().collect();
            superseded.sort_by_key(|mask| mask.reference_index);
        }
        changed
    }
}

/// Nothing is left for the provider to do.
fn fully_superseded(payload: &RemoteMutationPayload) -> bool {
    match payload {
        RemoteMutationPayload::Flags {
            refs,
            read,
            starred,
            superseded,
        } => batches(refs.clone(), *read, *starred, superseded).is_empty(),
        _ => false,
    }
}

/// Live flags rows of an account that may name one of `remote_ids`, except
/// row `except`. The remote id narrows the scan inside SQLite; folder and
/// generation are compared afterwards, by [`identity`].
async fn candidate_rows(
    tx: &mut Transaction<'_, Sqlite>,
    account_id: &str,
    except: &str,
    remote_ids: &[&str],
) -> Result<Vec<(String, String, String)>, String> {
    if remote_ids.is_empty() {
        return Ok(Vec::new());
    }
    sqlx::query_as(&format!(
        "SELECT id, payload, status FROM sync_queue WHERE account_id = ?
         AND action_type = 'remote_mutation' AND id <> ?
         AND status IN ('pending', 'scheduled', 'failed', 'sending')
         AND {QUEUED_KIND} = 'flags'
         AND EXISTS (
             SELECT 1 FROM json_each(CASE WHEN json_valid(payload) THEN payload END, '$.refs') r
             WHERE CASE WHEN r.type = 'object' THEN json_extract(r.value, '$.remote_id') END
                   IN (SELECT value FROM json_each(?))
         )"
    ))
    .bind(account_id)
    .bind(except)
    .bind(serde_json::to_string(remote_ids).map_err(|e| e.to_string())?)
    .fetch_all(&mut **tx)
    .await
    .map_err(|e| e.to_string())
}

async fn write_back(
    tx: &mut Transaction<'_, Sqlite>,
    id: &str,
    status: &str,
    payload: &RemoteMutationPayload,
) -> Result<(), String> {
    if status != "sending" && fully_superseded(payload) {
        sqlx::query("DELETE FROM sync_queue WHERE id = ?")
            .bind(id)
            .execute(&mut **tx)
            .await
            .map_err(|e| e.to_string())?;
    } else {
        sqlx::query("UPDATE sync_queue SET payload = ? WHERE id = ?")
            .bind(serde_json::to_string(payload).map_err(|e| e.to_string())?)
            .bind(id)
            .execute(&mut **tx)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// The write reservation covers supersession and the new queue row together.
/// The running request keeps its already cloned payload. Its durable retry
/// payload records supersession too, in case that request fails after a newer
/// choice was queued. The account worker runs the newer choice afterwards.
pub async fn supersede(
    tx: &mut Transaction<'_, Sqlite>,
    action: &QueuedAction,
) -> Result<(), String> {
    let Some(intent) = Intent::of(action) else {
        return Ok(());
    };
    let provider: Option<String> = sqlx::query_scalar("SELECT provider FROM accounts WHERE id = ?")
        .bind(&action.account_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| e.to_string())?;
    let global_ids = has_global_ids(provider.as_deref());
    let incoming = intent.identities(global_ids);
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
    let remote_ids: Vec<&str> = intent.refs.iter().map(|r| r.remote_id.as_str()).collect();
    let rows = candidate_rows(tx, &action.account_id, &action.id, &remote_ids).await?;
    for (id, payload, status) in rows {
        let Ok(mut previous) = serde_json::from_str::<RemoteMutationPayload>(&payload) else {
            continue;
        };
        if intent.supersede(&incoming, global_ids, &mut previous) {
            write_back(tx, &id, &status, &previous).await?;
        }
    }
    Ok(())
}

/// What a batch enqueue writes for one of its actions.
pub(crate) enum Planned {
    /// Insert the row with this payload.
    Insert(String),
    /// A later action of the same batch superseded all of it.
    Superseded,
}

/// A flags row as it stands while the batch is planned.
struct PlannedRow {
    id: String,
    status: String,
    payload: RemoteMutationPayload,
    changed: bool,
    deleted: bool,
}

fn add_row(
    rows: &mut Vec<PlannedRow>,
    index: &mut HashMap<(String, Identity), Vec<usize>>,
    account: &str,
    global_ids: bool,
    row: PlannedRow,
) {
    if let RemoteMutationPayload::Flags { refs, .. } = &row.payload {
        for reference in refs {
            let entry = index
                .entry((account.to_string(), identity(global_ids, reference)))
                .or_default();
            if entry.last() != Some(&rows.len()) {
                entry.push(rows.len());
            }
        }
    }
    rows.push(row);
}

/// [`supersede`] for a whole batch, as if each action were enqueued in turn:
/// an action supersedes older queue rows and earlier actions of the batch.
/// Older rows are updated here; the caller inserts the batch's rows as
/// planned.
///
/// The caller guarantees that every action id is new: no queue row has it and
/// no two actions share it. No row can therefore be in flight under one.
pub(crate) async fn plan_batch(
    tx: &mut Transaction<'_, Sqlite>,
    actions: &[QueuedAction],
) -> Result<Vec<Planned>, String> {
    let intents: Vec<Option<Intent>> = actions.iter().map(Intent::of).collect();
    let mut accounts: Vec<&str> = actions.iter().map(|a| a.account_id.as_str()).collect();
    accounts.sort_unstable();
    accounts.dedup();
    let providers: HashMap<String, String> = sqlx::query_as(
        "SELECT id, provider FROM accounts WHERE id IN (SELECT value FROM json_each(?))",
    )
    .bind(serde_json::to_string(&accounts).map_err(|e| e.to_string())?)
    .fetch_all(&mut **tx)
    .await
    .map_err(|e| e.to_string())?
    .into_iter()
    .collect();
    let global = |account: &str| has_global_ids(providers.get(account).map(String::as_str));

    let mut rows: Vec<PlannedRow> = Vec::new();
    let mut index: HashMap<(String, Identity), Vec<usize>> = HashMap::new();
    // Older rows the batch's choices can reach.
    for account in &accounts {
        let remote_ids: Vec<&str> = actions
            .iter()
            .zip(&intents)
            .filter(|(action, _)| action.account_id == *account)
            .filter_map(|(_, intent)| intent.as_ref())
            .flat_map(|intent| intent.refs.iter().map(|r| r.remote_id.as_str()))
            .collect();
        for (id, payload, status) in candidate_rows(tx, account, "", &remote_ids).await? {
            let Ok(payload) = serde_json::from_str::<RemoteMutationPayload>(&payload) else {
                continue;
            };
            add_row(
                &mut rows,
                &mut index,
                account,
                global(account),
                PlannedRow {
                    id,
                    status,
                    payload,
                    changed: false,
                    deleted: false,
                },
            );
        }
    }
    let older = rows.len();

    let mut own_rows = vec![None; actions.len()];
    for (position, (action, intent)) in actions.iter().zip(&intents).enumerate() {
        if let Some(intent) = intent {
            let global_ids = global(&action.account_id);
            let incoming = intent.identities(global_ids);
            let mut reached: Vec<usize> = incoming
                .iter()
                .filter_map(|ident| index.get(&(action.account_id.clone(), ident.clone())))
                .flatten()
                .copied()
                .collect();
            reached.sort_unstable();
            reached.dedup();
            for row in reached {
                let row = &mut rows[row];
                if row.deleted || row.id == action.id {
                    continue;
                }
                if intent.supersede(&incoming, global_ids, &mut row.payload) {
                    if row.status != "sending" && fully_superseded(&row.payload) {
                        row.deleted = true;
                    } else {
                        row.changed = true;
                    }
                }
            }
        }
        // Later actions find this one the way `supersede` finds queue rows.
        if action.action_type == super::remote_mutations::ACTION_REMOTE_MUTATION
            && matches!(
                action.status.as_str(),
                "pending" | "scheduled" | "failed" | "sending"
            )
        {
            if let Ok(payload @ RemoteMutationPayload::Flags { .. }) =
                serde_json::from_value::<RemoteMutationPayload>(action.payload.clone())
            {
                own_rows[position] = Some(rows.len());
                add_row(
                    &mut rows,
                    &mut index,
                    &action.account_id,
                    global(&action.account_id),
                    PlannedRow {
                        id: action.id.clone(),
                        status: action.status.clone(),
                        payload,
                        changed: false,
                        deleted: false,
                    },
                );
            }
        }
    }

    for row in &rows[..older] {
        if row.deleted {
            sqlx::query("DELETE FROM sync_queue WHERE id = ?")
                .bind(&row.id)
                .execute(&mut **tx)
                .await
                .map_err(|e| e.to_string())?;
        } else if row.changed {
            write_back(tx, &row.id, &row.status, &row.payload).await?;
        }
    }
    actions
        .iter()
        .zip(own_rows)
        .map(|(action, own)| {
            let row = own.map(|own| &rows[own]);
            Ok(match row {
                Some(row) if row.deleted => Planned::Superseded,
                Some(row) if row.changed => {
                    Planned::Insert(serde_json::to_string(&row.payload).map_err(|e| e.to_string())?)
                }
                _ => Planned::Insert(
                    serde_json::to_string(&action.payload).unwrap_or_else(|_| "null".into()),
                ),
            })
        })
        .collect()
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

    /// Queue rows in order, without generated timestamps.
    async fn queue_rows(
        pool: &sqlx::SqlitePool,
    ) -> Vec<(String, String, serde_json::Value, String)> {
        let rows: Vec<(String, String, String, String)> =
            sqlx::query_as("SELECT id, account_id, payload, status FROM sync_queue ORDER BY rowid")
                .fetch_all(pool)
                .await
                .unwrap();
        rows.into_iter()
            .map(|(id, account, payload, status)| {
                (id, account, serde_json::from_str(&payload).unwrap(), status)
            })
            .collect()
    }

    /// The parts of one generated queue row: account, kind (flags or a
    /// provider move), refs drawn from a few identities, a read and a star
    /// choice, a status.
    type Parts = (bool, bool, Vec<u8>, Option<bool>, Option<bool>, u8);

    fn generated(id: String, (account, flags, refs, read, starred, status): Parts) -> QueuedAction {
        let refs: Vec<MessageRef> = refs
            .into_iter()
            .map(|n| {
                // Remote ids repeat across folders and generations, and the
                // same IMAP folder appears with another case.
                let folder = ["INBOX", "inbox", "Work", "work"][usize::from(n % 4)];
                reference(&format!("{}", n / 4 % 3), folder, u32::from(n / 12 % 2))
            })
            .collect();
        let account = if account { "a" } else { "b" };
        let mut action = if flags {
            action(&id, refs, read, starred)
        } else {
            // A move names the same refs; flags must neither supersede nor
            // reorder around it.
            let mut action = super::super::remote_mutations::PreparedWorkflowMove {
                account_id: account.into(),
                source_folder_type: "folder".into(),
                target_folder_type: "archive".into(),
                refs,
                target_remote_folder_id: "Archive".into(),
            }
            .into_action(&format!("thread-{id}"), chrono::Utc::now())
            .unwrap();
            action.id = id;
            action
        };
        action.account_id = account.into();
        action.status =
            ["pending", "scheduled", "failed", "sending"][usize::from(status % 4)].into();
        action
    }

    fn generated_parts() -> impl proptest::strategy::Strategy<Value = Parts> {
        use proptest::prelude::*;
        (
            any::<bool>(),
            // Two flags rows for every move.
            prop_oneof![2 => Just(true), 1 => Just(false)],
            proptest::collection::vec(0u8..24, 0..4),
            proptest::option::of(any::<bool>()),
            proptest::option::of(any::<bool>()),
            0u8..4,
        )
    }

    /// The queue one path leaves on a reused database.
    async fn enqueue_case(
        pool: &sqlx::SqlitePool,
        gmail: bool,
        older: &[QueuedAction],
        batch: &[QueuedAction],
        batched: bool,
    ) -> Vec<(String, String, serde_json::Value, String)> {
        sqlx::query("DELETE FROM sync_queue")
            .execute(pool)
            .await
            .unwrap();
        sqlx::query(
            "UPDATE accounts SET provider = CASE WHEN id = 'a' AND ? THEN 'gmail' ELSE 'imap' END",
        )
        .bind(gmail)
        .execute(pool)
        .await
        .unwrap();
        let queue = SyncQueue::new();
        for action in older {
            queue.enqueue(pool, action.clone()).await.unwrap();
        }
        let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
        if batched {
            queue
                .enqueue_batch_in_transaction(&mut tx, batch)
                .await
                .unwrap();
        } else {
            for action in batch {
                queue.enqueue_in_transaction(&mut tx, action).await.unwrap();
            }
        }
        tx.commit().await.unwrap();
        queue_rows(pool).await
    }

    /// A batch leaves exactly the queue that enqueueing its actions one after
    /// the other leaves: older rows superseded, rows of the same batch
    /// superseded, Gmail ids global, in-flight rows kept, moves in place.
    /// Two databases, migrated once, are cleared for every case.
    #[test]
    fn a_batch_enqueue_matches_enqueueing_one_at_a_time() {
        use proptest::test_runner::{Config, TestRunner};
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (one_by_one_pool, batched_pool) =
            runtime.block_on(async { (pool().await, pool().await) });
        let strategy = (
            proptest::collection::vec(generated_parts(), 0..5),
            proptest::collection::vec(generated_parts(), 2..8),
            proptest::prelude::any::<bool>(),
        );
        TestRunner::new(Config::with_cases(128))
            .run(&strategy, |(older, batch, gmail)| {
                let older: Vec<_> = older
                    .into_iter()
                    .enumerate()
                    .map(|(n, parts)| generated(format!("older-{n}"), parts))
                    .collect();
                let batch: Vec<_> = batch
                    .into_iter()
                    .enumerate()
                    .map(|(n, parts)| generated(format!("new-{n}"), parts))
                    .collect();
                let (one_by_one, batched) = runtime.block_on(async {
                    (
                        enqueue_case(&one_by_one_pool, gmail, &older, &batch, false).await,
                        enqueue_case(&batched_pool, gmail, &older, &batch, true).await,
                    )
                });
                proptest::prop_assert_eq!(one_by_one, batched);
                Ok(())
            })
            .unwrap();
    }

    #[tokio::test]
    async fn a_batch_that_names_a_queued_row_merges_like_the_single_path() {
        let pool = pool().await;
        let queue = SyncQueue::new();
        let mut older = action("same", vec![reference("1", "INBOX", 7)], Some(true), None);
        older.status = "failed".into();
        queue.enqueue(&pool, older).await.unwrap();
        let batch = [
            action("same", vec![reference("2", "INBOX", 7)], None, Some(true)),
            action("other", vec![reference("1", "INBOX", 7)], Some(false), None),
        ];
        let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
        queue
            .enqueue_batch_in_transaction(&mut tx, &batch)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        let rows = queue_rows(&pool).await;
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].0, "same");
        assert_eq!(
            rows[0].2["refs"][0]["remote_id"], "2",
            "the re-enqueue replaced the row"
        );
        assert_eq!(rows[0].3, "pending");
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
