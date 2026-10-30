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
use std::time::{Duration, Instant};

/// Prefix of the marker keys in `app_settings`.
pub const MARKER_PREFIX: &str = "data_repair.";

/// Rows a block handles by default. Small enough that a block is a short
/// write lock, large enough that a big mailbox is not thousands of commits.
pub const DEFAULT_BLOCK_SIZE: i64 = 1_000;

/// How long one block should hold the write lock. A block that runs over
/// shrinks the next one; quick blocks grow back toward the caller's size.
pub const BLOCK_BUDGET: Duration = Duration::from_millis(100);

/// Pause between blocks. SQLite's busy handler backs a waiting writer off to
/// polls 100 ms apart, so a shorter gap is one that writer sleeps through and
/// the next block takes the lock again.
pub const BLOCK_PAUSE: Duration = Duration::from_millis(150);

/// Work a repair did before its block took the write lock.
pub type Prepared = Box<dyn std::any::Any + Send>;

/// Reads and computes a block's work without holding the write lock.
///
/// Called with the stored cursor and the block size. The block receives the
/// result only if the cursor is still the same once the lock is held, and must
/// treat it as a snapshot: rows can change between the two.
pub type RepairPrepare =
    for<'c> fn(&'c SqlitePool, &'c str, i64) -> BoxFuture<'c, Result<Prepared, sqlx::Error>>;

/// What a repair does with one block.
///
/// Called with the cursor stored last (`""` on the first block), the block
/// size and what [`RepairPrepare`] computed for that cursor, if anything,
/// inside the transaction that will also store the cursor it returns.
/// `Ok(None)` means nothing is left.
pub type RepairBlock = for<'c> fn(
    &'c mut SqliteConnection,
    &'c str,
    i64,
    Option<Prepared>,
) -> BoxFuture<'c, Result<Option<String>, sqlx::Error>>;

