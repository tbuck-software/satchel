//! Runs recurring sender sweep rules through the same durable actions the
//! commands use: Done through the workflow transition, trash and folder moves
//! through `move_threads`. Each queues its provider mutation in the same
//! transaction that changes the local rows, so a sweep is remote-first like
//! every other move, and nothing here has its own idea of how to file mail.
//!
//! A pass files at most [`SWEEP_PASS_LIMIT`] threads; whatever is left waits
//! for the next one. Every run goes through a [`SweepGate`], so the run a new
//! rule gets when it is saved and the scheduler's pass never select the same
//! threads at once.

use sqlx::SqlitePool;
use tokio::sync::Mutex;

use crate::db::sender_sweep::{
    SenderSweepError, SenderSweepRepository, SweepAction, SweepRuleRecord, SweepSelection,
    SweepTarget,
};
use crate::mail_actions::{move_threads, move_threads_reporting, MoveOutcome, MoveTarget};
use crate::sync::queue::SyncQueue;
use crate::workflow::states::WorkflowState;
use crate::workflow::transitions::{
    transition_threads_inner, BatchTransitionEntry, BatchTransitionInput,
};

/// Threads per transaction batch.
pub const SWEEP_BATCH: usize = 200;
/// Threads one pass files across all rules. Each one is its own short write
/// transaction; a pass of thousands would hold the scheduler for minutes.
pub const SWEEP_PASS_LIMIT: usize = 500;

/// What one rule run did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SweepRunOutcome {
    pub rule_id: String,
    /// Threads the run filed.
    pub swept: Vec<String>,
    /// Threads it selected but could not file this time (a pending move, a
    /// concurrent change). The next run selects them again.
    pub failed: usize,
    /// More qualified than the pass had room for.
    pub deferred: usize,
}

/// One sweep at a time per process. The app manages a single gate.
#[derive(Debug, Default)]
pub struct SweepGate {
    running: Mutex<()>,
}

