//! Persisted mail merge jobs and the step that moves them forward.
//!
//! [`tick`] is the whole runner: it settles rows whose outbox action has
//! finished, releases what finished jobs no longer need, then hands the next
//! due row to the outbox. The pace is per account: an account has at most one
//! merge copy in the outbox, and the next one goes `delay_seconds` after the
//! account's latest hand-over in any of its jobs. Everything lives in the
//! database; after a restart running jobs come back paused
//! ([`pause_after_restart`]) and wait for the user.

use std::collections::{BTreeMap, HashSet};

use chrono::{DateTime, Duration, SecondsFormat, Utc};
use serde::Serialize;
use sqlx::{FromRow, SqlitePool};

use super::plan::{render_message, MergeTemplate, MAX_JOB_RECIPIENTS};
use super::template::placeholders;
use crate::drafts::{outgoing::OutgoingMessageInput, send_queue::SendActionPayload};
use crate::sync::queue::{QueuedAction, SyncQueue, ACTION_SEND_MESSAGE};

/// Shortest and longest pause between two messages.
pub const MIN_DELAY_SECONDS: i64 = 1;
pub const MAX_DELAY_SECONDS: i64 = 3600;
/// Failed messages in a row before a job stops by itself. A rejected sender
/// or an exhausted quota fails every message the same way; going on would
/// only mark the account further.
pub const AUTO_PAUSE_FAILURES: i64 = 3;

pub const PAUSE_REASON_FAILURES: &str = "failures";
/// The app quit while the job was running; the user resumes it.
pub const PAUSE_REASON_RESTART: &str = "restart";
/// Finished jobs are removed this long after they finished.
pub const FINISHED_RETENTION_DAYS: i64 = 30;
/// Error key a row carries when its outbox action was removed by hand.
pub const ERROR_REMOVED_FROM_OUTBOX: &str = "backend.errors.mailMergeRemovedFromOutbox";
const ERROR_MISSING_VALUES: &str = "backend.errors.mailMergeMissingValues";

#[derive(Debug, thiserror::Error)]
pub enum MailMergeError {
    #[error("mail merge job not found")]
    NotFound,
    #[error("mail merge job has no recipients")]
    NoRecipients,
    #[error("mail merge job exceeds the recipient limit")]
    TooManyRecipients { limit: usize },
    #[error("mail merge delay out of range")]
    DelayInvalid,
    #[error("mail merge job cannot do that in its current state")]
    InvalidState,
    #[error("a mail merge cannot answer or forward a conversation")]
    ReplyNotAllowed,
    #[error("mail merge queue failed: {0}")]
    Queue(String),
    #[error("mail merge persistence failed")]
    Persistence(#[from] sqlx::Error),
    #[error("mail merge encoding failed")]
    Encoding(#[from] serde_json::Error),
}

pub type MailMergeResult<T> = Result<T, MailMergeError>;

/// A delay that keeps an account clear of its provider's sending limits.
///
/// Gmail allows about 500 messages a day on a personal account and reacts to
/// bursts long before that; Microsoft 365 allows 30 a minute and Outlook.com
/// far fewer. Other servers rarely publish theirs, and small hosters tend to
/// be the strictest.
pub fn default_delay_seconds(provider: &str, email: &str) -> i64 {
    let domain = email
        .rsplit_once('@')
        .map(|(_, domain)| domain.to_ascii_lowercase())
        .unwrap_or_default();
    let gmail = provider == "gmail" || matches!(domain.as_str(), "gmail.com" | "googlemail.com");
    let microsoft = provider == "outlook"
        || ["outlook.", "hotmail.", "live.", "msn."]
            .iter()
            .any(|prefix| domain.starts_with(prefix));
    if gmail {
        15
    } else if microsoft {
        10
    } else {
        20
    }
}

pub struct NewMailMergeJob {
    pub account_id: String,
    /// The composer's send input. Its To/Cc/Bcc are ignored.
    pub message: OutgoingMessageInput,
    pub source_label: String,
    pub delay_seconds: i64,
    /// The client's token for this create. The same token returns the job
    /// it already made instead of a second one.
    pub request_id: Option<String>,
    /// Address and field values per recipient, already checked by
    /// [`super::plan::plan`].
    pub recipients: Vec<(String, BTreeMap<String, String>)>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "core/mail_merge/"))]
pub struct MailMergeJobSummary {
    pub id: String,
    pub account_id: String,
    /// `running`, `paused`, `cancelled` or `completed`.
    pub status: String,
    pub pause_reason: Option<String>,
    pub subject: String,
    pub source_label: String,
    pub delay_seconds: i64,
    pub next_send_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub finished_at: Option<String>,
    pub total: i64,
    pub pending: i64,
    pub queued: i64,
    pub sent: i64,
    pub failed: i64,
    pub unknown: i64,
    pub cancelled: i64,
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "core/mail_merge/"))]
pub struct MailMergeRecipientRow {
    pub id: String,
    pub position: i64,
    pub email: String,
    /// `pending`, `queued`, `sent`, `failed`, `unknown` or `cancelled`.
    pub status: String,
    pub error: Option<String>,
    pub attempts: i64,
    pub sent_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "core/mail_merge/"))]
pub struct MailMergeJobDetail {
    pub job: MailMergeJobSummary,
    pub recipients: Vec<MailMergeRecipientRow>,
}

/// What a [`tick`] did, for events and for when to look again.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TickReport {
    /// Jobs whose rows or status changed.
    pub changed_jobs: Vec<String>,
    /// The earliest moment a waiting job may send again.
    pub next_due: Option<DateTime<Utc>>,
    /// Rows are in the outbox; their outcome is not known yet.
    pub in_flight: bool,
}

/// Fixed-width UTC timestamps, so SQLite can compare them as text.
fn ts(now: DateTime<Utc>) -> String {
    now.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn parse_ts(value: Option<&str>) -> Option<DateTime<Utc>> {
    value
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc))
}

pub fn validate_delay(delay_seconds: i64) -> MailMergeResult<i64> {
    if (MIN_DELAY_SECONDS..=MAX_DELAY_SECONDS).contains(&delay_seconds) {
        Ok(delay_seconds)
    } else {
        Err(MailMergeError::DelayInvalid)
    }
}

/// The subject and body placeholders render into, taken from the send input.
pub fn merge_template(message: &OutgoingMessageInput) -> MergeTemplate {
    MergeTemplate {
        subject: message.subject.clone(),
        body: message.body.clone(),
        body_html: message.body_html.clone(),
    }
}

