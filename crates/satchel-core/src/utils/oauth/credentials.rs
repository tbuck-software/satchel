//! The OAuth credentials one provider instance works from.
//!
//! A mailbox run or a send can outlast the access token it started with.
//! Bound to its stored account, an instance renews an expiring token through
//! [`fresh_account_config_for_provider`], which persists a rotation and yields
//! to a reauthorization. A detached snapshot only serves the token it holds:
//! nothing could save what refreshing it would rotate.

use std::sync::Arc;

use sqlx::SqlitePool;
use tokio::sync::Mutex;

use super::{
    access_token_from_account_config, fresh_account_config_for_provider,
    oauth_config_has_fresh_access_token, OAuthProvider,
};
use crate::accounts::AccountConfig;
use crate::sync::errors::{SyncError, SyncResult};

#[derive(Clone)]
pub struct OAuthCredentials(Arc<Inner>);

struct Inner {
    current: Mutex<Current>,
    account: Option<(SqlitePool, String)>,
}

struct Current {
    config: AccountConfig,
    /// The first permanent renewal failure. Asking again with the same
    /// rejected refresh token only repeats it, so the run keeps this answer.
    rejected: Option<SyncError>,
}

impl Current {
    fn new(config: &AccountConfig) -> Mutex<Self> {
        Mutex::new(Self {
            config: config.clone(),
            rejected: None,
        })
    }
}

impl OAuthCredentials {
    /// Serve `config` until its token expires, then ask for a reload.
    pub fn detached(config: &AccountConfig) -> Self {
        Self(Arc::new(Inner {
            current: Current::new(config),
            account: None,
        }))
    }

    /// Start from `config` and renew from the stored account once it expires.
    pub fn stored(pool: &SqlitePool, account_id: &str, config: &AccountConfig) -> Self {
        Self(Arc::new(Inner {
            current: Current::new(config),
            account: Some((pool.clone(), account_id.to_owned())),
        }))
    }

    /// The access token to send. Bound credentials first renew one that is
    /// about to expire; once a renewal is rejected, the rest of the run gets
    /// that rejection.
    pub async fn access_token(&self, provider: OAuthProvider) -> SyncResult<String> {
        // Held across the renewal, so concurrent requests of one run share a
        // refresh instead of each spending the same refresh token.
        let mut current = self.0.current.lock().await;
        if let Some((pool, account_id)) = &self.0.account {
            if !oauth_config_has_fresh_access_token(&current.config) {
                if let Some(rejected) = &current.rejected {
                    return Err(rejected.clone());
                }
                match fresh_account_config_for_provider(pool, account_id, provider).await {
                    Ok(config) => current.config = config,
                    Err(err) => {
                        if err.is_permanent() {
                            current.rejected = Some(err.clone());
                        }
                        return Err(err);
                    }
                }
            }
        }
        access_token_from_account_config(provider, &current.config).await
    }
}

/// Holds tokens; only the account it renews from is printed.
impl std::fmt::Debug for OAuthCredentials {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OAuthCredentials")
            .field("account_id", &self.0.account.as_ref().map(|(_, id)| id))
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::encrypt_account_config;
    use crate::utils::oauth::OAUTH_RELOAD_REQUIRED;

    fn config(access: &str, expires_at: &str) -> String {
        serde_json::json!({"oauth": {
            "provider": "gmail", "client_id": "fixture-client",
            "access_token": format!("{access}-access"), "refresh_token": format!("{access}-refresh"),
            "expires_at": expires_at
        }})
        .to_string()
    }

    async fn account(stored: &str) -> SqlitePool {
        crate::utils::crypto::seed_master_key_for_tests();
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        crate::db::init(&pool).await.unwrap();
        sqlx::query("INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
            VALUES ('account', 'Fixture', 'fixture@example.test', 'gmail', ?, 'encrypted-in-config', '2026-01-01', '2026-01-01')")
            .bind(encrypt_account_config("account", stored).unwrap())
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    #[tokio::test]
    async fn an_expired_snapshot_renews_from_the_stored_account() {
        // Another worker already rotated and persisted the token this run
        // started with; the run continues on the stored one.
        let pool = account(&config("rotated", "2099-01-01T00:00:00Z")).await;
        let started = AccountConfig::from_plaintext(config("started", "2000-01-01T00:00:00Z"));
        let stored = OAuthCredentials::stored(&pool, "account", &started);
        assert_eq!(
            stored.access_token(OAuthProvider::Gmail).await.unwrap(),
            "rotated-access"
        );
        // Once renewed, the run keeps the renewed token without reloading.
        sqlx::query("DELETE FROM accounts")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            stored.access_token(OAuthProvider::Gmail).await.unwrap(),
            "rotated-access"
        );
        // Detached credentials never refresh what they cannot persist.
        assert_eq!(
            OAuthCredentials::detached(&started)
                .access_token(OAuthProvider::Gmail)
                .await
                .unwrap_err(),
            SyncError::Transient(OAUTH_RELOAD_REQUIRED.into())
        );
    }

    #[tokio::test]
    async fn a_permanent_refresh_failure_ends_renewal_for_the_rest_of_the_run() {
        // Without a refresh token the renewal fails permanently, like a revoked one.
        let revoked = serde_json::json!({"oauth": {
            "provider": "gmail", "client_id": "fixture-client",
            "access_token": "started-access", "expires_at": "2000-01-01T00:00:00Z"
        }})
        .to_string();
        let pool = account(&revoked).await;
        let stored =
            OAuthCredentials::stored(&pool, "account", &AccountConfig::from_plaintext(revoked));
        let first = stored.access_token(OAuthProvider::Gmail).await.unwrap_err();
        assert!(first.is_permanent());
        // Later folders of the run neither reload the account nor ask the
        // token endpoint again; they get the first answer.
        sqlx::query("DELETE FROM accounts")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            stored
                .clone()
                .access_token(OAuthProvider::Gmail)
                .await
                .unwrap_err(),
            first
        );
    }

    #[tokio::test]
    async fn a_token_that_cannot_outlast_one_call_is_renewed_before_it_is_served() {
        // A single request can wait out rate limits for minutes on the token it
        // was built with: four minutes left is not enough for one call.
        let pool = account(&config("rotated", "2099-01-01T00:00:00Z")).await;
        let soon = chrono::Utc::now() + chrono::Duration::minutes(4);
        let started = AccountConfig::from_plaintext(config("started", &soon.to_rfc3339()));
        assert_eq!(
            OAuthCredentials::stored(&pool, "account", &started)
                .access_token(OAuthProvider::Gmail)
                .await
                .unwrap(),
            "rotated-access"
        );
    }

    #[tokio::test]
    async fn a_reconnected_provider_is_not_served_the_old_providers_token() {
        let pool = account(&config("started", "2000-01-01T00:00:00Z")).await;
        sqlx::query("UPDATE accounts SET provider = 'outlook'")
            .execute(&pool)
            .await
            .unwrap();
        let started = AccountConfig::from_plaintext(config("started", "2000-01-01T00:00:00Z"));
        assert_eq!(
            OAuthCredentials::stored(&pool, "account", &started)
                .access_token(OAuthProvider::Gmail)
                .await
                .unwrap_err(),
            SyncError::Transient(OAUTH_RELOAD_REQUIRED.into())
        );
    }
}
