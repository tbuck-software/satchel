//! Per-sender cleanup: which Inbox threads a sweep touches, which senders are
//! worth sweeping, and the recurring sweep rules.
//!
//! A sweep takes a thread only when the sender wrote **every** message in it:
//! a conversation with anyone else, one of the user's own replies or a draft
//! keeps it out, so filing a newsletter can never carry someone's reply or the
//! user's sent mail along with it. Starred threads stay as well.
//!
//! "Inbox" means what the Inbox list shows: a thread in the `inbox` workflow
//! state with at least one message whose effective folder is the account's
//! Inbox. The same folder predicate as the list, so a sweep can never reach a
//! thread the user does not see there, and a thread a sweep filed (Done,
//! trashed, moved to a folder) is not picked up again on the next run.

use serde::Serialize;
use sqlx::{FromRow, QueryBuilder, Sqlite, SqlitePool};
use thiserror::Error;

use crate::db::threads::mailbox_section_predicate;
use crate::workflow::states::WorkflowState;

/// Longest age a rule may name. Ten years is "everything", not a typo guard.
pub const MAX_OLDER_THAN_DAYS: i64 = 3650;
/// Most senders the cleanup view asks for at once.
pub const MAX_CLEANUP_SENDERS: i64 = 200;

#[derive(Debug, Error)]
pub enum SenderSweepError {
    #[error("sender sweep account id must not be empty")]
    AccountIdEmpty,
    #[error("sender sweep email is invalid")]
    SenderEmailInvalid,
    #[error("sender sweep cannot sweep the account's own address")]
    OwnAddress,
    #[error("sender sweep age is out of range")]
    DaysInvalid,
    #[error("a trash rule needs an age of at least one day")]
    TrashNeedsAge,
    #[error("sender sweep action is invalid")]
    ActionInvalid,
    #[error("sender sweep folder is missing or belongs to another account")]
    FolderInvalid,
    #[error("sender sweep rule not found")]
    NotFound,
    #[error("sender sweep persistence failed")]
    Persistence(#[from] sqlx::Error),
}

/// What a sweep does with the threads it selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SweepAction {
    Archive,
    Trash,
    Move,
}

impl SweepAction {
    pub fn as_db_value(self) -> &'static str {
        match self {
            Self::Archive => "archive",
            Self::Trash => "trash",
            Self::Move => "move",
        }
    }

    pub fn from_db_value(value: &str) -> Option<Self> {
        match value {
            "archive" => Some(Self::Archive),
            "trash" => Some(Self::Trash),
            "move" => Some(Self::Move),
            _ => None,
        }
    }
}

/// Which of a sender's Inbox threads a sweep takes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SweepSelection {
    /// Only threads whose last message is older than this UTC RFC 3339 instant.
    pub older_than: Option<String>,
    /// Leave the newest of the threads the sweep would take, whatever its age.
    pub keep_latest: bool,
}

impl SweepSelection {
    /// The selection a rule stands for, evaluated at `now`.
    pub fn for_rule(
        older_than_days: Option<i64>,
        keep_latest: bool,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Self {
        Self {
            older_than: older_than_days
                .filter(|days| *days > 0)
                .map(|days| (now - chrono::Duration::days(days)).to_rfc3339()),
            keep_latest,
        }
    }
}

/// One thread a sweep would file, with the state it is filed from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "core/db/sender_sweep/"))]
pub struct SweepTarget {
    pub thread_id: String,
    pub workflow_state: WorkflowState,
    pub last_message_date: String,
    /// Done marks a thread read; Undo needs to know which ones were not.
    pub unread_count: i64,
}

/// How much a sweep takes and what it leaves alone, without the ids.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "core/db/sender_sweep/"))]
pub struct SweepSummary {
    /// Threads the sweep would file.
    pub count: i64,
    /// The sender's Inbox threads with someone else, an own reply or a draft.
    pub skipped_shared: i64,
    /// The sender's own threads the user starred.
    pub skipped_starred: i64,
}

/// A sweep's selection with what it passed over.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SweepPlan {
    pub targets: Vec<SweepTarget>,
    pub skipped_shared: i64,
    pub skipped_starred: i64,
}

impl SweepPlan {
    pub fn summary(&self) -> SweepSummary {
        SweepSummary {
            count: self.targets.len() as i64,
            skipped_shared: self.skipped_shared,
            skipped_starred: self.skipped_starred,
        }
    }
}

/// A sender in the cleanup view: how much Inbox mail they sent lately.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "core/db/sender_sweep/"))]
pub struct CleanupSender {
    pub account_id: String,
    pub sender_email: String,
    pub sender_name: Option<String>,
    /// Messages from the sender inside the window whose thread is in the Inbox.
    pub message_count: i64,
    pub unread_count: i64,
    /// Inbox threads those messages belong to.
    pub thread_count: i64,
    pub newest_date: String,
    /// The newest message inside the window with a `List-Unsubscribe` header.
    pub unsubscribe_message_id: Option<String>,
    /// The recurring rule for this sender, if one exists.
    pub rule_id: Option<String>,
}

/// Whom a sweep of a thread is about: who started it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "core/db/sender_sweep/"))]
pub struct ThreadSweepSender {
    pub account_id: String,
    pub email: String,
    pub name: Option<String>,
}

