//! Coordination guards for mailbox work.
//!
//! The in-process lock serializes one account, the Android SQLite lease spans
//! processes, and the mobile permit caps memory-heavy mailbox work globally.

use std::{collections::HashMap, sync::Arc};

#[cfg(target_os = "android")]
use sqlx::SqlitePool;

/// Serialize sync work per account. IDLE events, provider pollers, manual sync
/// and queued IMAP reconciliation may fire concurrently; two runs over the same
/// account race on `accounts.sync_state`. Unrelated accounts must not block one
/// another, especially while a slow provider is backfilling folders.
pub(crate) fn sync_run_lock(account_id: &str) -> Arc<tokio::sync::Mutex<()>> {
    static LOCKS: std::sync::OnceLock<
        std::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    > = std::sync::OnceLock::new();
    let locks = LOCKS.get_or_init(|| std::sync::Mutex::new(HashMap::new()));
    let mut locks = locks
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    locks
        .entry(account_id.to_string())
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone()
}

/// A renewable SQLite lease serializes Android's UI and WorkManager processes.
/// The ordinary Tokio mutex above remains the cheaper first line of defence
/// within either process.
#[cfg(target_os = "android")]
pub(crate) struct CrossProcessSyncLease {
    pool: SqlitePool,
    account_id: String,
    owner_token: String,
    heartbeat: tokio::task::JoinHandle<()>,
    released: bool,
}

#[cfg(target_os = "android")]
impl CrossProcessSyncLease {
    const LEASE_SECONDS: i64 = 300;
    const HEARTBEAT_SECONDS: u64 = 30;

    pub(crate) async fn acquire(
        pool: &SqlitePool,
        account_id: &str,
    ) -> Result<Option<Self>, String> {
        let owner_token = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now();
        let expires_at = (now + chrono::Duration::seconds(Self::LEASE_SECONDS)).to_rfc3339();
        let claimed = sqlx::query_scalar::<_, String>(
            "INSERT INTO account_sync_leases
                 (account_id, owner_token, lease_expires_at, updated_at)
             VALUES (?, ?, ?, ?)
             ON CONFLICT(account_id) DO UPDATE SET
                 owner_token = excluded.owner_token,
                 lease_expires_at = excluded.lease_expires_at,
                 updated_at = excluded.updated_at
             WHERE julianday(account_sync_leases.lease_expires_at) <= julianday('now')
             RETURNING owner_token",
        )
        .bind(account_id)
        .bind(&owner_token)
        .bind(&expires_at)
        .bind(now.to_rfc3339())
        .fetch_optional(pool)
        .await
        .map_err(|error| format!("acquire cross-process mailbox lease: {error}"))?;
        if claimed.as_deref() != Some(owner_token.as_str()) {
            return Ok(None);
        }

        let heartbeat_pool = pool.clone();
        let heartbeat_account_id = account_id.to_string();
        let heartbeat_owner_token = owner_token.clone();
        let heartbeat = tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(std::time::Duration::from_secs(Self::HEARTBEAT_SECONDS));
            interval.tick().await;
            loop {
                interval.tick().await;
                let now = chrono::Utc::now();
                let expires_at =
                    (now + chrono::Duration::seconds(Self::LEASE_SECONDS)).to_rfc3339();
                match sqlx::query(
                    "UPDATE account_sync_leases
                     SET lease_expires_at = ?, updated_at = ?
                     WHERE account_id = ? AND owner_token = ?",
                )
                .bind(expires_at)
                .bind(now.to_rfc3339())
                .bind(&heartbeat_account_id)
                .bind(&heartbeat_owner_token)
                .execute(&heartbeat_pool)
                .await
                {
                    Ok(result) if result.rows_affected() == 0 => break,
                    Ok(_) => {}
                    Err(error) => tracing::warn!(
                        %error,
                        account_id = %heartbeat_account_id,
                        "cross-process mailbox lease renewal failed"
                    ),
                }
            }
        });
        Ok(Some(Self {
            pool: pool.clone(),
            account_id: account_id.to_string(),
            owner_token,
            heartbeat,
            released: false,
        }))
    }

    pub(crate) async fn release(mut self) {
        self.heartbeat.abort();
        Self::delete(&self.pool, &self.account_id, &self.owner_token).await;
        self.released = true;
    }

    async fn delete(pool: &SqlitePool, account_id: &str, owner_token: &str) {
        if let Err(error) = sqlx::query(
            "DELETE FROM account_sync_leases
             WHERE account_id = ? AND owner_token = ?",
        )
        .bind(account_id)
        .bind(owner_token)
        .execute(pool)
        .await
        {
            tracing::warn!(
                %error,
                %account_id,
                "cross-process mailbox lease release failed"
            );
        }
    }
}

/// A run cancelled by its deadline never reaches `release`. Without deleting
/// the row here, the other process would wait out the full lease.
#[cfg(target_os = "android")]
impl Drop for CrossProcessSyncLease {
    fn drop(&mut self) {
        self.heartbeat.abort();
        if self.released {
            return;
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let pool = self.pool.clone();
        let account_id = std::mem::take(&mut self.account_id);
        let owner_token = std::mem::take(&mut self.owner_token);
        runtime.spawn(async move {
            Self::delete(&pool, &account_id, &owner_token).await;
        });
    }
}

/// Android keeps the UI, background service, provider clients and SQLite in
/// one memory-constrained process. IDLE/poll supervisors for different
/// accounts may otherwise run large mailbox snapshots beside startup repairs,
/// which can push a large local mailbox past the process memory limit. Desktop
/// keeps the existing per-account concurrency; mobile admits one mailbox-heavy
/// operation at a time.
#[cfg(mobile)]
pub(crate) async fn acquire_mobile_mailbox_work_permit() -> tokio::sync::OwnedSemaphorePermit {
    static CAPACITY: std::sync::OnceLock<Arc<tokio::sync::Semaphore>> = std::sync::OnceLock::new();
    CAPACITY
        .get_or_init(|| Arc::new(tokio::sync::Semaphore::new(1)))
        .clone()
        .acquire_owned()
        .await
        .expect("mobile mailbox work semaphore is never closed")
}
