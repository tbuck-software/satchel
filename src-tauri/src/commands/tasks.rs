//! Task commands: lists and tasks, the mail → task link, and task sync.
//!
//! Local lists answer from the database alone. A change to a task in a
//! remote list is stored with its pending mark first and then pushed in the
//! background, so ticking a task off works offline and the server hears of
//! it as soon as it can.
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::commands::error::{CommandError, CommandResult};
use satchel_core::db::accounts::AccountRepository;
use satchel_core::providers::dav::tasks::CalDavTaskRemote;
use satchel_core::tasks::sync::{self, TaskRemote, TaskSyncSummary};
use satchel_core::tasks::{
    validate, TaskError, TaskInput, TaskInputError, TaskListRecord, TaskRecord, TaskRepository,
};

const TASK_SYNC_ERROR_KEY: &str = "backend.errors.groupwareTaskSyncFailed";

impl From<TaskError> for CommandError {
    fn from(error: TaskError) -> Self {
        let key = match error {
            TaskError::TaskNotFound => "backend.errors.taskNotFound",
            TaskError::ListNotFound => "backend.errors.taskListNotFound",
            TaskError::ListReadOnly => "backend.errors.taskListReadOnly",
            TaskError::ListNotLocal => "backend.errors.taskListNotLocal",
            TaskError::TaskChanged => "backend.errors.taskChanged",
            TaskError::Persistence(error) => {
                tracing::error!(%error, "task persistence failed");
                "backend.errors.taskPersistenceFailed"
            }
        };
        Self::new(key)
    }
}

