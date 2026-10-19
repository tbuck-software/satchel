use serde::Serialize;
use sqlx::{Executor, Sqlite, SqlitePool, Transaction};
use thiserror::Error;

use crate::sync::{
    queue::{QueuedAction, SyncQueue},
    remote_mutations::{enqueue_folder_mutation, RemoteMutationPayload, ACTION_REMOTE_MUTATION},
};

/// Derive a new mail-folder identity without discarding any provider bytes.
/// The account owns the provider namespace, so every folder creator uses the
/// same mail namespace even before its remote folders have been listed.
pub fn folder_db_id(account_id: &str, remote_folder_id: &str) -> String {
    crate::sync::provider_identity::provider_local_id(
        "fld",
        "mail",
        &[account_id, remote_folder_id],
    )
}

/// Preserve an existing folder's identity, including all persisted references.
/// Only folders that have never been stored receive a newly derived ID.
pub async fn resolve_provider_folder_id<'e>(
    executor: impl Executor<'e, Database = Sqlite>,
    account_id: &str,
    remote_folder_id: &str,
) -> Result<String, sqlx::Error> {
    let existing: Option<String> = sqlx::query_scalar(
        "SELECT id FROM folders WHERE account_id = ? AND COALESCE(remote_id, full_path) = ?",
    )
    .bind(account_id)
    .bind(remote_folder_id)
    .fetch_optional(executor)
    .await?;
    Ok(existing.unwrap_or_else(|| folder_db_id(account_id, remote_folder_id)))
}

const PROTECTED_FOLDER_TYPES: &[&str] = &[
    "inbox", "sent", "drafts", "trash", "archive", "spam", "aside",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "core/db/folders/"))]
pub struct Folder {
    pub id: String,
    pub account_id: String,
    pub name: String,
    pub full_path: String,
    pub folder_type: String,
}

#[derive(Debug, Error)]
pub enum FolderError {
    #[error("folder name is empty")]
    NameEmpty,
    #[error("folder name exceeds {max} characters")]
    NameTooLong { max: usize },
    #[error("folder name contains a hierarchy separator")]
    NameContainsSeparator,
    #[error("folder name contains control characters")]
    NameInvalidCharacters,
    #[error("folder was not found")]
    NotFound,
    #[error("system folder {name} is immutable")]
    SystemFolderImmutable { name: String },
    #[error("folder path {path} already exists")]
    AlreadyExists { path: String },
    #[error("folder {name} contains {count} messages")]
    ContainsMessages { name: String, count: i64 },
    #[error("folder {name} contains {count} children")]
    ContainsChildren { name: String, count: i64 },
    #[error("a queued message move still uses this folder")]
    PendingMove,
    #[error("folder persistence failed: {0}")]
    Persistence(#[from] sqlx::Error),
    #[error("folder mutation queue failed: {0}")]
    Queue(String),
}

#[derive(Debug, sqlx::FromRow)]
struct StoredFolder {
    id: String,
    account_id: String,
    name: String,
    full_path: String,
    folder_type: String,
    remote_id: Option<String>,
}

impl StoredFolder {
    fn to_folder(&self) -> Folder {
        Folder {
            id: self.id.clone(),
            account_id: self.account_id.clone(),
            name: self.name.clone(),
            full_path: self.full_path.clone(),
            folder_type: self.folder_type.clone(),
        }
    }
}

/// Owns custom-folder validation, durable remote intent and local projection.
///
/// Each mutation queues the provider operation before changing the local row.
/// Callers therefore cannot accidentally create a local-only folder that would
/// accept moved mail while the server knows nothing about it.
pub struct FolderRepository<'a> {
    pool: &'a SqlitePool,
    queue: &'a SyncQueue,
}

impl<'a> FolderRepository<'a> {
    pub fn new(pool: &'a SqlitePool, queue: &'a SyncQueue) -> Self {
        Self { pool, queue }
    }

