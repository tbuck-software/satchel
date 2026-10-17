//! Bounded queue scheduling and retry bookkeeping shared by the app and tests.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use std::{collections::HashSet, sync::Arc, time::Duration};
use tokio::task::JoinSet;

use super::{
    errors::SyncResult,
    queue::{backoff_seconds, QueuedAction, SyncQueue, MAX_RETRIES},
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
        serde_json::json!({"key": "backend.errors.queueActionFailed", "params": {"message": failure.to_string()}}).to_string()
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
    let retry_count = action.retry_count + u32::from(!rate_limited);
    let rate_limit_rounds = action.rate_limit_rounds + u32::from(rate_limited);
    if failure.is_permanent()
        || retry_count > MAX_RETRIES
        || rate_limit_rounds > MAX_RATE_LIMIT_ROUNDS
    {
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
                let outcome = settle_action(&pool, &queue, &action, result).await;
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
}
