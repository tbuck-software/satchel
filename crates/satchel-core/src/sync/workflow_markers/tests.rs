use super::*;
use crate::providers::traits::{FlagUpdate, MailProvider, RemoteFolder, SyncBatch, SyncCheckpoint};
use chrono::{Duration, Utc};
use sqlx::sqlite::SqlitePoolOptions;

async fn pool(provider: &str) -> SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("pool");
    sqlx::migrate!("./src/db/migrations")
        .run(&pool)
        .await
        .expect("migrations");
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
         VALUES ('acc', 'Me', 'me@example.com', ?, '{}', '', ?, ?)",
    )
    .bind(provider)
    .bind(&now)
    .bind(&now)
    .execute(&pool)
    .await
    .expect("account");
    for (id, kind) in [("INBOX", "inbox"), ("Archive", "archive")] {
        sqlx::query(
            "INSERT INTO folders (id, account_id, name, full_path, folder_type, remote_id, uidvalidity, created_at, updated_at)
             VALUES (?, 'acc', ?, ?, ?, ?, 7, ?, ?)",
        )
        .bind(id)
        .bind(id)
        .bind(id)
        .bind(kind)
        .bind(id)
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("folder");
    }
    thread(&pool, "t").await;
    message(&pool, "m1", "t", "INBOX", "41", None).await;
    pool
}

async fn thread(pool: &SqlitePool, id: &str) {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO threads (id, account_id, subject, last_message_date, workflow_state, board_column, created_at, updated_at)
         VALUES (?, 'acc', 'Subject', ?, 'inbox', 'inbox', ?, ?)",
    )
    .bind(id)
    .bind(&now)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .expect("thread");
}

