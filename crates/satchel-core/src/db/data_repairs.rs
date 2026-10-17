//! Data repairs that run after startup, a block at a time.
//!
//! sqlx runs each migration in one transaction, before the pool is handed out.
//! A migration that rewrites every thread therefore blocks the first frame for
//! as long as the rewrite takes, grows the WAL by everything it writes, and
//! starts over from nothing if the process dies halfway — which on a phone is
//! a question of when.
//!
//! The pattern instead: the migration only plants a marker,
//! `INSERT OR IGNORE INTO app_settings (key, value, updated_at)
//!  VALUES ('data_repair.<name>', '', …)`. After startup the worker finds the
//! marker, runs the matching [`DataRepair`] one block at a time and stores the
//! cursor with every block, in the same transaction as the block's writes. An
//! interrupted repair resumes where it stopped; a finished one deletes its
//! marker. Schema changes stay in the migration — only the data pass moves.

use futures::future::BoxFuture;
use sqlx::{SqliteConnection, SqlitePool};

/// Prefix of the marker keys in `app_settings`.
pub const MARKER_PREFIX: &str = "data_repair.";

/// Rows a block handles by default. Small enough that a block is a short
/// write lock, large enough that a big mailbox is not thousands of commits.
pub const DEFAULT_BLOCK_SIZE: i64 = 1_000;

/// What a repair does with one block.
///
/// Called with the cursor stored last (`""` on the first block) and the block
/// size, inside the transaction that will also store the cursor it returns.
/// `Ok(None)` means nothing is left.
pub type RepairBlock = for<'c> fn(
    &'c mut SqliteConnection,
    &'c str,
    i64,
) -> BoxFuture<'c, Result<Option<String>, sqlx::Error>>;

#[derive(Clone, Copy)]
pub struct DataRepair {
    /// The `<name>` in the marker key.
    pub name: &'static str,
    pub block: RepairBlock,
}

impl DataRepair {
    pub fn marker_key(&self) -> String {
        format!("{MARKER_PREFIX}{}", self.name)
    }
}

/// How a run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepairRun {
    /// No marker: nothing to do.
    NotPending,
    /// The marker is gone; every block ran.
    Completed { blocks: u64 },
    /// Stopped after `max_blocks`; the marker holds where to go on.
    Paused { blocks: u64 },
}

async fn stored_cursor(
    connection: &mut SqliteConnection,
    key: &str,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT value FROM app_settings WHERE key = ?")
        .bind(key)
        .fetch_optional(connection)
        .await
}

/// Whether a repair still has its marker.
pub async fn is_pending(pool: &SqlitePool, repair: &DataRepair) -> Result<bool, sqlx::Error> {
    let mut connection = pool.acquire().await?;
    Ok(stored_cursor(&mut connection, &repair.marker_key())
        .await?
        .is_some())
}

/// Run a repair's blocks until it is done or `max_blocks` have run.
///
/// Every block is its own `BEGIN IMMEDIATE` transaction: the cursor it reads,
/// the writes it makes and the cursor it stores commit together, so there is
/// no point at which a crash leaves the two out of step.
pub async fn run_repair(
    pool: &SqlitePool,
    repair: &DataRepair,
    block_size: i64,
    max_blocks: Option<u64>,
    mut on_block: impl FnMut(u64),
) -> Result<RepairRun, sqlx::Error> {
    let key = repair.marker_key();
    let mut blocks = 0_u64;
    loop {
        if max_blocks.is_some_and(|max| blocks >= max) {
            return Ok(RepairRun::Paused { blocks });
        }
        let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
        let Some(cursor) = stored_cursor(&mut tx, &key).await? else {
            tx.rollback().await?;
            return Ok(if blocks == 0 {
                RepairRun::NotPending
            } else {
                RepairRun::Completed { blocks }
            });
        };
        let next = (repair.block)(&mut tx, &cursor, block_size.max(1)).await?;
        let now = chrono::Utc::now().to_rfc3339();
        match &next {
            Some(next_cursor) => {
                sqlx::query("UPDATE app_settings SET value = ?, updated_at = ? WHERE key = ?")
                    .bind(next_cursor)
                    .bind(&now)
                    .bind(&key)
                    .execute(&mut *tx)
                    .await?;
            }
            None => {
                sqlx::query("DELETE FROM app_settings WHERE key = ?")
                    .bind(&key)
                    .execute(&mut *tx)
                    .await?;
            }
        }
        tx.commit().await?;
        blocks += 1;
        on_block(blocks);
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        if next.is_none() {
            return Ok(RepairRun::Completed { blocks });
        }
    }
}

