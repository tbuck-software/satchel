//! Two-way task sync, independent of the provider.
//!
//! Order per list: push what changed here, then pull — the pull runs even when
//! some pushes failed. Pushes carry the version the change was made against
//! (an ETag for CalDAV); a server that moved on refuses it, and then the server
//! wins — the rule calendar edits follow (`calendar::save_event` writes with
//! `If-Match` and keeps the server's copy when that fails). The summary counts
//! those cases so the UI can say so instead of changing tasks silently.
//!
//! Nothing is written without a version: a task that has a server copy but no
//! known ETag waits for the next pull to learn it.
//!
//! A provider implements [`TaskRemote`]; CalDAV is
//! `providers::dav::tasks::CalDavTaskRemote`. Google Tasks and Microsoft To Do
//! fit the same trait: lists, per-task versions (etag / changeKey), fetch,
//! put with a precondition, delete, and a codec for their payload.
use async_trait::async_trait;
use serde::Serialize;
use sqlx::SqlitePool;

use super::recurrence;
use super::vtodo::TaskFields;
use crate::db::tasks::{
    PendingTask, RemoteListWrite, RemoteTaskWrite, TaskError, TaskRepository, TaskWrite,
};

/// One sync at a time per process: a background push after an edit and the
/// periodic sync would otherwise both create the same new task.
static SYNC_GATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteError {
    /// The precondition failed: the server's copy is not the one we edited.
    Conflict,
    /// The resource is gone on the server.
    Gone,
    /// The server refuses writes to this list (403): it is read-only.
    Forbidden,
    /// Anything else (network, authentication, a malformed answer). The
    /// change stays pending and the task retries after a backoff.
    Other(String),
}

impl std::fmt::Display for RemoteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Conflict => f.write_str("remote task changed"),
            Self::Gone => f.write_str("remote task is gone"),
            Self::Forbidden => f.write_str("remote list is read-only"),
            Self::Other(detail) => f.write_str(detail),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteList {
    pub remote_id: String,
    pub name: String,
    pub color: Option<String>,
    /// None when the server does not say (no privilege set).
    pub read_only: Option<bool>,
    /// The collection also holds events.
    pub mixed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteItem {
    pub remote_id: String,
    pub version: Option<String>,
    pub payload: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Written {
    pub remote_id: String,
    pub version: Option<String>,
    /// The resource already existed as this task (an earlier write that
    /// landed before the app recorded it) and was taken over, not written:
    /// the local change still has to go out, now with `If-Match`.
    pub adopted: bool,
}

/// A task provider, as the sync engine sees it.
#[async_trait]
pub trait TaskRemote: Send + Sync {
    /// The `task_lists.provider` value of this provider's lists.
    fn provider(&self) -> &'static str;
    async fn lists(&self) -> Result<Vec<RemoteList>, RemoteError>;
    /// Every task in a list with its current version.
    async fn versions(&self, list: &str) -> Result<Vec<(String, Option<String>)>, RemoteError>;
    async fn fetch(&self, list: &str, ids: &[String]) -> Result<Vec<RemoteItem>, RemoteError>;
    /// Create (`remote_id` None) or replace a task. A replacement must fail
    /// with [`RemoteError::Conflict`] when `version` is no longer current, and
    /// is only ever asked for with a version.
    async fn put(
        &self,
        list: &str,
        uid: &str,
        remote_id: Option<&str>,
        version: Option<&str>,
        payload: &str,
    ) -> Result<Written, RemoteError>;
    async fn delete(&self, remote_id: &str, version: Option<&str>) -> Result<(), RemoteError>;
    /// Read a payload; `None` for one that holds no task.
    fn decode(&self, payload: &str) -> Option<TaskFields>;
    /// Write a payload, patching `previous` so foreign fields survive.
    fn encode(&self, task: &TaskFields, previous: Option<&str>) -> String;
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "core/tasks/sync/"))]
pub struct TaskSyncSummary {
    pub lists: usize,
    pub pulled: usize,
    pub pushed: usize,
    pub removed: usize,
    /// Local edits the server refused because its copy had moved on; the
    /// server's version replaced them.
    pub conflicts: usize,
    /// Tasks deleted here that the server still had changed: they came back.
    pub restored: usize,
    /// Pushes that failed and wait for a retry.
    pub failed: usize,
    /// Local changes to lists the server refused as read-only; dropped.
    pub read_only_dropped: usize,
    /// Unsent edits of tasks the server deleted, kept as local tasks.
    pub kept_local: usize,
    /// Lists the server dropped that stay, read-only, for their unsent work.
    pub lists_gone: usize,
}