async fn message(
    pool: &SqlitePool,
    id: &str,
    thread: &str,
    folder: &str,
    uid: &str,
    marker: Option<&str>,
) {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO messages (id, account_id, folder_id, thread_id, from_address, to_addresses, date, remote_id, remote_workflow_marker, created_at, updated_at)
         VALUES (?, 'acc', ?, ?, 'a@example.com', '[]', ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(folder)
    .bind(thread)
    .bind(&now)
    .bind(uid)
    .bind(marker)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .expect("message");
}

type QueuedRow = (
    String,
    WorkflowMarker,
    Vec<String>,
    Vec<Option<WorkflowMarker>>,
);

/// (row id, marker, thread ids, expected markers) of every queued row.
async fn queued(pool: &SqlitePool) -> Vec<QueuedRow> {
    sqlx::query_as::<_, (String, String)>(
        "SELECT id, payload FROM sync_queue WHERE json_extract(payload, '$.kind') = 'workflow_marker' ORDER BY rowid",
    )
    .fetch_all(pool)
    .await
    .expect("queue")
    .into_iter()
    .map(|(id, payload)| match serde_json::from_str(&payload).expect("payload") {
        RemoteMutationPayload::WorkflowMarker { threads, marker } => (
            id,
            marker,
            threads.iter().map(|t| t.thread_id.clone()).collect(),
            threads.iter().map(|t| t.expected).collect(),
        ),
        other => panic!("unexpected payload {other:?}"),
    })
    .collect()
}

async fn queued_threads(pool: &SqlitePool) -> Vec<MarkedThread> {
    let payload: String = sqlx::query_scalar(
        "SELECT payload FROM sync_queue WHERE json_extract(payload, '$.kind') = 'workflow_marker'",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    match serde_json::from_str(&payload).unwrap() {
        RemoteMutationPayload::WorkflowMarker { threads, .. } => threads,
        other => panic!("unexpected payload {other:?}"),
    }
}

async fn push_for(pool: &SqlitePool, thread: &str, marker: WorkflowMarker) -> Vec<String> {
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.expect("tx");
    let ids =
        enqueue_workflow_marker_in_transaction(&mut tx, &SyncQueue::new(), thread, &marker, None)
            .await
            .expect("enqueue");
    tx.commit().await.expect("commit");
    ids
}

async fn push(pool: &SqlitePool, marker: WorkflowMarker) -> Vec<String> {
    push_for(pool, "t", marker).await
}

async fn wake(pool: &SqlitePool) -> Vec<String> {
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.expect("tx");
    let ids = enqueue_wake_clear_in_transaction(&mut tx, &SyncQueue::new(), "t")
        .await
        .expect("wake");
    tx.commit().await.expect("commit");
    ids
}

async fn state(pool: &SqlitePool) -> (String, Option<String>, Option<String>) {
    sqlx::query_as(
        "SELECT workflow_state, snooze_until, remote_workflow_marker FROM threads WHERE id = 't'",
    )
    .fetch_one(pool)
    .await
    .expect("thread")
}

async fn set_server(pool: &SqlitePool, marker: &WorkflowMarker) {
    sqlx::query("UPDATE messages SET remote_workflow_marker = ? WHERE thread_id = 't'")
        .bind(marker.as_column())
        .execute(pool)
        .await
        .expect("server marker");
}

async fn record(pool: &SqlitePool, marker: &WorkflowMarker) {
    sqlx::query("UPDATE threads SET remote_workflow_marker = ? WHERE id = 't'")
        .bind(marker.as_column())
        .execute(pool)
        .await
        .expect("recorded marker");
}

async fn pull(pool: &SqlitePool) -> bool {
    !reconcile_remote_workflow_markers(pool, &HashSet::from(["t".to_string()]))
        .await
        .expect("reconcile")
        .is_empty()
}

fn snooze_at(wake: chrono::DateTime<Utc>, generation: u8) -> WorkflowMarker {
    WorkflowMarker::Snoozed(Snooze {
        wake: Some(chrono::DateTime::from_timestamp(wake.timestamp(), 0).unwrap()),
        generation,
    })
}

fn imap_snooze(generation: u8) -> WorkflowMarker {
    WorkflowMarker::Snoozed(Snooze {
        wake: None,
        generation,
    })
}

#[tokio::test]
async fn a_marker_push_carries_every_server_copy_and_records_nothing_yet() {
    let pool = pool("imap").await;
    message(&pool, "m2", "t", "Archive", "9", None).await;
    assert_eq!(push(&pool, WorkflowMarker::Done).await.len(), 1);
    let refs = queued_threads(&pool).await[0]
        .refs
        .iter()
        .map(|r| {
            (
                r.source_folder.clone().unwrap(),
                r.remote_id.clone(),
                r.source_uidvalidity,
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        refs,
        vec![
            ("INBOX".into(), "41".into(), Some(7)),
            ("Archive".into(), "9".into(), Some(7))
        ]
    );
    // Recorded only once the server accepted it.
    assert_eq!(state(&pool).await.2, None);
}

#[tokio::test]
async fn threads_with_the_same_marker_share_one_row() {
    let pool = pool("imap").await;
    for (id, uid) in [("t2", "52"), ("t3", "53")] {
        thread(&pool, id).await;
        message(&pool, &format!("m-{id}"), id, "INBOX", uid, None).await;
    }
    let first = push(&pool, WorkflowMarker::Done).await;
    assert_eq!(push_for(&pool, "t2", WorkflowMarker::Done).await, first);
    assert_eq!(push_for(&pool, "t3", WorkflowMarker::Done).await, first);
    let rows = queued(&pool).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].2, vec!["t", "t2", "t3"]);

    // Undo of one thread takes it out of the shared row only, and the server
    // is never told about either.
    push(&pool, WorkflowMarker::None).await;
    let rows = queued(&pool).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].2, vec!["t2", "t3"]);
}

#[tokio::test]
async fn a_row_waiting_for_a_retry_is_not_joined() {
    let pool = pool("gmail").await;
    thread(&pool, "t2").await;
    message(&pool, "m2", "t2", "INBOX", "m2", None).await;
    push(&pool, WorkflowMarker::Done).await;
    sqlx::query("UPDATE sync_queue SET retry_count = 1, next_retry_at = '2999-01-01'")
        .execute(&pool)
        .await
        .unwrap();
    push_for(&pool, "t2", WorkflowMarker::Done).await;
    assert_eq!(queued(&pool).await.len(), 2);
}

