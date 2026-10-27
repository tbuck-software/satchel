use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, Sqlite, SqlitePool, Transaction};

/// Queue action kind for an outgoing message. Defined here rather than in the
/// app's outbox worker so the queue can count and filter sends without it.
pub const ACTION_SEND_MESSAGE: &str = "send_message";
/// Draft uploads share the durable queue and the manual retry for sync actions.
pub const ACTION_SYNC_DRAFT: &str = "sync_draft";

/// The states an outgoing message may be discarded from. Anything else means
/// the worker already owns the row: `sending` is mid-submission, and a row that
/// is gone was completed. One list, used both by the UI-facing check and by the
/// DELETE that enforces it — two lists would drift apart.
pub const DISCARDABLE_MAIL_STATUSES: &[&str] = &[
    "pending",
    "scheduled",
    "failed",
    "delivered",
    "delivery_unknown",
];
/// What the outbox row says when a send neither succeeded nor plainly failed.
///
/// Shared by the crash path (`reset_sending`) and the in-process one
/// (`mark_delivery_unknown`): the situation is the same and so is the only
/// useful advice — look in the provider's Sent folder.
pub const DELIVERY_STATUS_UNKNOWN_ERROR_KEY: &str = "backend.errors.messageDeliveryStatusUnknown";

/// How long a `notification:` row belongs to the Android notification-action
/// worker after it last touched it.
///
/// That worker enqueues and drains its rows in its own process, so the main
/// worker leaves them alone. But WorkManager may never come back for them (the
/// work was cancelled, or ended on a failure the user then retried), and
/// skipping them for good kept their thread locked behind
/// `mailMovePending`. A row nobody has touched for this long is adopted by the
/// main worker like any other. Retrying a failed row hands it over at once.
pub const NOTIFICATION_ROW_LEASE_MS: i64 = 15 * 60 * 1000;

fn notification_lease_cutoff_ms() -> i64 {
    Utc::now().timestamp_millis() - NOTIFICATION_ROW_LEASE_MS
}

fn sending_session() -> &'static str {
    static SESSION: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    SESSION.get_or_init(|| uuid::Uuid::new_v4().to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueuedAction {
    pub id: String,
    pub account_id: String,
    pub action_type: String,
    pub payload: serde_json::Value,
    pub status: String,
    pub retry_count: u32,
    pub created_at: String,
    pub updated_at: String,
    pub last_error: Option<String>,
    pub next_retry_at: Option<String>,
    pub scheduled_for: Option<String>,
    /// Set the moment the message is handed to the provider. Until then a
    /// crash cannot have produced a delivery, so the row stays retryable.
    pub submission_started_at: Option<String>,
    /// How often this action has been deferred for a provider rate limit.
    /// Counted apart from `retry_count`, which a rate limit deliberately does
    /// not consume — see migration 0075.
    #[serde(default)]
    pub rate_limit_rounds: u32,
}

impl QueuedAction {
    pub fn new(
        account_id: String,
        action_type: impl Into<String>,
        payload: serde_json::Value,
    ) -> Self {
        let now = Utc::now().to_rfc3339();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            account_id,
            action_type: action_type.into(),
            payload,
            status: "pending".into(),
            retry_count: 0,
            created_at: now.clone(),
            updated_at: now,
            last_error: None,
            next_retry_at: None,
            scheduled_for: None,
            submission_started_at: None,
            rate_limit_rounds: 0,
        }
    }

    pub fn scheduled(mut self, when: DateTime<Utc>) -> Self {
        self.status = "scheduled".into();
        self.scheduled_for = Some(when.to_rfc3339());
        self
    }
}

#[derive(Debug, FromRow)]
struct QueueRow {
    id: String,
    account_id: String,
    action_type: String,
    payload: String,
    status: String,
    retry_count: i64,
    created_at: String,
    updated_at: String,
    last_error: Option<String>,
    next_retry_at: Option<String>,
    scheduled_for: Option<String>,
    submission_started_at: Option<String>,
    rate_limit_rounds: i64,
}