#[derive(Clone, Copy)]
pub struct DataRepair {
    /// The `<name>` in the marker key.
    pub name: &'static str,
    pub block: RepairBlock,
    /// CPU-heavy work (sanitizing, MIME parsing) belongs here rather than in
    /// `block`, so it does not run while every other writer waits.
    pub prepare: Option<RepairPrepare>,
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

/// The size of the next block, given how long the last one held the lock.
fn next_block_size(current: i64, max: i64, held: Duration) -> i64 {
    if held > BLOCK_BUDGET {
        let scaled = current.max(0) as u128 * BLOCK_BUDGET.as_nanos() / held.as_nanos();
        i64::try_from(scaled).unwrap_or(max).clamp(1, max)
    } else if held < BLOCK_BUDGET / 2 {
        current.saturating_mul(2).min(max)
    } else {
        current
    }
}

/// Run a repair's blocks until it is done or `max_blocks` have run.
///
/// Every block is its own `BEGIN IMMEDIATE` transaction: the cursor it reads,
/// the writes it makes and the cursor it stores commit together, so there is
/// no point at which a crash leaves the two out of step. `block_size` is the
/// upper bound; blocks that hold the lock longer than [`BLOCK_BUDGET`] get
/// smaller, and [`BLOCK_PAUSE`] separates them so interactive writes get in.
pub async fn run_repair(
    pool: &SqlitePool,
    repair: &DataRepair,
    block_size: i64,
    max_blocks: Option<u64>,
    mut on_block: impl FnMut(u64),
) -> Result<RepairRun, sqlx::Error> {
    let key = repair.marker_key();
    let max_size = block_size.max(1);
    let mut size = max_size;
    let mut blocks = 0_u64;
    let finished = |blocks| {
        if blocks == 0 {
            RepairRun::NotPending
        } else {
            RepairRun::Completed { blocks }
        }
    };
    loop {
        if max_blocks.is_some_and(|max| blocks >= max) {
            return Ok(RepairRun::Paused { blocks });
        }
        let prepared = match repair.prepare {
            Some(prepare) => {
                let cursor = {
                    let mut connection = pool.acquire().await?;
                    stored_cursor(&mut connection, &key).await?
                };
                let Some(cursor) = cursor else {
                    return Ok(finished(blocks));
                };
                let work = prepare(pool, &cursor, size).await?;
                Some((cursor, work))
            }
            None => None,
        };
        let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
        let locked_at = Instant::now();
        let Some(cursor) = stored_cursor(&mut tx, &key).await? else {
            tx.rollback().await?;
            return Ok(finished(blocks));
        };
        let prepared = prepared
            .filter(|(prepared_for, _)| *prepared_for == cursor)
            .map(|(_, work)| work);
        let next = (repair.block)(&mut tx, &cursor, size, prepared).await?;
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
        size = next_block_size(size, max_size, locked_at.elapsed());
        blocks += 1;
        on_block(blocks);
        if next.is_none() {
            return Ok(RepairRun::Completed { blocks });
        }
        tokio::time::sleep(BLOCK_PAUSE).await;
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

/// Run blocking CPU work off the async runtime.
async fn off_runtime<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, sqlx::Error> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| sqlx::Error::Io(std::io::Error::other(error)))
}

pub const SANITIZE_MESSAGE_HTML: DataRepair = DataRepair {
    name: "sanitize_message_html",
    block: sanitize_message_html_block,
    prepare: Some(sanitize_message_html_prepare),
};

/// `(rowid, stored html, sanitized html)` for one block.
struct SanitizedHtml(Vec<(i64, String, String)>);

async fn sanitize_html_rows<'e>(
    executor: impl sqlx::SqliteExecutor<'e>,
    cursor: &str,
    limit: i64,
) -> Result<SanitizedHtml, sqlx::Error> {
    let after: i64 = cursor.parse().unwrap_or(0);
    let rows: Vec<(i64, String)> = sqlx::query_as(
        "SELECT rowid, body_html FROM messages WHERE rowid > ? AND body_html IS NOT NULL ORDER BY rowid LIMIT ?",
    )
    .bind(after)
    .bind(limit.min(32))
    .fetch_all(executor)
    .await?;
    off_runtime(move || {
        SanitizedHtml(
            rows.into_iter()
                .map(|(rowid, html)| {
                    let sanitized = crate::email::sanitizer::sanitize_stored_email_html(&html);
                    (rowid, html, sanitized)
                })
                .collect(),
        )
    })
    .await
}

fn sanitize_message_html_prepare<'c>(
    pool: &'c SqlitePool,
    cursor: &'c str,
    limit: i64,
) -> BoxFuture<'c, Result<Prepared, sqlx::Error>> {
    Box::pin(
        async move { Ok(Box::new(sanitize_html_rows(pool, cursor, limit).await?) as Prepared) },
    )
}

fn sanitize_message_html_block<'c>(
    connection: &'c mut SqliteConnection,
    cursor: &'c str,
    limit: i64,
    prepared: Option<Prepared>,
) -> BoxFuture<'c, Result<Option<String>, sqlx::Error>> {
    Box::pin(async move {
        let cleaned = match prepared.and_then(|work| work.downcast::<SanitizedHtml>().ok()) {
            Some(work) => *work,
            None => sanitize_html_rows(&mut *connection, cursor, limit).await?,
        };
        let Some(last) = cleaned.0.last().map(|row| row.0) else {
            return Ok(None);
        };
        for (rowid, original, sanitized) in cleaned.0 {
            // A body rewritten since it was read is newer than this repair
            // and was sanitized on its way in.
            sqlx::query("UPDATE messages SET original_body_html = COALESCE(original_body_html, ?), body_html = ? WHERE rowid = ? AND body_html = ?")
                .bind(&original).bind(sanitized).bind(rowid).bind(&original).execute(&mut *connection).await?;
        }
        Ok(Some(last.to_string()))
    })
}

/// Bodies stored by parser versions that kept a whole multipart body as text.
pub const LEGACY_MIME: DataRepair = DataRepair {
    name: "legacy_mime",
    block: legacy_mime_block,
    prepare: Some(legacy_mime_prepare),
};

