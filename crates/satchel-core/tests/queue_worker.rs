//! Exercise the production dispatcher and retry policy against real SQLite.
use async_trait::async_trait;
use satchel_core::sync::{
    errors::{SyncError, SyncResult},
    queue::{QueuedAction, SyncQueue, MAX_RETRIES},
    worker::{ActionExecutor, Outcome, QueueWorker},
};
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::Semaphore;

struct Executor {
    blocked: Mutex<HashSet<String>>,
    failure: Mutex<Option<SyncError>>,
    started: Mutex<Vec<String>>,
    outcomes: Mutex<Vec<Outcome>>,
    gate: Semaphore,
}
impl Executor {
    fn new() -> Self {
        Self {
            blocked: Mutex::default(),
            failure: Mutex::default(),
            started: Mutex::default(),
            outcomes: Mutex::default(),
            gate: Semaphore::new(0),
        }
    }
}
#[async_trait]
impl ActionExecutor for Executor {
    type Permit = ();
    async fn try_acquire(&self, action: &QueuedAction) -> Result<Option<()>, String> {
        Ok((!self.blocked.lock().unwrap().contains(&action.account_id)).then_some(()))
    }
    async fn execute(&self, action: &QueuedAction) -> SyncResult<()> {
        self.started.lock().unwrap().push(action.account_id.clone());
        self.gate.acquire().await.unwrap().forget();
        match self.failure.lock().unwrap().clone() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
    async fn observe(&self, _: &QueuedAction, outcome: Result<Outcome, String>) {
        self.outcomes.lock().unwrap().push(outcome.unwrap());
    }
}
async fn fixture() -> (SqlitePool, Arc<SyncQueue>, Arc<Executor>) {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::migrate!("./src/db/migrations")
        .run(&pool)
        .await
        .unwrap();
    (pool, Arc::new(SyncQueue::new()), Arc::new(Executor::new()))
}
async fn enqueue(pool: &SqlitePool, queue: &SyncQueue, account: &str, kind: &str) -> String {
    let action = QueuedAction::new(account.into(), kind, serde_json::json!({}));
    let id = action.id.clone();
    queue.enqueue(pool, action).await.unwrap();
    id
}
async fn status(pool: &SqlitePool, id: &str) -> Option<String> {
    sqlx::query_scalar("SELECT status FROM sync_queue WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
        .unwrap()
}
#[tokio::test]
async fn busy_account_does_not_block_another_accounts_send() {
    let (pool, queue, executor) = fixture().await;
    let busy = enqueue(&pool, &queue, "a", "remote_mutation").await;
    let send = enqueue(&pool, &queue, "b", "send_message").await;
    executor.blocked.lock().unwrap().insert("a".into());
    let mut worker = QueueWorker::new(pool.clone(), queue.clone(), executor.clone(), 2);
    assert_eq!(worker.dispatch_ready().await.unwrap(), 1);
    assert_eq!(status(&pool, &busy).await.as_deref(), Some("pending"));
    executor.gate.add_permits(1);
    tokio::time::timeout(Duration::from_secs(2), worker.wait_for_completion())
        .await
        .unwrap();
    assert_eq!(status(&pool, &send).await, None);
    executor.blocked.lock().unwrap().clear();
    assert_eq!(worker.dispatch_ready().await.unwrap(), 1);
    executor.gate.add_permits(1);
    worker.wait_for_completion().await;
    assert_eq!(status(&pool, &busy).await, None);
}
#[tokio::test]
async fn concurrency_is_bounded_and_each_account_is_serial() {
    let (pool, queue, executor) = fixture().await;
    for account in ["a", "a", "b", "c"] {
        enqueue(&pool, &queue, account, "remote_mutation").await;
    }
    let mut worker = QueueWorker::new(pool.clone(), queue, executor.clone(), 2);
    assert_eq!(worker.dispatch_ready().await.unwrap(), 2);
    assert_eq!(worker.dispatch_ready().await.unwrap(), 0);
    let active: Vec<String> =
        sqlx::query_scalar("SELECT account_id FROM sync_queue WHERE status = 'sending'")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(active.len(), 2);
    assert_eq!(active.into_iter().collect::<HashSet<_>>().len(), 2);
    executor.gate.add_permits(4);
    worker.wait_for_completion().await;
    worker.dispatch_ready().await.unwrap();
    while status_count(&pool).await > 0 {
        worker.wait_for_completion().await;
        worker.dispatch_ready().await.unwrap();
    }
}
async fn status_count(pool: &SqlitePool) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM sync_queue")
        .fetch_one(pool)
        .await
        .unwrap()
}
#[tokio::test]
async fn production_worker_exhausts_transient_budget_and_preserves_order() {
    let (pool, queue, executor) = fixture().await;
    let first = enqueue(&pool, &queue, "a", "remote_mutation").await;
    let second = enqueue(&pool, &queue, "a", "remote_mutation").await;
    *executor.failure.lock().unwrap() = Some(SyncError::Transient("connection reset".into()));
    let mut worker = QueueWorker::new(pool.clone(), queue, executor.clone(), 2);
    for attempt in 0..=MAX_RETRIES {
        assert_eq!(worker.dispatch_ready().await.unwrap(), 1);
        executor.gate.add_permits(1);
        worker.wait_for_completion().await;
        if attempt < MAX_RETRIES {
            assert_eq!(
                worker.dispatch_ready().await.unwrap(),
                0,
                "later mutation must not overtake a retry"
            );
            sqlx::query("UPDATE sync_queue SET next_retry_at = '2000-01-01' WHERE id = ?")
                .bind(&first)
                .execute(&pool)
                .await
                .unwrap();
        }
    }
    assert_eq!(status(&pool, &first).await.as_deref(), Some("failed"));
    assert_eq!(status(&pool, &second).await.as_deref(), Some("pending"));
    let outcomes = executor.outcomes.lock().unwrap();
    let counts: Vec<u32> = outcomes
        .iter()
        .filter_map(|o| match o {
            Outcome::Retry { retry_count, .. } => Some(*retry_count),
            _ => None,
        })
        .collect();
    assert_eq!(counts, (1..=MAX_RETRIES).collect::<Vec<_>>());
    assert!(matches!(outcomes.last(), Some(Outcome::Failed { .. })));
}
#[tokio::test]
async fn rate_limits_preserve_budget_and_delivered_mail_never_retries() {
    let (pool, queue, executor) = fixture().await;
    let id = enqueue(&pool, &queue, "a", "send_message").await;
    *executor.failure.lock().unwrap() = Some(SyncError::rate_limited(
        "slow down",
        Some(Duration::from_secs(60)),
    ));
    let mut worker = QueueWorker::new(pool.clone(), queue, executor.clone(), 1);
    worker.dispatch_ready().await.unwrap();
    executor.gate.add_permits(1);
    worker.wait_for_completion().await;
    {
        let outcomes = executor.outcomes.lock().unwrap();
        assert!(
            matches!(outcomes.last(), Some(Outcome::Retry { retry_count: 0, next_at, .. }) if *next_at > chrono::Utc::now() + chrono::Duration::seconds(85))
        );
    }
    sqlx::query("UPDATE sync_queue SET next_retry_at = '2000-01-01' WHERE id = ?")
        .bind(&id)
        .execute(&pool)
        .await
        .unwrap();
    *executor.failure.lock().unwrap() = Some(SyncError::Delivered("sent record failed".into()));
    worker.dispatch_ready().await.unwrap();
    executor.gate.add_permits(1);
    worker.wait_for_completion().await;
    assert_eq!(status(&pool, &id).await.as_deref(), Some("delivered"));
    assert_eq!(worker.dispatch_ready().await.unwrap(), 0);
}
#[tokio::test]
async fn permanent_failure_is_terminal_on_first_execution() {
    let (pool, queue, executor) = fixture().await;
    let id = enqueue(&pool, &queue, "a", "send_message").await;
    *executor.failure.lock().unwrap() = Some(SyncError::Permanent("authentication failed".into()));
    let mut worker = QueueWorker::new(pool.clone(), queue, executor.clone(), 1);
    worker.dispatch_ready().await.unwrap();
    executor.gate.add_permits(1);
    worker.wait_for_completion().await;
    assert_eq!(status(&pool, &id).await.as_deref(), Some("failed"));
    assert_eq!(worker.dispatch_ready().await.unwrap(), 0);
}

#[tokio::test]
async fn failed_delivery_marker_never_requeues_a_delivered_send() {
    let (pool, queue, executor) = fixture().await;
    let id = enqueue(&pool, &queue, "a", "send_message").await;
    sqlx::query("CREATE TRIGGER reject_delivery BEFORE UPDATE OF status ON sync_queue WHEN NEW.status = 'delivered' BEGIN SELECT RAISE(ABORT, 'disk full'); END")
        .execute(&pool).await.unwrap();
    *executor.failure.lock().unwrap() = Some(SyncError::Delivered("sent record failed".into()));
    let mut worker = QueueWorker::new(pool.clone(), queue, executor.clone(), 1);
    worker.dispatch_ready().await.unwrap();
    executor.gate.add_permits(1);
    worker.wait_for_completion().await;
    assert_eq!(status(&pool, &id).await.as_deref(), Some("sending"));
    assert!(matches!(
        executor.outcomes.lock().unwrap().last(),
        Some(Outcome::DeliveryUnrecorded { .. })
    ));
    assert_eq!(worker.dispatch_ready().await.unwrap(), 0);
    assert_eq!(executor.started.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn startup_recovery_retries_a_database_error_then_dispatches() {
    use satchel_core::sync::worker::retry_with_backoff;
    let (pool, queue, executor) = fixture().await;
    let id = enqueue(&pool, &queue, "a", "send_message").await;
    queue.claim_ready(&pool, &id).await.unwrap().unwrap();
    sqlx::query("UPDATE sync_queue SET sending_session = NULL WHERE id = ?")
        .bind(&id)
        .execute(&pool)
        .await
        .unwrap();
    let mut live = QueuedAction::new("b".into(), "send_message", serde_json::json!({}));
    live.status = "sending".into();
    queue.enqueue(&pool, live.clone()).await.unwrap();
    sqlx::raw_sql("CREATE TRIGGER block_recovery BEFORE UPDATE ON sync_queue BEGIN SELECT RAISE(ABORT, 'busy startup'); END;")
        .execute(&pool).await.unwrap();
    let mut calls = 0;
    retry_with_backoff(
        "test recovery",
        Duration::from_millis(1),
        Duration::from_millis(4),
        || {
            calls += 1;
            let pool = &pool;
            let queue = &queue;
            let attempt = calls;
            async move {
                if attempt == 3 {
                    sqlx::query("DROP TRIGGER block_recovery")
                        .execute(pool)
                        .await
                        .map_err(|e| e.to_string())?;
                }
                queue.reset_sending(pool).await
            }
        },
    )
    .await;
    assert_eq!(calls, 3);
    assert_eq!(status(&pool, &live.id).await.as_deref(), Some("sending"));
    assert_eq!(status(&pool, &id).await.as_deref(), Some("pending"));
    let mut worker = QueueWorker::new(pool.clone(), queue, executor.clone(), 1);
    assert_eq!(worker.dispatch_ready().await.unwrap(), 1);
    executor.gate.add_permits(1);
    worker.wait_for_completion().await;
    assert_eq!(status(&pool, &id).await, None);
}