/// The repairs this build knows. A marker without an entry here stays put
/// until a build that knows it comes along.
pub fn registered_repairs() -> &'static [DataRepair] {
    &[
        CC_INDEX,
        LEGACY_MIME,
        SANITIZE_MESSAGE_HTML,
        super::body_window::BODY_WINDOW,
        THREAD_FOLDERS,
        MESSAGE_ADDRESSES,
        FTS_PREFIX,
    ]
}

pub const SANITIZE_MESSAGE_HTML: DataRepair = DataRepair {
    name: "sanitize_message_html",
    block: sanitize_message_html_block,
};

fn sanitize_message_html_block<'c>(
    connection: &'c mut SqliteConnection,
    cursor: &'c str,
    limit: i64,
) -> BoxFuture<'c, Result<Option<String>, sqlx::Error>> {
    Box::pin(async move {
        let after: i64 = cursor.parse().unwrap_or(0);
        let rows: Vec<(i64, String)> = sqlx::query_as(
            "SELECT rowid, body_html FROM messages WHERE rowid > ? AND body_html IS NOT NULL ORDER BY rowid LIMIT ?",
        )
        .bind(after).bind(limit.min(32)).fetch_all(&mut *connection).await?;
        let Some(last) = rows.last().map(|row| row.0) else {
            return Ok(None);
        };
        let cleaned = tokio::task::spawn_blocking(move || {
            rows.into_iter()
                .map(|(rowid, html)| {
                    let sanitized = crate::email::sanitizer::sanitize_stored_email_html(&html);
                    (rowid, html, sanitized)
                })
                .collect::<Vec<_>>()
        })
        .await
        .map_err(|error| sqlx::Error::Io(std::io::Error::other(error)))?;
        for (rowid, original, sanitized) in cleaned {
            sqlx::query("UPDATE messages SET original_body_html = COALESCE(original_body_html, ?), body_html = ? WHERE rowid = ?")
                .bind(original).bind(sanitized).bind(rowid).execute(&mut *connection).await?;
        }
        Ok(Some(last.to_string()))
    })
}

pub const LEGACY_MIME: DataRepair = DataRepair {
    name: "legacy_mime",
    block: crate::providers::imap::repair_legacy_mime_block,
};

/// Indexes the Cc lists of messages stored before migration 0076.
pub const CC_INDEX: DataRepair = DataRepair {
    name: "cc_index",
    block: index_cc_block,
};

fn index_cc_block<'c>(
    connection: &'c mut SqliteConnection,
    cursor: &'c str,
    limit: i64,
) -> BoxFuture<'c, Result<Option<String>, sqlx::Error>> {
    Box::pin(async move {
        let after: i64 = cursor.parse().unwrap_or(0);
        let last: Option<i64> = sqlx::query_scalar(
            "SELECT MAX(rowid) FROM (SELECT rowid FROM messages WHERE rowid > ? ORDER BY rowid LIMIT ?)",
        )
        .bind(after)
        .bind(limit)
        .fetch_one(&mut *connection)
        .await?;
        let Some(last) = last else {
            return Ok(None);
        };
        // Rows inserted since the migration were indexed by the trigger already.
        sqlx::query("DELETE FROM message_cc_fts WHERE rowid > ? AND rowid <= ?")
            .bind(after)
            .bind(last)
            .execute(&mut *connection)
            .await?;
        sqlx::query(
            "INSERT INTO message_cc_fts (rowid, cc_addresses)
             SELECT rowid, cc_addresses FROM messages WHERE rowid > ? AND rowid <= ?",
        )
        .bind(after)
        .bind(last)
        .execute(&mut *connection)
        .await?;
        Ok(Some(last.to_string()))
    })
}

pub const THREAD_FOLDERS: DataRepair = DataRepair {
    name: "thread_folders",
    block: thread_folders_block,
};

fn thread_folders_block<'c>(
    connection: &'c mut SqliteConnection,
    cursor: &'c str,
    limit: i64,
) -> BoxFuture<'c, Result<Option<String>, sqlx::Error>> {
    Box::pin(async move {
        let after: i64 = cursor.parse().unwrap_or(0);
        let last: Option<i64> = sqlx::query_scalar(
            "SELECT MAX(rowid) FROM (SELECT rowid FROM messages WHERE rowid > ? ORDER BY rowid LIMIT ?)",
        ).bind(after).bind(limit).fetch_one(&mut *connection).await?;
        let Some(last) = last else {
            return Ok(None);
        };
        sqlx::query(
            "DELETE FROM thread_folders WHERE thread_id IN (
            SELECT thread_id FROM messages WHERE rowid > ? AND rowid <= ?)",
        )
        .bind(after)
        .bind(last)
        .execute(&mut *connection)
        .await?;
        sqlx::query(
            "INSERT INTO thread_folders(folder_id, thread_id, message_count, unread_count, visible)
             SELECT f.id, m.thread_id, COUNT(*), COALESCE(SUM(m.is_read = 0), 0),
                    CASE WHEN LOWER(TRIM(f.folder_type)) = 'inbox'
                         THEN COALESCE(t.workflow_state NOT IN ('aside', 'done', 'snoozed'), 0) ELSE 1 END
             FROM messages m JOIN folders f ON f.id = COALESCE(m.pending_folder_id, m.folder_id)
             LEFT JOIN threads t ON t.id = m.thread_id
             WHERE m.thread_id IN (SELECT thread_id FROM messages WHERE rowid > ? AND rowid <= ?)
             GROUP BY f.id, m.thread_id",
        ).bind(after).bind(last).execute(&mut *connection).await?;
        Ok(Some(last.to_string()))
    })
}

