//! A server that refuses Satchel's keywords must never hold up the account's
//! queue: the marker row settles, and the archive move behind it runs.
use async_trait::async_trait;
use satchel_core::providers::traits::{
    FlagUpdate, MailProvider, MarkerTarget, MessageRef, OutgoingMessage, RemoteFolder, SyncBatch,
    SyncCheckpoint, WorkflowMarker, WorkflowMarkerReport,
};
use satchel_core::sync::{
    errors::SyncResult,
    queue::{QueuedAction, SyncQueue},
    remote_mutations::RemoteMutationPayload,
    worker::{ActionExecutor, Outcome, QueueWorker},
    workflow_markers::{
        enqueue_workflow_marker_in_transaction, execute_marker_write, list_local_only_accounts,
    },
};
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Refuses every keyword write, as a full keyword table or a server without
/// `\*` does.
struct RefusingServer;

#[async_trait]
impl MailProvider for RefusingServer {
    async fn sync_folders(&self) -> SyncResult<Vec<RemoteFolder>> {
        Ok(Vec::new())
    }
    async fn sync_messages(&self, _: &str, _: SyncCheckpoint) -> SyncResult<SyncBatch> {
        Err("unused".into())
    }
    async fn send(&self, _: OutgoingMessage) -> SyncResult<()> {
        Ok(())
    }
    async fn set_flags(&self, _: &[MessageRef], _: FlagUpdate) -> SyncResult<()> {
        Ok(())
    }
    async fn move_messages(&self, _: &[MessageRef], _: &str) -> SyncResult<()> {
        Ok(())
    }
    async fn delete_messages(&self, _: &[MessageRef]) -> SyncResult<()> {
        Ok(())
    }
    async fn set_workflow_marker(
        &self,
        targets: &[MarkerTarget],
        _: &WorkflowMarker,
    ) -> SyncResult<WorkflowMarkerReport> {
        Ok(WorkflowMarkerReport::refused(targets.len()))
    }
}

struct Executor {
    pool: SqlitePool,
    ran: Mutex<Vec<String>>,
}

#[async_trait]
impl ActionExecutor for Executor {
    type Permit = ();
    async fn try_acquire(&self, _: &QueuedAction) -> Result<Option<()>, String> {
        Ok(Some(()))
    }
    async fn execute(&self, action: &QueuedAction) -> SyncResult<()> {
        let kind = action.payload["kind"]
            .as_str()
            .unwrap_or("other")
            .to_string();
        self.ran.lock().unwrap().push(kind);
        match serde_json::from_value(action.payload.clone()) {
            Ok(RemoteMutationPayload::WorkflowMarker { threads, marker }) => {
                execute_marker_write(
                    &self.pool,
                    &action.account_id,
                    &RefusingServer,
                    &threads,
                    &marker,
                )
                .await
            }
            _ => Ok(()),
        }
    }
    async fn observe(&self, _: &QueuedAction, _: Result<Outcome, String>) {}
}

async fn fixture() -> SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::migrate!("./src/db/migrations")
        .run(&pool)
        .await
        .unwrap();
    for sql in [
        "INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
         VALUES ('acc', 'Me', 'me@example.com', 'imap', '{}', '', '', '')",
        "INSERT INTO folders (id, account_id, name, full_path, folder_type, remote_id, uidvalidity, created_at, updated_at)
         VALUES ('inbox', 'acc', 'INBOX', 'INBOX', 'inbox', 'INBOX', 7, '', '')",
        "INSERT INTO threads (id, account_id, subject, last_message_date, workflow_state, board_column, created_at, updated_at)
         VALUES ('t', 'acc', 'Subject', '2026-11-06', 'done', 'done', '', '')",
        "INSERT INTO messages (id, account_id, folder_id, thread_id, from_address, to_addresses, date, remote_id, created_at, updated_at)
         VALUES ('m', 'acc', 'inbox', 't', 'a@example.com', '[]', '2026-11-06', '41', '', '')",
    ] {
        sqlx::query(sql).execute(&pool).await.unwrap();
    }
    pool
}

#[tokio::test]
async fn a_refused_marker_settles_and_the_move_behind_it_runs() {
    let pool = fixture().await;
    let queue = Arc::new(SyncQueue::new());
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
    enqueue_workflow_marker_in_transaction(&mut tx, &queue, "t", &WorkflowMarker::Done, None)
        .await
        .unwrap();
    // The archive move a Done queues after its marker.
    let mut moved = QueuedAction::new(
        "acc".into(),
        "remote_mutation",
        serde_json::json!({"kind": "move", "refs": [], "target_remote_folder_id": "Archive"}),
    );
    moved.id = "archive-move".into();
    queue.enqueue_in_transaction(&mut tx, &moved).await.unwrap();
    tx.commit().await.unwrap();

    let executor = Arc::new(Executor {
        pool: pool.clone(),
        ran: Mutex::default(),
    });
    let mut worker = QueueWorker::new(pool.clone(), queue, executor.clone(), 1);
    for _ in 0..2 {
        assert_eq!(worker.dispatch_ready().await.unwrap(), 1);
        tokio::time::timeout(Duration::from_secs(2), worker.wait_for_completion())
            .await
            .unwrap();
    }
    assert_eq!(
        *executor.ran.lock().unwrap(),
        vec!["workflow_marker", "move"]
    );
    let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_queue")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(left, 0, "neither row waits for a retry");
    assert_eq!(
        list_local_only_accounts(&pool).await.unwrap(),
        vec!["acc".to_string()]
    );
}
