//! Owns refresh persistence. No database transaction spans a provider request.
//! After HTTP, a short write transaction reloads the current row. Settings-only
//! changes merge with a successful rotation; changed authorizations win instead.
//! Successful rotations merge once, regardless of intervening settings writes.

use std::future::Future;

use sqlx::SqlitePool;

use super::{ensure_fresh_account_config, OAuthProvider};
use crate::accounts::{decrypt_account_config, encrypt_account_config, AccountConfig};
use crate::sync::errors::{SyncError, SyncResult};

/// Load and persist current credentials before returning them for provider use.
pub async fn fresh_account_config(
    pool: &SqlitePool,
    account_id: &str,
) -> SyncResult<AccountConfig> {
    fresh_account_config_with(pool, account_id, None, |provider, config| async move {
        ensure_fresh_account_config(provider, &config).await
    })
    .await
}

/// Load credentials for callers holding provider-specific metadata. Reject a
/// changed provider before allowing its credentials to reach the old endpoint.
pub async fn fresh_account_config_for_provider(
    pool: &SqlitePool,
    account_id: &str,
    provider: OAuthProvider,
) -> SyncResult<AccountConfig> {
    let snapshot: Option<String> =
        sqlx::query_scalar("SELECT config FROM accounts WHERE id = ? AND provider = ?")
            .bind(account_id)
            .bind(provider.as_str())
            .fetch_optional(pool)
            .await
            .map_err(|err| SyncError::Transient(err.to_string()))?;
    let snapshot = snapshot.ok_or_else(|| {
        SyncError::Transient("account provider changed or account was removed".into())
    })?;
    refresh_account_config(pool, account_id, provider.as_str(), &snapshot).await
}

