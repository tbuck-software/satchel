//! Owns refresh persistence. No database transaction spans a provider request.
//! A fresh token, a temporary failure or a rejection already on record stores
//! nothing, so a plain read checks that it still applies. A rotation or a new
//! rejection reloads the row in a short
//! write transaction after HTTP: settings-only changes merge with a successful
//! rotation, changed authorizations win instead. Successful rotations merge
//! once, regardless of intervening settings writes.

use std::future::Future;

use sqlx::SqlitePool;

use super::{
    ensure_fresh_account_config, OAuthProvider, OAUTH_REFRESH_REJECTED, OAUTH_RELOAD_REQUIRED,
};
use crate::accounts::{decrypt_account_config, encrypt_account_config, AccountConfig};
use crate::sync::errors::{SyncError, SyncResult};

/// A re-authorization failure that one refresh newly stored for its account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedOAuthError {
    pub error: String,
    pub at: String,
}

/// Credentials after a refresh, plus the rejection it stored when that changed
/// what the account shows. Announcing only those keeps a repeated failure from
/// raising the reconnect prompt again on every attempt.
#[derive(Debug)]
pub struct RefreshOutcome {
    pub config: SyncResult<AccountConfig>,
    pub recorded_error: Option<RecordedOAuthError>,
}

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
    let snapshot = snapshot.ok_or_else(reload_required)?;
    refresh_account_config(pool, account_id, provider.as_str(), &snapshot).await
}

/// Start from a persisted row. A provider change requires metadata to be reloaded.
pub async fn refresh_account_config(
    pool: &SqlitePool,
    account_id: &str,
    provider: &str,
    snapshot: &str,
) -> SyncResult<AccountConfig> {
    refresh_account_config_reporting(pool, account_id, provider, snapshot)
        .await
        .config
}

/// [`refresh_account_config`] for callers that announce a newly stored rejection.
pub async fn refresh_account_config_reporting(
    pool: &SqlitePool,
    account_id: &str,
    provider: &str,
    snapshot: &str,
) -> RefreshOutcome {
    refresh_with(
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
    initial: Option<(String, String)>,
    refresh: F,
) -> SyncResult<AccountConfig>
where
    F: FnMut(OAuthProvider, AccountConfig) -> Fut,
    Fut: Future<Output = SyncResult<Option<String>>>,
{
    refresh_with(pool, account_id, initial, refresh)
        .await
        .config
}

async fn refresh_with<F, Fut>(
    pool: &SqlitePool,
    account_id: &str,
    initial: Option<(String, String)>,
    refresh: F,
) -> RefreshOutcome
where
    F: FnMut(OAuthProvider, AccountConfig) -> Fut,
    Fut: Future<Output = SyncResult<Option<String>>>,
{
    let mut recorded_error = None;
    let config =
        refresh_until_current(pool, account_id, initial, refresh, &mut recorded_error).await;
    RefreshOutcome {
        config,
        recorded_error,
    }
}

async fn refresh_until_current<F, Fut>(
    pool: &SqlitePool,
    account_id: &str,
    mut initial: Option<(String, String)>,
    mut refresh: F,
    recorded_error: &mut Option<RecordedOAuthError>,
) -> SyncResult<AccountConfig>
where
    F: FnMut(OAuthProvider, AccountConfig) -> Fut,
    Fut: Future<Output = SyncResult<Option<String>>>,
{
    let mut original_provider = None;
    for _ in 0..4 {
        let (provider, snapshot) = match initial.take() {
            Some(snapshot) => snapshot,
            None => current_row(pool, account_id).await?,
        };
        if original_provider
            .as_ref()
            .is_some_and(|original| original != &provider)
        {
            return Err(reload_required());
        }
        original_provider.get_or_insert_with(|| provider.clone());
        let config = decrypt_account_config(account_id, &snapshot).map_err(SyncError::Permanent)?;
        let Ok(oauth_provider) = OAuthProvider::parse(&provider) else {
            return Ok(config);
        };
        let result = refresh(oauth_provider, config.clone()).await;

        // A fresh token or a temporary failure stores nothing, so it must not
        // queue for the writer behind an ingest. A plain read still tells
        // whether a reconnect landed while the request was out.
        let persists = match &result {
            Ok(updated) => updated.is_some(),
            Err(err) => matches!(err, SyncError::Permanent(_)),
        };
        if !persists {
            let (current_provider, current_snapshot) = current_row(pool, account_id).await?;
            if current_provider != provider {
                return Err(reload_required());
            }
            let current = decrypt_account_config(account_id, &current_snapshot)
                .map_err(SyncError::Permanent)?;
            // Reading a fresh token is not a successful refresh. In particular,
            // reloading after a reconnect must not clear that authorization's
            // newer error state just because our old HTTP response was discarded.
            if oauth_block(&config) == oauth_block(&current) {
                return result.map(|_| current);
            }
            initial = Some((current_provider, current_snapshot));
            continue;
        }

        // A rejection already on record changes nothing either. Every later
        // attempt with a revoked refresh token lands here.
        if let Err(err) = &result {
            if rejection_on_record(pool, account_id, &provider, &snapshot, err).await? {
                return Err(err.clone());
            }
        }

        // Acquire the writer reservation only AFTER the request. Ordinary
        // settings writers cannot slip between this reload and the merge.
        let mut tx = pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(|err| SyncError::Transient(err.to_string()))?;
        let (current_provider, current_snapshot, stored_error): (String, String, Option<String>) =
            sqlx::query_as("SELECT provider, config, last_oauth_error FROM accounts WHERE id = ?")
                .bind(account_id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(|err| SyncError::Transient(err.to_string()))?
                .ok_or_else(account_not_found)?;
        if current_provider != provider {
            return Err(reload_required());
        }
        let current =
            decrypt_account_config(account_id, &current_snapshot).map_err(SyncError::Permanent)?;
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
        let mut recorded = None;
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
                .map_err(|err| SyncError::Transient(err.to_string()))?;
            }
            // A rejection already on record keeps its first occurrence and is
            // not announced again.
            Err(err) if !same_rejection(stored_error.as_deref(), err) => {
                let diagnostic = err.to_string();
                sqlx::query(
                    "UPDATE accounts SET last_oauth_error = ?, last_oauth_error_at = ?
                    WHERE id = ?",
                )
                .bind(&diagnostic)
                .bind(&now)
                .bind(account_id)
                .execute(&mut *tx)
                .await
                .map_err(|err| SyncError::Transient(err.to_string()))?;
                recorded = Some(RecordedOAuthError {
                    error: diagnostic,
                    at: now,
                });
            }
            Err(_) => {}
        }
        tx.commit()
            .await
            .map_err(|err| SyncError::Transient(err.to_string()))?;
        *recorded_error = recorded;
        return result.map(|_| {
            updated
                .map(AccountConfig::from_plaintext)
                .unwrap_or(current)
        });
    }
    Err(reload_required())
}