impl QueueRow {
    fn into_action(self) -> QueuedAction {
        QueuedAction {
            id: self.id,
            account_id: self.account_id,
            action_type: self.action_type,
            payload: serde_json::from_str(&self.payload).unwrap_or(serde_json::Value::Null),
            status: self.status,
            retry_count: self.retry_count.max(0) as u32,
            created_at: self.created_at,
            updated_at: self.updated_at,
            last_error: self.last_error,
            next_retry_at: self.next_retry_at,
            scheduled_for: self.scheduled_for,
            submission_started_at: self.submission_started_at,
            rate_limit_rounds: self.rate_limit_rounds.max(0) as u32,
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct SyncQueue {
    /// Wakes the outbox worker as soon as an action is enqueued/requeued, so a
    /// send doesn't sit in the queue until the worker's next poll (up to 60 s).
    notify: std::sync::Arc<tokio::sync::Notify>,
}

impl SyncQueue {
    /// One head per account. A mutation awaiting retry cannot be overtaken by
    /// a later mutation using identities it is about to change. Due sends may
    /// still take priority; future scheduled sends do not block the account.
    pub async fn ready_account_heads(
        &self,
        pool: &SqlitePool,
    ) -> Result<Vec<QueuedAction>, String> {
        let now = Utc::now().to_rfc3339();
        let rows = sqlx::query_as::<_, QueueRow>(
            "WITH heads AS (
                SELECT *, ROW_NUMBER() OVER (
                    PARTITION BY account_id ORDER BY
                    CASE action_type WHEN 'send_message' THEN 0 ELSE 1 END,
                    rowid
                ) AS position
                FROM sync_queue
                WHERE status IN ('pending', 'scheduled')
                  AND (id NOT LIKE 'notification:%' OR COALESCE(updated_at_ms, 0) <= ?)
                  AND NOT EXISTS (SELECT 1 FROM accounts WHERE accounts.id = sync_queue.account_id AND is_active = 0)
                  AND NOT EXISTS (SELECT 1 FROM sync_queue_dependencies d JOIN sync_queue predecessor ON predecessor.id = d.depends_on WHERE d.action_id = sync_queue.id)
                  AND (action_type <> 'send_message' OR
                       (next_retry_at IS NULL OR next_retry_at <= ?)
                       AND (scheduled_for IS NULL OR scheduled_for <= ?))
             ) SELECT id, account_id, action_type, payload, status, retry_count,
                 created_at, updated_at, last_error, next_retry_at, scheduled_for,
                 submission_started_at, rate_limit_rounds
               FROM heads WHERE position = 1
                 AND (next_retry_at IS NULL OR next_retry_at <= ?)
                 AND (scheduled_for IS NULL OR scheduled_for <= ?)
               ORDER BY CASE action_type WHEN 'send_message' THEN 0 ELSE 1 END, created_at",
        )
        .bind(notification_lease_cutoff_ms())
        .bind(&now)
        .bind(&now)
        .bind(&now)
        .bind(&now)
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;
        Ok(rows.into_iter().map(QueueRow::into_action).collect())
    }

    pub fn new() -> Self {
        Self::default()
    }

    /// Resolves when a new action has been enqueued since the call started.
    /// A notification fired while the worker was busy is buffered (one permit),
    /// so enqueue-during-processing wakes the next wait immediately.
    pub async fn wait_for_enqueue(&self) {
        self.notify.notified().await;
    }

    pub async fn enqueue(&self, pool: &SqlitePool, action: QueuedAction) -> Result<(), String> {
        let mut tx = pool.begin().await.map_err(|err| err.to_string())?;
        self.enqueue_in_transaction(&mut tx, &action).await?;
        tx.commit().await.map_err(|err| err.to_string())?;
        self.wake_worker();
        Ok(())
    }

    /// Insert an action as part of a caller-owned transaction. Workflow state,
    /// its durable provider operation and its audit event can therefore commit
    /// together instead of leaving a half-applied transition after a crash.
    pub async fn enqueue_in_transaction(
        &self,
        tx: &mut Transaction<'_, Sqlite>,
        action: &QueuedAction,
    ) -> Result<(), String> {
        let payload = serde_json::to_string(&action.payload).unwrap_or_else(|_| "null".into());
        let now_ms = Utc::now().timestamp_millis();
        sqlx::query(
            "INSERT INTO sync_queue (id, account_id, action_type, payload, status, retry_count, last_error, created_at, updated_at, next_retry_at, scheduled_for, updated_at_ms, submission_started_at, sending_session)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                payload = CASE
                    WHEN sync_queue.action_type = 'remote_mutation'
                     AND json_valid(sync_queue.payload)
                     AND json_valid(excluded.payload)
                     AND json_extract(sync_queue.payload, '$.kind') = 'blocked_move'
                     AND json_extract(excluded.payload, '$.kind') = 'blocked_move'
                     AND json_array_length(json_extract(sync_queue.payload, '$.refs')) = 1
                     AND json_array_length(json_extract(excluded.payload, '$.refs')) = 1
                     AND json_extract(sync_queue.payload, '$.blocked_thread_id')
                         IS json_extract(excluded.payload, '$.blocked_thread_id')
                     AND json_extract(sync_queue.payload, '$.target_remote_folder_id')
                         IS json_extract(excluded.payload, '$.target_remote_folder_id')
                     AND json_extract(sync_queue.payload, '$.refs[0].remote_id')
                         IS json_extract(excluded.payload, '$.refs[0].remote_id')
                     AND json_extract(sync_queue.payload, '$.refs[0].source_folder')
                         IS json_extract(excluded.payload, '$.refs[0].source_folder')
                     AND json_extract(sync_queue.payload, '$.refs[0].source_uidvalidity')
                         IS json_extract(excluded.payload, '$.refs[0].source_uidvalidity')
                    THEN json_set(
                        json_set(
                            json_set(
                                excluded.payload,
                                '$.provider_move_started',
                                CASE WHEN MAX(
                                    CAST(COALESCE(json_extract(sync_queue.payload, '$.provider_move_started'), 0) AS INTEGER),
                                    CAST(COALESCE(json_extract(excluded.payload, '$.provider_move_started'), 0) AS INTEGER)
                                ) > 0 THEN json('true') ELSE json('false') END
                            ),
                            '$.provider_move_completed',
                            CASE WHEN MAX(
                                CAST(COALESCE(json_extract(sync_queue.payload, '$.provider_move_completed'), 0) AS INTEGER),
                                CAST(COALESCE(json_extract(excluded.payload, '$.provider_move_completed'), 0) AS INTEGER)
                            ) > 0 THEN json('true') ELSE json('false') END
                        ),
                        '$.source_uidvalidity_before',
                        CASE
                            WHEN json_type(sync_queue.payload, '$.source_uidvalidity_before') IN ('object', 'array')
                            THEN json(json_extract(sync_queue.payload, '$.source_uidvalidity_before'))
                            WHEN json_type(excluded.payload, '$.source_uidvalidity_before') IN ('object', 'array')
                            THEN json(json_extract(excluded.payload, '$.source_uidvalidity_before'))
                            ELSE json('null')
                        END
                    )
                    ELSE excluded.payload
                END,
                status = excluded.status,
                retry_count = excluded.retry_count,
                last_error = excluded.last_error,
                updated_at = excluded.updated_at,
                next_retry_at = excluded.next_retry_at,
                scheduled_for = excluded.scheduled_for,
                updated_at_ms = excluded.updated_at_ms,
                submission_started_at = excluded.submission_started_at,
                sending_session = excluded.sending_session
             WHERE NOT (
                sync_queue.action_type = ?
                AND sync_queue.status IN ('sending', 'delivered', 'delivery_unknown')
                OR sync_queue.action_type = 'remote_mutation'
                AND sync_queue.status = 'sending'
             )",
        )
        .bind(&action.id)
        .bind(&action.account_id)
        .bind(&action.action_type)
        .bind(&payload)
        .bind(&action.status)
        .bind(action.retry_count as i64)
        .bind(action.last_error.as_deref())
        .bind(&action.created_at)
        .bind(&action.updated_at)
        .bind(action.next_retry_at.as_deref())
        .bind(action.scheduled_for.as_deref())
        .bind(now_ms)
        .bind(action.submission_started_at.as_deref())
        .bind((action.status == "sending").then(sending_session))
        .bind(ACTION_SEND_MESSAGE)
        .execute(&mut **tx)
        .await
        .map_err(|err| err.to_string())?;
        Ok(())
    }

    /// Wake the worker after a transaction containing one or more queue rows
    /// has committed.
    pub fn wake_worker(&self) {
        self.notify.notify_one();
    }

    /// Atomically claim the next ready action by flipping its status to `sending`.
    /// Returns None when no action is ready (either none queued, all scheduled in the future, or all backing off).
    pub async fn claim_next(&self, pool: &SqlitePool) -> Result<Option<QueuedAction>, String> {
        let Some(action) = self.peek_next_ready(pool).await? else {
            return Ok(None);
        };
        self.claim_ready(pool, &action.id).await
    }

    /// Inspect the next due action without publishing it as active. The worker
    /// uses this to acquire any provider/sync permit before claiming the row, so
    /// waiting for an internal lock is not shown to people as `sending`.
    pub async fn peek_next_ready(&self, pool: &SqlitePool) -> Result<Option<QueuedAction>, String> {
        let now = Utc::now().to_rfc3339();
        let row: Option<QueueRow> = sqlx::query_as::<_, QueueRow>(
            "SELECT id, account_id, action_type, payload, status, retry_count, created_at, updated_at, last_error, next_retry_at, scheduled_for,
                    submission_started_at, rate_limit_rounds
             FROM sync_queue
             WHERE status IN ('pending', 'scheduled')
               AND NOT EXISTS (SELECT 1 FROM accounts WHERE accounts.id = sync_queue.account_id AND is_active = 0)
               AND NOT EXISTS (SELECT 1 FROM sync_queue_dependencies d JOIN sync_queue predecessor ON predecessor.id = d.depends_on WHERE d.action_id = sync_queue.id)
               AND (id NOT LIKE 'notification:%' OR COALESCE(updated_at_ms, 0) <= ?)
               AND (next_retry_at IS NULL OR next_retry_at <= ?)
               AND (scheduled_for IS NULL OR scheduled_for <= ?)
             ORDER BY
               CASE action_type WHEN 'send_message' THEN 0 ELSE 1 END,
               COALESCE(next_retry_at, scheduled_for, created_at) ASC
             LIMIT 1",
        )
        .bind(notification_lease_cutoff_ms())
        .bind(&now)
        .bind(&now)
        .fetch_optional(pool)
        .await
        .map_err(|err| err.to_string())?;

        Ok(row.map(QueueRow::into_action))
    }

    /// Claim one previously inspected action if it is still due. Returning
    /// `None` is a normal race: Undo, cancellation, or another worker won.
    pub async fn claim_ready(
        &self,
        pool: &SqlitePool,
        id: &str,
    ) -> Result<Option<QueuedAction>, String> {
        let now = Utc::now().to_rfc3339();
        let now_ms = Utc::now().timestamp_millis();
        let mut tx = pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(|err| err.to_string())?;

        let row: Option<QueueRow> = sqlx::query_as::<_, QueueRow>(
            "SELECT id, account_id, action_type, payload, status, retry_count, created_at, updated_at, last_error, next_retry_at, scheduled_for,
                    submission_started_at, rate_limit_rounds
             FROM sync_queue
             WHERE id = ?
               AND NOT EXISTS (SELECT 1 FROM accounts WHERE accounts.id = sync_queue.account_id AND is_active = 0)
               AND NOT EXISTS (SELECT 1 FROM sync_queue_dependencies d JOIN sync_queue predecessor ON predecessor.id = d.depends_on WHERE d.action_id = sync_queue.id)
               AND status IN ('pending', 'scheduled')
               AND (next_retry_at IS NULL OR next_retry_at <= ?)
               AND (scheduled_for IS NULL OR scheduled_for <= ?)
             LIMIT 1",
        )
        .bind(id)
        .bind(&now)
        .bind(&now)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|err| err.to_string())?;

        let Some(row) = row else {
            tx.commit().await.map_err(|err| err.to_string())?;
            return Ok(None);
        };

        let claimed = sqlx::query(
            "UPDATE sync_queue
             SET status = 'sending', submission_started_at = NULL, sending_session = ?,
                 updated_at = ?, updated_at_ms = ?
             WHERE id = ? AND status = ?",
        )
        .bind(sending_session())
        .bind(&now)
        .bind(now_ms)
        .bind(&row.id)
        .bind(&row.status)
        .execute(&mut *tx)
        .await
        .map_err(|err| err.to_string())?
        .rows_affected();

        if claimed != 1 {
            tx.commit().await.map_err(|err| err.to_string())?;
            return Ok(None);
        }

        tx.commit().await.map_err(|err| err.to_string())?;

        let mut action = row.into_action();
        action.status = "sending".into();
        action.submission_started_at = None;
        Ok(Some(action))
    }

    /// Record that the message is on its way to the provider.
    ///
    /// This is the point of no return: a crash after it leaves delivery
    /// genuinely unknown, while a crash before it cannot have produced one.
    /// `reset_sending` reads exactly this difference on the next start.
    pub async fn mark_submission_started(&self, pool: &SqlitePool, id: &str) -> Result<(), String> {
        let now = Utc::now().to_rfc3339();
        let now_ms = Utc::now().timestamp_millis();
        sqlx::query(
            "UPDATE sync_queue
             SET submission_started_at = ?, updated_at = ?, updated_at_ms = ?
             WHERE id = ? AND action_type = ? AND submission_started_at IS NULL",
        )
        .bind(&now)
        .bind(&now)
        .bind(now_ms)
        .bind(id)
        .bind(ACTION_SEND_MESSAGE)
        .execute(pool)
        .await
        .map_err(|err| err.to_string())?;
        Ok(())
    }

    pub async fn mark_completed(&self, pool: &SqlitePool, id: &str) -> Result<(), String> {
        sqlx::query("DELETE FROM sync_queue WHERE id = ?")
            .bind(id)
            .execute(pool)
            .await
            .map_err(|err| err.to_string())?;
        Ok(())
    }

    /// Remove actions only when an account is permanently deleted.
    pub async fn purge_account(&self, pool: &SqlitePool, account_id: &str) -> Result<u64, String> {
        let mut tx = pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(|e| e.to_string())?;
        let removed = sqlx::query("DELETE FROM sync_queue WHERE account_id = ?")
            .bind(account_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?
            .rows_affected();
        // With no durable move left, presentation must return to the confirmed
        // provider location rather than keeping an orphaned destination.
        sqlx::query("UPDATE messages SET pending_folder_id = NULL WHERE account_id = ? AND pending_folder_id IS NOT NULL")
            .bind(account_id).execute(&mut *tx).await.map_err(|e| e.to_string())?;
        tx.commit().await.map_err(|e| e.to_string())?;
        Ok(removed)
    }

    /// Defer an action the provider asked us to slow down on.
    ///
    /// Separate from [`Self::mark_failed_retry`] because a rate limit does not
    /// consume a retry: it bumps its own counter instead, so the action still
    /// has an end even though `retry_count` never moves.
    pub async fn mark_rate_limited_retry(
        &self,
        pool: &SqlitePool,
        id: &str,
        rate_limit_rounds: u32,
        next_retry_at: DateTime<Utc>,
        error: &str,
    ) -> Result<(), String> {
        let now = Utc::now().to_rfc3339();
        let now_ms = Utc::now().timestamp_millis();
        sqlx::query(
            "UPDATE sync_queue
             SET status = 'pending',
                 rate_limit_rounds = ?,
                 next_retry_at = ?,
                 last_error = ?,
                 updated_at = ?,
                 updated_at_ms = ?
             WHERE id = ?",
        )
        .bind(rate_limit_rounds as i64)
        .bind(next_retry_at.to_rfc3339())
        .bind(error)
        .bind(&now)
        .bind(now_ms)
        .bind(id)
        .execute(pool)
        .await
        .map_err(|err| err.to_string())?;
        Ok(())
    }

    pub async fn mark_failed_retry(
        &self,
        pool: &SqlitePool,
        id: &str,
        retry_count: u32,
        next_retry_at: DateTime<Utc>,
        error: &str,
    ) -> Result<(), String> {
        let now = Utc::now().to_rfc3339();
        let now_ms = Utc::now().timestamp_millis();
        sqlx::query(
            "UPDATE sync_queue
             SET status = 'pending',
                 retry_count = ?,
                 next_retry_at = ?,
                 last_error = ?,
                 updated_at = ?,
                 updated_at_ms = ?
             WHERE id = ?",
        )
        .bind(retry_count as i64)
        .bind(next_retry_at.to_rfc3339())
        .bind(error)
        .bind(&now)
        .bind(now_ms)
        .bind(id)
        .execute(pool)
        .await
        .map_err(|err| err.to_string())?;
        Ok(())
    }

    pub async fn mark_failed_permanent(
        &self,
        pool: &SqlitePool,
        id: &str,
        error: &str,
    ) -> Result<(), String> {
        let now = Utc::now().to_rfc3339();
        let now_ms = Utc::now().timestamp_millis();
        sqlx::query(
            "UPDATE sync_queue
             SET status = 'failed',
                 last_error = ?,
                 next_retry_at = NULL,
                 updated_at = ?,
                 updated_at_ms = ?
             WHERE id = ?",
        )
        .bind(error)
        .bind(&now)
        .bind(now_ms)
        .bind(id)
        .execute(pool)
        .await
        .map_err(|err| err.to_string())?;
        Ok(())
    }

    /// Keep a provider-accepted mail visible without ever making it claimable
    /// again. Retrying this state would send a duplicate.
    pub async fn mark_delivered_with_error(
        &self,
        pool: &SqlitePool,
        id: &str,
        error: &str,
    ) -> Result<(), String> {
        let now = Utc::now().to_rfc3339();
        let now_ms = Utc::now().timestamp_millis();
        sqlx::query(
            "UPDATE sync_queue
             SET status = 'delivered',
                 last_error = ?,
                 next_retry_at = NULL,
                 updated_at = ?,
                 updated_at_ms = ?
             WHERE id = ? AND action_type = ?",
        )
        .bind(error)
        .bind(&now)
        .bind(now_ms)
        .bind(id)
        .bind(ACTION_SEND_MESSAGE)
        .execute(pool)
        .await
        .map_err(|err| err.to_string())?;
        Ok(())
    }

    /// Park a send whose outcome cannot be determined.
    ///
    /// Same terminal state a crash during submission produces (see
    /// `reset_sending`), reached here while the process is still alive: the
    /// connection broke after the message went over, so neither "sent" nor
    /// "failed" is true and only the user can decide.
    pub async fn mark_delivery_unknown(&self, pool: &SqlitePool, id: &str) -> Result<(), String> {
        let now = Utc::now().to_rfc3339();
        let now_ms = Utc::now().timestamp_millis();
        sqlx::query(
            "UPDATE sync_queue
             SET status = 'delivery_unknown',
                 last_error = ?,
                 next_retry_at = NULL,
                 updated_at = ?,
                 updated_at_ms = ?
             WHERE id = ? AND action_type = ?",
        )
        .bind(DELIVERY_STATUS_UNKNOWN_ERROR_KEY)
        .bind(&now)
        .bind(now_ms)
        .bind(id)
        .bind(ACTION_SEND_MESSAGE)
        .execute(pool)
        .await
        .map_err(|err| err.to_string())?;
        Ok(())
    }

    /// Recover old retry-budget failures at startup. Explicit rejections and
    /// ambiguous deliveries stay terminal. The status/error guard prevents a
    /// concurrent manual retry or edit from being overwritten.
    pub async fn requeue_failed_transient(&self, pool: &SqlitePool) -> Result<u64, String> {
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT id, last_error FROM sync_queue WHERE status = 'failed' AND last_error IS NOT NULL"
        ).fetch_all(pool).await.map_err(|err| err.to_string())?;
        let mut changed = 0;
        for (id, error) in rows {
            if !super::errors::SyncError::recoverable_stored_failure(&error) {
                continue;
            }
            changed += sqlx::query(
                "UPDATE sync_queue SET status = 'pending', next_retry_at = NULL,
                 sending_session = NULL, updated_at = ?, updated_at_ms = ?
                 WHERE id = ? AND status = 'failed' AND last_error = ?",
            )
            .bind(Utc::now().to_rfc3339())
            .bind(Utc::now().timestamp_millis())
            .bind(id)
            .bind(error)
            .execute(pool)
            .await
            .map_err(|err| err.to_string())?
            .rows_affected();
        }
        if changed > 0 {
            self.wake_worker();
        }
        Ok(changed)
    }

    /// Recover actions that were in `sending` state when the app crashed.
    ///
    /// Remote mutations are safe to retry. A mail send is ambiguous: the
    /// provider may have accepted it just before the crash, so automatically
    /// retrying could deliver a duplicate. Keep those rows terminal and visible.
    pub async fn reset_sending(&self, pool: &SqlitePool) -> Result<u64, String> {
        let now = Utc::now().to_rfc3339();
        let now_ms = Utc::now().timestamp_millis();
        let result = sqlx::query(
            "UPDATE sync_queue
             SET status = CASE
                     WHEN action_type = ? AND submission_started_at IS NOT NULL
                     THEN 'delivery_unknown'
                     ELSE 'pending'
                 END,
                 payload = CASE
                     WHEN action_type = 'remote_mutation'
                      AND json_valid(payload)
                      AND json_extract(payload, '$.kind') = 'blocked_move'
                      AND json_array_length(json_extract(payload, '$.refs')) = 1
                      AND json_type(payload, '$.provider_move_started') IS NULL
                     THEN json_set(payload, '$.provider_move_started', json('true'))
                     ELSE payload
                 END,
                 last_error = CASE
                     WHEN action_type = ? AND submission_started_at IS NOT NULL THEN ?
                     ELSE last_error
                 END,
                 next_retry_at = CASE WHEN action_type = ? THEN NULL ELSE next_retry_at END,
                 sending_session = NULL, updated_at = ?, updated_at_ms = ?
             WHERE status = 'sending'
               AND (id NOT LIKE 'notification:%' OR COALESCE(updated_at_ms, 0) <= ?)
               AND sending_session IS NOT ?",
        )
        .bind(ACTION_SEND_MESSAGE)
        .bind(ACTION_SEND_MESSAGE)
        .bind(DELIVERY_STATUS_UNKNOWN_ERROR_KEY)
        .bind(ACTION_SEND_MESSAGE)
        .bind(&now)
        .bind(now_ms)
        .bind(notification_lease_cutoff_ms())
        .bind(sending_session())
        .execute(pool)
        .await
        .map_err(|err| err.to_string())?;
        Ok(result.rows_affected())
    }

    /// Return the given rows to `pending` if they were left in `sending` for
    /// longer than [`NOTIFICATION_ROW_LEASE_MS`]. A younger `sending` row may
    /// be executing right now in the other process.
    pub async fn reset_stale_sending_ids(
        &self,
        pool: &SqlitePool,
        ids: &[String],
    ) -> Result<u64, String> {
        if ids.is_empty() {
            return Ok(0);
        }
        let now = Utc::now().to_rfc3339();
        let now_ms = Utc::now().timestamp_millis();
        let mut query = sqlx::QueryBuilder::<Sqlite>::new(
            "UPDATE sync_queue SET status = 'pending', sending_session = NULL, updated_at = ",
        );
        query
            .push_bind(now)
            .push(", updated_at_ms = ")
            .push_bind(now_ms);
        query
            .push(" WHERE status = 'sending' AND COALESCE(updated_at_ms, 0) <= ")
            .push_bind(notification_lease_cutoff_ms());
        query.push(" AND id IN (");
        let mut separated = query.separated(", ");
        for id in ids {
            separated.push_bind(id);
        }
        separated.push_unseparated(")");
        let result = query
            .build()
            .execute(pool)
            .await
            .map_err(|err| err.to_string())?;
        Ok(result.rows_affected())
    }

    /// Returns the earliest timestamp (RFC3339) at which a pending or scheduled
    /// action becomes due. Used by the worker to sleep efficiently.
    pub async fn next_wakeup(&self, pool: &SqlitePool) -> Option<DateTime<Utc>> {
        let raw: Option<String> = sqlx::query_scalar(
            "SELECT MIN(COALESCE(next_retry_at, scheduled_for, created_at))
             FROM sync_queue
             WHERE status IN ('pending', 'scheduled')
               AND NOT EXISTS (SELECT 1 FROM accounts WHERE accounts.id = sync_queue.account_id AND is_active = 0)
               AND NOT EXISTS (SELECT 1 FROM sync_queue_dependencies d JOIN sync_queue predecessor ON predecessor.id = d.depends_on WHERE d.action_id = sync_queue.id)
               AND (id NOT LIKE 'notification:%' OR COALESCE(updated_at_ms, 0) <= ?)",
        )
        .bind(notification_lease_cutoff_ms())
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
        raw.and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
            .map(|dt| dt.with_timezone(&Utc))
    }

    pub async fn list_failed(&self, pool: &SqlitePool) -> Result<Vec<QueuedAction>, String> {
        let rows: Vec<QueueRow> = sqlx::query_as::<_, QueueRow>(
            "SELECT id, account_id, action_type, payload, status, retry_count, created_at, updated_at, last_error, next_retry_at, scheduled_for,
                    submission_started_at, rate_limit_rounds
             FROM sync_queue
             WHERE status = 'failed'
             ORDER BY updated_at DESC
             LIMIT 100",
        )
        .fetch_all(pool)
        .await
        .map_err(|err| err.to_string())?;
        Ok(rows.into_iter().map(QueueRow::into_action).collect())
    }

    pub async fn list_visible(&self, pool: &SqlitePool) -> Result<Vec<QueuedAction>, String> {
        let rows: Vec<QueueRow> = sqlx::query_as::<_, QueueRow>(
            "SELECT id, account_id, action_type, payload, status, retry_count, created_at, updated_at, last_error, next_retry_at, scheduled_for,
                    submission_started_at, rate_limit_rounds
             FROM sync_queue
             WHERE status IN ('failed', 'delivered', 'delivery_unknown', 'pending', 'scheduled', 'sending')
             ORDER BY
               CASE status
                 WHEN 'delivered' THEN 0
                 WHEN 'delivery_unknown' THEN 1
                 WHEN 'failed' THEN 2
                 WHEN 'scheduled' THEN 3
                 WHEN 'sending' THEN 4
                 ELSE 5
               END,
               COALESCE(scheduled_for, next_retry_at, created_at) ASC
             LIMIT 200",
        )
        .fetch_all(pool)
        .await
        .map_err(|err| err.to_string())?;
        Ok(rows.into_iter().map(QueueRow::into_action).collect())
    }

    /// Public Outbox projection. Archive, flag, move and folder operations use
    /// the same durable worker but are implementation details, not outgoing mail.
    pub async fn list_mail_outbox(&self, pool: &SqlitePool) -> Result<Vec<QueuedAction>, String> {
        let rows: Vec<QueueRow> = sqlx::query_as::<_, QueueRow>(
            "SELECT id, account_id, action_type, payload, status, retry_count, created_at, updated_at, last_error, next_retry_at, scheduled_for,
                    submission_started_at, rate_limit_rounds
             FROM sync_queue
             WHERE status IN ('failed', 'delivered', 'delivery_unknown', 'pending', 'scheduled', 'sending')
               AND action_type = ?
             ORDER BY
               CASE status
                 WHEN 'delivered' THEN 0
                 WHEN 'delivery_unknown' THEN 1
                 WHEN 'failed' THEN 2
                 WHEN 'scheduled' THEN 3
                 WHEN 'sending' THEN 4
                 ELSE 5
               END,
               COALESCE(scheduled_for, next_retry_at, created_at) ASC
             LIMIT 200",
        )
        .bind(ACTION_SEND_MESSAGE)
        .fetch_all(pool)
        .await
        .map_err(|err| err.to_string())?;
        Ok(rows.into_iter().map(QueueRow::into_action).collect())
    }

    /// Discard an outgoing message the user took back, in one statement.
    ///
    /// Reading the status first and deleting afterwards loses the undo race:
    /// between the two the worker claims the row, the mail goes out, and
    /// deleting the row removes the only record that it did. The condition
    /// therefore travels with the DELETE, and a caller that gets `false` knows
    /// the action moved on.
    pub async fn discard_mail_action(&self, pool: &SqlitePool, id: &str) -> Result<bool, String> {
        let mut tx = pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(|e| e.to_string())?;
        let mut query = sqlx::QueryBuilder::<Sqlite>::new("DELETE FROM sync_queue WHERE id = ");
        query.push_bind(id);
        query.push(" AND action_type = ");
        query.push_bind(ACTION_SEND_MESSAGE);
        query.push(" AND status IN (");
        let mut statuses = query.separated(", ");
        for status in DISCARDABLE_MAIL_STATUSES {
            statuses.push_bind(*status);
        }
        statuses.push_unseparated(") RETURNING account_id, payload");
        let removed: Option<(String, String)> = query
            .build_query_as()
            .fetch_optional(&mut *tx)
            .await
            .map_err(|err| err.to_string())?;
        if let Some((account_id, payload)) = &removed {
            let payload: serde_json::Value =
                serde_json::from_str(payload).unwrap_or(serde_json::Value::Null);
            if let Some(id) = payload
                .pointer("/input/draft_id")
                .and_then(serde_json::Value::as_str)
            {
                sqlx::query("UPDATE messages SET scheduled_for = NULL WHERE id = ? AND account_id = ? AND is_draft = 1")
                    .bind(id).bind(account_id).execute(&mut *tx).await.map_err(|e| e.to_string())?;
            }
        }
        tx.commit().await.map_err(|e| e.to_string())?;
        Ok(removed.is_some())
    }

    pub async fn requeue(&self, pool: &SqlitePool, id: &str) -> Result<(), String> {
        let now = Utc::now().to_rfc3339();
        let now_ms = Utc::now().timestamp_millis();
        sqlx::query(
            "UPDATE sync_queue SET status = 'pending', retry_count = 0, next_retry_at = NULL, last_error = NULL, updated_at = ?, updated_at_ms = CASE WHEN id LIKE 'notification:%' THEN 0 ELSE ? END WHERE id = ?",
        )
        .bind(&now)
        .bind(now_ms)
        .bind(id)
        .execute(pool)
        .await
        .map_err(|err| err.to_string())?;
        self.notify.notify_one();
        Ok(())
    }

    pub async fn requeue_mail(&self, pool: &SqlitePool, id: &str) -> Result<(), String> {
        let now = Utc::now().to_rfc3339();
        let now_ms = Utc::now().timestamp_millis();
        let changed = sqlx::query(
            "UPDATE sync_queue
             SET status = 'pending', retry_count = 0, next_retry_at = NULL,
                 scheduled_for = NULL, last_error = NULL, submission_started_at = NULL,
                 updated_at = ?, updated_at_ms = ?
             WHERE id = ? AND action_type = ?
               AND (status IN ('failed', 'pending', 'scheduled')
                    OR (status = 'delivery_unknown' AND submission_started_at IS NULL))",
        )
        .bind(&now)
        .bind(now_ms)
        .bind(id)
        .bind(ACTION_SEND_MESSAGE)
        .execute(pool)
        .await
        .map_err(|err| err.to_string())?
        .rows_affected();
        if changed != 1 {
            return Err("Diese Mail kann gerade nicht erneut eingereiht werden.".to_string());
        }
        self.notify.notify_one();
        Ok(())
    }

    pub async fn list_failed_moves(&self, pool: &SqlitePool) -> Result<Vec<QueuedAction>, String> {
        let rows = sqlx::query_as::<_, QueueRow>(
            "SELECT id, account_id, action_type, payload, status, retry_count, created_at, updated_at,
             last_error, next_retry_at, scheduled_for, submission_started_at, rate_limit_rounds
             FROM sync_queue WHERE action_type = 'remote_mutation' AND status = 'failed'
             AND json_valid(payload) AND json_extract(payload, '$.kind') IN ('workflow_move', 'blocked_move') ORDER BY created_at LIMIT 200",
        ).fetch_all(pool).await.map_err(|e| e.to_string())?;
        Ok(rows.into_iter().map(QueueRow::into_action).collect())
    }

    pub async fn requeue_failed_move(&self, pool: &SqlitePool, id: &str) -> Result<bool, String> {
        let changed = sqlx::query(
            "UPDATE sync_queue SET status = 'pending', retry_count = 0, next_retry_at = NULL, last_error = NULL,
             updated_at = ?, updated_at_ms = CASE WHEN id LIKE 'notification:%' THEN 0 ELSE ? END WHERE id = ? AND action_type = 'remote_mutation' AND status = 'failed'
             AND json_valid(payload) AND json_extract(payload, '$.kind') IN ('workflow_move', 'blocked_move')",
        ).bind(Utc::now().to_rfc3339()).bind(Utc::now().timestamp_millis()).bind(id)
            .execute(pool).await.map_err(|e| e.to_string())?.rows_affected();
        if changed > 0 {
            self.wake_worker();
        }
        Ok(changed > 0)
    }

    pub async fn requeue_failed_remote_mutations(&self, pool: &SqlitePool) -> Result<u64, String> {
        let now = Utc::now().to_rfc3339();
        let now_ms = Utc::now().timestamp_millis();
        let changed = sqlx::query(
            "UPDATE sync_queue
             SET status = 'pending', retry_count = 0, next_retry_at = NULL,
                 last_error = NULL, updated_at = ?, updated_at_ms = CASE WHEN id LIKE 'notification:%' THEN 0 ELSE ? END
             WHERE action_type IN (?, ?) AND status = 'failed'",
        )
        .bind(&now)
        .bind(now_ms)
        .bind(crate::sync::remote_mutations::ACTION_REMOTE_MUTATION)
        .bind(ACTION_SYNC_DRAFT)
        .execute(pool)
        .await
        .map_err(|err| err.to_string())?
        .rows_affected();
        if changed > 0 {
            self.notify.notify_one();
        }
        Ok(changed)
    }
}

/// Compute exponential backoff in seconds for the given retry count.
/// Steps: 10 s → 1 min → 5 min → 30 min → 2 h, then capped at 2 h.
pub fn backoff_seconds(retry_count: u32) -> i64 {
    match retry_count {
        0 | 1 => 10,
        2 => 60,
        3 => 5 * 60,
        4 => 30 * 60,
        _ => 2 * 60 * 60,
    }
}

#[cfg(test)]
mod tests {
    //! Tests for the persistent outbox: enqueue → claim_next → mark_completed flow,
    //! plus crash-recovery via reset_sending and exponential backoff scheduling.

