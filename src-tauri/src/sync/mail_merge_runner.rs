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

/// First wait between attempts at the restart pause; it doubles up to a minute.
const RESTART_PAUSE_RETRY: Duration = Duration::from_secs(1);

/// Repeats the restart pause until it went through. Nothing may tick before:
/// a job the last run left running would otherwise go on without the user.
async fn retry_until_paused<F, Fut>(pause: F, initial_delay: Duration) -> Vec<String>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = satchel_core::mail_merge::job::MailMergeResult<Vec<String>>>,
{
    let mut pause = pause;
    satchel_core::sync::worker::retry_with_backoff(
        "mail merge restart pause",
        initial_delay,
        Duration::from_secs(60),
        move || {
            let attempt = pause();
            async move { attempt.await.map_err(|error| error.to_string()) }
        },
    )
    .await
}

/// `restart_pause` is the attempt `lib.rs` makes before the outbox worker
/// starts. When it failed, the runner repeats it before its first tick.
pub fn spawn_mail_merge_runner(
    state: AppState,
    pool: SqlitePool,
    app: AppHandle,
    restart_pause: Result<Vec<String>, String>,
) {
    tauri::async_runtime::spawn(async move {
        let paused = match restart_pause {
            Ok(paused) => paused,
            Err(error) => {
                tracing::warn!(%error, "mail merge restart pause failed; retrying before sending");
                retry_until_paused(
                    || {
                        satchel_core::mail_merge::job::pause_after_restart(
                            &pool,
                            chrono::Utc::now(),
                            false,
                        )
                    },
                    RESTART_PAUSE_RETRY,
                )
                .await
            }
        };
        if !paused.is_empty() {
            let _ = app.emit(
                "mail-merge:updated",
                serde_json::json!({ "job_ids": paused }),
            );
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

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use satchel_core::mail_merge::job::MailMergeError;

    use super::*;

    /// A pause that fails once (a locked database at startup) is repeated,
    /// and the runner gets past it only once it went through.
    #[tokio::test]
    async fn repeats_a_failed_restart_pause_until_it_goes_through() {
        let attempts = AtomicUsize::new(0);
        let paused = retry_until_paused(
            || {
                let attempt = attempts.fetch_add(1, Ordering::SeqCst);
                async move {
                    if attempt == 0 {
                        Err(MailMergeError::Persistence(sqlx::Error::PoolTimedOut))
                    } else {
                        Ok(vec!["job-1".to_string()])
                    }
                }
            },
            Duration::from_millis(1),
        )
        .await;
        assert_eq!(paused, vec!["job-1".to_string()]);
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
    }
}