/// Stores the job and its rows. It starts running at once. A `request_id`
/// seen before returns that job instead of storing a second one.
pub async fn create_job(
    pool: &SqlitePool,
    job: NewMailMergeJob,
    now: DateTime<Utc>,
) -> MailMergeResult<String> {
    if job.recipients.is_empty() {
        return Err(MailMergeError::NoRecipients);
    }
    if job.recipients.len() > MAX_JOB_RECIPIENTS {
        return Err(MailMergeError::TooManyRecipients {
            limit: MAX_JOB_RECIPIENTS,
        });
    }
    let delay = validate_delay(job.delay_seconds)?;
    let mut message = job.message;
    // A copy to everyone on a list is never an answer: In-Reply-To and the
    // quoted conversation would go to people who were not part of it.
    if message.thread_id.is_some() || message.reply_to_message_id.is_some() {
        return Err(MailMergeError::ReplyNotAllowed);
    }
    // Every copy is addressed to one person; no shared header travels along.
    message.to.clear();
    message.cc.clear();
    message.bcc.clear();
    message.draft_id = None;
    message.scheduled_for = None;
    message.follow_up_at = None;
    let template = serde_json::to_string(&message)?;
    let id = uuid::Uuid::new_v4().to_string();
    let now_text = ts(now);
    let request_id = job
        .request_id
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    if let Some(request_id) = request_id.as_deref() {
        let existing: Option<String> =
            sqlx::query_scalar("SELECT id FROM mail_merge_jobs WHERE request_id = ?")
                .bind(request_id)
                .fetch_optional(&mut *tx)
                .await?;
        if let Some(existing) = existing {
            tx.rollback().await?;
            return Ok(existing);
        }
    }
    sqlx::query(
        "INSERT INTO mail_merge_jobs
         (id, account_id, status, subject, source_label, template, delay_seconds, next_send_at,
          request_id, created_at, updated_at)
         VALUES (?, ?, 'running', ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&job.account_id)
    .bind(&message.subject)
    .bind(&job.source_label)
    .bind(&template)
    .bind(delay)
    .bind(&now_text)
    .bind(request_id.as_deref())
    .bind(&now_text)
    .bind(&now_text)
    .execute(&mut *tx)
    .await?;
    for (position, (email, values)) in job.recipients.iter().enumerate() {
        sqlx::query(
            "INSERT INTO mail_merge_recipients
             (id, job_id, position, email, fields, status, message_id_header, updated_at)
             VALUES (?, ?, ?, ?, ?, 'pending', ?, ?)",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(&id)
        .bind(position as i64)
        .bind(email)
        .bind(serde_json::to_string(values)?)
        .bind(format!("<{}@satchel.local>", uuid::Uuid::new_v4()))
        .bind(&now_text)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(id)
}

const SUMMARY_SELECT: &str =
    "SELECT j.id, j.account_id, j.status, j.pause_reason, j.subject, j.source_label,
        j.delay_seconds, j.next_send_at, j.created_at, j.updated_at, j.finished_at,
        COUNT(r.id) AS total,
        COALESCE(SUM(r.status = 'pending'), 0) AS pending,
        COALESCE(SUM(r.status = 'queued'), 0) AS queued,
        COALESCE(SUM(r.status = 'sent'), 0) AS sent,
        COALESCE(SUM(r.status = 'failed'), 0) AS failed,
        COALESCE(SUM(r.status = 'unknown'), 0) AS unknown,
        COALESCE(SUM(r.status = 'cancelled'), 0) AS cancelled
     FROM mail_merge_jobs j LEFT JOIN mail_merge_recipients r ON r.job_id = j.id";

/// Every job, newest first.
pub async fn list_jobs(pool: &SqlitePool) -> MailMergeResult<Vec<MailMergeJobSummary>> {
    Ok(sqlx::query_as::<_, MailMergeJobSummary>(&format!(
        "{SUMMARY_SELECT} GROUP BY j.id ORDER BY j.created_at DESC, j.id"
    ))
    .fetch_all(pool)
    .await?)
}

pub async fn job_summary(pool: &SqlitePool, id: &str) -> MailMergeResult<MailMergeJobSummary> {
    sqlx::query_as::<_, MailMergeJobSummary>(&format!(
        "{SUMMARY_SELECT} WHERE j.id = ? GROUP BY j.id"
    ))
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or(MailMergeError::NotFound)
}

pub async fn job_detail(pool: &SqlitePool, id: &str) -> MailMergeResult<MailMergeJobDetail> {
    let job = job_summary(pool, id).await?;
    let recipients = sqlx::query_as::<_, MailMergeRecipientRow>(
        "SELECT id, position, email, status, error, attempts, sent_at
         FROM mail_merge_recipients WHERE job_id = ? ORDER BY position",
    )
    .bind(id)
    .fetch_all(pool)
    .await?;
    Ok(MailMergeJobDetail { job, recipients })
}

async fn require_changed(pool: &SqlitePool, id: &str, changed: u64) -> MailMergeResult<()> {
    if changed > 0 {
        return Ok(());
    }
    let exists: Option<String> = sqlx::query_scalar("SELECT id FROM mail_merge_jobs WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Err(if exists.is_some() {
        MailMergeError::InvalidState
    } else {
        MailMergeError::NotFound
    })
}

pub async fn pause_job(pool: &SqlitePool, id: &str, now: DateTime<Utc>) -> MailMergeResult<()> {
    let changed = sqlx::query(
        "UPDATE mail_merge_jobs SET status = 'paused', pause_reason = NULL, updated_at = ?
         WHERE id = ? AND status = 'running'",
    )
    .bind(ts(now))
    .bind(id)
    .execute(pool)
    .await?
    .rows_affected();
    require_changed(pool, id, changed).await
}

/// Runs a paused job again. The slot it was waiting for stays: pausing and
/// resuming is no way past the pace. Only after an auto-pause for failures
/// may it go at once; the account's own pace still applies in [`tick`].
pub async fn resume_job(pool: &SqlitePool, id: &str, now: DateTime<Utc>) -> MailMergeResult<()> {
    let now_text = ts(now);
    let changed = sqlx::query(
        "UPDATE mail_merge_jobs
         SET status = 'running', consecutive_failures = 0,
             next_send_at = CASE
                 WHEN pause_reason = ? OR next_send_at IS NULL OR next_send_at < ? THEN ?
                 ELSE next_send_at END,
             pause_reason = NULL, updated_at = ?
         WHERE id = ? AND status = 'paused'",
    )
    .bind(PAUSE_REASON_FAILURES)
    .bind(&now_text)
    .bind(&now_text)
    .bind(&now_text)
    .bind(id)
    .execute(pool)
    .await?
    .rows_affected();
    require_changed(pool, id, changed).await
}

/// After a start, a job the last run left running waits for the user: a
/// list that starts sending on its own when the laptop opens is a surprise.
pub async fn pause_after_restart(
    pool: &SqlitePool,
    now: DateTime<Utc>,
) -> MailMergeResult<Vec<String>> {
    Ok(sqlx::query_scalar(
        "UPDATE mail_merge_jobs SET status = 'paused', pause_reason = ?, updated_at = ?
         WHERE status = 'running' RETURNING id",
    )
    .bind(PAUSE_REASON_RESTART)
    .bind(ts(now))
    .fetch_all(pool)
    .await?)
}

/// Stops the job. Rows not yet handed over are cancelled; one waiting in the
/// outbox is taken back unless the outbox is already sending it.
pub async fn cancel_job(pool: &SqlitePool, id: &str, now: DateTime<Utc>) -> MailMergeResult<()> {
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let changed = sqlx::query(
        "UPDATE mail_merge_jobs SET status = 'cancelled', pause_reason = NULL,
             finished_at = ?, updated_at = ?
         WHERE id = ? AND status IN ('running', 'paused')",
    )
    .bind(ts(now))
    .bind(ts(now))
    .bind(id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if changed == 0 {
        tx.rollback().await?;
        return require_changed(pool, id, 0).await;
    }
    sqlx::query(
        "UPDATE mail_merge_recipients SET status = 'cancelled', updated_at = ?
         WHERE job_id = ? AND status = 'pending'",
    )
    .bind(ts(now))
    .bind(id)
    .execute(&mut *tx)
    .await?;
    let waiting: Vec<(String, String)> = sqlx::query_as(
        "SELECT r.id, r.queue_id FROM mail_merge_recipients r
         JOIN sync_queue q ON q.id = r.queue_id
         WHERE r.job_id = ? AND r.status = 'queued'
           AND q.action_type = ? AND q.status IN ('pending', 'scheduled')",
    )
    .bind(id)
    .bind(ACTION_SEND_MESSAGE)
    .fetch_all(&mut *tx)
    .await?;
    for (row_id, queue_id) in waiting {
        let removed = sqlx::query(
            "DELETE FROM sync_queue WHERE id = ? AND action_type = ? AND status IN ('pending', 'scheduled')",
        )
        .bind(&queue_id)
        .bind(ACTION_SEND_MESSAGE)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if removed > 0 {
            sqlx::query(
                "UPDATE mail_merge_recipients SET status = 'cancelled', updated_at = ? WHERE id = ?",
            )
            .bind(ts(now))
            .bind(&row_id)
            .execute(&mut *tx)
            .await?;
        }
    }
    tx.commit().await?;
    release_finished(pool, now).await?;
    Ok(())
}

/// Puts failed rows back in line and runs the job again. Their old outbox
/// actions go; the rows keep their Message-ID, so a message that did leave
/// after all is recognised instead of sent twice. The slot the job waits for
/// stays, as on a resume.
pub async fn retry_failed(pool: &SqlitePool, id: &str, now: DateTime<Utc>) -> MailMergeResult<u64> {
    let now_text = ts(now);
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let status: Option<String> =
        sqlx::query_scalar("SELECT status FROM mail_merge_jobs WHERE id = ?")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
    match status.as_deref() {
        None => return Err(MailMergeError::NotFound),
        Some("cancelled") => return Err(MailMergeError::InvalidState),
        _ => {}
    }
    sqlx::query(
        "DELETE FROM sync_queue WHERE action_type = ? AND status IN ('failed', 'pending', 'scheduled')
           AND id IN (SELECT queue_id FROM mail_merge_recipients
                      WHERE job_id = ? AND status = 'failed' AND queue_id IS NOT NULL)",
    )
    .bind(ACTION_SEND_MESSAGE)
    .bind(id)
    .execute(&mut *tx)
    .await?;
    let retried = sqlx::query(
        "UPDATE mail_merge_recipients SET status = 'pending', error = NULL, updated_at = ?
         WHERE job_id = ? AND status = 'failed'",
    )
    .bind(&now_text)
    .bind(id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if retried > 0 {
        sqlx::query(
            "UPDATE mail_merge_jobs
             SET status = 'running', pause_reason = NULL, consecutive_failures = 0,
                 finished_at = NULL,
                 next_send_at = CASE WHEN next_send_at IS NULL OR next_send_at < ? THEN ? ELSE next_send_at END,
                 updated_at = ?
             WHERE id = ?",
        )
        .bind(&now_text)
        .bind(&now_text)
        .bind(&now_text)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(retried)
}

/// Removes a finished job and its rows, including the stored message.
pub async fn delete_job(pool: &SqlitePool, id: &str) -> MailMergeResult<()> {
    let changed = sqlx::query(
        "DELETE FROM mail_merge_jobs WHERE id = ? AND status IN ('completed', 'cancelled')
           AND NOT EXISTS (SELECT 1 FROM mail_merge_recipients WHERE job_id = ? AND status = 'queued')",
    )
    .bind(id)
    .bind(id)
    .execute(pool)
    .await?
    .rows_affected();
    require_changed(pool, id, changed).await
}

/// Removes jobs that finished more than [`FINISHED_RETENTION_DAYS`] ago.
/// The maintenance worker calls it; a job with a copy still in the outbox
/// stays until that copy settles.
pub async fn delete_expired_jobs(pool: &SqlitePool, now: DateTime<Utc>) -> MailMergeResult<u64> {
    let cutoff = ts(now - Duration::days(FINISHED_RETENTION_DAYS));
    Ok(sqlx::query(
        "DELETE FROM mail_merge_jobs
         WHERE status IN ('completed', 'cancelled') AND finished_at IS NOT NULL AND finished_at < ?
           AND NOT EXISTS (SELECT 1 FROM mail_merge_recipients r
                           WHERE r.job_id = mail_merge_jobs.id AND r.status = 'queued')",
    )
    .bind(cutoff)
    .execute(pool)
    .await?
    .rows_affected())
}

/// Drops what a finished job no longer needs: the attachments in its stored
/// message, and every field value the message does not use. A job is done
/// with them once it is cancelled, or completed with nothing left to retry
/// or settle; until then a retry must send what the first attempt sent.
pub async fn release_finished(
    pool: &SqlitePool,
    now: DateTime<Utc>,
) -> MailMergeResult<Vec<String>> {
    let finished: Vec<(String, String)> = sqlx::query_as(
        "SELECT j.id, j.template FROM mail_merge_jobs j
         WHERE j.released_at IS NULL AND j.status IN ('completed', 'cancelled')
           AND NOT EXISTS (
               SELECT 1 FROM mail_merge_recipients r WHERE r.job_id = j.id
                 AND (r.status = 'queued'
                      OR (j.status = 'completed' AND r.status IN ('pending', 'failed', 'unknown'))))",
    )
    .fetch_all(pool)
    .await?;
    let mut released = Vec::new();
    for (job_id, template) in finished {
        let mut message: OutgoingMessageInput = serde_json::from_str(&template)?;
        message.attachments.clear();
        let used = used_fields(&merge_template(&message));
        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT id, fields FROM mail_merge_recipients WHERE job_id = ?")
                .bind(&job_id)
                .fetch_all(pool)
                .await?;
        let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
        for (row_id, fields) in rows {
            let values: BTreeMap<String, String> = serde_json::from_str(&fields)?;
            let kept: BTreeMap<String, String> = values
                .into_iter()
                .filter(|(field, _)| used.contains(&field.trim().to_lowercase()))
                .collect();
            sqlx::query("UPDATE mail_merge_recipients SET fields = ? WHERE id = ?")
                .bind(serde_json::to_string(&kept)?)
                .bind(&row_id)
                .execute(&mut *tx)
                .await?;
        }
        sqlx::query(
            "UPDATE mail_merge_jobs SET template = ?, released_at = ?, updated_at = ?
             WHERE id = ? AND released_at IS NULL",
        )
        .bind(serde_json::to_string(&message)?)
        .bind(ts(now))
        .bind(ts(now))
        .bind(&job_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        released.push(job_id);
    }
    Ok(released)
}

/// The field names the message's placeholders name, lower-cased.
fn used_fields(template: &MergeTemplate) -> HashSet<String> {
    [
        template.subject.as_str(),
        template.body.as_str(),
        template.body_html.as_deref().unwrap_or(""),
    ]
    .into_iter()
    .flat_map(placeholders)
    .map(|placeholder| placeholder.name.trim().to_lowercase())
    .collect()
}

#[derive(FromRow)]
struct OpenRow {
    id: String,
    job_id: String,
    status: String,
    queue_id: Option<String>,
    message_id_header: String,
}

#[derive(FromRow)]
struct QueueState {
    status: String,
    last_error: Option<String>,
}

enum Settled {
    Sent,
    Failed(Option<String>),
    Unknown(Option<String>),
    Removed,
}

/// What the outbox did with a row's copy, or `None` while it is still on its
/// way (or, for an unconfirmed row, still unconfirmed).
async fn settle(pool: &SqlitePool, row: &OpenRow) -> MailMergeResult<Option<Settled>> {
    let state = match row.queue_id.as_deref() {
        Some(queue_id) => {
            sqlx::query_as::<_, QueueState>(
                "SELECT status, last_error FROM sync_queue WHERE id = ?",
            )
            .bind(queue_id)
            .fetch_optional(pool)
            .await?
        }
        None => None,
    };
    Ok(match state {
        Some(state) => match state.status.as_str() {
            "failed" => Some(Settled::Failed(state.last_error)),
            // Sent; only the local copy is missing. The outbox shows that.
            "delivered" => Some(Settled::Sent),
            "delivery_unknown" if row.status == "queued" => {
                Some(Settled::Unknown(state.last_error))
            }
            _ => None,
        },
        None => {
            let sent: Option<String> = sqlx::query_scalar(
                "SELECT id FROM messages WHERE message_id_header = ? AND is_sent = 1 LIMIT 1",
            )
            .bind(&row.message_id_header)
            .fetch_optional(pool)
            .await?;
            Some(if sent.is_some() {
                Settled::Sent
            } else {
                // Discarded in the outbox, or taken back with Undo.
                Settled::Removed
            })
        }
    })
}

fn push_changed(report: &mut TickReport, job_id: &str) {
    if !report.changed_jobs.iter().any(|id| id == job_id) {
        report.changed_jobs.push(job_id.to_string());
    }
}

fn earliest(current: Option<DateTime<Utc>>, candidate: DateTime<Utc>) -> Option<DateTime<Utc>> {
    Some(current.map_or(candidate, |current| current.min(candidate)))
}

/// Counts a copy that did not go out (failed, or delivery unconfirmed) and
/// pauses the job once [`AUTO_PAUSE_FAILURES`] came in a row.
async fn count_failure(pool: &SqlitePool, job_id: &str, now_text: &str) -> MailMergeResult<()> {
    sqlx::query(
        "UPDATE mail_merge_jobs SET consecutive_failures = consecutive_failures + 1, updated_at = ? WHERE id = ?",
    )
    .bind(now_text)
    .bind(job_id)
    .execute(pool)
    .await?;
    sqlx::query(
        "UPDATE mail_merge_jobs SET status = 'paused', pause_reason = ?, updated_at = ?
         WHERE id = ? AND status = 'running' AND consecutive_failures >= ?",
    )
    .bind(PAUSE_REASON_FAILURES)
    .bind(now_text)
    .bind(job_id)
    .bind(AUTO_PAUSE_FAILURES)
    .execute(pool)
    .await?;
    Ok(())
}

/// Settles open rows, releases finished jobs, then hands the next due row of
/// each account to the outbox. Safe to call at any time and as often as
/// needed.
pub async fn tick(
    pool: &SqlitePool,
    queue: &SyncQueue,
    now: DateTime<Utc>,
) -> MailMergeResult<TickReport> {
    let mut report = TickReport::default();
    let now_text = ts(now);

    // A copy the user retried from the Outbox is on its way again.
    let revived: Vec<String> = sqlx::query_scalar(
        "UPDATE mail_merge_recipients SET status = 'queued', error = NULL, updated_at = ?
         WHERE status IN ('failed', 'unknown') AND queue_id IN (
             SELECT id FROM sync_queue WHERE action_type = ? AND status IN ('pending', 'scheduled', 'sending'))
         RETURNING job_id",
    )
    .bind(&now_text)
    .bind(ACTION_SEND_MESSAGE)
    .fetch_all(pool)
    .await?;
    for job_id in revived {
        push_changed(&mut report, &job_id);
    }

    // Copies on their way, and unconfirmed ones the Outbox may since have
    // resolved (sent after all, failed for good, or discarded).
    let open = sqlx::query_as::<_, OpenRow>(
        "SELECT id, job_id, status, queue_id, message_id_header FROM mail_merge_recipients
         WHERE status IN ('queued', 'unknown')",
    )
    .fetch_all(pool)
    .await?;
    for row in open {
        let Some(outcome) = settle(pool, &row).await? else {
            if row.status == "queued" {
                report.in_flight = true;
            }
            continue;
        };
        let first_outcome = row.status == "queued";
        let (status, error, failure) = match outcome {
            Settled::Sent => ("sent", None, Some(false)),
            Settled::Failed(error) => ("failed", error, Some(true)),
            Settled::Unknown(error) => ("unknown", error, Some(true)),
            Settled::Removed => (
                "cancelled",
                Some(ERROR_REMOVED_FROM_OUTBOX.to_string()),
                None,
            ),
        };
        sqlx::query(
            "UPDATE mail_merge_recipients
             SET status = ?, error = ?, sent_at = CASE WHEN ? = 'sent' THEN ? ELSE sent_at END, updated_at = ?
             WHERE id = ? AND status = ?",
        )
        .bind(status)
        .bind(error)
        .bind(status)
        .bind(&now_text)
        .bind(&now_text)
        .bind(&row.id)
        .bind(&row.status)
        .execute(pool)
        .await?;
        // Only the first outcome of a hand-over counts toward the streak; an
        // unconfirmed copy that later fails was counted when it went unconfirmed.
        if first_outcome {
            match failure {
                Some(true) => count_failure(pool, &row.job_id, &now_text).await?,
                Some(false) => {
                    sqlx::query(
                        "UPDATE mail_merge_jobs SET consecutive_failures = 0, updated_at = ? WHERE id = ?",
                    )
                    .bind(&now_text)
                    .bind(&row.job_id)
                    .execute(pool)
                    .await?;
                }
                None => {}
            }
        }
        push_changed(&mut report, &row.job_id);
    }

    let running: Vec<(String, String, String, i64, Option<String>)> = sqlx::query_as(
        "SELECT id, account_id, template, delay_seconds, next_send_at FROM mail_merge_jobs
         WHERE status = 'running' ORDER BY created_at, rowid",
    )
    .fetch_all(pool)
    .await?;
    // Accounts that already have a merge copy in the outbox, this tick included.
    let mut busy_accounts: HashSet<String> = sqlx::query_scalar(
        "SELECT DISTINCT j.account_id FROM mail_merge_recipients r
         JOIN mail_merge_jobs j ON j.id = r.job_id WHERE r.status = 'queued'",
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .collect();
    for (job_id, account_id, template, delay, next_send_at) in running {
        let next: Option<(String, String, String, String)> = sqlx::query_as(
            "SELECT id, email, fields, message_id_header FROM mail_merge_recipients
             WHERE job_id = ? AND status = 'pending' ORDER BY position LIMIT 1",
        )
        .bind(&job_id)
        .fetch_optional(pool)
        .await?;
        let Some((row_id, email, fields, message_id_header)) = next else {
            let own_queued: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM mail_merge_recipients WHERE job_id = ? AND status = 'queued'",
            )
            .bind(&job_id)
            .fetch_one(pool)
            .await?;
            if own_queued > 0 {
                report.in_flight = true;
                continue;
            }
            sqlx::query(
                "UPDATE mail_merge_jobs SET status = 'completed', finished_at = ?, updated_at = ?
                 WHERE id = ? AND status = 'running'",
            )
            .bind(&now_text)
            .bind(&now_text)
            .bind(&job_id)
            .execute(pool)
            .await?;
            push_changed(&mut report, &job_id);
            continue;
        };
        if busy_accounts.contains(&account_id) {
            report.in_flight = true;
            continue;
        }
        // The account's pace: `delay` after its latest hand-over in any job.
        let last_hand_over: Option<String> = sqlx::query_scalar(
            "SELECT MAX(r.handed_over_at) FROM mail_merge_recipients r
             JOIN mail_merge_jobs j ON j.id = r.job_id WHERE j.account_id = ?",
        )
        .bind(&account_id)
        .fetch_one(pool)
        .await?;
        let paced = parse_ts(last_hand_over.as_deref()).map(|at| at + Duration::seconds(delay));
        let due = [parse_ts(next_send_at.as_deref()), paced]
            .into_iter()
            .flatten()
            .max();
        if let Some(due) = due.filter(|due| *due > now) {
            report.next_due = earliest(report.next_due, due);
            continue;
        }

        let mut message: OutgoingMessageInput = serde_json::from_str(&template)?;
        let values: BTreeMap<String, String> = serde_json::from_str(&fields)?;
        let rendered = match render_message(&merge_template(&message), &values) {
            Ok(rendered) => rendered,
            Err(_) => {
                // The plan checked every row before the job was stored; only
                // a hand-edited row lands here. It must not go out half-filled.
                sqlx::query(
                    "UPDATE mail_merge_recipients SET status = 'failed', error = ?, updated_at = ? WHERE id = ?",
                )
                .bind(ERROR_MISSING_VALUES)
                .bind(&now_text)
                .bind(&row_id)
                .execute(pool)
                .await?;
                push_changed(&mut report, &job_id);
                continue;
            }
        };
        message.to = vec![email];
        message.subject = rendered.subject;
        message.body = rendered.body;
        message.body_html = rendered.body_html;
        let payload = SendActionPayload {
            account_id: account_id.clone(),
            draft_reference: None,
            input: message,
            scheduled_for: None,
            message_id_header: Some(message_id_header),
        };
        let mut action = QueuedAction::new(
            account_id.clone(),
            ACTION_SEND_MESSAGE,
            serde_json::to_value(&payload)?,
        );
        action.id = format!("mail-merge-{row_id}");

        let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
        let next_at = now + Duration::seconds(delay);
        // Paused or cancelled since the row was picked: hand nothing over.
        let still_running = sqlx::query(
            "UPDATE mail_merge_jobs SET next_send_at = ?, updated_at = ? WHERE id = ? AND status = 'running'",
        )
        .bind(ts(next_at))
        .bind(&now_text)
        .bind(&job_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        let claimed = sqlx::query(
            "UPDATE mail_merge_recipients
             SET status = 'queued', queue_id = ?, attempts = attempts + 1,
                 handed_over_at = ?, updated_at = ?
             WHERE id = ? AND status = 'pending'",
        )
        .bind(&action.id)
        .bind(&now_text)
        .bind(&now_text)
        .bind(&row_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if still_running == 0 || claimed == 0 {
            tx.rollback().await?;
            continue;
        }
        queue
            .enqueue_in_transaction(&mut tx, &action)
            .await
            .map_err(MailMergeError::Queue)?;
        tx.commit().await?;
        queue.wake_worker();
        busy_accounts.insert(account_id);
        report.in_flight = true;
        push_changed(&mut report, &job_id);
    }

    for job_id in release_finished(pool, now).await? {
        push_changed(&mut report, &job_id);
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use sqlx::sqlite::SqlitePoolOptions;

    /// Whole seconds: stored timestamps keep milliseconds.
    fn start() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 6, 12, 0, 0).unwrap()
    }

    async fn pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("pool");
        crate::db::init(&pool).await.expect("migrations");
        sqlx::query(
            "INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
             VALUES ('acc', 'Me', 'me@example.test', 'imap', '{}', '', '', ''),
                    ('other', 'Other', 'other@example.test', 'imap', '{}', '', '', '');
             INSERT INTO folders (id, account_id, name, full_path, remote_id, folder_type, created_at, updated_at)
             VALUES ('sent', 'acc', 'Sent', 'Sent', 'Sent', 'sent', '', '')",
        )
        .execute(&pool)
        .await
        .expect("account");
        pool
    }

    fn message() -> OutgoingMessageInput {
        serde_json::from_value(serde_json::json!({
            "to": ["shared@example.test"],
            "cc": ["cc@example.test"],
            "bcc": ["bcc@example.test"],
            "subject": "Hello {{FirstName|there}}",
            "body": "Dear {{FirstName|friend}},",
            "body_html": "<p>Dear {{FirstName|friend}},</p>",
            "attachments": [{ "filename": "a.txt", "mime_type": "text/plain", "content_base64": "YQ==" }],
            "draft_id": "draft-1",
        }))
        .expect("input")
    }

    fn recipient(email: &str, first: &str) -> (String, BTreeMap<String, String>) {
        (
            email.to_string(),
            BTreeMap::from([
                ("Email".to_string(), email.to_string()),
                ("FirstName".to_string(), first.to_string()),
                ("Unused".to_string(), "private note".to_string()),
            ]),
        )
    }

    fn new_job_input(account: &str, prefix: &str, count: usize) -> NewMailMergeJob {
        NewMailMergeJob {
            account_id: account.into(),
            message: message(),
            source_label: "list.csv".into(),
            delay_seconds: 10,
            request_id: None,
            recipients: (0..count)
                .map(|index| {
                    recipient(
                        &format!("{prefix}{index}@example.test"),
                        &format!("P{index}"),
                    )
                })
                .collect(),
        }
    }

    async fn new_job(pool: &SqlitePool, now: DateTime<Utc>, count: usize) -> String {
        create_job(pool, new_job_input("acc", "p", count), now)
            .await
            .expect("job")
    }

    async fn queue_rows(pool: &SqlitePool) -> Vec<(String, String, String)> {
        sqlx::query_as("SELECT id, status, payload FROM sync_queue ORDER BY created_at, id")
            .fetch_all(pool)
            .await
            .expect("queue")
    }

    /// The outbox worker's two ways of finishing: delete on success, keep the
    /// row as `failed` otherwise. A sent copy lands in `messages`.
    async fn outbox_sends(pool: &SqlitePool, queue_id: &str) {
        let payload: String = sqlx::query_scalar("SELECT payload FROM sync_queue WHERE id = ?")
            .bind(queue_id)
            .fetch_one(pool)
            .await
            .expect("payload");
        let payload: serde_json::Value = serde_json::from_str(&payload).expect("json");
        sqlx::query(
            "INSERT INTO messages (id, account_id, folder_id, remote_id, from_address, to_addresses, subject, date, created_at, updated_at, is_sent, message_id_header)
             VALUES (?, 'acc', 'sent', '', 'me@example.test', '[]', '', '', '', '', 1, ?)",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(payload["message_id_header"].as_str())
        .execute(pool)
        .await
        .expect("sent copy");
        sqlx::query("DELETE FROM sync_queue WHERE id = ?")
            .bind(queue_id)
            .execute(pool)
            .await
            .expect("complete");
    }

    async fn outbox_marks(pool: &SqlitePool, queue_id: &str, status: &str) {
        sqlx::query("UPDATE sync_queue SET status = ?, last_error = 'backend.errors.smtpRejected' WHERE id = ?")
            .bind(status)
            .bind(queue_id)
            .execute(pool)
            .await
            .expect("mark");
    }

    async fn sole_queue_id(pool: &SqlitePool) -> String {
        let rows = queue_rows(pool).await;
        assert_eq!(rows.len(), 1, "one copy in the outbox");
        rows[0].0.clone()
    }

    #[tokio::test]
    async fn sends_one_personal_copy_at_a_time_with_the_delay_between() {
        let pool = pool().await;
        let queue = SyncQueue::new();
        let start = start();
        let job = new_job(&pool, start, 2).await;

        let report = tick(&pool, &queue, start).await.expect("tick");
        assert_eq!(report.changed_jobs, vec![job.clone()]);
        let rows = queue_rows(&pool).await;
        assert_eq!(rows.len(), 1);
        let payload: serde_json::Value = serde_json::from_str(&rows[0].2).expect("json");
        let input = &payload["input"];
        assert_eq!(input["to"], serde_json::json!(["p0@example.test"]));
        assert_eq!(input["cc"], serde_json::json!([]));
        assert_eq!(input["bcc"], serde_json::json!([]));
        assert_eq!(input["subject"], "Hello P0");
        assert_eq!(input["body_html"], "<p>Dear P0,</p>");
        assert_eq!(input["draft_id"], serde_json::Value::Null);
        assert_eq!(input["attachments"][0]["filename"], "a.txt");
        assert_eq!(rows[0].1, "pending");

        // Still in the outbox: nothing else goes.
        tick(&pool, &queue, start + Duration::seconds(60))
            .await
            .expect("tick");
        assert_eq!(queue_rows(&pool).await.len(), 1);

        outbox_sends(&pool, &rows[0].0).await;
        // Sent, but the delay since the hand-over has not passed yet.
        let report = tick(&pool, &queue, start + Duration::seconds(5))
            .await
            .expect("tick");
        assert!(queue_rows(&pool).await.is_empty());
        assert_eq!(report.next_due, Some(start + Duration::seconds(10)));

        tick(&pool, &queue, start + Duration::seconds(10))
            .await
            .expect("tick");
        let rows = queue_rows(&pool).await;
        assert_eq!(rows.len(), 1);
        assert!(rows[0].2.contains("p1@example.test"));
        outbox_sends(&pool, &rows[0].0).await;
        tick(&pool, &queue, start + Duration::seconds(11))
            .await
            .expect("tick");

        let detail = job_detail(&pool, &job).await.expect("detail");
        assert_eq!(detail.job.status, "completed");
        assert_eq!(detail.job.sent, 2);
        assert!(detail.recipients.iter().all(|row| row.sent_at.is_some()));
    }

    /// After a start, a job the last run left running waits for the user;
    /// resuming it continues without sending anything twice.
    #[tokio::test]
    async fn comes_back_paused_after_a_restart_and_resumes_without_resending() {
        let pool = pool().await;
        let start = start();
        let job = new_job(&pool, start, 3).await;
        tick(&pool, &SyncQueue::new(), start).await.expect("tick");
        let first = sole_queue_id(&pool).await;
        outbox_sends(&pool, &first).await;

        // "Restart": the runner pauses what was running, with a fresh queue.
        let queue = SyncQueue::new();
        let paused = pause_after_restart(&pool, start + Duration::seconds(30))
            .await
            .expect("restart");
        assert_eq!(paused, vec![job.clone()]);
        tick(&pool, &queue, start + Duration::seconds(30))
            .await
            .expect("tick");
        assert!(queue_rows(&pool).await.is_empty());
        let summary = job_summary(&pool, &job).await.expect("job");
        assert_eq!(summary.status, "paused");
        assert_eq!(summary.pause_reason.as_deref(), Some(PAUSE_REASON_RESTART));

        resume_job(&pool, &job, start + Duration::seconds(40))
            .await
            .expect("resume");
        tick(&pool, &queue, start + Duration::seconds(40))
            .await
            .expect("tick");
        let rows = queue_rows(&pool).await;
        assert_eq!(rows.len(), 1);
        assert!(rows[0].2.contains("p1@example.test"));
        let detail = job_detail(&pool, &job).await.expect("detail");
        assert_eq!(
            (detail.job.sent, detail.job.queued, detail.job.pending),
            (1, 1, 1)
        );
    }

    /// Pausing and resuming is no way past the pace.
    #[tokio::test]
    async fn resume_keeps_the_slot_the_job_was_waiting_for() {
        let pool = pool().await;
        let queue = SyncQueue::new();
        let start = start();
        let job = new_job(&pool, start, 3).await;
        tick(&pool, &queue, start).await.expect("tick");
        outbox_sends(&pool, &sole_queue_id(&pool).await).await;

        pause_job(&pool, &job, start + Duration::seconds(2))
            .await
            .expect("pause");
        resume_job(&pool, &job, start + Duration::seconds(3))
            .await
            .expect("resume");
        let summary = job_summary(&pool, &job).await.expect("job");
        assert_eq!(
            parse_ts(summary.next_send_at.as_deref()),
            Some(start + Duration::seconds(10))
        );
        let report = tick(&pool, &queue, start + Duration::seconds(3))
            .await
            .expect("tick");
        assert!(queue_rows(&pool).await.is_empty());
        assert_eq!(report.next_due, Some(start + Duration::seconds(10)));

        // Retrying failed copies keeps the slot as well.
        tick(&pool, &queue, start + Duration::seconds(10))
            .await
            .expect("tick");
        outbox_marks(&pool, &sole_queue_id(&pool).await, "failed").await;
        tick(&pool, &queue, start + Duration::seconds(11))
            .await
            .expect("tick");
        assert_eq!(
            retry_failed(&pool, &job, start + Duration::seconds(12))
                .await
                .expect("retry"),
            1
        );
        let report = tick(&pool, &queue, start + Duration::seconds(12))
            .await
            .expect("tick");
        assert!(queue_rows(&pool).await.is_empty());
        assert_eq!(report.next_due, Some(start + Duration::seconds(20)));
    }

    /// Two jobs on one account share its pace instead of doubling it.
    #[tokio::test]
    async fn two_jobs_on_one_account_share_the_pace() {
        let pool = pool().await;
        let queue = SyncQueue::new();
        let start = start();
        let first = create_job(&pool, new_job_input("acc", "a", 2), start)
            .await
            .expect("first");
        let second = create_job(&pool, new_job_input("acc", "b", 2), start)
            .await
            .expect("second");
        let elsewhere = create_job(&pool, new_job_input("other", "c", 2), start)
            .await
            .expect("other account");

        tick(&pool, &queue, start).await.expect("tick");
        let rows = queue_rows(&pool).await;
        // One copy for `acc` (the older job), one for the other account.
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().any(|row| row.2.contains("a0@example.test")));
        assert!(rows.iter().any(|row| row.2.contains("c0@example.test")));
        for row in &rows {
            outbox_sends(&pool, &row.0).await;
        }

        // The second job's first copy waits for the account's slot.
        let report = tick(&pool, &queue, start + Duration::seconds(5))
            .await
            .expect("tick");
        assert!(queue_rows(&pool).await.is_empty());
        assert_eq!(report.next_due, Some(start + Duration::seconds(10)));

        tick(&pool, &queue, start + Duration::seconds(10))
            .await
            .expect("tick");
        let rows = queue_rows(&pool).await;
        let for_acc: Vec<_> = rows
            .iter()
            .filter(|row| !row.2.contains("c1@example.test"))
            .collect();
        assert_eq!(for_acc.len(), 1, "still one copy per account");
        let _ = (first, second, elsewhere);
    }

    #[tokio::test]
    async fn pause_resume_and_cancel() {
        let pool = pool().await;
        let queue = SyncQueue::new();
        let start = start();
        let job = new_job(&pool, start, 3).await;

        pause_job(&pool, &job, start).await.expect("pause");
        tick(&pool, &queue, start).await.expect("tick");
        assert!(queue_rows(&pool).await.is_empty());
        assert!(matches!(
            pause_job(&pool, &job, start).await,
            Err(MailMergeError::InvalidState)
        ));

        resume_job(&pool, &job, start).await.expect("resume");
        tick(&pool, &queue, start).await.expect("tick");
        assert_eq!(queue_rows(&pool).await.len(), 1);

        // Cancelling takes the waiting message back out of the outbox.
        cancel_job(&pool, &job, start).await.expect("cancel");
        assert!(queue_rows(&pool).await.is_empty());
        let detail = job_detail(&pool, &job).await.expect("detail");
        assert_eq!(detail.job.status, "cancelled");
        assert_eq!(detail.job.cancelled, 3);
        tick(&pool, &queue, start + Duration::seconds(60))
            .await
            .expect("tick");
        assert!(queue_rows(&pool).await.is_empty());
        delete_job(&pool, &job).await.expect("delete");
        assert!(matches!(
            job_detail(&pool, &job).await,
            Err(MailMergeError::NotFound)
        ));
    }

    #[tokio::test]
    async fn pauses_itself_after_repeated_failures_and_retries_them() {
        let pool = pool().await;
        let queue = SyncQueue::new();
        let mut now = start();
        let job = new_job(&pool, now, 5).await;
        for _ in 0..AUTO_PAUSE_FAILURES {
            tick(&pool, &queue, now).await.expect("tick");
            let id = queue_rows(&pool).await.last().expect("row").0.clone();
            outbox_marks(&pool, &id, "failed").await;
            now += Duration::seconds(10);
        }
        tick(&pool, &queue, now).await.expect("tick");
        let detail = job_detail(&pool, &job).await.expect("detail");
        assert_eq!(detail.job.status, "paused");
        assert_eq!(
            detail.job.pause_reason.as_deref(),
            Some(PAUSE_REASON_FAILURES)
        );
        assert_eq!(detail.job.failed, 3);
        assert_eq!(
            detail.recipients[0].error.as_deref(),
            Some("backend.errors.smtpRejected")
        );

        let retried = retry_failed(&pool, &job, now).await.expect("retry");
        assert_eq!(retried, 3);
        // The failed outbox actions are gone; the first failed row goes again,
        // with the Message-ID it had.
        assert!(queue_rows(&pool).await.is_empty());
        let old_mid: String = sqlx::query_scalar(
            "SELECT message_id_header FROM mail_merge_recipients WHERE job_id = ? AND position = 0",
        )
        .bind(&job)
        .fetch_one(&pool)
        .await
        .expect("mid");
        tick(&pool, &queue, now).await.expect("tick");
        let rows = queue_rows(&pool).await;
        assert_eq!(rows.len(), 1);
        assert!(rows[0].2.contains("p0@example.test"));
        assert!(rows[0].2.contains(&old_mid));
        assert_eq!(
            job_summary(&pool, &job).await.expect("job").status,
            "running"
        );
    }

    /// An unconfirmed delivery counts toward the auto-pause, and its row
    /// follows the Outbox when the user resolves it there.
    #[tokio::test]
    async fn unconfirmed_copies_count_and_settle_later() {
        let pool = pool().await;
        let queue = SyncQueue::new();
        let mut now = start();
        let job = new_job(&pool, now, 5).await;
        let mut unknown_ids = Vec::new();
        for _ in 0..AUTO_PAUSE_FAILURES {
            tick(&pool, &queue, now).await.expect("tick");
            let id = queue_rows(&pool)
                .await
                .into_iter()
                .find(|row| row.1 == "pending")
                .expect("row")
                .0;
            outbox_marks(&pool, &id, "delivery_unknown").await;
            unknown_ids.push(id);
            now += Duration::seconds(10);
        }
        tick(&pool, &queue, now).await.expect("tick");
        let summary = job_summary(&pool, &job).await.expect("job");
        assert_eq!(summary.unknown, 3);
        assert_eq!(summary.status, "paused");
        assert_eq!(summary.pause_reason.as_deref(), Some(PAUSE_REASON_FAILURES));

        // Resolved in the Outbox: one turned out sent, one failed, one was
        // discarded. A later failure is not counted twice.
        outbox_sends(&pool, &unknown_ids[0]).await;
        outbox_marks(&pool, &unknown_ids[1], "failed").await;
        sqlx::query("DELETE FROM sync_queue WHERE id = ?")
            .bind(&unknown_ids[2])
            .execute(&pool)
            .await
            .expect("discard");
        tick(&pool, &queue, now).await.expect("tick");
        let detail = job_detail(&pool, &job).await.expect("detail");
        let statuses: Vec<&str> = detail.recipients[..3]
            .iter()
            .map(|row| row.status.as_str())
            .collect();
        assert_eq!(statuses, ["sent", "failed", "cancelled"]);
        let streak: i64 =
            sqlx::query_scalar("SELECT consecutive_failures FROM mail_merge_jobs WHERE id = ?")
                .bind(&job)
                .fetch_one(&pool)
                .await
                .expect("streak");
        assert_eq!(streak, 3);
    }

    #[tokio::test]
    async fn a_message_removed_from_the_outbox_counts_as_cancelled() {
        let pool = pool().await;
        let queue = SyncQueue::new();
        let now = start();
        let job = new_job(&pool, now, 1).await;
        tick(&pool, &queue, now).await.expect("tick");
        sqlx::query("DELETE FROM sync_queue")
            .execute(&pool)
            .await
            .expect("discard");
        tick(&pool, &queue, now).await.expect("tick");
        let detail = job_detail(&pool, &job).await.expect("detail");
        assert_eq!(detail.recipients[0].status, "cancelled");
        assert_eq!(
            detail.recipients[0].error.as_deref(),
            Some(ERROR_REMOVED_FROM_OUTBOX)
        );
        assert_eq!(detail.job.status, "completed");
    }

    async fn stored(pool: &SqlitePool, job: &str) -> (serde_json::Value, Vec<String>) {
        let template: String =
            sqlx::query_scalar("SELECT template FROM mail_merge_jobs WHERE id = ?")
                .bind(job)
                .fetch_one(pool)
                .await
                .expect("template");
        let fields: Vec<String> = sqlx::query_scalar(
            "SELECT fields FROM mail_merge_recipients WHERE job_id = ? ORDER BY position",
        )
        .bind(job)
        .fetch_all(pool)
        .await
        .expect("fields");
        (serde_json::from_str(&template).expect("json"), fields)
    }

    /// A finished job keeps neither the attachments nor field values the
    /// message never used; one with failures keeps them until retried.
    #[tokio::test]
    async fn finished_jobs_drop_attachments_and_unused_values() {
        let pool = pool().await;
        let queue = SyncQueue::new();
        let now = start();
        let job = new_job(&pool, now, 1).await;
        tick(&pool, &queue, now).await.expect("tick");
        outbox_marks(&pool, &sole_queue_id(&pool).await, "failed").await;
        tick(&pool, &queue, now + Duration::seconds(1))
            .await
            .expect("tick");
        assert_eq!(job_summary(&pool, &job).await.unwrap().status, "completed");
        let (template, fields) = stored(&pool, &job).await;
        assert_eq!(template["attachments"].as_array().map(Vec::len), Some(1));
        assert!(fields[0].contains("private note"));

        retry_failed(&pool, &job, now + Duration::seconds(20))
            .await
            .expect("retry");
        tick(&pool, &queue, now + Duration::seconds(20))
            .await
            .expect("tick");
        outbox_sends(&pool, &sole_queue_id(&pool).await).await;
        tick(&pool, &queue, now + Duration::seconds(21))
            .await
            .expect("tick");
        let (template, fields) = stored(&pool, &job).await;
        assert_eq!(template["attachments"], serde_json::json!([]));
        assert_eq!(fields[0], r#"{"FirstName":"P0"}"#);

        // A cancelled job lets go at once.
        let cancelled = new_job(&pool, now, 2).await;
        cancel_job(&pool, &cancelled, now).await.expect("cancel");
        let (template, _) = stored(&pool, &cancelled).await;
        assert_eq!(template["attachments"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn finished_jobs_go_after_thirty_days() {
        let pool = pool().await;
        let now = start();
        let old = new_job(&pool, now, 1).await;
        cancel_job(&pool, &old, now).await.expect("cancel");
        let running = new_job(&pool, now, 1).await;
        let later = now + Duration::days(FINISHED_RETENTION_DAYS);
        assert_eq!(delete_expired_jobs(&pool, later).await.expect("purge"), 0);
        assert_eq!(
            delete_expired_jobs(&pool, later + Duration::seconds(1))
                .await
                .expect("purge"),
            1
        );
        assert!(matches!(
            job_summary(&pool, &old).await,
            Err(MailMergeError::NotFound)
        ));
        assert!(job_summary(&pool, &running).await.is_ok());
    }

    #[tokio::test]
    async fn a_repeated_create_returns_the_job_it_made() {
        let pool = pool().await;
        let now = start();
        let mut input = new_job_input("acc", "p", 2);
        input.request_id = Some("req-1".into());
        let first = create_job(&pool, input, now).await.expect("first");
        let mut again = new_job_input("acc", "p", 2);
        again.request_id = Some("req-1".into());
        let second = create_job(&pool, again, now).await.expect("again");
        assert_eq!(first, second);
        assert_eq!(list_jobs(&pool).await.expect("jobs").len(), 1);
    }

    #[tokio::test]
    async fn refuses_replies_empty_and_oversized_jobs_and_bad_delays() {
        let pool = pool().await;
        let now = start();
        let build = |count: usize, delay: i64| NewMailMergeJob {
            delay_seconds: delay,
            ..new_job_input("acc", "p", count)
        };
        assert!(matches!(
            create_job(&pool, build(0, 10), now).await,
            Err(MailMergeError::NoRecipients)
        ));
        assert!(matches!(
            create_job(&pool, build(MAX_JOB_RECIPIENTS + 1, 10), now).await,
            Err(MailMergeError::TooManyRecipients {
                limit: MAX_JOB_RECIPIENTS
            })
        ));
        assert!(matches!(
            create_job(&pool, build(1, 0), now).await,
            Err(MailMergeError::DelayInvalid)
        ));
        for (thread, reply) in [(Some("th-1"), None), (None, Some("msg-1"))] {
            let mut reply_job = build(1, 10);
            reply_job.message.thread_id = thread.map(str::to_string);
            reply_job.message.reply_to_message_id = reply.map(str::to_string);
            assert!(matches!(
                create_job(&pool, reply_job, now).await,
                Err(MailMergeError::ReplyNotAllowed)
            ));
        }
        assert!(create_job(&pool, build(MAX_JOB_RECIPIENTS, 10), now)
            .await
            .is_ok());
    }

    #[test]
    fn picks_a_slower_pace_for_gmail() {
        assert_eq!(default_delay_seconds("gmail", "a@company.test"), 15);
        assert_eq!(default_delay_seconds("imap", "a@googlemail.com"), 15);
        assert_eq!(default_delay_seconds("outlook", "a@company.test"), 10);
        assert_eq!(default_delay_seconds("imap", "a@hotmail.de"), 10);
        assert_eq!(default_delay_seconds("imap", "a@example.test"), 20);
    }
}