    use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
    use sqlx::SqlitePool;

    use super::*;

    use serde_json::json;

    proptest::proptest! {
        #![proptest_config(proptest::test_runner::Config::with_cases(64))]
        #[test]
        fn mail_state_sequences_match_model(operations in proptest::collection::vec(0u8..9, 1..40)) {
            let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
            runtime.block_on(async {
                let pool = fresh_pool().await;
                let queue = SyncQueue::new();
                let action = QueuedAction::new("acc-1".into(), ACTION_SEND_MESSAGE, json!({}));
                queue.enqueue(&pool, action.clone()).await.unwrap();
                let mut state = Some("pending");
                let mut submitted = false;
                for operation in operations {
                    match operation {
                        0 => {
                            let claimed = queue.claim_ready(&pool, &action.id).await.unwrap();
                            assert_eq!(claimed.is_some(), state == Some("pending"));
                            if claimed.is_some() { state = Some("sending"); submitted = false; }
                        }
                        1 if state == Some("sending") => {
                            queue.mark_submission_started(&pool, &action.id).await.unwrap();
                            submitted = true;
                        }
                        2 if state == Some("sending") && !submitted => {
                            queue.mark_failed_retry(&pool, &action.id, 1, DateTime::from_timestamp(0, 0).unwrap(), "retry").await.unwrap();
                            state = Some("pending");
                        }
                        3 if state == Some("sending") && !submitted => {
                            queue.mark_failed_permanent(&pool, &action.id, "failed").await.unwrap();
                            state = Some("failed");
                        }
                        4 => {
                            reset_after_restart(&queue, &pool).await.unwrap();
                            if state == Some("sending") {
                                state = Some(if submitted { "delivery_unknown" } else { "pending" });
                            }
                        }
                        5 => {
                            let allowed = matches!(state, Some("pending" | "failed"))
                                || state == Some("delivery_unknown") && !submitted;
                            assert_eq!(queue.requeue_mail(&pool, &action.id).await.is_ok(), allowed);
                            if allowed { state = Some("pending"); submitted = false; }
                        }
                        6 if state == Some("sending") && submitted => {
                            queue.mark_delivered_with_error(&pool, &action.id, "sent-copy").await.unwrap();
                            state = Some("delivered");
                        }
                        7 => {
                            let allowed = matches!(state, Some("pending" | "failed" | "delivered" | "delivery_unknown"));
                            assert_eq!(queue.discard_mail_action(&pool, &action.id).await.unwrap(), allowed);
                            if allowed { state = None; }
                        }
                        8 => {
                            // Repeated enqueue must not steal worker ownership or resend uncertain mail.
                            queue.enqueue(&pool, action.clone()).await.unwrap();
                            if !matches!(state, Some("sending" | "delivered" | "delivery_unknown")) {
                                state = Some("pending"); submitted = false;
                            }
                        }
                        _ => {}
                    }
                    let row: Option<(String, Option<String>)> = sqlx::query_as(
                        "SELECT status, submission_started_at FROM sync_queue WHERE id = ?"
                    ).bind(&action.id).fetch_optional(&pool).await.unwrap();
                    assert_eq!(row.as_ref().map(|r| r.0.as_str()), state);
                    assert_eq!(row.as_ref().is_some_and(|r| r.1.is_some()), state.is_some() && submitted);
                }
            });
        }
    }

