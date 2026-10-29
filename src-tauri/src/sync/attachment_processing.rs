use sqlx::SqlitePool;
use tauri::{AppHandle, Manager};

use crate::providers::traits::RemoteAttachment;
use crate::sync::attachment_fetch::attachment_bytes;

/// Handle attachment types that need local processing while a message is synced.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn process_special_attachment(
    app: Option<&AppHandle>,
    pool: &SqlitePool,
    provider_kind: &str,
    account_id: &str,
    account_config: &crate::commands::accounts::AccountConfig,
    account_email: &str,
    local_message_id: &str,
    message_remote_id: &str,
    folder_remote_id: &str,
    source_uidvalidity: Option<u32>,
    attachment_id: &str,
    attachment: &RemoteAttachment,
) {
    let content_type = attachment.content_type.trim().to_ascii_lowercase();
    let filename = attachment
        .filename
        .as_deref()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let is_calendar = content_type.starts_with("text/calendar")
        || content_type == "application/ics"
        || filename.ends_with(".ics");
    let is_inline_image = attachment.is_inline && content_type.starts_with("image/");

    if !is_calendar && !is_inline_image {
        return;
    }

    let Ok(reference) =
        crate::db::remote_content::ContentReference::load(pool, local_message_id).await
    else {
        return;
    };
    if reference.account_id != account_id
        || reference.remote_id.as_deref() != Some(message_remote_id)
        || (provider_kind == "imap"
            && (reference.uidvalidity() != source_uidvalidity
                || reference.source_folder.as_deref() != Some(folder_remote_id)))
    {
        return;
    }
    let Ok(bytes) = attachment_bytes(
        provider_kind,
        account_config,
        account_email,
        message_remote_id,
        folder_remote_id,
        source_uidvalidity,
        attachment,
    )
    .await
    else {
        return;
    };

    if is_calendar {
        let raw = String::from_utf8_lossy(&bytes).into_owned();
        if let Some(invite) = crate::email::calendar::parse_invite(&raw) {
            if let Ok(mut tx) = reference.begin_write(pool).await {
                if crate::db::calendar::CalendarRepository::upsert_invite_on_connection(
                    &mut tx,
                    account_id,
                    Some(local_message_id),
                    &invite,
                )
                .await
                .is_ok()
                {
                    let _ = tx.commit().await;
                }
            }
        }
    }

    if is_inline_image {
        if let Some(app) = app {
            // Never silent: the bytes are already paid for in provider quota, so
            // losing them here means every reader re-fetches the same part. That
            // is precisely how this failure stayed invisible for so long.
            if let Err(err) =
                cache_attachment_bytes(app, pool, &reference, attachment_id, attachment, &bytes)
                    .await
            {
                tracing::warn!("caching inline image {attachment_id} failed: {err}");
            }
        }
    }
}

async fn cache_attachment_bytes(
    app: &AppHandle,
    pool: &SqlitePool,
    reference: &crate::db::remote_content::ContentReference,
    attachment_id: &str,
    attachment: &RemoteAttachment,
    bytes: &[u8],
) -> Result<(), String> {
    let cache_dir = app
        .path()
        .resolve("satchel/attachments", tauri::path::BaseDirectory::AppData)
        .map_err(|err| err.to_string())?;
    tokio::fs::create_dir_all(&cache_dir)
        .await
        .map_err(|err| format!("create attachment cache: {err}"))?;

    let filename = attachment
        .filename
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("inline-image");
    let path = crate::commands::attachments::attachment_cache_path(
        &cache_dir,
        &reference.attachment_cache_id(attachment_id),
        &format!("{}-{filename}", uuid::Uuid::new_v4()),
    );
    tokio::fs::write(&path, bytes)
        .await
        .map_err(|err| format!("write attachment cache: {err}"))?;
    let path_string = path.to_string_lossy().to_string();
    let size = i64::try_from(bytes.len()).unwrap_or(i64::MAX);
    let mut tx = reference.begin_write(pool).await?;
    crate::commands::attachments::set_attachment_cache(
        &mut tx,
        attachment_id,
        &reference.message_id,
        &path_string,
        size,
    )
    .await?;
    tx.commit().await.map_err(|err| err.to_string())?;
    Ok(())
}
