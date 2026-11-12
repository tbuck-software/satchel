//! Read-only database projection for the connection status surface.

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::collections::HashMap;

use crate::providers::cursor::history_backfill_stats;
use thiserror::Error;

use crate::sync::{
    queue::{ACTION_SEND_MESSAGE, ACTION_SYNC_DRAFT},
    remote_mutations::ACTION_REMOTE_MUTATION,
};

/// Durable progress survives window reloads and the pauses between bounded sync runs.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "core/db/connection_state/")
)]
pub struct AccountSyncProgress {
    pub account_id: String,
    pub account_name: String,
    pub email: String,
    pub phase: String,
    pub folder: Option<String>,
    pub folder_index: Option<u64>,
    pub folder_total: Option<u64>,
    pub backfill_remaining: u64,
    pub backfill_total: u64,
    pub updated_at: String,
}

#[derive(Default, Deserialize)]
struct StoredProgress {
    phase: Option<String>,
    folder: Option<String>,
    folder_index: Option<u64>,
    folder_total: Option<u64>,
}

#[derive(sqlx::FromRow)]
struct AccountProgressRow {
    id: String,
    display_name: String,
    email: String,
    sync_status: Option<String>,
    sync_progress_json: Option<String>,
    cursors_json: Option<String>,
    updated_at: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConnectionStateSnapshot {
    pub last_sync_at: Option<String>,
    pub pending_outbox: u64,
    pub pending_sends: u64,
    pub failed_outbox: u64,
    pub failed_sends: u64,
    pub failed_remote_mutations: u64,
    pub sync_progress: Vec<AccountSyncProgress>,
}

#[derive(Debug, Error)]
pub enum ConnectionStateError {
    #[error("connection state persistence failed")]
    Persistence(#[source] sqlx::Error),
}

impl From<sqlx::Error> for ConnectionStateError {
    fn from(error: sqlx::Error) -> Self {
        Self::Persistence(error)
    }
}

/// Owns the database-backed portion of the connection-state projection.
/// Reachability, active sync state and unread notifications are app-owned and
/// therefore deliberately remain outside this repository.
pub struct ConnectionStateRepository<'a> {
    pool: &'a SqlitePool,
}

impl<'a> ConnectionStateRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    /// Read the counts and latest account sync timestamp used by the status UI.
    ///
    /// The predicates intentionally mirror the existing queue APIs and
    /// connection-state callers, including notification-prefixed queue rows.
    pub async fn snapshot(&self) -> Result<ConnectionStateSnapshot, ConnectionStateError> {
        let row: (i64, i64, i64, i64, i64, Option<String>) = sqlx::query_as(
            "SELECT
                (SELECT COUNT(1) FROM sync_queue
                 WHERE status IN ('pending', 'sending', 'scheduled')),
                (SELECT COUNT(1) FROM sync_queue
                 WHERE status IN ('pending', 'sending', 'scheduled')
                   AND action_type = ?),
                (SELECT COUNT(1) FROM sync_queue
                 WHERE status = 'failed'),
                (SELECT COUNT(1) FROM sync_queue
                 WHERE status IN ('failed', 'delivered', 'delivery_unknown')
                   AND action_type = ?),
                (SELECT COUNT(1) FROM sync_queue
                 WHERE status IN ('failed', 'delivered', 'delivery_unknown')
                   AND action_type IN (?, ?)),
                (SELECT MAX(last_sync_at) FROM accounts
                 WHERE last_sync_at IS NOT NULL)",
        )
        .bind(ACTION_SEND_MESSAGE)
        .bind(ACTION_SEND_MESSAGE)
        .bind(ACTION_REMOTE_MUTATION)
        .bind(ACTION_SYNC_DRAFT)
        .fetch_one(self.pool)
        .await?;

        // Read only the cursor envelope, not deferred message/snapshot payloads.
        // This uses the existing ten-second connection ticker, with no UI poll.
        let accounts = sqlx::query_as::<_, AccountProgressRow>(
            "SELECT id, display_name, email, sync_status, sync_progress_json, updated_at,
                    CASE WHEN json_valid(sync_state) THEN json_extract(sync_state, '$.cursors') END AS cursors_json
             FROM accounts WHERE is_active = 1 AND provider IN ('imap', 'pop3', 'gmail', 'outlook')
             ORDER BY id",
        ).fetch_all(self.pool).await?;
        let mut sync_progress: Vec<AccountSyncProgress> = accounts
            .into_iter()
            .filter_map(|account| {
                let cursors: HashMap<String, String> = account
                    .cursors_json
                    .as_deref()
                    .and_then(|raw| serde_json::from_str(raw).ok())
                    .unwrap_or_default();
                let (remaining, total) = cursors
                    .values()
                    .filter_map(|cursor| history_backfill_stats(cursor))
                    .fold(
                        (0u64, 0u64),
                        |(remaining, total), (next_remaining, next_total)| {
                            (
                                remaining.saturating_add(next_remaining as u64),
                                total.saturating_add(next_total as u64),
                            )
                        },
                    );
                let busy = account.sync_status.as_deref() == Some("syncing");
                if !busy && remaining == 0 && total == 0 {
                    return None;
                }
                let stored: StoredProgress = account
                    .sync_progress_json
                    .as_deref()
                    .and_then(|raw| serde_json::from_str(raw).ok())
                    .unwrap_or_default();
                Some(AccountSyncProgress {
                    account_id: account.id,
                    account_name: account.display_name,
                    email: account.email,
                    phase: if busy {
                        stored.phase.unwrap_or_else(|| "starting".into())
                    } else if account.sync_status.as_deref() == Some("error") {
                        "error".into()
                    } else if remaining == 0 {
                        "complete".into()
                    } else {
                        "waiting".into()
                    },
                    folder: if busy { stored.folder } else { None },
                    folder_index: if busy { stored.folder_index } else { None },
                    folder_total: if busy { stored.folder_total } else { None },
                    backfill_remaining: remaining,
                    backfill_total: total,
                    updated_at: account.updated_at,
                })
            })
            .collect();
        // Finished accounts remain in the total until all imports finish, so
        // completing one account cannot make the overall counter jump backwards.
        let importing = sync_progress.iter().any(|row| row.backfill_remaining > 0);
        sync_progress.retain(|row| importing || row.phase != "complete");

        Ok(ConnectionStateSnapshot {
            pending_outbox: nonnegative_count(row.0),
            pending_sends: nonnegative_count(row.1),
            failed_outbox: nonnegative_count(row.2),
            failed_sends: nonnegative_count(row.3),
            failed_remote_mutations: nonnegative_count(row.4),
            last_sync_at: row.5,
            sync_progress,
        })
    }
}

fn nonnegative_count(value: i64) -> u64 {
    value.max(0) as u64
}
