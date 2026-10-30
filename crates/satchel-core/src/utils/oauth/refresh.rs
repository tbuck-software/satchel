//! Refresh persistence belongs here, never at a caller's snapshot-based UPDATE.
//!
//! Config writes encrypt with a new random nonce, so exact ciphertext equality
//! is a revision of the whole config, even if the plaintext is unchanged. This
//! avoids timestamp collisions and ABA across reauthorization. The provider is
//! compared too. No transaction spans the token request. The first refresh to
//! commit wins; a competing authorization/refresh discards the old response.
//! Settings-only changes retain a successful rotation: the unchanged OAuth
//! block permits rebasing it onto current settings, still gated on the entire
//! current ciphertext. This avoids reusing an already consumed refresh token.
//! Other conflicts reload/refresh current credentials. Missing rows fail closed.

use std::future::Future;

use sqlx::SqlitePool;

use super::{ensure_fresh_account_config, OAuthProvider};
use crate::accounts::{decrypt_account_config, encrypt_account_config, AccountConfig};

/// Load the current configuration and persist refreshed credentials before use.
/// OAuth accounts require a writer pool even when their token is already fresh.
/// Conflicts reload current credentials. Only settings-only changes can retain
/// a successful rotation, rebased onto those settings and saved conditionally.
/// Deletion fails closed. Repeated config churn is retried later.
pub async fn fresh_account_config(
    pool: &SqlitePool,
    account_id: &str,
) -> Result<AccountConfig, String> {
    fresh_account_config_with(pool, account_id, None, |provider, config| async move {
        ensure_fresh_account_config(provider, &config).await
    })
    .await
}

/// Like fresh_account_config, starting from an exact persisted row already read
/// by the caller. The raw ciphertext is the compare-and-save revision.
pub async fn refresh_account_config(
    pool: &SqlitePool,
    account_id: &str,
    provider: &str,
    snapshot: &str,
) -> Result<AccountConfig, String> {
    fresh_account_config_with(
        pool,
        account_id,
        Some((provider.to_owned(), snapshot.to_owned())),
        |provider, config| async move { ensure_fresh_account_config(provider, &config).await },
    )
    .await
}

async fn fresh_account_config_with<F, Fut>(
    pool: &SqlitePool,
    account_id: &str,
    mut initial: Option<(String, String)>,
    mut refresh: F,
) -> Result<AccountConfig, String>
where
    F: FnMut(OAuthProvider, AccountConfig) -> Fut,
    Fut: Future<Output = Result<Option<String>, String>>,
{
    let mut original_provider = None;
    let mut pending_refresh: Option<(serde_json::Value, serde_json::Value)> = None;
    for _ in 0..4 {
        let (provider, snapshot): (String, String) = match initial.take() {
            Some(snapshot) => snapshot,
            None => sqlx::query_as("SELECT provider, config FROM accounts WHERE id = ?")
                .bind(account_id)
                .fetch_optional(pool)
                .await
                .map_err(|err| err.to_string())?
                .ok_or_else(|| "account was not found".to_string())?,
        };
        if original_provider
            .as_ref()
            .is_some_and(|original| original != &provider)
        {
            // Callers also hold provider-specific metadata. Make them reload it
            // instead of sending a new provider's token to the old endpoint.
            return Err("[transient] account provider changed during oauth refresh".into());
        }
        original_provider.get_or_insert_with(|| provider.clone());
        let config = decrypt_account_config(account_id, &snapshot)?;
        let Ok(oauth_provider) = OAuthProvider::parse(&provider) else {
            return Ok(config);
        };
        // Keep a rotated token across settings-only writes without rebuilding
        // their config from our old snapshot. Any OAuth change invalidates the
        // pending response; the whole-row CAS below also closes the next race.
        let rebased = pending_refresh.take().and_then(|(expected, refreshed)| {
            let mut current: serde_json::Value = serde_json::from_str(&config).ok()?;
            if current.get("oauth")? != &expected {
                return None;
            }
            current["oauth"] = refreshed;
            Some(current.to_string())
        });
        let result = match rebased {
            Some(config) => Ok(Some(config)),
            None => refresh(oauth_provider, config.clone()).await,
        };
        let now = chrono::Utc::now().to_rfc3339();
        let changed = match &result {
            Ok(updated) => {
                let stored = match updated {
                    Some(updated) => encrypt_account_config(account_id, updated)?,
                    None => snapshot.clone(),
                };
                sqlx::query(
                    "UPDATE accounts SET config = ?, updated_at = ?,
                     last_oauth_error = NULL, last_oauth_error_at = NULL WHERE id = ? AND config = ? AND provider = ?",
                )
                .bind(stored)
                .bind(&now)
                .bind(account_id)
                .bind(&snapshot)
                .bind(&provider)
                .execute(pool)
                .await
            }
            Err(err) => {
                sqlx::query(
                    "UPDATE accounts SET last_oauth_error_at = CASE
                       WHEN last_oauth_error = ? THEN last_oauth_error_at ELSE ? END,
                     last_oauth_error = ? WHERE id = ? AND config = ? AND provider = ?",
                )
                .bind(err)
                .bind(&now)
                .bind(err)
                .bind(account_id)
                .bind(&snapshot)
                .bind(&provider)
                .execute(pool)
                .await
            }
        }
        .map_err(|err| err.to_string())?
        .rows_affected();
        if changed == 0 {
            if let Ok(Some(updated)) = &result {
                pending_refresh = oauth_block(&config).zip(oauth_block(updated));
            }
            continue;
        }
        return result.map(|updated| updated.map(AccountConfig::from_plaintext).unwrap_or(config));
    }
    Err("[transient] account configuration changed during oauth refresh".to_string())
}

