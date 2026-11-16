//! The last mailbox quota each account's server reported (`account_quota`).
//!
//! The quota is read on the folder-listing session of a full sync, at most
//! once per [`QUOTA_REFRESH_INTERVAL`]. A quota moves slowly; asking on every
//! five-minute sync would only add round trips.

use chrono::{DateTime, Utc};
use sqlx::{FromRow, SqlitePool};

use crate::providers::quota::{MailboxQuota, QuotaProbe, QuotaUsage};

/// How long a quota reading — or a failed or empty attempt — stands before
/// the next full sync asks again.
pub const QUOTA_REFRESH_INTERVAL: chrono::Duration = chrono::Duration::hours(6);

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct AccountQuotaRecord {
    pub account_id: String,
    pub storage_used_bytes: Option<i64>,
    pub storage_limit_bytes: Option<i64>,
    pub messages_used: Option<i64>,
    pub messages_limit: Option<i64>,
    /// RFC 3339; when the numbers were last read successfully.
    pub fetched_at: Option<String>,
    /// RFC 3339; the last attempt, successful or not.
    pub checked_at: String,
}

/// Whether a full sync should read the quota again. A missing or unreadable
/// `checked_at` is due; one in the future (a clock set back) is due as well,
/// or the reading would freeze until the clock caught up.
pub fn refresh_due(checked_at: Option<&str>, now: DateTime<Utc>) -> bool {
    let Some(checked_at) = checked_at.and_then(|raw| DateTime::parse_from_rfc3339(raw).ok()) else {
        return true;
    };
    let elapsed = now.signed_duration_since(checked_at.with_timezone(&Utc));
    elapsed < chrono::Duration::zero() || elapsed >= QUOTA_REFRESH_INTERVAL
}

fn as_db(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn split(usage: Option<QuotaUsage>) -> (Option<i64>, Option<i64>) {
    match usage {
        Some(usage) => (Some(as_db(usage.used)), Some(as_db(usage.limit))),
        None => (None, None),
    }
}

pub struct AccountQuotaRepository<'a> {
    pool: &'a SqlitePool,
}

