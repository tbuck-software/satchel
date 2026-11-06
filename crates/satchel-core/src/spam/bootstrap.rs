//! One-time training from mail the user already sorted.
//!
//! Spam: what sits in a Spam folder. Ham: mail the user acted on — starred,
//! answered, filed as Done or in the Archive — and mail from people the user
//! corresponds with. Merely read Inbox mail is not evidence of anything.
//!
//! Runs as a [`DataRepair`] after startup, a block at a time, with its cursor
//! stored alongside each block. Migration 0114 plants the marker, and nothing
//! is judged until it is gone. Each account contributes at most
//! [`BOOTSTRAP_PER_CLASS`] examples of each class, newest first, so one large
//! mailbox does not shape the model for all of them. A block reads one window
//! of [`SCAN_WINDOW`] rowids, so a mailbox with little to learn from never
//! holds the write lock for a full scan.

use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use sqlx::{SqliteConnection, SqlitePool};

use super::store::{self, Label, Signal};
use crate::db::data_repairs::{DataRepair, Prepared};

pub const BOOTSTRAP_PER_CLASS: i64 = 1_000;

/// Rowids one block looks at.
pub const SCAN_WINDOW: i64 = 2_000;

pub const SPAM_BOOTSTRAP: DataRepair = DataRepair {
    name: "spam_bootstrap",
    block: bootstrap_block,
    prepare: None,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Phase {
    Spam,
    Ham,
}

impl Phase {
    fn label(self) -> Label {
        match self {
            Self::Spam => Label::Spam,
            Self::Ham => Label::Ham,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Cursor {
    /// The account being read; `None` before the first one.
    account: Option<String>,
    phase: Option<Phase>,
    /// Exclusive upper rowid of the next window, read newest first.
    before: i64,
    /// The account's lowest rowid: below it the phase is done.
    floor: i64,
    /// The account's highest rowid, where each phase starts.
    top: i64,
    /// Examples trained in this phase of this account.
    count: i64,
}

impl Cursor {
    fn parse(value: &str) -> Self {
        serde_json::from_str(value).unwrap_or_default()
    }

    fn encode(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    fn start_phase(&mut self, phase: Phase) {
        self.phase = Some(phase);
        self.before = self.top + 1;
        self.count = 0;
    }
}

/// Opens the next account after `after`; `false` when there is none.
async fn open_next_account(
    connection: &mut SqliteConnection,
    cursor: &mut Cursor,
) -> Result<bool, sqlx::Error> {
    let after = cursor.account.clone();
    let next: Option<String> = sqlx::query_scalar(
        "SELECT id FROM accounts WHERE kind = 'email' AND (? IS NULL OR id > ?) ORDER BY id LIMIT 1",
    )
    .bind(&after)
    .bind(&after)
    .fetch_optional(&mut *connection)
    .await?;
    let Some(account) = next else {
        return Ok(false);
    };
    let (floor, top): (Option<i64>, Option<i64>) =
        sqlx::query_as("SELECT MIN(rowid), MAX(rowid) FROM messages WHERE account_id = ?")
            .bind(&account)
            .fetch_one(&mut *connection)
            .await?;
    cursor.account = Some(account);
    cursor.floor = floor.unwrap_or(0);
    cursor.top = top.unwrap_or(0);
    cursor.start_phase(Phase::Spam);
    Ok(true)
}

const SPAM_CANDIDATES: &str = "SELECT m.rowid, m.id FROM messages m
     JOIN accounts a ON a.id = m.account_id
     JOIN folders f ON f.id = m.folder_id
     WHERE m.account_id = ? AND m.rowid < ? AND m.rowid >= ?
       AND LOWER(COALESCE(f.folder_type, '')) = 'spam'
       AND COALESCE(m.is_sent, 0) = 0 AND COALESCE(m.is_draft, 0) = 0
       AND LOWER(TRIM(COALESCE(m.from_address, ''))) <> LOWER(TRIM(COALESCE(a.email, '')))
       AND NOT EXISTS (SELECT 1 FROM spam_training x WHERE x.message_id = m.id)
     ORDER BY m.rowid DESC LIMIT ?";

/// Mail the user acted on, or from someone the user corresponds with.
const HAM_CANDIDATES: &str =
    "SELECT m.rowid, m.id FROM messages m
     JOIN accounts a ON a.id = m.account_id
     LEFT JOIN folders f ON f.id = COALESCE(m.pending_folder_id, m.folder_id)
     LEFT JOIN threads t ON t.id = m.thread_id
     WHERE m.account_id = ? AND m.rowid < ? AND m.rowid >= ?
       AND COALESCE(m.is_sent, 0) = 0 AND COALESCE(m.is_draft, 0) = 0
       AND LOWER(TRIM(COALESCE(m.from_address, ''))) <> LOWER(TRIM(COALESCE(a.email, '')))
       AND LOWER(COALESCE(f.folder_type, '')) NOT IN ('spam', 'trash', 'sent', 'drafts')
       AND NOT EXISTS (SELECT 1 FROM spam_training x WHERE x.message_id = m.id)
       AND NOT EXISTS (SELECT 1 FROM spam_verdicts v WHERE v.message_id = m.id AND v.verdict = 'spam')
       AND (
            COALESCE(m.is_starred, 0) <> 0
         OR COALESCE(t.is_starred, 0) <> 0
         OR LOWER(COALESCE(f.folder_type, '')) = 'archive'
         OR t.workflow_state IN ('done', 'archived')
         OR EXISTS (SELECT 1 FROM messages reply
                    WHERE reply.thread_id = m.thread_id AND COALESCE(reply.is_sent, 0) <> 0)
         OR EXISTS (SELECT 1 FROM message_addresses ma
                    JOIN messages sent ON sent.id = ma.message_id
                    WHERE ma.email = LOWER(TRIM(m.from_address)) AND COALESCE(sent.is_sent, 0) <> 0)
         OR EXISTS (SELECT 1 FROM sender_preferences p
                    WHERE LOWER(TRIM(p.sender_email)) = LOWER(TRIM(m.from_address)))
         OR EXISTS (SELECT 1 FROM contacts c
                    WHERE c.email = LOWER(TRIM(m.from_address))
                      AND (c.avatar_source IN ('manual', 'local_contact', 'provider_contact')
                           OR TRIM(COALESCE(c.notes, '')) <> ''))
       )
     ORDER BY m.rowid DESC LIMIT ?";

fn bootstrap_block<'c>(
    connection: &'c mut SqliteConnection,
    cursor: &'c str,
    limit: i64,
    _prepared: Option<Prepared>,
) -> BoxFuture<'c, Result<Option<String>, sqlx::Error>> {
    Box::pin(async move {
        let mut cursor = Cursor::parse(cursor);
        // Move on to the next phase or account that still has work.
        loop {
            match cursor.phase {
                None => {
                    if !open_next_account(connection, &mut cursor).await? {
                        store::prune(connection, store::MAX_MODEL_TOKENS, store::PRUNE_TARGET)
                            .await?;
                        return Ok(None);
                    }
                }
                Some(phase)
                    if cursor.count >= BOOTSTRAP_PER_CLASS || cursor.before <= cursor.floor =>
                {
                    match phase {
                        Phase::Spam => cursor.start_phase(Phase::Ham),
                        Phase::Ham => cursor.phase = None,
                    }
                }
                Some(_) => break,
            }
        }
        let phase = cursor.phase.unwrap_or(Phase::Spam);
        let account = cursor.account.clone().unwrap_or_default();
        let window_start = cursor.floor.max(cursor.before - SCAN_WINDOW);
        let wanted = limit.clamp(1, 100).min(BOOTSTRAP_PER_CLASS - cursor.count);
        let sql = match phase {
            Phase::Spam => SPAM_CANDIDATES,
            Phase::Ham => HAM_CANDIDATES,
        };
        let rows: Vec<(i64, String)> = sqlx::query_as(sql)
            .bind(&account)
            .bind(cursor.before)
            .bind(window_start)
            .bind(wanted)
            .fetch_all(&mut *connection)
            .await?;
        for (_, message_id) in &rows {
            if store::train_message(
                connection,
                message_id,
                phase.label(),
                Signal::Bootstrap,
                None,
            )
            .await?
            {
                cursor.count += 1;
            }
        }
        cursor.before = match rows.last() {
            // The window may hold more: go on below the last row read.
            Some((rowid, _)) if rows.len() as i64 == wanted => *rowid,
            _ => window_start,
        };
        Ok(Some(cursor.encode()))
    })
}

/// Run the whole bootstrap now. The maintenance worker uses the generic
/// runner; this is the shortcut for callers that hold only a pool.
pub async fn run_to_completion(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    crate::db::data_repairs::run_repair(pool, &SPAM_BOOTSTRAP, 100, None, |_| {}).await?;
    Ok(())
}