/// A recurring sweep rule as stored, with the folder name for display.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "core/db/sender_sweep/"))]
pub struct SweepRuleRecord {
    pub id: String,
    pub account_id: String,
    pub sender_email: String,
    /// `archive`, `trash` or `move`.
    pub action: String,
    pub older_than_days: Option<i64>,
    pub keep_latest: bool,
    /// Empty on a move rule whose folder went away; such a rule does nothing
    /// until it is given a folder again.
    pub target_folder_id: Option<String>,
    pub target_folder_name: Option<String>,
    pub last_run_at: Option<String>,
    pub last_swept_count: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct SweepRuleUpsert {
    pub account_id: String,
    pub sender_email: String,
    pub action: SweepAction,
    pub older_than_days: Option<i64>,
    pub keep_latest: bool,
    pub target_folder_id: Option<String>,
}

#[derive(FromRow)]
struct CandidateRow {
    thread_id: String,
    workflow_state: String,
    last_message_date: String,
    unread_count: i64,
    starred: i64,
    sole_sender: i64,
}

#[derive(FromRow)]
struct SenderRow {
    account_id: String,
    sender_email: String,
    sender_name: Option<String>,
    message_count: i64,
    unread_count: i64,
    thread_count: i64,
    newest_date: String,
    unsubscribe_message_id: Option<String>,
    rule_id: Option<String>,
}

#[derive(FromRow)]
struct RuleRow {
    id: String,
    account_id: String,
    sender_email: String,
    action: String,
    older_than_days: Option<i64>,
    keep_latest: i64,
    target_folder_id: Option<String>,
    target_folder_name: Option<String>,
    last_run_at: Option<String>,
    last_swept_count: i64,
    created_at: String,
    updated_at: String,
}

impl From<RuleRow> for SweepRuleRecord {
    fn from(row: RuleRow) -> Self {
        Self {
            id: row.id,
            account_id: row.account_id,
            sender_email: row.sender_email,
            action: row.action,
            older_than_days: row.older_than_days,
            keep_latest: row.keep_latest != 0,
            target_folder_id: row.target_folder_id,
            target_folder_name: row.target_folder_name,
            last_run_at: row.last_run_at,
            last_swept_count: row.last_swept_count,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

/// A sender address as rules and queries compare it.
pub fn normalize_sender_email(value: &str) -> Result<String, SenderSweepError> {
    let email = value.trim().to_lowercase();
    let valid = email.len() <= 320
        && email.split_once('@').is_some_and(|(local, domain)| {
            !local.is_empty() && !domain.is_empty() && !domain.contains('@')
        })
        && !email.chars().any(|c| c.is_control() || c.is_whitespace());
    valid
        .then_some(email)
        .ok_or(SenderSweepError::SenderEmailInvalid)
}

fn normalize_account_id(value: &str) -> Result<String, SenderSweepError> {
    let id = value.trim();
    if id.is_empty() {
        return Err(SenderSweepError::AccountIdEmpty);
    }
    Ok(id.to_string())
}

pub fn validate_older_than_days(days: Option<i64>) -> Result<Option<i64>, SenderSweepError> {
    match days {
        None => Ok(None),
        Some(days) if (0..=MAX_OLDER_THAN_DAYS).contains(&days) => Ok(Some(days)),
        Some(_) => Err(SenderSweepError::DaysInvalid),
    }
}

/// The account's own addresses, as SQL: `?` is the account id, `<col>` the
/// address compared. Used to keep the user out of their own cleanup.
const OWN_ADDRESS_SQL: &str = "(SELECT LOWER(email) FROM accounts WHERE id = ?
    UNION SELECT LOWER(email) FROM account_aliases WHERE account_id = ?)";

/// Appends ` AND <thread t is in the Inbox list>`.
fn push_inbox_thread(query: &mut QueryBuilder<'_, Sqlite>) {
    let predicate = mailbox_section_predicate("inbox").expect("the inbox section has a predicate");
    query.push(
        " AND t.workflow_state = 'inbox'
          AND EXISTS (SELECT 1 FROM messages mailbox_messages
            JOIN folders mailbox_folders
              ON mailbox_folders.id = COALESCE(mailbox_messages.pending_folder_id, mailbox_messages.folder_id)
            WHERE mailbox_messages.thread_id = t.id AND ",
    );
    query.push(predicate);
    query.push(")");
}

pub struct SenderSweepRepository<'a> {
    pool: &'a SqlitePool,
}

impl<'a> SenderSweepRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    /// Refuses the account's own address and its aliases: their "mail" is the
    /// user's sent replies.
    async fn ensure_foreign_sender(
        &self,
        account_id: &str,
        sender_email: &str,
    ) -> Result<(), SenderSweepError> {
        let own: bool = sqlx::query_scalar(&format!("SELECT ? IN {OWN_ADDRESS_SQL}"))
            .bind(sender_email)
            .bind(account_id)
            .bind(account_id)
            .fetch_one(self.pool)
            .await?;
        if own {
            return Err(SenderSweepError::OwnAddress);
        }
        Ok(())
    }

    /// The sender's Inbox threads in one account that `selection` takes,
    /// newest first, and what was passed over.
    ///
    /// Only threads the sender wrote alone qualify (no other sender, no own
    /// reply — the sender is never an own address — and no draft), and none
    /// the user starred. Keep-latest then spares the newest of those, even
    /// when it is older than the cutoff, so a rule never empties the Inbox of
    /// a sender who writes once a month.
    pub async fn plan(
        &self,
        account_id: &str,
        sender_email: &str,
        selection: &SweepSelection,
    ) -> Result<SweepPlan, SenderSweepError> {
        let account_id = normalize_account_id(account_id)?;
        let sender_email = normalize_sender_email(sender_email)?;
        self.ensure_foreign_sender(&account_id, &sender_email)
            .await?;
        let mut query = QueryBuilder::<Sqlite>::new(
            "SELECT t.id AS thread_id, t.workflow_state AS workflow_state,
                    t.last_message_date AS last_message_date,
                    COALESCE(t.unread_count, 0) AS unread_count,
                    (COALESCE(t.is_starred, 0) = 1
                     OR EXISTS (SELECT 1 FROM messages pinned
                        WHERE pinned.thread_id = t.id AND pinned.is_starred = 1)) AS starred,
                    NOT EXISTS (SELECT 1 FROM messages other
                        WHERE other.thread_id = t.id
                          AND (COALESCE(other.is_draft, 0) = 1
                               OR other.from_address IS NULL
                               OR other.from_address <> ",
        );
        query.push_bind(sender_email.clone());
        query.push(
            " COLLATE NOCASE)) AS sole_sender
             FROM threads t
             JOIN accounts a ON a.id = t.account_id
             WHERE a.kind = 'email' AND t.account_id = ",
        );
        query.push_bind(account_id.clone());
        query.push(
            " AND EXISTS (SELECT 1 FROM messages m WHERE m.thread_id = t.id AND m.account_id = ",
        );
        query.push_bind(account_id);
        query.push(" AND m.from_address = ");
        query.push_bind(sender_email);
        query.push(" COLLATE NOCASE)");
        push_inbox_thread(&mut query);
        query.push(" ORDER BY t.last_message_date DESC, t.id DESC");
        let rows = query
            .build_query_as::<CandidateRow>()
            .fetch_all(self.pool)
            .await?;

        let mut plan = SweepPlan::default();
        let mut eligible = Vec::new();
        for row in rows {
            if row.sole_sender == 0 {
                plan.skipped_shared += 1;
            } else if row.starred != 0 {
                plan.skipped_starred += 1;
            } else {
                eligible.push(row);
            }
        }
        plan.targets = eligible
            .into_iter()
            .skip(usize::from(selection.keep_latest))
            .filter(|row| {
                selection
                    .older_than
                    .as_deref()
                    .is_none_or(|cutoff| row.last_message_date.as_str() < cutoff)
            })
            .filter_map(|row| {
                WorkflowState::from_db_value(&row.workflow_state).map(|workflow_state| {
                    SweepTarget {
                        thread_id: row.thread_id,
                        workflow_state,
                        last_message_date: row.last_message_date,
                        unread_count: row.unread_count,
                    }
                })
            })
            .collect();
        Ok(plan)
    }

    /// The threads [`plan`](Self::plan) takes.
    pub async fn targets(
        &self,
        account_id: &str,
        sender_email: &str,
        selection: &SweepSelection,
    ) -> Result<Vec<SweepTarget>, SenderSweepError> {
        Ok(self
            .plan(account_id, sender_email, selection)
            .await?
            .targets)
    }

    /// Who started a thread: the earliest message that is neither a draft
    /// nor the user's own. `None` for a thread only the user wrote.
    pub async fn thread_sender(
        &self,
        thread_id: &str,
    ) -> Result<Option<ThreadSweepSender>, SenderSweepError> {
        let row: Option<(String, String, Option<String>)> = sqlx::query_as(&format!(
            "SELECT m.account_id, LOWER(TRIM(m.from_address)), NULLIF(TRIM(m.from_name), '')
             FROM messages m
             WHERE m.thread_id = ?1
               AND COALESCE(m.is_draft, 0) = 0
               AND INSTR(COALESCE(m.from_address, ''), '@') > 0
               AND LOWER(TRIM(m.from_address)) NOT IN {own}
             ORDER BY m.date ASC, m.id ASC
             LIMIT 1",
            own = "(SELECT LOWER(email) FROM accounts WHERE id = m.account_id
                    UNION SELECT LOWER(email) FROM account_aliases WHERE account_id = m.account_id)",
        ))
        .bind(thread_id)
        .fetch_optional(self.pool)
        .await?;
        Ok(row.map(|(account_id, email, name)| ThreadSweepSender {
            account_id,
            email,
            name,
        }))
    }

    /// Senders by Inbox volume since `since` (UTC RFC 3339), busiest first.
    ///
    /// Counted per account: the same address in two mailboxes is two rows,
    /// because every action on a row is an action in one account. The
    /// account's own address and its aliases never appear. One statement and
    /// one scan of the window: name, unsubscribe message and rule included.
    pub async fn top_inbox_senders(
        &self,
        since: &str,
        account_id: Option<&str>,
        limit: i64,
    ) -> Result<Vec<CleanupSender>, SenderSweepError> {
        let mut query = QueryBuilder::<Sqlite>::new(
            "WITH recent AS (
               SELECT m.id, m.account_id, LOWER(m.from_address) AS sender_email, m.thread_id,
                      m.is_read, m.date, NULLIF(TRIM(m.from_name), '') AS from_name,
                      TRIM(COALESCE(m.list_unsubscribe, '')) <> '' AS has_unsubscribe
               FROM messages m
               JOIN threads t ON t.id = m.thread_id
               JOIN accounts a ON a.id = m.account_id
               WHERE a.kind = 'email'
                 AND m.date >= ",
        );
        query.push_bind(since.to_string());
        query.push(
            " AND COALESCE(m.is_draft, 0) = 0
              AND INSTR(COALESCE(m.from_address, ''), '@') > 0
              AND m.from_address <> a.email COLLATE NOCASE
              AND NOT EXISTS (SELECT 1 FROM account_aliases alias
                  WHERE alias.account_id = m.account_id AND alias.email = m.from_address COLLATE NOCASE)",
        );
        if let Some(account_id) = account_id.map(str::trim).filter(|id| !id.is_empty()) {
            query
                .push(" AND m.account_id = ")
                .push_bind(account_id.to_string());
        }
        push_inbox_thread(&mut query);
        // One pass over the window: the newest name and the newest message
        // with an unsubscribe header are folded into the aggregate as
        // `date ␟ value` maxima, so nothing scans the window again per sender.
        query.push(
            "),
             grouped AS (
               SELECT account_id, sender_email,
                      COUNT(*) AS message_count,
                      SUM(CASE WHEN is_read = 0 THEN 1 ELSE 0 END) AS unread_count,
                      COUNT(DISTINCT thread_id) AS thread_count,
                      MAX(date) AS newest_date,
                      MAX(CASE WHEN from_name IS NOT NULL THEN date || char(31) || from_name END) AS newest_name,
                      MAX(CASE WHEN has_unsubscribe THEN date || char(31) || id END) AS newest_unsubscribe
               FROM recent
               GROUP BY account_id, sender_email
               ORDER BY message_count DESC, newest_date DESC, sender_email ASC
               LIMIT ",
        );
        query.push_bind(limit.clamp(1, MAX_CLEANUP_SENDERS));
        query.push(
            ")
             SELECT g.account_id, g.sender_email, g.message_count, g.unread_count,
                    g.thread_count, g.newest_date,
                    SUBSTR(g.newest_name, INSTR(g.newest_name, char(31)) + 1) AS sender_name,
                    SUBSTR(g.newest_unsubscribe, INSTR(g.newest_unsubscribe, char(31)) + 1) AS unsubscribe_message_id,
                    s.id AS rule_id
             FROM grouped g
             LEFT JOIN sender_sweep_rules s
               ON s.account_id = g.account_id AND s.sender_email = g.sender_email
             ORDER BY g.message_count DESC, g.newest_date DESC, g.sender_email ASC",
        );
        let rows = query
            .build_query_as::<SenderRow>()
            .fetch_all(self.pool)
            .await?;
        Ok(rows
            .into_iter()
            .map(|row| CleanupSender {
                account_id: row.account_id,
                sender_email: row.sender_email,
                sender_name: row.sender_name,
                message_count: row.message_count,
                unread_count: row.unread_count,
                thread_count: row.thread_count,
                newest_date: row.newest_date,
                unsubscribe_message_id: row.unsubscribe_message_id,
                rule_id: row.rule_id,
            })
            .collect())
    }

    pub async fn list_rules(&self) -> Result<Vec<SweepRuleRecord>, SenderSweepError> {
        // Longest-waiting first: a pass that runs out of budget leaves the
        // rest to the next one, and the next one starts with them.
        let rows = sqlx::query_as::<_, RuleRow>(&rule_select(
            "ORDER BY r.last_run_at IS NOT NULL, r.last_run_at, r.sender_email, r.account_id",
        ))
        .fetch_all(self.pool)
        .await?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    pub async fn get_rule(&self, id: &str) -> Result<SweepRuleRecord, SenderSweepError> {
        sqlx::query_as::<_, RuleRow>(&rule_select("WHERE r.id = ?"))
            .bind(id)
            .fetch_optional(self.pool)
            .await?
            .map(Into::into)
            .ok_or(SenderSweepError::NotFound)
    }

    /// One rule per sender and account: a second "from now on" replaces the first.
    pub async fn upsert_rule(
        &self,
        input: SweepRuleUpsert,
    ) -> Result<SweepRuleRecord, SenderSweepError> {
        let account_id = normalize_account_id(&input.account_id)?;
        let sender_email = normalize_sender_email(&input.sender_email)?;
        let older_than_days = validate_older_than_days(input.older_than_days)?;
        // Trash on arrival is not a rule anybody means; it takes an age.
        if input.action == SweepAction::Trash && older_than_days.unwrap_or(0) < 1 {
            return Err(SenderSweepError::TrashNeedsAge);
        }
        let account_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM accounts WHERE id = ? AND kind = 'email')",
        )
        .bind(&account_id)
        .fetch_one(self.pool)
        .await?;
        if !account_exists {
            return Err(SenderSweepError::AccountIdEmpty);
        }
        self.ensure_foreign_sender(&account_id, &sender_email)
            .await?;
        let target_folder_id = match input.action {
            SweepAction::Move => {
                let folder = input
                    .target_folder_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|id| !id.is_empty())
                    .ok_or(SenderSweepError::FolderInvalid)?;
                let owned: bool = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM folders WHERE id = ? AND account_id = ?)",
                )
                .bind(folder)
                .bind(&account_id)
                .fetch_one(self.pool)
                .await?;
                if !owned {
                    return Err(SenderSweepError::FolderInvalid);
                }
                Some(folder.to_string())
            }
            SweepAction::Archive | SweepAction::Trash => None,
        };
        let now = chrono::Utc::now().to_rfc3339();
        let id: String = sqlx::query_scalar(
            "INSERT INTO sender_sweep_rules
               (id, account_id, sender_email, action, older_than_days, keep_latest,
                target_folder_id, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(account_id, sender_email) DO UPDATE SET
               action = excluded.action,
               older_than_days = excluded.older_than_days,
               keep_latest = excluded.keep_latest,
               target_folder_id = excluded.target_folder_id,
               updated_at = excluded.updated_at
             RETURNING id",
        )
        .bind(format!("sweep-{}", uuid::Uuid::new_v4()))
        .bind(&account_id)
        .bind(&sender_email)
        .bind(input.action.as_db_value())
        .bind(older_than_days)
        .bind(i64::from(input.keep_latest))
        .bind(target_folder_id)
        .bind(&now)
        .bind(&now)
        .fetch_one(self.pool)
        .await?;
        self.get_rule(&id).await
    }