    async fn status_of(pool: &SqlitePool, id: &str) -> String {
        sqlx::query_scalar::<_, String>("SELECT status FROM sync_queue WHERE id = ?")
            .bind(id)
            .fetch_one(pool)
            .await
            .expect("queue row")
    }

    async fn reset_after_restart(queue: &SyncQueue, pool: &SqlitePool) -> Result<u64, String> {
        sqlx::query("UPDATE sync_queue SET sending_session = NULL")
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
        queue.reset_sending(pool).await
    }

    #[tokio::test]
    async fn recovery_does_not_reclaim_a_live_immediate_send() {
        let pool = fresh_pool().await;
        let queue = SyncQueue::new();
        let mut action =
            QueuedAction::new("acc-1".into(), ACTION_SEND_MESSAGE, serde_json::json!({}));
        action.status = "sending".into();
        queue.enqueue(&pool, action.clone()).await.unwrap();
        assert_eq!(queue.reset_sending(&pool).await.unwrap(), 0);
        assert_eq!(status_of(&pool, &action.id).await, "sending");
        assert!(queue
            .claim_ready(&pool, &action.id)
            .await
            .unwrap()
            .is_none());
        queue
            .mark_submission_started(&pool, &action.id)
            .await
            .unwrap();
        assert_eq!(queue.reset_sending(&pool).await.unwrap(), 0);
        assert_eq!(status_of(&pool, &action.id).await, "sending");
    }

