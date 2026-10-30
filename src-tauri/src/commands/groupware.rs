use serde::Deserialize;
use sqlx::{FromRow, SqlitePool};

use crate::commands::error::CommandResult;
use crate::providers::dav::{self, GroupwareSyncSummary};

const GROUPWARE_SYNC_ERROR_KEY: &str = "backend.errors.groupwareSyncFailed";

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/groupware/", optional_fields = nullable))]
pub struct ConfigureGroupwareInput {
    pub account_id: String,
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub caldav_server: Option<String>,
    #[serde(default)]
    pub carddav_server: Option<String>,
}
#[derive(Debug, FromRow)]
struct GroupwareAccountRow {
    id: String,
    config: String,
}

#[tauri::command]
pub async fn configure_groupware(
    pool: tauri::State<'_, SqlitePool>,
    input: ConfigureGroupwareInput,
) -> CommandResult<GroupwareSyncSummary> {
    let username = input.username.trim();
    if username.is_empty() {
        return Err("GroupWise username is required".into());
    }
    if input.password.is_empty() {
        return Err("GroupWise password is required".into());
    }

    let caldav =
        dav::optional_dav_config(input.caldav_server.as_deref(), username, &input.password)?;
    let carddav =
        dav::optional_dav_config(input.carddav_server.as_deref(), username, &input.password)?;
    if caldav.is_none() && carddav.is_none() {
        return Err("at least one CalDAV or CardDAV server is required".into());
    }

    if let Some(config) = caldav.as_ref() {
        dav::probe_dav_config(config, true).await?;
    }
    if let Some(config) = carddav.as_ref() {
        dav::probe_dav_config(config, false).await?;
    }

    let config = crate::db::accounts::AccountRepository::new(pool.inner())
        .update_dav_config(&input.account_id, caldav, carddav, true)
        .await?;

    dav::sync_groupware_account(pool.inner(), &input.account_id, &config)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn sync_groupware(
    pool: tauri::State<'_, SqlitePool>,
    account_id: Option<String>,
) -> CommandResult<GroupwareSyncSummary> {
    let accounts = match account_id {
        Some(account_id) => sqlx::query_as::<_, GroupwareAccountRow>(
            "SELECT id, config FROM accounts WHERE id = ? AND is_active = 1",
        )
        .bind(account_id)
        .fetch_all(pool.inner())
        .await
        .map_err(|err| err.to_string())?,
        None => sqlx::query_as::<_, GroupwareAccountRow>(
            "SELECT id, config FROM accounts WHERE is_active = 1",
        )
        .fetch_all(pool.inner())
        .await
        .map_err(|err| err.to_string())?,
    };

    let mut total = GroupwareSyncSummary::default();
    for account in accounts {
        let config =
            crate::commands::accounts::decrypt_account_config(&account.id, &account.config)?;
        if !dav::has_configured_service(&config) {
            continue;
        }
        total.accounts_checked += 1;
        match dav::sync_groupware_account(pool.inner(), &account.id, &config).await {
            Ok(summary) => total.merge(summary),
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    account_id = %account.id,
                    "groupware synchronization failed"
                );
                total.errors.push(GROUPWARE_SYNC_ERROR_KEY.to_string());
            }
        }
    }
    Ok(total)
}
