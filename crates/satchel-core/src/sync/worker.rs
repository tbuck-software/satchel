//! Queue scheduling and retry bookkeeping shared by the app and tests.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use std::{collections::HashSet, sync::Arc, time::Duration};
use tokio::task::JoinSet;

use super::{
    errors::SyncResult,
    queue::{backoff_seconds, QueuedAction, SyncQueue},
};

/// How often one action may be deferred for a provider rate limit before it is
/// treated as failed. At the 15-minute fallback delay this is about six hours.
pub const MAX_RATE_LIMIT_ROUNDS: u32 = 24;

#[derive(Debug)]
pub enum Outcome {
    Completed,
    Retry {
        retry_count: u32,
        next_at: DateTime<Utc>,
        error: String,
    },
    Failed {
        error: String,
    },
    Delivered {
        error: String,
    },
    DeliveryUnrecorded {
        error: String,
        marker_error: String,
    },
    /// The message went over the wire and the server never answered.
    DeliveryUnknown {
        error: String,
    },
}

/// Retry background startup/recovery without abandoning the worker for this session.
pub async fn retry_with_backoff<T, F, Fut>(
    label: &str,
    initial_delay: Duration,
    max_delay: Duration,
    mut operation: F,
) -> T
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, String>>,
{
    let mut delay = initial_delay;
    loop {
        match operation().await {
            Ok(value) => return value,
            Err(error) => {
                tracing::warn!(%error, %label, ?delay, "background operation failed; retrying");
                tokio::time::sleep(delay).await;
                delay = delay.saturating_mul(2).min(max_delay);
            }
        }
    }
}

pub async fn mark_delivered_terminal(
    queue: &SyncQueue,
    pool: &SqlitePool,
    id: &str,
    error: &str,
) -> Result<(), String> {
    let mut last_error = None;
    for delay in [0, 100, 500] {
        if delay > 0 {
            tokio::time::sleep(Duration::from_millis(delay)).await;
        }
        match queue.mark_delivered_with_error(pool, id, error).await {
            Ok(()) => return Ok(()),
            Err(e) => last_error = Some(e),
        }
    }
    Err(last_error.unwrap_or_else(|| "delivery terminalization failed".into()))
}

pub async fn settle_action(
    pool: &SqlitePool,
    queue: &SyncQueue,
    action: &QueuedAction,
    result: SyncResult<()>,
) -> Result<Outcome, String> {
    let Err(failure) = result else {
        queue.mark_completed(pool, &action.id).await?;
        return Ok(Outcome::Completed);
    };
    let error = if failure.was_delivered() {
        failure.to_string()
    } else {
        failure.encoded()
    };
    if failure.was_delivered() {
        return Ok(
            match mark_delivered_terminal(queue, pool, &action.id, &error).await {
                Ok(()) => Outcome::Delivered { error },
                Err(marker_error) => Outcome::DeliveryUnrecorded {
                    error,
                    marker_error,
                },
            },
        );
    }
    // Neither sent nor failed: the body went over and the answer did not come
    // back. Retrying would risk a second delivery, so this waits for the user.
    // The row carries the same message as the crash path — what the user needs
    // is the advice to check the Sent folder, not the socket error.
    if failure.is_delivery_unknown() {
        queue.mark_delivery_unknown(pool, &action.id).await?;
        return Ok(Outcome::DeliveryUnknown { error });
    }
    // A rate limit is the provider asking us to wait, not this action failing,
    // so it does not consume a retry. It needs a bound of its own all the same:
    // without one the action circulates forever, never reaches `failed`, and so
    // never shows up in any failure count.
    let rate_limited = failure.is_rate_limited();
    let retry_count = action.retry_count.saturating_add(u32::from(!rate_limited));
    let rate_limit_rounds = action
        .rate_limit_rounds
        .saturating_add(u32::from(rate_limited));
    // Elapsed attempts do not make a network outage permanent.
    if failure.is_permanent() || rate_limit_rounds > MAX_RATE_LIMIT_ROUNDS {
        queue
            .mark_failed_permanent(pool, &action.id, &error)
            .await?;
        return Ok(Outcome::Failed { error });
    }
    let delay = if rate_limited {
        failure
            .retry_after()
            .map(|d| d + Duration::from_secs(30))
            .unwrap_or(Duration::from_secs(900))
    } else {
        Duration::from_secs(backoff_seconds(retry_count) as u64)
    };
    let next_at = Utc::now() + chrono::Duration::from_std(delay).map_err(|e| e.to_string())?;
    if rate_limited {
        queue
            .mark_rate_limited_retry(pool, &action.id, rate_limit_rounds, next_at, &error)
            .await?;
    } else {
        queue
            .mark_failed_retry(pool, &action.id, retry_count, next_at, &error)
            .await?;
    }
    Ok(Outcome::Retry {
        retry_count,
        next_at,
        error,
    })
}

