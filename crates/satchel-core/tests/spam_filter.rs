//! The local spam filter end to end over a migrated database: training,
//! untraining through Undo, relabelling, weak signals, pruning, the bootstrap,
//! classification of new mail and Not spam.

use satchel_core::mail_actions::{self, undo};
use satchel_core::spam::{
    self, bootstrap, classifier::Verdict, store, ClassifyOutcome, IncomingMessage, Label, Signal,
    SpamFilterMode,
};
use satchel_core::sync::queue::SyncQueue;
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

async fn pool(provider: &str) -> SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("pool");
    satchel_core::db::init(&pool).await.expect("migrations");
    sqlx::query(
        "INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
         VALUES ('acc', 'Me', 'me@example.test', ?, '{}', '', '2026-01-01', '2026-01-01')",
    )
    .bind(provider)
    .execute(&pool)
    .await
    .expect("account");
    for (id, kind, remote) in [
        ("inbox", "inbox", "INBOX"),
        ("spam", "spam", "Spam"),
        ("sent", "sent", "Sent"),
        ("work", "custom", "Work"),
    ] {
        sqlx::query(
            "INSERT INTO folders (id, account_id, name, full_path, folder_type, remote_id, uidvalidity, created_at, updated_at)
             VALUES (?, 'acc', ?, ?, ?, ?, 7, '2026-01-01', '2026-01-01')",
        )
        .bind(id)
        .bind(remote)
        .bind(remote)
        .bind(kind)
        .bind(remote)
        .execute(&pool)
        .await
        .expect("folder");
    }
    pool
}

/// One single-message thread `t-<id>` holding message `m-<id>`.
async fn message(pool: &SqlitePool, id: &str, folder: &str, from: &str, subject: &str, body: &str) {
    sqlx::query(
        "INSERT INTO threads (id, account_id, subject, workflow_state, board_column, last_message_date, created_at, updated_at)
         VALUES (?, 'acc', ?, 'inbox', 'inbox', '2026-01-01', '2026-01-01', '2026-01-01')
         ON CONFLICT(id) DO NOTHING",
    )
    .bind(format!("t-{id}"))
    .bind(subject)
    .execute(pool)
    .await
    .expect("thread");
    sqlx::query(
        "INSERT INTO messages (id, account_id, folder_id, thread_id, from_address, to_addresses, subject, body_text, date, remote_id, is_read, created_at, updated_at)
         VALUES (?, 'acc', ?, ?, ?, '[\"me@example.test\"]', ?, ?, '2026-01-01', ?, 1, '2026-01-01', '2026-01-01')",
    )
    .bind(format!("m-{id}"))
    .bind(folder)
    .bind(format!("t-{id}"))
    .bind(from)
    .bind(subject)
    .bind(body)
    .bind(format!("uid-{id}"))
    .execute(pool)
    .await
    .expect("message");
}

async fn totals(pool: &SqlitePool) -> (i64, i64) {
    let status = spam::status(pool).await.expect("status");
    (status.spam_messages, status.ham_messages)
}

async fn ledger(pool: &SqlitePool, message_id: &str) -> Option<String> {
    sqlx::query_scalar("SELECT label FROM spam_training WHERE message_id = ?")
        .bind(message_id)
        .fetch_optional(pool)
        .await
        .expect("ledger")
}

async fn spam_count(pool: &SqlitePool, token: &str) -> i64 {
    sqlx::query_scalar("SELECT COALESCE(SUM(spam_count), 0) FROM spam_tokens WHERE token = ?")
        .bind(spam::tokenizer::token_hash(token))
        .fetch_one(pool)
        .await
        .expect("token")
}

/// Ten spams about lotteries and ten hams about meetings: enough for the
/// model to be ready.
async fn trained_model(pool: &SqlitePool) {
    for n in 0..10 {
        let id = format!("s{n}");
        message(
            pool,
            &id,
            "inbox",
            &format!("winner{n}@lottery.example"),
            "You won the lottery jackpot",
            "Claim your jackpot prize now, wire the processing fee today",
        )
        .await;
        spam::train_messages(
            pool,
            &[format!("m-{id}")],
            Label::Spam,
            Signal::Explicit,
            None,
        )
        .await
        .expect("train spam");
        let id = format!("h{n}");
        message(
            pool,
            &id,
            "inbox",
            &format!("colleague{n}@work.example"),
            "Agenda for the project meeting",
            "Please review the agenda and the minutes before our meeting on Tuesday",
        )
        .await;
        spam::train_messages(
            pool,
            &[format!("m-{id}")],
            Label::Ham,
            Signal::Explicit,
            None,
        )
        .await
        .expect("train ham");
    }
}