    async fn fresh_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory db");
        sqlx::query("PRAGMA journal_mode = WAL")
            .execute(&pool)
            .await
            .ok();
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .expect("migrations");
        pool
    }

    #[tokio::test]
    async fn claim_next_returns_none_on_empty_queue() {
        let pool = fresh_pool().await;
        let queue = SyncQueue::new();
        assert!(queue.claim_next(&pool).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn enqueue_then_claim_marks_sending() {
        let pool = fresh_pool().await;
        let queue = SyncQueue::new();
        let action = QueuedAction::new(
            "acc-1".into(),
            "send_message",
            serde_json::json!({"hello": "world"}),
        );
        queue.enqueue(&pool, action.clone()).await.unwrap();

        let claimed = queue.claim_next(&pool).await.unwrap().expect("claimed");
        assert_eq!(claimed.id, action.id);
        assert_eq!(claimed.status, "sending");

        // Second claim returns nothing because the action is already 'sending'.
        assert!(queue.claim_next(&pool).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn reset_sending_never_automatically_retries_an_ambiguous_mail_delivery() {
        let pool = fresh_pool().await;
        let queue = SyncQueue::new();
        let action = QueuedAction::new("acc-1".into(), "send_message", serde_json::json!({}));
        queue.enqueue(&pool, action.clone()).await.unwrap();
        let _ = queue.claim_next(&pool).await.unwrap().unwrap();
        queue
            .mark_submission_started(&pool, &action.id)
            .await
            .unwrap();

        let count = reset_after_restart(&queue, &pool).await.unwrap();
        assert_eq!(count, 1);
        assert!(queue.claim_next(&pool).await.unwrap().is_none());
        let visible = queue.list_mail_outbox(&pool).await.unwrap();
        assert_eq!(visible[0].status, "delivery_unknown");
        assert_eq!(
            visible[0].last_error.as_deref(),
            Some("backend.errors.messageDeliveryStatusUnknown")
        );
        assert!(queue.requeue_mail(&pool, &action.id).await.is_err());
    }

    /// The in-process counterpart of `reset_sending`: the connection broke
    /// after the body went over, so the row is terminal and not claimable.
    #[tokio::test]
    async fn a_send_of_unknown_outcome_is_terminal_and_not_claimed_again() {
        let pool = fresh_pool().await;
        let queue = SyncQueue::new();
        let action = QueuedAction::new("acc-1".into(), ACTION_SEND_MESSAGE, json!({}));
        queue.enqueue(&pool, action.clone()).await.unwrap();
        let _ = queue.claim_next(&pool).await.unwrap().unwrap();
        queue
            .mark_submission_started(&pool, &action.id)
            .await
            .unwrap();

        queue
            .mark_delivery_unknown(&pool, &action.id)
            .await
            .unwrap();

        assert_eq!(status_of(&pool, &action.id).await, "delivery_unknown");
        assert!(queue.claim_next(&pool).await.unwrap().is_none());
        let visible = queue.list_mail_outbox(&pool).await.unwrap();
        assert_eq!(visible[0].status, "delivery_unknown");
        assert_eq!(
            visible[0].last_error.as_deref(),
            Some(DELIVERY_STATUS_UNKNOWN_ERROR_KEY)
        );
        // Same guard as the crash path: a mail that may be out there cannot be
        // silently re-queued. The user acknowledges it or reopens it as a draft.
        assert!(queue.requeue_mail(&pool, &action.id).await.is_err());
    }

    #[tokio::test]
    async fn reset_sending_separates_a_started_transmission_from_an_untouched_one() {
        let pool = fresh_pool().await;
        let queue = SyncQueue::new();
        let untouched = QueuedAction::new("acc-1".into(), ACTION_SEND_MESSAGE, json!({}));
        let started = QueuedAction::new("acc-2".into(), ACTION_SEND_MESSAGE, json!({}));
        queue.enqueue(&pool, untouched.clone()).await.unwrap();
        queue.enqueue(&pool, started.clone()).await.unwrap();
        queue
            .claim_ready(&pool, &untouched.id)
            .await
            .unwrap()
            .unwrap();
        queue
            .claim_ready(&pool, &started.id)
            .await
            .unwrap()
            .unwrap();
        queue
            .mark_submission_started(&pool, &started.id)
            .await
            .unwrap();

        assert_eq!(reset_after_restart(&queue, &pool).await.unwrap(), 2);

        assert_eq!(status_of(&pool, &untouched.id).await, "pending");
        assert_eq!(status_of(&pool, &started.id).await, "delivery_unknown");
    }

    #[tokio::test]
    async fn requeue_mail_reopens_only_the_row_that_never_reached_the_provider() {
        let pool = fresh_pool().await;
        let queue = SyncQueue::new();
        let untouched = QueuedAction::new("acc-1".into(), ACTION_SEND_MESSAGE, json!({}));
        let started = QueuedAction::new("acc-2".into(), ACTION_SEND_MESSAGE, json!({}));
        queue.enqueue(&pool, untouched.clone()).await.unwrap();
        queue.enqueue(&pool, started.clone()).await.unwrap();
        queue
            .claim_ready(&pool, &untouched.id)
            .await
            .unwrap()
            .unwrap();
        queue
            .claim_ready(&pool, &started.id)
            .await
            .unwrap()
            .unwrap();
        queue
            .mark_submission_started(&pool, &started.id)
            .await
            .unwrap();
        reset_after_restart(&queue, &pool).await.unwrap();

        queue.requeue_mail(&pool, &untouched.id).await.unwrap();
        assert_eq!(status_of(&pool, &untouched.id).await, "pending");
        assert!(queue.requeue_mail(&pool, &started.id).await.is_err());
        assert_eq!(status_of(&pool, &started.id).await, "delivery_unknown");
    }

    #[tokio::test]
    async fn discarding_loses_to_the_worker_instead_of_deleting_the_only_record() {
        let pool = fresh_pool().await;
        let queue = SyncQueue::new();
        let action = QueuedAction::new("acc-1".into(), ACTION_SEND_MESSAGE, json!({}));
        queue.enqueue(&pool, action.clone()).await.unwrap();

        // The undo click and the worker's claim land in the same instant.
        queue.claim_ready(&pool, &action.id).await.unwrap().unwrap();

        assert!(!queue.discard_mail_action(&pool, &action.id).await.unwrap());
        assert_eq!(status_of(&pool, &action.id).await, "sending");
    }

    #[tokio::test]
    async fn discarding_removes_a_mail_the_worker_has_not_taken_yet() {
        let pool = fresh_pool().await;
        let queue = SyncQueue::new();
        let action = QueuedAction::new("acc-1".into(), ACTION_SEND_MESSAGE, json!({}));
        queue.enqueue(&pool, action.clone()).await.unwrap();

        assert!(queue.discard_mail_action(&pool, &action.id).await.unwrap());
        assert!(queue.list_mail_outbox(&pool).await.unwrap().is_empty());
        // A second click has nothing left to discard.
        assert!(!queue.discard_mail_action(&pool, &action.id).await.unwrap());
    }

    #[tokio::test]
    async fn discarding_refuses_anything_that_is_not_an_outgoing_message() {
        let pool = fresh_pool().await;
        let queue = SyncQueue::new();
        let mutation = QueuedAction::new("acc-1".into(), "remote_mutation", json!({}));
        queue.enqueue(&pool, mutation.clone()).await.unwrap();

        assert!(!queue
            .discard_mail_action(&pool, &mutation.id)
            .await
            .unwrap());
        assert_eq!(status_of(&pool, &mutation.id).await, "pending");
    }

    #[tokio::test]
    async fn claiming_a_retry_forgets_the_previous_attempts_transmission() {
        let pool = fresh_pool().await;
        let queue = SyncQueue::new();
        let action = QueuedAction::new("acc-1".into(), ACTION_SEND_MESSAGE, json!({}));
        queue.enqueue(&pool, action.clone()).await.unwrap();
        queue.claim_ready(&pool, &action.id).await.unwrap().unwrap();
        queue
            .mark_submission_started(&pool, &action.id)
            .await
            .unwrap();
        queue
            .mark_failed_retry(&pool, &action.id, 1, Utc::now(), "temporary")
            .await
            .unwrap();

        let claimed = queue.claim_ready(&pool, &action.id).await.unwrap().unwrap();

        assert!(claimed.submission_started_at.is_none());
        reset_after_restart(&queue, &pool).await.unwrap();
        assert_eq!(status_of(&pool, &action.id).await, "pending");
    }

    #[tokio::test]
    async fn enqueue_preserves_distinct_created_and_updated_timestamps() {
        let pool = fresh_pool().await;
        let queue = SyncQueue::new();
        let mut action =
            QueuedAction::new("acc-1".into(), ACTION_SEND_MESSAGE, serde_json::json!({}));
        action.created_at = "2026-08-30T09:00:00Z".into();
        action.updated_at = "2026-08-30T09:05:00Z".into();

        queue.enqueue(&pool, action).await.unwrap();

        let visible = queue.list_mail_outbox(&pool).await.unwrap();
        assert_eq!(visible[0].created_at, "2026-08-30T09:00:00Z");
        assert_eq!(visible[0].updated_at, "2026-08-30T09:05:00Z");
    }

    #[tokio::test]
    async fn enqueue_never_reopens_an_in_flight_or_terminal_mail_send() {
        for protected_status in ["sending", "delivered", "delivery_unknown"] {
            let pool = fresh_pool().await;
            let queue = SyncQueue::new();
            let mut original = QueuedAction::new(
                "acc-1".into(),
                ACTION_SEND_MESSAGE,
                serde_json::json!({"version": "original"}),
            );
            original.id = "deterministic-send-id".into();
            original.status = protected_status.into();
            original.last_error = Some("original warning".into());
            queue.enqueue(&pool, original).await.unwrap();

            let mut repeated = QueuedAction::new(
                "acc-1".into(),
                ACTION_SEND_MESSAGE,
                serde_json::json!({"version": "repeated"}),
            );
            repeated.id = "deterministic-send-id".into();
            queue.enqueue(&pool, repeated).await.unwrap();

            let stored = queue.list_mail_outbox(&pool).await.unwrap();
            assert_eq!(stored[0].status, protected_status);
            assert_eq!(stored[0].payload["version"], "original");
            assert_eq!(stored[0].last_error.as_deref(), Some("original warning"));
            assert!(queue.claim_next(&pool).await.unwrap().is_none());
        }
    }

    #[tokio::test]
    async fn enqueue_does_not_reset_an_in_flight_remote_mutation() {
        let pool = fresh_pool().await;
        let queue = SyncQueue::new();
        let mut original = QueuedAction::new(
            "acc-1".into(),
            "remote_mutation",
            serde_json::json!({"kind": "blocked_move", "version": "original"}),
        );
        original.id = "deterministic-blocked-move-id".into();
        queue.enqueue(&pool, original).await.unwrap();
        let _ = queue.claim_next(&pool).await.unwrap().expect("claimed");

        let mut repeated = QueuedAction::new(
            "acc-1".into(),
            "remote_mutation",
            serde_json::json!({"kind": "blocked_move", "version": "repeated"}),
        );
        repeated.id = "deterministic-blocked-move-id".into();
        queue.enqueue(&pool, repeated).await.unwrap();

        let stored: (String, String) = sqlx::query_as(
            "SELECT status, payload FROM sync_queue WHERE id = 'deterministic-blocked-move-id'",
        )
        .fetch_one(&pool)
        .await
        .expect("stored mutation");
        assert_eq!(stored.0, "sending");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&stored.1)
                .expect("payload")
                .get("version"),
            Some(&serde_json::Value::String("original".into()))
        );
    }

    #[tokio::test]
    async fn reenqueue_of_blocked_move_preserves_crash_state_for_pending_and_failed() {
        for original_status in ["pending", "failed"] {
            let pool = fresh_pool().await;
            let queue = SyncQueue::new();
            let mut original = QueuedAction::new(
                "acc-1".into(),
                "remote_mutation",
                serde_json::json!({
                    "kind": "blocked_move",
                    "blocked_thread_id": "thread-1",
                    "refs": [{"remote_id": "old", "source_folder": "INBOX"}],
                    "target_remote_folder_id": "Spam",
                    "provider_move_started": true,
                    "provider_move_completed": true,
                    "source_uidvalidity_before": {"INBOX": 41},
                    "version": "original"
                }),
            );
            original.id = "stable-blocked-move-id".into();
            original.status = original_status.into();
            original.last_error = Some("old failure".into());
            queue.enqueue(&pool, original).await.unwrap();

            let mut repeated = QueuedAction::new(
                "acc-1".into(),
                "remote_mutation",
                serde_json::json!({
                    "kind": "blocked_move",
                    "blocked_thread_id": "thread-1",
                    "refs": [{"remote_id": "old", "source_folder": "INBOX"}],
                    "target_remote_folder_id": "Spam",
                    "provider_move_started": false,
                    "provider_move_completed": false,
                    "source_uidvalidity_before": null,
                    "version": "repeated"
                }),
            );
            repeated.id = "stable-blocked-move-id".into();
            queue.enqueue(&pool, repeated).await.unwrap();

            let stored: (String, Option<String>, String) = sqlx::query_as(
                "SELECT status, last_error, payload
                 FROM sync_queue WHERE id = 'stable-blocked-move-id'",
            )
            .fetch_one(&pool)
            .await
            .expect("stored blocked move");
            assert_eq!(stored.0, "pending");
            assert_eq!(stored.1, None);
            let payload: serde_json::Value =
                serde_json::from_str(&stored.2).expect("blocked move payload");
            assert_eq!(payload["version"], "repeated");
            assert_eq!(payload["provider_move_started"], true);
            assert_eq!(payload["provider_move_completed"], true);
            assert_eq!(payload["source_uidvalidity_before"]["INBOX"], 41);
            assert_eq!(payload["refs"][0]["remote_id"], "old");
        }
    }

    #[tokio::test]
    async fn reenqueue_of_a_changed_blocked_ref_does_not_inherit_crash_state() {
        let pool = fresh_pool().await;
        let queue = SyncQueue::new();
        let mut original = QueuedAction::new(
            "acc-1".into(),
            "remote_mutation",
            serde_json::json!({
                "kind": "blocked_move",
                "blocked_thread_id": "thread-1",
                "refs": [{"remote_id": "old", "source_folder": "INBOX"}],
                "target_remote_folder_id": "Spam",
                "provider_move_started": true,
                "provider_move_completed": true,
                "source_uidvalidity_before": {"INBOX": 41}
            }),
        );
        original.id = "reused-id".into();
        original.status = "failed".into();
        queue.enqueue(&pool, original).await.unwrap();

        let mut changed = QueuedAction::new(
            "acc-1".into(),
            "remote_mutation",
            serde_json::json!({
                "kind": "blocked_move",
                "blocked_thread_id": "thread-1",
                "refs": [{"remote_id": "new", "source_folder": "INBOX"}],
                "target_remote_folder_id": "Spam",
                "provider_move_started": false,
                "provider_move_completed": false,
                "source_uidvalidity_before": null
            }),
        );
        changed.id = "reused-id".into();
        queue.enqueue(&pool, changed).await.unwrap();

        let payload: String =
            sqlx::query_scalar("SELECT payload FROM sync_queue WHERE id = 'reused-id'")
                .fetch_one(&pool)
                .await
                .unwrap();
        let payload: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(payload["refs"][0]["remote_id"], "new");
        assert_eq!(payload["provider_move_started"], false);
        assert_eq!(payload["provider_move_completed"], false);
        assert!(payload["source_uidvalidity_before"].is_null());
    }

    #[tokio::test]
    async fn immutable_blocked_refs_queue_as_distinct_operations_while_one_is_sending() {
        let pool = fresh_pool().await;
        let queue = SyncQueue::new();
        let old_ref = crate::providers::traits::MessageRef {
            remote_id: "old".into(),
            source_folder: Some("INBOX".into()),
            source_uidvalidity: None,
        };
        let new_ref = crate::providers::traits::MessageRef {
            remote_id: "new".into(),
            source_folder: Some("INBOX".into()),
            source_uidvalidity: None,
        };
        let payload = |message_ref: crate::providers::traits::MessageRef| {
            serde_json::json!({
                "kind": "blocked_move",
                "blocked_thread_id": "thread-1",
                "refs": [message_ref],
                "target_remote_folder_id": "Spam",
                "provider_move_started": false,
                "provider_move_completed": false,
                "source_uidvalidity_before": null
            })
        };
        let mut old_action =
            QueuedAction::new("acc-1".into(), "remote_mutation", payload(old_ref.clone()));
        old_action.id = crate::sync::remote_mutations::stable_blocked_move_id(
            "thread-1", "acc-1", &old_ref, "Spam",
        );
        queue.enqueue(&pool, old_action.clone()).await.unwrap();
        queue
            .claim_next(&pool)
            .await
            .unwrap()
            .expect("old ref claimed");

        let mut new_action =
            QueuedAction::new("acc-1".into(), "remote_mutation", payload(new_ref.clone()));
        new_action.id = crate::sync::remote_mutations::stable_blocked_move_id(
            "thread-1", "acc-1", &new_ref, "Spam",
        );
        queue.enqueue(&pool, new_action.clone()).await.unwrap();

        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT id, status FROM sync_queue ORDER BY id")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.contains(&(old_action.id, "sending".into())));
        assert!(rows.contains(&(new_action.id, "pending".into())));
    }

    #[tokio::test]
    async fn same_imap_uid_in_different_generations_is_a_distinct_blocked_operation() {
        let pool = fresh_pool().await;
        let queue = SyncQueue::new();
        let payload = |generation| {
            serde_json::json!({
                "kind": "blocked_move",
                "blocked_thread_id": "thread-1",
                "refs": [{
                    "remote_id": "42",
                    "source_folder": "INBOX",
                    "source_uidvalidity": generation
                }],
                "target_remote_folder_id": "Spam",
                "provider_move_started": false,
                "provider_move_completed": false,
                "source_uidvalidity_before": {"INBOX": generation}
            })
        };
        let mut first = QueuedAction::new("acc-1".into(), "remote_mutation", payload(41));
        first.id = crate::sync::remote_mutations::stable_blocked_move_id(
            "thread-1",
            "acc-1",
            &crate::providers::traits::MessageRef {
                remote_id: "42".into(),
                source_folder: Some("INBOX".into()),
                source_uidvalidity: Some(41),
            },
            "Spam",
        );
        queue.enqueue(&pool, first.clone()).await.unwrap();

        let mut second = QueuedAction::new("acc-1".into(), "remote_mutation", payload(42));
        second.id = crate::sync::remote_mutations::stable_blocked_move_id(
            "thread-1",
            "acc-1",
            &crate::providers::traits::MessageRef {
                remote_id: "42".into(),
                source_folder: Some("INBOX".into()),
                source_uidvalidity: Some(42),
            },
            "Spam",
        );
        queue.enqueue(&pool, second.clone()).await.unwrap();

        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT id, status FROM sync_queue ORDER BY id")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.contains(&(first.id, "pending".into())));
        assert!(rows.contains(&(second.id, "pending".into())));
    }

    #[tokio::test]
    async fn reset_sending_marks_incomplete_blocked_moves_as_started_before_retry() {
        let pool = fresh_pool().await;
        let queue = SyncQueue::new();
        let mut legacy = QueuedAction::new(
            "acc-1".into(),
            "remote_mutation",
            serde_json::json!({
                "kind": "blocked_move",
                "blocked_thread_id": "thread-1",
                "refs": [{"remote_id": "42", "source_folder": "INBOX"}],
                "target_remote_folder_id": "Spam"
            }),
        );
        legacy.id = "legacy-blocked-move".into();
        queue.enqueue(&pool, legacy).await.unwrap();
        queue.claim_next(&pool).await.unwrap().expect("claimed");

        assert_eq!(reset_after_restart(&queue, &pool).await.unwrap(), 1);
        let recovered = queue.claim_next(&pool).await.unwrap().expect("recovered");
        assert_eq!(recovered.status, "sending");
        assert_eq!(recovered.payload["provider_move_started"], true);
        assert!(recovered.payload.get("provider_move_completed").is_none());
        let _: crate::sync::remote_mutations::RemoteMutationPayload =
            serde_json::from_value(recovered.payload).expect("legacy payload upgrades");
    }

    #[tokio::test]
    async fn reset_sending_keeps_remote_mutations_retryable() {
        let pool = fresh_pool().await;
        let queue = SyncQueue::new();
        let action = QueuedAction::new("acc-1".into(), "remote_mutation", serde_json::json!({}));
        queue.enqueue(&pool, action.clone()).await.unwrap();
        let _ = queue.claim_next(&pool).await.unwrap().unwrap();

        assert_eq!(reset_after_restart(&queue, &pool).await.unwrap(), 1);
        let reclaimed = queue.claim_next(&pool).await.unwrap().expect("reclaimed");
        assert_eq!(reclaimed.id, action.id);
    }

    #[tokio::test]
    async fn mark_completed_deletes_action() {
        let pool = fresh_pool().await;
        let queue = SyncQueue::new();
        let action = QueuedAction::new("acc-1".into(), "send_message", serde_json::json!({}));
        queue.enqueue(&pool, action.clone()).await.unwrap();
        let claimed = queue.claim_next(&pool).await.unwrap().unwrap();
        queue.mark_completed(&pool, &claimed.id).await.unwrap();
        assert!(queue.list_visible(&pool).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn mail_outbox_hides_internal_remote_mutations() {
        let pool = fresh_pool().await;
        let queue = SyncQueue::new();
        let mail = QueuedAction::new(
            "acc-1".into(),
            "send_message",
            serde_json::json!({"input": {"subject": "Hello"}}),
        );
        let mutation = QueuedAction::new(
            "acc-1".into(),
            "remote_mutation",
            serde_json::json!({"kind": "flags"}),
        );
        queue.enqueue(&pool, mail.clone()).await.unwrap();
        queue.enqueue(&pool, mutation).await.unwrap();

        let visible = queue.list_mail_outbox(&pool).await.unwrap();
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].id, mail.id);
        assert_eq!(visible[0].action_type, "send_message");
    }

    #[tokio::test]
    async fn mark_failed_permanent_keeps_actions_visible_as_failed() {
        let pool = fresh_pool().await;
        let queue = SyncQueue::new();
        let mail = QueuedAction::new("acc-1".into(), "send_message", serde_json::json!({}));
        let mutation = QueuedAction::new(
            "acc-1".into(),
            "remote_mutation",
            serde_json::json!({"kind": "flags"}),
        );
        queue.enqueue(&pool, mail.clone()).await.unwrap();
        queue.enqueue(&pool, mutation.clone()).await.unwrap();
        queue
            .mark_failed_permanent(&pool, &mail.id, "mail failed")
            .await
            .unwrap();
        queue
            .mark_failed_permanent(&pool, &mutation.id, "sync failed")
            .await
            .unwrap();

        let mut failed = queue.list_failed(&pool).await.unwrap();
        failed.sort_by(|left, right| left.action_type.cmp(&right.action_type));
        assert_eq!(failed.len(), 2);
        assert_eq!(failed[0].status, "failed");
        assert_eq!(failed[1].status, "failed");
        assert_eq!(failed[0].action_type, "remote_mutation");
        assert_eq!(failed[1].action_type, "send_message");
    }

    #[tokio::test]
    async fn manual_sync_retry_requeues_drafts_and_mutations_but_not_mail() {
        let pool = fresh_pool().await;
        let queue = SyncQueue::new();
        let draft = QueuedAction::new("acc-1".into(), ACTION_SYNC_DRAFT, json!({}));
        let mutation = QueuedAction::new(
            "acc-1".into(),
            crate::sync::remote_mutations::ACTION_REMOTE_MUTATION,
            json!({}),
        );
        let failed_mail = QueuedAction::new("acc-1".into(), ACTION_SEND_MESSAGE, json!({}));
        let unknown_mail = QueuedAction::new("acc-1".into(), ACTION_SEND_MESSAGE, json!({}));
        for action in [&draft, &mutation, &failed_mail, &unknown_mail] {
            queue.enqueue(&pool, (*action).clone()).await.unwrap();
        }
        for action in [&draft, &mutation, &failed_mail] {
            queue
                .mark_failed_permanent(&pool, &action.id, "provider error")
                .await
                .unwrap();
        }
        queue
            .mark_delivery_unknown(&pool, &unknown_mail.id)
            .await
            .unwrap();

        assert_eq!(
            queue.requeue_failed_remote_mutations(&pool).await.unwrap(),
            2
        );
        assert_eq!(status_of(&pool, &draft.id).await, "pending");
        assert_eq!(status_of(&pool, &mutation.id).await, "pending");
        assert_eq!(status_of(&pool, &failed_mail.id).await, "failed");
        assert_eq!(status_of(&pool, &unknown_mail.id).await, "delivery_unknown");
    }

    #[tokio::test]
    async fn inspecting_ready_work_does_not_publish_sending_before_claim() {
        let pool = fresh_pool().await;
        let queue = SyncQueue::new();
        let action = QueuedAction::new(
            "acc-1".into(),
            "remote_mutation",
            serde_json::json!({"kind": "workflow_move"}),
        );
        queue.enqueue(&pool, action.clone()).await.unwrap();

        let ready = queue
            .peek_next_ready(&pool)
            .await
            .unwrap()
            .expect("ready action");
        assert_eq!(ready.id, action.id);
        assert_eq!(ready.status, "pending");

        let visible = queue.list_visible(&pool).await.unwrap();
        assert_eq!(visible[0].status, "pending");

        let claimed = queue
            .claim_ready(&pool, &ready.id)
            .await
            .unwrap()
            .expect("claimed action");
        assert_eq!(claimed.status, "sending");
    }

    #[tokio::test]
    async fn user_retry_rejects_internal_and_in_flight_actions() {
        let pool = fresh_pool().await;
        let queue = SyncQueue::new();
        let mutation = QueuedAction::new(
            "acc-1".into(),
            "remote_mutation",
            serde_json::json!({"kind": "flags"}),
        );
        queue.enqueue(&pool, mutation.clone()).await.unwrap();
        let error = queue
            .requeue_mail(&pool, &mutation.id)
            .await
            .expect_err("internal action must not be user-requeued");
        assert!(error.contains("nicht erneut"));

        let mail = QueuedAction::new("acc-1".into(), "send_message", serde_json::json!({}));
        queue.enqueue(&pool, mail.clone()).await.unwrap();
        queue.claim_ready(&pool, &mail.id).await.unwrap().unwrap();
        let error = queue
            .requeue_mail(&pool, &mail.id)
            .await
            .expect_err("in-flight mail must not be user-requeued");
        assert!(error.contains("nicht erneut"));
    }

    #[tokio::test]
    async fn delivered_mail_with_local_error_stays_visible_and_cannot_be_requeued() {
        let pool = fresh_pool().await;
        let queue = SyncQueue::new();
        let mail = QueuedAction::new("acc-1".into(), "send_message", serde_json::json!({}));
        queue.enqueue(&pool, mail.clone()).await.unwrap();
        queue.claim_ready(&pool, &mail.id).await.unwrap().unwrap();
        queue
            .mark_delivered_with_error(&pool, &mail.id, "local persistence failed")
            .await
            .unwrap();

        assert!(queue.claim_next(&pool).await.unwrap().is_none());
        let visible = queue.list_mail_outbox(&pool).await.unwrap();
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].status, "delivered");
        assert_eq!(
            queue.requeue_mail(&pool, &mail.id).await.unwrap_err(),
            "Diese Mail kann gerade nicht erneut eingereiht werden."
        );
    }