    /// Find the account's preferred Abuse folder for provider moves.
    pub async fn preferred_abuse_folder_id(
        &self,
        account_id: &str,
    ) -> Result<Option<String>, FolderError> {
        sqlx::query_scalar(
            "SELECT id
             FROM folders
             WHERE account_id = ?
               AND (LOWER(name) LIKE '%abuse%' OR LOWER(full_path) LIKE '%abuse%')
             ORDER BY CASE WHEN LOWER(name) = 'abuse' THEN 0 ELSE 1 END, name
             LIMIT 1",
        )
        .bind(account_id)
        .fetch_optional(self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn create(
        &self,
        account_id: &str,
        name: &str,
        parent_path: Option<&str>,
    ) -> Result<Folder, FolderError> {
        let name = validate_name(name)?;
        let separator = hierarchy_separator(self.pool, account_id).await;
        let full_path = match parent_path.map(str::trim).filter(|path| !path.is_empty()) {
            Some(parent) => format!("{parent}{separator}{name}"),
            None => name.clone(),
        };

        if folder_path_exists(self.pool, account_id, &full_path, None).await? {
            return Err(FolderError::AlreadyExists { path: full_path });
        }

        enqueue_folder_mutation(
            self.pool,
            self.queue,
            account_id,
            RemoteMutationPayload::CreateFolder {
                path: full_path.clone(),
            },
        )
        .await
        .map_err(FolderError::Queue)?;

        let folder = Folder {
            id: folder_db_id(account_id, &full_path),
            account_id: account_id.to_string(),
            name,
            full_path,
            folder_type: "custom".to_string(),
        };
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO folders(id, account_id, name, full_path, folder_type, remote_id, sync_enabled, created_at, updated_at)
             VALUES(?, ?, ?, ?, 'custom', ?, 1, ?, ?)",
        )
        .bind(&folder.id)
        .bind(&folder.account_id)
        .bind(&folder.name)
        .bind(&folder.full_path)
        .bind(&folder.full_path)
        .bind(&now)
        .bind(&now)
        .execute(self.pool)
        .await?;

        Ok(folder)
    }

    pub async fn rename(&self, folder_id: &str, new_name: &str) -> Result<Folder, FolderError> {
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let folder = load_folder(&mut *tx, folder_id).await?;
        ensure_mutable(&folder)?;
        let name = validate_name(new_name)?;
        let separator = hierarchy_separator(&mut *tx, &folder.account_id).await;
        let parent = folder
            .full_path
            .rsplit_once(separator)
            .map(|(head, _)| head.to_string());
        let new_path = match parent {
            Some(parent) => format!("{parent}{separator}{name}"),
            None => name.clone(),
        };

        if new_path == folder.full_path {
            return Ok(folder.to_folder());
        }

        if folder_path_exists(&mut *tx, &folder.account_id, &new_path, Some(&folder.id)).await? {
            return Err(FolderError::AlreadyExists { path: new_path });
        }

        ensure_no_folder_move(&mut tx, &folder, separator).await?;
        let remote_from = folder
            .remote_id
            .clone()
            .unwrap_or_else(|| folder.full_path.clone());
        enqueue_folder_action(
            &mut tx,
            self.queue,
            &folder.account_id,
            RemoteMutationPayload::RenameFolder {
                from_path: remote_from,
                to_path: new_path.clone(),
            },
        )
        .await?;

        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "UPDATE folders SET name = ?, full_path = ?, remote_id = ?, updated_at = ? WHERE id = ?",
        )
        .bind(&name)
        .bind(&new_path)
        .bind(&new_path)
        .bind(&now)
        .bind(&folder.id)
        .execute(&mut *tx)
        .await?;

        let old_prefix = format!("{}{}", folder.full_path, separator);
        let new_prefix = format!("{new_path}{separator}");
        let old_prefix_chars = old_prefix.chars().count() as i64;
        sqlx::query(
            "UPDATE folders
             SET full_path = ? || SUBSTR(full_path, ?),
                 remote_id = CASE
                   WHEN SUBSTR(remote_id, 1, ?) = ?
                   THEN ? || SUBSTR(remote_id, ?)
                   ELSE remote_id
                 END,
                 updated_at = ?
             WHERE account_id = ? AND SUBSTR(full_path, 1, ?) = ?",
        )
        .bind(&new_prefix)
        .bind(old_prefix_chars + 1)
        .bind(old_prefix_chars)
        .bind(&old_prefix)
        .bind(&new_prefix)
        .bind(old_prefix_chars + 1)
        .bind(&now)
        .bind(&folder.account_id)
        .bind(old_prefix_chars)
        .bind(&old_prefix)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        self.queue.wake_worker();
        let mut renamed = folder.to_folder();
        renamed.name = name;
        renamed.full_path = new_path;
        Ok(renamed)
    }

