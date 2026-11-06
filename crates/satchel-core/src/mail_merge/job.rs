//! Persisted mail merge jobs and the step that moves them forward.
//!
//! [`tick`] is the whole runner: it settles rows whose outbox action has
//! finished, then hands the next due row of each running job to the outbox.
//! A job has at most one row in the outbox at a time, and the next one goes
//! `delay_seconds` after the previous one was handed over. Everything lives
//! in the database, so a restart continues where the last run stopped.

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use sqlx::{FromRow, SqlitePool};

use super::plan::{render_message, MergeTemplate, MAX_JOB_RECIPIENTS};
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

fn ts(now: DateTime<Utc>) -> String {
    now.to_rfc3339()
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

/// Stores the job and its rows. It starts running at once.
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
    // Every copy is a new message to one person; nothing of a shared
    // conversation or a shared header may travel along.
    message.to.clear();
    message.cc.clear();
    message.bcc.clear();
    message.thread_id = None;
    message.reply_to_message_id = None;
    message.draft_id = None;
    message.scheduled_for = None;
    message.follow_up_at = None;
    let template = serde_json::to_string(&message)?;
    let id = uuid::Uuid::new_v4().to_string();
    let now_text = ts(now);

    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    sqlx::query(
        "INSERT INTO mail_merge_jobs
         (id, account_id, status, subject, source_label, template, delay_seconds, next_send_at, created_at, updated_at)
         VALUES (?, ?, 'running', ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&job.account_id)
    .bind(&message.subject)
    .bind(&job.source_label)
    .bind(&template)
    .bind(delay)
    .bind(&now_text)
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

pub async fn resume_job(pool: &SqlitePool, id: &str, now: DateTime<Utc>) -> MailMergeResult<()> {
    let changed = sqlx::query(
        "UPDATE mail_merge_jobs
         SET status = 'running', pause_reason = NULL, consecutive_failures = 0,
             next_send_at = ?, updated_at = ?
         WHERE id = ? AND status = 'paused'",
    )
    .bind(ts(now))
    .bind(ts(now))
    .bind(id)
    .execute(pool)
    .await?
    .rows_affected();
    require_changed(pool, id, changed).await
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
    Ok(())
}

/// Puts failed rows back in line and runs the job again. Their old outbox
/// actions go; the rows keep their Message-ID, so a message that did leave
/// after all is recognised instead of sent twice.
pub async fn retry_failed(pool: &SqlitePool, id: &str, now: DateTime<Utc>) -> MailMergeResult<u64> {
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
    .bind(ts(now))
    .bind(id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if retried > 0 {
        sqlx::query(
            "UPDATE mail_merge_jobs
             SET status = 'running', pause_reason = NULL, consecutive_failures = 0,
                 finished_at = NULL, next_send_at = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(ts(now))
        .bind(ts(now))
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

#[derive(FromRow)]
struct QueuedRow {
    id: String,
    job_id: String,
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

async fn settle(pool: &SqlitePool, row: &QueuedRow) -> MailMergeResult<Option<Settled>> {
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
            "delivery_unknown" => Some(Settled::Unknown(state.last_error)),
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

/// Settles finished rows, then hands each running job's next due row to the
/// outbox. Safe to call at any time and as often as needed.
pub async fn tick(
    pool: &SqlitePool,
    queue: &SyncQueue,
    now: DateTime<Utc>,
) -> MailMergeResult<TickReport> {
    let mut report = TickReport::default();
    let now_text = ts(now);

    // A failed message the user retried from the Outbox is on its way again.
    let revived: Vec<String> = sqlx::query_scalar(
        "UPDATE mail_merge_recipients SET status = 'queued', error = NULL, updated_at = ?
         WHERE status = 'failed' AND queue_id IN (
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

    let queued = sqlx::query_as::<_, QueuedRow>(
        "SELECT id, job_id, queue_id, message_id_header FROM mail_merge_recipients WHERE status = 'queued'",
    )
    .fetch_all(pool)
    .await?;
    for row in queued {
        let Some(outcome) = settle(pool, &row).await? else {
            report.in_flight = true;
            continue;
        };
        let (status, error, failure) = match outcome {
            Settled::Sent => ("sent", None, Some(false)),
            Settled::Failed(error) => ("failed", error, Some(true)),
            Settled::Unknown(error) => ("unknown", error, None),
            Settled::Removed => (
                "cancelled",
                Some(ERROR_REMOVED_FROM_OUTBOX.to_string()),
                None,
            ),
        };
        sqlx::query(
            "UPDATE mail_merge_recipients
             SET status = ?, error = ?, sent_at = CASE WHEN ? = 'sent' THEN ? ELSE sent_at END, updated_at = ?
             WHERE id = ? AND status = 'queued'",
        )
        .bind(status)
        .bind(error)
        .bind(status)
        .bind(&now_text)
        .bind(&now_text)
        .bind(&row.id)
        .execute(pool)
        .await?;
        match failure {
            Some(true) => {
                sqlx::query(
                    "UPDATE mail_merge_jobs SET consecutive_failures = consecutive_failures + 1, updated_at = ? WHERE id = ?",
                )
                .bind(&now_text)
                .bind(&row.job_id)
                .execute(pool)
                .await?;
                sqlx::query(
                    "UPDATE mail_merge_jobs SET status = 'paused', pause_reason = ?, updated_at = ?
                     WHERE id = ? AND status = 'running' AND consecutive_failures >= ?",
                )
                .bind(PAUSE_REASON_FAILURES)
                .bind(&now_text)
                .bind(&row.job_id)
                .bind(AUTO_PAUSE_FAILURES)
                .execute(pool)
                .await?;
            }
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
        push_changed(&mut report, &row.job_id);
    }

    let running: Vec<(String, String, String, i64, Option<String>)> = sqlx::query_as(
        "SELECT id, account_id, template, delay_seconds, next_send_at FROM mail_merge_jobs WHERE status = 'running'",
    )
    .fetch_all(pool)
    .await?;
    for (job_id, account_id, template, delay, next_send_at) in running {
        let busy: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM mail_merge_recipients WHERE job_id = ? AND status = 'queued'",
        )
        .bind(&job_id)
        .fetch_one(pool)
        .await?;
        if busy > 0 {
            report.in_flight = true;
            continue;
        }
        let next: Option<(String, String, String, String)> = sqlx::query_as(
            "SELECT id, email, fields, message_id_header FROM mail_merge_recipients
             WHERE job_id = ? AND status = 'pending' ORDER BY position LIMIT 1",
        )
        .bind(&job_id)
        .fetch_optional(pool)
        .await?;
        let Some((row_id, email, fields, message_id_header)) = next else {
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
        let due = next_send_at
            .as_deref()
            .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
            .map(|value| value.with_timezone(&Utc));
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
             SET status = 'queued', queue_id = ?, attempts = attempts + 1, updated_at = ?
             WHERE id = ? AND status = 'pending'",
        )
        .bind(&action.id)
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
        report.in_flight = true;
        push_changed(&mut report, &job_id);
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("pool");
        crate::db::init(&pool).await.expect("migrations");
        sqlx::query(
            "INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
             VALUES ('acc', 'Me', 'me@example.com', 'imap', '{}', '', '', '');
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
            "to": ["shared@example.com"],
            "cc": ["cc@example.com"],
            "bcc": ["bcc@example.com"],
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
            ]),
        )
    }

    async fn new_job(pool: &SqlitePool, now: DateTime<Utc>, count: usize) -> String {
        let recipients = (0..count)
            .map(|index| recipient(&format!("p{index}@example.com"), &format!("P{index}")))
            .collect();
        create_job(
            pool,
            NewMailMergeJob {
                account_id: "acc".into(),
                message: message(),
                source_label: "list.csv".into(),
                delay_seconds: 10,
                recipients,
            },
            now,
        )
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
             VALUES (?, 'acc', 'sent', '', 'me@example.com', '[]', '', '', '', '', 1, ?)",
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

    async fn outbox_fails(pool: &SqlitePool, queue_id: &str) {
        sqlx::query("UPDATE sync_queue SET status = 'failed', last_error = 'backend.errors.smtpRejected' WHERE id = ?")
            .bind(queue_id)
            .execute(pool)
            .await
            .expect("fail");
    }

    #[tokio::test]
    async fn sends_one_personal_copy_at_a_time_with_the_delay_between() {
        let pool = pool().await;
        let queue = SyncQueue::new();
        let start = Utc::now();
        let job = new_job(&pool, start, 2).await;

        let report = tick(&pool, &queue, start).await.expect("tick");
        assert_eq!(report.changed_jobs, vec![job.clone()]);
        let rows = queue_rows(&pool).await;
        assert_eq!(rows.len(), 1);
        let payload: serde_json::Value = serde_json::from_str(&rows[0].2).expect("json");
        let input = &payload["input"];
        assert_eq!(input["to"], serde_json::json!(["p0@example.com"]));
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
        assert!(rows[0].2.contains("p1@example.com"));
        outbox_sends(&pool, &rows[0].0).await;
        tick(&pool, &queue, start + Duration::seconds(11))
            .await
            .expect("tick");

        let detail = job_detail(&pool, &job).await.expect("detail");
        assert_eq!(detail.job.status, "completed");
        assert_eq!(detail.job.sent, 2);
        assert!(detail.recipients.iter().all(|row| row.sent_at.is_some()));
    }

    /// The job lives in the database: a new process (new queue, new tick)
    /// picks up where the old one stopped, without sending anything twice.
    #[tokio::test]
    async fn resumes_after_a_restart_without_resending() {
        let pool = pool().await;
        let start = Utc::now();
        let job = new_job(&pool, start, 3).await;
        tick(&pool, &SyncQueue::new(), start).await.expect("tick");
        let first = queue_rows(&pool).await[0].0.clone();
        outbox_sends(&pool, &first).await;

        // "Restart": a fresh queue handle and a later clock.
        let queue = SyncQueue::new();
        tick(&pool, &queue, start + Duration::seconds(30))
            .await
            .expect("tick");
        let rows = queue_rows(&pool).await;
        assert_eq!(rows.len(), 1);
        assert!(rows[0].2.contains("p1@example.com"));
        let detail = job_detail(&pool, &job).await.expect("detail");
        assert_eq!(
            (detail.job.sent, detail.job.queued, detail.job.pending),
            (1, 1, 1)
        );
    }

    #[tokio::test]
    async fn pause_resume_and_cancel() {
        let pool = pool().await;
        let queue = SyncQueue::new();
        let start = Utc::now();
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
        let mut now = Utc::now();
        let job = new_job(&pool, now, 5).await;
        for _ in 0..AUTO_PAUSE_FAILURES {
            tick(&pool, &queue, now).await.expect("tick");
            let id = queue_rows(&pool).await.last().expect("row").0.clone();
            outbox_fails(&pool, &id).await;
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
        assert!(rows[0].2.contains("p0@example.com"));
        assert!(rows[0].2.contains(&old_mid));
        assert_eq!(
            job_summary(&pool, &job).await.expect("job").status,
            "running"
        );
    }

    #[tokio::test]
    async fn a_message_removed_from_the_outbox_counts_as_cancelled() {
        let pool = pool().await;
        let queue = SyncQueue::new();
        let now = Utc::now();
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

    #[tokio::test]
    async fn refuses_empty_and_oversized_jobs_and_bad_delays() {
        let pool = pool().await;
        let now = Utc::now();
        let build = |count: usize, delay: i64| NewMailMergeJob {
            account_id: "acc".into(),
            message: message(),
            source_label: String::new(),
            delay_seconds: delay,
            recipients: (0..count)
                .map(|index| recipient(&format!("p{index}@example.com"), "x"))
                .collect(),
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
        assert!(create_job(&pool, build(MAX_JOB_RECIPIENTS, 10), now)
            .await
            .is_ok());
    }

    #[test]
    fn picks_a_slower_pace_for_gmail() {
        assert_eq!(default_delay_seconds("gmail", "a@company.com"), 15);
        assert_eq!(default_delay_seconds("imap", "a@googlemail.com"), 15);
        assert_eq!(default_delay_seconds("outlook", "a@company.com"), 10);
        assert_eq!(default_delay_seconds("imap", "a@hotmail.de"), 10);
        assert_eq!(default_delay_seconds("imap", "a@example.org"), 20);
    }
}