impl TaskSyncSummary {
    pub fn merge(&mut self, other: &Self) {
        self.lists += other.lists;
        self.pulled += other.pulled;
        self.pushed += other.pushed;
        self.removed += other.removed;
        self.conflicts += other.conflicts;
        self.restored += other.restored;
        self.failed += other.failed;
        self.read_only_dropped += other.read_only_dropped;
        self.kept_local += other.kept_local;
        self.lists_gone += other.lists_gone;
    }

    /// Tasks on screen changed, or there is something to tell the person.
    pub fn changed_tasks(&self) -> bool {
        self.pulled
            + self.removed
            + self.kept_local
            + self.restored
            + self.conflicts
            + self.read_only_dropped
            + self.lists_gone
            > 0
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error("task provider failed: {0}")]
    Remote(RemoteError),
    #[error(transparent)]
    Store(#[from] TaskError),
}

impl From<RemoteError> for SyncError {
    fn from(error: RemoteError) -> Self {
        Self::Remote(error)
    }
}

impl From<sqlx::Error> for SyncError {
    fn from(error: sqlx::Error) -> Self {
        Self::Store(TaskError::Persistence(error))
    }
}

fn fields_of(task: &PendingTask) -> TaskFields {
    TaskFields {
        uid: task.uid.clone(),
        title: task.title.clone(),
        notes: task.notes.clone(),
        due_at: task.due_at.clone(),
        reminder_at: task.reminder_at.clone(),
        priority: u8::try_from(task.priority).unwrap_or(0),
        completed_at: task.completed_at.clone(),
        message_id: task.message_id.clone(),
        start_at: task.start_at.clone(),
        rrule: task.rrule.clone(),
        rrule_extras: false,
    }
}

fn write_of(fields: &TaskFields) -> TaskWrite {
    TaskWrite {
        title: fields.title.clone(),
        notes: fields.notes.clone(),
        due_at: fields.due_at.clone(),
        reminder_at: fields.reminder_at.clone(),
        priority: fields.priority,
        completed_at: fields.completed_at.clone(),
    }
}

/// Discover the account's lists, then push and pull each one.
pub async fn sync_account(
    pool: &SqlitePool,
    account_id: &str,
    remote: &dyn TaskRemote,
) -> Result<TaskSyncSummary, SyncError> {
    let _gate = SYNC_GATE.lock().await;
    let repo = TaskRepository::new(pool);
    let lists = remote.lists().await?;
    let mut ids = Vec::with_capacity(lists.len());
    for list in &lists {
        ids.push((
            repo.upsert_remote_list(&RemoteListWrite {
                account_id,
                provider: remote.provider(),
                remote_id: &list.remote_id,
                name: &list.name,
                color: list.color.as_deref(),
                read_only: list.read_only,
                mixed: list.mixed,
            })
            .await?,
            list.remote_id.clone(),
        ));
    }
    let keep = ids.iter().map(|(id, _)| id.clone()).collect::<Vec<_>>();
    let (_, lists_gone) = repo
        .retain_remote_lists(account_id, remote.provider(), &keep)
        .await?;

    let mut summary = TaskSyncSummary {
        lists: ids.len(),
        lists_gone: lists_gone as usize,
        ..Default::default()
    };
    let mut first_error = None;
    for (list_id, remote_list) in &ids {
        // A push that failed part-way does not keep the server's changes out.
        let pushed = push_list(&repo, remote, list_id, remote_list, &mut summary).await;
        let pulled = pull_list(&repo, remote, list_id, remote_list, &mut summary).await;
        let result = pushed.and(pulled);
        let result = match result {
            Ok(()) => repo
                .mark_list_synced(list_id)
                .await
                .map_err(SyncError::from),
            Err(error) => Err(error),
        };
        if let Err(error) = result {
            tracing::warn!(%error, account_id, "task list sync failed");
            first_error.get_or_insert(error);
        }
    }
    match first_error {
        Some(error) => Err(error),
        None => Ok(summary),
    }
}

/// Push pending changes of the account's known lists without discovery or a
/// pull: what an edit kicks off so it reaches the server right away.
pub async fn push_account(
    pool: &SqlitePool,
    account_id: &str,
    remote: &dyn TaskRemote,
) -> Result<TaskSyncSummary, SyncError> {
    let _gate = SYNC_GATE.lock().await;
    let repo = TaskRepository::new(pool);
    let lists = sqlx::query_as::<_, (String, String)>(
        "SELECT id, remote_id FROM task_lists
         WHERE account_id = ? AND provider = ? AND remote_id IS NOT NULL AND gone = 0",
    )
    .bind(account_id)
    .bind(remote.provider())
    .fetch_all(pool)
    .await?;
    let mut summary = TaskSyncSummary::default();
    for (list_id, remote_list) in &lists {
        push_list(&repo, remote, list_id, remote_list, &mut summary).await?;
    }
    Ok(summary)
}

/// Push each pending task on its own: a failure backs that task off and the
/// rest of the list goes on. Only a local storage error ends the list.
async fn push_list(
    repo: &TaskRepository<'_>,
    remote: &dyn TaskRemote,
    list_id: &str,
    remote_list: &str,
    summary: &mut TaskSyncSummary,
) -> Result<(), SyncError> {
    for task in repo.pending(list_id).await? {
        let creating = task.remote_id.is_none();
        if !creating && task.remote_version.is_none() {
            // A server copy without a known version: the pull learns it first.
            continue;
        }
        let deleting = task.pending_op == "delete";
        let outcome = match (deleting, task.remote_id.as_deref()) {
            (true, Some(remote_id)) => {
                match remote
                    .delete(remote_id, task.remote_version.as_deref())
                    .await
                {
                    Ok(()) | Err(RemoteError::Gone) => {
                        repo.remove(&task.id).await?;
                        Ok(())
                    }
                    Err(error) => Err(error),
                }
            }
            (true, None) => {
                repo.remove(&task.id).await?;
                Ok(())
            }
            (false, remote_id) => {
                let fields = fields_of(&task);
                let payload = remote.encode(&fields, task.remote_payload.as_deref());
                match remote
                    .put(
                        remote_list,
                        &task.uid,
                        remote_id,
                        task.remote_version.as_deref(),
                        &payload,
                    )
                    .await
                {
                    Ok(written) => {
                        repo.mark_pushed(
                            &task,
                            &written.remote_id,
                            written.version.as_deref(),
                            &payload,
                            written.adopted,
                        )
                        .await?;
                        Ok(())
                    }
                    Err(error) => Err(error),
                }
            }
        };
        match outcome {
            Ok(()) => summary.pushed += 1,
            Err(RemoteError::Conflict) => {
                if deleting {
                    summary.restored += 1;
                } else {
                    summary.conflicts += 1;
                }
                repo.yield_to_server(&task.id).await?;
            }
            Err(RemoteError::Gone) => {
                // Deleted on the server while edited here: keep the edit.
                summary.kept_local += 1;
                repo.keep_as_local(&task.id).await?;
            }
            Err(RemoteError::Forbidden) => {
                summary.read_only_dropped += 1;
                repo.mark_list_read_only(list_id).await?;
                repo.yield_to_server(&task.id).await?;
            }
            Err(RemoteError::Other(error)) => {
                tracing::info!(%error, task = %task.id, "task push failed, will retry");
                summary.failed += 1;
                repo.record_push_failure(&task.id).await?;
            }
        }
    }
    Ok(())
}

async fn pull_list(
    repo: &TaskRepository<'_>,
    remote: &dyn TaskRemote,
    list_id: &str,
    remote_list: &str,
    summary: &mut TaskSyncSummary,
) -> Result<(), SyncError> {
    let server = remote.versions(remote_list).await?;
    let local = repo.remote_versions(list_id).await?;
    let mut unknown_version = Vec::new();
    let changed = server
        .iter()
        .filter(|(remote_id, version)| {
            match local.iter().find(|(id, _, _)| id == remote_id) {
                None => true,
                // A pending change whose version we never learnt: fetch the
                // server copy to tell our own write from someone else's.
                Some((_, None, true)) => {
                    unknown_version.push(remote_id.clone());
                    true
                }
                Some((_, _, true)) => false,
                Some((_, local_version, false)) => {
                    version.is_none() || local_version.as_ref() != version.as_ref()
                }
            }
        })
        .map(|(remote_id, _)| remote_id.clone())
        .collect::<Vec<_>>();
    if !changed.is_empty() {
        for item in remote.fetch(remote_list, &changed).await? {
            if unknown_version.contains(&item.remote_id) {
                let ours = repo.remote_payload(list_id, &item.remote_id).await?;
                if ours.as_deref() == Some(item.payload.as_str()) {
                    repo.adopt_version(list_id, &item.remote_id, item.version.as_deref())
                        .await?;
                    continue;
                }
                summary.conflicts += 1;
                repo.drop_pending(list_id, &item.remote_id).await?;
            }
            let Some(fields) = remote.decode(&item.payload) else {
                continue;
            };
            let write = write_of(&fields);
            let recurrence = recurrence::classify(
                fields.rrule.as_deref(),
                fields.rrule_extras,
                fields.due_at.as_deref(),
            );
            if repo
                .apply_remote(
                    list_id,
                    &RemoteTaskWrite {
                        remote_id: &item.remote_id,
                        remote_version: item.version.as_deref(),
                        remote_payload: &item.payload,
                        uid: &fields.uid,
                        fields: &write,
                        message_id: fields.message_id.as_deref(),
                        start_at: fields.start_at.as_deref(),
                        rrule: fields.rrule.as_deref(),
                        recurrence: recurrence.stored(),
                    },
                )
                .await?
            {
                summary.pulled += 1;
            }
        }
    }
    let seen = server
        .into_iter()
        .map(|(remote_id, _)| remote_id)
        .collect::<Vec<_>>();
    let removal = repo.remove_missing(list_id, &seen).await?;
    summary.removed += removal.removed as usize;
    summary.kept_local += removal.kept_local as usize;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::tasks::{remote_list_id, TaskWrite};
    use crate::tasks::vtodo;
    use sqlx::sqlite::SqlitePoolOptions;
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    /// An in-memory server with ETags, so the engine is tested without HTTP.
    #[derive(Default)]
    struct FakeServer {
        lists: Vec<RemoteList>,
        items: Mutex<BTreeMap<String, (String, u32)>>,
        fail_puts: bool,
        /// Puts of a payload containing this text fail with a server error.
        fail_matching: Option<String>,
        forbidden: bool,
        writes: Mutex<u32>,
    }

    impl FakeServer {
        fn with_list() -> Self {
            Self {
                lists: vec![RemoteList {
                    remote_id: "/tasks/".into(),
                    name: "Tasks".into(),
                    color: Some("#ff0000".into()),
                    read_only: None,
                    mixed: false,
                }],
                ..Default::default()
            }
        }

        fn seed(&self, remote_id: &str, payload: &str) {
            self.items
                .lock()
                .unwrap()
                .insert(remote_id.into(), (payload.into(), 1));
        }

        fn bump(&self, remote_id: &str, payload: &str) {
            let mut items = self.items.lock().unwrap();
            let entry = items.get_mut(remote_id).unwrap();
            *entry = (payload.into(), entry.1 + 1);
        }

        fn payload(&self, remote_id: &str) -> Option<String> {
            self.items
                .lock()
                .unwrap()
                .get(remote_id)
                .map(|(payload, _)| payload.clone())
        }
    }

    #[async_trait]
    impl TaskRemote for FakeServer {
        fn provider(&self) -> &'static str {
            "caldav"
        }
        async fn lists(&self) -> Result<Vec<RemoteList>, RemoteError> {
            Ok(self.lists.clone())
        }
        async fn versions(&self, _: &str) -> Result<Vec<(String, Option<String>)>, RemoteError> {
            Ok(self
                .items
                .lock()
                .unwrap()
                .iter()
                .map(|(id, (_, version))| (id.clone(), Some(version.to_string())))
                .collect())
        }
        async fn fetch(&self, _: &str, ids: &[String]) -> Result<Vec<RemoteItem>, RemoteError> {
            let items = self.items.lock().unwrap();
            Ok(ids
                .iter()
                .filter_map(|id| {
                    items.get(id).map(|(payload, version)| RemoteItem {
                        remote_id: id.clone(),
                        version: Some(version.to_string()),
                        payload: payload.clone(),
                    })
                })
                .collect())
        }
        async fn put(
            &self,
            list: &str,
            uid: &str,
            remote_id: Option<&str>,
            version: Option<&str>,
            payload: &str,
        ) -> Result<Written, RemoteError> {
            if self.fail_puts
                || self
                    .fail_matching
                    .as_deref()
                    .is_some_and(|text| payload.contains(text))
            {
                return Err(RemoteError::Other("offline".into()));
            }
            if self.forbidden {
                return Err(RemoteError::Forbidden);
            }
            *self.writes.lock().unwrap() += 1;
            let mut items = self.items.lock().unwrap();
            let id = remote_id.map_or_else(|| format!("{list}{uid}.ics"), str::to_string);
            let next = match (items.get(&id), remote_id) {
                (None, Some(_)) => return Err(RemoteError::Gone),
                (Some((_, current)), Some(_)) => {
                    if version != Some(current.to_string().as_str()) {
                        return Err(RemoteError::Conflict);
                    }
                    current + 1
                }
                (Some(_), None) => return Err(RemoteError::Conflict),
                (None, None) => 1,
            };
            items.insert(id.clone(), (payload.into(), next));
            Ok(Written {
                remote_id: id,
                version: Some(next.to_string()),
                adopted: false,
            })
        }
        async fn delete(&self, remote_id: &str, version: Option<&str>) -> Result<(), RemoteError> {
            *self.writes.lock().unwrap() += 1;
            let mut items = self.items.lock().unwrap();
            match items.get(remote_id) {
                None => Err(RemoteError::Gone),
                Some((_, current)) if version != Some(current.to_string().as_str()) => {
                    Err(RemoteError::Conflict)
                }
                Some(_) => {
                    items.remove(remote_id);
                    Ok(())
                }
            }
        }
        fn decode(&self, payload: &str) -> Option<TaskFields> {
            vtodo::parse(payload)
        }
        fn encode(&self, task: &TaskFields, previous: Option<&str>) -> String {
            previous.map_or_else(
                || vtodo::build(task),
                |previous| vtodo::patch(previous, task),
            )
        }
    }

