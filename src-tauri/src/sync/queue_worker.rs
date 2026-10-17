//! Tauri adapters for the Core queue scheduler and its notifications.
use crate::sync::{
    leases::sync_run_lock,
    outbox::{execute_send_action, ACTION_SEND_MESSAGE},
    remote_mutations::{RemoteMutationPayload, ACTION_REMOTE_MUTATION},
    workflow_executor::execute_queued_remote_mutation,
};
use crate::{events::bus::AppEvent, AppState};
use async_trait::async_trait;
use satchel_core::drafts::execute_queued_draft_sync;
use satchel_core::sync::{
    errors::SyncResult,
    queue::{QueuedAction, ACTION_SYNC_DRAFT},
    worker::{ActionExecutor, Outcome, QueueWorker},
};
use sqlx::SqlitePool;
use std::sync::Arc;
use tauri::{AppHandle, Emitter};

pub fn spawn_queue_worker(state: AppState, pool: SqlitePool, app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        satchel_core::sync::worker::retry_with_backoff(
            "queue recovery",
            std::time::Duration::from_secs(1),
            std::time::Duration::from_secs(60),
            || state.queue.reset_sending(&pool),
        )
        .await;
        let repaired = satchel_core::sync::worker::retry_with_backoff(
            "POP3 move recovery",
            std::time::Duration::from_secs(1),
            std::time::Duration::from_secs(60),
            || satchel_core::mail_actions::reconcile_all_legacy_pop3_moves(&pool),
        )
        .await;
        if repaired > 0 {
            tracing::info!(repaired, "reconciled legacy POP3 moves");
            state.events.publish(AppEvent::SyncCompleted);
        }
        // Periodic connection-state emitter: every 10s publish current pending/failed counts
        let pool_for_ticker = pool.clone();
        let app_for_ticker = app.clone();
        let activity_for_ticker = state.sync_activity.clone();
        tauri::async_runtime::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));
            loop {
                interval.tick().await;
                let snapshot = crate::connection_state::database_snapshot(&pool_for_ticker).await;
                let unread = crate::commands::tray::open_unread_count(&pool_for_ticker)
                    .await
                    .unwrap_or(0);
                let _ = app_for_ticker.emit(
                    "connection:state",
                    serde_json::json!({
                        "online": null,
                        "syncing": activity_for_ticker.is_syncing(),
                        "last_sync_at": snapshot.last_sync_at,
                        "pending_outbox": snapshot.pending_outbox,
                        "pending_sends": snapshot.pending_sends,
                        "failed_outbox": snapshot.failed_outbox,
                        "failed_sends": snapshot.failed_sends,
                        "failed_remote_mutations": snapshot.failed_remote_mutations,
                        "unread_count": unread,
                    }),
                );
            }
        });

        let executor = Arc::new(TauriExecutor {
            pool: pool.clone(),
            state: state.clone(),
            app,
        });
        QueueWorker::new(
            pool,
            state.queue.clone(),
            executor,
            if cfg!(mobile) { 2 } else { 4 },
        )
        .run()
        .await;
    });
}

struct TauriExecutor {
    pool: SqlitePool,
    state: AppState,
    app: AppHandle,
}

#[async_trait]
impl ActionExecutor for TauriExecutor {
    type Permit = Option<tokio::sync::OwnedMutexGuard<()>>;

    async fn try_acquire(&self, action: &QueuedAction) -> Result<Option<Self::Permit>, String> {
        if !queued_action_needs_sync_permit(&self.pool, action).await? {
            return Ok(Some(None));
        }
        Ok(sync_run_lock(&action.account_id)
            .try_lock_owned()
            .ok()
            .map(Some))
    }

