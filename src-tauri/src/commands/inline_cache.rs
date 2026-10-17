//! Backfill for inline (`cid:`) images that have no local copy yet.
//!
//! Older messages were synced while the cache write silently failed, so their
//! signature logos are only reachable through a live provider call — one per
//! image, on every open, which is what drove the account into repeated Gmail
//! 429s. Fetching them once, slowly, in the background retires that cost.
//!
//! Deliberately unhurried: the run is throttled well below the provider budget
//! so it never competes with reading mail, it steps aside when the provider
//! signals a rate limit, and it is resumable — the work list is simply every
//! inline image that still has no `cached_path`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter};

use super::attachments::{download_attachment_inner, DownloadAttachmentInput};
use super::error::{CommandError, CommandResult};

/// Pause between attachments. At roughly three per second the run stays an
/// order of magnitude under Gmail's per-user budget, leaving the interactive
/// path unaffected — the point is to finish eventually, not quickly.
const ITEM_DELAY: Duration = Duration::from_millis(300);

/// Longest single pause honoured after a rate limit. Gmail sometimes asks for
/// an hour; waiting that out inside one run is fine, waiting longer is not.
const MAX_RATE_LIMIT_PAUSE: Duration = Duration::from_secs(3600);

/// Fallback when the provider does not say how long it wants to be left alone.
const DEFAULT_RATE_LIMIT_PAUSE: Duration = Duration::from_secs(300);

/// Attempts per attachment before it is counted as failed.
const ATTEMPTS_PER_ITEM: u32 = 3;

/// Consecutive non-rate-limit failures that end the run. Being offline should
/// stop the backfill, not grind through thousands of doomed requests.
const MAX_CONSECUTIVE_FAILURES: i64 = 25;

const PROGRESS_EVENT: &str = "inline-cache:progress";

#[derive(Debug, Clone, Default, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/inline_cache/")
)]
pub struct InlineCacheProgress {
    /// Attachments the run started with.
    pub total: i64,
    pub done: i64,
    pub failed: i64,
    pub running: bool,
    /// Estimated seconds left, once a rate can be measured.
    pub eta_seconds: Option<i64>,
    /// Set while waiting out a provider rate limit, so the UI can say why
    /// nothing is moving.
    pub paused_seconds: Option<i64>,
    pub last_error: Option<String>,
    /// Set once the run ends on its own terms rather than by user request.
    pub finished_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/inline_cache/")
)]
pub struct InlineCacheStatus {
    /// Inline images without a local copy.
    pub missing: i64,
    /// Their combined size as recorded during sync, in bytes.
    pub missing_bytes: i64,
    pub cached: i64,
    /// Floor for a full run: the pacing delay alone, before any transfer time
    /// or rate-limit pause. Enough to name a finishing time before starting.
    pub estimated_seconds: i64,
    pub progress: InlineCacheProgress,
}

static PROGRESS: LazyLock<Mutex<InlineCacheProgress>> =
    LazyLock::new(|| Mutex::new(InlineCacheProgress::default()));
static RUNNING: AtomicBool = AtomicBool::new(false);
static CANCELLED: AtomicBool = AtomicBool::new(false);

fn progress_snapshot() -> InlineCacheProgress {
    PROGRESS
        .lock()
        .map(|guard| guard.clone())
        .unwrap_or_default()
}

fn update_progress(app: &AppHandle, mutate: impl FnOnce(&mut InlineCacheProgress)) {
    let snapshot = {
        let Ok(mut guard) = PROGRESS.lock() else {
            return;
        };
        mutate(&mut guard);
        guard.clone()
    };
    let _ = app.emit(PROGRESS_EVENT, &snapshot);
}

#[tauri::command]
pub async fn inline_image_cache_status(
    pool: tauri::State<'_, crate::db::ReadPool>,
) -> CommandResult<InlineCacheStatus> {
    let (missing, missing_bytes): (i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), COALESCE(SUM(COALESCE(size, 0)), 0)
         FROM attachments
         WHERE is_inline = 1 AND cached_path IS NULL AND content_type LIKE 'image/%'",
    )
    .fetch_one(pool.inner().as_ref())
    .await
    .map_err(|err| err.to_string())?;

    let cached: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM attachments
         WHERE is_inline = 1 AND cached_path IS NOT NULL AND content_type LIKE 'image/%'",
    )
    .fetch_one(pool.inner().as_ref())
    .await
    .map_err(|err| err.to_string())?;

    Ok(InlineCacheStatus {
        missing,
        missing_bytes,
        cached,
        estimated_seconds: (missing as f64 * ITEM_DELAY.as_secs_f64()).round() as i64,
        progress: progress_snapshot(),
    })
}

