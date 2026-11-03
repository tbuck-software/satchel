//! Storage accounting and bounded eviction of remotely recoverable content.

use sqlx::SqlitePool;

pub const MOBILE_TARGET_BYTES: u64 = 500_000_000;
pub const EVICTION_BATCH: i64 = 64;
const NEXT_BUDGET_PASS: &str = "maintenance.storage_budget_next_at";

#[derive(Debug, Clone, serde::Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "core/db/storage/"))]
pub struct StorageUsage {
    /// SQLite's logical file size, including pages currently in the WAL.
    pub database_bytes: u64,
    pub reusable_bytes: u64,
    pub wal_bytes: u64,
    /// A cache target, never permission to delete headers or unique local data.
    pub target_bytes: Option<u64>,
}

impl StorageUsage {
    pub fn live_bytes(&self) -> u64 {
        self.database_bytes.saturating_sub(self.reusable_bytes)
    }
}

pub async fn usage(pool: &SqlitePool) -> Result<StorageUsage, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let (pages, free, size): (i64, i64, i64) = sqlx::query_as(
        "SELECT page_count, freelist_count, page_size
         FROM pragma_page_count(), pragma_freelist_count(), pragma_page_size()",
    )
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    let wal_bytes = if let Some(path) = super::main_database_file(pool).await? {
        let mut wal = path.into_os_string();
        wal.push("-wal");
        match tokio::fs::metadata(std::path::PathBuf::from(wal)).await {
            Ok(metadata) => metadata.len(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
            Err(error) => return Err(error.into()),
        }
    } else {
        0
    };
    Ok(StorageUsage {
        database_bytes: (pages.max(0) as u64).saturating_mul(size.max(0) as u64),
        reusable_bytes: (free.max(0) as u64).saturating_mul(size.max(0) as u64),
        wal_bytes,
        target_bytes: cfg!(any(target_os = "android", target_os = "ios"))
            .then_some(MOBILE_TARGET_BYTES),
    })
}

/// The UI and background sync share the same cooldown when no content can be
/// released. A header-only profile must not be rescanned every thirty seconds.
pub async fn maintain_mobile_budget(pool: &SqlitePool) -> Result<u64, sqlx::Error> {
    maintain_budget(pool, MOBILE_TARGET_BYTES).await
}

async fn maintain_budget(pool: &SqlitePool, target: u64) -> Result<u64, sqlx::Error> {
    let next: Option<String> = sqlx::query_scalar("SELECT value FROM app_settings WHERE key = ?")
        .bind(NEXT_BUDGET_PASS)
        .fetch_optional(pool)
        .await?;
    let now = chrono::Utc::now();
    if usage(pool).await?.live_bytes() <= target {
        // An empty profile must not defer its first eviction for six hours
        // while the initial sync grows the cache. Cool down only after an
        // over-budget scan finds no recoverable content.
        if next.is_some() {
            sqlx::query("DELETE FROM app_settings WHERE key = ?")
                .bind(NEXT_BUDGET_PASS)
                .execute(pool)
                .await?;
        }
        return Ok(0);
    }
    if next
        .and_then(|raw| chrono::DateTime::parse_from_rfc3339(&raw).ok())
        .is_some_and(|next| next > now)
    {
        return Ok(0);
    }
    let removed = evict_to_target(pool, target, EVICTION_BATCH).await?;
    if removed == 0 {
        sqlx::query("INSERT INTO app_settings (key, value, updated_at) VALUES (?, ?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at")
            .bind(NEXT_BUDGET_PASS).bind((now + chrono::Duration::hours(6)).to_rfc3339())
            .bind(now.to_rfc3339()).execute(pool).await?;
    }
    Ok(removed)
}

