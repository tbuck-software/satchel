//! One-time training from mail the user already sorted: what sits in a Spam
//! folder is spam, read Inbox mail is ham.
//!
//! Runs as a [`DataRepair`] after startup, a block at a time, with its cursor
//! stored alongside each block. Migration 0114 plants the marker. Each class is
//! capped at [`BOOTSTRAP_PER_CLASS`] messages, newest first: enough to make the
//! filter useful from day one without the ledger growing with the mailbox.

use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use sqlx::{SqliteConnection, SqlitePool};

use super::store::{self, Label, Signal};
use crate::db::data_repairs::{DataRepair, Prepared};

pub const BOOTSTRAP_PER_CLASS: i64 = 1_000;

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

    fn folder_type(self) -> &'static str {
        match self {
            Self::Spam => "spam",
            Self::Ham => "inbox",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Cursor {
    phase: Phase,
    /// The folder being read; `None` before the first folder of the phase.
    folder: Option<String>,
    /// Exclusive upper rowid within `folder`, read newest first.
    before: i64,
    /// Messages trained in this phase so far.
    count: i64,
}

impl Cursor {
    fn start(phase: Phase) -> Self {
        Self {
            phase,
            folder: None,
            before: i64::MAX,
            count: 0,
        }
    }

    fn parse(value: &str) -> Self {
        serde_json::from_str(value).unwrap_or_else(|_| Self::start(Phase::Spam))
    }

    fn encode(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

async fn next_folder(
    connection: &mut SqliteConnection,
    phase: Phase,
    after: Option<&str>,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT f.id FROM folders f JOIN accounts a ON a.id = f.account_id
         WHERE LOWER(COALESCE(f.folder_type, '')) = ? AND a.kind = 'email'
           AND (? IS NULL OR f.id > ?)
         ORDER BY f.id LIMIT 1",
    )
    .bind(phase.folder_type())
    .bind(after)
    .bind(after)
    .fetch_optional(&mut *connection)
    .await
}

fn bootstrap_block<'c>(
    connection: &'c mut SqliteConnection,
    cursor: &'c str,
    limit: i64,
    _prepared: Option<Prepared>,
) -> BoxFuture<'c, Result<Option<String>, sqlx::Error>> {
    Box::pin(async move {
        let mut cursor = Cursor::parse(cursor);
        loop {
            if cursor.count >= BOOTSTRAP_PER_CLASS {
                match cursor.phase {
                    Phase::Spam => cursor = Cursor::start(Phase::Ham),
                    Phase::Ham => return Ok(None),
                }
            }
            if cursor.folder.is_none() {
                match next_folder(connection, cursor.phase, None).await? {
                    Some(folder) => cursor.folder = Some(folder),
                    None => match cursor.phase {
                        Phase::Spam => {
                            cursor = Cursor::start(Phase::Ham);
                            continue;
                        }
                        Phase::Ham => return Ok(None),
                    },
                }
            }
            break;
        }
        let folder = cursor.folder.clone().unwrap_or_default();
        let wanted = limit.clamp(1, 100).min(BOOTSTRAP_PER_CLASS - cursor.count);
        let rows: Vec<(i64, String)> = sqlx::query_as(
            "SELECT m.rowid, m.id FROM messages m JOIN accounts a ON a.id = m.account_id
             WHERE m.folder_id = ? AND m.rowid < ?
               AND (? = 'spam' OR COALESCE(m.is_read, 0) <> 0)
               AND COALESCE(m.is_sent, 0) = 0 AND COALESCE(m.is_draft, 0) = 0
               AND LOWER(TRIM(COALESCE(m.from_address, ''))) <> LOWER(TRIM(COALESCE(a.email, '')))
               AND NOT EXISTS (SELECT 1 FROM spam_training t WHERE t.message_id = m.id)
               AND NOT EXISTS (SELECT 1 FROM spam_verdicts v WHERE v.message_id = m.id AND v.verdict = 'spam')
             ORDER BY m.rowid DESC LIMIT ?",
        )
        .bind(&folder)
        .bind(cursor.before)
        .bind(cursor.phase.folder_type())
        .bind(wanted)
        .fetch_all(&mut *connection)
        .await?;
        for (_, message_id) in &rows {
            if store::train_message(
                connection,
                message_id,
                cursor.phase.label(),
                Signal::Weak,
                None,
            )
            .await?
            {
                cursor.count += 1;
            }
        }
        if (rows.len() as i64) < wanted {
            // This folder is done; the next block opens the next one.
            match next_folder(connection, cursor.phase, Some(&folder)).await? {
                Some(next) => {
                    cursor.folder = Some(next);
                    cursor.before = i64::MAX;
                }
                None => match cursor.phase {
                    Phase::Spam => cursor = Cursor::start(Phase::Ham),
                    Phase::Ham => {
                        store::prune(connection, store::MAX_MODEL_TOKENS, store::PRUNE_TARGET)
                            .await?;
                        return Ok(None);
                    }
                },
            }
        } else if let Some((rowid, _)) = rows.last() {
            cursor.before = *rowid;
        }
        Ok(Some(cursor.encode()))
    })
}

/// Run the whole bootstrap now. Tests and the maintenance worker use the
/// generic runner; this is the shortcut for callers that hold only a pool.
pub async fn run_to_completion(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    crate::db::data_repairs::run_repair(pool, &SPAM_BOOTSTRAP, 100, None, |_| {}).await?;
    Ok(())
}