    pub async fn delete(&self, folder_id: &str) -> Result<(), FolderError> {
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let folder = load_folder(&mut *tx, folder_id).await?;
        ensure_mutable(&folder)?;

        let message_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM messages WHERE folder_id = ? OR pending_folder_id = ?",
        )
        .bind(&folder.id)
        .bind(&folder.id)
        .fetch_one(&mut *tx)
        .await?;
        if message_count > 0 {
            return Err(FolderError::ContainsMessages {
                name: folder.name,
                count: message_count,
            });
        }

        let separator = hierarchy_separator(&mut *tx, &folder.account_id).await;
        let child_prefix = format!("{}{separator}", folder.full_path);
        let child_prefix_chars = child_prefix.chars().count() as i64;
        let child_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM folders
             WHERE account_id = ? AND SUBSTR(full_path, 1, ?) = ? AND id <> ?",
        )
        .bind(&folder.account_id)
        .bind(child_prefix_chars)
        .bind(&child_prefix)
        .bind(&folder.id)
        .fetch_one(&mut *tx)
        .await?;
        if child_count > 0 {
            return Err(FolderError::ContainsChildren {
                name: folder.name,
                count: child_count,
            });
        }

        ensure_no_folder_move(&mut tx, &folder, separator).await?;
        let remote_path = folder.remote_id.unwrap_or_else(|| folder.full_path.clone());
        enqueue_folder_action(
            &mut tx,
            self.queue,
            &folder.account_id,
            RemoteMutationPayload::DeleteFolder { path: remote_path },
        )
        .await?;

        sqlx::query("DELETE FROM folders WHERE id = ?")
            .bind(&folder.id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        self.queue.wake_worker();
        Ok(())
    }
}

async fn enqueue_folder_action(
    tx: &mut Transaction<'_, Sqlite>,
    queue: &SyncQueue,
    account: &str,
    payload: RemoteMutationPayload,
) -> Result<(), FolderError> {
    let payload = serde_json::to_value(payload).map_err(|e| FolderError::Queue(e.to_string()))?;
    let action = QueuedAction::new(account.into(), ACTION_REMOTE_MUTATION, payload);
    queue
        .enqueue_in_transaction(tx, &action)
        .await
        .map_err(FolderError::Queue)
}

// A rename/delete must not invalidate the immutable source or destination
// identities captured by a pending move, including a move in a child folder.
async fn ensure_no_folder_move(
    tx: &mut Transaction<'_, Sqlite>,
    folder: &StoredFolder,
    separator: char,
) -> Result<(), FolderError> {
    let payloads: Vec<String> = sqlx::query_scalar(
        "SELECT payload FROM sync_queue WHERE account_id = ? AND action_type = 'remote_mutation' AND status IN ('pending', 'scheduled', 'sending', 'failed')"
    ).bind(&folder.account_id).fetch_all(&mut **tx).await?;
    let remote = folder.remote_id.as_deref().unwrap_or(&folder.full_path);
    let prefix = format!("{remote}{separator}");
    let touches = |path: &str| path == remote || path.starts_with(&prefix);
    for payload in payloads {
        let Ok(mutation) = serde_json::from_str::<RemoteMutationPayload>(&payload) else {
            continue;
        };
        let (refs, target) = match mutation {
            RemoteMutationPayload::WorkflowMove {
                refs,
                target_remote_folder_id,
                ..
            }
            | RemoteMutationPayload::BlockedMove {
                refs,
                target_remote_folder_id,
                ..
            }
            | RemoteMutationPayload::Move {
                refs,
                target_remote_folder_id,
            } => (refs, target_remote_folder_id),
            _ => continue,
        };
        if touches(&target)
            || refs
                .iter()
                .filter_map(|r| r.source_folder.as_deref())
                .any(touches)
        {
            return Err(FolderError::PendingMove);
        }
    }
    Ok(())
}

fn validate_name(name: &str) -> Result<String, FolderError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(FolderError::NameEmpty);
    }
    if trimmed.len() > 200 {
        return Err(FolderError::NameTooLong { max: 200 });
    }
    if trimmed.contains('/') || trimmed.contains('\\') {
        return Err(FolderError::NameContainsSeparator);
    }
    if trimmed.chars().any(char::is_control) {
        return Err(FolderError::NameInvalidCharacters);
    }
    Ok(trimmed.to_string())
}

