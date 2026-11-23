use base64::Engine as _;

use crate::providers::{
    provider_for_stored_account,
    traits::{MessageRef, RemoteAttachment},
};

/// Resolve an attachment's bytes from the message payload or its provider.
///
/// Attachments embedded in a provider message are preferred so the sync path
/// does not make a second network request. Providers are used only when the
/// message contains a remote part reference without inline bytes.
pub async fn attachment_bytes(
    pool: &sqlx::SqlitePool,
    account_id: &str,
    provider_kind: &str,
    account_config: &crate::accounts::AccountConfig,
    account_email: &str,
    message_remote_id: &str,
    folder_remote_id: &str,
    source_uidvalidity: Option<u32>,
    attachment: &RemoteAttachment,
) -> crate::sync::errors::SyncResult<Vec<u8>> {
    if let Some(raw) = attachment.content_base64.as_deref() {
        if let Ok(bytes) = decode_attachment_base64(raw) {
            return Ok(bytes);
        }
    }

    let provider = provider_for_stored_account(
        pool,
        account_id,
        provider_kind,
        account_config,
        account_email,
        None,
    )?;
    provider
        .fetch_attachment(
            &MessageRef {
                remote_id: message_remote_id.to_string(),
                source_folder: Some(folder_remote_id.to_string()),
                source_uidvalidity,
            },
            &attachment.remote_part_id,
        )
        .await
        .map(|fetched| fetched.bytes)
}

pub fn decode_attachment_base64(raw: &str) -> Result<Vec<u8>, base64::DecodeError> {
    base64::engine::general_purpose::STANDARD
        .decode(raw.as_bytes())
        .or_else(|_| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(raw.as_bytes()))
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(raw.as_bytes()))
}