impl SweepGate {
    /// The scheduler's pass: skipped (`None`) while another sweep runs, since
    /// the next tick comes soon enough.
    pub async fn run_pass(
        &self,
        pool: &SqlitePool,
        queue: &SyncQueue,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Option<Result<Vec<SweepRunOutcome>, String>> {
        let _running = self.running.try_lock().ok()?;
        Some(run_sweep_rules(pool, queue, now, SWEEP_PASS_LIMIT).await)
    }

    /// A rule's first run when it is saved; waits for a pass in progress and
    /// reads the rule again, since it may have changed or gone meanwhile.
    pub async fn run_rule_now(
        &self,
        pool: &SqlitePool,
        queue: &SyncQueue,
        rule_id: &str,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<SweepRunOutcome, String> {
        let _running = self.running.lock().await;
        let rule = SenderSweepRepository::new(pool)
            .get_rule(rule_id)
            .await
            .map_err(|error| error.to_string())?;
        run_sweep_rule(pool, queue, &rule, now, SWEEP_PASS_LIMIT).await
    }
}

/// Run the rules once at `now`, longest-waiting first, filing at most
/// `budget` threads. A rule that fails is logged, stamped as run and left for
/// the next pass; it does not stop the others.
pub async fn run_sweep_rules(
    pool: &SqlitePool,
    queue: &SyncQueue,
    now: chrono::DateTime<chrono::Utc>,
    budget: usize,
) -> Result<Vec<SweepRunOutcome>, String> {
    let repository = SenderSweepRepository::new(pool);
    let rules = repository
        .list_rules()
        .await
        .map_err(|error| error.to_string())?;
    let mut outcomes = Vec::with_capacity(rules.len());
    let mut left = budget;
    for listed in rules {
        if left == 0 {
            break;
        }
        // Read again right before running it: deleting or editing a rule does
        // not wait for the gate, and a rule's moves have no Undo. A rule that
        // went from Trash to Archive must not trash anything on the way out.
        let rule = match repository.get_rule(&listed.id).await {
            Ok(rule) => rule,
            Err(SenderSweepError::NotFound) => continue,
            Err(error) => {
                tracing::warn!(target: "satchel::sweep", rule = %listed.id, %error, "sweep rule could not be read");
                continue;
            }
        };
        match run_sweep_rule(pool, queue, &rule, now, left).await {
            Ok(outcome) => {
                left = left.saturating_sub(outcome.swept.len() + outcome.failed);
                outcomes.push(outcome);
            }
            Err(error) => {
                tracing::warn!(target: "satchel::sweep", rule = %rule.id, %error, "sweep rule failed");
                let _ = repository.record_attempt(&rule.id, &now.to_rfc3339()).await;
            }
        }
    }
    Ok(outcomes)
}

/// Run one rule at `now`, filing at most `budget` threads, and record when it
/// ran and how much it filed. A move rule whose folder is gone does nothing,
/// and neither does a rule of an account that is switched off. The rule is
/// checked again before every batch, and the run stops once it was deleted
/// or changed meanwhile.
pub async fn run_sweep_rule(
    pool: &SqlitePool,
    queue: &SyncQueue,
    rule: &SweepRuleRecord,
    now: chrono::DateTime<chrono::Utc>,
    budget: usize,
) -> Result<SweepRunOutcome, String> {
    let action = SweepAction::from_db_value(&rule.action)
        .ok_or_else(|| format!("unknown sweep action {}", rule.action))?;
    let mut outcome = SweepRunOutcome {
        rule_id: rule.id.clone(),
        ..SweepRunOutcome::default()
    };
    let folder = rule.target_folder_id.as_deref();
    if action == SweepAction::Move && folder.is_none() {
        return Ok(outcome);
    }
    // The schema refuses it too; a row from anywhere else still must not
    // trash the sender's mail the moment it arrives.
    if action == SweepAction::Trash && rule.older_than_days.unwrap_or(0) < 1 {
        return Err("trash rule without an age".into());
    }
    let repository = SenderSweepRepository::new(pool);
    if !repository
        .account_active(&rule.account_id)
        .await
        .map_err(|error| error.to_string())?
    {
        return Ok(outcome);
    }
    let selection = SweepSelection::for_rule(rule.older_than_days, rule.keep_latest, now);
    let targets = repository
        .targets(&rule.account_id, &rule.sender_email, &selection)
        .await
        .map_err(|error| error.to_string())?;
    let take = targets.len().min(budget);
    outcome.deferred = targets.len() - take;

    for chunk in targets[..take].chunks(SWEEP_BATCH) {
        if rule_changed(&repository, rule).await {
            outcome.deferred += chunk.len();
            continue;
        }
        let (swept, failed) = match (action, folder) {
            (SweepAction::Archive, _) => archive(pool, queue, chunk).await,
            (SweepAction::Trash, _) => move_chunk(pool, queue, chunk, MoveTarget::Trash).await,
            (SweepAction::Move, Some(folder)) => {
                move_chunk(pool, queue, chunk, MoveTarget::Folder(folder)).await
            }
            (SweepAction::Move, None) => unreachable!("checked above"),
        };
        outcome.swept.extend(swept);
        outcome.failed += failed;
    }
    repository
        .record_run(&rule.id, &now.to_rfc3339(), outcome.swept.len() as i64)
        .await
        .map_err(|error| error.to_string())?;
    Ok(outcome)
}

/// Whether the stored rule is gone or no longer the one this run started with.
async fn rule_changed(repository: &SenderSweepRepository<'_>, rule: &SweepRuleRecord) -> bool {
    match repository.get_rule(&rule.id).await {
        Ok(fresh) => fresh.updated_at != rule.updated_at,
        Err(_) => true,
    }
}

async fn archive(
    pool: &SqlitePool,
    queue: &SyncQueue,
    chunk: &[SweepTarget],
) -> (Vec<String>, usize) {
    let result = transition_threads_inner(
        pool,
        queue,
        BatchTransitionInput {
            threads: chunk
                .iter()
                .map(|target| BatchTransitionEntry {
                    thread_id: target.thread_id.clone(),
                    from: target.workflow_state,
                })
                .collect(),
            to: WorkflowState::Done,
            snooze_until: None,
        },
    )
    .await;
    (result.applied, result.failed.len())
}

/// One move for the chunk; a chunk the move refuses (one thread with a move
/// still pending fails all of it) is retried thread by thread, so one busy
/// thread does not hold back the other 199. So are the threads a move that
/// stopped part way left behind.
async fn move_chunk(
    pool: &SqlitePool,
    queue: &SyncQueue,
    chunk: &[SweepTarget],
    target: MoveTarget<'_>,
) -> (Vec<String>, usize) {
    let ids: Vec<String> = chunk.iter().map(|t| t.thread_id.clone()).collect();
    let (mut swept, rest) = match move_threads_reporting(pool, queue, &ids, target).await {
        Ok(MoveOutcome {
            stopped_early: None,
            ..
        }) => return (ids, 0),
        Ok(MoveOutcome {
            stopped_early: Some(stopped),
            ..
        }) => (stopped.applied, stopped.remaining),
        Err(_) => (Vec::new(), ids),
    };
    let mut failed = 0;
    for id in rest {
        match move_threads(pool, queue, std::slice::from_ref(&id), target).await {
            Ok(_) => swept.push(id),
            Err(error) => {
                tracing::info!(target: "satchel::sweep", thread = %id, %error, "sweep left a thread for the next run");
                failed += 1;
            }
        }
    }
    (swept, failed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::sender_sweep::tests::{insert_account, insert_thread, pool};
    use crate::db::sender_sweep::SweepRuleUpsert;

    async fn state_of(pool: &SqlitePool, id: &str) -> String {
        sqlx::query_scalar("SELECT workflow_state FROM threads WHERE id = ?")
            .bind(id)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    async fn effective_folder(pool: &SqlitePool, id: &str) -> String {
        sqlx::query_scalar(
            "SELECT COALESCE(pending_folder_id, folder_id) FROM messages WHERE thread_id = ?",
        )
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn queued_moves(pool: &SqlitePool) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM sync_queue WHERE action_type = 'remote_mutation'")
            .fetch_one(pool)
            .await
            .unwrap()
    }

    async fn rule(
        pool: &SqlitePool,
        account: &str,
        action: SweepAction,
        days: Option<i64>,
        keep_latest: bool,
        folder: Option<&str>,
    ) -> SweepRuleRecord {
        SenderSweepRepository::new(pool)
            .upsert_rule(SweepRuleUpsert {
                account_id: account.into(),
                sender_email: "news@shop.example".into(),
                action,
                older_than_days: days,
                keep_latest,
                target_folder_id: folder.map(Into::into),
            })
            .await
            .unwrap()
    }

    /// A chunk a later batch stops: what moved counts as swept, the rest is
    /// retried one by one, and only the thread that fails again is left.
    #[tokio::test]
    async fn a_chunk_a_later_batch_stops_retries_only_the_rest() {
        let pool = pool().await;
        insert_account(&pool, "a").await;
        for id in ["s1", "s2", "s3", "s4"] {
            insert_thread(
                &pool,
                "a",
                id,
                "news@shop.example",
                30,
                "inbox",
                "inbox",
                true,
            )
            .await;
        }
        sqlx::query(
            "CREATE TRIGGER stop_bulk_move BEFORE UPDATE ON messages WHEN new.thread_id = 's3'
             BEGIN SELECT RAISE(ABORT, 'database is locked'); END",
        )
        .execute(&pool)
        .await
        .unwrap();
        let chunk: Vec<SweepTarget> = ["s1", "s2", "s3", "s4"]
            .into_iter()
            .map(|id| SweepTarget {
                thread_id: id.into(),
                workflow_state: WorkflowState::Inbox,
                last_message_date: String::new(),
                unread_count: 0,
            })
            .collect();

        // Batches of one, so the chunk crosses batches the way a chunk of 200
        // would in batches of 50.
        crate::mail_actions::set_move_batch_threads_for_tests(Some(1));
        let (swept, failed) = move_chunk(&pool, &SyncQueue::new(), &chunk, MoveTarget::Trash).await;
        crate::mail_actions::set_move_batch_threads_for_tests(None);

        assert_eq!(swept, ["s1", "s2", "s4"]);
        assert_eq!(failed, 1);
        assert_eq!(effective_folder(&pool, "s3").await, "a-inbox");
        assert_eq!(effective_folder(&pool, "s4").await, "a-trash");
    }

    #[tokio::test]
    async fn an_archive_rule_files_old_mail_and_queues_the_provider_move() {
        let pool = pool().await;
        insert_account(&pool, "a").await;
        insert_thread(
            &pool,
            "a",
            "fresh",
            "news@shop.example",
            2,
            "inbox",
            "inbox",
            false,
        )
        .await;
        insert_thread(
            &pool,
            "a",
            "old",
            "news@shop.example",
            12,
            "inbox",
            "inbox",
            false,
        )
        .await;
        insert_thread(
            &pool,
            "a",
            "friend",
            "friend@example.org",
            40,
            "inbox",
            "inbox",
            false,
        )
        .await;
        let rule = rule(&pool, "a", SweepAction::Archive, Some(10), false, None).await;

        let queue = SyncQueue::new();
        let outcome = run_sweep_rule(&pool, &queue, &rule, chrono::Utc::now(), SWEEP_PASS_LIMIT)
            .await
            .unwrap();
        assert_eq!(outcome.swept, ["old"]);
        assert_eq!(outcome.failed, 0);
        assert_eq!(state_of(&pool, "old").await, "done");
        assert_eq!(state_of(&pool, "fresh").await, "inbox");
        assert_eq!(state_of(&pool, "friend").await, "inbox");
        // Done includes Read, and both intents reach the provider queue.
        assert!(queued_moves(&pool).await > 0);

        let stored = SenderSweepRepository::new(&pool)
            .get_rule(&rule.id)
            .await
            .unwrap();
        assert_eq!(stored.last_swept_count, 1);
        assert!(stored.last_run_at.is_some());

        // The filed thread left the Inbox; a second pass finds nothing new.
        let again = run_sweep_rule(&pool, &queue, &stored, chrono::Utc::now(), SWEEP_PASS_LIMIT)
            .await
            .unwrap();
        assert!(again.swept.is_empty());
    }

    #[tokio::test]
    async fn the_cutoff_moves_with_the_clock() {
        let pool = pool().await;
        insert_account(&pool, "a").await;
        insert_thread(
            &pool,
            "a",
            "t",
            "news@shop.example",
            2,
            "inbox",
            "inbox",
            true,
        )
        .await;
        let rule = rule(&pool, "a", SweepAction::Archive, Some(3), false, None).await;
        let queue = SyncQueue::new();

        let today = run_sweep_rule(&pool, &queue, &rule, chrono::Utc::now(), SWEEP_PASS_LIMIT)
            .await
            .unwrap();
        assert!(today.swept.is_empty());
        let in_two_days = chrono::Utc::now() + chrono::Duration::days(2);
        let later = run_sweep_rule(&pool, &queue, &rule, in_two_days, SWEEP_PASS_LIMIT)
            .await
            .unwrap();
        assert_eq!(later.swept, ["t"]);
    }

    #[tokio::test]
    async fn a_trash_rule_keeps_the_latest_and_runs_per_account() {
        let pool = pool().await;
        insert_account(&pool, "a").await;
        insert_account(&pool, "b").await;
        for (id, days) in [("a1", 1), ("a2", 5), ("a3", 9)] {
            insert_thread(
                &pool,
                "a",
                id,
                "news@shop.example",
                days,
                "inbox",
                "inbox",
                true,
            )
            .await;
        }
        insert_thread(
            &pool,
            "b",
            "b1",
            "news@shop.example",
            30,
            "inbox",
            "inbox",
            true,
        )
        .await;
        insert_thread(
            &pool,
            "b",
            "b2",
            "news@shop.example",
            40,
            "inbox",
            "inbox",
            true,
        )
        .await;
        rule(&pool, "a", SweepAction::Trash, Some(1), true, None).await;

        let queue = SyncQueue::new();
        let outcomes = run_sweep_rules(&pool, &queue, chrono::Utc::now(), SWEEP_PASS_LIMIT)
            .await
            .unwrap();
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].swept, ["a2", "a3"]);
        assert_eq!(effective_folder(&pool, "a2").await, "a-trash");
        assert_eq!(effective_folder(&pool, "a3").await, "a-trash");
        assert_eq!(effective_folder(&pool, "a1").await, "a-inbox");
        // The other account's mail from the same sender has no rule.
        assert_eq!(effective_folder(&pool, "b1").await, "b-inbox");
        assert_eq!(effective_folder(&pool, "b2").await, "b-inbox");
    }

    #[tokio::test]
    async fn a_move_rule_files_into_the_folder_and_leaves_the_inbox() {
        let pool = pool().await;
        insert_account(&pool, "a").await;
        insert_thread(
            &pool,
            "a",
            "n1",
            "news@shop.example",
            0,
            "inbox",
            "inbox",
            false,
        )
        .await;
        insert_thread(
            &pool,
            "a",
            "n2",
            "news@shop.example",
            4,
            "inbox",
            "inbox",
            false,
        )
        .await;
        let rule = rule(&pool, "a", SweepAction::Move, None, false, Some("a-news")).await;

        let queue = SyncQueue::new();
        let outcome = run_sweep_rule(&pool, &queue, &rule, chrono::Utc::now(), SWEEP_PASS_LIMIT)
            .await
            .unwrap();
        assert_eq!(outcome.swept.len(), 2);
        assert_eq!(effective_folder(&pool, "n1").await, "a-news");
        assert_eq!(effective_folder(&pool, "n2").await, "a-news");
        assert!(queued_moves(&pool).await >= 1);
        assert!(SenderSweepRepository::new(&pool)
            .targets("a", "news@shop.example", &SweepSelection::default())
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn large_sweeps_run_in_batches() {
        let pool = pool().await;
        insert_account(&pool, "a").await;
        let total = SWEEP_BATCH + 25;
        for index in 0..total {
            insert_thread(
                &pool,
                "a",
                &format!("t{index:04}"),
                "news@shop.example",
                20,
                "inbox",
                "inbox",
                true,
            )
            .await;
        }
        let rule = rule(&pool, "a", SweepAction::Trash, Some(10), false, None).await;
        let queue = SyncQueue::new();
        let outcome = run_sweep_rule(&pool, &queue, &rule, chrono::Utc::now(), SWEEP_PASS_LIMIT)
            .await
            .unwrap();
        assert_eq!(outcome.swept.len(), total);
        let left: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM threads WHERE workflow_state = 'inbox'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(left, 0);
    }

    #[tokio::test]
    async fn a_pass_stops_at_its_budget_and_the_next_one_continues() {
        let pool = pool().await;
        insert_account(&pool, "a").await;
        for index in 0..30 {
            insert_thread(
                &pool,
                "a",
                &format!("t{index:02}"),
                "news@shop.example",
                20,
                "inbox",
                "inbox",
                true,
            )
            .await;
        }
        rule(&pool, "a", SweepAction::Archive, Some(10), false, None).await;
        let queue = SyncQueue::new();

        let first = run_sweep_rules(&pool, &queue, chrono::Utc::now(), 12)
            .await
            .unwrap();
        assert_eq!(first[0].swept.len(), 12);
        assert_eq!(first[0].deferred, 18);
        let second = run_sweep_rules(&pool, &queue, chrono::Utc::now(), 100)
            .await
            .unwrap();
        assert_eq!(second[0].swept.len(), 18);
        assert_eq!(second[0].deferred, 0);
    }

    #[tokio::test]
    async fn a_pass_waits_out_a_running_sweep() {
        let pool = pool().await;
        insert_account(&pool, "a").await;
        let gate = SweepGate::default();
        let queue = SyncQueue::new();
        let held = gate.running.lock().await;
        assert!(gate
            .run_pass(&pool, &queue, chrono::Utc::now())
            .await
            .is_none());
        drop(held);
        assert!(gate
            .run_pass(&pool, &queue, chrono::Utc::now())
            .await
            .is_some());
    }

    #[tokio::test]
    async fn the_first_run_reads_the_rule_again() {
        let pool = pool().await;
        insert_account(&pool, "a").await;
        insert_thread(
            &pool,
            "a",
            "t",
            "news@shop.example",
            20,
            "inbox",
            "inbox",
            true,
        )
        .await;
        let saved = rule(&pool, "a", SweepAction::Archive, Some(10), false, None).await;
        let gate = SweepGate::default();
        let queue = SyncQueue::new();
        let outcome = gate
            .run_rule_now(&pool, &queue, &saved.id, chrono::Utc::now())
            .await
            .unwrap();
        assert_eq!(outcome.swept, ["t"]);
        SenderSweepRepository::new(&pool)
            .delete_rule(&saved.id)
            .await
            .unwrap();
        assert!(gate
            .run_rule_now(&pool, &queue, &saved.id, chrono::Utc::now())
            .await
            .is_err());
    }

    #[tokio::test]
    async fn a_move_rule_without_its_folder_waits() {
        let pool = pool().await;
        insert_account(&pool, "a").await;
        insert_thread(
            &pool,
            "a",
            "n",
            "news@shop.example",
            1,
            "inbox",
            "inbox",
            true,
        )
        .await;
        let saved = rule(&pool, "a", SweepAction::Move, None, false, Some("a-news")).await;
        sqlx::query("DELETE FROM folders WHERE id = 'a-news'")
            .execute(&pool)
            .await
            .unwrap();
        let orphan = SenderSweepRepository::new(&pool)
            .get_rule(&saved.id)
            .await
            .unwrap();
        let queue = SyncQueue::new();
        let outcome = run_sweep_rule(&pool, &queue, &orphan, chrono::Utc::now(), SWEEP_PASS_LIMIT)
            .await
            .unwrap();
        assert!(outcome.swept.is_empty());
        assert_eq!(effective_folder(&pool, "n").await, "a-inbox");
    }

    #[tokio::test]
    async fn a_rule_never_files_a_shared_conversation() {
        let pool = pool().await;
        insert_account(&pool, "a").await;
        insert_thread(
            &pool,
            "a",
            "shared",
            "news@shop.example",
            20,
            "inbox",
            "inbox",
            true,
        )
        .await;
        crate::db::sender_sweep::tests::add_message(
            &pool,
            "a",
            "shared",
            "shared-me",
            "a@example.com",
            19,
            false,
        )
        .await;
        insert_thread(
            &pool,
            "a",
            "solo",
            "news@shop.example",
            20,
            "inbox",
            "inbox",
            true,
        )
        .await;
        let saved = rule(&pool, "a", SweepAction::Trash, Some(10), false, None).await;
        let queue = SyncQueue::new();
        let outcome = run_sweep_rule(&pool, &queue, &saved, chrono::Utc::now(), SWEEP_PASS_LIMIT)
            .await
            .unwrap();
        assert_eq!(outcome.swept, ["solo"]);
        assert_eq!(effective_folder_of(&pool, "shared-me").await, "a-inbox");
    }

    async fn effective_folder_of(pool: &SqlitePool, message: &str) -> String {
        sqlx::query_scalar(
            "SELECT COALESCE(pending_folder_id, folder_id) FROM messages WHERE id = ?",
        )
        .bind(message)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn a_pass_runs_the_rule_as_it_is_now_not_as_it_was_listed() {
        let pool = pool().await;
        insert_account(&pool, "a").await;
        insert_thread(
            &pool,
            "a",
            "t",
            "news@shop.example",
            20,
            "inbox",
            "inbox",
            true,
        )
        .await;
        let trash = rule(&pool, "a", SweepAction::Trash, Some(10), false, None).await;
        // Changed to Archive after a pass read the list.
        let archive = rule(&pool, "a", SweepAction::Archive, Some(10), false, None).await;
        assert_eq!(trash.id, archive.id);
        let queue = SyncQueue::new();

        // The stale record files nothing: the rule changed under it.
        let stale = run_sweep_rule(&pool, &queue, &trash, chrono::Utc::now(), SWEEP_PASS_LIMIT)
            .await
            .unwrap();
        assert!(stale.swept.is_empty());
        assert_eq!(stale.deferred, 1);
        assert_eq!(effective_folder(&pool, "t").await, "a-inbox");

        // A pass reads it again and archives instead.
        let outcomes = run_sweep_rules(&pool, &queue, chrono::Utc::now(), SWEEP_PASS_LIMIT)
            .await
            .unwrap();
        assert_eq!(outcomes[0].swept, ["t"]);
        assert_eq!(state_of(&pool, "t").await, "done");
        assert_ne!(effective_folder(&pool, "t").await, "a-trash");
    }

    #[tokio::test]
    async fn a_deleted_rule_runs_no_more() {
        let pool = pool().await;
        insert_account(&pool, "a").await;
        insert_thread(
            &pool,
            "a",
            "t",
            "news@shop.example",
            20,
            "inbox",
            "inbox",
            true,
        )
        .await;
        let saved = rule(&pool, "a", SweepAction::Trash, Some(10), false, None).await;
        SenderSweepRepository::new(&pool)
            .delete_rule(&saved.id)
            .await
            .unwrap();
        let queue = SyncQueue::new();
        let stale = run_sweep_rule(&pool, &queue, &saved, chrono::Utc::now(), SWEEP_PASS_LIMIT)
            .await
            .unwrap();
        assert!(stale.swept.is_empty());
        assert!(
            run_sweep_rules(&pool, &queue, chrono::Utc::now(), SWEEP_PASS_LIMIT)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(effective_folder(&pool, "t").await, "a-inbox");
    }

    #[tokio::test]
    async fn rules_of_a_switched_off_account_wait() {
        let pool = pool().await;
        insert_account(&pool, "a").await;
        insert_thread(
            &pool,
            "a",
            "t",
            "news@shop.example",
            20,
            "inbox",
            "inbox",
            true,
        )
        .await;
        rule(&pool, "a", SweepAction::Archive, Some(10), false, None).await;
        sqlx::query("UPDATE accounts SET is_active = 0 WHERE id = 'a'")
            .execute(&pool)
            .await
            .unwrap();
        let queue = SyncQueue::new();
        let outcomes = run_sweep_rules(&pool, &queue, chrono::Utc::now(), SWEEP_PASS_LIMIT)
            .await
            .unwrap();
        assert!(outcomes.iter().all(|outcome| outcome.swept.is_empty()));
        assert_eq!(state_of(&pool, "t").await, "inbox");
    }

    #[tokio::test]
    async fn a_trash_rule_without_an_age_never_runs() {
        let pool = pool().await;
        insert_account(&pool, "a").await;
        insert_thread(
            &pool,
            "a",
            "t",
            "news@shop.example",
            20,
            "inbox",
            "inbox",
            true,
        )
        .await;
        let mut saved = rule(&pool, "a", SweepAction::Trash, Some(10), false, None).await;
        // The table refuses such a row outright…
        let refused =
            sqlx::query("UPDATE sender_sweep_rules SET older_than_days = NULL WHERE id = ?")
                .bind(&saved.id)
                .execute(&pool)
                .await;
        assert!(refused.is_err());
        // …and the worker refuses one that reaches it anyway.
        saved.older_than_days = None;
        let queue = SyncQueue::new();
        assert!(
            run_sweep_rule(&pool, &queue, &saved, chrono::Utc::now(), SWEEP_PASS_LIMIT)
                .await
                .is_err()
        );
        assert_eq!(effective_folder(&pool, "t").await, "a-inbox");
    }
}