    /// Whether a rule was there to delete.
    pub async fn delete_rule(&self, id: &str) -> Result<bool, SenderSweepError> {
        let removed = sqlx::query("DELETE FROM sender_sweep_rules WHERE id = ?")
            .bind(id)
            .execute(self.pool)
            .await?
            .rows_affected();
        Ok(removed > 0)
    }

    pub async fn record_run(&self, id: &str, at: &str, swept: i64) -> Result<(), SenderSweepError> {
        sqlx::query(
            "UPDATE sender_sweep_rules SET last_run_at = ?, last_swept_count = ? WHERE id = ?",
        )
        .bind(at)
        .bind(swept)
        .bind(id)
        .execute(self.pool)
        .await?;
        Ok(())
    }

    /// Whether the account syncs at all; rules of a switched-off one wait.
    pub async fn account_active(&self, account_id: &str) -> Result<bool, SenderSweepError> {
        let active: Option<i64> =
            sqlx::query_scalar("SELECT COALESCE(is_active, 1) FROM accounts WHERE id = ?")
                .bind(account_id)
                .fetch_optional(self.pool)
                .await?;
        Ok(active.is_some_and(|active| active != 0))
    }

    /// A run that failed still ran: it goes to the back of the queue.
    pub async fn record_attempt(&self, id: &str, at: &str) -> Result<(), SenderSweepError> {
        sqlx::query("UPDATE sender_sweep_rules SET last_run_at = ? WHERE id = ?")
            .bind(at)
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }
}

fn rule_select(tail: &str) -> String {
    format!(
        "SELECT r.id, r.account_id, r.sender_email, r.action, r.older_than_days, r.keep_latest,
                r.target_folder_id, COALESCE(f.full_path, f.name) AS target_folder_name,
                r.last_run_at, r.last_swept_count, r.created_at, r.updated_at
         FROM sender_sweep_rules r
         LEFT JOIN folders f ON f.id = r.target_folder_id
         {tail}"
    )
}

#[cfg(test)]
pub(crate) mod tests {
    use sqlx::sqlite::SqlitePoolOptions;