impl<'a> AccountQuotaRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn checked_at(&self, account_id: &str) -> Result<Option<String>, sqlx::Error> {
        sqlx::query_scalar("SELECT checked_at FROM account_quota WHERE account_id = ?")
            .bind(account_id)
            .fetch_optional(self.pool)
            .await
    }

    /// Whether the account's next full sync should read the quota.
    pub async fn due(&self, account_id: &str, now: DateTime<Utc>) -> Result<bool, sqlx::Error> {
        Ok(refresh_due(
            self.checked_at(account_id).await?.as_deref(),
            now,
        ))
    }

    /// Stores what a probe found. A reading replaces the numbers; a server
    /// without quota clears them (it may have dropped the extension); a
    /// failed read only moves `checked_at`, so the last known numbers stay.
    pub async fn record(
        &self,
        account_id: &str,
        probe: &QuotaProbe,
        now: DateTime<Utc>,
    ) -> Result<(), sqlx::Error> {
        let now = now.to_rfc3339();
        let reading = match probe {
            QuotaProbe::Read(reading) => Some(*reading),
            QuotaProbe::Unsupported => Some(MailboxQuota::default()),
            QuotaProbe::Failed(_) => None,
        };
        let Some(reading) = reading else {
            sqlx::query(
                "INSERT INTO account_quota (account_id, checked_at) VALUES (?, ?)
                 ON CONFLICT(account_id) DO UPDATE SET checked_at = excluded.checked_at",
            )
            .bind(account_id)
            .bind(&now)
            .execute(self.pool)
            .await?;
            return Ok(());
        };
        let (storage_used, storage_limit) = split(reading.storage);
        let (messages_used, messages_limit) = split(reading.messages);
        let fetched_at = matches!(probe, QuotaProbe::Read(_)).then_some(now.as_str());
        sqlx::query(
            "INSERT INTO account_quota
                 (account_id, storage_used_bytes, storage_limit_bytes, messages_used,
                  messages_limit, fetched_at, checked_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(account_id) DO UPDATE SET
                 storage_used_bytes = excluded.storage_used_bytes,
                 storage_limit_bytes = excluded.storage_limit_bytes,
                 messages_used = excluded.messages_used,
                 messages_limit = excluded.messages_limit,
                 fetched_at = excluded.fetched_at,
                 checked_at = excluded.checked_at",
        )
        .bind(account_id)
        .bind(storage_used)
        .bind(storage_limit)
        .bind(messages_used)
        .bind(messages_limit)
        .bind(fetched_at)
        .bind(&now)
        .execute(self.pool)
        .await?;
        Ok(())
    }

    /// Quotas of active accounts that have at least one limit to show.
    pub async fn list_known(&self) -> Result<Vec<AccountQuotaRecord>, sqlx::Error> {
        sqlx::query_as::<_, AccountQuotaRecord>(
            "SELECT q.account_id, q.storage_used_bytes, q.storage_limit_bytes,
                    q.messages_used, q.messages_limit, q.fetched_at, q.checked_at
             FROM account_quota q
             JOIN accounts a ON a.id = q.account_id
             WHERE a.is_active = 1
               AND (q.storage_limit_bytes IS NOT NULL OR q.messages_limit IS NOT NULL)
             ORDER BY q.account_id",
        )
        .fetch_all(self.pool)
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(hour: u32, minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 5, hour, minute, 0).unwrap()
    }

    #[test]
    fn a_reading_stands_for_six_hours() {
        let checked = at(8, 0).to_rfc3339();
        assert!(!refresh_due(Some(&checked), at(8, 5)));
        assert!(!refresh_due(Some(&checked), at(13, 59)));
        assert!(refresh_due(Some(&checked), at(14, 0)));
    }

    #[test]
    fn never_checked_unreadable_or_future_is_due() {
        assert!(refresh_due(None, at(8, 0)));
        assert!(refresh_due(Some("yesterday"), at(8, 0)));
        assert!(refresh_due(Some(&at(9, 0).to_rfc3339()), at(8, 0)));
    }

    async fn pool() -> SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::db::init(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref,
                                   created_at, updated_at)
             VALUES ('acc', 'Work', 'me@example.test', 'imap', '{}', '', '', '')",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    fn read(used: u64, limit: u64) -> QuotaProbe {
        QuotaProbe::Read(MailboxQuota {
            storage: Some(QuotaUsage { used, limit }),
            messages: None,
        })
    }

    #[tokio::test]
    async fn a_failed_read_keeps_the_last_numbers_and_waits_like_a_good_one() {
        let pool = pool().await;
        let repo = AccountQuotaRepository::new(&pool);
        assert!(repo.due("acc", at(8, 0)).await.unwrap());

        repo.record("acc", &read(90, 100), at(8, 0)).await.unwrap();
        assert!(!repo.due("acc", at(9, 0)).await.unwrap());

        repo.record("acc", &QuotaProbe::Failed("NO".into()), at(14, 0))
            .await
            .unwrap();
        let rows = repo.list_known().await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].storage_used_bytes, Some(90));
        assert_eq!(rows[0].storage_limit_bytes, Some(100));
        assert_eq!(
            rows[0].fetched_at.as_deref(),
            Some(at(8, 0).to_rfc3339().as_str())
        );
        assert_eq!(rows[0].checked_at, at(14, 0).to_rfc3339());
        assert!(!repo.due("acc", at(15, 0)).await.unwrap());
    }

    #[tokio::test]
    async fn a_server_that_dropped_quota_clears_the_numbers() {
        let pool = pool().await;
        let repo = AccountQuotaRepository::new(&pool);
        repo.record("acc", &read(90, 100), at(8, 0)).await.unwrap();
        repo.record("acc", &QuotaProbe::Unsupported, at(14, 0))
            .await
            .unwrap();
        assert!(repo.list_known().await.unwrap().is_empty());
        // Still paced: no limit to show is an answer, too.
        assert!(!repo.due("acc", at(15, 0)).await.unwrap());
    }

    #[tokio::test]
    async fn a_failure_before_any_reading_shows_nothing() {
        let pool = pool().await;
        let repo = AccountQuotaRepository::new(&pool);
        repo.record("acc", &QuotaProbe::Failed("timeout".into()), at(8, 0))
            .await
            .unwrap();
        assert!(repo.list_known().await.unwrap().is_empty());
        assert!(!repo.due("acc", at(9, 0)).await.unwrap());
    }

    #[tokio::test]
    async fn the_quota_goes_with_its_account() {
        let pool = pool().await;
        let repo = AccountQuotaRepository::new(&pool);
        repo.record("acc", &read(1, 2), at(8, 0)).await.unwrap();
        sqlx::query("DELETE FROM accounts WHERE id = 'acc'")
            .execute(&pool)
            .await
            .unwrap();
        let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM account_quota")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(left, 0);
    }
}
