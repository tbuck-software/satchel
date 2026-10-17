//! Read-only database projection for the connection status surface.

use sqlx::SqlitePool;
use thiserror::Error;

use crate::sync::{
    queue::{ACTION_SEND_MESSAGE, ACTION_SYNC_DRAFT},
    remote_mutations::ACTION_REMOTE_MUTATION,
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConnectionStateSnapshot {
    pub last_sync_at: Option<String>,
    pub pending_outbox: u64,
    pub pending_sends: u64,
    pub failed_outbox: u64,
    pub failed_sends: u64,
    pub failed_remote_mutations: u64,
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

        Ok(ConnectionStateSnapshot {
            pending_outbox: nonnegative_count(row.0),
            pending_sends: nonnegative_count(row.1),
            failed_outbox: nonnegative_count(row.2),
            failed_sends: nonnegative_count(row.3),
            failed_remote_mutations: nonnegative_count(row.4),
            last_sync_at: row.5,
        })
    }
}

fn nonnegative_count(value: i64) -> u64 {
    value.max(0) as u64
}