async fn current_row(pool: &SqlitePool, account_id: &str) -> SyncResult<(String, String)> {
    sqlx::query_as("SELECT provider, config FROM accounts WHERE id = ?")
        .bind(account_id)
        .fetch_optional(pool)
        .await
        .map_err(|err| SyncError::Transient(err.to_string()))?
        .ok_or_else(account_not_found)
}

/// The credentials changed under this refresh; reloading the account resolves it.
fn reload_required() -> SyncError {
    SyncError::Transient(OAUTH_RELOAD_REQUIRED.into())
}

fn account_not_found() -> SyncError {
    SyncError::Permanent("backend.errors.accountNotFound".into())
}

/// Whether the unchanged row already records `err`. A plain read: when it
/// does, the refresh has nothing to write and need not wait for the writer.
async fn rejection_on_record(
    pool: &SqlitePool,
    account_id: &str,
    provider: &str,
    snapshot: &str,
    err: &SyncError,
) -> SyncResult<bool> {
    let (current_provider, current_snapshot, stored_error): (String, String, Option<String>) =
        sqlx::query_as("SELECT provider, config, last_oauth_error FROM accounts WHERE id = ?")
            .bind(account_id)
            .fetch_optional(pool)
            .await
            .map_err(|err| SyncError::Transient(err.to_string()))?
            .ok_or_else(account_not_found)?;
    Ok(current_provider == provider
        && current_snapshot == snapshot
        && same_rejection(stored_error.as_deref(), err))
}

fn same_rejection(stored: Option<&str>, err: &SyncError) -> bool {
    stored.is_some_and(|stored| rejection_key(stored) == rejection_key(&err.to_string()))
}