async fn hierarchy_separator<'e>(
    pool: impl Executor<'e, Database = Sqlite>,
    account_id: &str,
) -> char {
    let existing: Option<String> = sqlx::query_scalar(
        "SELECT full_path FROM folders
         WHERE account_id = ? AND (full_path LIKE '%/%' OR full_path LIKE '%.%')
         ORDER BY LENGTH(full_path) DESC LIMIT 1",
    )
    .bind(account_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();

    match existing {
        Some(path) if path.contains('/') => '/',
        Some(path) if path.contains('.') => '.',
        _ => '/',
    }
}

async fn load_folder<'e>(
    pool: impl Executor<'e, Database = Sqlite>,
    folder_id: &str,
) -> Result<StoredFolder, FolderError> {
    sqlx::query_as::<_, StoredFolder>(
        "SELECT id, account_id, name, full_path, folder_type, remote_id
         FROM folders WHERE id = ? LIMIT 1",
    )
    .bind(folder_id)
    .fetch_optional(pool)
    .await?
    .ok_or(FolderError::NotFound)
}

async fn folder_path_exists<'e>(
    pool: impl Executor<'e, Database = Sqlite>,
    account_id: &str,
    full_path: &str,
    excluding_id: Option<&str>,
) -> Result<bool, sqlx::Error> {
    let exists: i64 = sqlx::query_scalar(
        "SELECT EXISTS(
           SELECT 1 FROM folders
           WHERE account_id = ? AND LOWER(full_path) = LOWER(?)
             AND (? IS NULL OR id <> ?)
         )",
    )
    .bind(account_id)
    .bind(full_path)
    .bind(excluding_id)
    .bind(excluding_id)
    .fetch_one(pool)
    .await?;
    Ok(exists != 0)
}

