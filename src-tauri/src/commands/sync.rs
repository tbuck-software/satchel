use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::{
    commands::error::CommandResult,
    providers::{imap::ImapProvider, traits::MailProvider},
};

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/sync/"))]
pub struct SyncSummaryDto {
    pub accounts_synced: usize,
    pub accounts_failed: usize,
    pub folders_synced: usize,
    pub messages_synced: usize,
    pub backfill_pending_accounts: usize,
    pub backfill_remaining_messages: usize,
    pub backfill_total_messages: usize,
    pub failures: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/sync/", optional_fields = nullable))]
pub struct TestImapConnectionInput {
    pub email: String,
    pub config: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/sync/"))]
pub struct TestImapConnectionDto {
    pub ok: bool,
    pub folders: usize,
    pub folder_sample: Vec<String>,
}

#[tauri::command]
pub async fn run_imap_sync(
    app: tauri::AppHandle,
    pool: tauri::State<'_, SqlitePool>,
    state: tauri::State<'_, crate::AppState>,
) -> CommandResult<SyncSummaryDto> {
    let summary = crate::sync::mailbox_sync::sync_mailboxes(
        pool.inner(),
        crate::sync::mailbox_sync::MailboxSyncRequest {
            events: Some(&state.events),
            app: Some(&app),
            accounts: crate::sync::mailbox_sync::AccountSelection::AllActive,
            folders: crate::sync::mailbox_sync::FolderSelection::All,
        },
    )
    .await?;
    Ok(SyncSummaryDto {
        accounts_synced: summary.accounts_synced,
        accounts_failed: summary.accounts_failed,
        folders_synced: summary.folders_synced,
        messages_synced: summary.messages_synced,
        backfill_pending_accounts: summary.backfill_pending_accounts,
        backfill_remaining_messages: summary.backfill_remaining_messages,
        backfill_total_messages: summary.backfill_total_messages,
        failures: summary.failures,
    })
}

#[tauri::command]
pub async fn test_imap_connection(
    input: TestImapConnectionInput,
) -> CommandResult<TestImapConnectionDto> {
    let provider = ImapProvider::from_account_config(&input.config.to_string(), &input.email)?;
    let folders = provider.sync_folders().await?;
    let folder_sample = folders
        .iter()
        .take(5)
        .map(|folder| folder.name.clone())
        .collect::<Vec<_>>();

    Ok(TestImapConnectionDto {
        ok: true,
        folders: folders.len(),
        folder_sample,
    })
}