/// What identifies a rejection across attempts: the HTTP status and the OAuth
/// `error` code (`400 invalid_grant`). The rest of a token endpoint's body,
/// such as Microsoft's trace and correlation ids, differs on every response.
/// Local failures carry no response and compare as written.
fn rejection_key(diagnostic: &str) -> String {
    let Some(response) = diagnostic.strip_prefix(OAUTH_REFRESH_REJECTED) else {
        return diagnostic.to_owned();
    };
    let (status, body) = response.split_once(": ").unwrap_or((response, ""));
    let status = status.split_whitespace().next().unwrap_or_default();
    let code = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|body| body.get("error")?.as_str().map(str::to_owned));
    match code {
        Some(code) => format!("{status} {code}"),
        None => status.to_owned(),
    }
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
        OAUTH_RELOAD_REQUIRED,
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
                SyncError::Permanent("backend.errors.accountNotFound".into())
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
        assert_eq!(result, SyncError::Transient(OAUTH_RELOAD_REQUIRED.into()));
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
        assert_eq!(result, SyncError::Transient(OAUTH_RELOAD_REQUIRED.into()));
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

    async fn wal_fixture(dir: &std::path::Path, config: &str) -> SqlitePool {
        crate::utils::crypto::seed_master_key_for_tests();
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(
                sqlx::sqlite::SqliteConnectOptions::new()
                    .filename(dir.join("reader.db"))
                    .create_if_missing(true)
                    .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal),
            )
            .await
            .unwrap();
        crate::db::init(&pool).await.unwrap();
        sqlx::query("INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
            VALUES ('account', 'Fixture', 'fixture@example.test', 'gmail', ?, 'encrypted-in-config', '2026-01-01', '2026-01-01')")
            .bind(encrypt_account_config("account", config).unwrap()).execute(&pool).await.unwrap();
        pool
    }

    async fn stored_error(pool: &SqlitePool) -> (Option<String>, Option<String>) {
        sqlx::query_as("SELECT last_oauth_error, last_oauth_error_at FROM accounts")
            .fetch_one(pool)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn fresh_tokens_and_temporary_failures_do_not_wait_for_the_writer() {
        let dir = tempfile::tempdir().unwrap();
        let pool = wal_fixture(dir.path(), &config("old", true)).await;
        // An ingest transaction holds the writer reservation. Opening a message
        // only reads credentials and must not queue behind it.
        let mut ingest = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
        sqlx::query("UPDATE accounts SET updated_at = 'ingest' WHERE id = 'account'")
            .execute(&mut *ingest)
            .await
            .unwrap();
        let wait = std::time::Duration::from_secs(2);
        let fresh = tokio::time::timeout(wait, fresh_account_config(&pool, "account"))
            .await
            .expect("a fresh token must not wait for the writer")
            .unwrap();
        assert_eq!(fresh.as_str(), config("old", true));
        let offline = tokio::time::timeout(
            wait,
            fresh_account_config_with(&pool, "account", None, |_, _| async {
                Err(SyncError::Transient("offline".into()))
            }),
        )
        .await
        .expect("a temporary failure must not wait for the writer")
        .unwrap_err();
        assert_eq!(offline, SyncError::Transient("offline".into()));
        ingest.commit().await.unwrap();
        assert_eq!(stored_error(&pool).await, (None, None));
    }

    #[tokio::test]
    async fn only_reauthorization_failures_are_stored_and_reported_once() {
        let pool = fixture().await;
        let attempt = |error: SyncError| {
            let pool = pool.clone();
            async move {
                refresh_with(&pool, "account", None, move |_, _| {
                    let error = error.clone();
                    async move { Err(error) }
                })
                .await
            }
        };
        // A network flap after resume is not a reason to ask for a reconnect.
        for error in [
            SyncError::Transient("oauth refresh request failed: offline".into()),
            SyncError::rate_limited("oauth refresh failed with status 429", None),
        ] {
            let outcome = attempt(error.clone()).await;
            assert_eq!(outcome.config.unwrap_err(), error);
            assert_eq!(outcome.recorded_error, None);
            assert_eq!(stored_error(&pool).await, (None, None));
        }
        let first = attempt(SyncError::Permanent("invalid_grant".into())).await;
        let recorded = first.recorded_error.expect("first rejection is reported");
        assert_eq!(recorded.error, "invalid_grant");
        assert_eq!(
            stored_error(&pool).await,
            (Some("invalid_grant".into()), Some(recorded.at.clone()))
        );
        // The same rejection again changes nothing, so nothing is announced.
        let repeated = attempt(SyncError::Permanent("invalid_grant".into())).await;
        assert_eq!(repeated.recorded_error, None);
        // A temporary failure keeps the stored rejection as it is.
        let flap = attempt(SyncError::Transient("offline".into())).await;
        assert_eq!(flap.recorded_error, None);
        assert_eq!(
            stored_error(&pool).await,
            (Some("invalid_grant".into()), Some(recorded.at))
        );
        let changed = attempt(SyncError::Permanent("missing oauth refresh token".into())).await;
        assert_eq!(
            changed.recorded_error.map(|recorded| recorded.error),
            Some("missing oauth refresh token".into())
        );
    }

    /// A token endpoint rejection as `refresh_access_token` reports it.
    /// Microsoft stamps every response with its own trace and correlation id.
    fn rejection(status: &str, code: &str, trace: &str) -> SyncError {
        SyncError::from_message(format!(
            r#"oauth refresh failed with status {status}: {{"error":"{code}","error_description":"AADSTS70000: revoked. Trace ID: {trace} Correlation ID: corr-{trace}","trace_id":"{trace}","correlation_id":"corr-{trace}"}}"#
        ))
    }

    #[tokio::test]
    async fn a_repeated_provider_rejection_is_not_announced_again_despite_new_trace_ids() {
        let pool = fixture().await;
        let attempt = |error: SyncError| {
            let pool = pool.clone();
            async move {
                refresh_with(&pool, "account", None, move |_, _| {
                    let error = error.clone();
                    async move { Err(error) }
                })
                .await
            }
        };
        let first = attempt(rejection("400 Bad Request", "invalid_grant", "trace-1")).await;
        assert!(first.config.unwrap_err().is_permanent());
        let recorded = first.recorded_error.expect("first rejection is reported");
        let detail = rejection("400 Bad Request", "invalid_grant", "trace-1").to_string();
        assert_eq!(
            recorded.error, detail,
            "the banner keeps the provider's own text"
        );
        for trace in ["trace-2", "trace-3"] {
            let repeated = attempt(rejection("400 Bad Request", "invalid_grant", trace)).await;
            assert_eq!(repeated.recorded_error, None);
        }
        assert_eq!(
            stored_error(&pool).await,
            (Some(detail), Some(recorded.at)),
            "the first occurrence stays on record"
        );
        // A different answer from the endpoint is news.
        let changed = attempt(rejection("400 Bad Request", "invalid_client", "trace-4")).await;
        assert!(changed.recorded_error.is_some());
    }

    #[test]
    fn rejections_compare_by_status_and_oauth_error_code() {
        let key = |error: SyncError| rejection_key(&error.to_string());
        assert_eq!(
            key(rejection("400 Bad Request", "invalid_grant", "a")),
            key(rejection("400 Bad Request", "invalid_grant", "b"))
        );
        assert_ne!(
            key(rejection("400 Bad Request", "invalid_grant", "a")),
            key(rejection("400 Bad Request", "invalid_client", "a"))
        );
        assert_ne!(
            key(rejection("400 Bad Request", "invalid_grant", "a")),
            key(rejection("401 Unauthorized", "invalid_grant", "a"))
        );
        // Local failures carry no provider body and compare as written.
        assert_eq!(
            rejection_key("missing oauth refresh token"),
            "missing oauth refresh token"
        );
        assert_ne!(rejection_key("invalid_grant"), rejection_key("other"));
    }

    #[tokio::test]
    async fn a_rejection_already_on_record_does_not_wait_for_the_writer() {
        let dir = tempfile::tempdir().unwrap();
        let pool = wal_fixture(dir.path(), &config("old", false)).await;
        let on_record = rejection("400 Bad Request", "invalid_grant", "trace-1").to_string();
        sqlx::query("UPDATE accounts SET last_oauth_error = ?, last_oauth_error_at = 'first'")
            .bind(&on_record)
            .execute(&pool)
            .await
            .unwrap();
        // Every later folder of a revoked run lands here; an ingest holding the
        // writer must not hold them up, since there is nothing to store.
        let mut ingest = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
        sqlx::query("UPDATE accounts SET updated_at = 'ingest' WHERE id = 'account'")
            .execute(&mut *ingest)
            .await
            .unwrap();
        let repeated = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            refresh_with(&pool, "account", None, |_, _| async {
                Err(rejection("400 Bad Request", "invalid_grant", "trace-2"))
            }),
        )
        .await
        .expect("a rejection already on record must not wait for the writer");
        assert!(repeated.config.unwrap_err().is_permanent());
        assert_eq!(repeated.recorded_error, None);
        ingest.commit().await.unwrap();
        assert_eq!(
            stored_error(&pool).await,
            (Some(on_record), Some("first".into()))
        );
    }

    #[tokio::test]
    async fn local_races_ask_for_a_reload_by_message_key() {
        let pool = fixture().await;
        sqlx::query("UPDATE accounts SET provider = 'outlook'")
            .execute(&pool)
            .await
            .unwrap();
        let error = fresh_account_config_for_provider(&pool, "account", OAuthProvider::Gmail)
            .await
            .unwrap_err();
        assert_eq!(error, SyncError::Transient(OAUTH_RELOAD_REQUIRED.into()));
        // Queue and command seams keep the key instead of a generic reason.
        assert_eq!(error.encoded(), OAUTH_RELOAD_REQUIRED);
        let missing = fresh_account_config(&pool, "missing").await.unwrap_err();
        assert_eq!(
            missing,
            SyncError::Permanent("backend.errors.accountNotFound".into())
        );
    }
}
