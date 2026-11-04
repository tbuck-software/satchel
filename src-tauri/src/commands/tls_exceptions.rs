//! Expired-certificate exceptions, as the UI grants, lists and ends them.
//!
//! The policy (what may be excepted, for how long, when it ends) lives in
//! `satchel_core::providers::tls_trust`; this module only exposes it.

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::commands::error::{CommandError, CommandResult};
use satchel_core::providers::tls_trust::{self, CertificateException, GrantError};

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/tls_exceptions/")
)]
pub struct CertificateExceptionDto {
    pub account_id: String,
    pub host: String,
    pub fingerprint: String,
    /// RFC 3339.
    pub expires_at: String,
    /// RFC 3339.
    pub created_at: String,
}

impl From<CertificateException> for CertificateExceptionDto {
    fn from(exception: CertificateException) -> Self {
        Self {
            account_id: exception.account_id,
            host: exception.host,
            fingerprint: exception.fingerprint,
            expires_at: tls_trust::timestamp(exception.expires_at),
            created_at: tls_trust::timestamp(exception.created_at),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/tls_exceptions/")
)]
pub struct TrustExpiredCertificateInput {
    pub account_id: String,
    pub host: String,
    /// The fingerprint the person saw and confirmed.
    pub fingerprint: String,
}

fn persistence_failed(error: impl std::fmt::Display) -> CommandError {
    tracing::error!(error = %error, "certificate exception persistence failed");
    CommandError::new("backend.errors.tlsExceptionPersistenceFailed")
}

impl From<GrantError> for CommandError {
    fn from(error: GrantError) -> Self {
        match error {
            GrantError::NoRejection => CommandError::new("backend.errors.tlsExceptionNoRejection"),
            GrantError::CertificateChanged => {
                CommandError::new("backend.errors.tlsExceptionCertificateChanged")
            }
            GrantError::NotAllowed => CommandError::new("backend.errors.tlsExceptionNotAllowed"),
            GrantError::Persistence(error) => persistence_failed(error),
        }
    }
}

#[tauri::command]
pub async fn list_certificate_exceptions(
    pool: tauri::State<'_, SqlitePool>,
) -> CommandResult<Vec<CertificateExceptionDto>> {
    // The database is the record; reading it also drops the ones that ran out.
    tls_trust::refresh(pool.inner())
        .await
        .map_err(persistence_failed)?;
    Ok(tls_trust::active_exceptions()
        .into_iter()
        .map(Into::into)
        .collect())
}

#[tauri::command]
pub async fn trust_expired_certificate(
    pool: tauri::State<'_, SqlitePool>,
    input: TrustExpiredCertificateInput,
) -> CommandResult<CertificateExceptionDto> {
    let pool = pool.inner();
    // Host and login come from the account itself; the UI only names the
    // certificate it showed. An exception for a server the account does not
    // use would never apply, and one for another account's login must not.
    let connection = account_connection(pool, &input.account_id)
        .await
        .ok_or_else(|| CommandError::new("backend.errors.tlsExceptionNotAllowed"))?;
    if !same_host(&connection.host, &input.host) {
        return Err(CommandError::new("backend.errors.tlsExceptionNotAllowed"));
    }
    let grant = || {
        tls_trust::grant(
            pool,
            &input.account_id,
            &connection.host,
            &connection.login,
            &input.fingerprint,
        )
    };
    match grant().await {
        // The rejection lives in memory. After a restart the person may confirm
        // what the stored error showed before a sync has met the server again;
        // one connection attempt puts the rejection back on record.
        Err(GrantError::NoRejection) => {
            probe(&connection).await;
            grant().await.map(Into::into).map_err(Into::into)
        }
        result => result.map(Into::into).map_err(Into::into),
    }
}

#[tauri::command]
pub async fn revoke_certificate_exception(
    pool: tauri::State<'_, SqlitePool>,
    account_id: String,
    host: String,
) -> CommandResult<bool> {
    tls_trust::revoke(pool.inner(), &account_id, &host)
        .await
        .map_err(persistence_failed)
}

/// What a mail account connects to, read from its own configuration.
struct AccountConnection {
    account_id: String,
    host: String,
    login: String,
    provider: std::sync::Arc<dyn satchel_core::providers::traits::MailProvider>,
}

async fn account_connection(pool: &SqlitePool, account_id: &str) -> Option<AccountConnection> {
    let (kind, email, encrypted_config) = sqlx::query_as::<_, (String, String, String)>(
        "SELECT provider, email, config FROM accounts
         WHERE id = ? AND provider IN ('imap', 'pop3') AND is_active = 1",
    )
    .bind(account_id)
    .fetch_optional(pool)
    .await
    .ok()??;
    let config =
        crate::commands::accounts::decrypt_account_config(account_id, &encrypted_config).ok()?;
    let (host, login) = if kind == "pop3" {
        let cfg = satchel_core::providers::pop3::Pop3Provider::from_account_config(
            config.as_str(),
            &email,
        )
        .ok()?
        .config?;
        (cfg.host, cfg.username)
    } else {
        let cfg = satchel_core::providers::imap::ImapProvider::from_account_config(
            config.as_str(),
            &email,
        )
        .ok()?
        .config?;
        (cfg.host, cfg.username)
    };
    let provider = crate::providers::provider_for_account(&kind, &config, &email, None).ok()?;
    Some(AccountConnection {
        account_id: account_id.to_string(),
        host,
        login,
        provider,
    })
}

fn same_host(left: &str, right: &str) -> bool {
    left.trim_end_matches('.')
        .eq_ignore_ascii_case(right.trim_end_matches('.'))
}

/// How long the dialog waits for the check connection before giving up.
const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// Opens one connection for the account so its certificate is checked again.
/// Only the side effect matters; the outcome is read from `tls_trust`.
async fn probe(connection: &AccountConnection) {
    match tokio::time::timeout(PROBE_TIMEOUT, connection.provider.sync_folders()).await {
        Ok(Ok(_)) => {}
        Ok(Err(error)) => tracing::debug!(account_id = %connection.account_id, error = %error,
            "certificate probe connection failed"),
        Err(_) => tracing::debug!(account_id = %connection.account_id,
            "certificate probe connection timed out"),
    }
}
