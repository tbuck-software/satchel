//! Persistence for tasks and task lists.
//!
//! Local lists are plain rows. Remote lists mirror a provider collection; a
//! local change to one of their tasks sets `pending_op` in the same statement,
//! and the sync engine (`crate::tasks::sync`) pushes it and clears the mark.
//! Rows marked for deletion stay hidden until the server confirmed it.
use serde::Serialize;
use sqlx::{FromRow, SqlitePool};
use thiserror::Error;

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
    pub read_only: bool,
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
        t.priority, t.completed_at,
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
        completed_at, message_id, remote_id, remote_version, remote_payload, pending_op, updated_at
     FROM tasks";

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
}

#[derive(Debug, FromRow)]
struct TaskState {
    list_id: String,
    pending_op: Option<String>,
    remote_id: Option<String>,
    reminder_at: Option<String>,
    updated_at: String,
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
            "SELECT l.id, l.account_id, l.provider, l.name, l.color, l.read_only, l.last_synced_at,
                    (SELECT COUNT(*) FROM tasks t
                      WHERE t.list_id = l.id AND t.completed_at IS NULL
                        AND COALESCE(t.pending_op, '') <> 'delete') AS open_count
             FROM task_lists l
             LEFT JOIN accounts a ON a.id = l.account_id
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

