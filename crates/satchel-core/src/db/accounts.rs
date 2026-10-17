//! Tauri-free persistence for mail account lifecycle operations.

use sqlx::{FromRow, Sqlite, SqlitePool, Transaction};
use thiserror::Error;

use crate::{
    accounts::{decrypt_account_config, encrypt_account_config, AccountConfig},
    utils::oauth::{self, OAuthIdentity, OAuthProvider, OAuthTokenPayload},
};

const DEFAULT_COLOR: &str = "#55a6ff";
const CREDENTIALS_REF: &str = "encrypted-in-config";
const IMAP_PROVIDER: &str = "imap";
const POP3_PROVIDER: &str = "pop3";

#[derive(Clone)]
pub struct CreateEmailAccount {
    pub display_name: String,
    pub email: String,
    pub provider: String,
    /// Plain JSON. The repository encrypts it before the database write.
    pub config: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct AccountSummary {
    pub id: String,
    pub display_name: String,
    pub email: String,
    pub provider: String,
    pub kind: String,
    pub color: Option<String>,
    pub last_oauth_error: Option<String>,
    pub last_oauth_error_at: Option<String>,
    pub last_sync_at: Option<String>,
    pub sync_status: Option<String>,
    pub last_sync_error: Option<String>,
    pub last_sync_error_at: Option<String>,
    pub sync_progress_json: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImapAccountConnection {
    pub id: String,
    pub email: String,
    /// Decrypted JSON. The stored representation never leaves this repository.
    pub config: AccountConfig,
}

/// A password account's config with a new password in place, checked against
/// the server before [`AccountRepository::store_password_change`] keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasswordChange {
    pub account_id: String,
    pub email: String,
    pub provider: String,
    /// Decrypted JSON. The stored representation never leaves this repository.
    pub config: AccountConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountProviderConfig {
    pub provider: String,
    /// Decrypted JSON. The stored representation never leaves this repository.
    pub config: AccountConfig,
}

/// Metadata and a decrypted configuration for active OAuth accounts.
pub struct ActiveAccountProviderConfig {
    pub id: String,
    pub provider: String,
    /// Decrypted JSON. The stored representation never leaves this repository.
    pub config: AccountConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthAccountStatus {
    pub account_id: String,
    pub provider: String,
    pub connected: bool,
    pub has_refresh_token: bool,
    pub expires_at: Option<String>,
    /// When the stored credentials were established, if the account has been
    /// connected since that field was introduced.
    pub connected_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthAccountConnection {
    pub account_id: String,
    pub provider: String,
    pub has_refresh_token: bool,
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountIdentity {
    pub email: String,
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarAccountConnection {
    pub id: String,
    pub email: String,
    pub display_name: Option<String>,
    /// Decrypted provider configuration.
    pub config: AccountConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct CalendarAccountRef {
    pub id: String,
    pub email: String,
}

#[derive(Debug, FromRow)]
struct StoredAccountConnection {
    id: String,
    email: String,
    provider: String,
    config: String,
}

#[derive(Debug, FromRow)]
struct StoredProviderConfig {
    provider: String,
    config: String,
}

#[derive(Debug, FromRow)]
struct StoredActiveProviderConfig {
    id: String,
    provider: String,
    config: String,
}

#[derive(Debug, FromRow)]
struct StoredCalendarAccountConnection {
    id: String,
    email: String,
    display_name: Option<String>,
    config: String,
}

#[derive(Clone, FromRow)]
struct StoredOAuthAccount {
    id: String,
    display_name: String,
    email: String,
    provider: String,
    config: String,
}

#[derive(serde::Deserialize)]
struct OAuthConfigSnapshot {
    oauth: Option<StoredOAuthData>,
}

#[derive(serde::Deserialize)]
struct StoredOAuthData {
    provider: Option<String>,
    refresh_token: Option<String>,
    expires_at: Option<String>,
    connected_at: Option<String>,
}

#[derive(Debug, Error)]
pub enum AccountError {
    #[error("account was not found")]
    NotFound,
    #[error("account does not use IMAP")]
    NotImap,
    #[error("account has no password to replace")]
    NoPassword,
    #[error("account configuration is invalid")]
    ConfigurationInvalid,
    #[error("account credentials could not be accessed")]
    CredentialAccess { detail: String },
    #[error("oauth provider did not return an email address")]
    OAuthIdentityMissing,
    #[error("oauth identity does not match the selected account")]
    OAuthIdentityMismatch,
    #[error("an IMAP account cannot be replaced by OAuth")]
    OAuthImapConflict,
    #[error("oauth account configuration is invalid")]
    OAuthConfiguration { detail: String },
    #[error("account persistence failed")]
    Persistence(#[source] sqlx::Error),
}

impl From<sqlx::Error> for AccountError {
    fn from(error: sqlx::Error) -> Self {
        Self::Persistence(error)
    }
}

/// Owns the durable account lifecycle and the credential-at-rest policy.
pub struct AccountRepository<'a> {
    pool: &'a SqlitePool,
}

impl<'a> AccountRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn exists(&self, account_id: &str) -> Result<bool, AccountError> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(1) FROM accounts WHERE id = ?")
            .bind(account_id)
            .fetch_one(self.pool)
            .await?;
        Ok(count > 0)
    }

    pub async fn owns_primary_identity(
        &self,
        account_id: &str,
        identity_email: &str,
    ) -> Result<bool, AccountError> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(1) FROM accounts WHERE id = ? AND lower(trim(email)) = ?",
        )
        .bind(account_id)
        .bind(identity_email)
        .fetch_one(self.pool)
        .await?;
        Ok(count > 0)
    }

    pub async fn primary_email(&self, account_id: &str) -> Result<Option<String>, AccountError> {
        sqlx::query_scalar("SELECT email FROM accounts WHERE id = ? LIMIT 1")
            .bind(account_id)
            .fetch_optional(self.pool)
            .await
            .map_err(Into::into)
    }

    pub async fn first_active_id(&self) -> Result<Option<String>, AccountError> {
        sqlx::query_scalar(
            "SELECT id FROM accounts WHERE is_active = 1 ORDER BY created_at ASC LIMIT 1",
        )
        .fetch_optional(self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn identity(&self, account_id: &str) -> Result<AccountIdentity, AccountError> {
        let row = sqlx::query_as::<_, (String, Option<String>)>(
            "SELECT email, display_name FROM accounts WHERE id = ? LIMIT 1",
        )
        .bind(account_id)
        .fetch_optional(self.pool)
        .await?
        .ok_or(AccountError::NotFound)?;
        Ok(AccountIdentity {
            email: row.0,
            display_name: row.1,
        })
    }

    pub async fn calendar_connection(
        &self,
        account_id: &str,
    ) -> Result<CalendarAccountConnection, AccountError> {
        let stored = sqlx::query_as::<_, StoredCalendarAccountConnection>(
            "SELECT id, email, display_name, config FROM accounts WHERE id = ? LIMIT 1",
        )
        .bind(account_id)
        .fetch_optional(self.pool)
        .await?
        .ok_or(AccountError::NotFound)?;
        wait_for_credentials().await?;
        let config = decrypt_account_config(&stored.id, &stored.config)
            .map_err(|detail| AccountError::CredentialAccess { detail })?;
        Ok(CalendarAccountConnection {
            id: stored.id,
            email: stored.email,
            display_name: stored.display_name,
            config,
        })
    }

    pub async fn list_active_gmail_calendar_accounts(
        &self,
        account_id: Option<&str>,
    ) -> Result<Vec<CalendarAccountRef>, AccountError> {
        match account_id {
            Some(account_id) => sqlx::query_as::<_, CalendarAccountRef>(
                "SELECT id, email FROM accounts
                 WHERE id = ? AND provider = 'gmail' AND is_active = 1",
            )
            .bind(account_id)
            .fetch_all(self.pool)
            .await
            .map_err(Into::into),
            None => sqlx::query_as::<_, CalendarAccountRef>(
                "SELECT id, email FROM accounts
                 WHERE provider = 'gmail' AND is_active = 1
                 ORDER BY created_at ASC",
            )
            .fetch_all(self.pool)
            .await
            .map_err(Into::into),
        }
    }

    pub async fn list_active(&self) -> Result<Vec<AccountSummary>, AccountError> {
        sqlx::query_as::<_, AccountSummary>(
            "SELECT id, display_name, email, provider, kind, color,
                    last_oauth_error, last_oauth_error_at, last_sync_at,
                    sync_status, last_sync_error, last_sync_error_at,
                    sync_progress_json
             FROM accounts
             WHERE is_active = 1
             ORDER BY created_at ASC",
        )
        .fetch_all(self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn create_email(
        &self,
        input: CreateEmailAccount,
    ) -> Result<AccountSummary, AccountError> {
        let id = format!("acc-{}", uuid::Uuid::new_v4());
        let now = chrono::Utc::now().to_rfc3339();
        let config = input.config.unwrap_or_else(|| serde_json::json!({}));
        wait_for_credentials().await?;
        let stored_config = encrypt_account_config(&id, &config.to_string())
            .map_err(|detail| AccountError::CredentialAccess { detail })?;

        sqlx::query_as::<_, AccountSummary>(
            "INSERT INTO accounts
                (id, display_name, email, provider, config, credentials_ref,
                 sync_state, color, is_active, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, 1, ?, ?)
             RETURNING id, display_name, email, provider, kind, color,
                       last_oauth_error, last_oauth_error_at, last_sync_at,
                       sync_status, last_sync_error, last_sync_error_at,
                       sync_progress_json",
        )
        .bind(&id)
        .bind(&input.display_name)
        .bind(&input.email)
        .bind(&input.provider)
        .bind(stored_config)
        // Vestigial schema column. Credentials live in the encrypted config.
        .bind(CREDENTIALS_REF)
        .bind("{}")
        .bind(DEFAULT_COLOR)
        .bind(&now)
        .bind(&now)
        .fetch_one(self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn clear_oauth_error(&self, account_id: &str) -> Result<(), AccountError> {
        sqlx::query(
            "UPDATE accounts
             SET last_oauth_error = NULL, last_oauth_error_at = NULL, updated_at = ?
             WHERE id = ?",
        )
        .bind(chrono::Utc::now().to_rfc3339())
        .bind(account_id)
        .execute(self.pool)
        .await?;
        Ok(())
    }

    pub async fn deactivate(&self, account_id: &str) -> Result<(), AccountError> {
        let result = sqlx::query("UPDATE accounts SET is_active = 0, updated_at = ? WHERE id = ?")
            .bind(chrono::Utc::now().to_rfc3339())
            .bind(account_id)
            .execute(self.pool)
            .await?;

        if result.rows_affected() == 0 {
            return Err(AccountError::NotFound);
        }
        Ok(())
    }

    /// Delete an account and everything that belongs to it.
    ///
    /// `deactivate` only sets `is_active = 0`: the mail vanishes from the UI —
    /// every list filters on that flag — and stays in the file, in every future
    /// backup, together with the account's encrypted credentials. `VACUUM`
    /// frees nothing, because nothing was deleted.
    ///
    /// The four tables from migration 0001 have no `ON DELETE CASCADE`, so they
    /// go by hand and in order; everything added later declares one and follows
    /// the account row. `messages` is deleted before `folders` because
    /// `pending_folder_id` (0062) points there, and the FTS index and the
    /// search projection empty themselves through their `AFTER DELETE`
    /// triggers.
    pub async fn delete(&self, account_id: &str) -> Result<(), AccountError> {
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;

        let exists: Option<i64> = sqlx::query_scalar("SELECT 1 FROM accounts WHERE id = ?")
            .bind(account_id)
            .fetch_optional(&mut *tx)
            .await?;
        if exists.is_none() {
            return Err(AccountError::NotFound);
        }

        sqlx::query(
            "DELETE FROM attachments
             WHERE message_id IN (SELECT id FROM messages WHERE account_id = ?)",
        )
        .bind(account_id)
        .execute(&mut *tx)
        .await?;

        for statement in [
            "DELETE FROM thread_tags
             WHERE thread_id IN (SELECT id FROM threads WHERE account_id = ?)",
            "DELETE FROM workflow_events
             WHERE thread_id IN (SELECT id FROM threads WHERE account_id = ?)",
            "DELETE FROM messages WHERE account_id = ?",
            "DELETE FROM folders WHERE account_id = ?",
            "DELETE FROM threads WHERE account_id = ?",
            "DELETE FROM accounts WHERE id = ?",
        ] {
            sqlx::query(statement)
                .bind(account_id)
                .execute(&mut *tx)
                .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    pub async fn load_active_imap_connection(
        &self,
        account_id: &str,
    ) -> Result<ImapAccountConnection, AccountError> {
        let stored = sqlx::query_as::<_, StoredAccountConnection>(
            "SELECT id, email, provider, config
             FROM accounts
             WHERE id = ? AND is_active = 1
             LIMIT 1",
        )
        .bind(account_id)
        .fetch_optional(self.pool)
        .await?
        .ok_or(AccountError::NotFound)?;

        if stored.provider != IMAP_PROVIDER {
            return Err(AccountError::NotImap);
        }
        wait_for_credentials().await?;
        let config = decrypt_account_config(&stored.id, &stored.config)
            .map_err(|detail| AccountError::CredentialAccess { detail })?;
        Ok(ImapAccountConnection {
            id: stored.id,
            email: stored.email,
            config,
        })
    }

    /// The account's config with `password` in place of the stored one.
    /// Only IMAP and POP3 accounts have a password; OAuth ones reconnect.
    pub async fn prepare_password_change(
        &self,
        account_id: &str,
        password: &str,
    ) -> Result<PasswordChange, AccountError> {
        let stored = sqlx::query_as::<_, StoredAccountConnection>(
            "SELECT id, email, provider, config
             FROM accounts
             WHERE id = ? AND is_active = 1
             LIMIT 1",
        )
        .bind(account_id)
        .fetch_optional(self.pool)
        .await?
        .ok_or(AccountError::NotFound)?;

        if stored.provider != IMAP_PROVIDER && stored.provider != POP3_PROVIDER {
            return Err(AccountError::NoPassword);
        }
        wait_for_credentials().await?;
        let config = decrypt_account_config(&stored.id, &stored.config)
            .map_err(|detail| AccountError::CredentialAccess { detail })?;
        let replaced = config_with_password(&stored.provider, &config, password)?;
        Ok(PasswordChange {
            account_id: stored.id,
            email: stored.email,
            provider: stored.provider,
            config: AccountConfig::from_plaintext(replaced),
        })
    }

    pub async fn store_password_change(&self, change: &PasswordChange) -> Result<(), AccountError> {
        let stored_config = encrypt_account_config(&change.account_id, &change.config)
            .map_err(|detail| AccountError::CredentialAccess { detail })?;
        let result = sqlx::query(
            "UPDATE accounts SET config = ?, updated_at = ?
             WHERE id = ? AND provider = ? AND is_active = 1",
        )
        .bind(stored_config)
        .bind(chrono::Utc::now().to_rfc3339())
        .bind(&change.account_id)
        .bind(&change.provider)
        .execute(self.pool)
        .await?;
        if result.rows_affected() == 0 {
            return Err(AccountError::NotFound);
        }
        Ok(())
    }

    /// Load provider metadata and its decrypted configuration for an account.
    ///
    /// Unlike active-connection loading, this is intentionally not limited to
    /// active accounts: vacation settings historically attempted a remote
    /// update for any existing account. A missing account is represented as
    /// `None` so callers can preserve that no-op behavior.
    pub async fn load_provider_config(
        &self,
        account_id: &str,
    ) -> Result<Option<AccountProviderConfig>, AccountError> {
        let stored = sqlx::query_as::<_, StoredProviderConfig>(
            "SELECT provider, config FROM accounts WHERE id = ? LIMIT 1",
        )
        .bind(account_id)
        .fetch_optional(self.pool)
        .await?;

        let Some(stored) = stored else {
            return Ok(None);
        };
        wait_for_credentials().await?;
        let config = decrypt_account_config(account_id, &stored.config)
            .map_err(|detail| AccountError::CredentialAccess { detail })?;
        Ok(Some(AccountProviderConfig {
            provider: stored.provider,
            config,
        }))
    }

    /// List active Gmail and Outlook accounts without exposing encrypted
    /// storage. Decryption is fail-fast, matching other account accessors.
    pub async fn list_active_oauth_configs(
        &self,
    ) -> Result<Vec<ActiveAccountProviderConfig>, AccountError> {
        let stored = sqlx::query_as::<_, StoredActiveProviderConfig>(
            "SELECT id, provider, config
             FROM accounts
             WHERE is_active = 1 AND provider IN ('gmail', 'outlook')",
        )
        .fetch_all(self.pool)
        .await?;

        if !stored.is_empty() {
            wait_for_credentials().await?;
        }
        stored
            .into_iter()
            .map(|account| {
                let config = decrypt_account_config(&account.id, &account.config)
                    .map_err(|detail| AccountError::CredentialAccess { detail })?;
                Ok(ActiveAccountProviderConfig {
                    id: account.id,
                    provider: account.provider,
                    config,
                })
            })
            .collect()
    }

    pub async fn oauth_status(&self, account_id: &str) -> Result<OAuthAccountStatus, AccountError> {
        let stored = load_stored_oauth_account(self.pool, account_id)
            .await?
            .ok_or(AccountError::NotFound)?;
        wait_for_credentials().await?;
        let config = decrypt_oauth_account(stored)?;
        let snapshot = serde_json::from_str::<OAuthConfigSnapshot>(&config.config)
            .unwrap_or(OAuthConfigSnapshot { oauth: None });
        let provider = snapshot
            .oauth
            .as_ref()
            .and_then(|oauth| oauth.provider.clone())
            .unwrap_or(config.provider);

        Ok(OAuthAccountStatus {
            account_id: config.id,
            provider,
            connected: snapshot.oauth.is_some(),
            has_refresh_token: snapshot
                .oauth
                .as_ref()
                .and_then(|oauth| oauth.refresh_token.as_deref())
                .is_some_and(|token| !token.trim().is_empty()),
            connected_at: snapshot
                .oauth
                .as_ref()
                .and_then(|oauth| oauth.connected_at.clone()),
            expires_at: snapshot.oauth.and_then(|oauth| oauth.expires_at),
        })
    }

    /// Attach an OAuth identity to the selected placeholder or an existing
    /// provider account. Every config read is decrypted and every config write
    /// is encrypted; account mutation and placeholder retirement commit as one
    /// transaction.
    pub async fn connect_oauth(
        &self,
        session_account_id: &str,
        provider: OAuthProvider,
        token: &OAuthTokenPayload,
        identity: &OAuthIdentity,
        now: &str,
    ) -> Result<OAuthAccountConnection, AccountError> {
        let normalized_email = identity.email.trim().to_lowercase();
        if normalized_email.is_empty() {
            return Err(AccountError::OAuthIdentityMissing);
        }

        wait_for_credentials().await?;
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let session_account =
            load_stored_oauth_account_in_transaction(&mut transaction, session_account_id)
                .await?
                .ok_or(AccountError::NotFound)
                .and_then(decrypt_oauth_account)?;

        if !session_account.email.trim().is_empty()
            && !oauth_emails_match(provider, &session_account.email, &normalized_email)
        {
            return Err(AccountError::OAuthIdentityMismatch);
        }

        let matching = sqlx::query_as::<_, StoredOAuthAccount>(
            "SELECT id, display_name, email, provider, config
             FROM accounts
             WHERE lower(email) = lower(?) AND is_active = 1
             LIMIT 1",
        )
        .bind(&normalized_email)
        .fetch_optional(&mut *transaction)
        .await?
        .map(decrypt_oauth_account)
        .transpose()?;

        let result = if let Some(existing) = matching {
            ensure_oauth_compatible(&existing)?;
            let connection = update_oauth_account(
                &mut transaction,
                existing,
                provider,
                token,
                identity,
                &normalized_email,
                now,
            )
            .await?;
            retire_oauth_placeholder(
                &mut transaction,
                &session_account,
                &connection.account_id,
                now,
            )
            .await?;
            connection
        } else if oauth_emails_match(provider, &session_account.email, &normalized_email) {
            ensure_oauth_compatible(&session_account)?;
            update_oauth_account(
                &mut transaction,
                session_account,
                provider,
                token,
                identity,
                &normalized_email,
                now,
            )
            .await?
        } else {
            let connection = insert_oauth_account(
                &mut transaction,
                &session_account,
                provider,
                token,
                identity,
                &normalized_email,
                now,
            )
            .await?;
            retire_oauth_placeholder(
                &mut transaction,
                &session_account,
                &connection.account_id,
                now,
            )
            .await?;
            connection
        };

        transaction.commit().await?;
        Ok(result)
    }
}

async fn load_stored_oauth_account(
    pool: &SqlitePool,
    account_id: &str,
) -> Result<Option<StoredOAuthAccount>, AccountError> {
    sqlx::query_as::<_, StoredOAuthAccount>(
        "SELECT id, display_name, email, provider, config
         FROM accounts WHERE id = ? LIMIT 1",
    )
    .bind(account_id)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

async fn load_stored_oauth_account_in_transaction(
    transaction: &mut Transaction<'_, Sqlite>,
    account_id: &str,
) -> Result<Option<StoredOAuthAccount>, AccountError> {
    sqlx::query_as::<_, StoredOAuthAccount>(
        "SELECT id, display_name, email, provider, config
         FROM accounts WHERE id = ? LIMIT 1",
    )
    .bind(account_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(Into::into)
}

async fn wait_for_credentials() -> Result<(), AccountError> {
    crate::utils::crypto::wait_for_master_key()
        .await
        .map_err(|detail| AccountError::CredentialAccess { detail })
}

fn decrypt_oauth_account(
    mut account: StoredOAuthAccount,
) -> Result<StoredOAuthAccount, AccountError> {
    account.config = decrypt_account_config(&account.id, &account.config)
        .map_err(|detail| AccountError::CredentialAccess { detail })?
        .into_string();
    Ok(account)
}

fn encrypt_oauth_config(account_id: &str, config: &str) -> Result<String, AccountError> {
    encrypt_account_config(account_id, config)
        .map_err(|detail| AccountError::CredentialAccess { detail })
}

fn oauth_config_error(detail: String) -> AccountError {
    AccountError::OAuthConfiguration { detail }
}

async fn update_oauth_account(
    transaction: &mut Transaction<'_, Sqlite>,
    account: StoredOAuthAccount,
    provider: OAuthProvider,
    token: &OAuthTokenPayload,
    identity: &OAuthIdentity,
    normalized_email: &str,
    now: &str,
) -> Result<OAuthAccountConnection, AccountError> {
    let config = oauth::merge_token_into_config(&account.config, provider, token)
        .map_err(oauth_config_error)?;
    let stored_config = encrypt_oauth_config(&account.id, &config)?;
    let display_name =
        resolved_oauth_display_name(identity, &account.display_name, normalized_email);
    sqlx::query(
        "UPDATE accounts
         SET display_name = ?, email = ?, provider = ?, config = ?, updated_at = ?,
             last_oauth_error = NULL, last_oauth_error_at = NULL
         WHERE id = ?",
    )
    .bind(display_name)
    .bind(normalized_email)
    .bind(provider.as_str())
    .bind(stored_config)
    .bind(now)
    .bind(&account.id)
    .execute(&mut **transaction)
    .await?;

    oauth_connection(&account.id, provider, &config)
}

async fn insert_oauth_account(
    transaction: &mut Transaction<'_, Sqlite>,
    session_account: &StoredOAuthAccount,
    provider: OAuthProvider,
    token: &OAuthTokenPayload,
    identity: &OAuthIdentity,
    normalized_email: &str,
    now: &str,
) -> Result<OAuthAccountConnection, AccountError> {
    let id = format!("acc-{}", uuid::Uuid::new_v4());
    let config =
        oauth::merge_token_into_config("{}", provider, token).map_err(oauth_config_error)?;
    let stored_config = encrypt_oauth_config(&id, &config)?;
    let display_name =
        resolved_oauth_display_name(identity, &session_account.display_name, normalized_email);
    sqlx::query(
        "INSERT INTO accounts
         (id, display_name, email, provider, config, credentials_ref, sync_state, color,
          is_active, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, 1, ?, ?)",
    )
    .bind(&id)
    .bind(display_name)
    .bind(normalized_email)
    .bind(provider.as_str())
    .bind(stored_config)
    .bind(CREDENTIALS_REF)
    .bind("{}")
    .bind(DEFAULT_COLOR)
    .bind(now)
    .bind(now)
    .execute(&mut **transaction)
    .await?;

    oauth_connection(&id, provider, &config)
}

fn oauth_connection(
    account_id: &str,
    provider: OAuthProvider,
    config: &str,
) -> Result<OAuthAccountConnection, AccountError> {
    let stored = oauth::load_stored_oauth(config).map_err(oauth_config_error)?;
    Ok(OAuthAccountConnection {
        account_id: account_id.to_string(),
        provider: provider.as_str().to_string(),
        has_refresh_token: stored
            .refresh_token
            .as_deref()
            .is_some_and(|token| !token.trim().is_empty()),
        expires_at: stored.expires_at,
    })
}

fn ensure_oauth_compatible(account: &StoredOAuthAccount) -> Result<(), AccountError> {
    if account.provider.eq_ignore_ascii_case(IMAP_PROVIDER)
        || serde_json::from_str::<serde_json::Value>(&account.config)
            .ok()
            .and_then(|value| value.get("imap").cloned())
            .is_some()
    {
        return Err(AccountError::OAuthImapConflict);
    }
    Ok(())
}

async fn retire_oauth_placeholder(
    transaction: &mut Transaction<'_, Sqlite>,
    session_account: &StoredOAuthAccount,
    retained_account_id: &str,
    now: &str,
) -> Result<(), AccountError> {
    if session_account.id == retained_account_id {
        return Ok(());
    }
    let message_count: i64 =
        sqlx::query_scalar("SELECT COUNT(1) FROM messages WHERE account_id = ?")
            .bind(&session_account.id)
            .fetch_one(&mut **transaction)
            .await?;
    let has_oauth = serde_json::from_str::<OAuthConfigSnapshot>(&session_account.config)
        .ok()
        .and_then(|snapshot| snapshot.oauth)
        .is_some();
    if message_count > 0 || has_oauth {
        return Ok(());
    }
    sqlx::query("UPDATE accounts SET is_active = 0, updated_at = ? WHERE id = ?")
        .bind(now)
        .bind(&session_account.id)
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

fn oauth_emails_match(provider: OAuthProvider, left: &str, right: &str) -> bool {
    canonical_oauth_email(provider, left) == canonical_oauth_email(provider, right)
}

fn canonical_oauth_email(provider: OAuthProvider, email: &str) -> String {
    let normalized = email.trim().to_lowercase();
    let Some((local_part, domain)) = normalized.rsplit_once('@') else {
        return normalized;
    };
    match provider {
        OAuthProvider::Gmail if matches!(domain, "gmail.com" | "googlemail.com") => {
            format!("{}@gmail.com", local_part.replace('.', ""))
        }
        _ => normalized,
    }
}

fn resolved_oauth_display_name(
    identity: &OAuthIdentity,
    fallback_display_name: &str,
    email: &str,
) -> String {
    identity
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            let fallback = fallback_display_name.trim();
            (!fallback.is_empty()).then_some(fallback.to_string())
        })
        .unwrap_or_else(|| {
            email
                .split('@')
                .next()
                .filter(|value| !value.is_empty())
                .unwrap_or("OAuth Account")
                .to_string()
        })
}

/// Put `password` where the incoming server reads it. SMTP follows where it
/// reused the old password; one of its own stays, and none means it already
/// falls back to the incoming password.
fn config_with_password(
    provider: &str,
    config: &str,
    password: &str,
) -> Result<String, AccountError> {
    let mut value: serde_json::Value = if config.trim().is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(config).map_err(|_| AccountError::ConfigurationInvalid)?
    };
    if !value.is_object() {
        return Err(AccountError::ConfigurationInvalid);
    }
    // POP3 always nests; IMAP is nested or flat, the shapes `ImapProvider` reads.
    let incoming = if provider == POP3_PROVIDER {
        "/pop3"
    } else if value.get("imap").is_some_and(serde_json::Value::is_object) {
        "/imap"
    } else {
        ""
    };
    let old = value
        .pointer(&format!("{incoming}/password"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);

    let mut targets = vec![(incoming.to_string(), "password")];
    for (parent, field) in [
        ("/smtp".to_string(), "password"),
        (format!("{incoming}/smtp"), "password"),
        (String::new(), "smtp_password"),
    ] {
        let current = value
            .pointer(&format!("{parent}/{field}"))
            .and_then(serde_json::Value::as_str);
        if current.is_some()
            && current == old.as_deref()
            && !targets.contains(&(parent.clone(), field))
        {
            targets.push((parent, field));
        }
    }
    for (parent, field) in targets {
        value
            .pointer_mut(&parent)
            .and_then(serde_json::Value::as_object_mut)
            .ok_or(AccountError::ConfigurationInvalid)?
            .insert(field.to_string(), serde_json::Value::from(password));
    }
    Ok(value.to_string())
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
            .expect("memory db");
        crate::db::init(&pool).await.expect("migrations");
        pool
    }

    /// Seed one account with a row in every table that hangs off it.
    async fn seed_account_with_dependents(pool: &SqlitePool, id: &str, email: &str) {
        let now = "2026-01-01T00:00:00Z";
        sqlx::query(
            "INSERT INTO accounts
             (id, display_name, email, provider, config, credentials_ref, is_active,
              created_at, updated_at)
             VALUES (?, 'A', ?, 'imap', '{}', '', 1, ?, ?)",
        )
        .bind(id)
        .bind(email)
        .bind(now)
        .bind(now)
        .execute(pool)
        .await
        .expect("account");
        sqlx::query(
            "INSERT INTO folders (id, account_id, name, full_path, folder_type, created_at, updated_at)
             VALUES (?, ?, 'Inbox', 'INBOX', 'inbox', ?, ?)",
        )
        .bind(format!("{id}-folder"))
        .bind(id)
        .bind(now)
        .bind(now)
        .execute(pool)
        .await
        .expect("folder");
        sqlx::query(
            "INSERT INTO threads
             (id, account_id, subject, last_message_date, message_count, unread_count,
              created_at, updated_at)
             VALUES (?, ?, 'Subject', ?, 1, 0, ?, ?)",
        )
        .bind(format!("{id}-thread"))
        .bind(id)
        .bind(now)
        .bind(now)
        .bind(now)
        .execute(pool)
        .await
        .expect("thread");
        for suffix in ["m1", "m2"] {
            sqlx::query(
                "INSERT INTO messages
                 (id, account_id, folder_id, thread_id, from_address, to_addresses, subject,
                  body_text, date, is_read, created_at, updated_at)
                 VALUES (?, ?, ?, ?, 'sender@example.test', '[]', 'Betreff', 'Zebrafisch',
                         ?, 0, ?, ?)",
            )
            .bind(format!("{id}-{suffix}"))
            .bind(id)
            .bind(format!("{id}-folder"))
            .bind(format!("{id}-thread"))
            .bind(now)
            .bind(now)
            .bind(now)
            .execute(pool)
            .await
            .expect("message");
        }
        sqlx::query(
            "INSERT INTO attachments
             (id, message_id, filename, content_type, size, is_inline, created_at)
             VALUES (?, ?, 'bericht.pdf', 'application/pdf', 12, 0, ?)",
        )
        .bind(format!("{id}-att"))
        .bind(format!("{id}-m1"))
        .bind(now)
        .execute(pool)
        .await
        .expect("attachment");
        sqlx::query(
            "INSERT INTO workflow_events (id, thread_id, from_state, to_state, reason, created_at)
             VALUES (?, ?, 'inbox', 'done', 'test', ?)",
        )
        .bind(format!("{id}-event"))
        .bind(format!("{id}-thread"))
        .bind(now)
        .execute(pool)
        .await
        .expect("workflow event");
    }

    async fn count_for(pool: &SqlitePool, table: &str, column: &str, value: &str) -> i64 {
        sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table} WHERE {column} = ?"))
            .bind(value)
            .fetch_one(pool)
            .await
            .unwrap_or_else(|err| panic!("count {table}: {err}"))
    }

    /// Removing an account used to leave every byte of it in the file.
    #[tokio::test]
    async fn deleting_an_account_removes_every_dependent_row() {
        let pool = pool().await;
        seed_account_with_dependents(&pool, "acc-gone", "gone@example.test").await;

        let indexed_before: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM messages_fts WHERE messages_fts MATCH ?")
                .bind("Zebrafisch")
                .fetch_one(&pool)
                .await
                .expect("fts before");
        assert_eq!(indexed_before, 2);

        AccountRepository::new(&pool)
            .delete("acc-gone")
            .await
            .expect("delete account");

        for (table, column) in [
            ("accounts", "id"),
            ("messages", "account_id"),
            ("threads", "account_id"),
            ("folders", "account_id"),
            ("message_search_projection", "account_id"),
        ] {
            assert_eq!(
                count_for(&pool, table, column, "acc-gone").await,
                0,
                "{table} still holds rows"
            );
        }
        assert_eq!(
            count_for(&pool, "attachments", "message_id", "acc-gone-m1").await,
            0
        );
        assert_eq!(
            count_for(&pool, "workflow_events", "thread_id", "acc-gone-thread").await,
            0
        );

        let indexed_after: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM messages_fts WHERE messages_fts MATCH ?")
                .bind("Zebrafisch")
                .fetch_one(&pool)
                .await
                .expect("fts after");
        assert_eq!(
            indexed_after, 0,
            "the search index still answers for the account"
        );
    }

    #[tokio::test]
    async fn deleting_one_account_leaves_the_other_untouched() {
        let pool = pool().await;
        seed_account_with_dependents(&pool, "acc-gone", "gone@example.test").await;
        seed_account_with_dependents(&pool, "acc-kept", "kept@example.test").await;

        AccountRepository::new(&pool)
            .delete("acc-gone")
            .await
            .expect("delete account");

        assert_eq!(count_for(&pool, "accounts", "id", "acc-kept").await, 1);
        assert_eq!(
            count_for(&pool, "messages", "account_id", "acc-kept").await,
            2
        );
        assert_eq!(
            count_for(&pool, "attachments", "message_id", "acc-kept-m1").await,
            1
        );
        assert_eq!(
            count_for(&pool, "message_search_projection", "account_id", "acc-kept").await,
            2
        );
    }

    #[tokio::test]
    async fn deleting_an_unknown_account_is_not_found() {
        let pool = pool().await;
        assert!(matches!(
            AccountRepository::new(&pool).delete("acc-nope").await,
            Err(AccountError::NotFound)
        ));
    }

    #[tokio::test]
    async fn create_encrypts_config_at_rest() {
        crate::utils::crypto::seed_master_key_for_tests();
        let pool = pool().await;
        let secret = serde_json::json!({
            "imap": {
                "host": "imap.example.test",
                "password": "not-readable-at-rest"
            }
        });

        let created = AccountRepository::new(&pool)
            .create_email(CreateEmailAccount {
                display_name: "Personal".into(),
                email: "person@example.test".into(),
                provider: "imap".into(),
                config: Some(secret.clone()),
            })
            .await
            .expect("create account");
        let stored: String = sqlx::query_scalar("SELECT config FROM accounts WHERE id = ?")
            .bind(&created.id)
            .fetch_one(&pool)
            .await
            .expect("stored config");

        assert!(stored.starts_with("enc:v2:"));
        assert!(!stored.contains("not-readable-at-rest"));
        assert!(created.id.starts_with("acc-"));
        assert_eq!(created.kind, "email");
        assert_eq!(created.color.as_deref(), Some(DEFAULT_COLOR));
        assert_eq!(created.sync_status.as_deref(), Some("idle"));
        assert_eq!(created.last_oauth_error, None);

        let defaults: (String, String, String, i64, String, String) = sqlx::query_as(
            "SELECT credentials_ref, sync_state, color, is_active, kind, sync_status
             FROM accounts WHERE id = ?",
        )
        .bind(&created.id)
        .fetch_one(&pool)
        .await
        .expect("stored defaults");
        assert_eq!(
            defaults,
            (
                CREDENTIALS_REF.to_string(),
                "{}".to_string(),
                DEFAULT_COLOR.to_string(),
                1,
                "email".to_string(),
                "idle".to_string(),
            )
        );

        let connection = AccountRepository::new(&pool)
            .load_active_imap_connection(&created.id)
            .await
            .expect("connection account");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&connection.config).expect("plain config"),
            secret
        );
    }

    fn replaced(provider: &str, config: serde_json::Value) -> serde_json::Value {
        let next = config_with_password(provider, &config.to_string(), "new-secret")
            .expect("replace password");
        serde_json::from_str(&next).expect("json")
    }

    #[test]
    fn password_replacement_follows_every_config_shape() {
        // Nested IMAP, SMTP sharing the old password: both move.
        assert_eq!(
            replaced(
                "imap",
                serde_json::json!({
                    "imap": { "host": "imap.example.test", "password": "old" },
                    "smtp": { "host": "smtp.example.test", "password": "old" }
                })
            ),
            serde_json::json!({
                "imap": { "host": "imap.example.test", "password": "new-secret" },
                "smtp": { "host": "smtp.example.test", "password": "new-secret" }
            })
        );
        // An SMTP password of its own stays.
        assert_eq!(
            replaced(
                "imap",
                serde_json::json!({
                    "imap": { "password": "old", "smtp": { "password": "relay" } }
                })
            ),
            serde_json::json!({
                "imap": { "password": "new-secret", "smtp": { "password": "relay" } }
            })
        );
        // Flat IMAP with the SMTP password beside it.
        assert_eq!(
            replaced(
                "imap",
                serde_json::json!({ "host": "h", "password": "old", "smtp_password": "old" })
            ),
            serde_json::json!({ "host": "h", "password": "new-secret", "smtp_password": "new-secret" })
        );
        // POP3 is always nested; SMTP without a password already falls back.
        assert_eq!(
            replaced(
                "pop3",
                serde_json::json!({ "pop3": { "host": "h", "password": "old" }, "smtp": { "host": "s" } })
            ),
            serde_json::json!({ "pop3": { "host": "h", "password": "new-secret" }, "smtp": { "host": "s" } })
        );
        assert!(matches!(
            config_with_password("pop3", "{}", "x"),
            Err(AccountError::ConfigurationInvalid)
        ));
    }

    #[tokio::test]
    async fn password_change_is_stored_encrypted_and_refused_for_oauth_accounts() {
        crate::utils::crypto::seed_master_key_for_tests();
        let pool = pool().await;
        let repository = AccountRepository::new(&pool);
        let created = repository
            .create_email(CreateEmailAccount {
                display_name: "Work".into(),
                email: "person@example.test".into(),
                provider: "imap".into(),
                config: Some(serde_json::json!({
                    "imap": { "host": "imap.example.test", "password": "rejected" }
                })),
            })
            .await
            .expect("create account");

        let change = repository
            .prepare_password_change(&created.id, "accepted")
            .await
            .expect("prepare change");
        assert_eq!(change.provider, "imap");
        assert_eq!(change.email, "person@example.test");
        repository
            .store_password_change(&change)
            .await
            .expect("store change");

        let stored: String = sqlx::query_scalar("SELECT config FROM accounts WHERE id = ?")
            .bind(&created.id)
            .fetch_one(&pool)
            .await
            .expect("stored config");
        assert!(stored.starts_with("enc:v2:"));
        assert!(!stored.contains("accepted"));
        let connection = repository
            .load_active_imap_connection(&created.id)
            .await
            .expect("connection");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&connection.config).expect("json")["imap"]
                ["password"],
            "accepted"
        );

        let oauth = repository
            .create_email(CreateEmailAccount {
                display_name: "Mail".into(),
                email: "person@gmail.example".into(),
                provider: "gmail".into(),
                config: None,
            })
            .await
            .expect("create oauth account");
        assert!(matches!(
            repository.prepare_password_change(&oauth.id, "x").await,
            Err(AccountError::NoPassword)
        ));
    }

    #[tokio::test]
    async fn create_without_config_returns_an_empty_plaintext_view() {
        crate::utils::crypto::seed_master_key_for_tests();
        let pool = pool().await;

        let created = AccountRepository::new(&pool)
            .create_email(CreateEmailAccount {
                display_name: "Empty".into(),
                email: "empty@example.test".into(),
                provider: "imap".into(),
                config: None,
            })
            .await
            .expect("create account");
        let connection = AccountRepository::new(&pool)
            .load_active_imap_connection(&created.id)
            .await
            .expect("connection account");

        assert_eq!(connection.config.as_str(), "{}");
    }

    #[tokio::test]
    async fn provider_config_decrypts_stored_credentials_and_returns_none_for_missing_accounts() {
        crate::utils::crypto::seed_master_key_for_tests();
        let pool = pool().await;
        let plain_config = r#"{"oauth":{"refresh_token":"secret-refresh-token"}}"#;
        let encrypted_config =
            encrypt_account_config("acc-provider", plain_config).expect("encrypt config");
        sqlx::query(
            "INSERT INTO accounts
                (id, display_name, email, provider, config, credentials_ref,
                 sync_state, color, is_active, created_at, updated_at)
             VALUES ('acc-provider', 'Provider', 'provider@example.test', 'gmail', ?,
                     'encrypted-in-config', '{}', '#123456', 1, '2026-01-01', '2026-01-01')",
        )
        .bind(&encrypted_config)
        .execute(&pool)
        .await
        .expect("provider account");

        let provider_config = AccountRepository::new(&pool)
            .load_provider_config("acc-provider")
            .await
            .expect("provider config")
            .expect("existing provider account");
        assert_eq!(provider_config.provider, "gmail");
        assert_eq!(provider_config.config.as_str(), plain_config);

        let missing = AccountRepository::new(&pool)
            .load_provider_config("missing-account")
            .await
            .expect("missing provider config");
        assert_eq!(missing, None);
    }

    #[tokio::test]
    async fn active_oauth_config_listing_decrypts_config_and_fails_on_bad_ciphertext() {
        crate::utils::crypto::seed_master_key_for_tests();
        let pool = pool().await;
        let plain_config = r#"{"oauth":{"refresh_token":"secret-refresh-token"}}"#;
        let encrypted_config =
            encrypt_account_config("active-oauth", plain_config).expect("encrypt config");
        sqlx::query(
            "INSERT INTO accounts
                (id, display_name, email, provider, config, credentials_ref,
                 sync_state, color, is_active, created_at, updated_at)
             VALUES ('active-oauth', 'Provider', 'provider@example.test', 'gmail', ?,
                     'encrypted-in-config', '{}', '#123456', 1, '2026-01-01', '2026-01-01')",
        )
        .bind(&encrypted_config)
        .execute(&pool)
        .await
        .expect("provider account");

        let configs = AccountRepository::new(&pool)
            .list_active_oauth_configs()
            .await
            .expect("list oauth configs");
        assert_eq!(configs.len(), 1);
        assert_eq!(configs[0].id, "active-oauth");
        assert_eq!(configs[0].provider, "gmail");
        assert_eq!(configs[0].config.as_str(), plain_config);

        sqlx::query(
            "INSERT INTO accounts
                (id, display_name, email, provider, config, credentials_ref,
                 sync_state, color, is_active, created_at, updated_at)
             VALUES ('bad-oauth', 'Bad', 'bad@example.test', 'outlook', 'enc:v1:AAAA:BBBB',
                     'encrypted-in-config', '{}', '#123456', 1, '2026-01-01', '2026-01-01')",
        )
        .execute(&pool)
        .await
        .expect("bad provider account");
        let result = AccountRepository::new(&pool)
            .list_active_oauth_configs()
            .await;
        assert!(matches!(result, Err(AccountError::CredentialAccess { .. })));
    }

    #[tokio::test]
    async fn existence_and_primary_identity_checks_are_account_scoped() {
        let pool = pool().await;
        sqlx::query(
            "INSERT INTO accounts
             (id, display_name, email, provider, config, credentials_ref, sync_state,
              color, is_active, created_at, updated_at)
             VALUES ('identity', 'Identity', ' Person@Example.Test ', 'imap', '{}', '', '{}',
                     '#123456', 1, '2026-01-01', '2026-01-01')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let repository = AccountRepository::new(&pool);
        assert!(repository.exists("identity").await.unwrap());
        assert!(!repository.exists("missing").await.unwrap());
        assert!(repository
            .owns_primary_identity("identity", "person@example.test")
            .await
            .unwrap());
        assert!(!repository
            .owns_primary_identity("missing", "person@example.test")
            .await
            .unwrap());
        assert_eq!(
            repository
                .primary_email("identity")
                .await
                .unwrap()
                .as_deref(),
            Some(" Person@Example.Test ")
        );
        assert_eq!(repository.primary_email("missing").await.unwrap(), None);
    }

    #[tokio::test]
    async fn calendar_accessors_keep_account_selection_and_credentials_in_the_repository() {
        crate::utils::crypto::seed_master_key_for_tests();
        let pool = pool().await;
        for (id, provider, created_at) in [
            ("gmail-first", "gmail", "2026-01-01"),
            ("imap", "imap", "2026-01-02"),
            ("gmail-last", "gmail", "2026-01-03"),
        ] {
            let config = encrypt_account_config(id, &format!(r#"{{"id":"{id}"}}"#)).unwrap();
            sqlx::query(
                "INSERT INTO accounts
                    (id, display_name, email, provider, config, credentials_ref, sync_state,
                     color, is_active, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, '', '{}', '#123456', 1, ?, ?)",
            )
            .bind(id)
            .bind(id)
            .bind(format!("{id}@example.test"))
            .bind(provider)
            .bind(config)
            .bind(created_at)
            .bind(created_at)
            .execute(&pool)
            .await
            .unwrap();
        }
        let repository = AccountRepository::new(&pool);
        assert_eq!(
            repository.first_active_id().await.unwrap().as_deref(),
            Some("gmail-first")
        );
        assert_eq!(
            repository.identity("imap").await.unwrap().email,
            "imap@example.test"
        );
        assert_eq!(
            repository
                .calendar_connection("imap")
                .await
                .unwrap()
                .config
                .as_str(),
            r#"{"id":"imap"}"#
        );
        let gmail = repository
            .list_active_gmail_calendar_accounts(None)
            .await
            .unwrap();
        assert_eq!(
            gmail
                .iter()
                .map(|account| account.id.as_str())
                .collect::<Vec<_>>(),
            vec!["gmail-first", "gmail-last"]
        );
        assert_eq!(
            repository
                .list_active_gmail_calendar_accounts(Some("gmail-last"))
                .await
                .unwrap()
                .len(),
            1
        );
        assert!(repository
            .list_active_gmail_calendar_accounts(Some("imap"))
            .await
            .unwrap()
            .is_empty());
    }

    async fn insert_oauth_fixture(pool: &SqlitePool, id: &str, email: &str, config: &str) {
        let stored = encrypt_account_config(id, config).unwrap();
        sqlx::query(
            "INSERT INTO accounts
             (id, display_name, email, provider, config, credentials_ref, sync_state,
              color, is_active, created_at, updated_at)
             VALUES (?, 'Existing', ?, 'gmail', ?, 'encrypted-in-config', '{}',
                     '#123456', 1, '2026-01-01', '2026-01-01')",
        )
        .bind(id)
        .bind(email)
        .bind(stored)
        .execute(pool)
        .await
        .unwrap();
    }

    fn oauth_config(refresh_token: &str, access_token: &str) -> String {
        serde_json::json!({
            "oauth": {
                "provider": "gmail",
                "client_id": "client",
                "redirect_uri": "http://127.0.0.1/oauth/callback",
                "access_token": access_token,
                "refresh_token": refresh_token,
                "expires_at": "2026-01-01T00:00:00Z"
            }
        })
        .to_string()
    }

    fn oauth_token(refresh_token: Option<&str>) -> OAuthTokenPayload {
        OAuthTokenPayload {
            access_token: "new-access".into(),
            refresh_token: refresh_token.map(str::to_string),
            token_type: Some("Bearer".into()),
            scope: Some("mail".into()),
            expires_at: Some("2027-01-01T00:00:00Z".into()),
        }
    }

    fn oauth_identity(email: &str) -> OAuthIdentity {
        OAuthIdentity {
            email: email.into(),
            display_name: Some("OAuth User".into()),
        }
    }

    #[test]
    fn oauth_email_matching_preserves_provider_specific_identity_rules() {
        assert!(oauth_emails_match(
            OAuthProvider::Gmail,
            "sample.user@googlemail.com",
            "SampleUser@GMAIL.com",
        ));
        assert!(!oauth_emails_match(
            OAuthProvider::Outlook,
            "sample.user@outlook.com",
            "sampleuser@outlook.com",
        ));
    }

    #[tokio::test]
    async fn oauth_update_encrypts_config_and_preserves_omitted_refresh_token() {
        crate::utils::crypto::seed_master_key_for_tests();
        let pool = pool().await;
        insert_oauth_fixture(
            &pool,
            "existing",
            "person@gmail.com",
            &oauth_config("old-refresh", "old-access"),
        )
        .await;

        let connection = AccountRepository::new(&pool)
            .connect_oauth(
                "existing",
                OAuthProvider::Gmail,
                &oauth_token(None),
                &oauth_identity("Person@Gmail.com"),
                "2026-02-01",
            )
            .await
            .unwrap();
        assert!(connection.has_refresh_token);

        let stored: String =
            sqlx::query_scalar("SELECT config FROM accounts WHERE id = 'existing'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(stored.starts_with("enc:v2:"));
        assert!(!stored.contains("old-refresh"));
        let plain = decrypt_account_config("existing", &stored).unwrap();
        let oauth = oauth::load_stored_oauth(&plain).unwrap();
        assert_eq!(oauth.refresh_token.as_deref(), Some("old-refresh"));
        assert_eq!(oauth.access_token.as_deref(), Some("new-access"));
    }

    #[tokio::test]
    async fn oauth_matching_reuses_account_and_retires_empty_placeholder_atomically() {
        crate::utils::crypto::seed_master_key_for_tests();
        let pool = pool().await;
        insert_oauth_fixture(&pool, "placeholder", "", "{}").await;
        insert_oauth_fixture(
            &pool,
            "existing",
            "person@gmail.com",
            &oauth_config("old-refresh", "old-access"),
        )
        .await;

        let connection = AccountRepository::new(&pool)
            .connect_oauth(
                "placeholder",
                OAuthProvider::Gmail,
                &oauth_token(Some("new-refresh")),
                &oauth_identity("person@gmail.com"),
                "2026-02-01",
            )
            .await
            .unwrap();
        assert_eq!(connection.account_id, "existing");
        let active: i64 =
            sqlx::query_scalar("SELECT is_active FROM accounts WHERE id = 'placeholder'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(active, 0);
    }

    #[tokio::test]
    async fn oauth_placeholder_failure_rolls_back_existing_account_update() {
        crate::utils::crypto::seed_master_key_for_tests();
        let pool = pool().await;
        insert_oauth_fixture(&pool, "placeholder", "", "{}").await;
        let original = oauth_config("old-refresh", "old-access");
        insert_oauth_fixture(&pool, "existing", "person@gmail.com", &original).await;
        sqlx::query(
            "CREATE TRIGGER fail_placeholder_retirement
             BEFORE UPDATE ON accounts
             WHEN OLD.id = 'placeholder'
             BEGIN SELECT RAISE(ABORT, 'retirement failed'); END",
        )
        .execute(&pool)
        .await
        .unwrap();

        let result = AccountRepository::new(&pool)
            .connect_oauth(
                "placeholder",
                OAuthProvider::Gmail,
                &oauth_token(Some("new-refresh")),
                &oauth_identity("person@gmail.com"),
                "2026-02-01",
            )
            .await;
        assert!(matches!(result, Err(AccountError::Persistence(_))));
        let stored: String =
            sqlx::query_scalar("SELECT config FROM accounts WHERE id = 'existing'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            decrypt_account_config("existing", &stored)
                .unwrap()
                .as_str(),
            original
        );
    }

    #[tokio::test]
    async fn oauth_identity_mismatch_does_not_mutate_the_selected_account() {
        crate::utils::crypto::seed_master_key_for_tests();
        let pool = pool().await;
        let original = oauth_config("old-refresh", "old-access");
        insert_oauth_fixture(&pool, "selected", "first@gmail.com", &original).await;

        let result = AccountRepository::new(&pool)
            .connect_oauth(
                "selected",
                OAuthProvider::Gmail,
                &oauth_token(Some("new-refresh")),
                &oauth_identity("other@gmail.com"),
                "2026-02-01",
            )
            .await;
        assert!(matches!(result, Err(AccountError::OAuthIdentityMismatch)));
        let stored: String =
            sqlx::query_scalar("SELECT config FROM accounts WHERE id = 'selected'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            decrypt_account_config("selected", &stored)
                .unwrap()
                .as_str(),
            original
        );
    }
}