#[tauri::command]
pub async fn start_inline_image_backfill(
    app: AppHandle,
    pool: tauri::State<'_, SqlitePool>,
) -> CommandResult<()> {
    if RUNNING.swap(true, Ordering::SeqCst) {
        return Err(CommandError::new(
            "backend.errors.inlineBackfillAlreadyRunning",
        ));
    }
    CANCELLED.store(false, Ordering::SeqCst);

    let pool = pool.inner().clone();
    tauri::async_runtime::spawn(async move {
        run_backfill(&app, &pool).await;
        RUNNING.store(false, Ordering::SeqCst);
        update_progress(&app, |progress| progress.running = false);
    });
    Ok(())
}

#[tauri::command]
pub fn stop_inline_image_backfill() {
    CANCELLED.store(true, Ordering::SeqCst);
}

async fn run_backfill(app: &AppHandle, pool: &SqlitePool) {
    // Newest first: the mail most likely to be opened next heals first.
    let ids: Vec<String> = match sqlx::query_scalar(
        "SELECT a.id
         FROM attachments a
         JOIN messages m ON m.id = a.message_id
         WHERE a.is_inline = 1
           AND a.cached_path IS NULL
           AND a.content_type LIKE 'image/%'
         ORDER BY m.date DESC",
    )
    .fetch_all(pool)
    .await
    {
        Ok(ids) => ids,
        Err(err) => {
            update_progress(app, |progress| {
                *progress = InlineCacheProgress {
                    last_error: Some(err.to_string()),
                    finished_reason: Some("error".into()),
                    ..InlineCacheProgress::default()
                };
            });
            return;
        }
    };

    let total = i64::try_from(ids.len()).unwrap_or(i64::MAX);
    update_progress(app, |progress| {
        *progress = InlineCacheProgress {
            total,
            running: true,
            ..InlineCacheProgress::default()
        };
    });

    let started = Instant::now();
    let mut done = 0i64;
    let mut failed = 0i64;
    let mut consecutive_failures = 0i64;
    let mut finished_reason = "completed";

    for id in ids {
        if CANCELLED.load(Ordering::SeqCst) {
            finished_reason = "cancelled";
            break;
        }

        match fetch_one(app, pool, &id).await {
            Ok(()) => {
                done += 1;
                consecutive_failures = 0;
            }
            // Stopping mid-item is not a failure of that item.
            Err(_) if CANCELLED.load(Ordering::SeqCst) => {
                finished_reason = "cancelled";
                break;
            }
            Err(err) => {
                failed += 1;
                consecutive_failures += 1;
                update_progress(app, |progress| progress.last_error = Some(err));
                if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                    finished_reason = "stalled";
                    break;
                }
            }
        }

        // Measured, not assumed: peer-cache hits, rate-limit pauses and slow
        // parts all land in the same average, so the estimate tracks reality
        // instead of a nominal item rate.
        let eta = eta_seconds(started.elapsed(), done + failed, total);
        update_progress(app, |progress| {
            progress.done = done;
            progress.failed = failed;
            progress.eta_seconds = eta;
            progress.paused_seconds = None;
        });

        sleep_interruptible(ITEM_DELAY).await;
    }

    update_progress(app, |progress| {
        progress.done = done;
        progress.failed = failed;
        progress.running = false;
        progress.eta_seconds = None;
        progress.paused_seconds = None;
        progress.finished_reason = Some(finished_reason.to_string());
    });
}

