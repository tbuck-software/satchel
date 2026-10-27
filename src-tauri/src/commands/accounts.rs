use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::providers::{imap::ImapProvider, pop3::Pop3Provider, traits::MailProvider};
use crate::sync::idle::IdleControl;
use crate::sync::queue::SyncQueue;
use satchel_core::db::accounts::{
    AccountError, AccountRepository, AccountSummary, CreateEmailAccount, PasswordChange,
};

use super::error::{CommandError, CommandResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/accounts/")
)]
pub struct AccountDto {
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

impl From<AccountSummary> for AccountDto {
    fn from(account: AccountSummary) -> Self {
        Self {
            id: account.id,
            display_name: account.display_name,
            email: account.email,
            provider: account.provider,
            kind: account.kind,
            color: account.color,
            last_oauth_error: account.last_oauth_error,
            last_oauth_error_at: account.last_oauth_error_at,
            last_sync_at: account.last_sync_at,
            sync_status: account.sync_status,
            last_sync_error: account.last_sync_error,
            last_sync_error_at: account.last_sync_error_at,
            sync_progress_json: account.sync_progress_json,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/accounts/", optional_fields = nullable))]
pub struct NewAccount {
    pub display_name: String,
    pub email: String,
    pub provider: String,
    pub config: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/accounts/")
)]
pub struct ImapAccountConnectionDto {
    pub account_id: String,
    pub ok: bool,
    pub folders: usize,
    pub folder_sample: Vec<String>,
}

#[tauri::command]
pub async fn list_accounts(
    pool: tauri::State<'_, crate::db::ReadPool>,
) -> CommandResult<Vec<AccountDto>> {
    AccountRepository::new(pool.inner().as_ref())
        .list_active()
        .await
        .map(|accounts| accounts.into_iter().map(Into::into).collect())
        .map_err(Into::into)
}

#[tauri::command]
pub async fn clear_account_oauth_error(
    pool: tauri::State<'_, SqlitePool>,
    account_id: String,
) -> CommandResult<()> {
    AccountRepository::new(pool.inner())
        .clear_oauth_error(&account_id)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn add_account(
    pool: tauri::State<'_, SqlitePool>,
    idle: tauri::State<'_, IdleControl>,
    input: NewAccount,
) -> CommandResult<AccountDto> {
    let account = AccountRepository::new(pool.inner())
        .create_email(CreateEmailAccount {
            display_name: input.display_name,
            email: input.email,
            provider: input.provider,
            config: input.config,
        })
        .await
        .map_err(CommandError::from)?;

    idle.add_account(&account.id);
    Ok(account.into())
}

#[tauri::command]
pub async fn deactivate_account(
    pool: tauri::State<'_, SqlitePool>,
    idle: tauri::State<'_, IdleControl>,
    account_id: String,
) -> CommandResult<()> {
    AccountRepository::new(pool.inner())
        .deactivate(&account_id)
        .await
        .map_err(CommandError::from)?;

    idle.remove_account(&account_id);

    Ok(())
}

/// Delete an account and everything belonging to it, for good.
///
/// Separate from [`deactivate_account`] on purpose: deactivating keeps the mail
/// and the credentials in the file and only hides them, which is the right
/// answer for "pause this account" and the wrong one for "remove it".
#[tauri::command]
pub async fn delete_account(
    pool: tauri::State<'_, SqlitePool>,
    idle: tauri::State<'_, IdleControl>,
    account_id: String,
) -> CommandResult<()> {
    // Before the rows go: the supervisor must not reconnect to an account that
    // is about to stop existing, and a queued action for it can never run.
    idle.remove_account(&account_id);
    if let Err(error) = SyncQueue::new()
        .purge_account(pool.inner(), &account_id)
        .await
    {
        tracing::error!(%account_id, %error, "could not purge queue for deleted account");
    }

    AccountRepository::new(pool.inner())
        .delete(&account_id)
        .await
        .map_err(CommandError::from)?;

    // Deleting frees no disk on its own. Clearing the stamp makes the next
    // maintenance pass compact instead of waiting out its interval; doing it
    // here would hold the UI for minutes on a large mailbox.
    let _ = sqlx::query("DELETE FROM app_settings WHERE key = 'maintenance.last_vacuum_at'")
        .execute(pool.inner())
        .await;

    Ok(())
}

#[tauri::command]
pub async fn test_imap_account_connection(
    pool: tauri::State<'_, SqlitePool>,
    account_id: String,
) -> CommandResult<ImapAccountConnectionDto> {
    let account = AccountRepository::new(pool.inner())
        .load_active_imap_connection(&account_id)
        .await
        .map_err(CommandError::from)?;

    let provider =
        ImapProvider::from_account_config(&account.config, &account.email).map_err(|error| {
            tracing::warn!(account_id = %account.id, %error, "invalid IMAP account configuration");
            CommandError::new("backend.errors.accountConfigurationInvalid")
        })?;
    let folders = provider.sync_folders().await.map_err(|error| {
        let reason = error.to_string();
        tracing::warn!(account_id = %account.id, %reason, "IMAP connection test failed");
        CommandError::new("backend.errors.imapConnectionFailed").with_param("reason", reason)
    })?;
    let folder_sample = folders
        .iter()
        .take(5)
        .map(|folder| folder.name.clone())
        .collect::<Vec<_>>();

    Ok(ImapAccountConnectionDto {
        account_id: account.id,
        ok: true,
        folders: folders.len(),
        folder_sample,
    })
}

/// Replace the password of an IMAP or POP3 account once the server accepts it.
///
/// Checked first, so a typo is answered here rather than by the next sync.
#[tauri::command]
pub async fn update_account_password(
    pool: tauri::State<'_, SqlitePool>,
    idle: tauri::State<'_, IdleControl>,
    account_id: String,
    password: String,
) -> CommandResult<()> {
    let repository = AccountRepository::new(pool.inner());
    let change = repository
        .prepare_password_change(&account_id, &password)
        .await
        .map_err(CommandError::from)?;
    verify_mail_login(&change).await.map_err(|reason| {
        tracing::warn!(%account_id, %reason, "new account password was not accepted");
        CommandError::new("backend.errors.passwordCheckFailed").with_param("reason", reason)
    })?;
    repository
        .store_password_change(&change)
        .await
        .map_err(CommandError::from)?;
    // Restarts the IDLE or poll task, which still holds the old password.
    idle.add_account(&account_id);
    Ok(())
}

async fn verify_mail_login(change: &PasswordChange) -> Result<(), String> {
    if change.provider == "pop3" {
        return Pop3Provider::from_account_config(&change.config, &change.email)?
            .verify_credentials()
            .await;
    }
    ImapProvider::from_account_config(&change.config, &change.email)?
        .sync_folders()
        .await
        .map(|_| ())
        .map_err(|error| error.to_string())
}

impl From<AccountError> for CommandError {
    fn from(error: AccountError) -> Self {
        match error {
            AccountError::NotFound => Self::new("backend.errors.accountNotFound"),
            AccountError::NotImap => Self::new("backend.errors.accountNotImap"),
            AccountError::NoPassword => Self::new("backend.errors.accountHasNoPassword"),
            AccountError::ConfigurationInvalid => {
                Self::new("backend.errors.accountConfigurationInvalid")
            }
            AccountError::CredentialAccess { detail } => {
                tracing::error!(%detail, "account credentials could not be accessed");
                Self::new("backend.errors.accountCredentialAccessFailed")
            }
            AccountError::OAuthIdentityMissing => Self::new("backend.errors.oauthIdentityMissing"),
            AccountError::OAuthIdentityMismatch => {
                Self::new("backend.errors.oauthIdentityMismatch")
            }
            AccountError::OAuthImapConflict => Self::new("backend.errors.oauthAccountConflict"),
            AccountError::OAuthConfiguration { detail } => {
                tracing::error!(%detail, "OAuth account configuration is invalid");
                Self::new("backend.errors.oauthConfigurationInvalid")
            }
            AccountError::Persistence(error) => {
                tracing::error!(error = ?error, "account persistence failed");
                Self::new("backend.errors.accountPersistenceFailed")
            }
        }
    }
}

// `accounts.config` holds IMAP/SMTP passwords and OAuth refresh tokens and is
// encrypted at rest. The accessor moved to `satchel_core::accounts` when the
// sync and provider paths stopped being able to reach into this crate; the
// choke point is unchanged, only its address.
pub use satchel_core::accounts::{
    count_account_config_encryption, decrypt_account_config, encrypt_account_config,
    report_plaintext_account_configs, AccountConfig,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repository_errors_use_stable_keys_without_sql_details() {
        assert_eq!(
            CommandError::from(AccountError::NotFound).key,
            "backend.errors.accountNotFound"
        );

        let detail = "near SELECT: secret account SQL";
        let error = CommandError::from(AccountError::Persistence(sqlx::Error::Protocol(
            detail.into(),
        )));
        let serialized = serde_json::to_value(error).expect("command error JSON");
        assert_eq!(
            serialized,
            serde_json::json!({ "key": "backend.errors.accountPersistenceFailed" })
        );
        assert!(!serialized.to_string().contains(detail));

        let detail = "secret keychain implementation detail";
        let error = CommandError::from(AccountError::CredentialAccess {
            detail: detail.to_string(),
        });
        let serialized = serde_json::to_value(error).expect("command error JSON");
        assert_eq!(
            serialized,
            serde_json::json!({ "key": "backend.errors.accountCredentialAccessFailed" })
        );
        assert!(!serialized.to_string().contains(detail));
    }

    #[test]
    fn non_imap_accounts_use_a_specific_stable_key() {
        let error = CommandError::from(AccountError::NotImap);
        assert_eq!(
            serde_json::to_value(error).expect("command error JSON"),
            serde_json::json!({ "key": "backend.errors.accountNotImap" })
        );
    }

    #[test]
    fn oauth_account_errors_use_specific_stable_keys() {
        for (error, key) in [
            (
                AccountError::OAuthIdentityMissing,
                "backend.errors.oauthIdentityMissing",
            ),
            (
                AccountError::OAuthIdentityMismatch,
                "backend.errors.oauthIdentityMismatch",
            ),
            (
                AccountError::OAuthImapConflict,
                "backend.errors.oauthAccountConflict",
            ),
        ] {
            assert_eq!(CommandError::from(error).key, key);
        }
        let detail = "plaintext refresh token";
        let error = CommandError::from(AccountError::OAuthConfiguration {
            detail: detail.into(),
        });
        assert_eq!(error.key, "backend.errors.oauthConfigurationInvalid");
        assert!(!serde_json::to_string(&error).unwrap().contains(detail));
    }
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/accounts/")
)]
pub struct AccountVisibilityStatus {
    pub account_id: String,
    pub policy: satchel_core::db::account_visibility::AccountVisibilityPolicy,
    pub visible: bool,
    pub local_date: String,
}

#[tauri::command]
pub async fn get_account_visibility(
    pool: tauri::State<'_, crate::db::ReadPool>,
) -> CommandResult<Vec<AccountVisibilityStatus>> {
    let rows = sqlx::query_as::<_, (String, Option<String>, bool, String)>(&format!(
        "SELECT a.id, s.value, {}, date('now', 'localtime') FROM accounts a
         LEFT JOIN app_settings s ON s.key = 'account.visibility.' || a.id
         WHERE a.is_active = 1",
        crate::db::account_visibility::visible_account_sql("a.id")
    ))
    .fetch_all(pool.inner().as_ref())
    .await
    .map_err(CommandError::unexpected)?;
    Ok(rows
        .into_iter()
        .map(
            |(account_id, raw, visible, local_date)| AccountVisibilityStatus {
                account_id,
                policy: raw
                    .and_then(|raw| serde_json::from_str(&raw).ok())
                    .unwrap_or_default(),
                visible,
                local_date,
            },
        )
        .collect())
}

#[tauri::command]
pub async fn set_account_visibility(
    app: tauri::AppHandle,
    pool: tauri::State<'_, SqlitePool>,
    account_id: String,
    policy: satchel_core::db::account_visibility::AccountVisibilityPolicy,
) -> CommandResult<()> {
    if !policy.valid() {
        return Err(CommandError::new("backend.errors.accountVisibilityInvalid"));
    }
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM accounts WHERE id = ?)")
        .bind(&account_id)
        .fetch_one(pool.inner())
        .await
        .map_err(CommandError::unexpected)?;
    if !exists {
        return Err(CommandError::new("backend.errors.accountNotFound"));
    }
    sqlx::query("INSERT INTO app_settings (key, value, updated_at) VALUES (?, ?, datetime('now'))
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at")
        .bind(format!("{}{}", crate::db::account_visibility::KEY_PREFIX, account_id))
        .bind(serde_json::to_string(&policy).map_err(CommandError::unexpected)?)
        .execute(pool.inner()).await.map_err(CommandError::unexpected)?;
    crate::commands::tray::refresh_account_visibility(&app, pool.inner()).await;
    Ok(())
}
