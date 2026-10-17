use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::commands::error::{CommandError, CommandResult};
use crate::{sync::remote_mutations, AppState};
use satchel_core::db::blocked_senders::{
    BlockedSenderError, BlockedSenderPatternType, BlockedSenderRecord, BlockedSenderRepository,
    BlockedSenderUpsertRequest,
};
use satchel_core::db::folders::FolderRepository;
use satchel_core::db::threads::ThreadRepository;
use satchel_core::sync::queue::{QueuedAction, SyncQueue};

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/blocked_senders/", optional_fields = nullable))]
pub struct BlockSenderInput {
    /// Address or domain pattern (e.g. "alice@example.com" or "*@spam.example.com").
    pub pattern: String,
    #[serde(default)]
    pub account_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/blocked_senders/")
)]
pub struct BlockedSenderDto {
    pub id: String,
    pub account_id: Option<String>,
    pub pattern: String,
    pub pattern_type: String,
    pub created_at: String,
}

impl From<BlockedSenderRecord> for BlockedSenderDto {
    fn from(record: BlockedSenderRecord) -> Self {
        Self {
            id: record.id,
            account_id: record.account_id,
            pattern: record.pattern,
            pattern_type: record.pattern_type,
            created_at: record.created_at,
        }
    }
}

impl From<BlockSenderInput> for BlockedSenderUpsertRequest {
    fn from(input: BlockSenderInput) -> Self {
        Self {
            account_id: input.account_id,
            pattern: input.pattern,
        }
    }
}

pub(crate) fn blocked_sender_error_key(error: BlockedSenderError) -> String {
    match error {
        BlockedSenderError::PatternEmpty => "backend.errors.blockedSenderPatternEmpty",
        BlockedSenderError::Persistence(error) => {
            tracing::error!(error = ?error, "blocked sender persistence failed");
            "backend.errors.blockedSenderPersistenceFailed"
        }
    }
    .to_owned()
}

impl From<BlockedSenderError> for CommandError {
    fn from(error: BlockedSenderError) -> Self {
        Self::new(blocked_sender_error_key(error))
    }
}

#[tauri::command]
pub async fn list_blocked_senders(
    pool: tauri::State<'_, crate::db::ReadPool>,
) -> CommandResult<Vec<BlockedSenderDto>> {
    BlockedSenderRepository::new(pool.inner().as_ref())
        .list()
        .await
        .map(|rows| rows.into_iter().map(Into::into).collect())
        .map_err(Into::into)
}

#[tauri::command]
pub async fn block_sender(
    pool: tauri::State<'_, SqlitePool>,
    state: tauri::State<'_, AppState>,
    input: BlockSenderInput,
) -> CommandResult<BlockedSenderDto> {
    let record = BlockedSenderRepository::new(pool.inner())
        .upsert(input.into())
        .await?;
    let Some(pattern_type) = BlockedSenderPatternType::parse(&record.pattern_type) else {
        tracing::warn!(
            pattern_type = %record.pattern_type,
            "blocked sender rule has an unknown pattern type"
        );
        return Ok(record.into());
    };
    let affected_threads = ThreadRepository::new(pool.inner())
        .find_matching_sender_threads(record.account_id.as_deref(), &record.pattern, pattern_type)
        .await
        .map_err(|error| {
            tracing::error!(error = ?error, "blocked sender thread selection failed");
            CommandError::new("backend.errors.blockedSenderPersistenceFailed")
        })?;

    // Prepare provider refs before the repository atomically commits the
    // queue rows and local projection.
    let mut actions = Vec::new();
    for thread_id in &affected_threads {
        actions.extend(
            prepare_blocked_thread_moves(
                pool.inner(),
                record.account_id.as_deref(),
                thread_id,
            )
            .await
            .map_err(|error| {
                tracing::error!(error = %error, "blocked sender remote move preparation failed");
                CommandError::new("backend.errors.blockedSenderPersistenceFailed")
            })?,
        );
    }
    ThreadRepository::new(pool.inner())
        .commit_blocked_moves(state.inner().queue.as_ref(), &affected_threads, &actions)
        .await
        .map_err(|error| {
            tracing::error!(error = ?error, "blocked sender move commit failed");
            CommandError::new("backend.errors.blockedSenderPersistenceFailed")
        })?;
    Ok(record.into())
}

