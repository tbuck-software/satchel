//! Tauri-free persistence for follow-up reminders.

use sqlx::{FromRow, Sqlite, SqlitePool, Transaction};
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct FollowUpUpsertRequest {
    pub id: Option<String>,
    pub thread_id: String,
    pub due_at: String,
    pub note: Option<String>,
}

/// The complete follow-up projection used by the app and maintenance worker.
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct FollowUpRecord {
    pub id: String,
    pub account_id: String,
    pub thread_id: String,
    pub message_id: Option<String>,
    pub due_at: String,
    pub status: String,
    pub note: Option<String>,
    pub subject: Option<String>,
    pub snippet: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub resolved_at: Option<String>,
}

#[derive(Debug, Error)]
pub enum FollowUpError {
    #[error("follow-up timestamp is invalid")]
    InvalidDueAt,
    #[error("thread was not found")]
    ThreadNotFound,
    #[error("follow-up persistence failed: {0}")]
    Persistence(#[source] sqlx::Error),
}

impl From<sqlx::Error> for FollowUpError {
    fn from(error: sqlx::Error) -> Self {
        Self::Persistence(error)
    }
}

#[derive(Debug, FromRow)]
struct ClaimedFollowUpId {
    id: String,
}

const FOLLOW_UP_SELECT: &str =
    "SELECT f.id, f.account_id, f.thread_id, f.message_id, f.due_at, f.status, f.note,
            t.subject, t.snippet, f.created_at, f.updated_at, f.resolved_at
     FROM follow_up_reminders f
     LEFT JOIN threads t ON t.id = f.thread_id";

/// Owns durable follow-up reminders while leaving serialization, notifications
/// and Tauri command concerns to the app crate.
pub struct FollowUpRepository<'a> {
    pool: &'a SqlitePool,
}