/// Settle an action, retrying the bookkeeping itself a few times.
///
/// The provider work already happened; only its record is missing. A row left
/// in `sending` belongs to this session, so `reset_sending` does not pick it up
/// before the next start, and `ensure_no_pending_move` keeps refusing every
/// further action on the thread until then. Every branch of [`settle_action`]
/// writes an absolute state, so running it again is safe.
pub async fn settle_action_with_retry(
    pool: &SqlitePool,
    queue: &SyncQueue,
    action: &QueuedAction,
    result: SyncResult<()>,
) -> Result<Outcome, String> {
    let mut last_error = None;
    for delay in SETTLE_RETRY_DELAYS_MS {
        if delay > 0 {
            tokio::time::sleep(Duration::from_millis(delay)).await;
        }
        match settle_action(pool, queue, action, result.clone()).await {
            Ok(outcome) => return Ok(outcome),
            Err(error) => {
                tracing::warn!(%error, action = %action.id, "queue bookkeeping failed; retrying");
                last_error = Some(error);
            }
        }
    }
    Err(last_error.unwrap_or_else(|| "queue bookkeeping failed".into()))
}

const SETTLE_RETRY_DELAYS_MS: [u64; 4] = [0, 250, 1_000, 5_000];

/// How a drain over a fixed set of queue rows ended.
#[derive(Debug, PartialEq, Eq)]
pub enum DrainOutcome {
    /// Every row ran and is gone.
    Completed,
    /// A row is waiting for its next attempt or is being executed elsewhere.
    Pending { error: String },
    /// A row has reached `failed`. It stays visible to the user, who can retry
    /// or discard it; draining again cannot change that.
    Failed { error: String },
}

/// Execute the given rows in order outside the [`QueueWorker`], settling each
/// one exactly as the worker does: same capped backoff, same permanent-error
/// path. Stops at the first row that is not done.
///
/// Android's notification quick actions run this in their own process. Before
/// it went through [`settle_action`], every failure was rescheduled without a
/// bound, so a move that could never succeed stayed pending forever and kept
/// its thread locked against any further action.
pub async fn drain_actions(
    pool: &SqlitePool,
    queue: &SyncQueue,
    ids: &[String],
    mut execute: impl AsyncFnMut(&QueuedAction) -> SyncResult<()>,
) -> Result<DrainOutcome, String> {
    queue.reset_stale_sending_ids(pool, ids).await?;
    for id in ids {
        let Some(action) = queue.claim_ready(pool, id).await? else {
            let row: Option<(String, Option<String>)> =
                sqlx::query_as("SELECT status, last_error FROM sync_queue WHERE id = ?")
                    .bind(id)
                    .fetch_optional(pool)
                    .await
                    .map_err(|error| error.to_string())?;
            match row {
                None => continue,
                Some((status, error)) if status == "failed" => {
                    return Ok(DrainOutcome::Failed {
                        error: error.unwrap_or_default(),
                    });
                }
                Some(_) => {
                    return Ok(DrainOutcome::Pending {
                        error: "provider mutation is waiting for retry".into(),
                    });
                }
            }
        };
        let result = execute(&action).await;
        match settle_action_with_retry(pool, queue, &action, result).await? {
            Outcome::Completed => {}
            Outcome::Retry { error, .. } | Outcome::DeliveryUnrecorded { error, .. } => {
                return Ok(DrainOutcome::Pending { error });
            }
            Outcome::Failed { error }
            | Outcome::Delivered { error }
            | Outcome::DeliveryUnknown { error } => return Ok(DrainOutcome::Failed { error }),
        }
    }
    Ok(DrainOutcome::Completed)
}

#[async_trait]
pub trait ActionExecutor: Send + Sync + 'static {
    type Permit: Send + 'static;
    /// Return immediately when the account is busy; never wait on its lock.
    async fn try_acquire(&self, action: &QueuedAction) -> Result<Option<Self::Permit>, String>;
    async fn execute(&self, action: &QueuedAction) -> SyncResult<()>;
    async fn observe(&self, action: &QueuedAction, outcome: Result<Outcome, String>);
}

pub struct QueueWorker<E: ActionExecutor> {
    pool: SqlitePool,
    queue: Arc<SyncQueue>,
    executor: Arc<E>,
    active: HashSet<String>,
    tasks: JoinSet<String>,
    concurrency: usize,
}

