use sqlx::SqlitePool;

use satchel_core::db::connection_state::{ConnectionStateRepository, ConnectionStateSnapshot};

pub(crate) async fn database_snapshot(pool: &SqlitePool) -> ConnectionStateSnapshot {
    match ConnectionStateRepository::new(pool).snapshot().await {
        Ok(snapshot) => snapshot,
        Err(error) => {
            tracing::warn!(?error, "connection state database snapshot unavailable");
            ConnectionStateSnapshot::default()
        }
    }
}

use std::sync::{Arc, Mutex};

use crate::events::bus::{AppEvent, EventBus};

/// Counts work, not accounts: a manual sync can overlap IDLE and queued sends.
/// Only guard ownership changes the count; legacy completion notifications do not.
pub(crate) struct SyncActivity {
    active: Mutex<usize>,
    events: Arc<EventBus>,
}

impl SyncActivity {
    pub(crate) fn new(events: Arc<EventBus>) -> Self {
        Self {
            active: Mutex::new(0),
            events,
        }
    }

    pub(crate) fn is_syncing(&self) -> bool {
        *self
            .active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            > 0
    }

    pub(crate) fn start_work(self: &Arc<Self>) -> SyncActivityGuard {
        let mut active = self
            .active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *active += 1;
        if *active == 1 {
            self.events.publish(AppEvent::SyncStarted);
        }
        SyncActivityGuard(self.clone())
    }
}

/// Dropping an operation future, including cancellation or timeout, releases its work.
pub(crate) struct SyncActivityGuard(Arc<SyncActivity>);

impl Drop for SyncActivityGuard {
    fn drop(&mut self) {
        let mut active = self
            .0
            .active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *active -= 1;
        if *active == 0 {
            self.0.events.publish(AppEvent::SyncCompleted);
        }
    }
}

#[cfg(test)]
mod activity_tests {
    use super::*;

    #[test]
    fn legacy_completion_events_do_not_end_work_or_change_another_tracker() {
        let events = Arc::new(EventBus::new(16));
        let activity = Arc::new(SyncActivity::new(events.clone()));
        let independent = Arc::new(SyncActivity::new(events.clone()));
        let clone = activity.clone();
        let work = clone.start_work();
        events.publish(AppEvent::SyncCompleted);
        assert!(activity.is_syncing());
        assert!(!independent.is_syncing());
        drop(work);
        assert!(!activity.is_syncing());
    }

    #[tokio::test]
    async fn overlapping_account_runs_remain_busy_until_the_last_finishes() {
        let events = Arc::new(EventBus::new(16));
        let mut received = events.subscribe();
        let activity = Arc::new(SyncActivity::new(events));
        let ready = Arc::new(tokio::sync::Barrier::new(3));
        let (finish_a, wait_a) = tokio::sync::oneshot::channel::<()>();
        let (finish_b, wait_b) = tokio::sync::oneshot::channel::<()>();
        let mut tasks = Vec::new();
        for wait in [wait_a, wait_b] {
            let activity = activity.clone();
            let ready = ready.clone();
            tasks.push(tokio::spawn(async move {
                let _work = activity.start_work();
                ready.wait().await;
                wait.await.unwrap();
            }));
        }
        ready.wait().await;
        assert!(activity.is_syncing());
        assert!(matches!(received.try_recv(), Ok(AppEvent::SyncStarted)));
        assert!(received.try_recv().is_err());
        finish_a.send(()).unwrap();
        tasks.remove(0).await.unwrap();
        assert!(activity.is_syncing());
        assert!(received.try_recv().is_err());
        finish_b.send(()).unwrap();
        tasks.remove(0).await.unwrap();
        assert!(!activity.is_syncing());
        assert!(matches!(received.try_recv(), Ok(AppEvent::SyncCompleted)));
    }

    #[tokio::test]
    async fn failed_and_cancelled_work_release_only_their_own_activity() {
        let events = Arc::new(EventBus::new(16));
        let activity = Arc::new(SyncActivity::new(events));
        let other = activity.start_work();
        let failed: Result<(), &str> = async {
            let _work = activity.start_work();
            Err("account credential rejected")
        }
        .await;
        assert!(failed.is_err());
        assert!(activity.is_syncing());
        let (entered, ready) = tokio::sync::oneshot::channel();
        let running = tokio::spawn({
            let activity = activity.clone();
            async move {
                let _work = activity.start_work();
                entered.send(()).unwrap();
                std::future::pending::<()>().await;
            }
        });
        ready.await.unwrap();
        running.abort();
        assert!(running.await.unwrap_err().is_cancelled());
        assert!(activity.is_syncing());
        drop(other);
        assert!(!activity.is_syncing());
    }
}
