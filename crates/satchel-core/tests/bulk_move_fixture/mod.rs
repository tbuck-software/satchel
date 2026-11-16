//! Shared seed for the bulk move tests.
#![allow(dead_code)]

use sqlx::sqlite::SqlitePoolOptions;
use sqlx::SqlitePool;

/// Accounts of every provider family the move path distinguishes.
pub const ACCOUNTS: [(&str, &str); 4] = [
    ("imap-a", "imap"),
    ("imap-b", "imap"),
    ("gmail", "gmail"),
    ("pop", "pop3"),
];

pub async fn pool() -> SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("pool");
    satchel_core::db::init(&pool).await.expect("migrations");
    pool
}

/// `threads` threads spread over four accounts. Each has two Inbox messages
/// (one in a second folder for IMAP), a Sent copy, and a Gmail thread carries
/// labels. `imap-a` already has an Archive folder; the others get theirs
/// provisioned by the move.
pub async fn seed(pool: &SqlitePool, threads: usize) -> Vec<String> {
    let mut tx = pool.begin().await.expect("seed tx");
    for (account, provider) in ACCOUNTS {
        sqlx::query(
            "INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
             VALUES (?, ?, ?, ?, '{}', '', '2026-01-01', '2026-01-01')",
        )
        .bind(account)
        .bind(account)
        .bind(format!("{account}@example.test"))
        .bind(provider)
        .execute(&mut *tx)
        .await
        .expect("account");
        let mut folders = vec![
            ("inbox", "INBOX", "inbox", Some(7)),
            ("sent", "Sent", "sent", Some(8)),
            ("projects", "Projects", "custom", Some(9)),
        ];
        if account == "imap-a" {
            folders.push(("archive", "Archive", "archive", Some(10)));
        }
        if provider == "gmail" {
            folders = vec![
                ("inbox", "INBOX", "inbox", None),
                ("sent", "SENT", "sent", None),
                ("projects", "Label_7", "custom", None),
            ];
        }
        for (suffix, remote, kind, uidvalidity) in folders {
            sqlx::query(
                "INSERT INTO folders (id, account_id, name, full_path, folder_type, remote_id, uidvalidity, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, '2026-01-01', '2026-01-01')",
            )
            .bind(format!("{account}-{suffix}"))
            .bind(account)
            .bind(remote)
            .bind(remote)
            .bind(kind)
            .bind(remote)
            .bind(uidvalidity)
            .execute(&mut *tx)
            .await
            .expect("folder");
        }
    }
    let mut ids = Vec::with_capacity(threads);
    for n in 0..threads {
        let (account, provider) = ACCOUNTS[n % ACCOUNTS.len()];
        let thread = format!("t{n:05}");
        sqlx::query(
            "INSERT INTO threads (id, account_id, subject, last_message_date, workflow_state, board_column, created_at, updated_at)
             VALUES (?, ?, 'Subject', '2026-01-01', 'inbox', 'inbox', '2026-01-01', '2026-01-01')",
        )
        .bind(&thread)
        .bind(account)
        .execute(&mut *tx)
        .await
        .expect("thread");
        let second_folder = if provider == "imap" && n % 3 == 0 {
            "projects"
        } else {
            "inbox"
        };
        for (index, (folder, is_sent)) in [("inbox", false), (second_folder, false), ("sent", true)]
            .into_iter()
            .enumerate()
        {
            let remote = if provider == "gmail" {
                format!("g{n}x{index}")
            } else if provider == "pop3" {
                format!("UIDL-{n}-{index}")
            } else {
                (n * 3 + index + 1).to_string()
            };
            let labels = if provider == "gmail" {
                "[\"INBOX\",\"Label_7\"]"
            } else {
                "[]"
            };
            sqlx::query(
                "INSERT INTO messages (id, account_id, folder_id, thread_id, from_address, to_addresses, subject, date, is_read, is_sent, remote_id, provider_labels, created_at, updated_at)
                 VALUES (?, ?, ?, ?, 'sender@example.test', '[\"me@example.test\"]', 'Subject', ?, ?, ?, ?, ?, '2026-01-01', '2026-01-01')",
            )
            .bind(format!("{thread}-m{index}"))
            .bind(account)
            .bind(format!("{account}-{folder}"))
            .bind(&thread)
            .bind(format!("2026-01-01T00:0{index}:00Z"))
            .bind(i64::from(index == 2))
            .bind(is_sent)
            .bind(remote)
            .bind(labels)
            .execute(&mut *tx)
            .await
            .expect("message");
        }
        ids.push(thread);
    }
    tx.commit().await.expect("seed commit");
    ids
}
