pub mod cursor;
pub mod dav;
pub mod folder_mapping;
pub mod gmail;
pub mod google_calendar;
pub mod imap;
pub mod imap_sessions;
pub mod outlook;
pub mod pop3;
pub mod smtp;
pub mod traits;

use std::sync::Arc;

use traits::MailProvider;

/// Build a provider from a stored account config.
///
/// Decrypts here rather than at each SELECT: the column is read from a dozen
/// queries with different shapes and aliases, and patching every one of them
/// individually is how sync silently broke once already. Decryption is a no-op
/// on plaintext, so it is safe to apply again even if a caller already did.
/// `body_window_days` overrides how much history is stored with its body:
/// `None` takes whatever the account config or the platform default says,
/// `Some(0)` keeps everything, `Some(n)` keeps n days. Only the sync path has
/// an opinion — drafts, attachments and mutation replay never backfill.
pub fn provider_for_account(
    kind: &str,
    config_json: &crate::accounts::AccountConfig,
    fallback_email: &str,
    body_window_days: Option<u32>,
) -> Result<Arc<dyn MailProvider>, String> {
    match kind {
        "gmail" => Ok(Arc::new(gmail::GmailProvider::from_account_config(
            config_json,
        ))),
        "outlook" => Ok(Arc::new(outlook::OutlookProvider::from_account_config(
            config_json,
        ))),
        "imap" => {
            let mut provider =
                imap::ImapProvider::from_account_config(config_json, fallback_email)?;
            if let Some(days) = body_window_days {
                provider.body_window_days = (days > 0).then_some(days);
            }
            Ok(Arc::new(provider))
        }
        "pop3" => Ok(Arc::new(pop3::Pop3Provider::from_account_config(
            config_json,
            fallback_email,
        )?)),
        _ => Err(format!("unsupported provider: {kind}")),
    }
}
