//! A bulk move costs a constant number of SQL statements per batch, however
//! many threads the batch holds. Counted, not timed, so it gates in CI: a
//! return to per-thread queries makes the larger move cost more statements.
//!
//! sqlx logs every statement it runs on target `sqlx::query`, from the
//! connection's worker thread. The counter is therefore process-global, and
//! this file holds a single test.

mod bulk_move_fixture;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use bulk_move_fixture::{pool, seed};
use satchel_core::mail_actions::{self, MoveTarget};
use satchel_core::sync::queue::SyncQueue;
use satchel_core::workflow::states::WorkflowState;
use satchel_core::workflow::transitions::{
    transition_threads_inner, BatchTransitionEntry, BatchTransitionInput,
};
use tracing::span::{Attributes, Id, Record};
use tracing::{subscriber::Interest, Event, Metadata};

struct StatementCounter(Arc<AtomicUsize>);

impl tracing::Subscriber for StatementCounter {
    fn register_callsite(&self, metadata: &'static Metadata<'static>) -> Interest {
        if metadata.target() == "sqlx::query" {
            Interest::always()
        } else {
            Interest::never()
        }
    }

    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.target() == "sqlx::query"
    }

    fn new_span(&self, _: &Attributes<'_>) -> Id {
        Id::from_u64(1)
    }

    fn record(&self, _: &Id, _: &Record<'_>) {}

    fn record_follows_from(&self, _: &Id, _: &Id) {}

    fn event(&self, event: &Event<'_>) {
        if event.metadata().target() == "sqlx::query" {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn enter(&self, _: &Id) {}

    fn exit(&self, _: &Id) {}
}

#[derive(Clone, Copy, Debug)]
enum Kind {
    Archive,
    Trash,
    Folder,
    /// Done through `transition_threads`: one transaction per thread.
    Done,
    /// One rule's move of one thread, inside the rule run's transaction,
    /// after every other thread was archived: the queue is full.
    RuleMove,
}

async fn marker_rows(pool: &sqlx::SqlitePool) -> usize {
    let rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sync_queue WHERE json_extract(payload, '$.kind') = 'workflow_marker'",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    usize::try_from(rows).unwrap()
}

/// Statements one move of `threads` seeded threads runs.
async fn statements(counter: &AtomicUsize, kind: Kind, threads: usize) -> usize {
    let pool = pool().await;
    let mut ids = seed(&pool, threads).await;
    let queue = SyncQueue::new();
    if matches!(kind, Kind::RuleMove) {
        mail_actions::move_threads(&pool, &queue, &ids[1..], MoveTarget::Archive)
            .await
            .expect("archive the others");
    }
    let markers_before = marker_rows(&pool).await;
    let before = counter.load(Ordering::SeqCst);
    match kind {
        Kind::Archive => {
            mail_actions::move_threads(&pool, &queue, &ids, MoveTarget::Archive)
                .await
                .expect("archive");
        }
        Kind::Trash => {
            mail_actions::trash_threads(&pool, &queue, &ids)
                .await
                .expect("trash");
        }
        Kind::Folder => {
            // Folder moves stay within one account.
            ids.retain(|id| id.trim_start_matches('t').parse::<usize>().unwrap() % 4 == 0);
            mail_actions::move_threads(&pool, &queue, &ids, MoveTarget::Folder("imap-a-projects"))
                .await
                .expect("folder");
        }
        Kind::Done => {
            let result = transition_threads_inner(
                &pool,
                &queue,
                BatchTransitionInput {
                    threads: ids
                        .iter()
                        .map(|id| BatchTransitionEntry {
                            thread_id: id.clone(),
                            from: WorkflowState::Inbox,
                        })
                        .collect(),
                    to: WorkflowState::Done,
                    snooze_until: None,
                },
            )
            .await;
            assert!(result.failed.is_empty(), "{:?}", result.failed.first());
        }
        Kind::RuleMove => {
            let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
            mail_actions::move_inbox_messages_in_transaction(
                &mut tx,
                &queue,
                &ids[0],
                "imap-a-projects",
            )
            .await
            .expect("rule move");
            tx.commit().await.unwrap();
        }
    }
    let used = counter.load(Ordering::SeqCst) - before;
    // A marker row holds a fixed number of threads, and each new one is an
    // INSERT of its own; count statements as if every marker row were one.
    let created = marker_rows(&pool).await - markers_before;
    pool.close().await;
    used - created
}

#[tokio::test]
async fn bulk_moves_cost_statements_per_batch_not_per_thread() {
    let counter = Arc::new(AtomicUsize::new(0));
    tracing::subscriber::set_global_default(StatementCounter(counter.clone()))
        .expect("the only subscriber of this test binary");

    // Both sizes stay within one INSERT of queue rows per batch.
    for kind in [Kind::Archive, Kind::Trash, Kind::Folder] {
        // One batch: three times the threads, the same statements.
        mail_actions::set_move_batch_threads_for_tests(Some(1_000));
        let small = statements(&counter, kind, 40).await;
        let large = statements(&counter, kind, 120).await;
        assert!(small > 0, "statements are counted");
        assert_eq!(
            large, small,
            "{kind:?}: 120 threads in one batch ran {large} statements, 40 ran {small}"
        );

        // Batches of 10: the cost follows the batches.
        mail_actions::set_move_batch_threads_for_tests(Some(10));
        let batched = statements(&counter, kind, 120).await;
        assert!(
            batched <= 12 * small,
            "{kind:?}: 12 batches ran {batched} statements, one batch {small}"
        );
    }
    mail_actions::set_move_batch_threads_for_tests(None);

    // Done is a transaction per thread. Each costs the same, however many
    // threads went before it: the cost is linear, not quadratic.
    let forty = statements(&counter, Kind::Done, 40).await;
    let eighty = statements(&counter, Kind::Done, 80).await;
    let hundred_twenty = statements(&counter, Kind::Done, 120).await;
    assert_eq!(
        hundred_twenty - eighty,
        eighty - forty,
        "Done: 40 threads ran {forty} statements, 80 ran {eighty}, 120 ran {hundred_twenty}"
    );

    // A rule's move of one thread costs the same however much is queued.
    let short_queue = statements(&counter, Kind::RuleMove, 40).await;
    let long_queue = statements(&counter, Kind::RuleMove, 120).await;
    assert_eq!(
        long_queue, short_queue,
        "a rule move ran {short_queue} statements behind 40 archived threads, {long_queue} behind 120"
    );
}