    #[tokio::test]
    async fn mark_failed_retry_reschedules_with_next_retry_at() {
        let pool = fresh_pool().await;
        let queue = SyncQueue::new();
        let action = QueuedAction::new("acc-1".into(), "send_message", serde_json::json!({}));
        queue.enqueue(&pool, action.clone()).await.unwrap();
        let claimed = queue.claim_next(&pool).await.unwrap().unwrap();
        let next_at = chrono::Utc::now() + chrono::Duration::seconds(60);
        queue
            .mark_failed_retry(&pool, &claimed.id, 1, next_at, "boom")
            .await
            .unwrap();
        // Immediate claim should return nothing — retry is in the future.
        assert!(queue.claim_next(&pool).await.unwrap().is_none());
        let visible = queue.list_visible(&pool).await.unwrap();
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].status, "pending");
        assert!(visible[0].next_retry_at.is_some());
    }

    #[tokio::test]
    async fn scheduled_action_not_claimed_until_due() {
        let pool = fresh_pool().await;
        let queue = SyncQueue::new();
        let mut action = QueuedAction::new("acc-1".into(), "send_message", serde_json::json!({}));
        action = action.scheduled(chrono::Utc::now() + chrono::Duration::hours(1));
        queue.enqueue(&pool, action).await.unwrap();
        assert!(queue.claim_next(&pool).await.unwrap().is_none());

        // Force-update the row so it becomes due immediately.
        sqlx::query("UPDATE sync_queue SET scheduled_for = ?")
            .bind((chrono::Utc::now() - chrono::Duration::seconds(10)).to_rfc3339())
            .execute(&pool)
            .await
            .unwrap();
        assert!(queue.claim_next(&pool).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn undo_transaction_and_worker_claim_cannot_both_win() {
        let directory = tempfile::tempdir().expect("temp directory");
        let options = SqliteConnectOptions::new()
            .filename(directory.path().join("queue-race.sqlite"))
            .busy_timeout(std::time::Duration::from_secs(2))
            .journal_mode(SqliteJournalMode::Wal)
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect_with(options)
            .await
            .expect("file pool");
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .expect("migrations");
        let queue = SyncQueue::new();
        let action = QueuedAction::new("acc-1".into(), "remote_mutation", serde_json::json!({}))
            .scheduled(chrono::Utc::now() - chrono::Duration::seconds(1));
        queue.enqueue(&pool, action.clone()).await.expect("enqueue");

        // This is the same reservation transition_thread uses before checking and
        // cancelling the scheduled workflow row. The worker can start, but cannot
        // claim from a snapshot that predates the committed delete.
        let mut undo = pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .expect("undo reservation");
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let worker_pool = pool.clone();
        let worker_queue = queue.clone();
        let worker = tokio::spawn(async move {
            let _ = started_tx.send(());
            worker_queue.claim_next(&worker_pool).await
        });
        started_rx.await.expect("worker started");
        tokio::task::yield_now().await;
        let removed = sqlx::query("DELETE FROM sync_queue WHERE id = ? AND status = 'scheduled'")
            .bind(&action.id)
            .execute(&mut *undo)
            .await
            .expect("cancel scheduled row")
            .rows_affected();
        assert_eq!(removed, 1);
        undo.commit().await.expect("commit undo");

        match worker.await.expect("worker task") {
            Ok(None) => {}
            // SQLITE_BUSY/SQLITE_BUSY_SNAPSHOT is also a safe losing outcome. The
            // production worker retries claim_next; crucially it did not return an
            // action from the pre-Undo snapshot.
            Err(error) if error.contains("database is locked") => {}
            other => panic!("worker must not claim the cancelled row: {other:?}"),
        }
    }

    #[tokio::test]
    async fn startup_requeues_only_known_transient_failures() {
        let pool = fresh_pool().await;
        let queue = SyncQueue::new();
        for (id, status, error) in [
            (
                "offline-send",
                "failed",
                r#"{"key":"backend.errors.queueActionFailed","params":{"message":"DNS lookup failed"}}"#,
            ),
            ("offline-move", "failed", "connection reset"),
            ("rejected", "failed", "invalid recipient"),
            ("revoked", "failed", "invalid_grant"),
            ("unknown", "delivery_unknown", "connection reset"),
            ("delivered", "delivered", "timeout"),
            ("ambiguous", "failed", "gmail send failed with status 503"),
        ] {
            let mut action = QueuedAction::new(
                "acc-1".into(),
                if id == "offline-move" {
                    "remote_mutation"
                } else {
                    "send_message"
                },
                serde_json::json!({}),
            );
            action.id = id.into();
            queue.enqueue(&pool, action).await.unwrap();
            sqlx::query(
                "UPDATE sync_queue SET status = ?, last_error = ?, retry_count = 6 WHERE id = ?",
            )
            .bind(status)
            .bind(error)
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
        }
        assert_eq!(queue.requeue_failed_transient(&pool).await.unwrap(), 2);
        assert_eq!(queue.requeue_failed_transient(&pool).await.unwrap(), 0);
        let statuses: Vec<(String, String)> = sqlx::query_as("SELECT id, status FROM sync_queue")
            .fetch_all(&pool)
            .await
            .unwrap();
        for (id, status) in statuses {
            assert_eq!(
                status,
                match id.as_str() {
                    "offline-send" | "offline-move" => "pending",
                    "unknown" => "delivery_unknown",
                    "delivered" => "delivered",
                    _ => "failed",
                }
            );
        }
    }

    #[test]
    fn backoff_progression_is_monotonic() {
        let values: Vec<i64> = (1..=12).map(backoff_seconds).collect();
        for window in values.windows(2) {
            assert!(window[1] >= window[0], "backoff non-monotonic: {values:?}");
        }
        // Initial step should be short (~10s) and final step capped at 2h.
        assert_eq!(values[0], 10);
        assert_eq!(*values.last().unwrap(), 2 * 60 * 60);
    }

    async fn enqueue_notification_move(pool: &SqlitePool, queue: &SyncQueue) -> String {
        let mut action = QueuedAction::new(
            "acc-1".into(),
            "remote_mutation",
            json!({"kind": "workflow_move", "thread_id": "t1"}),
        );
        action.id = "notification:digest:1".into();
        queue.enqueue(pool, action.clone()).await.unwrap();
        action.id
    }

    async fn age_row(pool: &SqlitePool, id: &str, millis: i64) {
        sqlx::query("UPDATE sync_queue SET updated_at_ms = ? WHERE id = ?")
            .bind(chrono::Utc::now().timestamp_millis() - millis)
            .bind(id)
            .execute(pool)
            .await
            .unwrap();
    }

    async fn head_ids(queue: &SyncQueue, pool: &SqlitePool) -> Vec<String> {
        queue
            .ready_account_heads(pool)
            .await
            .unwrap()
            .into_iter()
            .map(|action| action.id)
            .collect()
    }

    #[tokio::test]
    async fn the_main_worker_adopts_an_abandoned_notification_row() {
        let pool = fresh_pool().await;
        let queue = SyncQueue::new();
        let id = enqueue_notification_move(&pool, &queue).await;
        assert!(head_ids(&queue, &pool).await.is_empty());
        assert!(queue.peek_next_ready(&pool).await.unwrap().is_none());

        age_row(&pool, &id, NOTIFICATION_ROW_LEASE_MS + 1_000).await;
        assert_eq!(head_ids(&queue, &pool).await, vec![id.clone()]);
        assert_eq!(
            queue.peek_next_ready(&pool).await.unwrap().map(|a| a.id),
            Some(id)
        );
        assert!(queue.next_wakeup(&pool).await.is_some());
    }

    #[tokio::test]
    async fn retrying_a_failed_notification_move_hands_it_to_the_main_worker() {
        let pool = fresh_pool().await;
        let queue = SyncQueue::new();
        let id = enqueue_notification_move(&pool, &queue).await;
        queue
            .mark_failed_permanent(&pool, &id, "imapUidValidityChanged")
            .await
            .unwrap();
        assert!(queue.requeue_failed_move(&pool, &id).await.unwrap());
        assert_eq!(head_ids(&queue, &pool).await, vec![id]);
    }

    #[tokio::test]
    async fn only_a_stale_sending_notification_row_is_reset() {
        let pool = fresh_pool().await;
        let queue = SyncQueue::new();
        let id = enqueue_notification_move(&pool, &queue).await;
        queue
            .claim_ready(&pool, &id)
            .await
            .unwrap()
            .expect("claimed");
        let ids = vec![id.clone()];

        // Young: possibly running in the other process right now.
        assert_eq!(queue.reset_stale_sending_ids(&pool, &ids).await.unwrap(), 0);
        sqlx::query("UPDATE sync_queue SET sending_session = 'other-process'")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(queue.reset_sending(&pool).await.unwrap(), 0);

        age_row(&pool, &id, NOTIFICATION_ROW_LEASE_MS + 1_000).await;
        assert_eq!(queue.reset_stale_sending_ids(&pool, &ids).await.unwrap(), 1);
        queue
            .claim_ready(&pool, &id)
            .await
            .unwrap()
            .expect("claimed");
        sqlx::query("UPDATE sync_queue SET sending_session = 'other-process'")
            .execute(&pool)
            .await
            .unwrap();
        age_row(&pool, &id, NOTIFICATION_ROW_LEASE_MS + 1_000).await;
        assert_eq!(queue.reset_sending(&pool).await.unwrap(), 1);
    }
}