#[tokio::test]
async fn a_rapid_toggle_leaves_only_the_last_marker() {
    let pool = pool("gmail").await;
    let wake = Utc::now() + Duration::hours(3);
    push(&pool, WorkflowMarker::Done).await;
    push(&pool, snooze_at(wake, 1)).await;
    assert_eq!(queued(&pool).await.len(), 1);
    // Back where the server still is: nothing goes out at all.
    push(&pool, WorkflowMarker::None).await;
    assert!(queued(&pool).await.is_empty());
}

#[tokio::test]
async fn a_marker_in_flight_is_followed_by_the_new_one() {
    let pool = pool("imap").await;
    push(&pool, WorkflowMarker::Done).await;
    sqlx::query("UPDATE sync_queue SET status = 'sending'")
        .execute(&pool)
        .await
        .unwrap();
    // The Done is on its way; going back to Inbox must follow it, although
    // nothing was recorded yet.
    push(&pool, WorkflowMarker::None).await;
    let rows = queued(&pool).await;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1].1, WorkflowMarker::None);
}

#[tokio::test]
async fn each_imap_snooze_takes_the_other_generation() {
    let pool = pool("imap").await;
    let wake = Utc::now() + Duration::hours(3);
    record(&pool, &snooze_at(wake, 1)).await;
    push(&pool, snooze_at(wake, 1)).await;
    assert_eq!(queued(&pool).await[0].1, snooze_at(wake, 2));
}

#[tokio::test]
async fn local_only_accounts_and_other_providers_queue_nothing() {
    let pool = pool("imap").await;
    record_inbox_keyword_support(&pool, "acc", false)
        .await
        .unwrap();
    assert!(push(&pool, WorkflowMarker::Done).await.is_empty());
    assert_eq!(
        list_local_only_accounts(&pool).await.unwrap(),
        vec!["acc".to_string()]
    );
    record_inbox_keyword_support(&pool, "acc", true)
        .await
        .unwrap();
    assert!(list_local_only_accounts(&pool).await.unwrap().is_empty());

    let outlook = self::pool("outlook").await;
    assert!(push(&outlook, WorkflowMarker::Done).await.is_empty());
}