    use super::*;

    pub(crate) async fn pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory pool");
        crate::db::init(&pool).await.expect("migrations");
        pool
    }

    pub(crate) async fn insert_account(pool: &SqlitePool, id: &str) {
        sqlx::query(
            "INSERT INTO accounts
             (id, display_name, email, provider, config, credentials_ref, sync_state,
              color, is_active, created_at, updated_at)
             VALUES (?, 'Account', ?, 'imap', '{}', '', '{}', '#000', 1, '2026-01-01', '2026-01-01')",
        )
        .bind(id)
        .bind(format!("{id}@example.com"))
        .execute(pool)
        .await
        .expect("account");
        for (suffix, name, kind) in [
            ("inbox", "INBOX", "inbox"),
            ("archive", "Archive", "archive"),
            ("trash", "Trash", "trash"),
            ("news", "Newsletters", "custom"),
        ] {
            sqlx::query(
                "INSERT INTO folders
                 (id, account_id, name, full_path, remote_id, folder_type, uidvalidity,
                  created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, 7, '2026-01-01', '2026-01-01')",
            )
            .bind(format!("{id}-{suffix}"))
            .bind(id)
            .bind(name)
            .bind(name)
            .bind(name)
            .bind(kind)
            .execute(pool)
            .await
            .expect("folder");
        }
    }