impl<'a> FollowUpRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn list(&self, status: Option<&str>) -> Result<Vec<FollowUpRecord>, FollowUpError> {
        let status = status.unwrap_or("pending");
        sqlx::query_as::<_, FollowUpRecord>(&format!(
            "{FOLLOW_UP_SELECT} WHERE f.status = ? ORDER BY f.due_at ASC"
        ))
        .bind(status)
        .fetch_all(self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn upsert(
        &self,
        input: FollowUpUpsertRequest,
    ) -> Result<FollowUpRecord, FollowUpError> {
        let due_at = normalize_due_at(&input.due_at)?;
        let id = input
            .id
            .unwrap_or_else(|| format!("follow-{}", uuid::Uuid::new_v4()));
        let now = chrono::Utc::now().to_rfc3339();
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;

        let account_id =
            sqlx::query_scalar::<_, String>("SELECT account_id FROM threads WHERE id = ? LIMIT 1")
                .bind(&input.thread_id)
                .fetch_optional(&mut *transaction)
                .await?
                .ok_or(FollowUpError::ThreadNotFound)?;

        // On conflict this deliberately updates only reminder-owned fields.
        // The original account/thread/message association is immutable.
        sqlx::query(
            "INSERT INTO follow_up_reminders
                (id, account_id, thread_id, message_id, due_at, status, note,
                 created_at, updated_at, resolved_at)
             VALUES (?, ?, ?, NULL, ?, 'pending', ?, ?, ?, NULL)
             ON CONFLICT(id) DO UPDATE SET
                due_at = excluded.due_at,
                status = 'pending',
                note = excluded.note,
                updated_at = excluded.updated_at,
                resolved_at = NULL",
        )
        .bind(&id)
        .bind(account_id)
        .bind(&input.thread_id)
        .bind(due_at)
        .bind(input.note.as_deref())
        .bind(&now)
        .bind(&now)
        .execute(&mut *transaction)
        .await?;

        let record =
            sqlx::query_as::<_, FollowUpRecord>(&format!("{FOLLOW_UP_SELECT} WHERE f.id = ?"))
                .bind(&id)
                .fetch_one(&mut *transaction)
                .await?;
        transaction.commit().await?;
        Ok(record)
    }

    /// Cancel a reminder. Missing ids are intentionally a no-op so repeated
    /// UI actions and stale undo callbacks remain harmless.
    pub async fn cancel(&self, id: &str) -> Result<(), FollowUpError> {
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "UPDATE follow_up_reminders
             SET status = 'cancelled', resolved_at = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(&now)
        .bind(&now)
        .bind(id)
        .execute(self.pool)
        .await?;
        Ok(())
    }

    /// Create a reminder associated with the exact sent message. No due date
    /// means no follow-up and, importantly, no database access at all.
    pub async fn create_for_sent_message(
        &self,
        account_id: &str,
        thread_id: &str,
        message_id: &str,
        due_at: Option<&str>,
    ) -> Result<(), FollowUpError> {
        let Some(due_at) = due_at else {
            return Ok(());
        };
        let due_at = normalize_due_at(due_at)?;
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO follow_up_reminders
                (id, account_id, thread_id, message_id, due_at, status, note,
                 created_at, updated_at, resolved_at)
             VALUES (?, ?, ?, ?, ?, 'pending', NULL, ?, ?, NULL)",
        )
        .bind(format!("follow-{}", uuid::Uuid::new_v4()))
        .bind(account_id)
        .bind(thread_id)
        .bind(message_id)
        .bind(due_at)
        .bind(&now)
        .bind(&now)
        .execute(self.pool)
        .await?;
        Ok(())
    }

    /// Resolve pending reminders only for a genuinely external reply.
    pub async fn resolve_for_thread_reply(
        &self,
        thread_id: &str,
        from_address: &str,
        account_email: &str,
    ) -> Result<(), FollowUpError> {
        if from_address.eq_ignore_ascii_case(account_email) {
            return Ok(());
        }
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "UPDATE follow_up_reminders
             SET status = 'resolved', resolved_at = ?, updated_at = ?
             WHERE thread_id = ? AND status = 'pending'",
        )
        .bind(&now)
        .bind(&now)
        .bind(thread_id)
        .execute(self.pool)
        .await?;
        Ok(())
    }

    /// Atomically claim at most 25 due reminders.
    ///
    /// The guarded UPDATE owns the selection and state transition in one
    /// write statement. A second worker therefore sees no pending rows after
    /// the first worker commits, rather than receiving the same pre-claimed
    /// rows. Subject and snippet are loaded before this transaction commits.
    pub async fn claim_due(&self) -> Result<Vec<FollowUpRecord>, FollowUpError> {
        let now = chrono::Utc::now().to_rfc3339();
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let claimed = sqlx::query_as::<_, ClaimedFollowUpId>(
            "UPDATE follow_up_reminders
             SET status = 'due', updated_at = ?
             WHERE status = 'pending'
               AND due_at <= ?
               AND id IN (
                   SELECT id
                   FROM follow_up_reminders
                   WHERE status = 'pending' AND due_at <= ?
                   ORDER BY due_at ASC
                   LIMIT 25
               )
             RETURNING id",
        )
        .bind(&now)
        .bind(&now)
        .bind(&now)
        .fetch_all(&mut *transaction)
        .await?;

        if claimed.is_empty() {
            transaction.commit().await?;
            return Ok(Vec::new());
        }

        let placeholders = std::iter::repeat_n("?", claimed.len())
            .collect::<Vec<_>>()
            .join(", ");
        let records_sql = format!(
            "{FOLLOW_UP_SELECT} WHERE f.id IN ({placeholders})
             ORDER BY f.due_at ASC"
        );
        let mut records_query = sqlx::query_as::<_, FollowUpRecord>(&records_sql);
        for item in &claimed {
            records_query = records_query.bind(&item.id);
        }
        let records = records_query.fetch_all(&mut *transaction).await?;
        transaction.commit().await?;
        Ok(records)
    }
}

/// Reassign follow-ups while the caller's transaction is already open.
///
/// This is crate-internal because a thread merge must move every dependent in
/// its existing transaction; opening a second transaction here would make a
/// partial merge observable.
pub(crate) async fn reassign_thread_tx(
    transaction: &mut Transaction<'_, Sqlite>,
    from_thread_id: &str,
    to_thread_id: &str,
) -> Result<(), sqlx::Error> {
    if from_thread_id == to_thread_id {
        return Ok(());
    }
    sqlx::query("UPDATE follow_up_reminders SET thread_id = ? WHERE thread_id = ?")
        .bind(to_thread_id)
        .bind(from_thread_id)
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

fn normalize_due_at(raw: &str) -> Result<String, FollowUpError> {
    chrono::DateTime::parse_from_rfc3339(raw)
        .map(|value| value.with_timezone(&chrono::Utc).to_rfc3339())
        .map_err(|_| FollowUpError::InvalidDueAt)
}