    async fn execute(&self, action: &QueuedAction) -> SyncResult<()> {
        let _work = self.state.sync_activity.start_work();
        match action.action_type.as_str() {
            ACTION_SEND_MESSAGE => execute_send_action(&self.pool, &self.app, &self.state, action)
                .await
                .map(|_| ()),
            ACTION_REMOTE_MUTATION => {
                execute_queued_remote_mutation(&self.pool, Some(&self.app), &self.state, action)
                    .await
            }
            ACTION_SYNC_DRAFT => {
                execute_queued_draft_sync(&self.pool, &self.state.queue, action).await
            }
            other => Err(format!("unknown action_type `{other}`").into()),
        }
    }

    async fn observe(&self, action: &QueuedAction, outcome: Result<Outcome, String>) {
        match outcome {
            Ok(Outcome::Completed) => {
                self.state.events.publish(AppEvent::SyncCompleted);
                if action.action_type == ACTION_SEND_MESSAGE {
                    let _ = self.app.emit("outbox:sent", serde_json::json!({"queued_id": action.id, "action_type": action.action_type}));
                }
            }
            Ok(Outcome::Retry {
                retry_count,
                next_at,
                error,
            }) => {
                let _ = self.app.emit("outbox:retry", serde_json::json!({"queued_id": action.id, "retry_count": retry_count, "next_retry_at": next_at.to_rfc3339(), "error": error}));
            }
            Ok(Outcome::Failed { error }) => {
                let send = action.action_type == ACTION_SEND_MESSAGE;
                let strings = crate::commands::ui_strings::current();
                self.state.events.publish(AppEvent::Notification {
                    title: if send {
                        crate::events::bus::NotificationTitle::SendFailed
                    } else {
                        crate::events::bus::NotificationTitle::SyncFailed
                    },
                    body: if send {
                        strings.notification_send_failed
                    } else {
                        strings.notification_sync_failed
                    },
                });
                let _ = self.app.emit("outbox:failed", serde_json::json!({"queued_id": action.id, "action_type": action.action_type, "error": error}));
            }
            Ok(Outcome::Delivered { error }) => {
                let _ = self.app.emit("outbox:failed", serde_json::json!({"queued_id": action.id, "action_type": action.action_type, "status": "delivered", "error": error}));
            }
            // Reported like a failure so the outbox shows it, but the status
            // says the send may have gone out — the user decides, not a retry.
            Ok(Outcome::DeliveryUnknown { error }) => {
                let strings = crate::commands::ui_strings::current();
                self.state.events.publish(AppEvent::Notification {
                    title: crate::events::bus::NotificationTitle::SendFailed,
                    body: strings.notification_send_failed,
                });
                let _ = self.app.emit("outbox:failed", serde_json::json!({"queued_id": action.id, "action_type": action.action_type, "status": "delivery_unknown", "error": error}));
            }
            Ok(Outcome::DeliveryUnrecorded {
                error,
                marker_error,
            }) => {
                self.state
                    .delivery_reconciler
                    .enqueue(action.id.clone(), error, marker_error);
            }
            Err(error) => tracing::error!(%error, action = %action.id, "queue bookkeeping failed"),
        }
    }
}

async fn queued_action_needs_sync_permit(
    pool: &SqlitePool,
    action: &crate::sync::queue::QueuedAction,
) -> Result<bool, String> {
    if action.action_type != ACTION_REMOTE_MUTATION
        || !matches!(
            serde_json::from_value::<RemoteMutationPayload>(action.payload.clone()),
            Ok(RemoteMutationPayload::WorkflowMove { .. })
                | Ok(RemoteMutationPayload::BlockedMove { .. })
        )
    {
        return Ok(false);
    }
    let provider = sqlx::query_scalar::<_, String>(
        "SELECT provider FROM accounts WHERE id = ? AND is_active = 1 LIMIT 1",
    )
    .bind(&action.account_id)
    .fetch_optional(pool)
    .await
    .map_err(|err| err.to_string())?;
    Ok(provider.is_some_and(|provider| provider.eq_ignore_ascii_case("imap")))
}