    /// A one-message thread from `from` in `folder`, `days_ago` days old.
    pub(crate) async fn insert_thread(
        pool: &SqlitePool,
        account: &str,
        id: &str,
        from: &str,
        days_ago: i64,
        folder: &str,
        state: &str,
        read: bool,
    ) {
        let date = (chrono::Utc::now() - chrono::Duration::days(days_ago)).to_rfc3339();
        sqlx::query(
            "INSERT INTO threads
             (id, account_id, subject, last_message_date, message_count, unread_count,
              workflow_state, created_at, updated_at)
             VALUES (?, ?, ?, ?, 1, ?, ?, '2026-01-01', '2026-01-01')",
        )
        .bind(id)
        .bind(account)
        .bind(id)
        .bind(&date)
        .bind(i64::from(!read))
        .bind(state)
        .execute(pool)
        .await
        .expect("thread");
        sqlx::query(
            "INSERT INTO messages
             (id, account_id, folder_id, thread_id, from_address, from_name, to_addresses,
              subject, date, is_read, remote_id, remote_uidvalidity, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, 'Sender', '[]', ?, ?, ?, ?, 7, '2026-01-01', '2026-01-01')",
        )
        .bind(format!("{id}-m"))
        .bind(account)
        .bind(format!("{account}-{folder}"))
        .bind(id)
        .bind(from)
        .bind(id)
        .bind(&date)
        .bind(i64::from(read))
        .bind(format!("uid-{id}"))
        .execute(pool)
        .await
        .expect("message");
    }

    fn ids(targets: &[SweepTarget]) -> Vec<&str> {
        targets
            .iter()
            .map(|target| target.thread_id.as_str())
            .collect()
    }

    /// One more message in an existing thread.
    pub(crate) async fn add_message(
        pool: &SqlitePool,
        account: &str,
        thread: &str,
        id: &str,
        from: &str,
        days_ago: i64,
        draft: bool,
    ) {
        let date = (chrono::Utc::now() - chrono::Duration::days(days_ago)).to_rfc3339();
        sqlx::query(
            "INSERT INTO messages
             (id, account_id, folder_id, thread_id, from_address, from_name, to_addresses,
              subject, date, is_read, is_draft, remote_id, remote_uidvalidity, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, 'Someone', '[]', 's', ?, 1, ?, ?, 7, '2026-01-01', '2026-01-01')",
        )
        .bind(id)
        .bind(account)
        .bind(format!("{account}-inbox"))
        .bind(thread)
        .bind(from)
        .bind(&date)
        .bind(i64::from(draft))
        .bind(format!("uid-{id}"))
        .execute(pool)
        .await
        .expect("message");
    }

    async fn add_alias(pool: &SqlitePool, account: &str, email: &str) {
        sqlx::query(
            "INSERT INTO account_aliases (id, account_id, email, created_at, updated_at)
             VALUES (?, ?, ?, '2026-01-01', '2026-01-01')",
        )
        .bind(format!("alias-{email}"))
        .bind(account)
        .bind(email)
        .execute(pool)
        .await
        .expect("alias");
    }

    #[tokio::test]
    async fn only_threads_the_sender_wrote_alone_are_taken() {
        let pool = pool().await;
        insert_account(&pool, "a").await;
        add_alias(&pool, "a", "me.alias@example.com").await;
        insert_thread(
            &pool,
            "a",
            "solo",
            "news@shop.example",
            3,
            "inbox",
            "inbox",
            true,
        )
        .await;
        add_message(&pool, "a", "solo", "solo-2", "News@Shop.Example", 2, false).await;
        // Someone else joined the conversation.
        insert_thread(
            &pool,
            "a",
            "mixed",
            "news@shop.example",
            4,
            "inbox",
            "inbox",
            true,
        )
        .await;
        add_message(
            &pool,
            "a",
            "mixed",
            "mixed-2",
            "friend@example.org",
            3,
            false,
        )
        .await;
        // The user answered, from the account address and from an alias.
        insert_thread(
            &pool,
            "a",
            "replied",
            "news@shop.example",
            5,
            "inbox",
            "inbox",
            true,
        )
        .await;
        add_message(
            &pool,
            "a",
            "replied",
            "replied-2",
            "a@example.com",
            4,
            false,
        )
        .await;
        insert_thread(
            &pool,
            "a",
            "alias",
            "news@shop.example",
            5,
            "inbox",
            "inbox",
            true,
        )
        .await;
        add_message(
            &pool,
            "a",
            "alias",
            "alias-2",
            "Me.Alias@example.com",
            4,
            false,
        )
        .await;
        // A reply is being written.
        insert_thread(
            &pool,
            "a",
            "drafted",
            "news@shop.example",
            6,
            "inbox",
            "inbox",
            true,
        )
        .await;
        add_message(
            &pool,
            "a",
            "drafted",
            "drafted-2",
            "news@shop.example",
            5,
            true,
        )
        .await;

        let repo = SenderSweepRepository::new(&pool);
        let plan = repo
            .plan("a", "news@shop.example", &SweepSelection::default())
            .await
            .unwrap();
        assert_eq!(ids(&plan.targets), ["solo"]);
        assert_eq!(plan.skipped_shared, 4);
        assert_eq!(
            plan.summary(),
            SweepSummary {
                count: 1,
                skipped_shared: 4,
                skipped_starred: 0
            }
        );
    }

