//! The local spam filter end to end over a migrated database: training,
//! untraining through Undo, relabelling, weak signals, pruning, the gates
//! before any verdict and before Move, the bootstrap, classification of new
//! mail and Not spam.

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
    add_account(&pool, "acc", "me@example.test", provider).await;
    pool
}

async fn add_account(pool: &SqlitePool, id: &str, email: &str, provider: &str) {
    sqlx::query(
        "INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
         VALUES (?, 'Me', ?, ?, '{}', '', '2026-01-01', '2026-01-01')",
    )
    .bind(id)
    .bind(email)
    .bind(provider)
    .execute(pool)
    .await
    .expect("account");
    for (kind, remote) in [
        ("inbox", "INBOX"),
        ("spam", "Spam"),
        ("sent", "Sent"),
        ("archive", "Archive"),
    ] {
        let folder = if id == "acc" {
            kind.to_string()
        } else {
            format!("{id}-{kind}")
        };
        sqlx::query(
            "INSERT INTO folders (id, account_id, name, full_path, folder_type, remote_id, uidvalidity, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, 7, '2026-01-01', '2026-01-01')",
        )
        .bind(folder)
        .bind(id)
        .bind(remote)
        .bind(remote)
        .bind(kind)
        .bind(remote)
        .execute(pool)
        .await
        .expect("folder");
    }
}

/// Let the model judge: the bootstrap marker gone, as after its run.
async fn finish_bootstrap(pool: &SqlitePool) {
    sqlx::query("DELETE FROM app_settings WHERE key = ?")
        .bind(store::BOOTSTRAP_MARKER)
        .execute(pool)
        .await
        .unwrap();
}

#[derive(Clone)]
struct Mail {
    id: String,
    account: &'static str,
    thread: String,
    folder: String,
    from: String,
    subject: String,
    body: String,
    unsubscribe: bool,
    dmarc: Option<&'static str>,
}

impl Mail {
    fn new(id: &str, folder: &str, from: &str, subject: &str, body: &str) -> Self {
        Self {
            id: format!("m-{id}"),
            account: "acc",
            thread: format!("t-{id}"),
            folder: folder.into(),
            from: from.into(),
            subject: subject.into(),
            body: body.into(),
            unsubscribe: false,
            dmarc: None,
        }
    }

    fn in_thread(mut self, thread: &str) -> Self {
        self.thread = thread.into();
        self
    }
}

async fn insert(pool: &SqlitePool, mail: &Mail) {
    sqlx::query(
        "INSERT INTO threads (id, account_id, subject, workflow_state, board_column, last_message_date, created_at, updated_at)
         VALUES (?, ?, ?, 'inbox', 'inbox', '2026-01-01', '2026-01-01', '2026-01-01')
         ON CONFLICT(id) DO NOTHING",
    )
    .bind(&mail.thread)
    .bind(mail.account)
    .bind(&mail.subject)
    .execute(pool)
    .await
    .expect("thread");
    sqlx::query(
        "INSERT INTO messages (id, account_id, folder_id, thread_id, from_address, to_addresses, subject, body_text,
                               list_unsubscribe, auth_dmarc, date, remote_id, is_read, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, '[\"me@example.test\"]', ?, ?, ?, ?, '2026-01-01', ?, 1, '2026-01-01', '2026-01-01')",
    )
    .bind(&mail.id)
    .bind(mail.account)
    .bind(&mail.folder)
    .bind(&mail.thread)
    .bind(&mail.from)
    .bind(&mail.subject)
    .bind(&mail.body)
    .bind(mail.unsubscribe.then_some("<mailto:leave@list.example>"))
    .bind(mail.dmarc)
    .bind(format!("uid-{}", mail.id))
    .execute(pool)
    .await
    .expect("message");
}

async fn message(pool: &SqlitePool, id: &str, folder: &str, from: &str, subject: &str, body: &str) {
    insert(pool, &Mail::new(id, folder, from, subject, body)).await;
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

async fn folder_of(pool: &SqlitePool, message_id: &str) -> (String, Option<String>) {
    sqlx::query_as("SELECT folder_id, pending_folder_id FROM messages WHERE id = ?")
        .bind(message_id)
        .fetch_one(pool)
        .await
        .expect("message folder")
}

async fn classify(pool: &SqlitePool, queue: &SyncQueue, mail: &Mail) -> ClassifyOutcome {
    spam::classify_incoming(
        pool,
        queue,
        IncomingMessage {
            provider: "imap",
            message_id: &mail.id,
            thread_id: &mail.thread,
            from_address: &mail.from,
            hint_headers: None,
        },
    )
    .await
    .expect("classify")
}

/* ------------------------------------------------------------------------ */
/* A small, realistic corpus                                                */
/* ------------------------------------------------------------------------ */

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Spam,
    Personal,
    Newsletter,
    Receipt,
    /// Legitimate promotional mail: sales, codes, "limited time".
    Promotion,
}