fn ensure_mutable(folder: &StoredFolder) -> Result<(), FolderError> {
    if PROTECTED_FOLDER_TYPES.contains(&folder.folder_type.to_ascii_lowercase().as_str()) {
        return Err(FolderError::SystemFolderImmutable {
            name: folder.name.clone(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

    use super::{FolderError, FolderRepository};
    use crate::sync::{queue::SyncQueue, remote_mutations::RemoteMutationPayload};

    async fn fresh_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("pool");
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .expect("migrations");
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO accounts
             (id, display_name, email, provider, config, credentials_ref, is_active, created_at, updated_at)
             VALUES ('account', 'Person', 'person@example.test', 'imap', '{}', '', 1, ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("account");
        pool
    }

    #[tokio::test]
    async fn rename_updates_unicode_and_wildcard_paths_without_touching_neighbors() {
        let pool = fresh_pool().await;
        let queue = SyncQueue::new();
        let repository = FolderRepository::new(&pool, &queue);

        let parent = repository
            .create("account", "Büro%", None)
            .await
            .expect("parent");
        let child = repository
            .create("account", "Satchel", Some(&parent.full_path))
            .await
            .expect("child");
        let neighbor = repository
            .create("account", "BüroX", None)
            .await
            .expect("neighbor");
        let neighbor_child = repository
            .create("account", "Other", Some(&neighbor.full_path))
            .await
            .expect("neighbor child");
        let renamed = repository.rename(&parent.id, "Work").await.expect("rename");

        assert_eq!(renamed.full_path, "Work");
        let (child_path, child_remote_id): (String, Option<String>) =
            sqlx::query_as("SELECT full_path, remote_id FROM folders WHERE id = ?")
                .bind(&child.id)
                .fetch_one(&pool)
                .await
                .expect("child paths");
        assert_eq!(child_path, "Work/Satchel");
        assert_eq!(child_remote_id.as_deref(), Some("Work/Satchel"));
        let neighbor_path: String =
            sqlx::query_scalar("SELECT full_path FROM folders WHERE id = ?")
                .bind(&neighbor_child.id)
                .fetch_one(&pool)
                .await
                .expect("neighbor path");
        assert_eq!(neighbor_path, "BüroX/Other");

        let payload: String =
            sqlx::query_scalar("SELECT payload FROM sync_queue ORDER BY rowid DESC LIMIT 1")
                .fetch_one(&pool)
                .await
                .expect("queued rename");
        assert!(matches!(
            serde_json::from_str::<RemoteMutationPayload>(&payload).expect("payload"),
            RemoteMutationPayload::RenameFolder { from_path, to_path }
                if from_path == "Büro%" && to_path == "Work"
        ));
    }

    #[tokio::test]
    async fn preferred_abuse_folder_prefers_exact_name() {
        let pool = fresh_pool().await;
        let now = chrono::Utc::now().to_rfc3339();
        for (id, name, full_path) in [
            ("abuse-nested", "Inbox", "Inbox/Abuse"),
            ("abuse-exact", "Abuse", "Abuse"),
        ] {
            sqlx::query(
                "INSERT INTO folders
                 (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at)
                 VALUES (?, 'account', ?, ?, 'custom', ?, ?, ?)",
            )
            .bind(id)
            .bind(name)
            .bind(full_path)
            .bind(full_path)
            .bind(&now)
            .bind(&now)
            .execute(&pool)
            .await
            .expect("folder");
        }

        let queue = SyncQueue::new();
        assert_eq!(
            FolderRepository::new(&pool, &queue)
                .preferred_abuse_folder_id("account")
                .await
                .expect("preferred Abuse folder"),
            Some("abuse-exact".into())
        );
    }

    #[tokio::test]
    async fn validation_and_delete_guards_cross_the_repository_interface() {
        let pool = fresh_pool().await;
        let queue = SyncQueue::new();
        let repository = FolderRepository::new(&pool, &queue);

        assert!(matches!(
            repository.create("account", "a/b", None).await,
            Err(FolderError::NameContainsSeparator)
        ));
        let parent = repository
            .create("account", "Projects", None)
            .await
            .expect("parent");
        repository
            .create("account", "Satchel", Some(&parent.full_path))
            .await
            .expect("child");
        assert!(matches!(
            repository.delete(&parent.id).await,
            Err(FolderError::ContainsChildren { count: 1, .. })
        ));
    }

    #[tokio::test]
    async fn delete_guard_uses_the_accounts_hierarchy_separator() {
        let pool = fresh_pool().await;
        let queue = SyncQueue::new();
        let repository = FolderRepository::new(&pool, &queue);

        repository
            .create("account", "Existing.Path", None)
            .await
            .expect("separator hint");
        let parent = repository
            .create("account", "Projects", None)
            .await
            .expect("parent");
        let child = repository
            .create("account", "Satchel", Some(&parent.full_path))
            .await
            .expect("child");

        assert_eq!(parent.full_path, "Projects");
        assert_eq!(child.full_path, "Projects.Satchel");
        assert!(matches!(
            repository.delete(&parent.id).await,
            Err(FolderError::ContainsChildren { count: 1, .. })
        ));
    }

    #[tokio::test]
    async fn rename_collision_does_not_queue_or_change_the_local_folder() {
        let pool = fresh_pool().await;
        let queue = SyncQueue::new();
        let repository = FolderRepository::new(&pool, &queue);

        let source = repository
            .create("account", "Source", None)
            .await
            .expect("source");
        repository
            .create("account", "Target", None)
            .await
            .expect("target");
        let queued_before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_queue")
            .fetch_one(&pool)
            .await
            .expect("queue count");

        assert!(matches!(
            repository.rename(&source.id, "target").await,
            Err(FolderError::AlreadyExists { path }) if path == "target"
        ));
        let path: String = sqlx::query_scalar("SELECT full_path FROM folders WHERE id = ?")
            .bind(&source.id)
            .fetch_one(&pool)
            .await
            .expect("source path");
        let queued_after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_queue")
            .fetch_one(&pool)
            .await
            .expect("queue count");
        assert_eq!(path, "Source");
        assert_eq!(queued_after, queued_before);
    }

    #[tokio::test]
    async fn a_queue_failure_never_changes_the_local_projection() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("pool");
        sqlx::query(
            "CREATE TABLE folders (
                id TEXT PRIMARY KEY,
                account_id TEXT NOT NULL,
                name TEXT NOT NULL,
                full_path TEXT NOT NULL,
                folder_type TEXT NOT NULL,
                remote_id TEXT,
                sync_enabled INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
             )",
        )
        .execute(&pool)
        .await
        .expect("folders");
        let queue = SyncQueue::new();
        let repository = FolderRepository::new(&pool, &queue);

        assert!(matches!(
            repository.create("account", "Projects", None).await,
            Err(FolderError::Queue(_))
        ));
        let folder_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM folders")
            .fetch_one(&pool)
            .await
            .expect("count");
        assert_eq!(folder_count, 0);
    }
}