    #[tokio::test]
    async fn starred_and_todo_threads_stay() {
        let pool = pool().await;
        insert_account(&pool, "a").await;
        insert_thread(
            &pool,
            "a",
            "plain",
            "news@shop.example",
            1,
            "inbox",
            "inbox",
            true,
        )
        .await;
        insert_thread(
            &pool,
            "a",
            "starred",
            "news@shop.example",
            2,
            "inbox",
            "inbox",
            true,
        )
        .await;
        insert_thread(
            &pool,
            "a",
            "todo",
            "news@shop.example",
            3,
            "inbox",
            "todo",
            true,
        )
        .await;
        sqlx::query("UPDATE threads SET is_starred = 1 WHERE id = 'starred'")
            .execute(&pool)
            .await
            .unwrap();
        let plan = SenderSweepRepository::new(&pool)
            .plan("a", "news@shop.example", &SweepSelection::default())
            .await
            .unwrap();
        assert_eq!(ids(&plan.targets), ["plain"]);
        assert_eq!(plan.skipped_starred, 1);
    }

    #[tokio::test]
    async fn the_users_own_addresses_cannot_be_swept() {
        let pool = pool().await;
        insert_account(&pool, "a").await;
        add_alias(&pool, "a", "me.alias@example.com").await;
        let repo = SenderSweepRepository::new(&pool);
        for own in ["a@example.com", "A@Example.com", "me.alias@example.com"] {
            assert!(matches!(
                repo.plan("a", own, &SweepSelection::default()).await,
                Err(SenderSweepError::OwnAddress)
            ));
            assert!(matches!(
                repo.upsert_rule(SweepRuleUpsert {
                    account_id: "a".into(),
                    sender_email: own.into(),
                    action: SweepAction::Archive,
                    older_than_days: Some(10),
                    keep_latest: false,
                    target_folder_id: None,
                })
                .await,
                Err(SenderSweepError::OwnAddress)
            ));
        }
    }

    #[tokio::test]
    async fn a_trash_rule_needs_an_age() {
        let pool = pool().await;
        insert_account(&pool, "a").await;
        let repo = SenderSweepRepository::new(&pool);
        for days in [None, Some(0)] {
            assert!(matches!(
                repo.upsert_rule(SweepRuleUpsert {
                    account_id: "a".into(),
                    sender_email: "news@shop.example".into(),
                    action: SweepAction::Trash,
                    older_than_days: days,
                    keep_latest: false,
                    target_folder_id: None,
                })
                .await,
                Err(SenderSweepError::TrashNeedsAge)
            ));
        }
    }

    #[tokio::test]
    async fn a_move_rule_survives_its_folder_disappearing() {
        let pool = pool().await;
        insert_account(&pool, "a").await;
        let repo = SenderSweepRepository::new(&pool);
        let rule = repo
            .upsert_rule(SweepRuleUpsert {
                account_id: "a".into(),
                sender_email: "news@shop.example".into(),
                action: SweepAction::Move,
                older_than_days: None,
                keep_latest: false,
                target_folder_id: Some("a-news".into()),
            })
            .await
            .unwrap();
        // What the folder reconcile does when the server folder is gone.
        sqlx::query("DELETE FROM folders WHERE id = 'a-news'")
            .execute(&pool)
            .await
            .unwrap();
        let kept = repo.get_rule(&rule.id).await.unwrap();
        assert_eq!(kept.action, "move");
        assert_eq!(kept.target_folder_id, None);
    }

    #[tokio::test]
    async fn the_thread_sender_is_whoever_started_it() {
        let pool = pool().await;
        insert_account(&pool, "a").await;
        insert_thread(
            &pool,
            "a",
            "t",
            "first@shop.example",
            5,
            "inbox",
            "inbox",
            true,
        )
        .await;
        add_message(&pool, "a", "t", "t-2", "later@other.example", 1, false).await;
        insert_thread(
            &pool,
            "a",
            "mine",
            "a@example.com",
            5,
            "inbox",
            "inbox",
            true,
        )
        .await;
        add_message(
            &pool,
            "a",
            "mine",
            "mine-2",
            "reply@other.example",
            1,
            false,
        )
        .await;
        insert_thread(
            &pool,
            "a",
            "alone",
            "a@example.com",
            5,
            "inbox",
            "inbox",
            true,
        )
        .await;
        let repo = SenderSweepRepository::new(&pool);
        assert_eq!(
            repo.thread_sender("t").await.unwrap().map(|s| s.email),
            Some("first@shop.example".into())
        );
        // The user's own first message does not count as the start.
        assert_eq!(
            repo.thread_sender("mine").await.unwrap().map(|s| s.email),
            Some("reply@other.example".into())
        );
        assert_eq!(repo.thread_sender("alone").await.unwrap(), None);
    }

    #[tokio::test]
    async fn top_senders_leave_out_aliases() {
        let pool = pool().await;
        insert_account(&pool, "a").await;
        add_alias(&pool, "a", "me.alias@example.com").await;
        insert_thread(
            &pool,
            "a",
            "x",
            "Me.Alias@example.com",
            1,
            "inbox",
            "inbox",
            true,
        )
        .await;
        insert_thread(
            &pool,
            "a",
            "y",
            "news@shop.example",
            1,
            "inbox",
            "inbox",
            true,
        )
        .await;
        let since = (chrono::Utc::now() - chrono::Duration::days(90)).to_rfc3339();
        let senders = SenderSweepRepository::new(&pool)
            .top_inbox_senders(&since, None, 50)
            .await
            .unwrap();
        let emails: Vec<_> = senders.iter().map(|s| s.sender_email.as_str()).collect();
        assert_eq!(emails, ["news@shop.example"]);
    }

    #[test]
    fn sender_emails_are_normalized_and_checked() {
        assert_eq!(
            normalize_sender_email("  News@Shop.Example ").unwrap(),
            "news@shop.example"
        );
        for invalid in ["", "nobody", "@shop.example", "a@", "a b@c.d", "a@b@c"] {
            assert!(normalize_sender_email(invalid).is_err(), "{invalid}");
        }
    }

    #[tokio::test]
    async fn targets_are_the_senders_inbox_threads_newest_first() {
        let pool = pool().await;
        insert_account(&pool, "a").await;
        insert_thread(
            &pool,
            "a",
            "new",
            "news@shop.example",
            1,
            "inbox",
            "inbox",
            false,
        )
        .await;
        insert_thread(
            &pool,
            "a",
            "old",
            "News@Shop.Example",
            20,
            "inbox",
            "inbox",
            true,
        )
        .await;
        insert_thread(
            &pool,
            "a",
            "done",
            "news@shop.example",
            5,
            "inbox",
            "done",
            true,
        )
        .await;
        insert_thread(
            &pool,
            "a",
            "filed",
            "news@shop.example",
            5,
            "archive",
            "inbox",
            true,
        )
        .await;
        insert_thread(
            &pool,
            "a",
            "other",
            "friend@example.org",
            2,
            "inbox",
            "inbox",
            true,
        )
        .await;

        let repo = SenderSweepRepository::new(&pool);
        let all = repo
            .targets("a", "NEWS@shop.example", &SweepSelection::default())
            .await
            .unwrap();
        assert_eq!(ids(&all), ["new", "old"]);
        assert_eq!(all[0].workflow_state, WorkflowState::Inbox);
    }