/// One background pass. Freed SQLite pages are reused without a full VACUUM.
/// Headers stay indefinitely, so a profile made of headers or unique local
/// messages can exceed the target even after every recoverable body is gone.
pub async fn evict_to_target(
    pool: &SqlitePool,
    target: u64,
    limit: i64,
) -> Result<u64, sqlx::Error> {
    if usage(pool).await?.live_bytes() <= target {
        return Ok(0);
    }
    use super::content_eviction::{release, FROM, HAS_PAYLOAD, RECOVERABLE};
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let ids: Vec<String> = sqlx::query_scalar(&format!(
        "SELECT m.id FROM {FROM} WHERE {RECOVERABLE} AND {HAS_PAYLOAD} ORDER BY m.date, m.id LIMIT ?"
    ))
    .bind(limit.clamp(1, EVICTION_BATCH))
    .fetch_all(&mut *tx)
    .await?;
    let released = release(&mut tx, &ids).await?;
    tx.commit().await?;
    Ok(released)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn pool() -> SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        super::super::init(&pool).await.unwrap();
        for provider in ["imap", "gmail", "outlook", "pop3", "local"] {
            sqlx::query("INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, created_at, updated_at) VALUES (?, '', 'a@example.test', ?, '{}', '', '', '')")
                .bind(provider).bind(provider).execute(&pool).await.unwrap();
            sqlx::query("INSERT INTO folders (id, account_id, name, full_path, remote_id, folder_type, uidvalidity, created_at, updated_at) VALUES (?, ?, 'INBOX', 'INBOX', 'INBOX', 'inbox', 7, '', '')")
                .bind(provider).bind(provider).execute(&pool).await.unwrap();
        }
        for (id, provider, remote, validity, draft, date) in [
            ("oldest", "imap", "1", Some(7), false, "2020"),
            ("newer", "gmail", "2", None, false, "2021"),
            ("outlook", "outlook", "3", None, false, "2022"),
            ("imported", "gmail", "import-1", None, false, "2010"),
            ("draft", "gmail", "5", None, true, "2010"),
            ("pop3", "pop3", "6", None, false, "2010"),
            ("local", "local", "7", None, false, "2010"),
            ("stale", "imap", "8", Some(6), false, "2010"),
            ("unverified", "imap", "9", None, false, "2010"),
        ] {
            sqlx::query("INSERT INTO messages (id, account_id, folder_id, remote_id, remote_uidvalidity, is_draft, date, from_address, to_addresses, subject, snippet, body_text, original_source, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, 'from@example.test', '[]', 'Subject', 'Preview', 'Searchable', zeroblob(16384), '', '')")
                .bind(id).bind(provider).bind(provider).bind(remote).bind(validity).bind(draft).bind(date).execute(&pool).await.unwrap();
        }
        pool
    }

    #[tokio::test]
    async fn initial_sync_growth_does_not_inherit_a_cooldown_from_an_empty_cache() {
        let pool = pool().await;
        let target = usage(&pool).await.unwrap().live_bytes() + 1_000_000;
        assert_eq!(maintain_budget(&pool, target).await.unwrap(), 0);
        sqlx::query(
            "UPDATE messages SET original_source = zeroblob(2_000_000) WHERE id = 'oldest'",
        )
        .execute(&pool)
        .await
        .unwrap();
        assert!(
            maintain_budget(&pool, target).await.unwrap() > 0,
            "cache growth during initial sync must trigger the next budget pass"
        );
    }

    #[tokio::test]
    async fn reports_live_wal_bytes_and_reusable_pages_separately() {
        let directory = tempfile::tempdir().unwrap();
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                sqlx::sqlite::SqliteConnectOptions::new()
                    .filename(directory.path().join("mail.db"))
                    .create_if_missing(true)
                    .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal),
            )
            .await
            .unwrap();
        sqlx::query("PRAGMA wal_autocheckpoint = 0")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("CREATE TABLE ballast (payload BLOB)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO ballast VALUES (zeroblob(131072))")
            .execute(&pool)
            .await
            .unwrap();
        let before = usage(&pool).await.unwrap();
        assert!(before.wal_bytes > 0);
        assert!(before.live_bytes() >= 131072);
        sqlx::query("DELETE FROM ballast")
            .execute(&pool)
            .await
            .unwrap();
        let after = usage(&pool).await.unwrap();
        assert_eq!(after.database_bytes, before.database_bytes);
        assert!(after.reusable_bytes > before.reusable_bytes);
        sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(usage(&pool).await.unwrap().wal_bytes, 0);
    }

    #[tokio::test]
    async fn eviction_is_bounded_oldest_first_and_preserves_unique_content() {
        let pool = pool().await;
        sqlx::query("INSERT INTO attachments (id, message_id, filename, content_type, size, cached_path, content_base64, created_at) VALUES ('a', 'oldest', 'file', 'text/plain', 3, '/cache/file', 'YWJj', '')")
            .execute(&pool).await.unwrap();
        assert_eq!(evict_to_target(&pool, u64::MAX, 1).await.unwrap(), 0);
        assert_eq!(evict_to_target(&pool, 0, 1).await.unwrap(), 1);
        let oldest: (String, String, Option<String>, Option<Vec<u8>>) = sqlx::query_as(
            "SELECT subject, snippet, body_text, original_source FROM messages WHERE id = 'oldest'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(oldest, ("Subject".into(), "Preview".into(), None, None));
        let file: (Option<String>, Option<String>) =
            sqlx::query_as("SELECT cached_path, content_base64 FROM attachments WHERE id = 'a'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(file, (None, None));
        assert_eq!(evict_to_target(&pool, 0, 64).await.unwrap(), 2);
        assert_eq!(evict_to_target(&pool, 0, 64).await.unwrap(), 0);
        // A later attachment-only download must not escape the budget just
        // because the message's body was already released by an earlier pass.
        sqlx::query("UPDATE attachments SET content_base64 = 'YWJj' WHERE id = 'a'")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(evict_to_target(&pool, 0, 64).await.unwrap(), 1);
        assert_eq!(evict_to_target(&pool, 0, 64).await.unwrap(), 0);
        let retained: Vec<String> = sqlx::query_scalar(
            "SELECT id FROM messages WHERE original_source IS NOT NULL ORDER BY id",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(
            retained,
            ["draft", "imported", "local", "pop3", "stale", "unverified"]
        );
        let headers: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(headers, 9);
        let hits: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM messages_fts WHERE messages_fts MATCH 'Searchable'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(hits, 6, "eviction must update the search projection too");
        let usage = usage(&pool).await.unwrap();
        assert!(usage.database_bytes > 0);
        assert!(usage.reusable_bytes > 0);
        assert_eq!(usage.wal_bytes, 0);
    }
}