/// Start from a persisted row. A provider change requires metadata to be reloaded.
pub async fn refresh_account_config(
    pool: &SqlitePool,
    account_id: &str,
    provider: &str,
    snapshot: &str,
) -> SyncResult<AccountConfig> {
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
) -> SyncResult<AccountConfig>
where
    F: FnMut(OAuthProvider, AccountConfig) -> Fut,
    Fut: Future<Output = SyncResult<Option<String>>>,
{
    let mut original_provider = None;
    for _ in 0..4 {
        let (provider, snapshot): (String, String) = match initial.take() {
            Some(snapshot) => snapshot,
            None => sqlx::query_as("SELECT provider, config FROM accounts WHERE id = ?")
                .bind(account_id)
                .fetch_optional(pool)
                .await
                .map_err(|err| SyncError::Transient(err.to_string()))?
                .ok_or_else(|| SyncError::Permanent("account was not found".into()))?,
        };
        if original_provider
            .as_ref()
            .is_some_and(|original| original != &provider)
        {
            return Err(SyncError::Transient(
                "account provider changed during oauth refresh".into(),
            ));
        }
        original_provider.get_or_insert_with(|| provider.clone());
        let config = decrypt_account_config(account_id, &snapshot).map_err(SyncError::Permanent)?;
        let Ok(oauth_provider) = OAuthProvider::parse(&provider) else {
            return Ok(config);
        };
        let result = refresh(oauth_provider, config.clone()).await;

        // Acquire the writer reservation only AFTER the request. Ordinary
        // settings writers cannot slip between this reload and the merge.
        let mut tx = pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(|err| SyncError::Transient(err.to_string()))?;
        let (current_provider, current_snapshot): (String, String) =
            sqlx::query_as("SELECT provider, config FROM accounts WHERE id = ?")
                .bind(account_id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(|err| SyncError::Transient(err.to_string()))?
                .ok_or_else(|| SyncError::Permanent("account was not found".into()))?;
        if current_provider != provider {
            return Err(SyncError::Transient(
                "account provider changed during oauth refresh".into(),
            ));
        }
        let current =
            decrypt_account_config(account_id, &current_snapshot).map_err(SyncError::Permanent)?;
        // Reading a fresh token is not a successful refresh. In particular,
        // reloading after a reconnect must not clear that authorization's newer
        // error state just because our old HTTP response was discarded.
        if matches!(result, Ok(None)) && oauth_block(&config) == oauth_block(&current) {
            tx.commit()
                .await
                .map_err(|err| SyncError::Transient(err.to_string()))?;
            return Ok(current);
        }
        let updated = match &result {
            Ok(Some(updated))
                if oauth_block(&config).is_some()
                    && oauth_block(&config) == oauth_block(&current) =>
            {
                let mut merged: serde_json::Value = serde_json::from_str(&current)
                    .map_err(|err| SyncError::Permanent(err.to_string()))?;
                merged["oauth"] = oauth_block(updated).ok_or_else(|| {
                    SyncError::Permanent("refreshed oauth configuration is invalid".into())
                })?;
                Some(merged.to_string())
            }
            _ if snapshot == current_snapshot => None,
            _ => {
                // Never apply an old response or error to a new authorization.
                tx.rollback()
                    .await
                    .map_err(|err| SyncError::Transient(err.to_string()))?;
                initial = Some((current_provider, current_snapshot));
                continue;
            }
        };
        let now = chrono::Utc::now().to_rfc3339();
        match &result {
            Ok(_) => {
                let stored = match &updated {
                    Some(updated) => {
                        encrypt_account_config(account_id, updated).map_err(SyncError::Permanent)?
                    }
                    None => current_snapshot,
                };
                sqlx::query(
                    "UPDATE accounts SET config = ?, updated_at = ?,
                    last_oauth_error = NULL, last_oauth_error_at = NULL WHERE id = ?",
                )
                .bind(stored)
                .bind(&now)
                .bind(account_id)
                .execute(&mut *tx)
                .await
            }
            Err(err) => {
                let diagnostic = err.to_string();
                sqlx::query(
                    "UPDATE accounts SET last_oauth_error_at = CASE
                    WHEN last_oauth_error = ? THEN last_oauth_error_at ELSE ? END,
                    last_oauth_error = ? WHERE id = ?",
                )
                .bind(&diagnostic)
                .bind(&now)
                .bind(&diagnostic)
                .bind(account_id)
                .execute(&mut *tx)
                .await
            }
        }
        .map_err(|err| SyncError::Transient(err.to_string()))?;
        tx.commit()
            .await
            .map_err(|err| SyncError::Transient(err.to_string()))?;
        return result.map(|_| {
            updated
                .map(AccountConfig::from_plaintext)
                .unwrap_or(current)
        });
    }
    Err(SyncError::Transient(
        "account configuration changed during oauth refresh".into(),
    ))
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
        crate::db::init(&pool).await.unwrap();
        sqlx::query("INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
            VALUES ('account', 'Fixture', 'fixture@example.test', 'gmail', ?, 'encrypted-in-config', '2026-01-01', '2026-01-01')")
            .bind(encrypt_account_config("account", &config("old", false)).unwrap())
            .execute(&pool).await.unwrap();
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
    async fn paused_reconnect(fail: bool, new_error: bool) {
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
                                Err(SyncError::Permanent("invalid_grant".into()))
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
        sqlx::query("UPDATE accounts SET config = ?, last_oauth_error = ?, last_oauth_error_at = ? WHERE id = 'account'")
            .bind(encrypt_account_config("account", &current).unwrap())
            .bind(new_error.then_some("new authorization error"))
            .bind(new_error.then_some("new error time"))
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
        assert_eq!(
            error.as_deref(),
            new_error.then_some("new authorization error")
        );
        if new_error {
            let at: String = sqlx::query_scalar("SELECT last_oauth_error_at FROM accounts")
                .fetch_one(&pool)
                .await
                .unwrap();
            assert_eq!(at, "new error time");
        }
    }

    #[tokio::test]
    async fn paused_refresh_success_preserves_reauthorization() {
        paused_reconnect(false, false).await;
    }
    #[tokio::test]
    async fn paused_refresh_failure_uses_reauthorization() {
        paused_reconnect(true, false).await;
    }

    #[tokio::test]
    async fn late_refresh_results_preserve_new_authorization_error_state() {
        paused_reconnect(false, true).await;
        paused_reconnect(true, true).await;
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
                        Err(SyncError::Permanent("invalid_grant".into()))
                    } else {
                        Ok(Some(refreshed(&config)))
                    }
                }
            })
            .await;
            assert_eq!(
                result.unwrap_err(),
                SyncError::Permanent("account was not found".into())
            );
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
                    Err(SyncError::Permanent("invalid_grant".into()))
                })
                .await
                .unwrap_err(),
                SyncError::Permanent("invalid_grant".into())
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
                Err(SyncError::Permanent("invalid_grant".into()))
            }
        })
        .await
        .unwrap_err();
        assert_eq!(
            result,
            SyncError::Transient("account configuration changed during oauth refresh".into())
        );
        assert!(!result.is_permanent());
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
            SyncError::Transient("account provider changed during oauth refresh".into())
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
    fn dav_config(server: &str) -> crate::providers::dav::DavConfig {
        crate::providers::dav::DavConfig {
            server: server.into(),
            username: "fixture".into(),
            password: "fixture-only".into(),
        }
    }

    #[tokio::test]
    async fn dav_settings_before_and_after_rotation_preserve_credentials_and_errors() {
        let pool = fixture().await;
        let writer = pool.clone();
        let result = fresh_account_config_with(&pool, "account", None, move |_, config| {
            let writer = writer.clone();
            async move {
                // This production writer runs while the provider request is
                // outstanding. It must complete without a refresh-held lock.
                crate::db::accounts::AccountRepository::new(&writer)
                    .update_dav_config(
                        "account",
                        Some(dav_config("https://cal.example.test")),
                        None,
                        true,
                    )
                    .await
                    .unwrap();
                Ok(Some(refreshed(&config)))
            }
        })
        .await
        .unwrap();
        assert_eq!(
            load_stored_oauth(&result).unwrap().refresh_token.as_deref(),
            Some("rotated-refresh")
        );
        sqlx::query(
            "UPDATE accounts SET last_oauth_error = 'new error', last_oauth_error_at = 'new time'",
        )
        .execute(&pool)
        .await
        .unwrap();
        // A settings intent captured before refresh must only touch its DAV fields.
        let current = crate::db::accounts::AccountRepository::new(&pool)
            .update_dav_config(
                "account",
                None,
                Some(dav_config("https://card.example.test")),
                false,
            )
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_str(&current).unwrap();
        assert_eq!(value["caldav"]["server"], "https://cal.example.test");
        assert_eq!(value["carddav"]["server"], "https://card.example.test");
        assert_eq!(value["setting"], "old");
        assert_eq!(value["oauth"]["refresh_token"], "rotated-refresh");
        assert_eq!(stored(&pool).await, current.as_str());
        let error: (String, String) =
            sqlx::query_as("SELECT last_oauth_error, last_oauth_error_at FROM accounts")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(error, ("new error".into(), "new time".into()));
    }

    #[tokio::test]
    async fn dav_settings_apply_to_reauthorization_without_restoring_old_oauth() {
        let pool = fixture().await;
        let repo = crate::db::accounts::AccountRepository::new(&pool);
        let client = OAuthProvider::Gmail.build_config(
            "new-client".into(),
            None,
            "http://127.0.0.1:1".into(),
        );
        repo.connect_oauth(
            "account",
            &client,
            &OAuthTokenPayload {
                access_token: "new-access".into(),
                refresh_token: Some("new-refresh".into()),
                token_type: None,
                scope: Some("new-scope".into()),
                expires_at: Some("2099-01-01T00:00:00Z".into()),
            },
            &crate::utils::oauth::OAuthIdentity {
                email: "fixture@example.test".into(),
                display_name: None,
            },
            "2026-01-01",
        )
        .await
        .unwrap();
        let config = repo
            .update_dav_config(
                "account",
                Some(dav_config("https://cal.example.test")),
                None,
                true,
            )
            .await
            .unwrap();
        let oauth = load_stored_oauth(&config).unwrap();
        assert_eq!(oauth.client_id.as_deref(), Some("new-client"));
        assert_eq!(oauth.scope.as_deref(), Some("new-scope"));
        assert_eq!(oauth.refresh_token.as_deref(), Some("new-refresh"));
        assert_eq!(stored(&pool).await, config.as_str());
    }

    #[tokio::test]
    async fn settings_churn_queued_around_refresh_commit_cannot_discard_rotation() {
        crate::utils::crypto::seed_master_key_for_tests();
        let dir = tempfile::tempdir().unwrap();
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(
                sqlx::sqlite::SqliteConnectOptions::new()
                    .filename(dir.path().join("race.db"))
                    .create_if_missing(true)
                    .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal),
            )
            .await
            .unwrap();
        crate::db::init(&pool).await.unwrap();
        sqlx::query("INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
            VALUES ('account', 'Fixture', 'fixture@example.test', 'gmail', ?, 'encrypted-in-config', '2026-01-01', '2026-01-01')")
            .bind(encrypt_account_config("account", &config("old", false)).unwrap()).execute(&pool).await.unwrap();
        // Hold the writer reservation until HTTP has completed. Refresh must
        // wait, then reload rather than exhausting settings-only CAS attempts.
        let mut writer = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
        let (done_tx, done_rx) = tokio::sync::oneshot::channel();
        let refresh = tokio::spawn({
            let pool = pool.clone();
            async move {
                let mut done = Some(done_tx);
                fresh_account_config_with(&pool, "account", None, move |_, config| {
                    done.take()
                        .expect("one-use refresh token must be exchanged only once")
                        .send(())
                        .unwrap();
                    async move { Ok(Some(refreshed(&config))) }
                })
                .await
                .unwrap()
            }
        });
        done_rx.await.unwrap();
        for revision in 0..8 {
            let mut current: serde_json::Value =
                serde_json::from_str(&config("old", false)).unwrap();
            current["setting"] = revision.into();
            sqlx::query("UPDATE accounts SET config = ? WHERE id = 'account'")
                .bind(encrypt_account_config("account", &current.to_string()).unwrap())
                .execute(&mut *writer)
                .await
                .unwrap();
        }
        let settings = tokio::spawn({
            let pool = pool.clone();
            async move {
                for revision in 0..32 {
                    crate::db::accounts::AccountRepository::new(&pool)
                        .update_dav_config(
                            "account",
                            Some(dav_config(&format!("https://cal.example.test/{revision}"))),
                            None,
                            true,
                        )
                        .await
                        .unwrap();
                    tokio::task::yield_now().await;
                }
            }
        });
        writer.commit().await.unwrap();
        let result = refresh.await.unwrap();
        settings.await.unwrap();
        assert_eq!(
            load_stored_oauth(&result).unwrap().refresh_token.as_deref(),
            Some("rotated-refresh")
        );
        let persisted: serde_json::Value = serde_json::from_str(&stored(&pool).await).unwrap();
        assert_eq!(persisted["oauth"]["refresh_token"], "rotated-refresh");
        assert_eq!(persisted["setting"], 7);
        assert_eq!(persisted["caldav"]["server"], "https://cal.example.test/31");
    }
}