/// Move blocked mail to an account's Abuse folder when one exists, otherwise
/// use the provider's standard spam/junk folder.
pub async fn prepare_blocked_thread_moves(
    pool: &SqlitePool,
    account_id: Option<&str>,
    thread_id: &str,
) -> Result<Vec<QueuedAction>, String> {
    // The folder lookup is a read-only FolderRepository operation. A
    // temporary queue keeps the lookup out of the command's SQL boundary;
    // prepared actions are committed with the caller's real queue below.
    let lookup_queue = SyncQueue::new();

    if let Some(account_id) = account_id {
        if let Some(folder_id) = FolderRepository::new(pool, &lookup_queue)
            .preferred_abuse_folder_id(account_id)
            .await
            .map_err(|error| error.to_string())?
        {
            return remote_mutations::prepare_blocked_thread_moves_to_folder(
                pool, thread_id, &folder_id,
            )
            .await;
        }
    }

    remote_mutations::prepare_blocked_thread_moves_to_kind(pool, thread_id, "spam").await
}

#[tauri::command]
pub async fn unblock_sender(pool: tauri::State<'_, SqlitePool>, id: String) -> CommandResult<()> {
    BlockedSenderRepository::new(pool.inner())
        .delete(&id)
        .await
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use satchel_core::sync::queue::SyncQueue;
    use sqlx::sqlite::SqlitePoolOptions;

    use super::*;

    #[test]
    fn persistence_errors_use_a_stable_key_without_sql_details() {
        let detail = "near SELECT: secret blocked sender SQL";
        let error = CommandError::from(BlockedSenderError::Persistence(sqlx::Error::Protocol(
            detail.into(),
        )));
        let serialized = serde_json::to_value(error).expect("command error JSON");

        assert_eq!(
            serialized,
            serde_json::json!({ "key": "backend.errors.blockedSenderPersistenceFailed" })
        );
        assert!(!serialized.to_string().contains(detail));
    }

    #[test]
    fn empty_patterns_use_a_stable_key() {
        assert_eq!(
            CommandError::from(BlockedSenderError::PatternEmpty).key,
            "backend.errors.blockedSenderPatternEmpty"
        );
    }

    #[test]
    fn dto_preserves_the_ipc_shape() {
        let dto = BlockedSenderDto::from(BlockedSenderRecord {
            id: "blk-1".into(),
            account_id: Some("acc-1".into()),
            pattern: "alice@example.com".into(),
            pattern_type: "address".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
        });
        assert_eq!(
            serde_json::to_value(dto).expect("blocked sender DTO JSON"),
            serde_json::json!({
                "id": "blk-1",
                "account_id": "acc-1",
                "pattern": "alice@example.com",
                "pattern_type": "address",
                "created_at": "2026-01-01T00:00:00Z"
            })
        );
    }

    #[tokio::test]
    async fn blocked_move_commit_queues_without_a_local_target_folder() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory pool");
        sqlx::migrate!("../crates/satchel-core/src/db/migrations")
            .run(&pool)
            .await
            .expect("migrations");
        for statement in [
            "INSERT INTO accounts
             (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
             VALUES ('account', 'Account', 'account@example.com', 'imap', '{}', '', '', '')",
            "INSERT INTO folders
             (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at)
             VALUES ('inbox', 'account', 'Inbox', 'Inbox', 'inbox', 'INBOX', '', '')",
            "INSERT INTO threads
             (id, account_id, subject, last_message_date, workflow_state, created_at, updated_at)
             VALUES ('thread', 'account', 'Subject', '', 'inbox', '', '')",
            "INSERT INTO messages
             (id, account_id, folder_id, thread_id, from_address, to_addresses, date, remote_id,
              created_at, updated_at)
             VALUES ('message', 'account', 'inbox', 'thread', 'sender@example.com', '[]', '', '42', '', '')",
        ] {
            sqlx::query(statement)
                .execute(&pool)
                .await
                .expect("seed data");
        }

        let queue = SyncQueue::new();
        let actions = prepare_blocked_thread_moves(&pool, Some("account"), "thread")
            .await
            .expect("prepare blocked move");
        ThreadRepository::new(&pool)
            .commit_blocked_moves(&queue, &["thread".into()], &actions)
            .await
            .expect("atomic blocked move commit");

        let state: (String, Option<String>) =
            sqlx::query_as("SELECT workflow_state, board_column FROM threads WHERE id = 'thread'")
                .fetch_one(&pool)
                .await
                .expect("thread projection");
        assert_eq!(state, ("done".into(), Some("done".into())));
        let (action_type, payload): (String, String) = sqlx::query_as(
            "SELECT action_type, payload FROM sync_queue WHERE account_id = 'account'",
        )
        .fetch_one(&pool)
        .await
        .expect("queued move");
        assert_eq!(action_type, "remote_mutation");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&payload)
                .expect("move payload")
                .get("kind")
                .and_then(serde_json::Value::as_str),
            Some("blocked_move")
        );
        let folder_id: String =
            sqlx::query_scalar("SELECT folder_id FROM messages WHERE id = 'message'")
                .fetch_one(&pool)
                .await
                .expect("message source projection");
        assert_eq!(folder_id, "inbox");
    }
}