    #[tokio::test]
    async fn keep_latest_spares_the_newest_thread_even_when_it_is_old() {
        let pool = pool().await;
        insert_account(&pool, "a").await;
        for (id, days) in [("t1", 40), ("t2", 50), ("t3", 60)] {
            insert_thread(
                &pool,
                "a",
                id,
                "news@shop.example",
                days,
                "inbox",
                "inbox",
                true,
            )
            .await;
        }
        let repo = SenderSweepRepository::new(&pool);
        let now = chrono::Utc::now();

        let rest = repo
            .targets(
                "a",
                "news@shop.example",
                &SweepSelection::for_rule(None, true, now),
            )
            .await
            .unwrap();
        assert_eq!(ids(&rest), ["t2", "t3"]);

        // Everything is older than ten days; the newest still stays.
        let aged = repo
            .targets(
                "a",
                "news@shop.example",
                &SweepSelection::for_rule(Some(10), true, now),
            )
            .await
            .unwrap();
        assert_eq!(ids(&aged), ["t2", "t3"]);
    }

    #[tokio::test]
    async fn the_age_cutoff_takes_only_threads_older_than_n_days() {
        let pool = pool().await;
        insert_account(&pool, "a").await;
        for (id, days) in [("d2", 2), ("d9", 9), ("d11", 11), ("d31", 31)] {
            insert_thread(
                &pool,
                "a",
                id,
                "news@shop.example",
                days,
                "inbox",
                "inbox",
                true,
            )
            .await;
        }
        let repo = SenderSweepRepository::new(&pool);
        let now = chrono::Utc::now();
        let selection = |days| SweepSelection::for_rule(Some(days), false, now);

        let three = repo
            .targets("a", "news@shop.example", &selection(3))
            .await
            .unwrap();
        assert_eq!(ids(&three), ["d9", "d11", "d31"]);
        let ten = repo
            .targets("a", "news@shop.example", &selection(10))
            .await
            .unwrap();
        assert_eq!(ids(&ten), ["d11", "d31"]);
        let thirty = repo
            .targets("a", "news@shop.example", &selection(30))
            .await
            .unwrap();
        assert_eq!(ids(&thirty), ["d31"]);
        // Zero days is "everything", the shape a move-to-folder rule has.
        let zero = repo
            .targets("a", "news@shop.example", &selection(0))
            .await
            .unwrap();
        assert_eq!(zero.len(), 4);
    }

    #[tokio::test]
    async fn targets_stay_inside_one_account() {
        let pool = pool().await;
        insert_account(&pool, "a").await;
        insert_account(&pool, "b").await;
        insert_thread(
            &pool,
            "a",
            "a1",
            "news@shop.example",
            1,
            "inbox",
            "inbox",
            true,
        )
        .await;
        insert_thread(
            &pool,
            "b",
            "b1",
            "news@shop.example",
            1,
            "inbox",
            "inbox",
            true,
        )
        .await;
        insert_thread(
            &pool,
            "b",
            "b2",
            "news@shop.example",
            3,
            "inbox",
            "inbox",
            true,
        )
        .await;
        let repo = SenderSweepRepository::new(&pool);
        let keep = SweepSelection {
            keep_latest: true,
            ..SweepSelection::default()
        };
        assert!(repo
            .targets("a", "news@shop.example", &keep)
            .await
            .unwrap()
            .is_empty());
        assert_eq!(
            ids(&repo.targets("b", "news@shop.example", &keep).await.unwrap()),
            ["b2"]
        );
    }

    #[tokio::test]
    async fn top_senders_count_recent_inbox_mail_per_account() {
        let pool = pool().await;
        insert_account(&pool, "a").await;
        insert_account(&pool, "b").await;
        for (index, days) in [1, 2, 3, 120].into_iter().enumerate() {
            insert_thread(
                &pool,
                "a",
                &format!("n{index}"),
                "news@shop.example",
                days,
                "inbox",
                "inbox",
                index % 2 == 0,
            )
            .await;
        }
        insert_thread(
            &pool,
            "a",
            "f1",
            "friend@example.org",
            4,
            "inbox",
            "inbox",
            true,
        )
        .await;
        insert_thread(
            &pool,
            "a",
            "f-done",
            "friend@example.org",
            4,
            "inbox",
            "done",
            true,
        )
        .await;
        insert_thread(&pool, "a", "me", "a@example.com", 1, "inbox", "inbox", true).await;
        insert_thread(
            &pool,
            "b",
            "b1",
            "news@shop.example",
            1,
            "inbox",
            "inbox",
            true,
        )
        .await;
        sqlx::query(
            "UPDATE messages SET list_unsubscribe = '<https://shop.example/u>' WHERE id = 'n1-m'",
        )
        .execute(&pool)
        .await
        .unwrap();

        let since = (chrono::Utc::now() - chrono::Duration::days(90)).to_rfc3339();
        let repo = SenderSweepRepository::new(&pool);
        let senders = repo.top_inbox_senders(&since, None, 50).await.unwrap();
        let rows: Vec<_> = senders
            .iter()
            .map(|s| {
                (
                    s.account_id.as_str(),
                    s.sender_email.as_str(),
                    s.message_count,
                )
            })
            .collect();
        assert_eq!(
            rows,
            [
                ("a", "news@shop.example", 3),
                // Equal counts: the more recent sender first.
                ("b", "news@shop.example", 1),
                ("a", "friend@example.org", 1),
            ]
        );
        let news = &senders[0];
        // n1 is the only unread one inside the window (n3 is outside it).
        assert_eq!(news.unread_count, 1);
        // Threads inside the window; the one from 120 days ago is outside.
        assert_eq!(news.thread_count, 3);
        assert_eq!(news.unsubscribe_message_id.as_deref(), Some("n1-m"));
        assert_eq!(news.sender_name.as_deref(), Some("Sender"));
        assert!(senders[1].unsubscribe_message_id.is_none());

        let only_b = repo.top_inbox_senders(&since, Some("b"), 50).await.unwrap();
        assert_eq!(only_b.len(), 1);
        assert_eq!(only_b[0].account_id, "b");
    }