    /// Delete a local list with its tasks. The default list comes back empty
    /// the next time lists are read if it was the last local one.
    pub async fn delete_local_list(&self, id: &str) -> Result<(), TaskError> {
        self.require_local_list(id).await?;
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
        sqlx::query_as::<_, ListState>("SELECT provider, read_only FROM task_lists WHERE id = ?")
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
            "SELECT list_id, pending_op, remote_id, reminder_at, updated_at FROM tasks
             WHERE id = ? AND COALESCE(pending_op, '') <> 'delete'",
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
        let reminder_changed = fields.reminder_at != state.reminder_at;
        let result = sqlx::query(
            "UPDATE tasks SET title = ?, notes = ?, due_at = ?, reminder_at = ?, priority = ?,
                    completed_at = ?, pending_op = ?,
                    reminded_at = CASE WHEN ? THEN NULL ELSE reminded_at END,
                    updated_at = ?
             WHERE id = ? AND updated_at = ?",
        )
        .bind(&fields.title)
        .bind(fields.notes.as_deref())
        .bind(fields.due_at.as_deref())
        .bind(fields.reminder_at.as_deref())
        .bind(i64::from(fields.priority))
        .bind(fields.completed_at.as_deref())
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
        sqlx::query(
            "UPDATE tasks SET completed_at = ?, pending_op = ?, updated_at = ? WHERE id = ?",
        )
        .bind(completed_at)
        .bind(pending)
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
        if list.read_only {
            return Err(TaskError::ListReadOnly);
        }
        if list.provider == LOCAL_PROVIDER || state.remote_id.is_none() {
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
    pub async fn claim_due_reminders(&self, now_utc: &str) -> Result<Vec<TaskRecord>, TaskError> {
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
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
                (id, account_id, provider, remote_id, name, color, read_only, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name, color = excluded.color, read_only = excluded.read_only,
                updated_at = CASE WHEN (task_lists.name, task_lists.color, task_lists.read_only)
                    IS (excluded.name, excluded.color, excluded.read_only)
                    THEN task_lists.updated_at ELSE excluded.updated_at END",
        )
        .bind(&id)
        .bind(list.account_id)
        .bind(list.provider)
        .bind(list.remote_id)
        .bind(list.name)
        .bind(list.color)
        .bind(list.read_only)
        .bind(&at)
        .bind(&at)
        .execute(self.pool)
        .await?;
        Ok(id)
    }

    /// Drop this account's lists of `provider` that the server no longer has.
    pub async fn retain_remote_lists(
        &self,
        account_id: &str,
        provider: &str,
        keep: &[String],
    ) -> Result<u64, TaskError> {
        let existing = sqlx::query_scalar::<_, String>(
            "SELECT id FROM task_lists WHERE account_id = ? AND provider = ?",
        )
        .bind(account_id)
        .bind(provider)
        .fetch_all(self.pool)
        .await?;
        let mut removed = 0;
        for id in existing.into_iter().filter(|id| !keep.contains(id)) {
            removed += sqlx::query("DELETE FROM task_lists WHERE id = ?")
                .bind(id)
                .execute(self.pool)
                .await?
                .rows_affected();
        }
        Ok(removed)
    }

    pub async fn mark_list_synced(&self, list_id: &str) -> Result<(), TaskError> {
        sqlx::query("UPDATE task_lists SET last_synced_at = ? WHERE id = ?")
            .bind(now())
            .bind(list_id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    pub async fn pending(&self, list_id: &str) -> Result<Vec<PendingTask>, TaskError> {
        sqlx::query_as::<_, PendingTask>(&format!(
            "{PENDING_SELECT} WHERE list_id = ? AND pending_op IS NOT NULL ORDER BY updated_at"
        ))
        .bind(list_id)
        .fetch_all(self.pool)
        .await
        .map_err(Into::into)
    }

    /// Record a successful push. An edit made while the push was in flight
    /// keeps its pending mark, now as an update of the stored resource.
    pub async fn mark_pushed(
        &self,
        task: &PendingTask,
        remote_id: &str,
        remote_version: Option<&str>,
        remote_payload: &str,
    ) -> Result<(), TaskError> {
        sqlx::query(
            "UPDATE tasks SET remote_id = ?, remote_version = ?, remote_payload = ?,
                    pending_op = CASE WHEN updated_at = ? THEN NULL ELSE 'update' END
             WHERE id = ?",
        )
        .bind(remote_id)
        .bind(remote_version)
        .bind(remote_payload)
        .bind(&task.updated_at)
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
                (id, list_id, uid, title, notes, due_at, reminder_at, priority, completed_at,
                 thread_id, message_id, remote_id, remote_version, remote_payload, pending_op,
                 created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?,
                     (SELECT thread_id FROM messages WHERE ? IS NOT NULL
                        AND account_id IN (SELECT id FROM accounts) AND message_id_header = ?
                        AND thread_id IS NOT NULL LIMIT 1),
                     ?, ?, ?, ?, NULL, ?, ?)
             ON CONFLICT(list_id, remote_id) WHERE remote_id IS NOT NULL DO UPDATE SET
                uid = excluded.uid, title = excluded.title, notes = excluded.notes,
                due_at = excluded.due_at,
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
    /// remote id yet and stay; a pending edit of a task deleted on the server
    /// loses, as the calendar's would.
    pub async fn remove_missing(&self, list_id: &str, seen: &[String]) -> Result<u64, TaskError> {
        let mut removed = 0;
        for (remote_id, _, _) in self.remote_versions(list_id).await? {
            if seen.contains(&remote_id) {
                continue;
            }
            removed += sqlx::query("DELETE FROM tasks WHERE list_id = ? AND remote_id = ?")
                .bind(list_id)
                .bind(&remote_id)
                .execute(self.pool)
                .await?
                .rows_affected();
        }
        Ok(removed)
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
                read_only: false,
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
                read_only: false,
            })
            .await
            .unwrap();
        let task = repo.create(&list, &write("One"), None, None).await.unwrap();
        let snapshot = repo.pending(&list).await.unwrap().remove(0);
        // Different timestamp than the snapshot.
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        repo.update(&task.id, &write("Two"), None).await.unwrap();
        repo.mark_pushed(&snapshot, "/tasks/one.ics", Some("e1"), "RAW")
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
                read_only: false,
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
                },
            )
            .await
            .unwrap();
        assert!(!applied);
        assert_eq!(repo.get(&a.id).await.unwrap().title, "Local title");

        assert_eq!(
            repo.remove_missing(&list, &["/tasks/a.ics".to_string()])
                .await
                .unwrap(),
            1
        );
        assert_eq!(repo.list_tasks(true).await.unwrap().len(), 1);

        let kept = vec![];
        assert_eq!(
            repo.retain_remote_lists("acc", "caldav", &kept)
                .await
                .unwrap(),
            1
        );
        assert!(repo.list_tasks(true).await.unwrap().is_empty());
    }
}
