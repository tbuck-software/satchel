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

    let Ok(bytes) = attachment_bytes(
        provider_kind,
        account_config,
        account_email,
        message_remote_id,
        folder_remote_id,
        attachment,
    )
    .await
    else {
        return;
    };

    if is_calendar {
        let raw = String::from_utf8_lossy(&bytes).into_owned();
        if let Some(invite) = crate::email::calendar::parse_invite(&raw) {
            let _ = crate::db::calendar::CalendarRepository::new(pool)
                .upsert_invite(account_id, Some(local_message_id), &invite)
                .await;
        }
    }

    if is_inline_image {
        if let Some(app) = app {
            // Never silent: the bytes are already paid for in provider quota, so
            // losing them here means every reader re-fetches the same part. That
            // is precisely how this failure stayed invisible for so long.
            if let Err(err) =
                cache_attachment_bytes(app, pool, attachment_id, attachment, &bytes).await
            {
                tracing::warn!("caching inline image {attachment_id} failed: {err}");
            }
        }
    }
}

async fn cache_attachment_bytes(
    app: &AppHandle,
    pool: &SqlitePool,
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
    let path =
        crate::commands::attachments::attachment_cache_path(&cache_dir, attachment_id, filename);
    tokio::fs::write(&path, bytes)
        .await
        .map_err(|err| format!("write attachment cache: {err}"))?;
    let path_string = path.to_string_lossy().to_string();
    let size = i64::try_from(bytes.len()).unwrap_or(i64::MAX);
    sqlx::query("UPDATE attachments SET cached_path = ?, size = ? WHERE id = ?")
        .bind(path_string)
        .bind(size)
        .bind(attachment_id)
        .execute(pool)
        .await
        .map_err(|err| err.to_string())?;
    Ok(())
}