#[tokio::test]
async fn a_refused_inbox_store_stays_local_only_whatever_the_flags_say() {
    let pool = pool("imap").await;
    let threads = vec![MarkedThread {
        thread_id: "t".into(),
        refs: Vec::new(),
        expected: None,
    }];
    apply_marker_report(
        &pool,
        "acc",
        &threads,
        &WorkflowMarker::Done,
        &WorkflowMarkerReport {
            targets: vec![WorkflowMarkerWrite::Refused],
            refused_in_inbox: true,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        state(&pool).await.2,
        None,
        "a refused write is not recorded"
    );
    // INBOX still lists `\*`; that does not undo a refused STORE.
    record_inbox_keyword_support(&pool, "acc", true)
        .await
        .unwrap();
    assert_eq!(
        list_local_only_accounts(&pool).await.unwrap(),
        vec!["acc".to_string()]
    );
}

#[tokio::test]
async fn an_accepted_write_is_recorded_per_thread() {
    let pool = pool("imap").await;
    thread(&pool, "t2").await;
    let threads = ["t", "t2"]
        .map(|id| MarkedThread {
            thread_id: id.into(),
            refs: Vec::new(),
            expected: None,
        })
        .to_vec();
    apply_marker_report(
        &pool,
        "acc",
        &threads,
        &WorkflowMarker::Done,
        &WorkflowMarkerReport {
            targets: vec![WorkflowMarkerWrite::Applied, WorkflowMarkerWrite::Skipped],
            refused_in_inbox: false,
        },
    )
    .await
    .unwrap();
    let recorded: Vec<Option<String>> =
        sqlx::query_scalar("SELECT remote_workflow_marker FROM threads ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(recorded, vec![Some("done".to_string()), None]);
    // An accepted write is proof the server takes keywords.
    assert!(list_local_only_accounts(&pool).await.unwrap().is_empty());
}

/// A provider that refuses every write the way a full keyword table does.
struct RefusingProvider;

#[async_trait::async_trait]
impl MailProvider for RefusingProvider {
    async fn sync_folders(&self) -> crate::sync::errors::SyncResult<Vec<RemoteFolder>> {
        Ok(Vec::new())
    }
    async fn sync_messages(
        &self,
        _: &str,
        _: SyncCheckpoint,
    ) -> crate::sync::errors::SyncResult<SyncBatch> {
        Err("unused".into())
    }
    async fn send(
        &self,
        _: crate::providers::traits::OutgoingMessage,
    ) -> crate::sync::errors::SyncResult<()> {
        Ok(())
    }
    async fn set_flags(
        &self,
        _: &[MessageRef],
        _: FlagUpdate,
    ) -> crate::sync::errors::SyncResult<()> {
        Ok(())
    }
    async fn move_messages(
        &self,
        _: &[MessageRef],
        _: &str,
    ) -> crate::sync::errors::SyncResult<()> {
        Ok(())
    }
    async fn delete_messages(&self, _: &[MessageRef]) -> crate::sync::errors::SyncResult<()> {
        Ok(())
    }
    async fn set_workflow_marker(
        &self,
        targets: &[MarkerTarget],
        _: &WorkflowMarker,
    ) -> crate::sync::errors::SyncResult<WorkflowMarkerReport> {
        Ok(WorkflowMarkerReport {
            targets: vec![WorkflowMarkerWrite::Refused; targets.len()],
            refused_in_inbox: true,
        })
    }
}

#[tokio::test]
async fn a_refused_write_settles_and_leaves_the_queue_free() {
    let pool = pool("imap").await;
    push(&pool, WorkflowMarker::Done).await;
    let threads = queued_threads(&pool).await;
    // Ok, not an error: the worker completes the row, and the account's next
    // head (an archive move, a read flag) runs. See `queue_worker` for the
    // dispatcher half of this.
    execute_marker_write(
        &pool,
        "acc",
        &RefusingProvider,
        &threads,
        &WorkflowMarker::Done,
    )
    .await
    .expect("a refusal is not a failure");
    assert_eq!(state(&pool).await.2, None);
    assert_eq!(
        list_local_only_accounts(&pool).await.unwrap(),
        vec!["acc".to_string()]
    );
    // And nothing more is queued for this account.
    sqlx::query("DELETE FROM sync_queue")
        .execute(&pool)
        .await
        .unwrap();
    assert!(push(&pool, WorkflowMarker::None).await.is_empty());
}

#[tokio::test]
async fn waking_clears_only_this_devices_own_unreplaced_snooze() {
    let pool = pool("imap").await;
    let own = snooze_at(Utc::now() - Duration::minutes(1), 1);
    record(&pool, &own).await;
    assert_eq!(wake(&pool).await.len(), 1);
    let rows = queued(&pool).await;
    assert_eq!(rows[0].1, WorkflowMarker::None);
    assert_eq!(rows[0].3, vec![Some(own)]);

    let threads = queued_threads(&pool).await;
    assert_eq!(
        retain_current_entries(&pool, threads.clone())
            .await
            .unwrap()
            .len(),
        1
    );
    // Another device re-snoozed, and this device saw it before its clear ran.
    record(&pool, &imap_snooze(2)).await;
    assert!(retain_current_entries(&pool, threads)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn waking_an_adopted_or_gmail_snooze_pushes_nothing() {
    let pool = pool("imap").await;
    // Adopted from IMAP: the time is ours, not the snoozing device's.
    record(&pool, &imap_snooze(1)).await;
    assert!(wake(&pool).await.is_empty());

    let gmail = self::pool("gmail").await;
    record(&gmail, &snooze_at(Utc::now(), 1)).await;
    assert!(wake(&gmail).await.is_empty());
}

#[tokio::test]
async fn waking_cancels_a_snooze_that_never_left() {
    let pool = pool("imap").await;
    push(&pool, snooze_at(Utc::now() + Duration::minutes(1), 1)).await;
    assert!(wake(&pool).await.is_empty());
    assert!(queued(&pool).await.is_empty());
}

#[tokio::test]
async fn a_mailbox_without_markers_writes_nothing_on_pull() {
    let pool = pool("imap").await;
    let before: i64 = sqlx::query_scalar("SELECT total_changes()")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(!pull(&pool).await);
    assert!(!pull(&pool).await);
    let after: i64 = sqlx::query_scalar("SELECT total_changes()")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(before, after);
    assert_eq!(state(&pool).await.2, None);
}

#[tokio::test]
async fn a_gmail_snooze_from_another_device_is_adopted_and_wakes_here() {
    let pool = pool("gmail").await;
    let marker = snooze_at(Utc::now() + Duration::hours(2), 1);
    let WorkflowMarker::Snoozed(Snooze {
        wake: Some(wake), ..
    }) = marker
    else {
        unreachable!()
    };
    set_server(&pool, &marker).await;
    assert!(pull(&pool).await);
    let (state_now, until, seen) = state(&pool).await;
    assert_eq!(state_now, "snoozed");
    assert_eq!(until, Some(wake.to_rfc3339()));
    assert_eq!(seen, marker.as_column());
    // The snooze worker compares this column as text against
    // `Utc::now().to_rfc3339()`; the stored form must sort the same way.
    assert!(until.unwrap() > Utc::now().to_rfc3339());
    assert!(!pull(&pool).await);

    set_server(&pool, &WorkflowMarker::None).await;
    assert!(pull(&pool).await);
    assert_eq!(state(&pool).await.0, "inbox");
}

#[tokio::test]
async fn an_imap_snooze_from_another_device_is_adopted_for_a_day() {
    let pool = pool("imap").await;
    set_server(&pool, &imap_snooze(1)).await;
    assert!(pull(&pool).await);
    let (state_now, until, _) = state(&pool).await;
    assert_eq!(state_now, "snoozed");
    let until = chrono::DateTime::parse_from_rfc3339(&until.unwrap()).unwrap();
    let ahead = until.with_timezone(&Utc) - Utc::now();
    assert!(ahead > Duration::hours(23) && ahead <= Duration::hours(24));
}

#[tokio::test]
async fn the_servers_silence_does_not_reset_a_local_state() {
    let pool = pool("imap").await;
    sqlx::query("UPDATE threads SET workflow_state = 'todo'")
        .execute(&pool)
        .await
        .unwrap();
    set_server(&pool, &WorkflowMarker::None).await;
    assert!(!pull(&pool).await);
    assert_eq!(state(&pool).await, ("todo".into(), None, None));
}

#[tokio::test]
async fn an_unsent_local_change_wins_over_the_server() {
    let pool = pool("imap").await;
    let wake_at = Utc::now() + Duration::hours(2);
    let marker = WorkflowMarker::for_state(WorkflowState::Snoozed, Some(&wake_at.to_rfc3339()));
    sqlx::query("UPDATE threads SET workflow_state = 'snoozed', snooze_until = ?")
        .bind(wake_at.to_rfc3339())
        .execute(&pool)
        .await
        .unwrap();
    push(&pool, marker).await;
    // Another device's Done shows up before ours went out.
    sqlx::query("UPDATE messages SET folder_id = 'Archive', remote_workflow_marker = 'done'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(!pull(&pool).await);
    assert_eq!(state(&pool).await.0, "snoozed");

    // Pushed, accepted and echoed back without its time: nothing to do.
    sqlx::query("DELETE FROM sync_queue")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE messages SET folder_id = 'INBOX'")
        .execute(&pool)
        .await
        .unwrap();
    record(&pool, &marker).await;
    set_server(&pool, &imap_snooze(1)).await;
    assert!(!pull(&pool).await);
    assert_eq!(state(&pool).await.0, "snoozed");
}

#[tokio::test]
async fn local_only_accounts_ignore_the_server() {
    let pool = pool("imap").await;
    record_inbox_keyword_support(&pool, "acc", false)
        .await
        .unwrap();
    set_server(&pool, &snooze_at(Utc::now() + Duration::hours(2), 1)).await;
    assert!(!pull(&pool).await);
    assert_eq!(state(&pool).await.0, "inbox");
}

#[tokio::test]
async fn done_elsewhere_is_applied_once_the_mail_left_the_inbox() {
    let pool = pool("imap").await;
    sqlx::query("UPDATE messages SET folder_id = 'Archive', remote_workflow_marker = 'done'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(pull(&pool).await);
    assert_eq!(state(&pool).await.0, "done");
    let reason: String =
        sqlx::query_scalar("SELECT reason FROM workflow_events WHERE thread_id = 't'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(reason, "server");
}