#[tokio::test]
async fn mark_as_spam_trains_and_its_undo_untrains() {
    let pool = pool("imap").await;
    let queue = SyncQueue::new();
    message(
        &pool,
        "1",
        "inbox",
        "x@promo.example",
        "Cheap watches",
        "Buy cheap watches",
    )
    .await;

    let receipt = mail_actions::mark_threads_spam(&pool, &queue, &["t-1".into()])
        .await
        .expect("spam");
    assert_eq!(totals(&pool).await, (1, 0));
    assert_eq!(ledger(&pool, "m-1").await.as_deref(), Some("spam"));
    assert_eq!(spam_count(&pool, "subj:watches").await, 1);

    undo::undo_trash(
        &pool,
        &queue,
        receipt.undo_token.as_deref().expect("receipt"),
    )
    .await
    .expect("undo");
    assert_eq!(totals(&pool).await, (0, 0));
    assert_eq!(ledger(&pool, "m-1").await, None);
    assert_eq!(spam_count(&pool, "subj:watches").await, 0);
    let tokens: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM spam_tokens")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(tokens, 0, "untrained tokens with no counts are removed");
}

#[tokio::test]
async fn relabelling_moves_the_example_and_undo_restores_the_old_label() {
    let pool = pool("imap").await;
    message(
        &pool,
        "1",
        "inbox",
        "x@promo.example",
        "Newsletter",
        "Our weekly news",
    )
    .await;
    let ids = vec!["m-1".to_string()];

    spam::train_messages(&pool, &ids, Label::Ham, Signal::Explicit, None)
        .await
        .unwrap();
    // The same label twice counts once.
    assert_eq!(
        spam::train_messages(&pool, &ids, Label::Ham, Signal::Explicit, None)
            .await
            .unwrap(),
        0
    );
    assert_eq!(totals(&pool).await, (0, 1));

    spam::train_messages(&pool, &ids, Label::Spam, Signal::Explicit, Some("r1"))
        .await
        .unwrap();
    assert_eq!(totals(&pool).await, (1, 0));

    spam::untrain_receipt(&pool, "r1").await.unwrap();
    assert_eq!(totals(&pool).await, (0, 1));
    assert_eq!(ledger(&pool, "m-1").await.as_deref(), Some("ham"));
    // Running the reversal again changes nothing.
    assert_eq!(spam::untrain_receipt(&pool, "r1").await.unwrap(), 0);
    assert_eq!(totals(&pool).await, (0, 1));
}

#[tokio::test]
async fn weak_signals_never_override_a_label() {
    let pool = pool("imap").await;
    message(
        &pool,
        "1",
        "inbox",
        "x@promo.example",
        "Offer",
        "A great offer",
    )
    .await;
    let ids = vec!["m-1".to_string()];
    spam::train_messages(&pool, &ids, Label::Spam, Signal::Explicit, None)
        .await
        .unwrap();
    // Starring or replying to mail already called spam teaches nothing.
    spam::train_threads(&pool, &["t-1".into()], Label::Ham, Signal::Weak, None)
        .await
        .unwrap();
    assert_eq!(totals(&pool).await, (1, 0));

    message(
        &pool,
        "2",
        "inbox",
        "friend@home.example",
        "Dinner",
        "See you at dinner",
    )
    .await;
    spam::train_threads(&pool, &["t-2".into()], Label::Ham, Signal::Weak, None)
        .await
        .unwrap();
    assert_eq!(totals(&pool).await, (1, 1));
}

#[tokio::test]
async fn own_messages_are_never_examples() {
    let pool = pool("imap").await;
    message(
        &pool,
        "1",
        "sent",
        "me@example.test",
        "Hello",
        "My own words",
    )
    .await;
    sqlx::query("UPDATE messages SET is_sent = 1")
        .execute(&pool)
        .await
        .unwrap();
    spam::train_threads(&pool, &["t-1".into()], Label::Spam, Signal::Explicit, None)
        .await
        .unwrap();
    assert_eq!(totals(&pool).await, (0, 0));
}