/// Deterministic, so a failure is reproducible.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) as usize
    }

    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.next() % items.len()]
    }

    fn sentences(&mut self, items: &[&str], count: usize) -> String {
        (0..count)
            .map(|_| self.pick(items))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

fn sample(rng: &mut Rng, kind: Kind, n: usize, tag: &str) -> Mail {
    let id = format!("{tag}-{kind:?}-{n}");
    let (from, subject, body, unsubscribe) = match kind {
        Kind::Spam => (
            format!(
                "{}{n}@{}{}.{}",
                rng.pick(&["info", "support", "winner", "admin", "promo", "service"]),
                rng.pick(&["cash", "prize", "secure-login", "deal", "crypto", "pharma"]),
                n % 17,
                rng.pick(&["biz", "top", "xyz", "info"])
            ),
            rng.pick(&[
                "You have been selected for a cash prize",
                "Final notice: claim your reward",
                "Exclusive offer just for you",
                "Your account will be suspended",
                "Earn money from home fast",
                "Cheap medication without prescription",
                "Double your crypto in 24 hours",
                "Urgent business proposal",
                "Problem with your order",
                "Invoice overdue: action required",
            ])
            .to_string(),
            rng.sentences(
                &[
                    "Click the link below to claim your reward immediately.",
                    "This offer expires in 24 hours, act now.",
                    "Send your bank details and a small processing fee.",
                    "Verify your password to avoid account suspension.",
                    "Guaranteed returns with zero risk investment.",
                    "Order discount pills online, no prescription needed.",
                    "Dear beneficiary, the transfer of funds awaits your reply.",
                    "You are the lucky winner of our international lottery.",
                    "Limited time: earn thousands of dollars weekly.",
                    "Confidential: kindly reply with your full name and phone number.",
                    "Your order could not be delivered, confirm your shipping address here.",
                    "Your account invoice is overdue, pay now to avoid suspension.",
                ],
                5,
            ),
            false,
        ),
        Kind::Personal => (
            format!(
                "{}@{}",
                rng.pick(&["anna", "ben", "clara", "david", "eva", "felix", "greta", "hannes"]),
                rng.pick(&["work.example", "family.example", "club.example"])
            ),
            rng.pick(&[
                "Notes from today's meeting",
                "Lunch on Thursday?",
                "Draft of the quarterly report",
                "Can you review my pull request",
                "Weekend plans",
                "Updated project timeline",
                "Question about the budget",
                "Photos from the trip",
            ])
            .to_string(),
            rng.sentences(
                &[
                    "I attached the notes from our meeting this morning.",
                    "Could you take a look before Friday?",
                    "Let me know if Thursday works for lunch.",
                    "The timeline moved by a week because of the review.",
                    "Thanks again for your help with the slides.",
                    "Do you have time for a quick call tomorrow?",
                    "Here are the photos from the weekend.",
                    "I updated the budget sheet with the new numbers.",
                    "See you at the office on Monday.",
                    "The kids are looking forward to the visit.",
                ],
                4,
            ),
            false,
        ),
        Kind::Newsletter => (
            format!(
                "{}@{}",
                rng.pick(&["news", "digest", "updates", "hello"]),
                rng.pick(&[
                    "opensource.example",
                    "product.example",
                    "magazine.example",
                    "meetup.example"
                ])
            ),
            rng.pick(&[
                "This week in open source",
                "Your monthly product update",
                "New articles you might like",
                "Release notes for version 4",
                "Community digest",
                "Events near you this month",
            ])
            .to_string(),
            rng.sentences(
                &[
                    "Here are the most read articles of the week.",
                    "We shipped new features based on your feedback.",
                    "Read the full release notes on our blog.",
                    "Join the community call next Tuesday.",
                    "You are receiving this because you subscribed to our newsletter.",
                    "Manage your email preferences or unsubscribe at any time.",
                    "Our editors picked these stories for you.",
                    "Thank you for being part of the community.",
                ],
                4,
            ),
            true,
        ),
        Kind::Receipt => (
            format!(
                "{}@{}",
                rng.pick(&["orders", "billing", "noreply", "service"]),
                rng.pick(&[
                    "shop.example",
                    "rail.example",
                    "books.example",
                    "energy.example"
                ])
            ),
            rng.pick(&[
                "Your order has shipped",
                "Receipt for your payment",
                "Invoice for October",
                "Order confirmation",
                "Your package was delivered",
                "Booking confirmation",
            ])
            .to_string(),
            rng.sentences(
                &[
                    "Thank you for your order.",
                    "Your payment of 42.90 EUR was received.",
                    "Track your package with the tracking number below.",
                    "The invoice is attached as a PDF.",
                    "Your order will arrive on Wednesday.",
                    "View your order details in your account.",
                    "Questions? Contact our customer service.",
                    "Your booking reference is ABC123.",
                ],
                4,
            ),
            false,
        ),
        Kind::Promotion => (
            format!(
                "{}@{}",
                rng.pick(&["offers", "deals", "newsletter"]),
                rng.pick(&["shop.example", "books.example", "travel.example"])
            ),
            rng.pick(&[
                "Summer sale: 20% off everything",
                "Members get early access",
                "New arrivals this week",
                "Last chance: free shipping ends tonight",
                "A special offer for your next trip",
            ])
            .to_string(),
            rng.sentences(
                &[
                    "Our summer sale starts today with 20% off.",
                    "Members get free shipping on all orders.",
                    "Shop the new arrivals before they sell out.",
                    "Use code SUMMER20 at checkout, limited time only.",
                    "You are receiving this because you have an account with us.",
                    "Unsubscribe from these offers at any time.",
                    "Book now and save on your next trip.",
                ],
                4,
            ),
            true,
        ),
    };
    let mut mail = Mail::new(&id, "inbox", &from, &subject, &body);
    mail.unsubscribe = unsubscribe;
    mail
}

/// Train `spam` spams and `ham` hams — a quarter each personal mail,
/// newsletters, receipts and promotions — each its own thread.
async fn train_corpus(pool: &SqlitePool, seed: u64, spam: usize, ham: usize) {
    let mut rng = Rng(seed);
    let mut spam_ids = Vec::new();
    let mut ham_ids = Vec::new();
    for n in 0..spam {
        let mail = sample(&mut rng, Kind::Spam, n, &format!("train{seed}"));
        insert(pool, &mail).await;
        spam_ids.push(mail.id);
    }
    for n in 0..ham {
        let kind = [
            Kind::Personal,
            Kind::Newsletter,
            Kind::Receipt,
            Kind::Promotion,
        ][n % 4];
        let mail = sample(&mut rng, kind, n, &format!("train{seed}"));
        insert(pool, &mail).await;
        ham_ids.push(mail.id);
    }
    spam::train_messages(pool, &spam_ids, Label::Spam, Signal::Explicit, None)
        .await
        .expect("train spam");
    spam::train_messages(pool, &ham_ids, Label::Ham, Signal::Explicit, None)
        .await
        .expect("train ham");
}

/* ------------------------------------------------------------------------ */
/* Training                                                                 */
/* ------------------------------------------------------------------------ */

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
async fn an_explicit_label_upgrades_a_weak_one_and_its_undo_restores_it() {
    let pool = pool("imap").await;
    message(
        &pool,
        "1",
        "inbox",
        "friend@home.example",
        "Dinner",
        "See you at dinner",
    )
    .await;
    let ids = vec!["m-1".to_string()];
    spam::train_messages(&pool, &ids, Label::Ham, Signal::Weak, None)
        .await
        .unwrap();
    let mut conn = pool.acquire().await.unwrap();
    assert!(!store::rescued_sender(&mut conn, "friend@home.example")
        .await
        .unwrap());
    drop(conn);

    spam::train_messages(&pool, &ids, Label::Ham, Signal::Explicit, Some("r"))
        .await
        .unwrap();
    assert_eq!(totals(&pool).await, (0, 1), "the count does not double");
    let mut conn = pool.acquire().await.unwrap();
    assert!(store::rescued_sender(&mut conn, "Friend@Home.example")
        .await
        .unwrap());
    drop(conn);

    spam::untrain_receipt(&pool, "r").await.unwrap();
    let mut conn = pool.acquire().await.unwrap();
    let label = store::trained_label(&mut conn, "m-1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!((label.label, label.source), (Label::Ham, Signal::Weak));
    assert!(!store::rescued_sender(&mut conn, "friend@home.example")
        .await
        .unwrap());
    drop(conn);
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
    spam::train_messages(&pool, &["m-1".into()], Label::Spam, Signal::Explicit, None)
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
async fn a_thread_is_one_example_per_sender() {
    let pool = pool("imap").await;
    for n in 0..5 {
        insert(
            &pool,
            &Mail::new(
                &format!("a{n}"),
                "inbox",
                "flood@promo.example",
                "Offer",
                "Buy now",
            )
            .in_thread("t-long"),
        )
        .await;
    }
    insert(
        &pool,
        &Mail::new("b", "inbox", "other@promo.example", "Re: Offer", "Also buy")
            .in_thread("t-long"),
    )
    .await;
    spam::train_threads(
        &pool,
        &["t-long".into()],
        Label::Spam,
        Signal::Explicit,
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        totals(&pool).await,
        (2, 0),
        "five messages from one sender are one example"
    );
    // The relabel reaches the same two examples.
    spam::train_threads(
        &pool,
        &["t-long".into()],
        Label::Ham,
        Signal::Explicit,
        None,
    )
    .await
    .unwrap();
    assert_eq!(totals(&pool).await, (0, 2));
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

/* ------------------------------------------------------------------------ */
/* The gates before any verdict                                             */
/* ------------------------------------------------------------------------ */

#[tokio::test]
async fn a_thin_model_gives_no_verdict() {
    let pool = pool("imap").await;
    let queue = SyncQueue::new();
    train_corpus(&pool, 1, 10, 10).await;
    finish_bootstrap(&pool).await;
    let mail = sample(&mut Rng(99), Kind::Spam, 0, "new");
    insert(&pool, &mail).await;
    assert_eq!(
        classify(&pool, &queue, &mail).await,
        ClassifyOutcome::Unscored
    );
    let status = spam::status(&pool).await.unwrap();
    assert!(!status.ready);
    assert_eq!((status.spam_needed, status.ham_needed), (40, 40));
}

#[tokio::test]
async fn nothing_is_judged_before_the_bootstrap_has_finished() {
    let pool = pool("imap").await;
    let queue = SyncQueue::new();
    train_corpus(&pool, 2, 60, 60).await;
    let mail = sample(&mut Rng(98), Kind::Spam, 0, "new");
    insert(&pool, &mail).await;
    assert_eq!(
        classify(&pool, &queue, &mail).await,
        ClassifyOutcome::Unscored
    );
    assert!(spam::status(&pool).await.unwrap().bootstrap_pending);

    finish_bootstrap(&pool).await;
    let next = sample(&mut Rng(97), Kind::Spam, 1, "new");
    insert(&pool, &next).await;
    assert_eq!(
        classify(&pool, &queue, &next).await,
        ClassifyOutcome::Scored(Verdict::Spam)
    );
}

#[tokio::test]
async fn a_lopsided_model_never_calls_wanted_mail_spam() {
    let pool = pool("imap").await;
    let queue = SyncQueue::new();
    train_corpus(&pool, 3, 1_000, 30).await;
    finish_bootstrap(&pool).await;
    let status = spam::status(&pool).await.unwrap();
    assert!(!status.ready, "1000:30 is worse than 4:1");
    assert_eq!(status.ham_needed, 220);

    let mut rng = Rng(96);
    for (n, kind) in [
        Kind::Personal,
        Kind::Newsletter,
        Kind::Receipt,
        Kind::Promotion,
    ]
    .into_iter()
    .enumerate()
    {
        let mail = sample(&mut rng, kind, n, "wanted");
        insert(&pool, &mail).await;
        let outcome = classify(&pool, &queue, &mail).await;
        assert_ne!(outcome, ClassifyOutcome::Scored(Verdict::Spam), "{kind:?}");
    }
}

#[tokio::test]
async fn a_realistic_corpus_stays_under_the_false_positive_ceiling() {
    let pool = pool("imap").await;
    let queue = SyncQueue::new();
    train_corpus(&pool, 4, 60, 60).await;
    finish_bootstrap(&pool).await;

    // 48 unseen messages: 16 spam (some about orders and accounts), 12
    // personal, and 20 newsletters, receipts and promotions.
    let mut rng = Rng(2026);
    let mut false_positives = Vec::new();
    let mut caught = 0;
    for n in 0..48 {
        let kind = match n % 12 {
            0..=3 => Kind::Spam,
            4..=6 => Kind::Personal,
            7 => Kind::Newsletter,
            8 | 9 => Kind::Receipt,
            _ => Kind::Promotion,
        };
        let mail = sample(&mut rng, kind, n, "held-out");
        insert(&pool, &mail).await;
        let outcome = classify(&pool, &queue, &mail).await;
        let spam = outcome == ClassifyOutcome::Scored(Verdict::Spam);
        if kind == Kind::Spam {
            caught += usize::from(spam);
        } else if spam {
            false_positives.push((kind, mail.subject.clone()));
        }
    }
    assert!(
        false_positives.len() <= 1,
        "at most 1 of 32 wanted mails marked: {false_positives:?}"
    );
    assert!(
        caught >= 12,
        "the filter still catches spam: {caught} of 16"
    );
}

/* ------------------------------------------------------------------------ */
/* Classification and Move                                                  */
/* ------------------------------------------------------------------------ */

#[tokio::test]
async fn hint_headers_count_only_once_both_classes_carried_them() {
    let pool = pool("imap").await;
    train_corpus(&pool, 5, 60, 60).await;
    finish_bootstrap(&pool).await;
    let mut rng = Rng(95);
    // A message the body words say nothing about, so one hint would show.
    let probe = Mail::new("probe", "inbox", "someone@unknown.example", "Hello", "text");
    insert(&pool, &probe).await;
    let mut conn = pool.acquire().await.unwrap();
    let plain = store::features_for_message(&mut conn, &probe.id)
        .await
        .unwrap()
        .unwrap();
    let mut flagged = plain.clone();
    flagged.hint_headers = Some("X-Spam-Flag: YES".into());
    drop(conn);

    // Fifty hinted examples of each class, the flag set on the spams only.
    let mut ids = (Vec::new(), Vec::new());
    for n in 0..50 {
        for (kind, flag) in [(Kind::Spam, "YES"), (Kind::Personal, "NO")] {
            let mail = sample(&mut rng, kind, n, "hinted");
            insert(&pool, &mail).await;
            sqlx::query(
                "INSERT INTO spam_verdicts (message_id, score, verdict, hint_headers, classified_at)
                 VALUES (?, 0.5, 'none', ?, '')",
            )
            .bind(&mail.id)
            .bind(format!("X-Spam-Flag: {flag}"))
            .execute(&pool)
            .await
            .unwrap();
            if kind == Kind::Spam {
                ids.0.push(mail.id);
            } else {
                ids.1.push(mail.id);
            }
        }
        if n == 48 {
            // One short of the threshold: the hint still says nothing.
            spam::train_messages(&pool, &ids.0, Label::Spam, Signal::Explicit, None)
                .await
                .unwrap();
            spam::train_messages(&pool, &ids.1, Label::Ham, Signal::Explicit, None)
                .await
                .unwrap();
            let mut conn = pool.acquire().await.unwrap();
            let with = store::score_message(&mut conn, &flagged)
                .await
                .unwrap()
                .unwrap();
            let without = store::score_message(&mut conn, &plain)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(with.score, without.score);
            ids = (Vec::new(), Vec::new());
        }
    }
    spam::train_messages(&pool, &ids.0, Label::Spam, Signal::Explicit, None)
        .await
        .unwrap();
    spam::train_messages(&pool, &ids.1, Label::Ham, Signal::Explicit, None)
        .await
        .unwrap();
    let mut conn = pool.acquire().await.unwrap();
    assert_eq!(
        store::hint_totals(&mut conn).await.unwrap(),
        spam::classifier::ModelTotals { spam: 50, ham: 50 }
    );
    let with = store::score_message(&mut conn, &flagged)
        .await
        .unwrap()
        .unwrap();
    let without = store::score_message(&mut conn, &plain)
        .await
        .unwrap()
        .unwrap();
    assert!(with.score > without.score, "{with:?} vs {without:?}");
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
    train_corpus(&pool, 6, 60, 60).await;
    finish_bootstrap(&pool).await;
    let mail = sample(&mut Rng(94), Kind::Spam, 0, "new");
    insert(&pool, &mail).await;
    let incoming = IncomingMessage {
        provider: "gmail",
        message_id: &mail.id,
        thread_id: &mail.thread,
        from_address: &mail.from,
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
async fn move_behaves_like_mark_until_the_model_is_large_enough() {
    let pool = pool("imap").await;
    let queue = SyncQueue::new();
    train_corpus(&pool, 7, 60, 60).await;
    finish_bootstrap(&pool).await;
    spam::set_mode(&pool, SpamFilterMode::Move).await.unwrap();
    let mail = sample(&mut Rng(93), Kind::Spam, 0, "new");
    insert(&pool, &mail).await;
    assert_eq!(
        classify(&pool, &queue, &mail).await,
        ClassifyOutcome::Scored(Verdict::Spam)
    );
    assert_eq!(folder_of(&pool, &mail.id).await, ("inbox".into(), None));
    let status = spam::status(&pool).await.unwrap();
    assert!(status.ready && !status.move_ready);
    assert_eq!(
        (status.move_spam_needed, status.move_ham_needed),
        (140, 140)
    );
}

#[tokio::test]
async fn move_files_spam_remote_first_and_not_spam_undoes_it() {
    let pool = pool("imap").await;
    let queue = SyncQueue::new();
    train_corpus(&pool, 8, 200, 200).await;
    finish_bootstrap(&pool).await;
    spam::set_mode(&pool, SpamFilterMode::Move).await.unwrap();
    let mail = sample(&mut Rng(92), Kind::Spam, 0, "new");
    insert(&pool, &mail).await;
    let before = totals(&pool).await;

    let outcome = classify(&pool, &queue, &mail).await;
    let ClassifyOutcome::Moved {
        receipt_id: Some(receipt_id),
    } = outcome
    else {
        panic!("expected a move with a receipt, got {outcome:?}");
    };
    // The provider move is queued and the local copy waits for it.
    assert_eq!(
        folder_of(&pool, &mail.id).await,
        ("inbox".into(), Some("spam".into()))
    );
    let moves: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sync_queue WHERE json_extract(payload, '$.kind') = 'workflow_move'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(moves, 1);
    let (stored, moved_at): (Option<String>, Option<String>) =
        sqlx::query_as("SELECT receipt_id, moved_at FROM spam_verdicts WHERE message_id = ?")
            .bind(&mail.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(stored.as_deref(), Some(receipt_id.as_str()));
    assert!(moved_at.is_some());
    assert_eq!(
        totals(&pool).await,
        before,
        "the filter's own verdict is not an example"
    );
    let thread = satchel_core::db::threads::ThreadRepository::new(&pool)
        .get(&mail.thread)
        .await
        .unwrap();
    assert!(thread.likely_spam && thread.spam_moved);

    let outcome =
        mail_actions::mark_threads_not_spam(&pool, &queue, std::slice::from_ref(&mail.thread))
            .await
            .unwrap();
    assert_eq!(outcome.moved_thread_ids, vec![mail.thread.clone()]);
    assert_eq!(outcome.restored_thread_ids, vec![mail.thread.clone()]);
    assert_eq!(folder_of(&pool, &mail.id).await, ("inbox".into(), None));
    let state: String = sqlx::query_scalar("SELECT workflow_state FROM threads WHERE id = ?")
        .bind(&mail.thread)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(state, "inbox");
    assert_eq!(ledger(&pool, &mail.id).await.as_deref(), Some("ham"));
    assert_eq!(totals(&pool).await, (before.0, before.1 + 1));
    let thread = satchel_core::db::threads::ThreadRepository::new(&pool)
        .get(&mail.thread)
        .await
        .unwrap();
    assert!(
        !thread.likely_spam && !thread.spam_moved,
        "Not spam clears both marks"
    );
}

#[tokio::test]
async fn mail_rescued_with_not_spam_is_never_moved_back() {
    let pool = pool("imap").await;
    let queue = SyncQueue::new();
    train_corpus(&pool, 9, 200, 200).await;
    finish_bootstrap(&pool).await;
    let mut rng = Rng(91);
    // Marked, then rescued.
    let first = sample(&mut rng, Kind::Spam, 0, "rescued");
    insert(&pool, &first).await;
    assert_eq!(
        classify(&pool, &queue, &first).await,
        ClassifyOutcome::Scored(Verdict::Spam)
    );
    mail_actions::mark_threads_not_spam(&pool, &queue, std::slice::from_ref(&first.thread))
        .await
        .unwrap();

    spam::set_mode(&pool, SpamFilterMode::Move).await.unwrap();
    // A spam reply from someone else in the same thread marks, but does not
    // take the rescued message back to Spam.
    let mut reply = sample(&mut rng, Kind::Spam, 1, "reply");
    reply.thread = first.thread.clone();
    insert(&pool, &reply).await;
    assert_eq!(
        classify(&pool, &queue, &reply).await,
        ClassifyOutcome::Scored(Verdict::Spam)
    );
    assert_eq!(folder_of(&pool, &first.id).await, ("inbox".into(), None));
    assert_eq!(folder_of(&pool, &reply.id).await, ("inbox".into(), None));

    // The rescued sender is marked again in a new thread, never moved.
    let mut again = sample(&mut rng, Kind::Spam, 2, "again");
    again.from = first.from.clone();
    insert(&pool, &again).await;
    assert_eq!(
        classify(&pool, &queue, &again).await,
        ClassifyOutcome::Scored(Verdict::Spam)
    );
    assert_eq!(folder_of(&pool, &again.id).await, ("inbox".into(), None));
}

#[tokio::test]
async fn senders_the_user_wrote_to_are_never_judged() {
    let pool = pool("imap").await;
    let queue = SyncQueue::new();
    train_corpus(&pool, 10, 60, 60).await;
    finish_bootstrap(&pool).await;
    let mail = sample(&mut Rng(90), Kind::Spam, 0, "new");
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
         VALUES ('m-out', 'acc', 'sent', 't-out', 'me@example.test', json_array(?), '2026-01-01', 1, '2026-01-01', '2026-01-01')",
    )
    .bind(mail.from.to_uppercase())
    .execute(&pool)
    .await
    .unwrap();
    insert(&pool, &mail).await;
    assert_eq!(
        classify(&pool, &queue, &mail).await,
        ClassifyOutcome::Unscored
    );
}

#[tokio::test]
async fn an_unverified_own_address_is_marked_but_never_moved() {
    let pool = pool("imap").await;
    let queue = SyncQueue::new();
    train_corpus(&pool, 11, 200, 200).await;
    finish_bootstrap(&pool).await;
    spam::set_mode(&pool, SpamFilterMode::Move).await.unwrap();
    let mut rng = Rng(89);

    // Sextortion "from" the user's own address, DMARC failed.
    let mut forged = sample(&mut rng, Kind::Spam, 0, "forged");
    forged.from = "me@example.test".into();
    forged.dmarc = Some("fail");
    insert(&pool, &forged).await;
    assert_eq!(
        classify(&pool, &queue, &forged).await,
        ClassifyOutcome::Scored(Verdict::Spam)
    );
    assert_eq!(folder_of(&pool, &forged.id).await, ("inbox".into(), None));

    // Without Authentication-Results the address is taken at its word.
    let mut plain = sample(&mut rng, Kind::Spam, 1, "plain");
    plain.from = "me@example.test".into();
    insert(&pool, &plain).await;
    assert_eq!(
        classify(&pool, &queue, &plain).await,
        ClassifyOutcome::Unscored
    );
}

/* ------------------------------------------------------------------------ */
/* Not spam                                                                 */
/* ------------------------------------------------------------------------ */

#[tokio::test]
async fn not_spam_brings_mail_back_from_spam_and_its_undo_returns_it() {
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

    let receipt = mail_actions::mark_threads_not_spam(&pool, &queue, &["t-1".into(), "t-1".into()])
        .await
        .unwrap();
    assert_eq!(receipt.thread_ids, vec!["t-1".to_string()]);
    assert_eq!(receipt.moved_thread_ids, vec!["t-1".to_string()]);
    assert!(receipt.undo_token.is_some());
    assert_eq!(receipt.move_receipts.len(), 1);
    // POP3 has no server folders: the move is local and immediate.
    assert_eq!(folder_of(&pool, "m-1").await, ("inbox".into(), None));
    let state: String = sqlx::query_scalar("SELECT workflow_state FROM threads WHERE id = 't-1'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(state, "inbox");
    assert_eq!(totals(&pool).await, (0, 1));

    mail_actions::undo_not_spam(&pool, &queue, &receipt)
        .await
        .unwrap();
    assert_eq!(folder_of(&pool, "m-1").await, ("spam".into(), None));
    let state: String = sqlx::query_scalar("SELECT workflow_state FROM threads WHERE id = 't-1'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(state, "done");
    assert_eq!(totals(&pool).await, (0, 0));
    assert_eq!(ledger(&pool, "m-1").await, None);
}

#[tokio::test]
async fn not_spam_clears_the_mark_of_a_whole_thread_from_one_sender() {
    let pool = pool("imap").await;
    let queue = SyncQueue::new();
    for id in ["a", "b"] {
        let mail =
            Mail::new(id, "inbox", "promo@deal.example", "Offer", "Buy now").in_thread("t-two");
        insert(&pool, &mail).await;
        sqlx::query(
            "INSERT INTO spam_verdicts (message_id, score, verdict, classified_at)
             VALUES (?, 0.95, 'spam', '')",
        )
        .bind(&mail.id)
        .execute(&pool)
        .await
        .unwrap();
    }
    let threads = satchel_core::db::threads::ThreadRepository::new(&pool);
    assert!(threads.get("t-two").await.unwrap().likely_spam);

    let receipt = mail_actions::mark_threads_not_spam(&pool, &queue, &["t-two".into()])
        .await
        .unwrap();
    assert_eq!(
        totals(&pool).await,
        (0, 1),
        "one example for the one sender"
    );
    assert!(
        !threads.get("t-two").await.unwrap().likely_spam,
        "the example clears every message of that sender in the thread"
    );

    mail_actions::undo_not_spam(&pool, &queue, &receipt)
        .await
        .unwrap();
    assert!(
        threads.get("t-two").await.unwrap().likely_spam,
        "Undo brings the mark back"
    );
}

#[tokio::test]
async fn undo_of_not_spam_forgets_nothing_when_a_move_cannot_be_replayed() {
    let pool = pool("pop3").await;
    let queue = SyncQueue::new();
    message(
        &pool,
        "1",
        "inbox",
        "news@shop.example",
        "Your order",
        "Order shipped",
    )
    .await;
    let mut receipt = mail_actions::mark_threads_not_spam(&pool, &queue, &["t-1".into()])
        .await
        .unwrap();
    receipt.move_receipts.push("no-such-receipt".into());

    assert!(mail_actions::undo_not_spam(&pool, &queue, &receipt)
        .await
        .is_err());
    assert_eq!(
        ledger(&pool, "m-1").await.as_deref(),
        Some("ham"),
        "still learned"
    );
    assert_eq!(totals(&pool).await, (0, 1));
}

#[tokio::test]
async fn not_spam_restores_a_thread_the_filter_moved_twice() {
    let pool = pool("pop3").await;
    let queue = SyncQueue::new();
    insert(
        &pool,
        &Mail::new("a", "inbox", "x@promo.example", "Offer", "Buy").in_thread("t-x"),
    )
    .await;
    let first = mail_actions::move_spam_with_receipt(&pool, &queue, &["t-x".into()])
        .await
        .unwrap();
    insert(
        &pool,
        &Mail::new("b", "inbox", "y@promo.example", "Re: Offer", "Buy more").in_thread("t-x"),
    )
    .await;
    let second = mail_actions::move_spam_with_receipt(&pool, &queue, &["t-x".into()])
        .await
        .unwrap();
    for (id, receipt) in [("m-a", &first), ("m-b", &second)] {
        sqlx::query(
            "INSERT INTO spam_verdicts (message_id, score, verdict, receipt_id, moved_at, classified_at)
             VALUES (?, 0.995, 'spam', ?, '2026-01-01', '')",
        )
        .bind(id)
        .bind(receipt.undo_token.as_deref())
        .execute(&pool)
        .await
        .unwrap();
    }
    assert_eq!(folder_of(&pool, "m-a").await.0, "spam");
    assert_eq!(folder_of(&pool, "m-b").await.0, "spam");

    let receipt = mail_actions::mark_threads_not_spam(&pool, &queue, &["t-x".into()])
        .await
        .unwrap();
    assert_eq!(receipt.moved_thread_ids, vec!["t-x".to_string()]);
    assert_eq!(folder_of(&pool, "m-a").await.0, "inbox");
    assert_eq!(folder_of(&pool, "m-b").await.0, "inbox");
}

/* ------------------------------------------------------------------------ */
/* Bootstrap and bookkeeping                                                */
/* ------------------------------------------------------------------------ */

#[tokio::test]
async fn bootstrap_learns_from_spam_and_from_mail_the_user_acted_on() {
    let pool = pool("imap").await;
    add_account(&pool, "acc2", "me@second.test", "imap").await;
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
    }
    let mut other = Mail::new("s-other", "acc2-spam", "y@promo.example", "Offer", "Cheap");
    other.account = "acc2";
    insert(&pool, &other).await;
    // Acted on: starred, archived, Done, answered.
    message(
        &pool,
        "starred",
        "inbox",
        "a@work.example",
        "Agenda",
        "Meeting agenda",
    )
    .await;
    sqlx::query("UPDATE messages SET is_starred = 1 WHERE id = 'm-starred'")
        .execute(&pool)
        .await
        .unwrap();
    message(
        &pool,
        "archived",
        "archive",
        "b@work.example",
        "Minutes",
        "Meeting minutes",
    )
    .await;
    message(
        &pool,
        "done",
        "inbox",
        "c@work.example",
        "Plan",
        "Project plan",
    )
    .await;
    sqlx::query("UPDATE threads SET workflow_state = 'done' WHERE id = 't-done'")
        .execute(&pool)
        .await
        .unwrap();
    message(
        &pool,
        "answered",
        "inbox",
        "d@work.example",
        "Lunch",
        "Lunch today?",
    )
    .await;
    insert(
        &pool,
        &Mail::new("answer", "sent", "me@example.test", "Re: Lunch", "Yes").in_thread("t-answered"),
    )
    .await;
    sqlx::query("UPDATE messages SET is_sent = 1 WHERE id = 'm-answer'")
        .execute(&pool)
        .await
        .unwrap();
    // Merely read Inbox mail and the user's own mail are not examples.
    message(&pool, "read", "inbox", "e@shop.example", "Sale", "Big sale").await;
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
    assert!(
        !satchel_core::db::data_repairs::is_pending(&pool, &bootstrap::SPAM_BOOTSTRAP)
            .await
            .unwrap()
    );
    assert_eq!(
        totals(&pool).await,
        (4, 4),
        "both accounts' Spam, four acted-on mails"
    );
    for id in ["m-starred", "m-archived", "m-done", "m-answered"] {
        assert_eq!(ledger(&pool, id).await.as_deref(), Some("ham"), "{id}");
    }
    assert_eq!(ledger(&pool, "m-read").await, None);
    assert_eq!(ledger(&pool, "m-own").await, None);
    assert_eq!(ledger(&pool, "m-s-other").await.as_deref(), Some("spam"));
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

/* ------------------------------------------------------------------------ */
/* A bulk move a later batch stops                                          */
/* ------------------------------------------------------------------------ */

/// Every message write of `thread` fails, as a locked database would.
async fn fail_writes_of(pool: &SqlitePool, thread: &str) {
    sqlx::query(&format!(
        "CREATE TRIGGER stop_bulk_move BEFORE UPDATE ON messages WHEN new.thread_id = '{thread}'
         BEGIN SELECT RAISE(ABORT, 'database is locked'); END"
    ))
    .execute(pool)
    .await
    .unwrap();
}

#[tokio::test]
async fn mark_as_spam_a_later_batch_stops_teaches_only_what_moved() {
    let pool = pool("pop3").await;
    let queue = SyncQueue::new();
    message(
        &pool,
        "1",
        "inbox",
        "promo@deal.example",
        "Offer",
        "Buy now",
    )
    .await;
    message(
        &pool,
        "2",
        "inbox",
        "other@deal.example",
        "Offer",
        "Buy now",
    )
    .await;
    fail_writes_of(&pool, "t-2").await;

    mail_actions::set_move_batch_threads_for_tests(Some(1));
    let receipt = mail_actions::mark_threads_spam(&pool, &queue, &["t-1".into(), "t-2".into()])
        .await
        .expect("the first batch committed");
    mail_actions::set_move_batch_threads_for_tests(None);

    assert_eq!(receipt.stopped_early.expect("stopped").remaining, ["t-2"]);
    assert_eq!(folder_of(&pool, "m-1").await, ("spam".into(), None));
    assert_eq!(folder_of(&pool, "m-2").await, ("inbox".into(), None));
    assert_eq!(ledger(&pool, "m-1").await.as_deref(), Some("spam"));
    assert_eq!(
        ledger(&pool, "m-2").await,
        None,
        "mail that stayed teaches nothing"
    );
}

/// A refile a later batch stops keeps the lesson and reports the reason; a
/// retry files the rest and only then unlearns.
#[tokio::test]
async fn undo_of_not_spam_a_later_batch_stops_keeps_the_lesson_until_a_retry() {
    let pool = pool("pop3").await;
    let queue = SyncQueue::new();
    message(
        &pool,
        "1",
        "inbox",
        "news@shop.example",
        "Your order",
        "Order shipped",
    )
    .await;
    message(
        &pool,
        "2",
        "inbox",
        "info@shop.example",
        "Your order",
        "Order shipped",
    )
    .await;
    let mut receipt =
        mail_actions::mark_threads_not_spam(&pool, &queue, &["t-1".into(), "t-2".into()])
            .await
            .unwrap();
    // As if the filter had moved both to Spam before Not spam brought them back.
    receipt.restored_thread_ids = vec!["t-1".into(), "t-2".into()];
    fail_writes_of(&pool, "t-2").await;

    mail_actions::set_move_batch_threads_for_tests(Some(1));
    let stopped = mail_actions::undo_not_spam(&pool, &queue, &receipt).await;
    mail_actions::set_move_batch_threads_for_tests(None);

    assert_eq!(
        stopped,
        Err("backend.errors.mailMoveInterrupted".to_string())
    );
    assert_eq!(folder_of(&pool, "m-1").await, ("spam".into(), None));
    assert_eq!(folder_of(&pool, "m-2").await, ("inbox".into(), None));
    assert_eq!(
        ledger(&pool, "m-1").await.as_deref(),
        Some("ham"),
        "still learned"
    );
    assert_eq!(
        ledger(&pool, "m-2").await.as_deref(),
        Some("ham"),
        "still learned"
    );

    sqlx::query("DROP TRIGGER stop_bulk_move")
        .execute(&pool)
        .await
        .unwrap();
    mail_actions::undo_not_spam(&pool, &queue, &receipt)
        .await
        .expect("the retry finishes");
    assert_eq!(folder_of(&pool, "m-2").await, ("spam".into(), None));
    assert_eq!(ledger(&pool, "m-1").await, None);
    assert_eq!(ledger(&pool, "m-2").await, None);
}