pub const FTS_PREFIX: DataRepair = DataRepair {
    name: "fts_prefix",
    block: fts_prefix_block,
};
fn fts_prefix_block<'c>(
    connection: &'c mut SqliteConnection,
    cursor: &'c str,
    limit: i64,
) -> BoxFuture<'c, Result<Option<String>, sqlx::Error>> {
    Box::pin(async move {
        let after: i64 = cursor.parse().unwrap_or(0);
        let last: Option<i64> = sqlx::query_scalar(
            "SELECT MAX(rowid) FROM (SELECT rowid FROM messages WHERE rowid > ? ORDER BY rowid LIMIT ?)",
        ).bind(after).bind(limit).fetch_one(&mut *connection).await?;
        let Some(last) = last else {
            sqlx::query(include_str!("fts_prefix_finish.sql"))
                .execute(&mut *connection)
                .await?;
            return Ok(None);
        };
        sqlx::query(
            "INSERT INTO messages_prefix_fts(rowid, subject, body_text, from_name, from_address, to_addresses)
             SELECT rowid, subject, body_text, from_name, from_address, to_addresses FROM messages
             WHERE rowid > ? AND rowid <= ? AND rowid NOT IN (SELECT message_rowid FROM message_prefix_indexed)",
        ).bind(after).bind(last).execute(&mut *connection).await?;
        sqlx::query("INSERT OR IGNORE INTO message_prefix_indexed SELECT rowid FROM messages WHERE rowid > ? AND rowid <= ?")
            .bind(after).bind(last).execute(&mut *connection).await?;
        Ok(Some(last.to_string()))
    })
}