    fn todo(uid: &str, title: &str) -> String {
        vtodo::build(&TaskFields {
            uid: uid.into(),
            title: title.into(),
            ..Default::default()
        })
    }

    async fn pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::db::init(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO accounts
                (id, display_name, email, provider, config, credentials_ref, sync_state,
                 color, is_active, created_at, updated_at)
             VALUES ('acc', 'acc', 'acc@example.test', 'imap', '{}', '', '{}', '#123456', 1,
                     '2026-01-01', '2026-01-01')",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    fn list_id() -> String {
        remote_list_id("acc", "caldav", "/tasks/")
    }

    fn new_task(title: &str) -> TaskWrite {
        TaskWrite {
            title: title.into(),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn one_failing_task_does_not_hold_up_the_list_or_the_pull() {
        let pool = pool().await;
        let server = FakeServer {
            fail_matching: Some("Poison".into()),
            ..FakeServer::with_list()
        };
        sync_account(&pool, "acc", &server).await.unwrap();
        let repo = TaskRepository::new(&pool);
        repo.create(&list_id(), &new_task("Poison"), None, None)
            .await
            .unwrap();
        repo.create(&list_id(), &new_task("Fine"), None, None)
            .await
            .unwrap();
        server.seed("/tasks/s.ics", &todo("s", "From the server"));

        let summary = sync_account(&pool, "acc", &server).await.unwrap();
        assert_eq!((summary.pushed, summary.failed, summary.pulled), (1, 1, 1));
        let titles = repo
            .list_tasks(false)
            .await
            .unwrap()
            .into_iter()
            .map(|task| (task.title, task.pending))
            .collect::<Vec<_>>();
        assert!(titles.contains(&("Poison".to_string(), true)));
        assert!(titles.contains(&("Fine".to_string(), false)));
        assert!(titles.contains(&("From the server".to_string(), false)));
    }

    #[tokio::test]
    async fn a_refused_write_makes_the_list_read_only_and_drops_the_change() {
        let pool = pool().await;
        let mut server = FakeServer::with_list();
        server.seed("/tasks/a.ics", &todo("a", "Shared"));
        sync_account(&pool, "acc", &server).await.unwrap();
        let repo = TaskRepository::new(&pool);
        let shared = repo.list_tasks(false).await.unwrap().remove(0);
        repo.update(&shared.id, &new_task("Renamed here"), None)
            .await
            .unwrap();
        repo.create(&list_id(), &new_task("New here"), None, None)
            .await
            .unwrap();
        server.forbidden = true;
        let summary = sync_account(&pool, "acc", &server).await.unwrap();
        assert_eq!(summary.read_only_dropped, 2);
        let lists = repo.list_lists().await.unwrap();
        assert!(
            lists
                .iter()
                .find(|list| list.id == list_id())
                .unwrap()
                .read_only
        );
        let titles = repo
            .list_tasks(false)
            .await
            .unwrap()
            .into_iter()
            .map(|task| task.title)
            .collect::<Vec<_>>();
        assert_eq!(titles, ["Shared"]);
    }

    #[tokio::test]
    async fn a_deleted_task_the_server_changed_comes_back_and_says_so() {
        let pool = pool().await;
        let server = FakeServer::with_list();
        server.seed("/tasks/a.ics", &todo("a", "Original"));
        sync_account(&pool, "acc", &server).await.unwrap();
        let repo = TaskRepository::new(&pool);
        let task = repo.list_tasks(false).await.unwrap().remove(0);
        repo.delete(&task.id).await.unwrap();
        server.bump("/tasks/a.ics", &todo("a", "Changed on the phone"));
        let summary = sync_account(&pool, "acc", &server).await.unwrap();
        assert_eq!(summary.restored, 1);
        assert_eq!(
            repo.list_tasks(false).await.unwrap()[0].title,
            "Changed on the phone"
        );
    }

    #[tokio::test]
    async fn an_edit_of_a_task_deleted_on_the_server_stays_as_a_local_task() {
        let pool = pool().await;
        let server = FakeServer::with_list();
        server.seed("/tasks/a.ics", &todo("a", "Original"));
        sync_account(&pool, "acc", &server).await.unwrap();
        let repo = TaskRepository::new(&pool);
        let task = repo.list_tasks(false).await.unwrap().remove(0);
        repo.update(&task.id, &new_task("Edited here"), None)
            .await
            .unwrap();
        server.items.lock().unwrap().remove("/tasks/a.ics");
        let summary = sync_account(&pool, "acc", &server).await.unwrap();
        assert_eq!(summary.kept_local, 1);
        let kept = repo.get(&task.id).await.unwrap();
        assert_eq!(kept.title, "Edited here");
        assert_eq!(kept.list_id, crate::db::tasks::DEFAULT_LOCAL_LIST_ID);
    }

    #[tokio::test]
    async fn nothing_is_written_without_a_version() {
        let pool = pool().await;
        let server = FakeServer::with_list();
        let payload = todo("a", "Original");
        server.seed("/tasks/a.ics", &payload);
        sync_account(&pool, "acc", &server).await.unwrap();
        let repo = TaskRepository::new(&pool);
        let task = repo.list_tasks(false).await.unwrap().remove(0);
        // As after a PUT whose response carried no ETag.
        sqlx::query("UPDATE tasks SET remote_version = NULL")
            .execute(&pool)
            .await
            .unwrap();
        repo.update(&task.id, &new_task("Edited here"), None)
            .await
            .unwrap();
        let writes = *server.writes.lock().unwrap();
        assert_eq!(push_account(&pool, "acc", &server).await.unwrap().pushed, 0);
        assert_eq!(*server.writes.lock().unwrap(), writes);
        // The pull recognises the server copy as ours and learns its version;
        // the edit then goes out with it.
        sync_account(&pool, "acc", &server).await.unwrap();
        sync_account(&pool, "acc", &server).await.unwrap();
        assert!(server
            .payload("/tasks/a.ics")
            .unwrap()
            .contains("Edited here"));
    }

    #[tokio::test]
    async fn completing_a_weekly_task_moves_it_on_the_server_too() {
        let pool = pool().await;
        let server = FakeServer::with_list();
        let raw = "BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:w\r\nSUMMARY:Bins out\r\nDUE;VALUE=DATE:20261110\r\nRRULE:FREQ=WEEKLY\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";
        server.seed("/tasks/w.ics", raw);
        sync_account(&pool, "acc", &server).await.unwrap();
        let repo = TaskRepository::new(&pool);
        let task = repo.list_tasks(false).await.unwrap().remove(0);
        assert_eq!(task.recurrence.as_deref(), Some("simple"));
        let next = repo
            .set_completed(&task.id, Some("2026-11-10T08:00:00Z"))
            .await
            .unwrap();
        assert_eq!(
            (next.completed_at, next.due_at.as_deref()),
            (None, Some("2026-11-17"))
        );
        sync_account(&pool, "acc", &server).await.unwrap();
        let stored = server.payload("/tasks/w.ics").unwrap();
        assert!(stored.contains("DUE;VALUE=DATE:20261117\r\n"));
        assert!(stored.contains("RRULE:FREQ=WEEKLY\r\n"));
        assert!(!stored.contains("STATUS:COMPLETED"));
    }

    #[tokio::test]
    async fn pulls_creates_and_pushes_both_ways() {
        let pool = pool().await;
        let server = FakeServer::with_list();
        server.seed("/tasks/a.ics", &todo("a", "From the server"));
        let summary = sync_account(&pool, "acc", &server).await.unwrap();
        assert_eq!((summary.lists, summary.pulled), (1, 1));

        let repo = TaskRepository::new(&pool);
        let lists = repo.list_lists().await.unwrap();
        assert_eq!(lists[0].name, "Tasks");
        assert_eq!(lists[0].color.as_deref(), Some("#ff0000"));
        let task = repo
            .create(
                &list_id(),
                &TaskWrite {
                    title: "From Satchel".into(),
                    ..Default::default()
                },
                None,
                Some("<mail@example.test>"),
            )
            .await
            .unwrap();
        let summary = sync_account(&pool, "acc", &server).await.unwrap();
        assert_eq!(summary.pushed, 1);
        assert!(repo.pending(&list_id()).await.unwrap().is_empty());
        let uid = sqlx::query_scalar::<_, String>("SELECT uid FROM tasks WHERE id = ?")
            .bind(&task.id)
            .fetch_one(&pool)
            .await
            .unwrap();
        let stored = server.payload(&format!("/tasks/{uid}.ics")).unwrap();
        assert!(stored.contains("X-SATCHEL-MESSAGE-ID:<mail@example.test>"));

        // Completing reaches the server as a patch of its copy.
        repo.set_completed(&task.id, Some("2026-11-06T12:00:00Z"))
            .await
            .unwrap();
        sync_account(&pool, "acc", &server).await.unwrap();
        let stored = server.payload(&format!("/tasks/{uid}.ics")).unwrap();
        assert!(stored.contains("STATUS:COMPLETED"));

        // Server-side edit arrives; nothing is pushed back.
        server.bump("/tasks/a.ics", &todo("a", "Renamed on the phone"));
        let summary = sync_account(&pool, "acc", &server).await.unwrap();
        assert_eq!((summary.pulled, summary.pushed), (1, 0));
        let titles = repo
            .list_tasks(true)
            .await
            .unwrap()
            .into_iter()
            .map(|task| task.title)
            .collect::<Vec<_>>();
        assert!(titles.contains(&"Renamed on the phone".to_string()));
    }

    #[tokio::test]
    async fn the_server_wins_a_concurrent_edit() {
        let pool = pool().await;
        let server = FakeServer::with_list();
        server.seed("/tasks/a.ics", &todo("a", "Original"));
        sync_account(&pool, "acc", &server).await.unwrap();
        let repo = TaskRepository::new(&pool);
        let task = repo.list_tasks(false).await.unwrap().remove(0);
        repo.update(
            &task.id,
            &TaskWrite {
                title: "Edited here".into(),
                ..Default::default()
            },
            None,
        )
        .await
        .unwrap();
        server.bump("/tasks/a.ics", &todo("a", "Edited there"));

        let summary = sync_account(&pool, "acc", &server).await.unwrap();
        assert_eq!(summary.conflicts, 1);
        assert_eq!(repo.get(&task.id).await.unwrap().title, "Edited there");
        assert!(server
            .payload("/tasks/a.ics")
            .unwrap()
            .contains("Edited there"));
    }

    #[tokio::test]
    async fn deletions_go_both_ways() {
        let pool = pool().await;
        let server = FakeServer::with_list();
        server.seed("/tasks/a.ics", &todo("a", "Delete here"));
        server.seed("/tasks/b.ics", &todo("b", "Delete there"));
        sync_account(&pool, "acc", &server).await.unwrap();
        let repo = TaskRepository::new(&pool);
        let here = repo
            .list_tasks(false)
            .await
            .unwrap()
            .into_iter()
            .find(|task| task.title == "Delete here")
            .unwrap();
        repo.delete(&here.id).await.unwrap();
        server.items.lock().unwrap().remove("/tasks/b.ics");

        let summary = sync_account(&pool, "acc", &server).await.unwrap();
        assert_eq!((summary.pushed, summary.removed), (1, 1));
        assert!(server.payload("/tasks/a.ics").is_none());
        assert!(repo.list_tasks(true).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_offline_push_stays_pending_and_lists_vanish_with_the_server() {
        let pool = pool().await;
        let mut server = FakeServer::with_list();
        sync_account(&pool, "acc", &server).await.unwrap();
        let repo = TaskRepository::new(&pool);
        repo.create(
            &list_id(),
            &TaskWrite {
                title: "Offline".into(),
                ..Default::default()
            },
            None,
            None,
        )
        .await
        .unwrap();
        server.fail_puts = true;
        assert_eq!(push_account(&pool, "acc", &server).await.unwrap().failed, 1);
        // Still pending, but backing off.
        assert!(repo.pending(&list_id()).await.unwrap().is_empty());
        sqlx::query("UPDATE tasks SET push_retry_at = NULL")
            .execute(&pool)
            .await
            .unwrap();
        server.fail_puts = false;
        assert_eq!(push_account(&pool, "acc", &server).await.unwrap().pushed, 1);

        server.lists.clear();
        sync_account(&pool, "acc", &server).await.unwrap();
        assert!(repo
            .list_lists()
            .await
            .unwrap()
            .iter()
            .all(|list| list.provider == "local"));
    }
}