impl<E: ActionExecutor> QueueWorker<E> {
    pub fn new(
        pool: SqlitePool,
        queue: Arc<SyncQueue>,
        executor: Arc<E>,
        concurrency: usize,
    ) -> Self {
        Self {
            pool,
            queue,
            executor,
            active: HashSet::new(),
            tasks: JoinSet::new(),
            concurrency: concurrency.max(1),
        }
    }

    fn retire(&mut self, result: Result<String, tokio::task::JoinError>) {
        match result {
            Ok(account) => {
                self.active.remove(&account);
            }
            Err(error) => {
                // Panics must not accidentally permit another action from an
                // account whose previous provider result is now unknown.
                tracing::error!(%error, "queue task interrupted; account held until restart");
            }
        }
    }

    /// Execute the production dispatch step without timers, for deterministic
    /// tests and embedding. Completion and retry handling run in the same tasks.
    pub async fn dispatch_ready(&mut self) -> Result<usize, String> {
        while let Some(done) = self.tasks.try_join_next() {
            self.retire(done);
        }
        let mut started = 0;
        for ready in self.queue.ready_account_heads(&self.pool).await? {
            if self.tasks.len() >= self.concurrency {
                break;
            }
            if self.active.contains(&ready.account_id) {
                continue;
            }
            let permit = match self.executor.try_acquire(&ready).await {
                Ok(Some(permit)) => permit,
                Ok(None) => continue,
                Err(error) => {
                    tracing::error!(%error, account = %ready.account_id, "queue execution permit unavailable");
                    continue;
                }
            };
            let Some(action) = self.queue.claim_ready(&self.pool, &ready.id).await? else {
                continue;
            };
            self.active.insert(action.account_id.clone());
            let (pool, queue, executor) =
                (self.pool.clone(), self.queue.clone(), self.executor.clone());
            self.tasks.spawn(async move {
                let result = executor.execute(&action).await;
                let outcome = settle_action_with_retry(&pool, &queue, &action, result).await;
                drop(permit);
                executor.observe(&action, outcome).await;
                action.account_id
            });
            started += 1;
        }
        Ok(started)
    }

    pub async fn wait_for_completion(&mut self) {
        if let Some(done) = self.tasks.join_next().await {
            self.retire(done);
        }
    }