struct LegacyMimeFix {
    message_id: String,
    thread_id: Option<String>,
    /// The stored `body_text` the fix was parsed from.
    source: String,
    text: Option<String>,
    html: Option<String>,
    original_html: Option<String>,
    snippet: String,
}

struct LegacyMimeWork {
    last: Option<String>,
    fixes: Vec<LegacyMimeFix>,
}

async fn legacy_mime_rows<'e>(
    executor: impl sqlx::SqliteExecutor<'e>,
    cursor: &str,
    limit: i64,
) -> Result<LegacyMimeWork, sqlx::Error> {
    let rows = sqlx::query_as::<_, (String, Option<String>, Option<String>, Option<String>)>(
        "SELECT id, thread_id, subject,
                CASE WHEN body_html IS NULL AND COALESCE(body_text LIKE '--%Content-Type:%', 0)
                     THEN body_text END
         FROM messages WHERE id > ? ORDER BY id LIMIT ?",
    )
    .bind(cursor)
    .bind(limit)
    .fetch_all(executor)
    .await?;
    let last = rows.last().map(|row| row.0.clone());
    let fixes = off_runtime(move || {
        rows.into_iter()
            .filter_map(|(message_id, thread_id, subject, source)| {
                let source = source?;
                let (text, original_html) =
                    crate::providers::imap::parse_mime_body_fragment(&source)?;
                if text.is_none() && original_html.is_none() {
                    return None;
                }
                let html = original_html
                    .as_deref()
                    .map(crate::email::sanitizer::sanitize_email_html);
                let subject = subject.unwrap_or_else(|| "(no subject)".to_string());
                let snippet = crate::providers::imap::build_snippet(
                    &subject,
                    text.as_deref(),
                    html.as_deref(),
                );
                Some(LegacyMimeFix {
                    message_id,
                    thread_id,
                    source,
                    text,
                    html,
                    original_html,
                    snippet,
                })
            })
            .collect()
    })
    .await?;
    Ok(LegacyMimeWork { last, fixes })
}

fn legacy_mime_prepare<'c>(
    pool: &'c SqlitePool,
    cursor: &'c str,
    limit: i64,
) -> BoxFuture<'c, Result<Prepared, sqlx::Error>> {
    Box::pin(async move { Ok(Box::new(legacy_mime_rows(pool, cursor, limit).await?) as Prepared) })
}

fn legacy_mime_block<'c>(
    connection: &'c mut SqliteConnection,
    cursor: &'c str,
    limit: i64,
    prepared: Option<Prepared>,
) -> BoxFuture<'c, Result<Option<String>, sqlx::Error>> {
    Box::pin(async move {
        let work = match prepared.and_then(|work| work.downcast::<LegacyMimeWork>().ok()) {
            Some(work) => *work,
            None => legacy_mime_rows(&mut *connection, cursor, limit).await?,
        };
        let Some(last) = work.last else {
            return Ok(None);
        };
        for fix in work.fixes {
            let now = chrono::Utc::now().to_rfc3339();
            // Only the body the fix was parsed from is replaced. It is no
            // longer the body the strict splitter wrote with a signature
            // bundle, so a verdict shows that bundle's entity instead.
            let affected = sqlx::query(
                "UPDATE messages
                 SET body_text = ?, body_html = ?, original_body_html = ?, snippet = ?, updated_at = ?,
                     trust_metadata = CASE WHEN json_valid(trust_metadata)
                         AND json_extract(trust_metadata, '$.signed_binding')
                         THEN json_set(trust_metadata, '$.signed_binding', json('false'))
                         ELSE trust_metadata END
                 WHERE id = ? AND body_html IS NULL AND body_text = ?",
            )
            .bind(fix.text.as_deref())
            .bind(fix.html.as_deref())
            .bind(fix.original_html.as_deref())
            .bind(&fix.snippet)
            .bind(&now)
            .bind(&fix.message_id)
            .bind(&fix.source)
            .execute(&mut *connection)
            .await?
            .rows_affected();
            let thread_id = fix
                .thread_id
                .as_deref()
                .filter(|value| !value.trim().is_empty());
            if let (true, Some(thread_id)) = (affected > 0, thread_id) {
                sqlx::query(
                    "UPDATE threads
                     SET snippet = ?, updated_at = ?
                     WHERE id = ?
                       AND (snippet IS NULL OR TRIM(snippet) = '' OR snippet LIKE '--%')",
                )
                .bind(&fix.snippet)
                .bind(&now)
                .bind(thread_id)
                .execute(&mut *connection)
                .await?;
            }
        }
        Ok(Some(last))
    })
}