#[tokio::test]
async fn pruning_drops_the_weakest_tokens_first() {
    let pool = pool("imap").await;
    let mut conn = pool.acquire().await.unwrap();
    for (token, count, seen) in [
        (1_i64, 9_i64, "2026-03-01"),
        (2, 1, "2026-01-01"),
        (3, 1, "2026-02-01"),
        (4, 5, "2026-01-01"),
    ] {
        sqlx::query(
            "INSERT INTO spam_tokens (token, spam_count, ham_count, last_seen) VALUES (?, ?, 0, ?)",
        )
        .bind(token)
        .bind(count)
        .bind(seen)
        .execute(&mut *conn)
        .await
        .unwrap();
    }
    assert_eq!(
        store::prune(&mut conn, 10, 2).await.unwrap(),
        0,
        "under the cap"
    );
    assert_eq!(store::prune(&mut conn, 3, 2).await.unwrap(), 2);
    let left: Vec<i64> = sqlx::query_scalar("SELECT token FROM spam_tokens ORDER BY token")
        .fetch_all(&mut *conn)
        .await
        .unwrap();
    assert_eq!(left, vec![1, 4], "once-seen tokens go, oldest first");
}

#[tokio::test]
async fn new_mail_is_scored_once_the_model_is_ready() {
    let pool = pool("imap").await;
    let queue = SyncQueue::new();
    message(
        &pool,
        "early",
        "inbox",
        "a@lottery.example",
        "Jackpot",
        "jackpot prize",
    )
    .await;
    let outcome = spam::classify_incoming(
        &pool,
        &queue,
        IncomingMessage {
            provider: "imap",
            message_id: "m-early",
            thread_id: "t-early",
            from_address: "a@lottery.example",
            hint_headers: Some("X-Spam-Flag: YES"),
        },
    )
    .await
    .unwrap();
    assert_eq!(outcome, ClassifyOutcome::Unscored, "too little training");

    trained_model(&pool).await;
    message(
        &pool,
        "new",
        "inbox",
        "boss@lottery.example",
        "Lottery jackpot winner",
        "Claim your prize, pay the fee",
    )
    .await;
    let outcome = spam::classify_incoming(
        &pool,
        &queue,
        IncomingMessage {
            provider: "imap",
            message_id: "m-new",
            thread_id: "t-new",
            from_address: "boss@lottery.example",
            hint_headers: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(outcome, ClassifyOutcome::Scored(Verdict::Spam));
    // Classified once: a second look is a no-op.
    let again = spam::classify_incoming(
        &pool,
        &queue,
        IncomingMessage {
            provider: "imap",
            message_id: "m-new",
            thread_id: "t-new",
            from_address: "boss@lottery.example",
            hint_headers: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(again, ClassifyOutcome::Skipped);

    // Mark only: the thread stays where it is, with the mark on it.
    let folder: String = sqlx::query_scalar(
        "SELECT COALESCE(pending_folder_id, folder_id) FROM messages WHERE id = 'm-new'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(folder, "inbox");
    let thread = satchel_core::db::threads::ThreadRepository::new(&pool)
        .get("t-new")
        .await
        .expect("thread");
    assert!(thread.likely_spam);

    message(
        &pool,
        "meet",
        "inbox",
        "pm@work.example",
        "Meeting agenda",
        "Review the minutes before the meeting",
    )
    .await;
    let ham = spam::classify_incoming(
        &pool,
        &queue,
        IncomingMessage {
            provider: "imap",
            message_id: "m-meet",
            thread_id: "t-meet",
            from_address: "pm@work.example",
            hint_headers: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(ham, ClassifyOutcome::Scored(Verdict::Ham));
}

#[tokio::test]
async fn hint_headers_reach_training_through_the_verdict() {
    let pool = pool("imap").await;
    let queue = SyncQueue::new();
    message(&pool, "1", "inbox", "a@b.example", "Hello", "text").await;
    spam::classify_incoming(
        &pool,
        &queue,
        IncomingMessage {
            provider: "pop3",
            message_id: "m-1",
            thread_id: "t-1",
            from_address: "a@b.example",
            hint_headers: Some("X-Spam-Flag: YES"),
        },
    )
    .await
    .unwrap();
    spam::train_messages(&pool, &["m-1".into()], Label::Spam, Signal::Explicit, None)
        .await
        .unwrap();
    assert_eq!(spam_count(&pool, "hdr:x-spam-flag:yes").await, 1);
}

#[tokio::test]
async fn providers_with_their_own_filter_and_the_off_mode_are_left_alone() {
    let pool = pool("gmail").await;
    let queue = SyncQueue::new();
    trained_model(&pool).await;
    message(
        &pool,
        "new",
        "inbox",
        "boss@lottery.example",
        "Lottery jackpot winner",
        "Claim your prize",
    )
    .await;
    let incoming = IncomingMessage {
        provider: "gmail",
        message_id: "m-new",
        thread_id: "t-new",
        from_address: "boss@lottery.example",
        hint_headers: None,
    };
    assert_eq!(
        spam::classify_incoming(&pool, &queue, incoming.clone())
            .await
            .unwrap(),
        ClassifyOutcome::Skipped
    );
    spam::set_mode(&pool, SpamFilterMode::Off).await.unwrap();
    let imap = IncomingMessage {
        provider: "imap",
        ..incoming
    };
    assert_eq!(
        spam::classify_incoming(&pool, &queue, imap).await.unwrap(),
        ClassifyOutcome::Skipped
    );
}

#[tokio::test]
async fn move_mode_files_spam_remote_first_and_not_spam_undoes_it() {
    let pool = pool("imap").await;
    let queue = SyncQueue::new();
    trained_model(&pool).await;
    spam::set_mode(&pool, SpamFilterMode::Move).await.unwrap();
    message(
        &pool,
        "new",
        "inbox",
        "boss@lottery.example",
        "Lottery jackpot winner",
        "Claim your jackpot prize, wire the fee",
    )
    .await;
    let before = totals(&pool).await;

    let outcome = spam::classify_incoming(
        &pool,
        &queue,
        IncomingMessage {
            provider: "imap",
            message_id: "m-new",
            thread_id: "t-new",
            from_address: "boss@lottery.example",
            hint_headers: None,
        },
    )
    .await
    .unwrap();
    let ClassifyOutcome::Moved {
        receipt_id: Some(receipt_id),
    } = outcome
    else {
        panic!("expected a move with a receipt, got {outcome:?}");
    };
    // The provider move is queued and the local copy waits for it.
    let (folder, pending): (String, Option<String>) =
        sqlx::query_as("SELECT folder_id, pending_folder_id FROM messages WHERE id = 'm-new'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        (folder.as_str(), pending.as_deref()),
        ("inbox", Some("spam"))
    );
    let moves: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sync_queue WHERE json_extract(payload, '$.kind') = 'workflow_move'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(moves, 1);
    let stored: Option<String> =
        sqlx::query_scalar("SELECT receipt_id FROM spam_verdicts WHERE message_id = 'm-new'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(stored.as_deref(), Some(receipt_id.as_str()));
    assert_eq!(
        totals(&pool).await,
        before,
        "the filter's own verdict is not an example"
    );

    let outcome = mail_actions::mark_threads_not_spam(&pool, &queue, &["t-new".into()])
        .await
        .unwrap();
    assert_eq!(outcome.moved_thread_ids, vec!["t-new".to_string()]);
    let (folder, pending): (String, Option<String>) =
        sqlx::query_as("SELECT folder_id, pending_folder_id FROM messages WHERE id = 'm-new'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((folder.as_str(), pending), ("inbox", None));
    let state: String = sqlx::query_scalar("SELECT workflow_state FROM threads WHERE id = 't-new'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(state, "inbox");
    assert_eq!(ledger(&pool, "m-new").await.as_deref(), Some("ham"));
    assert_eq!(totals(&pool).await, (before.0, before.1 + 1));
    let thread = satchel_core::db::threads::ThreadRepository::new(&pool)
        .get("t-new")
        .await
        .unwrap();
    assert!(!thread.likely_spam, "Not spam clears the mark");
}

#[tokio::test]
async fn senders_the_user_wrote_to_are_never_judged() {
    let pool = pool("imap").await;
    let queue = SyncQueue::new();
    trained_model(&pool).await;
    spam::set_mode(&pool, SpamFilterMode::Move).await.unwrap();
    // A message the user sent to the address.
    sqlx::query(
        "INSERT INTO threads (id, account_id, subject, last_message_date, created_at, updated_at)
         VALUES ('t-out', 'acc', 'Hi', '2026-01-01', '2026-01-01', '2026-01-01')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO messages (id, account_id, folder_id, thread_id, from_address, to_addresses, date, is_sent, created_at, updated_at)
         VALUES ('m-out', 'acc', 'sent', 't-out', 'me@example.test', '[\"Boss@Lottery.example\"]', '2026-01-01', 1, '2026-01-01', '2026-01-01')",
    )
    .execute(&pool)
    .await
    .unwrap();
    message(
        &pool,
        "new",
        "inbox",
        "boss@lottery.example",
        "Lottery jackpot winner",
        "Claim your jackpot prize",
    )
    .await;
    let outcome = spam::classify_incoming(
        &pool,
        &queue,
        IncomingMessage {
            provider: "imap",
            message_id: "m-new",
            thread_id: "t-new",
            from_address: "boss@lottery.example",
            hint_headers: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(outcome, ClassifyOutcome::Unscored);
    let pending: Option<String> =
        sqlx::query_scalar("SELECT pending_folder_id FROM messages WHERE id = 'm-new'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(pending, None);
}

#[tokio::test]
async fn not_spam_brings_mail_back_from_spam_without_a_receipt() {
    let pool = pool("pop3").await;
    let queue = SyncQueue::new();
    message(
        &pool,
        "1",
        "spam",
        "news@shop.example",
        "Your order",
        "Order shipped",
    )
    .await;
    sqlx::query(
        "UPDATE threads SET workflow_state = 'done', board_column = 'done' WHERE id = 't-1'",
    )
    .execute(&pool)
    .await
    .unwrap();

    let outcome = mail_actions::mark_threads_not_spam(&pool, &queue, &["t-1".into(), "t-1".into()])
        .await
        .unwrap();
    assert_eq!(outcome.thread_ids, vec!["t-1".to_string()]);
    assert_eq!(outcome.moved_thread_ids, vec!["t-1".to_string()]);
    // POP3 has no server folders: the move is local and immediate.
    let folder: String = sqlx::query_scalar("SELECT folder_id FROM messages WHERE id = 'm-1'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(folder, "inbox");
    let state: String = sqlx::query_scalar("SELECT workflow_state FROM threads WHERE id = 't-1'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(state, "inbox");
    assert_eq!(totals(&pool).await, (0, 1));
}

#[tokio::test]
async fn an_imap_move_carries_the_ledger_to_the_new_local_id() {
    let pool = pool("imap").await;
    message(
        &pool,
        "1",
        "inbox",
        "x@promo.example",
        "Offer",
        "Offer text",
    )
    .await;
    spam::train_messages(
        &pool,
        &["m-1".into()],
        Label::Spam,
        Signal::Explicit,
        Some("r"),
    )
    .await
    .unwrap();
    let mut tx = pool.begin().await.unwrap();
    undo::remap_message_in_transaction(&mut tx, "m-1", "m-1-moved")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(ledger(&pool, "m-1-moved").await.as_deref(), Some("spam"));
    assert_eq!(spam::untrain_receipt(&pool, "r").await.unwrap(), 1);
    assert_eq!(totals(&pool).await, (0, 0));
}

#[tokio::test]
async fn bootstrap_learns_from_the_spam_folder_and_read_inbox_mail() {
    let pool = pool("imap").await;
    for n in 0..3 {
        message(
            &pool,
            &format!("s{n}"),
            "spam",
            "x@promo.example",
            "Offer",
            "Cheap offer",
        )
        .await;
        message(
            &pool,
            &format!("h{n}"),
            "inbox",
            "pm@work.example",
            "Agenda",
            "Meeting agenda",
        )
        .await;
    }
    // Unread Inbox mail and the user's own mail are not examples.
    message(
        &pool,
        "unread",
        "inbox",
        "pm@work.example",
        "Agenda",
        "Meeting agenda",
    )
    .await;
    sqlx::query("UPDATE messages SET is_read = 0 WHERE id = 'm-unread'")
        .execute(&pool)
        .await
        .unwrap();
    message(
        &pool,
        "own",
        "inbox",
        "me@example.test",
        "Note to self",
        "Remember",
    )
    .await;

    assert!(
        satchel_core::db::data_repairs::is_pending(&pool, &bootstrap::SPAM_BOOTSTRAP)
            .await
            .unwrap()
    );
    satchel_core::db::data_repairs::run_repair(&pool, &bootstrap::SPAM_BOOTSTRAP, 2, None, |_| {})
        .await
        .unwrap();
    assert_eq!(totals(&pool).await, (3, 3));
    assert!(
        !satchel_core::db::data_repairs::is_pending(&pool, &bootstrap::SPAM_BOOTSTRAP)
            .await
            .unwrap()
    );
    assert_eq!(ledger(&pool, "m-unread").await, None);
    assert_eq!(ledger(&pool, "m-own").await, None);
}

#[tokio::test]
async fn deleting_a_message_drops_its_spam_rows() {
    let pool = pool("imap").await;
    message(
        &pool,
        "1",
        "inbox",
        "x@promo.example",
        "Offer",
        "Offer text",
    )
    .await;
    spam::train_messages(&pool, &["m-1".into()], Label::Spam, Signal::Explicit, None)
        .await
        .unwrap();
    sqlx::query("DELETE FROM messages WHERE id = 'm-1'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(ledger(&pool, "m-1").await, None);
    // What the model learned stays learned.
    assert_eq!(totals(&pool).await, (1, 0));
}