    pub async fn run(mut self) {
        loop {
            if let Err(error) = self.dispatch_ready().await {
                tracing::error!(%error, "queue dispatch failed");
            }
            let delay = self
                .queue
                .next_wakeup(&self.pool)
                .await
                .map(|at| (at - Utc::now()).to_std().unwrap_or_default())
                .unwrap_or(Duration::from_secs(60))
                .clamp(Duration::from_secs(1), Duration::from_secs(60));
            tokio::select! {
                done = self.tasks.join_next(), if !self.tasks.is_empty() => {
                    if let Some(done) = done { self.retire(done); }
                }
                _ = self.queue.wait_for_enqueue() => {},
                // Revisit skipped account locks without tying up a worker slot.
                _ = tokio::time::sleep(delay) => {},
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::errors::SyncError;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn fresh_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory db");
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .expect("migrations");
        pool
    }

    /// Settle the same action over and over, always rate-limited, and report
    /// how many rounds it took to end up in `failed`.
    async fn rounds_until_failed() -> u32 {
        let pool = fresh_pool().await;
        let queue = SyncQueue::new();
        let mut action = QueuedAction::new(
            "acc-1".into(),
            super::super::queue::ACTION_SEND_MESSAGE,
            serde_json::json!({}),
        );
        queue.enqueue(&pool, action.clone()).await.expect("enqueue");

        for round in 1..=MAX_RATE_LIMIT_ROUNDS * 2 {
            let outcome = settle_action(
                &pool,
                &queue,
                &action,
                Err(SyncError::rate_limited("slow down", None)),
            )
            .await
            .expect("settled");
            if matches!(outcome, Outcome::Failed { .. }) {
                return round;
            }
            let stored: (i64, i64) = sqlx::query_as(
                "SELECT retry_count, rate_limit_rounds FROM sync_queue WHERE id = ?",
            )
            .bind(&action.id)
            .fetch_one(&pool)
            .await
            .expect("row");
            assert_eq!(stored.0, 0, "a rate limit must not consume a retry");
            action.rate_limit_rounds = stored.1 as u32;
        }
        panic!("a rate-limited action circulated past its own bound");
    }

    #[tokio::test]
    async fn a_permanently_rate_limited_action_stops_circulating() {
        assert_eq!(rounds_until_failed().await, MAX_RATE_LIMIT_ROUNDS + 1);
    }

    async fn notification_row(pool: &SqlitePool, queue: &SyncQueue) -> String {
        let mut action = QueuedAction::new(
            "acc-1".into(),
            "remote_mutation",
            serde_json::json!({"kind": "workflow_move"}),
        );
        action.id = "notification:digest:1".into();
        queue.enqueue(pool, action.clone()).await.expect("enqueue");
        action.id
    }

    async fn status_of(pool: &SqlitePool, id: &str) -> Option<String> {
        sqlx::query_scalar("SELECT status FROM sync_queue WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await
            .expect("status")
    }

    #[tokio::test]
    async fn a_drained_permanent_error_settles_as_failed_and_stays_there() {
        let pool = fresh_pool().await;
        let queue = SyncQueue::new();
        let id = notification_row(&pool, &queue).await;
        let mut calls = 0;
        let outcome = drain_actions(
            &pool,
            &queue,
            std::slice::from_ref(&id),
            async |_: &QueuedAction| {
                calls += 1;
                Err(SyncError::Permanent("imapUidValidityChanged".into()))
            },
        )
        .await
        .expect("drained");
        assert!(
            matches!(outcome, DrainOutcome::Failed { .. }),
            "{outcome:?}"
        );
        assert_eq!(status_of(&pool, &id).await.as_deref(), Some("failed"));

        // WorkManager coming back must not run it again nor report it pending.
        let again = drain_actions(
            &pool,
            &queue,
            std::slice::from_ref(&id),
            async |_: &QueuedAction| {
                calls += 1;
                Ok(())
            },
        )
        .await
        .expect("drained");
        assert!(matches!(again, DrainOutcome::Failed { .. }), "{again:?}");
        assert_eq!(calls, 1);
    }

    #[tokio::test]
    async fn a_drained_transient_error_keeps_retrying_then_recovers() {
        let pool = fresh_pool().await;
        let queue = SyncQueue::new();
        let id = notification_row(&pool, &queue).await;
        for _round in 1..=12 {
            let outcome = drain_actions(
                &pool,
                &queue,
                std::slice::from_ref(&id),
                async |_: &QueuedAction| Err(SyncError::Transient("connection reset".into())),
            )
            .await
            .expect("drained");
            match outcome {
                DrainOutcome::Pending { .. } => {}
                DrainOutcome::Failed { .. } => panic!("transient failure must stay pending"),
                DrainOutcome::Completed => panic!("a failing row cannot complete"),
            }
            // Skip the backoff so the next attempt finds the row due.
            sqlx::query("UPDATE sync_queue SET next_retry_at = NULL")
                .execute(&pool)
                .await
                .expect("due");
        }
        assert_eq!(status_of(&pool, &id).await.as_deref(), Some("pending"));
        let outcome = drain_actions(&pool, &queue, &[id], async |_: &QueuedAction| Ok(()))
            .await
            .expect("recovered");
        assert_eq!(outcome, DrainOutcome::Completed);
    }

    #[tokio::test]
    async fn a_drain_completes_and_skips_rows_already_done() {
        let pool = fresh_pool().await;
        let queue = SyncQueue::new();
        let id = notification_row(&pool, &queue).await;
        let ids = vec![id.clone(), "notification:digest:gone".into()];
        let outcome = drain_actions(&pool, &queue, &ids, async |_: &QueuedAction| Ok(()))
            .await
            .expect("drained");
        assert_eq!(outcome, DrainOutcome::Completed);
        assert_eq!(status_of(&pool, &id).await, None);
    }

    #[tokio::test]
    async fn settle_retries_its_bookkeeping_while_the_database_is_locked() {
        use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode};
        let directory = tempfile::tempdir().expect("temp directory");
        let options = SqliteConnectOptions::new()
            .filename(directory.path().join("settle.sqlite"))
            .busy_timeout(Duration::ZERO)
            .journal_mode(SqliteJournalMode::Wal)
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect_with(options)
            .await
            .expect("file pool");
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .expect("migrations");
        let queue = SyncQueue::new();
        let action = QueuedAction::new("acc-1".into(), "remote_mutation", serde_json::json!({}));
        queue.enqueue(&pool, action.clone()).await.expect("enqueue");
        let claimed = queue
            .claim_ready(&pool, &action.id)
            .await
            .expect("claim")
            .expect("claimed");

        let writer = pool.begin_with("BEGIN IMMEDIATE").await.expect("lock");
        assert!(
            settle_action(&pool, &queue, &claimed, Ok(()))
                .await
                .is_err(),
            "the lock must make a single attempt fail"
        );
        let release = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(400)).await;
            writer.commit().await.expect("release");
        });
        let outcome = settle_action_with_retry(&pool, &queue, &claimed, Ok(()))
            .await
            .expect("settled after the lock was released");
        release.await.expect("release task");
        assert!(matches!(outcome, Outcome::Completed));
        assert_eq!(status_of(&pool, &action.id).await, None);
    }
}
