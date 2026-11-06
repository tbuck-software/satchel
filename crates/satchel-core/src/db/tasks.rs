//! Persistence for tasks and task lists.
//!
//! Local lists are plain rows. Remote lists mirror a provider collection; a
//! local change to one of their tasks sets `pending_op` in the same statement,
//! and the sync engine (`crate::tasks::sync`) pushes it and clears the mark.
//! Rows marked for deletion stay hidden until the server confirmed it.
use serde::Serialize;
use sqlx::{FromRow, SqlitePool};
use thiserror::Error;

use crate::tasks::recurrence::{self, Recurrence};

/// The provider id of lists that live only on this device.
pub const LOCAL_PROVIDER: &str = "local";
/// The id of the list that exists without any account.
pub const DEFAULT_LOCAL_LIST_ID: &str = "tasks-local-default";

#[derive(Debug, Clone, Serialize, FromRow)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "core/db/tasks/"))]
pub struct TaskListRecord {
    pub id: String,
    /// None for a local list.
    pub account_id: Option<String>,
    /// `local`, `caldav`; later `google`, `microsoft`.
    pub provider: String,
    /// Empty for the default local list, which the frontend names.
    pub name: String,
    pub color: Option<String>,
    pub read_only: bool,
    /// The server no longer lists it; it stays while it holds unsent work.
    pub gone: bool,
    pub last_synced_at: Option<String>,
    pub open_count: i64,
}

#[derive(Debug, Clone, Serialize, FromRow, PartialEq, Eq)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "core/db/tasks/"))]
pub struct TaskRecord {
    pub id: String,
    pub list_id: String,
    pub title: String,
    pub notes: Option<String>,
    /// `YYYY-MM-DD` or RFC 3339 UTC.
    pub due_at: Option<String>,
    /// RFC 3339 UTC.
    pub reminder_at: Option<String>,
    /// 0 none, 1 highest … 9 lowest (iCalendar).
    pub priority: i64,
    pub completed_at: Option<String>,
    /// `simple`: completing moves the due date to the next occurrence;
    /// `complex`: completing ends the series, so the UI asks first.
    pub recurrence: Option<String>,
    /// The thread the task was made from, re-found by Message-ID when the
    /// stored id no longer exists.
    pub thread_id: Option<String>,
    pub message_id: Option<String>,
    /// A change that has not reached the server yet.
    pub pending: bool,
    pub created_at: String,
    pub updated_at: String,
}

/// What the editor writes. The repository owns ids, timestamps and sync state.
#[derive(Debug, Clone, Default)]
pub struct TaskWrite {
    pub title: String,
    pub notes: Option<String>,
    pub due_at: Option<String>,
    pub reminder_at: Option<String>,
    pub priority: u8,
    pub completed_at: Option<String>,
}

/// A pending change as the sync engine pushes it.
#[derive(Debug, Clone, FromRow)]
pub struct PendingTask {
    pub id: String,
    pub uid: String,
    pub title: String,
    pub notes: Option<String>,
    pub due_at: Option<String>,
    pub reminder_at: Option<String>,
    pub priority: i64,
    pub completed_at: Option<String>,
    pub message_id: Option<String>,
    pub start_at: Option<String>,
    pub rrule: Option<String>,
    pub remote_id: Option<String>,
    pub remote_version: Option<String>,
    pub remote_payload: Option<String>,
    pub pending_op: String,
    pub updated_at: String,
}

/// A remote list as a provider describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteListWrite<'a> {
    pub account_id: &'a str,
    pub provider: &'a str,
    pub remote_id: &'a str,
    pub name: &'a str,
    pub color: Option<&'a str>,
    /// None when the provider does not say; the list keeps what it learnt.
    pub read_only: Option<bool>,
    /// The collection also holds events.
    pub mixed: bool,
}

/// A remote task as the server holds it now.
#[derive(Debug, Clone)]
pub struct RemoteTaskWrite<'a> {
    pub remote_id: &'a str,
    pub remote_version: Option<&'a str>,
    pub remote_payload: &'a str,
    pub uid: &'a str,
    pub fields: &'a TaskWrite,
    pub message_id: Option<&'a str>,
    pub start_at: Option<&'a str>,
    pub rrule: Option<&'a str>,
    /// `recurrence::Recurrence::stored()` of the resource.
    pub recurrence: Option<&'a str>,
}

/// What became of the tasks of lists or resources the server dropped.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Removal {
    pub removed: u64,
    /// Pending edits of server-deleted tasks, kept as local tasks.
    pub kept_local: u64,
}