async fn fetch_one(app: &AppHandle, pool: &SqlitePool, attachment_id: &str) -> Result<(), String> {
    let mut last_error = String::new();
    for attempt in 1..=ATTEMPTS_PER_ITEM {
        if CANCELLED.load(Ordering::SeqCst) {
            return Err("cancelled".into());
        }
        // The decoded payload is discarded: the point of the call is the cache
        // file and the `cached_path` it records.
        // Healing old inline images is bulk work: it yields to whatever the
        // user is doing and stops entirely while the provider is throttled.
        let outcome = crate::providers::gmail::as_background(download_attachment_inner(
            app,
            pool,
            DownloadAttachmentInput {
                attachment_id: attachment_id.to_string(),
            },
        ))
        .await;

        match outcome {
            Ok(_) => return Ok(()),
            Err(err) => {
                let pause = rate_limit_pause(&err);
                last_error = err;
                match pause {
                    Some(pause) if attempt < ATTEMPTS_PER_ITEM => {
                        update_progress(app, |progress| {
                            progress.paused_seconds = Some(pause.as_secs() as i64);
                            progress.last_error = Some(last_error.clone());
                        });
                        sleep_interruptible(pause).await;
                        update_progress(app, |progress| progress.paused_seconds = None);
                    }
                    _ => break,
                }
            }
        }
    }
    Err(last_error)
}

/// How long to stand down after a rate-limit error, if that is what it was.
fn rate_limit_pause(error: &str) -> Option<Duration> {
    if !error.to_ascii_lowercase().contains("rate limit") {
        return None;
    }
    let requested = first_seconds_value(error)
        .map(Duration::from_secs)
        .unwrap_or(DEFAULT_RATE_LIMIT_PAUSE);
    Some(requested.min(MAX_RATE_LIMIT_PAUSE))
}

/// First `<digits>s` token in a message — how both the pacer and Gmail phrase
/// the wait ("rate limited for another 812s").
fn first_seconds_value(text: &str) -> Option<u64> {
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if !bytes[index].is_ascii_digit() {
            index += 1;
            continue;
        }
        let start = index;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }
        let followed_by_unit = bytes.get(index) == Some(&b's')
            && !bytes
                .get(index + 1)
                .is_some_and(|byte| byte.is_ascii_alphanumeric());
        if followed_by_unit {
            return text[start..index].parse().ok();
        }
    }
    None
}

fn eta_seconds(elapsed: Duration, handled: i64, total: i64) -> Option<i64> {
    if handled <= 0 || handled >= total {
        return None;
    }
    let per_item = elapsed.as_secs_f64() / handled as f64;
    let remaining = (total - handled) as f64;
    Some((per_item * remaining).round() as i64)
}

/// Sleep in slices so cancelling stays responsive during a long rate-limit wait.
async fn sleep_interruptible(total: Duration) {
    let slice = Duration::from_millis(250);
    let mut waited = Duration::ZERO;
    while waited < total {
        if CANCELLED.load(Ordering::SeqCst) {
            return;
        }
        let step = slice.min(total - waited);
        tokio::time::sleep(step).await;
        waited += step;
    }
}

#[cfg(test)]
mod tests {
    use super::{eta_seconds, first_seconds_value, rate_limit_pause, MAX_RATE_LIMIT_PAUSE};
    use std::time::Duration;

    #[test]
    fn honours_the_wait_the_provider_asked_for() {
        let pause =
            rate_limit_pause("gmail attachment fetch skipped: rate limited for another 812s");

        assert_eq!(pause, Some(Duration::from_secs(812)));
    }

    #[test]
    fn caps_absurd_waits_and_ignores_unrelated_errors() {
        let capped = rate_limit_pause("rate limited; Gmail asks for 99999s");

        assert_eq!(capped, Some(MAX_RATE_LIMIT_PAUSE));
        assert_eq!(rate_limit_pause("attachment not found"), None);
    }

    #[test]
    fn reads_the_seconds_token_not_the_status_code() {
        assert_eq!(
            first_seconds_value("rate limited (429 Too Many Requests); asks for 900s"),
            Some(900)
        );
        assert_eq!(first_seconds_value("no duration here"), None);
    }

    #[test]
    fn estimates_from_the_measured_rate() {
        // 10 of 100 done in 20s → 2s each → 180s left.
        assert_eq!(eta_seconds(Duration::from_secs(20), 10, 100), Some(180));
        assert_eq!(eta_seconds(Duration::from_secs(20), 0, 100), None);
        assert_eq!(eta_seconds(Duration::from_secs(20), 100, 100), None);
    }
}
