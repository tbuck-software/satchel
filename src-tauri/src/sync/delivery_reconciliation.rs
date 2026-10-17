//! App-owned reconciliation for provider-accepted messages whose terminal
//! queue marker could not be written immediately.

use std::{collections::HashMap, sync::Mutex, time::Duration};

use sqlx::SqlitePool;
use tauri::Emitter;

use crate::sync::{outbox::ACTION_SEND_MESSAGE, queue::SyncQueue};

#[derive(Debug)]
struct DeliveryReconciliationRequest {
    action_id: String,
    error: String,
}

#[derive(Debug)]
struct PendingDeliveryReconciliation {
    error: String,
    next_attempt: tokio::time::Instant,
    delay_seconds: u64,
}

/// Supervises all delivery-marker retries in one deduplicated, cancellable task.
pub struct DeliveryReconciler {
    sender: tokio::sync::mpsc::UnboundedSender<DeliveryReconciliationRequest>,
    receiver: Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<DeliveryReconciliationRequest>>>,
    handle: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
}

impl DeliveryReconciler {
    pub fn new() -> Self {
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        Self {
            sender,
            receiver: Mutex::new(Some(receiver)),
            handle: Mutex::new(None),
        }
    }

    pub fn start(&self, queue: std::sync::Arc<SyncQueue>, pool: SqlitePool, app: tauri::AppHandle) {
        let Some(mut receiver) = self.receiver.lock().ok().and_then(|mut slot| slot.take()) else {
            tracing::error!("delivery reconciler was already started or its lock was poisoned");
            return;
        };
        let handle = tauri::async_runtime::spawn(async move {
            let mut pending = HashMap::<String, PendingDeliveryReconciliation>::new();
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            loop {
                tokio::select! {
                    request = receiver.recv() => {
                        let Some(request) = request else { return; };
                        insert_pending(&mut pending, request, tokio::time::Instant::now());
                    }
                    _ = interval.tick() => {
                        reconcile_due(&queue, &pool, &app, &mut pending).await;
                    }
                }
            }
        });
        if let Ok(mut slot) = self.handle.lock() {
            *slot = Some(handle);
        } else {
            handle.abort();
            tracing::error!("delivery reconciler handle lock was poisoned");
        }
    }

    pub fn enqueue(&self, action_id: String, error: String, initial_marker_error: String) {
        tracing::warn!(
            marker_error = initial_marker_error,
            action = action_id,
            "delivery marker unavailable; queued for reconciliation"
        );
        if self
            .sender
            .send(DeliveryReconciliationRequest { action_id, error })
            .is_err()
        {
            tracing::error!("delivery reconciler is not running");
        }
    }

    pub fn shutdown(&self) {
        if let Ok(mut slot) = self.handle.lock() {
            if let Some(handle) = slot.take() {
                handle.abort();
            }
        }
    }
}

impl Default for DeliveryReconciler {
    fn default() -> Self {
        Self::new()
    }
}

fn insert_pending(
    pending: &mut HashMap<String, PendingDeliveryReconciliation>,
    request: DeliveryReconciliationRequest,
    now: tokio::time::Instant,
) {
    pending.insert(
        request.action_id,
        PendingDeliveryReconciliation {
            error: request.error,
            next_attempt: now + Duration::from_secs(1),
            delay_seconds: 1,
        },
    );
}

async fn reconcile_due(
    queue: &SyncQueue,
    pool: &SqlitePool,
    app: &tauri::AppHandle,
    pending: &mut HashMap<String, PendingDeliveryReconciliation>,
) {
    let now = tokio::time::Instant::now();
    let due_ids = pending
        .iter()
        .filter(|(_, item)| item.next_attempt <= now)
        .map(|(action_id, _)| action_id.clone())
        .collect::<Vec<_>>();
    for action_id in due_ids {
        let Some(mut item) = pending.remove(&action_id) else {
            continue;
        };
        match queue
            .mark_delivered_with_error(pool, &action_id, &item.error)
            .await
        {
            Ok(()) => {
                let _ = app.emit(
                    "outbox:failed",
                    serde_json::json!({
                        "queued_id": action_id,
                        "action_type": ACTION_SEND_MESSAGE,
                        "status": "delivered",
                        "error": item.error,
                    }),
                );
                tracing::info!(
                    action = action_id,
                    "reconciled delivered-mail terminal state"
                );
            }
            Err(marker_error) => {
                tracing::debug!(
                    error = marker_error,
                    action = action_id,
                    "delivery marker still unavailable"
                );
                item.delay_seconds = (item.delay_seconds * 2).min(60);
                item.next_attempt =
                    tokio::time::Instant::now() + Duration::from_secs(item.delay_seconds);
                pending.insert(action_id, item);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct DropSignal(Option<tokio::sync::oneshot::Sender<()>>);

    impl Drop for DropSignal {
        fn drop(&mut self) {
            if let Some(sender) = self.0.take() {
                let _ = sender.send(());
            }
        }
    }

    #[tokio::test]
    async fn duplicate_action_ids_share_one_pending_reconciliation() {
        let now = tokio::time::Instant::now();
        let mut pending = HashMap::new();
        insert_pending(
            &mut pending,
            DeliveryReconciliationRequest {
                action_id: "send-1".into(),
                error: "first".into(),
            },
            now,
        );
        insert_pending(
            &mut pending,
            DeliveryReconciliationRequest {
                action_id: "send-1".into(),
                error: "latest".into(),
            },
            now,
        );

        assert_eq!(pending.len(), 1);
        assert_eq!(pending["send-1"].error, "latest");
        assert_eq!(pending["send-1"].delay_seconds, 1);
    }

    #[tokio::test]
    async fn shutdown_aborts_the_owned_supervisor_task() {
        let reconciler = DeliveryReconciler::new();
        let (started_sender, started_receiver) = tokio::sync::oneshot::channel();
        let (dropped_sender, dropped_receiver) = tokio::sync::oneshot::channel();
        let handle = tauri::async_runtime::spawn(async move {
            let _drop_signal = DropSignal(Some(dropped_sender));
            let _ = started_sender.send(());
            std::future::pending::<()>().await;
        });
        *reconciler.handle.lock().unwrap() = Some(handle);
        started_receiver.await.unwrap();

        reconciler.shutdown();

        tokio::time::timeout(Duration::from_secs(1), dropped_receiver)
            .await
            .expect("supervisor task should be cancelled")
            .expect("drop signal");
        assert!(reconciler.handle.lock().unwrap().is_none());
    }
}