fn oauth_block(config: &str) -> Option<serde_json::Value> {
    serde_json::from_str::<serde_json::Value>(config)
        .ok()?
        .get("oauth")
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::oauth::{
        load_stored_oauth, merge_authorized_token_into_config, OAuthTokenPayload,
    };

    async fn fixture() -> SqlitePool {
        crate::utils::crypto::seed_master_key_for_tests();
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::raw_sql(
            "CREATE TABLE accounts (id TEXT PRIMARY KEY, provider TEXT, config TEXT,
             updated_at TEXT, last_oauth_error TEXT, last_oauth_error_at TEXT);",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO accounts (id, provider, config) VALUES ('account', 'gmail', ?)")
            .bind(encrypt_account_config("account", &config("old", false)).unwrap())
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    fn config(name: &str, fresh: bool) -> String {
        serde_json::json!({"setting": name, "oauth": {
            "client_id": format!("{name}-client"), "redirect_uri": "http://127.0.0.1:1",
            "access_token": format!("{name}-access"), "refresh_token": format!("{name}-refresh"),
            "scope": format!("{name}-scope"),
            "expires_at": if fresh {"2099-01-01T00:00:00Z"} else {"2000-01-01T00:00:00Z"}
        }})
        .to_string()
    }

    fn refreshed(config: &AccountConfig) -> String {
        merge_authorized_token_into_config(
            config,
            &OAuthProvider::Gmail.build_config(
                "old-client".into(),
                None,
                "http://127.0.0.1:1".into(),
            ),
            &OAuthTokenPayload {
                access_token: "rotated-access".into(),
                refresh_token: Some("rotated-refresh".into()),
                token_type: Some("Bearer".into()),
                scope: None,
                expires_at: Some("2099-01-01T00:00:00Z".into()),
            },
        )
        .unwrap()
    }

    async fn stored(pool: &SqlitePool) -> String {
        let raw: String = sqlx::query_scalar("SELECT config FROM accounts WHERE id = 'account'")
            .fetch_one(pool)
            .await
            .unwrap();
        decrypt_account_config("account", &raw)
            .unwrap()
            .into_string()
    }

    // The oneshot channels place reauthorization exactly inside the token await.
    async fn paused_reconnect(fail: bool) {
        let pool = fixture().await;
        let (read_tx, read_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn({
            let pool = pool.clone();
            async move {
                let mut pause = Some((read_tx, release_rx));
                fresh_account_config_with(&pool, "account", None, move |_, config| {
                    let pause = pause.take();
                    async move {
                        if let Some((read, release)) = pause {
                            assert_eq!(
                                load_stored_oauth(&config).unwrap().client_id.as_deref(),
                                Some("old-client")
                            );
                            read.send(()).unwrap();
                            release.await.unwrap();
                            if fail {
                                Err("invalid_grant".into())
                            } else {
                                Ok(Some(refreshed(&config)))
                            }
                        } else {
                            assert!(super::super::oauth_config_has_fresh_access_token(&config));
                            Ok(None)
                        }
                    }
                })
                .await
            }
        });
        read_rx.await.unwrap();
        // Use the production authorization merger and encrypted account write.
        let current = config("new", true);
        let client = OAuthProvider::Gmail.build_config(
            "new-client".into(),
            None,
            "http://127.0.0.1:1".into(),
        );
        let token = OAuthTokenPayload {
            access_token: "new-access".into(),
            refresh_token: Some("new-refresh".into()),
            token_type: Some("Bearer".into()),
            scope: Some("new-scope".into()),
            expires_at: Some("2099-01-01T00:00:00Z".into()),
        };
        let current = merge_authorized_token_into_config(&current, &client, &token).unwrap();
        sqlx::query("UPDATE accounts SET config = ?, last_oauth_error = NULL WHERE id = 'account'")
            .bind(encrypt_account_config("account", &current).unwrap())
            .execute(&pool)
            .await
            .unwrap();
        release_tx.send(()).unwrap();
        let returned = task.await.unwrap().unwrap();
        assert_eq!(
            returned.as_str(),
            current,
            "downstream must use the current authorization"
        );
        assert_eq!(
            stored(&pool).await,
            current,
            "late refresh must preserve reauthorization and settings"
        );
        let error: Option<String> = sqlx::query_scalar("SELECT last_oauth_error FROM accounts")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(error, None);
    }

    #[tokio::test]
    async fn paused_refresh_success_preserves_reauthorization() {
        paused_reconnect(false).await;
    }
    #[tokio::test]
    async fn paused_refresh_failure_uses_reauthorization() {
        paused_reconnect(true).await;
    }

    #[tokio::test]
    async fn normal_refresh_keeps_rotated_tokens_scopes_and_settings() {
        let pool = fixture().await;
        let result = fresh_account_config_with(&pool, "account", None, |_, config| async move {
            Ok(Some(refreshed(&config)))
        })
        .await
        .unwrap();
        let oauth = load_stored_oauth(&result).unwrap();
        assert_eq!(oauth.refresh_token.as_deref(), Some("rotated-refresh"));
        assert_eq!(oauth.scope.as_deref(), Some("old-scope"));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&result).unwrap()["setting"],
            "old"
        );
        assert_eq!(stored(&pool).await, result.as_str());
        assert!(fresh_account_config(&pool, "account").await.is_ok());
    }

    #[tokio::test]
    async fn competing_refresh_uses_the_committed_winner() {
        let pool = fixture().await;
        let (read_tx, read_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let loser = tokio::spawn({
            let pool = pool.clone();
            async move {
                let mut pause = Some((read_tx, release_rx));
                fresh_account_config_with(&pool, "account", None, move |_, config| {
                    let pause = pause.take();
                    async move {
                        if let Some((read, release)) = pause {
                            read.send(()).unwrap();
                            release.await.unwrap();
                            let mut token: serde_json::Value =
                                serde_json::from_str(&refreshed(&config)).unwrap();
                            token["oauth"]["access_token"] = "loser-access".into();
                            token["oauth"]["refresh_token"] = "loser-refresh".into();
                            Ok(Some(token.to_string()))
                        } else {
                            Ok(None)
                        }
                    }
                })
                .await
                .unwrap()
            }
        });
        read_rx.await.unwrap();
        let winner = fresh_account_config_with(&pool, "account", None, |_, config| async move {
            Ok(Some(refreshed(&config)))
        })
        .await
        .unwrap();
        release_tx.send(()).unwrap();
        assert_eq!(loser.await.unwrap(), winner);
        assert_eq!(stored(&pool).await, winner.as_str());
    }

    #[tokio::test]
    async fn deleted_account_never_returns_or_recreates_credentials() {
        for fail in [false, true] {
            let pool = fixture().await;
            let writer = pool.clone();
            let result = fresh_account_config_with(&pool, "account", None, move |_, config| {
                let pool = writer.clone();
                async move {
                    sqlx::query("DELETE FROM accounts WHERE id = 'account'")
                        .execute(&pool)
                        .await
                        .unwrap();
                    if fail {
                        Err("invalid_grant".into())
                    } else {
                        Ok(Some(refreshed(&config)))
                    }
                }
            })
            .await;
            assert_eq!(result.unwrap_err(), "account was not found");
            assert_eq!(
                sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM accounts")
                    .fetch_one(&pool)
                    .await
                    .unwrap(),
                0
            );
            assert!(fresh_account_config(&pool, "missing").await.is_err());
        }
    }

    #[tokio::test]
    async fn current_failure_preserves_first_occurrence_and_clears_after_refresh() {
        let pool = fixture().await;
        for attempt in 0..2 {
            assert_eq!(
                fresh_account_config_with(&pool, "account", None, |_, _| async {
                    Err("invalid_grant".into())
                })
                .await
                .unwrap_err(),
                "invalid_grant"
            );
            if attempt == 0 {
                sqlx::query("UPDATE accounts SET last_oauth_error_at = 'first'")
                    .execute(&pool)
                    .await
                    .unwrap();
            }
        }
        let at: String = sqlx::query_scalar("SELECT last_oauth_error_at FROM accounts")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(at, "first");
        fresh_account_config_with(&pool, "account", None, |_, config| async move {
            Ok(Some(refreshed(&config)))
        })
        .await
        .unwrap();
        let error: Option<String> = sqlx::query_scalar("SELECT last_oauth_error FROM accounts")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(error.is_none());
    }

    fn new_expired_config() -> String {
        config("new", false)
    }

    #[tokio::test]
    async fn conflict_retries_current_expired_credentials_and_keeps_settings() {
        let pool = fixture().await;
        let writer = pool.clone();
        let mut first = true;
        let result = fresh_account_config_with(&pool, "account", None, move |provider, config| {
            let first = std::mem::replace(&mut first, false);
            let pool = writer.clone();
            async move {
                if first {
                    sqlx::query("UPDATE accounts SET config = ? WHERE id = 'account'")
                        .bind(encrypt_account_config("account", &new_expired_config()).unwrap())
                        .execute(&pool)
                        .await
                        .unwrap();
                    return Ok(Some(refreshed(&config)));
                }
                let stored = load_stored_oauth(&config).unwrap();
                assert_eq!(stored.refresh_token.as_deref(), Some("new-refresh"));
                let client = provider.load_config_for_stored(&stored).unwrap();
                Ok(Some(
                    merge_authorized_token_into_config(
                        &config,
                        &client,
                        &OAuthTokenPayload {
                            access_token: "current-refreshed".into(),
                            refresh_token: Some("current-rotated".into()),
                            token_type: None,
                            scope: None,
                            expires_at: Some("2099-01-01T00:00:00Z".into()),
                        },
                    )
                    .unwrap(),
                ))
            }
        })
        .await
        .unwrap();
        let value: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(value["setting"], "new");
        assert_eq!(value["oauth"]["scope"], "new-scope");
        assert_eq!(value["oauth"]["client_id"], "new-client");
        assert_eq!(value["oauth"]["refresh_token"], "current-rotated");
        assert_eq!(stored(&pool).await, result.as_str());
    }

    #[tokio::test]
    async fn repeated_config_changes_stop_without_persisting_a_stale_error() {
        let pool = fixture().await;
        let writer = pool.clone();
        let result = fresh_account_config_with(&pool, "account", None, move |_, config| {
            let pool = writer.clone();
            async move {
                // Even identical plaintext reencrypted is a new revision.
                sqlx::query("UPDATE accounts SET config = ? WHERE id = 'account'")
                    .bind(encrypt_account_config("account", &config).unwrap())
                    .execute(&pool)
                    .await
                    .unwrap();
                Err("invalid_grant".into())
            }
        })
        .await
        .unwrap_err();
        assert_eq!(
            result,
            "[transient] account configuration changed during oauth refresh"
        );
        assert!(!crate::sync::errors::SyncError::from_message(result).is_permanent());
        let error: Option<String> = sqlx::query_scalar("SELECT last_oauth_error FROM accounts")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(error.is_none());
    }

    #[tokio::test]
    async fn provider_change_requires_metadata_reload_without_using_old_result() {
        let pool = fixture().await;
        let writer = pool.clone();
        let result = fresh_account_config_with(&pool, "account", None, move |_, config| {
            let pool = writer.clone();
            async move {
                sqlx::query(
                    "UPDATE accounts SET provider = 'outlook', config = ? WHERE id = 'account'",
                )
                .bind(encrypt_account_config("account", &new_expired_config()).unwrap())
                .execute(&pool)
                .await
                .unwrap();
                Ok(Some(refreshed(&config)))
            }
        })
        .await
        .unwrap_err();
        assert_eq!(
            result,
            "[transient] account provider changed during oauth refresh"
        );
        assert_eq!(stored(&pool).await, new_expired_config());
    }

    #[tokio::test]
    async fn settings_change_preserves_rotation_without_reusing_consumed_refresh_token() {
        let pool = fixture().await;
        let writer = pool.clone();
        let mut calls = 0;
        let result = fresh_account_config_with(&pool, "account", None, move |_, config| {
            calls += 1;
            assert_eq!(
                calls, 1,
                "a rotated refresh token may already have consumed the old token"
            );
            let pool = writer.clone();
            async move {
                let mut current: serde_json::Value = serde_json::from_str(&config).unwrap();
                current["setting"] = "changed-while-refreshing".into();
                sqlx::query("UPDATE accounts SET config = ? WHERE id = 'account'")
                    .bind(encrypt_account_config("account", &current.to_string()).unwrap())
                    .execute(&pool)
                    .await
                    .unwrap();
                Ok(Some(refreshed(&config)))
            }
        })
        .await
        .unwrap();
        let value: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(value["setting"], "changed-while-refreshing");
        assert_eq!(value["oauth"]["refresh_token"], "rotated-refresh");
        assert_eq!(value["oauth"]["access_token"], "rotated-access");
        assert_eq!(value["oauth"]["scope"], "old-scope");
        assert_eq!(stored(&pool).await, result.as_str());
    }
}
