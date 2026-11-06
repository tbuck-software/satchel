//! Moves mail merge jobs forward: one `satchel_core::mail_merge::job::tick`
//! at a time, and a `mail-merge:updated` event whenever a job changed.
//!
//! The runner sleeps until the next job is due, looks every few seconds while
//! a message sits in the outbox, and wakes at once when a command changed a
//! job. Nothing is kept in memory that a restart would lose; at start it
//! pauses whatever the last run left running, so the user decides.

use std::sync::OnceLock;
use std::time::Duration;

use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter};
use tokio::sync::Notify;

use crate::AppState;

/// How often to look while a message is in the outbox. The outbox sends
/// within a second or two; a failure is reported as soon as it settles.
const IN_FLIGHT_POLL: Duration = Duration::from_secs(3);
/// The longest sleep. Catches a job a command created while the runner was
/// between ticks, should the wake-up be missed.
const IDLE_POLL: Duration = Duration::from_secs(60);

fn notify() -> &'static Notify {
    static NOTIFY: OnceLock<Notify> = OnceLock::new();
    NOTIFY.get_or_init(Notify::new)
}

/// Asks the runner to look now: a job was created, resumed or changed.
pub fn wake() {
    notify().notify_one();
}

pub fn spawn_mail_merge_runner(state: AppState, pool: SqlitePool, app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        // A job the last run left running waits for the user to resume it.
        match satchel_core::mail_merge::job::pause_after_restart(&pool, chrono::Utc::now()).await {
            Ok(paused) if !paused.is_empty() => {
                let _ = app.emit(
                    "mail-merge:updated",
                    serde_json::json!({ "job_ids": paused }),
                );
            }
            Ok(_) => {}
            Err(error) => tracing::warn!(error = ?error, "mail merge restart pause failed"),
        }
        loop {
            let sleep =
                match satchel_core::mail_merge::job::tick(&pool, &state.queue, chrono::Utc::now())
                    .await
                {
                    Ok(report) => {
                        if !report.changed_jobs.is_empty() {
                            let _ = app.emit(
                                "mail-merge:updated",
                                serde_json::json!({ "job_ids": report.changed_jobs }),
                            );
                        }
                        let until_due = report.next_due.map(|due| {
                            (due - chrono::Utc::now())
                                .to_std()
                                .unwrap_or(Duration::ZERO)
                                .max(Duration::from_millis(200))
                        });
                        let mut sleep = until_due.unwrap_or(IDLE_POLL).min(IDLE_POLL);
                        if report.in_flight {
                            sleep = sleep.min(IN_FLIGHT_POLL);
                        }
                        sleep
                    }
                    Err(error) => {
                        tracing::warn!(error = ?error, "mail merge tick failed");
                        Duration::from_secs(15)
                    }
                };
            tokio::select! {
                _ = tokio::time::sleep(sleep) => {}
                _ = notify().notified() => {}
            }
        }
    });
}