pub const MESSAGE_ADDRESSES: DataRepair = DataRepair {
    name: "message_addresses",
    block: message_addresses_block,
};
fn message_addresses_block<'c>(
    connection: &'c mut SqliteConnection,
    cursor: &'c str,
    limit: i64,
) -> BoxFuture<'c, Result<Option<String>, sqlx::Error>> {
    Box::pin(async move {
        let after: i64 = cursor.parse().unwrap_or(0);
        let last: Option<i64> = sqlx::query_scalar(
            "SELECT MAX(rowid) FROM (SELECT rowid FROM messages WHERE rowid > ? ORDER BY rowid LIMIT ?)",
        ).bind(after).bind(limit).fetch_one(&mut *connection).await?;
        let Some(last) = last else {
            return Ok(None);
        };
        sqlx::query("DELETE FROM message_addresses WHERE message_id IN (SELECT id FROM messages WHERE rowid > ? AND rowid <= ?)")
            .bind(after).bind(last).execute(&mut *connection).await?;
        sqlx::query(include_str!("message_addresses_backfill.sql"))
            .bind(after)
            .bind(last)
            .execute(&mut *connection)
            .await?;
        Ok(Some(last.to_string()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("pool");
        crate::db::init(&pool).await.expect("migrations");
        sqlx::query(
            "CREATE TABLE repair_targets (id TEXT PRIMARY KEY, fixed INTEGER NOT NULL DEFAULT 0)",
        )
        .execute(&pool)
        .await
        .expect("table");
        for index in 0..10 {
            sqlx::query("INSERT INTO repair_targets (id) VALUES (?)")
                .bind(format!("row-{index:02}"))
                .execute(&pool)
                .await
                .expect("row");
        }
        pool
    }

    /// Marks rows fixed in id order, `limit` at a time.
    fn fix_rows<'c>(
        connection: &'c mut SqliteConnection,
        cursor: &'c str,
        limit: i64,
    ) -> BoxFuture<'c, Result<Option<String>, sqlx::Error>> {
        Box::pin(async move {
            let ids: Vec<String> = sqlx::query_scalar(
                "SELECT id FROM repair_targets WHERE id > ? ORDER BY id LIMIT ?",
            )
            .bind(cursor)
            .bind(limit)
            .fetch_all(&mut *connection)
            .await?;
            let Some(last) = ids.last().cloned() else {
                return Ok(None);
            };
            sqlx::query("UPDATE repair_targets SET fixed = 1 WHERE id > ? AND id <= ?")
                .bind(cursor)
                .bind(&last)
                .execute(&mut *connection)
                .await?;
            Ok(Some(last))
        })
    }

    const FIX_ROWS: DataRepair = DataRepair {
        name: "test_fix_rows",
        block: fix_rows,
    };

    async fn plant_marker(pool: &SqlitePool) {
        sqlx::query(
            "INSERT OR IGNORE INTO app_settings (key, value, updated_at) VALUES (?, '', '2026-01-01')",
        )
        .bind(FIX_ROWS.marker_key())
        .execute(pool)
        .await
        .expect("marker");
    }

    async fn fixed(pool: &SqlitePool) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM repair_targets WHERE fixed = 1")
            .fetch_one(pool)
            .await
            .expect("count")
    }

    #[tokio::test]
    async fn html_repair_preserves_source_and_resumes_after_each_block() {
        let pool = pool().await;
        sqlx::query("INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, created_at, updated_at) VALUES ('a', 'A', 'a@example.test', 'imap', '{}', '', '', '')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO folders (id, account_id, name, full_path, folder_type, created_at, updated_at) VALUES ('f', 'a', 'INBOX', 'INBOX', 'inbox', '', '')").execute(&pool).await.unwrap();
        for id in ["first", "second"] {
            sqlx::query("INSERT INTO messages (id, account_id, folder_id, from_address, to_addresses, date, body_html, created_at, updated_at) VALUES (?, 'a', 'f', 'a@example.test', '[]', '', '<p onclick=bad()>Hi<script>bad()</script></p>', '', '')")
                .bind(id).execute(&pool).await.unwrap();
        }
        assert_eq!(
            run_repair(&pool, &SANITIZE_MESSAGE_HTML, 1, Some(1), |_| {})
                .await
                .unwrap(),
            RepairRun::Paused { blocks: 1 }
        );
        let rows: Vec<(String, Option<String>)> =
            sqlx::query_as("SELECT body_html, original_body_html FROM messages ORDER BY rowid")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(rows[0].0, "<p>Hi</p>");
        assert!(rows[0].1.as_deref().unwrap().contains("<script>"));
        assert!(rows[1].0.contains("<script>"));
        assert!(is_pending(&pool, &SANITIZE_MESSAGE_HTML).await.unwrap());
        run_repair(&pool, &SANITIZE_MESSAGE_HTML, 1, None, |_| {})
            .await
            .unwrap();
        let unsafe_rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE body_html LIKE '%<script>%' OR body_html LIKE '%onclick%'").fetch_one(&pool).await.unwrap();
        assert_eq!(unsafe_rows, 0);
        assert!(!is_pending(&pool, &SANITIZE_MESSAGE_HTML).await.unwrap());
    }

    #[tokio::test]
    async fn a_repair_resumes_from_its_cursor() {
        let pool = pool().await;
        plant_marker(&pool).await;

        // An interrupted first run: two blocks of four, then the process stops.
        let first = run_repair(&pool, &FIX_ROWS, 4, Some(2), |_| {})
            .await
            .expect("first");
        assert_eq!(first, RepairRun::Paused { blocks: 2 });
        assert_eq!(fixed(&pool).await, 8);
        let cursor: String = sqlx::query_scalar("SELECT value FROM app_settings WHERE key = ?")
            .bind(FIX_ROWS.marker_key())
            .fetch_one(&pool)
            .await
            .expect("cursor");
        assert_eq!(cursor, "row-07");

        // The next start picks up at row-08, not at the beginning.
        let mut seen_blocks = Vec::new();
        let second = run_repair(&pool, &FIX_ROWS, 4, None, |blocks| seen_blocks.push(blocks))
            .await
            .expect("second");
        assert_eq!(second, RepairRun::Completed { blocks: 2 });
        assert_eq!(seen_blocks, [1, 2]);
        assert_eq!(fixed(&pool).await, 10);
        assert!(!is_pending(&pool, &FIX_ROWS).await.expect("pending"));
    }

    #[tokio::test]
    async fn without_a_marker_nothing_runs() {
        let pool = pool().await;

        let run = run_repair(&pool, &FIX_ROWS, 4, None, |_| {})
            .await
            .expect("run");

        assert_eq!(run, RepairRun::NotPending);
        assert_eq!(fixed(&pool).await, 0);
    }

    /// The split must not take schema migrations along with the data passes.
    #[tokio::test]
    async fn schema_migrations_still_run_at_startup() {
        let pool = pool().await;

        let applied: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations")
            .fetch_one(&pool)
            .await
            .expect("migrations table");
        let has_threads: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'threads'",
        )
        .fetch_one(&pool)
        .await
        .expect("schema");

        assert!(applied > 0);
        assert_eq!(has_threads, 1);
    }
}
