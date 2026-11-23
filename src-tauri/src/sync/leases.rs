//! Coordination guards for mailbox work.
//!
//! The in-process lock serializes one account, the Android SQLite lease spans
//! processes, and the mobile permit caps memory-heavy mailbox work globally.

use std::{collections::HashMap, sync::Arc};

use sqlx::{Sqlite, SqlitePool, Transaction};

#[cfg(any(test, target_os = "android"))]
use tokio::sync::watch;

#[cfg(any(test, target_os = "android"))]
const LEASE_LOST: &str = "cross-process mailbox lease lost";

#[cfg(any(test, target_os = "android"))]
tokio::task_local! {
    // Follows only the account run, including its nested sync writes.
    static LEASE_OWNER: String;
}

/// Check ownership after taking the SQLite write lock. A successor cannot
/// claim the lease between this check and the transaction's commit.
pub(crate) async fn begin_sync_write(
    pool: &SqlitePool,
    _account_id: &str,
) -> Result<Transaction<'static, Sqlite>, String> {
    let tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|error| error.to_string())?;
    #[cfg(any(test, target_os = "android"))]
    if let Ok(owner_token) = LEASE_OWNER.try_with(Clone::clone) {
        let mut tx = tx;
        let held: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM account_sync_leases
             WHERE account_id = ? AND owner_token = ?
               AND julianday(lease_expires_at) > julianday('now'))",
        )
        .bind(_account_id)
        .bind(owner_token)
        .fetch_one(&mut *tx)
        .await
        .map_err(|error| error.to_string())?;
        if !held {
            tx.rollback().await.map_err(|error| error.to_string())?;
            return Err(LEASE_LOST.into());
        }
        return Ok(tx);
    }
    Ok(tx)
}

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
#[cfg(any(test, target_os = "android"))]
pub(crate) struct CrossProcessSyncLease {
    pool: SqlitePool,
    account_id: String,
    owner_token: String,
    heartbeat: tokio::task::JoinHandle<()>,
    lost: watch::Receiver<bool>,
    released: bool,
}

#[cfg(any(test, target_os = "android"))]
impl CrossProcessSyncLease {
    const LEASE_DURATION: &str = "+300 seconds";
    const HEARTBEAT_SECONDS: u64 = 30;

    pub(crate) async fn acquire(
        pool: &SqlitePool,
        account_id: &str,
    ) -> Result<Option<Self>, String> {
        Self::acquire_with_heartbeat(
            pool,
            account_id,
            std::time::Duration::from_secs(Self::HEARTBEAT_SECONDS),
        )
        .await
    }

    async fn acquire_with_heartbeat(
        pool: &SqlitePool,
        account_id: &str,
        heartbeat_period: std::time::Duration,
    ) -> Result<Option<Self>, String> {
        let owner_token = uuid::Uuid::new_v4().to_string();
        let claimed = sqlx::query_scalar::<_, String>(
            "INSERT INTO account_sync_leases
                 (account_id, owner_token, lease_expires_at, updated_at)
             VALUES (?, ?, strftime('%Y-%m-%dT%H:%M:%fZ', 'now', ?),
                     strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
             ON CONFLICT(account_id) DO UPDATE SET
                 owner_token = excluded.owner_token,
                 lease_expires_at = excluded.lease_expires_at,
                 updated_at = excluded.updated_at
             WHERE julianday(account_sync_leases.lease_expires_at) <= julianday('now')
             RETURNING owner_token",
        )
        .bind(account_id)
        .bind(&owner_token)
        .bind(Self::LEASE_DURATION)
        .fetch_optional(pool)
        .await
        .map_err(|error| format!("acquire cross-process mailbox lease: {error}"))?;
        if claimed.as_deref() != Some(owner_token.as_str()) {
            return Ok(None);
        }

        let heartbeat_pool = pool.clone();
        let heartbeat_account_id = account_id.to_string();
        let heartbeat_owner_token = owner_token.clone();
        let (signal_loss, lost) = watch::channel(false);
        let heartbeat = tokio::spawn(async move {
            let mut interval = tokio::time::interval(heartbeat_period);
            interval.tick().await;
            loop {
                interval.tick().await;
                match sqlx::query(
                    "UPDATE account_sync_leases
                     SET lease_expires_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now', ?),
                         updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                     WHERE account_id = ? AND owner_token = ?
                       AND julianday(lease_expires_at) > julianday('now')",
                )
                .bind(Self::LEASE_DURATION)
                .bind(&heartbeat_account_id)
                .bind(&heartbeat_owner_token)
                .execute(&heartbeat_pool)
                .await
                {
                    Ok(result) if result.rows_affected() == 0 => break,
                    Ok(_) => {}
                    Err(error) => {
                        // We can no longer confirm ownership. Stop this run
                        // and let the next attempt claim or renew its own lease.
                        tracing::warn!(
                            %error,
                            account_id = %heartbeat_account_id,
                            "cross-process mailbox lease renewal failed"
                        );
                        break;
                    }
                }
            }
            signal_loss.send_replace(true);
        });
        Ok(Some(Self {
            pool: pool.clone(),
            account_id: account_id.to_string(),
            owner_token,
            heartbeat,
            lost,
            released: false,
        }))
    }

    /// Cancel the provider and database futures when the heartbeat loses
    /// ownership. Check the signal first if both branches are ready.
    pub(crate) async fn run<T>(
        &self,
        work: impl std::future::Future<Output = Result<T, String>>,
    ) -> Result<T, String> {
        let mut lost = self.lost.clone();
        tokio::select! {
            biased;
            _ = lost.wait_for(|lost| *lost) => Err(LEASE_LOST.into()),
            result = LEASE_OWNER.scope(self.owner_token.clone(), work) => result,
        }
    }

    pub(crate) fn was_lost(error: &str) -> bool {
        error == LEASE_LOST
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
#[cfg(any(test, target_os = "android"))]
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn heartbeat_cancels_an_active_run_after_a_takeover() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        crate::db::init(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO accounts
             (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
             VALUES ('test', 'Test', 'test@example.test', 'imap', '{}', '', '', '')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let lease =
            CrossProcessSyncLease::acquire_with_heartbeat(&pool, "test", Duration::from_millis(10))
                .await
                .unwrap()
                .unwrap();

        let result = tokio::time::timeout(
            Duration::from_secs(5),
            lease.run(async {
                sqlx::query("UPDATE account_sync_leases SET owner_token = 'successor'")
                    .execute(&pool)
                    .await
                    .unwrap();
                std::future::pending::<Result<(), String>>().await
            }),
        )
        .await
        .expect("heartbeat should wake the cancelled run");
        assert_eq!(result, Err(LEASE_LOST.into()));

        lease.release().await;
        let owner: String = sqlx::query_scalar("SELECT owner_token FROM account_sync_leases")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(owner, "successor");
    }
}