#[derive(Debug, Error)]
pub enum TaskError {
    #[error("task was not found")]
    TaskNotFound,
    #[error("task list was not found")]
    ListNotFound,
    #[error("task list is read-only")]
    ListReadOnly,
    #[error("only local task lists can be changed here")]
    ListNotLocal,
    #[error("task changed elsewhere")]
    TaskChanged,
    #[error("task persistence failed: {0}")]
    Persistence(#[from] sqlx::Error),
}

// The Message-ID lookups name the accounts so `idx_messages_account_msgid`
// (account_id, message_id_header) serves them; without the account term they
// would scan every message for each task whose thread is gone.
const TASK_SELECT: &str = "SELECT t.id, t.list_id, t.title, t.notes, t.due_at, t.reminder_at,
        t.priority, t.completed_at, t.recurrence,
        COALESCE(
            (SELECT th.id FROM threads th WHERE th.id = t.thread_id),
            (SELECT m.thread_id FROM messages m
              WHERE t.message_id IS NOT NULL
                AND m.account_id IN (SELECT id FROM accounts)
                AND m.message_id_header = t.message_id
                AND m.thread_id IS NOT NULL
              LIMIT 1),
            t.thread_id
        ) AS thread_id,
        t.message_id, t.pending_op IS NOT NULL AS pending, t.created_at, t.updated_at
     FROM tasks t";

const PENDING_SELECT: &str = "SELECT id, uid, title, notes, due_at, reminder_at, priority,
        completed_at, message_id, start_at, rrule, remote_id, remote_version, remote_payload,
        pending_op, updated_at
     FROM tasks";

/// Reminders older than this when first seen (an import, days with the app
/// closed) are marked shown without a notification: a pile of stale alarms
/// is noise, not a reminder.
const REMINDER_GRACE_HOURS: i64 = 24;

/// How long a task whose push failed waits before the next try.
fn push_backoff(failures: i64) -> chrono::Duration {
    chrono::Duration::minutes(1_i64 << failures.clamp(0, 9)).min(chrono::Duration::hours(6))
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn stable_id(prefix: &str, value: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = crate::utils::hex::lower_hex(&Sha256::digest(value.as_bytes()));
    format!("{prefix}-{}", &digest[..24])
}

/// The list id a remote collection keeps across syncs.
pub fn remote_list_id(account_id: &str, provider: &str, remote_id: &str) -> String {
    stable_id("tl", &format!("{account_id}\0{provider}\0{remote_id}"))
}

pub struct TaskRepository<'a> {
    pool: &'a SqlitePool,
}

#[derive(Debug, FromRow)]
struct ListState {
    provider: String,
    read_only: bool,
    gone: bool,
}

#[derive(Debug, FromRow)]
struct TaskState {
    list_id: String,
    pending_op: Option<String>,
    remote_id: Option<String>,
    due_at: Option<String>,
    start_at: Option<String>,
    reminder_at: Option<String>,
    completed_at: Option<String>,
    rrule: Option<String>,
    recurrence: Option<String>,
    updated_at: String,
}

/// The stored fields a completion writes: for a simple repeating task the
/// next occurrence and an open task, otherwise the completion as asked.
struct Completion {
    due_at: Option<String>,
    start_at: Option<String>,
    reminder_at: Option<String>,
    rrule: Option<String>,
    completed_at: Option<String>,
}

fn plan_completion(
    state: &TaskState,
    due_at: Option<&str>,
    reminder_at: Option<&str>,
    completed_at: Option<&str>,
) -> Completion {
    let unchanged = Completion {
        due_at: due_at.map(str::to_string),
        start_at: state.start_at.clone(),
        reminder_at: reminder_at.map(str::to_string),
        rrule: state.rrule.clone(),
        completed_at: completed_at.map(str::to_string),
    };
    let completing = completed_at.is_some() && state.completed_at.is_none();
    if !completing || state.recurrence.as_deref() != Some(recurrence::SIMPLE) {
        return unchanged;
    }
    let (Some(rrule), Some(due)) = (state.rrule.as_deref(), due_at) else {
        return unchanged;
    };
    let Recurrence::Simple(rule) = recurrence::classify(Some(rrule), false, Some(due)) else {
        return unchanged;
    };
    match recurrence::advance(&rule, rrule, due, state.start_at.as_deref(), reminder_at) {
        Some(next) => Completion {
            due_at: Some(next.due_at),
            start_at: next.start_at,
            reminder_at: next.reminder_at,
            rrule: Some(next.rrule),
            completed_at: None,
        },
        // The last occurrence: the series ends with it.
        None => unchanged,
    }
}

impl<'a> TaskRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    /// The default local list exists as long as no local list does: tasks
    /// need somewhere to go before any account is set up.
    pub async fn ensure_default_list(&self) -> Result<(), TaskError> {
        let at = now();
        sqlx::query(
            "INSERT INTO task_lists (id, account_id, provider, remote_id, name, created_at, updated_at)
             SELECT ?, NULL, ?, NULL, '', ?, ?
             WHERE NOT EXISTS (SELECT 1 FROM task_lists WHERE provider = ?)",
        )
        .bind(DEFAULT_LOCAL_LIST_ID)
        .bind(LOCAL_PROVIDER)
        .bind(&at)
        .bind(&at)
        .bind(LOCAL_PROVIDER)
        .execute(self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_lists(&self) -> Result<Vec<TaskListRecord>, TaskError> {
        sqlx::query_as::<_, TaskListRecord>(
            "SELECT l.id, l.account_id, l.provider, l.name, l.color, l.read_only, l.gone,
                    l.last_synced_at,
                    (SELECT COUNT(*) FROM tasks t
                      WHERE t.list_id = l.id AND t.completed_at IS NULL
                        AND COALESCE(t.pending_op, '') <> 'delete') AS open_count
             FROM task_lists l
             LEFT JOIN accounts a ON a.id = l.account_id
             -- A calendar that also holds events shows as a task list once it
             -- holds a task, so an account does not sprout a list per calendar.
             WHERE NOT (l.mixed = 1 AND NOT EXISTS (SELECT 1 FROM tasks t WHERE t.list_id = l.id))
             ORDER BY l.account_id IS NOT NULL, a.created_at, l.name COLLATE NOCASE, l.id",
        )
        .fetch_all(self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn create_local_list(&self, name: &str) -> Result<TaskListRecord, TaskError> {
        let id = format!("tl-{}", uuid::Uuid::new_v4());
        let at = now();
        sqlx::query(
            "INSERT INTO task_lists (id, account_id, provider, remote_id, name, created_at, updated_at)
             VALUES (?, NULL, ?, NULL, ?, ?, ?)",
        )
        .bind(&id)
        .bind(LOCAL_PROVIDER)
        .bind(name)
        .bind(&at)
        .bind(&at)
        .execute(self.pool)
        .await?;
        self.list(&id).await
    }

    pub async fn rename_local_list(
        &self,
        id: &str,
        name: &str,
    ) -> Result<TaskListRecord, TaskError> {
        self.require_local_list(id).await?;
        sqlx::query("UPDATE task_lists SET name = ?, updated_at = ? WHERE id = ?")
            .bind(name)
            .bind(now())
            .bind(id)
            .execute(self.pool)
            .await?;
        self.list(id).await
    }

    /// Delete a local list with its tasks, or the leftover of a remote list
    /// the server dropped. The default list comes back empty the next time
    /// lists are read if it was the last local one.
    pub async fn delete_local_list(&self, id: &str) -> Result<(), TaskError> {
        let state = self.list_state(id).await?;
        if state.provider != LOCAL_PROVIDER && !state.gone {
            return Err(TaskError::ListNotLocal);
        }
        sqlx::query("DELETE FROM task_lists WHERE id = ?")
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    async fn require_local_list(&self, id: &str) -> Result<(), TaskError> {
        let state = self.list_state(id).await?;
        if state.provider != LOCAL_PROVIDER {
            return Err(TaskError::ListNotLocal);
        }
        Ok(())
    }

    async fn list(&self, id: &str) -> Result<TaskListRecord, TaskError> {
        self.list_lists()
            .await?
            .into_iter()
            .find(|list| list.id == id)
            .ok_or(TaskError::ListNotFound)
    }

    async fn list_state(&self, id: &str) -> Result<ListState, TaskError> {
        sqlx::query_as::<_, ListState>(
            "SELECT provider, read_only, gone FROM task_lists WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(self.pool)
        .await?
        .ok_or(TaskError::ListNotFound)
    }

    /// Open tasks, and the most recently completed ones when asked for.
    pub async fn list_tasks(&self, include_completed: bool) -> Result<Vec<TaskRecord>, TaskError> {
        let completed = if include_completed {
            " UNION ALL SELECT * FROM (
                 SELECT * FROM visible WHERE completed_at IS NOT NULL
                 ORDER BY completed_at DESC LIMIT 500)"
        } else {
            ""
        };
        sqlx::query_as::<_, TaskRecord>(&format!(
            "WITH visible AS ({TASK_SELECT} WHERE COALESCE(t.pending_op, '') <> 'delete')
             SELECT * FROM visible WHERE completed_at IS NULL{completed}"
        ))
        .fetch_all(self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn get(&self, id: &str) -> Result<TaskRecord, TaskError> {
        sqlx::query_as::<_, TaskRecord>(&format!(
            "{TASK_SELECT} WHERE t.id = ? AND COALESCE(t.pending_op, '') <> 'delete'"
        ))
        .bind(id)
        .fetch_optional(self.pool)
        .await?
        .ok_or(TaskError::TaskNotFound)
    }

    /// The account whose server a task's change has to reach, if any.
    pub async fn remote_account_of_list(&self, list_id: &str) -> Result<Option<String>, TaskError> {
        sqlx::query_scalar::<_, Option<String>>("SELECT account_id FROM task_lists WHERE id = ?")
            .bind(list_id)
            .fetch_optional(self.pool)
            .await?
            .ok_or(TaskError::ListNotFound)
    }

    pub async fn create(
        &self,
        list_id: &str,
        fields: &TaskWrite,
        thread_id: Option<&str>,
        message_id: Option<&str>,
    ) -> Result<TaskRecord, TaskError> {
        let list = self.list_state(list_id).await?;
        if list.read_only {
            return Err(TaskError::ListReadOnly);
        }
        let id = format!("task-{}", uuid::Uuid::new_v4());
        let uid = format!("{}@satchel.local", uuid::Uuid::new_v4());
        let pending = (list.provider != LOCAL_PROVIDER).then_some("create");
        let at = now();
        sqlx::query(
            "INSERT INTO tasks
                (id, list_id, uid, title, notes, due_at, reminder_at, priority, completed_at,
                 thread_id, message_id, pending_op, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(list_id)
        .bind(&uid)
        .bind(&fields.title)
        .bind(fields.notes.as_deref())
        .bind(fields.due_at.as_deref())
        .bind(fields.reminder_at.as_deref())
        .bind(i64::from(fields.priority))
        .bind(fields.completed_at.as_deref())
        .bind(thread_id)
        .bind(message_id)
        .bind(pending)
        .bind(&at)
        .bind(&at)
        .execute(self.pool)
        .await?;
        self.get(&id).await
    }

    async fn task_state(&self, id: &str) -> Result<TaskState, TaskError> {
        sqlx::query_as::<_, TaskState>(
            "SELECT list_id, pending_op, remote_id, due_at, start_at, reminder_at, completed_at,
                    rrule, recurrence, updated_at
             FROM tasks WHERE id = ? AND COALESCE(pending_op, '') <> 'delete'",
        )
        .bind(id)
        .fetch_optional(self.pool)
        .await?
        .ok_or(TaskError::TaskNotFound)
    }

    /// Replace the editable fields. `expected_updated_at` is the revision the
    /// editor opened; a sync that changed the task since wins.
    pub async fn update(
        &self,
        id: &str,
        fields: &TaskWrite,
        expected_updated_at: Option<&str>,
    ) -> Result<TaskRecord, TaskError> {
        let state = self.task_state(id).await?;
        if expected_updated_at.is_some_and(|expected| expected != state.updated_at) {
            return Err(TaskError::TaskChanged);
        }
        let list = self.list_state(&state.list_id).await?;
        if list.read_only {
            return Err(TaskError::ListReadOnly);
        }
        let pending = next_pending(&list.provider, state.pending_op.as_deref());
        let completion = plan_completion(
            &state,
            fields.due_at.as_deref(),
            fields.reminder_at.as_deref(),
            fields.completed_at.as_deref(),
        );
        // A simple rule stays simple only while there is a due date to step.
        let recurrence = match state.recurrence.as_deref() {
            Some(recurrence::SIMPLE) => recurrence::classify(
                completion.rrule.as_deref(),
                false,
                completion.due_at.as_deref(),
            )
            .stored(),
            other => other.map(|_| recurrence::COMPLEX),
        };
        let reminder_changed = completion.reminder_at != state.reminder_at;
        let result = sqlx::query(
            "UPDATE tasks SET title = ?, notes = ?, due_at = ?, start_at = ?, reminder_at = ?,
                    priority = ?, completed_at = ?, rrule = ?, recurrence = ?, pending_op = ?,
                    push_failures = 0, push_retry_at = NULL,
                    reminded_at = CASE WHEN ? THEN NULL ELSE reminded_at END,
                    updated_at = ?
             WHERE id = ? AND updated_at = ?",
        )
        .bind(&fields.title)
        .bind(fields.notes.as_deref())
        .bind(completion.due_at.as_deref())
        .bind(completion.start_at.as_deref())
        .bind(completion.reminder_at.as_deref())
        .bind(i64::from(fields.priority))
        .bind(completion.completed_at.as_deref())
        .bind(completion.rrule.as_deref())
        .bind(recurrence)
        .bind(pending)
        .bind(reminder_changed)
        .bind(now())
        .bind(id)
        .bind(&state.updated_at)
        .execute(self.pool)
        .await?;
        if result.rows_affected() == 0 {
            return Err(TaskError::TaskChanged);
        }
        self.get(id).await
    }

    pub async fn set_completed(
        &self,
        id: &str,
        completed_at: Option<&str>,
    ) -> Result<TaskRecord, TaskError> {
        let state = self.task_state(id).await?;
        let list = self.list_state(&state.list_id).await?;
        if list.read_only {
            return Err(TaskError::ListReadOnly);
        }
        let pending = next_pending(&list.provider, state.pending_op.as_deref());
        let completion = plan_completion(
            &state,
            state.due_at.as_deref(),
            state.reminder_at.as_deref(),
            completed_at,
        );
        sqlx::query(
            "UPDATE tasks SET completed_at = ?, due_at = ?, start_at = ?, reminder_at = ?,
                    rrule = ?, pending_op = ?, push_failures = 0, push_retry_at = NULL,
                    reminded_at = CASE WHEN reminder_at IS ? THEN reminded_at ELSE NULL END,
                    updated_at = ?
             WHERE id = ?",
        )
        .bind(completion.completed_at.as_deref())
        .bind(completion.due_at.as_deref())
        .bind(completion.start_at.as_deref())
        .bind(completion.reminder_at.as_deref())
        .bind(completion.rrule.as_deref())
        .bind(pending)
        .bind(completion.reminder_at.as_deref())
        .bind(now())
        .bind(id)
        .execute(self.pool)
        .await?;
        self.get(id).await
    }

    /// Local tasks and tasks the server never saw go at once; the others
    /// wait, hidden, for the server to confirm.
    pub async fn delete(&self, id: &str) -> Result<(), TaskError> {
        let state = self.task_state(id).await?;
        let list = self.list_state(&state.list_id).await?;
        if list.read_only && !list.gone {
            return Err(TaskError::ListReadOnly);
        }
        // A list the server dropped cannot take the deletion any more.
        if list.provider == LOCAL_PROVIDER || list.gone || state.remote_id.is_none() {
            sqlx::query("DELETE FROM tasks WHERE id = ?")
                .bind(id)
                .execute(self.pool)
                .await?;
        } else {
            sqlx::query("UPDATE tasks SET pending_op = 'delete', updated_at = ? WHERE id = ?")
                .bind(now())
                .bind(id)
                .execute(self.pool)
                .await?;
        }
        Ok(())
    }

    /// Restore a task removed by [`Self::delete`] (undo). A task whose deletion
    /// already reached the server is gone; that is reported as not found.
    pub async fn undelete(&self, id: &str) -> Result<TaskRecord, TaskError> {
        let result = sqlx::query(
            "UPDATE tasks SET pending_op = CASE
                    WHEN remote_payload IS NULL THEN 'create' ELSE 'update' END,
                    updated_at = ?
             WHERE id = ? AND pending_op = 'delete'",
        )
        .bind(now())
        .bind(id)
        .execute(self.pool)
        .await?;
        if result.rows_affected() == 0 {
            return Err(TaskError::TaskNotFound);
        }
        self.get(id).await
    }

    /// Claim reminders that are due, at most 25 at a time, so each shows once.
    /// Ones more than a day old are marked shown without being returned.
    pub async fn claim_due_reminders(&self, now_utc: &str) -> Result<Vec<TaskRecord>, TaskError> {
        let cutoff = chrono::DateTime::parse_from_rfc3339(now_utc)
            .map(|now| {
                (now.with_timezone(&chrono::Utc) - chrono::Duration::hours(REMINDER_GRACE_HOURS))
                    .format("%Y-%m-%dT%H:%M:%SZ")
                    .to_string()
            })
            .unwrap_or_else(|_| now_utc.to_string());
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        sqlx::query(
            "UPDATE tasks SET reminded_at = ?
             WHERE reminder_at IS NOT NULL AND reminded_at IS NULL AND reminder_at < ?",
        )
        .bind(now_utc)
        .bind(&cutoff)
        .execute(&mut *transaction)
        .await?;
        let ids = sqlx::query_scalar::<_, String>(
            "UPDATE tasks SET reminded_at = ?
             WHERE id IN (
                 SELECT id FROM tasks
                 WHERE reminder_at IS NOT NULL AND reminded_at IS NULL
                   AND completed_at IS NULL AND COALESCE(pending_op, '') <> 'delete'
                   AND reminder_at <= ?
                 ORDER BY reminder_at LIMIT 25)
             RETURNING id",
        )
        .bind(now_utc)
        .bind(now_utc)
        .fetch_all(&mut *transaction)
        .await?;
        let mut records = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(record) =
                sqlx::query_as::<_, TaskRecord>(&format!("{TASK_SELECT} WHERE t.id = ?"))
                    .bind(&id)
                    .fetch_optional(&mut *transaction)
                    .await?
            {
                records.push(record);
            }
        }
        transaction.commit().await?;
        Ok(records)
    }

    // ── Sync ────────────────────────────────────────────────────────────

    /// Insert or refresh a remote list; returns its id.
    pub async fn upsert_remote_list(
        &self,
        list: &RemoteListWrite<'_>,
    ) -> Result<String, TaskError> {
        let id = remote_list_id(list.account_id, list.provider, list.remote_id);
        let at = now();
        sqlx::query(
            "INSERT INTO task_lists
                (id, account_id, provider, remote_id, name, color, read_only, mixed, gone,
                 created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, COALESCE(?, 0), ?, 0, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name, color = excluded.color, mixed = excluded.mixed, gone = 0,
                -- Unknown privileges keep what a refused write taught the list.
                read_only = CASE WHEN ? IS NULL THEN task_lists.read_only
                    ELSE excluded.read_only END,
                updated_at = CASE WHEN (task_lists.name, task_lists.color, task_lists.mixed)
                    IS (excluded.name, excluded.color, excluded.mixed)
                    THEN task_lists.updated_at ELSE excluded.updated_at END",
        )
        .bind(&id)
        .bind(list.account_id)
        .bind(list.provider)
        .bind(list.remote_id)
        .bind(list.name)
        .bind(list.color)
        .bind(list.read_only)
        .bind(list.mixed)
        .bind(&at)
        .bind(&at)
        .bind(list.read_only)
        .execute(self.pool)
        .await?;
        Ok(id)
    }

    /// Drop this account's lists of `provider` that the server no longer has.
    /// A list holding unsent changes is kept, read-only and marked gone, so
    /// that work is not cascaded away; it returns the count of those.
    pub async fn retain_remote_lists(
        &self,
        account_id: &str,
        provider: &str,
        keep: &[String],
    ) -> Result<(u64, u64), TaskError> {
        let existing = sqlx::query_scalar::<_, String>(
            "SELECT id FROM task_lists WHERE account_id = ? AND provider = ?",
        )
        .bind(account_id)
        .bind(provider)
        .fetch_all(self.pool)
        .await?;
        let mut removed = 0;
        let mut kept = 0;
        for id in existing.into_iter().filter(|id| !keep.contains(id)) {
            let pending: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM tasks WHERE list_id = ? AND pending_op IN ('create', 'update'))",
            )
            .bind(&id)
            .fetch_one(self.pool)
            .await?;
            if pending {
                kept += sqlx::query(
                    "UPDATE task_lists SET gone = 1, read_only = 1, updated_at = ?
                     WHERE id = ? AND gone = 0",
                )
                .bind(now())
                .bind(&id)
                .execute(self.pool)
                .await?
                .rows_affected();
            } else {
                removed += sqlx::query("DELETE FROM task_lists WHERE id = ?")
                    .bind(id)
                    .execute(self.pool)
                    .await?
                    .rows_affected();
            }
        }
        Ok((removed, kept))
    }

    /// A refused write (403) taught us the list is read-only.
    pub async fn mark_list_read_only(&self, list_id: &str) -> Result<(), TaskError> {
        sqlx::query("UPDATE task_lists SET read_only = 1, updated_at = ? WHERE id = ?")
            .bind(now())
            .bind(list_id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    pub async fn mark_list_synced(&self, list_id: &str) -> Result<(), TaskError> {
        sqlx::query("UPDATE task_lists SET last_synced_at = ? WHERE id = ?")
            .bind(now())
            .bind(list_id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    /// Pending changes whose backoff, if any, has run out.
    pub async fn pending(&self, list_id: &str) -> Result<Vec<PendingTask>, TaskError> {
        sqlx::query_as::<_, PendingTask>(&format!(
            "{PENDING_SELECT} WHERE list_id = ? AND pending_op IS NOT NULL
               AND (push_retry_at IS NULL OR push_retry_at <= ?)
             ORDER BY updated_at"
        ))
        .bind(list_id)
        .bind(now())
        .fetch_all(self.pool)
        .await
        .map_err(Into::into)
    }

    /// This task alone failed to push; it waits longer each time.
    pub async fn record_push_failure(&self, id: &str) -> Result<(), TaskError> {
        let failures: i64 = sqlx::query_scalar(
            "UPDATE tasks SET push_failures = push_failures + 1 WHERE id = ? RETURNING push_failures",
        )
        .bind(id)
        .fetch_optional(self.pool)
        .await?
        .unwrap_or(1);
        sqlx::query("UPDATE tasks SET push_retry_at = ? WHERE id = ?")
            .bind((chrono::Utc::now() + push_backoff(failures - 1)).to_rfc3339())
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    /// Record a successful push. An edit made while the push was in flight
    /// keeps its pending mark, now as an update of the stored resource.
    pub async fn mark_pushed(
        &self,
        task: &PendingTask,
        remote_id: &str,
        remote_version: Option<&str>,
        remote_payload: &str,
        still_pending: bool,
    ) -> Result<(), TaskError> {
        sqlx::query(
            "UPDATE tasks SET remote_id = ?, remote_version = ?, remote_payload = ?,
                    push_failures = 0, push_retry_at = NULL,
                    pending_op = CASE WHEN updated_at = ? AND NOT ? THEN NULL ELSE 'update' END
             WHERE id = ?",
        )
        .bind(remote_id)
        .bind(remote_version)
        .bind(remote_payload)
        .bind(&task.updated_at)
        .bind(still_pending)
        .bind(&task.id)
        .execute(self.pool)
        .await?;
        Ok(())
    }

    /// The server refused the change because its copy moved on (or is gone).
    /// The server wins, as with calendar events: forget the local change and
    /// let the next pull bring the server's version (or the deletion).
    pub async fn yield_to_server(&self, id: &str) -> Result<(), TaskError> {
        sqlx::query(
            "UPDATE tasks SET pending_op = NULL, remote_version = NULL WHERE id = ? AND remote_id IS NOT NULL",
        )
        .bind(id)
        .execute(self.pool)
        .await?;
        sqlx::query("DELETE FROM tasks WHERE id = ? AND remote_id IS NULL")
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    /// Turn a task whose server copy is gone, or whose list was dropped, into
    /// a local task: the unsent edit survives on this device.
    pub async fn keep_as_local(&self, id: &str) -> Result<(), TaskError> {
        let list_id = self.local_list().await?;
        sqlx::query(
            "UPDATE tasks SET list_id = ?, remote_id = NULL, remote_version = NULL,
                    remote_payload = NULL, pending_op = NULL, push_failures = 0,
                    push_retry_at = NULL, updated_at = ?
             WHERE id = ?",
        )
        .bind(list_id)
        .bind(now())
        .bind(id)
        .execute(self.pool)
        .await?;
        Ok(())
    }

    /// The local list kept work lands in: the default one, or the first.
    async fn local_list(&self) -> Result<String, TaskError> {
        self.ensure_default_list().await?;
        sqlx::query_scalar::<_, String>(
            "SELECT id FROM task_lists WHERE provider = ? ORDER BY id = ? DESC, created_at LIMIT 1",
        )
        .bind(LOCAL_PROVIDER)
        .bind(DEFAULT_LOCAL_LIST_ID)
        .fetch_one(self.pool)
        .await
        .map_err(Into::into)
    }

    /// Learn the version of a resource this device wrote but whose ETag the
    /// server did not return. Pending edits can then go out with `If-Match`.
    pub async fn adopt_version(
        &self,
        list_id: &str,
        remote_id: &str,
        version: Option<&str>,
    ) -> Result<(), TaskError> {
        sqlx::query("UPDATE tasks SET remote_version = ? WHERE list_id = ? AND remote_id = ?")
            .bind(version)
            .bind(list_id)
            .bind(remote_id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    /// The stored payload of a remote task, to compare with the server's.
    pub async fn remote_payload(
        &self,
        list_id: &str,
        remote_id: &str,
    ) -> Result<Option<String>, TaskError> {
        Ok(sqlx::query_scalar::<_, Option<String>>(
            "SELECT remote_payload FROM tasks WHERE list_id = ? AND remote_id = ?",
        )
        .bind(list_id)
        .bind(remote_id)
        .fetch_optional(self.pool)
        .await?
        .flatten())
    }

    /// Let the server's copy replace a pending local change.
    pub async fn drop_pending(&self, list_id: &str, remote_id: &str) -> Result<(), TaskError> {
        sqlx::query(
            "UPDATE tasks SET pending_op = NULL, push_failures = 0, push_retry_at = NULL
             WHERE list_id = ? AND remote_id = ?",
        )
        .bind(list_id)
        .bind(remote_id)
        .execute(self.pool)
        .await?;
        Ok(())
    }

    pub async fn remove(&self, id: &str) -> Result<(), TaskError> {
        sqlx::query("DELETE FROM tasks WHERE id = ?")
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    /// Every remote task of a list: `(remote_id, remote_version, pending)`.
    pub async fn remote_versions(
        &self,
        list_id: &str,
    ) -> Result<Vec<(String, Option<String>, bool)>, TaskError> {
        sqlx::query_as::<_, (String, Option<String>, bool)>(
            "SELECT remote_id, remote_version, pending_op IS NOT NULL FROM tasks
             WHERE list_id = ? AND remote_id IS NOT NULL",
        )
        .bind(list_id)
        .fetch_all(self.pool)
        .await
        .map_err(Into::into)
    }

    /// Mirror one server resource. A row with a local change still pending is
    /// left alone; it is pushed (or yields) first.
    pub async fn apply_remote(
        &self,
        list_id: &str,
        task: &RemoteTaskWrite<'_>,
    ) -> Result<bool, TaskError> {
        let id = stable_id("task", &format!("{list_id}\0{}", task.remote_id));
        let fields = task.fields;
        let at = now();
        let result = sqlx::query(
            "INSERT INTO tasks
                (id, list_id, uid, title, notes, due_at, start_at, rrule, recurrence,
                 reminder_at, priority, completed_at,
                 thread_id, message_id, remote_id, remote_version, remote_payload, pending_op,
                 created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?,
                     (SELECT thread_id FROM messages WHERE ? IS NOT NULL
                        AND account_id IN (SELECT id FROM accounts) AND message_id_header = ?
                        AND thread_id IS NOT NULL LIMIT 1),
                     ?, ?, ?, ?, NULL, ?, ?)
             ON CONFLICT(list_id, remote_id) WHERE remote_id IS NOT NULL DO UPDATE SET
                uid = excluded.uid, title = excluded.title, notes = excluded.notes,
                due_at = excluded.due_at, start_at = excluded.start_at,
                rrule = excluded.rrule, recurrence = excluded.recurrence,
                reminder_at = excluded.reminder_at,
                reminded_at = CASE WHEN tasks.reminder_at IS excluded.reminder_at
                    THEN tasks.reminded_at ELSE NULL END,
                priority = excluded.priority, completed_at = excluded.completed_at,
                message_id = excluded.message_id,
                thread_id = COALESCE(tasks.thread_id, excluded.thread_id),
                remote_version = excluded.remote_version,
                remote_payload = excluded.remote_payload,
                updated_at = excluded.updated_at
             WHERE tasks.pending_op IS NULL",
        )
        .bind(&id)
        .bind(list_id)
        .bind(task.uid)
        .bind(&fields.title)
        .bind(fields.notes.as_deref())
        .bind(fields.due_at.as_deref())
        .bind(task.start_at)
        .bind(task.rrule)
        .bind(task.recurrence)
        .bind(fields.reminder_at.as_deref())
        .bind(i64::from(fields.priority))
        .bind(fields.completed_at.as_deref())
        .bind(task.message_id)
        .bind(task.message_id)
        .bind(task.message_id)
        .bind(task.remote_id)
        .bind(task.remote_version)
        .bind(task.remote_payload)
        .bind(&at)
        .bind(&at)
        .execute(self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Remove tasks the server no longer lists. Pending creations have no
    /// remote id yet and stay. A pending edit of a task deleted on the server
    /// is not thrown away: it stays as a local task, and the caller says so.
    pub async fn remove_missing(
        &self,
        list_id: &str,
        seen: &[String],
    ) -> Result<Removal, TaskError> {
        let mut removal = Removal::default();
        let rows = sqlx::query_as::<_, (String, String, Option<String>)>(
            "SELECT id, remote_id, pending_op FROM tasks WHERE list_id = ? AND remote_id IS NOT NULL",
        )
        .bind(list_id)
        .fetch_all(self.pool)
        .await?;
        for (id, remote_id, pending) in rows {
            if seen.contains(&remote_id) {
                continue;
            }
            if pending.as_deref() == Some("update") {
                self.keep_as_local(&id).await?;
                removal.kept_local += 1;
            } else {
                self.remove(&id).await?;
                removal.removed += 1;
            }
        }
        Ok(removal)
    }
}

/// The pending mark after a local change: none for local lists, and a task
/// the server has not seen yet stays a creation.
fn next_pending(provider: &str, current: Option<&str>) -> Option<&'static str> {
    if provider == LOCAL_PROVIDER {
        None
    } else if current == Some("create") {
        Some("create")
    } else {
        Some("update")
    }
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
            .unwrap();
        crate::db::init(&pool).await.unwrap();
        pool
    }

    async fn account(pool: &SqlitePool, id: &str) {
        sqlx::query(
            "INSERT INTO accounts
                (id, display_name, email, provider, config, credentials_ref, sync_state,
                 color, is_active, created_at, updated_at)
             VALUES (?, ?, ?, 'imap', '{}', '', '{}', '#123456', 1, '2026-01-01', '2026-01-01')",
        )
        .bind(id)
        .bind(id)
        .bind(format!("{id}@example.test"))
        .execute(pool)
        .await
        .unwrap();
    }

    fn write(title: &str) -> TaskWrite {
        TaskWrite {
            title: title.into(),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn a_default_local_list_exists_without_accounts() {
        let pool = pool().await;
        let repo = TaskRepository::new(&pool);
        repo.ensure_default_list().await.unwrap();
        repo.ensure_default_list().await.unwrap();
        let lists = repo.list_lists().await.unwrap();
        assert_eq!(lists.len(), 1);
        assert_eq!(lists[0].id, DEFAULT_LOCAL_LIST_ID);
        assert_eq!(lists[0].provider, LOCAL_PROVIDER);
        assert_eq!(lists[0].name, "");

        let task = repo
            .create(DEFAULT_LOCAL_LIST_ID, &write("Buy milk"), None, None)
            .await
            .unwrap();
        assert!(!task.pending);
        assert_eq!(repo.list_lists().await.unwrap()[0].open_count, 1);
        repo.set_completed(&task.id, Some("2026-11-06T10:00:00Z"))
            .await
            .unwrap();
        assert!(repo.list_tasks(false).await.unwrap().is_empty());
        assert_eq!(repo.list_tasks(true).await.unwrap().len(), 1);
        repo.delete(&task.id).await.unwrap();
        assert!(repo.list_tasks(true).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn remote_changes_queue_and_deletions_wait_for_the_server() {
        let pool = pool().await;
        account(&pool, "acc").await;
        let repo = TaskRepository::new(&pool);
        let list = repo
            .upsert_remote_list(&RemoteListWrite {
                account_id: "acc",
                provider: "caldav",
                remote_id: "https://dav.example.test/tasks/",
                name: "Work",
                color: None,
                read_only: None,
                mixed: false,
            })
            .await
            .unwrap();
        let task = repo
            .create(&list, &write("Draft"), None, None)
            .await
            .unwrap();
        assert!(task.pending);
        let pending = repo.pending(&list).await.unwrap();
        assert_eq!(pending[0].pending_op, "create");

        // Edited before the push: still a creation.
        let task = repo
            .update(&task.id, &write("Draft v2"), Some(&task.updated_at))
            .await
            .unwrap();
        assert_eq!(repo.pending(&list).await.unwrap()[0].pending_op, "create");

        let pending = repo.pending(&list).await.unwrap().remove(0);
        repo.mark_pushed(
            &pending,
            "https://dav.example.test/tasks/a.ics",
            Some("\"1\""),
            "RAW",
            false,
        )
        .await
        .unwrap();
        assert!(repo.pending(&list).await.unwrap().is_empty());

        repo.delete(&task.id).await.unwrap();
        assert!(repo.list_tasks(true).await.unwrap().is_empty());
        assert_eq!(repo.pending(&list).await.unwrap()[0].pending_op, "delete");
        let restored = repo.undelete(&task.id).await.unwrap();
        assert_eq!(restored.title, "Draft v2");
        assert_eq!(repo.pending(&list).await.unwrap()[0].pending_op, "update");
    }

    #[tokio::test]
    async fn an_edit_during_a_push_stays_pending() {
        let pool = pool().await;
        account(&pool, "acc").await;
        let repo = TaskRepository::new(&pool);
        let list = repo
            .upsert_remote_list(&RemoteListWrite {
                account_id: "acc",
                provider: "caldav",
                remote_id: "/tasks/",
                name: "",
                color: None,
                read_only: None,
                mixed: false,
            })
            .await
            .unwrap();
        let task = repo.create(&list, &write("One"), None, None).await.unwrap();
        let snapshot = repo.pending(&list).await.unwrap().remove(0);
        // Different timestamp than the snapshot.
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        repo.update(&task.id, &write("Two"), None).await.unwrap();
        repo.mark_pushed(&snapshot, "/tasks/one.ics", Some("e1"), "RAW", false)
            .await
            .unwrap();
        let pending = repo.pending(&list).await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].pending_op, "update");
        assert_eq!(pending[0].remote_id.as_deref(), Some("/tasks/one.ics"));
    }

    #[tokio::test]
    async fn stale_editor_revisions_are_rejected() {
        let pool = pool().await;
        let repo = TaskRepository::new(&pool);
        repo.ensure_default_list().await.unwrap();
        let task = repo
            .create(DEFAULT_LOCAL_LIST_ID, &write("A"), None, None)
            .await
            .unwrap();
        assert!(matches!(
            repo.update(&task.id, &write("B"), Some("1999-01-01")).await,
            Err(TaskError::TaskChanged)
        ));
    }

    #[tokio::test]
    async fn reminders_fire_once_and_rearm_when_moved() {
        let pool = pool().await;
        let repo = TaskRepository::new(&pool);
        repo.ensure_default_list().await.unwrap();
        let mut fields = write("Call");
        fields.reminder_at = Some("2026-11-06T09:00:00Z".into());
        let task = repo
            .create(DEFAULT_LOCAL_LIST_ID, &fields, None, None)
            .await
            .unwrap();
        assert!(repo
            .claim_due_reminders("2026-11-06T08:59:00Z")
            .await
            .unwrap()
            .is_empty());
        assert_eq!(
            repo.claim_due_reminders("2026-11-06T09:00:00Z")
                .await
                .unwrap()
                .len(),
            1
        );
        assert!(repo
            .claim_due_reminders("2026-11-06T10:00:00Z")
            .await
            .unwrap()
            .is_empty());
        fields.reminder_at = Some("2026-11-07T09:00:00Z".into());
        repo.update(&task.id, &fields, None).await.unwrap();
        assert_eq!(
            repo.claim_due_reminders("2026-11-07T09:00:00Z")
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn the_thread_link_follows_the_message_id() {
        let pool = pool().await;
        account(&pool, "acc").await;
        sqlx::query(
            "INSERT INTO threads (id, account_id, subject, last_message_date, created_at, updated_at)
             VALUES ('thread-new', 'acc', 'Hi', '2026-01-01', '2026-01-01', '2026-01-01')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO folders (id, account_id, name, full_path, folder_type, created_at, updated_at)
             VALUES ('f', 'acc', 'INBOX', 'INBOX', 'inbox', '2026-01-01', '2026-01-01')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO messages (id, account_id, folder_id, thread_id, message_id_header,
                from_address, to_addresses, date, created_at, updated_at)
             VALUES ('m', 'acc', 'f', 'thread-new', '<m@example.test>', 'a@example.test', '[]',
                '2026-01-01', '2026-01-01', '2026-01-01')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let repo = TaskRepository::new(&pool);
        repo.ensure_default_list().await.unwrap();
        // The stored thread was merged away; the Message-ID still leads to the mail.
        let task = repo
            .create(
                DEFAULT_LOCAL_LIST_ID,
                &write("Reply"),
                Some("thread-gone"),
                Some("<m@example.test>"),
            )
            .await
            .unwrap();
        assert_eq!(task.thread_id.as_deref(), Some("thread-new"));
    }

    #[tokio::test]
    async fn stale_reminders_are_marked_shown_without_a_notification() {
        let pool = pool().await;
        let repo = TaskRepository::new(&pool);
        repo.ensure_default_list().await.unwrap();
        // A first import of many old alarms, and one from this morning.
        for day in 1..=30 {
            let mut fields = write(&format!("Old {day}"));
            fields.reminder_at = Some(format!("2026-10-{day:02}T09:00:00Z"));
            repo.create(DEFAULT_LOCAL_LIST_ID, &fields, None, None)
                .await
                .unwrap();
        }
        let mut fresh = write("This morning");
        fresh.reminder_at = Some("2026-11-06T07:00:00Z".into());
        repo.create(DEFAULT_LOCAL_LIST_ID, &fresh, None, None)
            .await
            .unwrap();

        let shown = repo
            .claim_due_reminders("2026-11-06T10:00:00Z")
            .await
            .unwrap();
        assert_eq!(
            shown
                .iter()
                .map(|task| task.title.as_str())
                .collect::<Vec<_>>(),
            ["This morning"]
        );
        let unclaimed: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM tasks WHERE reminded_at IS NULL")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(unclaimed, 0);
        // The app closed for three days: what fell due meanwhile stays quiet too.
        let mut weekend = write("Saturday");
        weekend.reminder_at = Some("2026-11-07T09:00:00Z".into());
        repo.create(DEFAULT_LOCAL_LIST_ID, &weekend, None, None)
            .await
            .unwrap();
        assert!(repo
            .claim_due_reminders("2026-11-10T09:00:00Z")
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn completing_a_simple_repeating_task_moves_it_to_the_next_occurrence() {
        let pool = pool().await;
        let repo = TaskRepository::new(&pool);
        repo.ensure_default_list().await.unwrap();
        let mut fields = write("Water plants");
        fields.due_at = Some("2026-11-06".into());
        let task = repo
            .create(DEFAULT_LOCAL_LIST_ID, &fields, None, None)
            .await
            .unwrap();
        // A local repeating task (an imported or kept one carries its rule).
        sqlx::query(
            "UPDATE tasks SET rrule = 'FREQ=WEEKLY;COUNT=2', recurrence = 'simple' WHERE id = ?",
        )
        .bind(&task.id)
        .execute(&pool)
        .await
        .unwrap();
        let next = repo
            .set_completed(&task.id, Some("2026-11-06T12:00:00Z"))
            .await
            .unwrap();
        assert_eq!(next.completed_at, None);
        assert_eq!(next.due_at.as_deref(), Some("2026-11-13"));
        assert_eq!(next.recurrence.as_deref(), Some("simple"));
        // COUNT=1 now: this occurrence is the last, so the series ends.
        let done = repo
            .set_completed(&task.id, Some("2026-11-13T12:00:00Z"))
            .await
            .unwrap();
        assert_eq!(done.completed_at.as_deref(), Some("2026-11-13T12:00:00Z"));
        assert_eq!(done.due_at.as_deref(), Some("2026-11-13"));
    }

    #[tokio::test]
    async fn a_complex_series_completes_as_asked() {
        let pool = pool().await;
        let repo = TaskRepository::new(&pool);
        repo.ensure_default_list().await.unwrap();
        let mut fields = write("Board meeting prep");
        fields.due_at = Some("2026-11-06".into());
        let task = repo
            .create(DEFAULT_LOCAL_LIST_ID, &fields, None, None)
            .await
            .unwrap();
        sqlx::query("UPDATE tasks SET rrule = 'FREQ=MONTHLY;BYDAY=1FR', recurrence = 'complex' WHERE id = ?")
            .bind(&task.id)
            .execute(&pool)
            .await
            .unwrap();
        let done = repo
            .set_completed(&task.id, Some("2026-11-06T12:00:00Z"))
            .await
            .unwrap();
        assert!(done.completed_at.is_some());
        assert_eq!(done.due_at.as_deref(), Some("2026-11-06"));
    }

    #[tokio::test]
    async fn calendars_that_also_hold_events_show_once_they_hold_a_task() {
        let pool = pool().await;
        account(&pool, "acc").await;
        let repo = TaskRepository::new(&pool);
        let list = repo
            .upsert_remote_list(&RemoteListWrite {
                account_id: "acc",
                provider: "caldav",
                remote_id: "/cal/personal/",
                name: "Personal",
                color: None,
                read_only: None,
                mixed: true,
            })
            .await
            .unwrap();
        assert!(repo
            .list_lists()
            .await
            .unwrap()
            .iter()
            .all(|item| item.id != list));
        repo.create(&list, &write("Errand"), None, None)
            .await
            .unwrap();
        assert!(repo
            .list_lists()
            .await
            .unwrap()
            .iter()
            .any(|item| item.id == list));
    }

    #[tokio::test]
    async fn a_learnt_read_only_list_stays_read_only_when_discovery_does_not_say() {
        let pool = pool().await;
        account(&pool, "acc").await;
        let repo = TaskRepository::new(&pool);
        let write_list = |read_only| RemoteListWrite {
            account_id: "acc",
            provider: "caldav",
            remote_id: "/shared/",
            name: "Shared",
            color: None,
            read_only,
            mixed: false,
        };
        let list = repo.upsert_remote_list(&write_list(None)).await.unwrap();
        repo.mark_list_read_only(&list).await.unwrap();
        repo.upsert_remote_list(&write_list(None)).await.unwrap();
        assert!(matches!(
            repo.create(&list, &write("x"), None, None).await,
            Err(TaskError::ListReadOnly)
        ));
        repo.upsert_remote_list(&write_list(Some(false)))
            .await
            .unwrap();
        assert!(repo.create(&list, &write("x"), None, None).await.is_ok());
    }

    #[tokio::test]
    async fn a_failed_push_backs_off_for_that_task_only() {
        let pool = pool().await;
        account(&pool, "acc").await;
        let repo = TaskRepository::new(&pool);
        let list = repo
            .upsert_remote_list(&RemoteListWrite {
                account_id: "acc",
                provider: "caldav",
                remote_id: "/tasks/",
                name: "",
                color: None,
                read_only: None,
                mixed: false,
            })
            .await
            .unwrap();
        let bad = repo.create(&list, &write("Bad"), None, None).await.unwrap();
        repo.create(&list, &write("Good"), None, None)
            .await
            .unwrap();
        repo.record_push_failure(&bad.id).await.unwrap();
        let pending = repo.pending(&list).await.unwrap();
        assert_eq!(
            pending
                .iter()
                .map(|task| task.title.as_str())
                .collect::<Vec<_>>(),
            ["Good"]
        );
        // A new local edit retries at once.
        repo.update(&bad.id, &write("Bad, fixed"), None)
            .await
            .unwrap();
        assert_eq!(repo.pending(&list).await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn the_message_id_fallback_probes_the_account_index() {
        let pool = pool().await;
        let plan = sqlx::query_as::<_, (i64, i64, i64, String)>(
            "EXPLAIN QUERY PLAN SELECT m.thread_id FROM messages m
             WHERE m.account_id IN (SELECT id FROM accounts)
               AND m.message_id_header = ? AND m.thread_id IS NOT NULL LIMIT 1",
        )
        .bind("<m@example.test>")
        .fetch_all(&pool)
        .await
        .unwrap();
        let details = plan.into_iter().map(|row| row.3).collect::<Vec<_>>();
        assert!(
            details
                .iter()
                .any(|detail| detail.contains("idx_messages_account_msgid")),
            "{details:?}"
        );
    }

    #[tokio::test]
    async fn pulls_skip_pending_rows_and_prune_what_the_server_dropped() {
        let pool = pool().await;
        account(&pool, "acc").await;
        let repo = TaskRepository::new(&pool);
        let list = repo
            .upsert_remote_list(&RemoteListWrite {
                account_id: "acc",
                provider: "caldav",
                remote_id: "/tasks/",
                name: "Tasks",
                color: None,
                read_only: None,
                mixed: false,
            })
            .await
            .unwrap();
        let fields = write("Server title");
        for remote_id in ["/tasks/a.ics", "/tasks/b.ics"] {
            assert!(repo
                .apply_remote(
                    &list,
                    &RemoteTaskWrite {
                        remote_id,
                        remote_version: Some("v1"),
                        remote_payload: "RAW",
                        uid: remote_id,
                        fields: &fields,
                        message_id: None,
                        start_at: None,
                        rrule: None,
                        recurrence: None,
                    },
                )
                .await
                .unwrap());
        }
        let tasks = repo.list_tasks(false).await.unwrap();
        let a = tasks
            .iter()
            .find(|task| task.id == stable_id("task", &format!("{list}\0/tasks/a.ics")))
            .unwrap();
        repo.update(&a.id, &write("Local title"), None)
            .await
            .unwrap();
        let applied = repo
            .apply_remote(
                &list,
                &RemoteTaskWrite {
                    remote_id: "/tasks/a.ics",
                    remote_version: Some("v2"),
                    remote_payload: "RAW2",
                    uid: "/tasks/a.ics",
                    fields: &write("Newer server title"),
                    message_id: None,
                    start_at: None,
                    rrule: None,
                    recurrence: None,
                },
            )
            .await
            .unwrap();
        assert!(!applied);
        assert_eq!(repo.get(&a.id).await.unwrap().title, "Local title");

        // b is gone on the server and unchanged here: removed. a has a local
        // edit and is still listed: kept.
        assert_eq!(
            repo.remove_missing(&list, &["/tasks/a.ics".to_string()])
                .await
                .unwrap(),
            Removal {
                removed: 1,
                kept_local: 0
            }
        );
        assert_eq!(repo.list_tasks(true).await.unwrap().len(), 1);

        // The list goes, but a's unsent edit keeps it (read-only, gone).
        assert_eq!(
            repo.retain_remote_lists("acc", "caldav", &[])
                .await
                .unwrap(),
            (0, 1)
        );
        let lists = repo.list_lists().await.unwrap();
        let gone = lists.iter().find(|item| item.id == list).unwrap();
        assert!(gone.gone && gone.read_only);
        // Then a goes on the server too: the edit becomes a local task.
        assert_eq!(
            repo.remove_missing(&list, &[]).await.unwrap(),
            Removal {
                removed: 0,
                kept_local: 1
            }
        );
        let kept = repo.get(&a.id).await.unwrap();
        assert_eq!(kept.title, "Local title");
        assert_ne!(kept.list_id, list);
        assert!(!kept.pending);
        // With no unsent work left the list goes for good.
        assert_eq!(
            repo.retain_remote_lists("acc", "caldav", &[])
                .await
                .unwrap(),
            (1, 0)
        );
    }
}
