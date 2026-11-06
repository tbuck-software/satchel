//! Two-way task sync, independent of the provider.
//!
//! Order per list: push what changed here first, then pull. Pushes carry the
//! version the change was made against (an ETag for CalDAV); a server that
//! moved on refuses it, and then the server wins — the rule calendar edits
//! follow (`calendar::save_event` writes with `If-Match` and keeps the server's
//! copy when that fails). The pull then brings the server's version.
//!
//! A provider implements [`TaskRemote`]; CalDAV is
//! `providers::dav::tasks::CalDavTaskRemote`. Google Tasks and Microsoft To Do
//! fit the same trait: lists, per-task versions (etag / changeKey), fetch,
//! put with a precondition, delete, and a codec for their payload.
use async_trait::async_trait;
use serde::Serialize;
use sqlx::SqlitePool;

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
    /// Anything else (network, authentication, a malformed answer). The
    /// change stays pending and the next sync tries again.
    Other(String),
}

impl std::fmt::Display for RemoteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Conflict => f.write_str("remote task changed"),
            Self::Gone => f.write_str("remote task is gone"),
            Self::Other(detail) => f.write_str(detail),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteList {
    pub remote_id: String,
    pub name: String,
    pub color: Option<String>,
    pub read_only: bool,
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
    /// with [`RemoteError::Conflict`] when `version` is no longer current.
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
    /// Local changes the server refused because its copy had moved on.
    pub conflicts: usize,
}

impl TaskSyncSummary {
    pub fn merge(&mut self, other: &Self) {
        self.lists += other.lists;
        self.pulled += other.pulled;
        self.pushed += other.pushed;
        self.removed += other.removed;
        self.conflicts += other.conflicts;
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
            })
            .await?,
            list.remote_id.clone(),
        ));
    }
    let keep = ids.iter().map(|(id, _)| id.clone()).collect::<Vec<_>>();
    repo.retain_remote_lists(account_id, remote.provider(), &keep)
        .await?;

    let mut summary = TaskSyncSummary {
        lists: ids.len(),
        ..Default::default()
    };
    let mut first_error = None;
    for (list_id, remote_list) in &ids {
        let result = async {
            push_list(&repo, remote, list_id, remote_list, &mut summary).await?;
            pull_list(&repo, remote, list_id, remote_list, &mut summary).await?;
            repo.mark_list_synced(list_id).await?;
            Ok::<_, SyncError>(())
        }
        .await;
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
         WHERE account_id = ? AND provider = ? AND remote_id IS NOT NULL",
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

async fn push_list(
    repo: &TaskRepository<'_>,
    remote: &dyn TaskRemote,
    list_id: &str,
    remote_list: &str,
    summary: &mut TaskSyncSummary,
) -> Result<(), SyncError> {
    for task in repo.pending(list_id).await? {
        let outcome = match (task.pending_op.as_str(), task.remote_id.as_deref()) {
            ("delete", Some(remote_id)) => {
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
            ("delete", None) => {
                repo.remove(&task.id).await?;
                Ok(())
            }
            (_, remote_id) => {
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
                        )
                        .await?;
                        Ok(())
                    }
                    Err(RemoteError::Gone) => {
                        // Deleted on the server while edited here: the server wins.
                        summary.conflicts += 1;
                        repo.remove(&task.id).await?;
                        continue;
                    }
                    Err(error) => Err(error),
                }
            }
        };
        match outcome {
            Ok(()) => summary.pushed += 1,
            Err(RemoteError::Conflict) => {
                summary.conflicts += 1;
                repo.yield_to_server(&task.id).await?;
            }
            Err(error) => return Err(error.into()),
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
    let changed = server
        .iter()
        .filter(
            |(remote_id, version)| match local.iter().find(|(id, _, _)| id == remote_id) {
                None => true,
                Some((_, _, true)) => false,
                Some((_, local_version, false)) => {
                    version.is_none() || local_version.as_ref() != version.as_ref()
                }
            },
        )
        .map(|(remote_id, _)| remote_id.clone())
        .collect::<Vec<_>>();
    if !changed.is_empty() {
        for item in remote.fetch(remote_list, &changed).await? {
            let Some(fields) = remote.decode(&item.payload) else {
                continue;
            };
            let write = write_of(&fields);
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
    summary.removed += repo.remove_missing(list_id, &seen).await? as usize;
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
    }

    impl FakeServer {
        fn with_list() -> Self {
            Self {
                lists: vec![RemoteList {
                    remote_id: "/tasks/".into(),
                    name: "Tasks".into(),
                    color: Some("#ff0000".into()),
                    read_only: false,
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
            if self.fail_puts {
                return Err(RemoteError::Other("offline".into()));
            }
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
            })
        }
        async fn delete(&self, remote_id: &str, version: Option<&str>) -> Result<(), RemoteError> {
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
        assert!(push_account(&pool, "acc", &server).await.is_err());
        assert_eq!(repo.pending(&list_id()).await.unwrap().len(), 1);
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