/// Indexes the Cc lists of messages stored before migration 0076.
pub const CC_INDEX: DataRepair = DataRepair {
    name: "cc_index",
    block: index_cc_block,
    prepare: None,
};

fn index_cc_block<'c>(
    connection: &'c mut SqliteConnection,
    cursor: &'c str,
    limit: i64,
    _prepared: Option<Prepared>,
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
    prepare: None,
};

fn thread_folders_block<'c>(
    connection: &'c mut SqliteConnection,
    cursor: &'c str,
    limit: i64,
    _prepared: Option<Prepared>,
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
    prepare: None,
};
fn fts_prefix_block<'c>(
    connection: &'c mut SqliteConnection,
    cursor: &'c str,
    limit: i64,
    _prepared: Option<Prepared>,
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
    prepare: None,
};
fn message_addresses_block<'c>(
    connection: &'c mut SqliteConnection,
    cursor: &'c str,
    limit: i64,
    _prepared: Option<Prepared>,
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
        _prepared: Option<Prepared>,
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
        prepare: None,
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

    #[test]
    fn blocks_over_budget_shrink_and_quick_blocks_grow_back() {
        assert_eq!(
            next_block_size(1_000, 1_000, Duration::from_millis(250)),
            400
        );
        assert_eq!(next_block_size(1, 1_000, Duration::from_secs(5)), 1);
        assert_eq!(next_block_size(400, 1_000, Duration::from_millis(10)), 800);
        assert_eq!(
            next_block_size(800, 1_000, Duration::from_millis(10)),
            1_000
        );
        assert_eq!(next_block_size(600, 1_000, Duration::from_millis(70)), 600);
    }

    #[tokio::test]
    async fn html_sanitized_before_the_lock_does_not_overwrite_a_newer_body() {
        let pool = pool().await;
        sqlx::query("INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, created_at, updated_at) VALUES ('a', 'A', 'a@example.test', 'imap', '{}', '', '', '')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO folders (id, account_id, name, full_path, folder_type, created_at, updated_at) VALUES ('f', 'a', 'INBOX', 'INBOX', 'inbox', '', '')").execute(&pool).await.unwrap();
        for id in ["kept", "rewritten"] {
            sqlx::query("INSERT INTO messages (id, account_id, folder_id, from_address, to_addresses, date, body_html, created_at, updated_at) VALUES (?, 'a', 'f', 'a@example.test', '[]', '', '<p>Hi<script>bad()</script></p>', '', '')")
                .bind(id).execute(&pool).await.unwrap();
        }
        let prepared = sanitize_message_html_prepare(&pool, "", 10).await.unwrap();
        // Ingest rewrites one body between the read and the write lock.
        sqlx::query("UPDATE messages SET body_html = '<p>Newer</p>' WHERE id = 'rewritten'")
            .execute(&pool)
            .await
            .unwrap();
        let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
        let next = sanitize_message_html_block(&mut tx, "", 10, Some(prepared))
            .await
            .unwrap();
        tx.commit().await.unwrap();
        assert_eq!(next.as_deref(), Some("2"));
        let rows: Vec<(String, Option<String>)> =
            sqlx::query_as("SELECT body_html, original_body_html FROM messages ORDER BY rowid")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(rows[0].0, "<p>Hi</p>");
        assert_eq!(rows[1], ("<p>Newer</p>".to_string(), None));
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