    #[tokio::test]
    async fn rules_are_one_per_sender_and_account() {
        let pool = pool().await;
        insert_account(&pool, "a").await;
        insert_account(&pool, "b").await;
        let repo = SenderSweepRepository::new(&pool);
        let first = repo
            .upsert_rule(SweepRuleUpsert {
                account_id: "a".into(),
                sender_email: "News@Shop.Example".into(),
                action: SweepAction::Archive,
                older_than_days: Some(10),
                keep_latest: false,
                target_folder_id: Some("a-news".into()),
            })
            .await
            .unwrap();
        assert_eq!(first.sender_email, "news@shop.example");
        // An archive rule carries no folder, whatever the caller sent.
        assert_eq!(first.target_folder_id, None);

        let replaced = repo
            .upsert_rule(SweepRuleUpsert {
                account_id: "a".into(),
                sender_email: "news@shop.example".into(),
                action: SweepAction::Move,
                older_than_days: None,
                keep_latest: false,
                target_folder_id: Some("a-news".into()),
            })
            .await
            .unwrap();
        assert_eq!(replaced.id, first.id);
        assert_eq!(replaced.action, "move");
        assert_eq!(replaced.target_folder_name.as_deref(), Some("Newsletters"));

        // A folder of another account is refused.
        let foreign = repo
            .upsert_rule(SweepRuleUpsert {
                account_id: "b".into(),
                sender_email: "news@shop.example".into(),
                action: SweepAction::Move,
                older_than_days: None,
                keep_latest: false,
                target_folder_id: Some("a-news".into()),
            })
            .await;
        assert!(matches!(foreign, Err(SenderSweepError::FolderInvalid)));
        let too_old = repo
            .upsert_rule(SweepRuleUpsert {
                account_id: "b".into(),
                sender_email: "news@shop.example".into(),
                action: SweepAction::Trash,
                older_than_days: Some(MAX_OLDER_THAN_DAYS + 1),
                keep_latest: false,
                target_folder_id: None,
            })
            .await;
        assert!(matches!(too_old, Err(SenderSweepError::DaysInvalid)));

        assert_eq!(repo.list_rules().await.unwrap().len(), 1);
        assert!(repo.delete_rule(&first.id).await.unwrap());
        assert!(!repo.delete_rule(&first.id).await.unwrap());
        assert!(repo.list_rules().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn deleting_the_account_removes_its_rules() {
        let pool = pool().await;
        insert_account(&pool, "a").await;
        let repo = SenderSweepRepository::new(&pool);
        repo.upsert_rule(SweepRuleUpsert {
            account_id: "a".into(),
            sender_email: "news@shop.example".into(),
            action: SweepAction::Trash,
            older_than_days: Some(30),
            keep_latest: true,
            target_folder_id: None,
        })
        .await
        .unwrap();
        // The app's own removal path: folders go by hand, the rule follows the account.
        crate::db::accounts::AccountRepository::new(&pool)
            .delete("a")
            .await
            .unwrap();
        assert!(repo.list_rules().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_pinned_message_keeps_its_thread() {
        let pool = pool().await;
        insert_account(&pool, "a").await;
        insert_thread(
            &pool,
            "a",
            "t",
            "news@shop.example",
            2,
            "inbox",
            "inbox",
            true,
        )
        .await;
        sqlx::query("UPDATE messages SET is_starred = 1 WHERE id = 't-m'")
            .execute(&pool)
            .await
            .unwrap();
        let plan = SenderSweepRepository::new(&pool)
            .plan("a", "news@shop.example", &SweepSelection::default())
            .await
            .unwrap();
        assert!(plan.targets.is_empty());
        assert_eq!(plan.skipped_starred, 1);
    }

    #[tokio::test]
    async fn top_senders_take_the_newest_name_and_unsubscribe_message() {
        let pool = pool().await;
        insert_account(&pool, "a").await;
        insert_thread(
            &pool,
            "a",
            "old",
            "news@shop.example",
            9,
            "inbox",
            "inbox",
            true,
        )
        .await;
        insert_thread(
            &pool,
            "a",
            "mid",
            "news@shop.example",
            5,
            "inbox",
            "inbox",
            true,
        )
        .await;
        insert_thread(
            &pool,
            "a",
            "new",
            "news@shop.example",
            1,
            "inbox",
            "inbox",
            true,
        )
        .await;
        for (id, name, header) in [
            (
                "old-m",
                Some("Shop Weekly"),
                Some("<https://shop.example/u1>"),
            ),
            (
                "mid-m",
                Some("Shop News"),
                Some("<https://shop.example/u2>"),
            ),
            // The newest carries neither: the previous ones stand.
            ("new-m", None, None),
        ] {
            sqlx::query("UPDATE messages SET from_name = ?, list_unsubscribe = ? WHERE id = ?")
                .bind(name)
                .bind(header)
                .bind(id)
                .execute(&pool)
                .await
                .unwrap();
        }
        let rule = SenderSweepRepository::new(&pool)
            .upsert_rule(SweepRuleUpsert {
                account_id: "a".into(),
                sender_email: "news@shop.example".into(),
                action: SweepAction::Archive,
                older_than_days: Some(30),
                keep_latest: false,
                target_folder_id: None,
            })
            .await
            .unwrap();
        let since = (chrono::Utc::now() - chrono::Duration::days(90)).to_rfc3339();
        let senders = SenderSweepRepository::new(&pool)
            .top_inbox_senders(&since, None, 50)
            .await
            .unwrap();
        assert_eq!(senders.len(), 1);
        let news = &senders[0];
        assert_eq!(news.sender_name.as_deref(), Some("Shop News"));
        assert_eq!(news.unsubscribe_message_id.as_deref(), Some("mid-m"));
        assert_eq!(news.rule_id.as_deref(), Some(rule.id.as_str()));
        assert_eq!(news.message_count, 3);
        assert_eq!(news.thread_count, 3);
    }
}