impl From<TaskInputError> for CommandError {
    fn from(error: TaskInputError) -> Self {
        Self::new(error.key())
    }
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/tasks/", optional_fields = nullable))]
pub struct TaskFieldsInput {
    pub title: String,
    #[serde(default)]
    pub notes: Option<String>,
    /// `YYYY-MM-DD` or an RFC 3339 instant.
    #[serde(default)]
    pub due_at: Option<String>,
    /// RFC 3339 instant.
    #[serde(default)]
    pub reminder_at: Option<String>,
    /// 0 none, 1 high, 5 medium, 9 low (iCalendar scale).
    #[serde(default)]
    pub priority: u8,
    #[serde(default)]
    pub completed_at: Option<String>,
}

impl From<TaskFieldsInput> for TaskInput {
    fn from(input: TaskFieldsInput) -> Self {
        Self {
            title: input.title,
            notes: input.notes,
            due_at: input.due_at,
            reminder_at: input.reminder_at,
            priority: input.priority,
            completed_at: input.completed_at,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/tasks/", optional_fields = nullable))]
pub struct CreateTaskInput {
    pub list_id: String,
    pub fields: TaskFieldsInput,
    /// The thread the task is made from; its newest Message-ID is stored too.
    #[serde(default)]
    pub thread_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/tasks/", optional_fields = nullable))]
pub struct UpdateTaskInput {
    pub id: String,
    /// The revision the editor opened; a newer one (a sync) wins.
    #[serde(default)]
    pub expected_updated_at: Option<String>,
    pub fields: TaskFieldsInput,
}

#[derive(Debug, Clone, Default, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/tasks/"))]
pub struct TasksSyncResult {
    pub summary: TaskSyncSummary,
    /// Encoded `{ key, params }` errors, one per account that failed.
    pub errors: Vec<String>,
}

/// The provider seam on the app side: which remote serves an account. Google
/// Tasks and Microsoft To Do join here.
fn remote_for(
    config: &satchel_core::accounts::AccountConfig,
) -> Result<Option<Box<dyn TaskRemote>>, String> {
    Ok(CalDavTaskRemote::from_account_config(config)?
        .map(|remote| Box::new(remote) as Box<dyn TaskRemote>))
}

async fn account_remote(
    pool: &SqlitePool,
    account_id: &str,
) -> Result<Option<Box<dyn TaskRemote>>, CommandError> {
    let connection = AccountRepository::new(pool)
        .calendar_connection(account_id)
        .await?;
    remote_for(&connection.config).map_err(CommandError::unexpected)
}

/// Push a remote list's pending changes in the background. A failure leaves
/// them pending for the next sync; the edit itself already succeeded.
async fn push_soon(pool: &SqlitePool, list_id: &str) {
    let Ok(Some(account_id)) = TaskRepository::new(pool)
        .remote_account_of_list(list_id)
        .await
    else {
        return;
    };
    let pool = pool.clone();
    tauri::async_runtime::spawn(async move {
        match account_remote(&pool, &account_id).await {
            Ok(Some(remote)) => {
                if let Err(error) = sync::push_account(&pool, &account_id, remote.as_ref()).await {
                    tracing::info!(%error, %account_id, "task push deferred to the next sync");
                }
            }
            Ok(None) => {}
            Err(error) => tracing::info!(?error, %account_id, "task push skipped"),
        }
    });
}

#[tauri::command]
pub async fn list_task_lists(
    pool: tauri::State<'_, SqlitePool>,
) -> CommandResult<Vec<TaskListRecord>> {
    let repository = TaskRepository::new(pool.inner());
    repository.ensure_default_list().await?;
    repository.list_lists().await.map_err(Into::into)
}

#[tauri::command]
pub async fn list_tasks(
    pool: tauri::State<'_, crate::db::ReadPool>,
    include_completed: Option<bool>,
) -> CommandResult<Vec<TaskRecord>> {
    TaskRepository::new(pool.inner().as_ref())
        .list_tasks(include_completed.unwrap_or(false))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn create_task(
    pool: tauri::State<'_, SqlitePool>,
    input: CreateTaskInput,
) -> CommandResult<TaskRecord> {
    let fields = validate(input.fields.into())?;
    let message_id = match input.thread_id.as_deref() {
        Some(thread_id) => sqlx::query_scalar::<_, Option<String>>(
            "SELECT message_id_header FROM messages
             WHERE thread_id = ? AND message_id_header IS NOT NULL
             ORDER BY date DESC LIMIT 1",
        )
        .bind(thread_id)
        .fetch_optional(pool.inner())
        .await
        .map_err(TaskError::from)?
        .flatten(),
        None => None,
    };
    let task = TaskRepository::new(pool.inner())
        .create(
            &input.list_id,
            &fields,
            input.thread_id.as_deref(),
            message_id.as_deref(),
        )
        .await?;
    push_soon(pool.inner(), &task.list_id).await;
    Ok(task)
}

#[tauri::command]
pub async fn update_task(
    pool: tauri::State<'_, SqlitePool>,
    input: UpdateTaskInput,
) -> CommandResult<TaskRecord> {
    let fields = validate(input.fields.into())?;
    let task = TaskRepository::new(pool.inner())
        .update(&input.id, &fields, input.expected_updated_at.as_deref())
        .await?;
    push_soon(pool.inner(), &task.list_id).await;
    Ok(task)
}

#[tauri::command]
pub async fn set_task_completed(
    pool: tauri::State<'_, SqlitePool>,
    id: String,
    completed: bool,
) -> CommandResult<TaskRecord> {
    let completed_at =
        completed.then(|| chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string());
    let task = TaskRepository::new(pool.inner())
        .set_completed(&id, completed_at.as_deref())
        .await?;
    push_soon(pool.inner(), &task.list_id).await;
    Ok(task)
}

#[tauri::command]
pub async fn delete_task(pool: tauri::State<'_, SqlitePool>, id: String) -> CommandResult<()> {
    let repository = TaskRepository::new(pool.inner());
    let list_id = repository.get(&id).await?.list_id;
    repository.delete(&id).await?;
    push_soon(pool.inner(), &list_id).await;
    Ok(())
}

/// Undo a deletion that has not reached the server yet.
#[tauri::command]
pub async fn restore_task(
    pool: tauri::State<'_, SqlitePool>,
    id: String,
) -> CommandResult<TaskRecord> {
    TaskRepository::new(pool.inner())
        .undelete(&id)
        .await
        .map_err(Into::into)
}

fn list_name(name: &str) -> CommandResult<&str> {
    let name = name.trim();
    if name.is_empty() {
        return Err(CommandError::new("backend.errors.taskListNameRequired"));
    }
    Ok(name)
}

#[tauri::command]
pub async fn create_task_list(
    pool: tauri::State<'_, SqlitePool>,
    name: String,
) -> CommandResult<TaskListRecord> {
    TaskRepository::new(pool.inner())
        .create_local_list(list_name(&name)?)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn rename_task_list(
    pool: tauri::State<'_, SqlitePool>,
    id: String,
    name: String,
) -> CommandResult<TaskListRecord> {
    TaskRepository::new(pool.inner())
        .rename_local_list(&id, list_name(&name)?)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn delete_task_list(pool: tauri::State<'_, SqlitePool>, id: String) -> CommandResult<()> {
    TaskRepository::new(pool.inner())
        .delete_local_list(&id)
        .await
        .map_err(Into::into)
}

/// Push and pull every account with a task provider (or just one).
#[tauri::command]
pub async fn sync_tasks(
    pool: tauri::State<'_, SqlitePool>,
    account_id: Option<String>,
) -> CommandResult<TasksSyncResult> {
    let accounts = sqlx::query_as::<_, (String, String)>(
        "SELECT id, email FROM accounts WHERE is_active = 1 AND (? IS NULL OR id = ?)",
    )
    .bind(account_id.as_deref())
    .bind(account_id.as_deref())
    .fetch_all(pool.inner())
    .await
    .map_err(TaskError::from)?;
    let mut result = TasksSyncResult::default();
    for (account_id, email) in accounts {
        let remote = match account_remote(pool.inner(), &account_id).await {
            Ok(Some(remote)) => remote,
            Ok(None) => continue,
            Err(error) => {
                tracing::warn!(?error, %account_id, "task sync could not read the account");
                result.errors.push(
                    CommandError::new(TASK_SYNC_ERROR_KEY)
                        .with_param("account", email.as_str())
                        .encoded(),
                );
                continue;
            }
        };
        match sync::sync_account(pool.inner(), &account_id, remote.as_ref()).await {
            Ok(summary) => result.summary.merge(&summary),
            Err(error) => {
                tracing::warn!(%error, %account_id, "task sync failed");
                result.errors.push(
                    CommandError::new(TASK_SYNC_ERROR_KEY)
                        .with_param("account", email.as_str())
                        .encoded(),
                );
            }
        }
    }
    Ok(result)
}
