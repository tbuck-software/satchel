use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

use crate::commands::error::CommandResult;
use crate::sync::queue::{QueuedAction, SyncQueue};
use crate::workflow::states::WorkflowState;
use satchel_core::rules::{decide_forward, rule_token, ForwardDecision, RuleAction};
use satchel_core::server_rules::compiler::RuleSpec;

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/rules/"))]
pub struct RuleDto {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub trigger_type: String,
    pub trigger_config: serde_json::Value,
    pub actions: Vec<String>,
    pub is_active: bool,
    pub priority: i64,
    /// The one account the rule applies to; `None` for every account. A rule
    /// that moves mail names the account whose folder it moves to.
    pub account_id: Option<String>,
    /// Set when the rule forwarded more than allowed within an hour; its
    /// forward stays off until the person resumes it (`resume_rule_forwarding`).
    pub forward_paused_at: Option<String>,
    /// Forwards left to the server because the rule ran there when the mail
    /// arrived.
    pub server_forward_skips: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/rules/", optional_fields = nullable))]
pub struct UpsertRuleInput {
    pub id: Option<String>,
    pub name: String,
    pub description: Option<String>,
    pub trigger_type: String,
    pub trigger_config: serde_json::Value,
    pub actions: Vec<String>,
    pub is_active: bool,
    pub priority: i64,
    #[serde(default)]
    pub account_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/rules/", optional_fields = nullable))]
pub struct RunRulesForThreadsInput {
    pub thread_ids: Vec<String>,
}

#[derive(Debug, FromRow)]
struct RuleRow {
    id: String,
    name: String,
    description: Option<String>,
    trigger_type: String,
    trigger_config: String,
    actions: String,
    is_active: i64,
    priority: i64,
    account_id: Option<String>,
    forward_paused_at: Option<String>,
}

impl RuleRow {
    fn spec(&self) -> RuleSpec {
        RuleSpec {
            id: self.id.clone(),
            name: self.name.clone(),
            trigger_type: self.trigger_type.clone(),
            trigger_config: self.trigger_config.clone(),
            actions: self.actions.clone(),
            account_id: self.account_id.clone(),
        }
    }
}

const RULE_COLUMNS: &str = "id, name, description, trigger_type, trigger_config, actions, \
    is_active, priority, account_id, forward_paused_at";

#[derive(Debug, Default, FromRow)]
struct ThreadRuleContext {
    /// The message the rule run is about: the new one, or the thread's
    /// newest when rules run on demand.
    message_id: Option<String>,
    account_id: String,
    message_id_header: Option<String>,
    received_at: Option<String>,
    /// Where the message is: no forward out of Spam or Trash.
    folder_type: Option<String>,
    subject: Option<String>,
    snippet: Option<String>,
    from_address: Option<String>,
    to_addresses: Option<String>,
    cc_addresses: Option<String>,
    body_text: Option<String>,
}

#[tauri::command]
pub async fn list_rules(
    pool: tauri::State<'_, crate::db::ReadPool>,
) -> CommandResult<Vec<RuleDto>> {
    Ok(list_rules_inner(pool.inner().as_ref()).await?)
}

pub async fn list_rules_inner(pool: &SqlitePool) -> Result<Vec<RuleDto>, String> {
    let rows = sqlx::query_as::<_, RuleRow>(&format!(
        "SELECT {RULE_COLUMNS} FROM rules ORDER BY priority DESC, created_at ASC"
    ))
    .fetch_all(pool)
    .await
    .map_err(|err| err.to_string())?;
    let skips: std::collections::HashMap<String, i64> = sqlx::query_as::<_, (String, i64)>(
        "SELECT rule_token, COUNT(*) FROM rule_forwards
         WHERE outcome = 'skipped_server_live' GROUP BY rule_token",
    )
    .fetch_all(pool)
    .await
    .map_err(|err| err.to_string())?
    .into_iter()
    .collect();
    Ok(rows
        .into_iter()
        .map(|row| {
            let skips = skips.get(&rule_token(&row.id)).copied().unwrap_or(0);
            let mut dto = to_rule_dto(row);
            dto.server_forward_skips = skips;
            dto
        })
        .collect())
}

#[tauri::command]
pub async fn upsert_rule(
    pool: tauri::State<'_, SqlitePool>,
    input: UpsertRuleInput,
) -> CommandResult<RuleDto> {
    let rule = upsert_rule_inner(pool.inner(), input).await?;
    spawn_server_rules_push(pool.inner().clone());
    Ok(rule)
}

/// How long rule edits settle before the rule set goes to the servers.
const SERVER_PUSH_SETTLE: std::time::Duration = std::time::Duration::from_secs(3);
static SERVER_PUSH_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// A changed rule set goes to every account with server rules on. In the
/// background, so the save never waits on a mail server, and coalesced: a
/// burst of edits (toggling several rules) becomes one push per account
/// after the last of them, instead of one push each queued behind the lock.
///
/// Local rules run regardless; the server copy only adds runs while
/// Satchel is closed.
pub(crate) fn spawn_server_rules_push(pool: SqlitePool) {
    use std::sync::atomic::Ordering;
    let generation = SERVER_PUSH_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(SERVER_PUSH_SETTLE).await;
        if SERVER_PUSH_GENERATION.load(Ordering::SeqCst) == generation {
            satchel_core::server_rules::service::push_enabled_accounts(&pool).await;
        }
    });
}

pub async fn upsert_rule_inner(
    pool: &SqlitePool,
    mut input: UpsertRuleInput,
) -> Result<RuleDto, String> {
    validate_rule(&mut input)?;
    validate_rule_targets(pool, &input).await?;
    let now = chrono::Utc::now().to_rfc3339();
    let id = input
        .id
        .clone()
        .unwrap_or_else(|| format!("rule-{}", uuid::Uuid::new_v4()));
    let trigger_config =
        serde_json::to_string(&input.trigger_config).map_err(|err| err.to_string())?;
    let actions = serde_json::to_string(&input.actions).map_err(|err| err.to_string())?;

    sqlx::query(
        "INSERT INTO rules (id, name, description, is_active, priority, trigger_type, trigger_config, actions, account_id, last_triggered, trigger_count, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, 0, ?, ?)
         ON CONFLICT(id) DO UPDATE SET
             name = excluded.name,
             description = excluded.description,
             is_active = excluded.is_active,
             priority = excluded.priority,
             trigger_type = excluded.trigger_type,
             trigger_config = excluded.trigger_config,
             actions = excluded.actions,
             account_id = excluded.account_id,
             updated_at = excluded.updated_at",
    )
    .bind(&id)
    .bind(&input.name)
    .bind(&input.description)
    .bind(if input.is_active { 1 } else { 0 })
    .bind(input.priority)
    .bind(&input.trigger_type)
    .bind(&trigger_config)
    .bind(&actions)
    .bind(&input.account_id)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|err| err.to_string())?;

    Ok(RuleDto {
        id,
        name: input.name,
        description: input.description,
        trigger_type: input.trigger_type,
        trigger_config: input.trigger_config,
        actions: input.actions,
        is_active: input.is_active,
        priority: input.priority,
        account_id: input.account_id,
        forward_paused_at: None,
        server_forward_skips: 0,
    })
}

#[tauri::command]
pub async fn delete_rule(pool: tauri::State<'_, SqlitePool>, rule_id: String) -> CommandResult<()> {
    let changed = sqlx::query("DELETE FROM rules WHERE id = ?")
        .bind(rule_id.trim())
        .execute(pool.inner())
        .await
        .map_err(|err| err.to_string())?
        .rows_affected();
    if changed == 0 {
        return Err("rule not found".into());
    }
    spawn_server_rules_push(pool.inner().clone());
    Ok(())
}

#[tauri::command]
pub async fn run_rules_for_thread(
    pool: tauri::State<'_, SqlitePool>,
    thread_id: String,
) -> CommandResult<Vec<String>> {
    Ok(run_rules_for_thread_inner(pool.inner(), &thread_id).await?)
}

#[tauri::command]
pub async fn run_rules_for_threads(
    pool: tauri::State<'_, SqlitePool>,
    input: RunRulesForThreadsInput,
) -> CommandResult<Vec<String>> {
    if input.thread_ids.len() > 5_000 {
        return Err("too many threads".into());
    }
    let mut applied = Vec::new();
    for thread_id in input.thread_ids {
        let thread_id = thread_id.trim();
        if thread_id.is_empty() {
            continue;
        }
        validate_plain_value(thread_id, 320, "thread_id")?;
        applied.extend(run_rules_for_thread_inner(pool.inner(), thread_id).await?);
    }
    Ok(applied)
}

pub async fn run_rules_for_thread_inner(
    pool: &SqlitePool,
    thread_id: &str,
) -> Result<Vec<String>, String> {
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|error| error.to_string())?;
    let applied = run_rules_for_thread_tx(&mut tx, thread_id, None).await?;
    tx.commit().await.map_err(|error| error.to_string())?;
    crate::sync::queue_worker::app_queue().wake_worker();
    Ok(applied)
}

pub(crate) async fn run_pending_ingest_rules(
    pool: &SqlitePool,
    pending: &[(String, String)],
) -> Result<(), String> {
    use sqlx::Acquire;
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|error| error.to_string())?;
    for (message_id, thread_id) in pending {
        let mut savepoint = tx.begin().await.map_err(|error| error.to_string())?;
        let result = async {
            let pending: Option<i64> =
                sqlx::query_scalar("SELECT run_rules FROM ingest_effects WHERE message_id = ?")
                    .bind(message_id)
                    .fetch_optional(&mut *savepoint)
                    .await
                    .map_err(|error| error.to_string())?;
            if pending == Some(1) {
                run_rules_for_thread_tx(&mut savepoint, thread_id, Some(message_id)).await?;
            }
            sqlx::query("DELETE FROM ingest_effects WHERE message_id = ?")
                .bind(message_id)
                .execute(&mut *savepoint)
                .await
                .map_err(|error| error.to_string())?;
            Ok::<_, String>(())
        }
        .await;
        match result {
            Ok(()) => savepoint
                .commit()
                .await
                .map_err(|error| error.to_string())?,
            Err(error) => {
                savepoint
                    .rollback()
                    .await
                    .map_err(|error| error.to_string())?;
                tracing::warn!(%error, %thread_id, "automatic rules remain pending");
            }
        }
    }
    tx.commit().await.map_err(|error| error.to_string())?;
    crate::sync::queue_worker::app_queue().wake_worker();
    Ok(())
}

async fn run_rules_for_thread_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    thread_id: &str,
    message_id: Option<&str>,
) -> Result<Vec<String>, String> {
    validate_plain_value(thread_id.trim(), 320, "thread_id")?;
    let ctx = if let Some(message_id) = message_id {
        sqlx::query_as::<_, ThreadRuleContext>(
            "SELECT id AS message_id, account_id, message_id_header, received_at,
                    (SELECT folder_type FROM folders WHERE folders.id = messages.folder_id) AS folder_type,
                    subject, snippet, from_address, to_addresses, cc_addresses, body_text
             FROM messages WHERE id = ? AND thread_id = ?"
        ).bind(message_id).bind(thread_id).fetch_one(&mut **tx).await.map_err(|error| error.to_string())?
    } else {
        sqlx::query_as::<_, ThreadRuleContext>(
            "WITH newest AS (
           SELECT * FROM messages WHERE thread_id = ? AND COALESCE(is_draft, 0) = 0
           ORDER BY date DESC, id DESC LIMIT 1
         )
         SELECT
           (SELECT id FROM newest) AS message_id,
           threads.account_id,
           (SELECT message_id_header FROM newest) AS message_id_header,
           (SELECT received_at FROM newest) AS received_at,
           (SELECT folder_type FROM folders WHERE folders.id = (SELECT folder_id FROM newest)) AS folder_type,
           threads.subject,
           threads.snippet,
           (SELECT from_address FROM newest) AS from_address,
           (SELECT to_addresses FROM newest) AS to_addresses,
           (SELECT cc_addresses FROM newest) AS cc_addresses,
           (SELECT body_text FROM newest) AS body_text
         FROM threads WHERE id = ? LIMIT 1",
        )
        .bind(thread_id)
        .bind(thread_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|err| err.to_string())?
        .ok_or_else(|| "thread not found".to_string())?
    };

    let rows = sqlx::query_as::<_, RuleRow>(&format!(
        "SELECT {RULE_COLUMNS} FROM rules WHERE is_active = 1 ORDER BY priority DESC, created_at ASC"
    ))
    .fetch_all(&mut **tx)
    .await
    .map_err(|err| err.to_string())?;

    // Every rule runs here, also those the account's server runs too:
    // flags are idempotent, and a move into the folder the server already
    // filed the message in does nothing. A forward is not idempotent; see
    // `forward_once`. The first matching rule's move wins, and it runs after
    // every flag was queued: IMAP UIDs are per folder.
    let mut applied = Vec::new();
    let mut chosen_move: Option<String> = None;
    let queue = crate::sync::queue_worker::app_queue();
    for row in rows {
        let spec = row.spec();
        if !spec.applies_to(&ctx.account_id) {
            continue;
        }
        let trigger_config =
            serde_json::from_str::<serde_json::Value>(&row.trigger_config).unwrap_or_default();
        if !matches_rule(&row.trigger_type, &trigger_config, &ctx) {
            continue;
        }
        let actions = serde_json::from_str::<Vec<String>>(&row.actions).unwrap_or_default();
        for action in actions {
            match RuleAction::parse(&action) {
                Some(RuleAction::MoveToFolder(folder_id)) => {
                    if chosen_move.is_some() {
                        continue;
                    }
                    chosen_move = Some(folder_id);
                }
                // Only arriving mail is forwarded: applying the rules to
                // what is already there (a whole visible list) must not send
                // old mail out.
                Some(RuleAction::Forward { to, .. }) => {
                    if message_id.is_none()
                        || !forward_once(tx, &queue, &row, &spec, &ctx, &to).await?
                    {
                        continue;
                    }
                }
                Some(RuleAction::Star) => {
                    satchel_core::mail_actions::set_flags_in_transaction(
                        tx,
                        &queue,
                        &[thread_id.to_string()],
                        None,
                        Some(true),
                        None,
                    )
                    .await?;
                }
                _ => apply_action(tx, thread_id, &action).await?,
            }
            applied.push(format!("{}: {action}", row.name));
        }
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "UPDATE rules SET last_triggered = ?, trigger_count = trigger_count + 1, updated_at = ? WHERE id = ?",
        )
        .bind(&now)
        .bind(&now)
        .bind(&row.id)
        .execute(&mut **tx)
        .await
        .map_err(|err| err.to_string())?;
    }
    if let Some(folder_id) = chosen_move {
        move_for_rule(tx, &queue, thread_id, &ctx.account_id, &folder_id).await?;
    }
    Ok(applied)
}

/// Folder types a rule never moves mail into: the Inbox (rules run on Inbox
/// mail, it would run them again) and the folders Satchel writes itself.
const NOT_A_MOVE_TARGET: [&str; 3] = ["inbox", "sent", "drafts"];

/// Move the thread's Inbox messages into a folder of its own account. A
/// folder that is gone, or belongs to another account, leaves the mail where
/// it is: the rest of the rule still applies.
async fn move_for_rule(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    queue: &SyncQueue,
    thread_id: &str,
    account_id: &str,
    folder_id: &str,
) -> Result<(), String> {
    let folder_type: Option<String> =
        sqlx::query_scalar("SELECT folder_type FROM folders WHERE id = ? AND account_id = ?")
            .bind(folder_id)
            .bind(account_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|error| error.to_string())?;
    match folder_type {
        Some(kind) if !NOT_A_MOVE_TARGET.contains(&kind.as_str()) => {
            satchel_core::mail_actions::move_inbox_messages_in_transaction(
                tx, queue, thread_id, folder_id,
            )
            .await?;
        }
        other => {
            tracing::warn!(%thread_id, %folder_id, kind = ?other, "rule move skipped: not a folder of this account");
        }
    }
    Ok(())
}

/// Whether `address` is one of Satchel's own accounts or aliases, a `+tag`
/// of one included: forwarding there would bring the message back into an
/// Inbox and run the rule again.
async fn is_own_address<'e, E>(executor: E, address: &str) -> Result<bool, String>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    let own: Vec<String> = sqlx::query_scalar(
        "SELECT email FROM accounts UNION ALL SELECT email FROM account_aliases",
    )
    .fetch_all(executor)
    .await
    .map_err(|error| error.to_string())?;
    Ok(own
        .iter()
        .any(|email| satchel_core::rules::same_mailbox(email, address)))
}

/// Since when the server may forward this message for the rule: the
/// earliest start of a forward of it (verified, or written by a push that
/// never finished or failed) that goes to the same address and whose trigger
/// matches the message. The forward is what counts, not the whole rule: a pin
/// added since does not stop the server forwarding, and mail that matches an
/// older trigger still running there is the server's.
async fn server_forward_since(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    account_id: &str,
    rule_id: &str,
    to: &str,
    ctx: &ThreadRuleContext,
) -> Result<Option<chrono::DateTime<chrono::Utc>>, String> {
    let row: Option<(String, String)> = sqlx::query_as(
        "SELECT pushed_forwards, possibly_live_rules FROM managesieve_accounts WHERE account_id = ?",
    )
    .bind(account_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|error| error.to_string())?;
    let Some((forwards, possibly)) = row else {
        return Ok(None);
    };
    let mut state = satchel_core::db::server_rules::ServerRulesState::empty(account_id);
    state.pushed_forwards = serde_json::from_str(&forwards).unwrap_or_default();
    state.possibly_live = serde_json::from_str(&possibly).unwrap_or_default();
    Ok(state
        .live_forwards(rule_id)
        .into_iter()
        .filter(|forward| forward.to.trim().eq_ignore_ascii_case(to.trim()))
        .filter(|forward| {
            let config = serde_json::from_str::<serde_json::Value>(&forward.trigger_config)
                .unwrap_or_default();
            matches_rule(&forward.trigger_type, &config, ctx)
        })
        .map(|forward| {
            chrono::DateTime::parse_from_rfc3339(&forward.since)
                .map(|since| since.with_timezone(&chrono::Utc))
                // Live without a recorded start: since always.
                .unwrap_or(chrono::DateTime::<chrono::Utc>::MIN_UTC)
        })
        .min())
}

/// Whether the rule has handed [`satchel_core::rules::FORWARDS_PER_HOUR`]
/// forwards to the outbox within the last hour, counted from the person's
/// last resume at the earliest. Only forwards that passed the loop guard
/// count: a flood the guard stops is no flood.
pub(crate) async fn forward_limit_reached<'e, E>(
    executor: E,
    rule_id: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<bool, String>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM rule_forwards
         WHERE rule_token = ?1 AND outcome = 'outbox' AND created_at > ?2
           AND created_at > COALESCE((SELECT forward_resumed_at FROM rules WHERE id = ?3), '')",
    )
    .bind(rule_token(rule_id))
    .bind((now - chrono::Duration::hours(1)).to_rfc3339())
    .bind(rule_id)
    .fetch_one(executor)
    .await
    .map_err(|error| error.to_string())?;
    Ok(count >= satchel_core::rules::FORWARDS_PER_HOUR)
}

/// Pause a rule's forward: the circuit breaker tripped.
pub(crate) async fn pause_forwarding<'e, E>(
    executor: E,
    rule_id: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<(), String>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    sqlx::query(
        "UPDATE rules SET forward_paused_at = ? WHERE id = ? AND forward_paused_at IS NULL",
    )
    .bind(now.to_rfc3339())
    .bind(rule_id)
    .execute(executor)
    .await
    .map_err(|error| error.to_string())?;
    tracing::warn!(%rule_id, "rule forward paused: too many forwards within an hour");
    Ok(())
}

/// Resume a rule's paused forward. Only this does: saving the rule does not,
/// and the hourly count starts again from now.
#[tauri::command]
pub async fn resume_rule_forwarding(
    pool: tauri::State<'_, SqlitePool>,
    rule_id: String,
) -> CommandResult<()> {
    Ok(resume_rule_forwarding_inner(pool.inner(), &rule_id).await?)
}

pub async fn resume_rule_forwarding_inner(pool: &SqlitePool, rule_id: &str) -> Result<(), String> {
    let changed = sqlx::query(
        "UPDATE rules SET forward_paused_at = NULL, forward_resumed_at = ? WHERE id = ?",
    )
    .bind(chrono::Utc::now().to_rfc3339())
    .bind(rule_id.trim())
    .execute(pool)
    .await
    .map_err(|error| error.to_string())?
    .rows_affected();
    if changed == 0 {
        return Err("rule not found".into());
    }
    Ok(())
}

/// Queue a rule's forward of the message, unless it already went out (here,
/// on another account, or from the server), the server is the one to send
/// it, or the rule's forward is paused. Records the decision in
/// `rule_forwards` in the same transaction, so a second run of the rules, a
/// second copy of the message or a crash never sends it twice. `true` when a
/// forward was queued.
async fn forward_once(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    queue: &SyncQueue,
    row: &RuleRow,
    spec: &RuleSpec,
    ctx: &ThreadRuleContext,
    to: &str,
) -> Result<bool, String> {
    let Some(message_id) = ctx.message_id.as_deref() else {
        return Ok(false);
    };
    let message_key = ctx
        .message_id_header
        .as_deref()
        .map(str::trim)
        .filter(|header| !header.is_empty())
        .unwrap_or(message_id)
        .to_string();
    let token = rule_token(&spec.id);
    let now = chrono::Utc::now();
    let stamp = now.to_rfc3339();
    let record = |outcome: &'static str| {
        sqlx::query(
            "INSERT OR IGNORE INTO rule_forwards (account_id, rule_token, message_key, message_id, outcome, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(ctx.account_id.clone())
        .bind(token.clone())
        .bind(message_key.clone())
        .bind(message_id.to_string())
        .bind(outcome)
        .bind(stamp.clone())
        .bind(stamp.clone())
    };
    // Once per Message-ID across accounts: an unscoped rule that sees the same
    // mail on two accounts forwards it once.
    let recorded: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM rule_forwards WHERE rule_token = ? AND message_key = ?")
            .bind(&token)
            .bind(&message_key)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|error| error.to_string())?;
    if recorded.is_some() {
        return Ok(false);
    }
    let skip = |outcome: &'static str| record(outcome);
    if row.forward_paused_at.is_some() {
        skip("skipped_paused")
            .execute(&mut **tx)
            .await
            .map_err(|error| error.to_string())?;
        return Ok(false);
    }
    if matches!(ctx.folder_type.as_deref(), Some("spam" | "trash")) {
        skip("skipped_spam")
            .execute(&mut **tx)
            .await
            .map_err(|error| error.to_string())?;
        return Ok(false);
    }
    if !satchel_core::rules::is_forward_address(to) || is_own_address(&mut **tx, to).await? {
        skip("skipped_own_address")
            .execute(&mut **tx)
            .await
            .map_err(|error| error.to_string())?;
        return Ok(false);
    }
    let since = server_forward_since(tx, &ctx.account_id, &spec.id, to, ctx).await?;
    let received = ctx
        .received_at
        .as_deref()
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&chrono::Utc));
    let marked: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM rule_server_marks WHERE account_id = ? AND message_key = ?",
    )
    .bind(&ctx.account_id)
    .bind(&message_key)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|error| error.to_string())?;
    let decision = decide_forward(false, since, received, marked.is_some());
    let Some(outcome) = decision.outcome() else {
        return Ok(false);
    };
    if decision != ForwardDecision::Forward {
        record(outcome)
            .execute(&mut **tx)
            .await
            .map_err(|error| error.to_string())?;
        return Ok(false);
    }
    // The circuit breaker: a loop the guards missed, or a flood. Checked
    // again where the forward goes to the outbox, for a burst queued at once.
    if forward_limit_reached(&mut **tx, &spec.id, now).await? {
        pause_forwarding(&mut **tx, &spec.id, now).await?;
        skip("skipped_rate_limit")
            .execute(&mut **tx)
            .await
            .map_err(|error| error.to_string())?;
        return Ok(false);
    }
    record(outcome)
        .execute(&mut **tx)
        .await
        .map_err(|error| error.to_string())?;
    let payload = crate::sync::rule_forward::RuleForwardPayload {
        rule_id: spec.id.clone(),
        rule_token: token.clone(),
        message_id: message_id.to_string(),
        message_key: message_key.clone(),
        to: to.trim().to_string(),
    };
    let mut action = QueuedAction::new(
        ctx.account_id.clone(),
        satchel_core::sync::queue::ACTION_RULE_FORWARD,
        serde_json::to_value(&payload).map_err(|error| error.to_string())?,
    );
    action.id = payload.queue_id();
    action.status = "pending".into();
    queue.enqueue_in_transaction(tx, &action).await?;
    Ok(true)
}

fn to_rule_dto(row: RuleRow) -> RuleDto {
    RuleDto {
        id: row.id,
        name: row.name,
        description: row.description,
        trigger_type: row.trigger_type,
        trigger_config: serde_json::from_str(&row.trigger_config).unwrap_or_default(),
        actions: serde_json::from_str(&row.actions).unwrap_or_default(),
        is_active: row.is_active == 1,
        priority: row.priority,
        account_id: row.account_id,
        forward_paused_at: row.forward_paused_at,
        server_forward_skips: 0,
    }
}

fn validate_rule(input: &mut UpsertRuleInput) -> Result<(), String> {
    input.name = input.name.trim().to_string();
    if input.name.is_empty() || input.name.chars().count() > 160 {
        return Err("rule name must contain 1 to 160 characters".to_string());
    }
    validate_plain_value(&input.name, 160, "rule name")?;
    if let Some(id) = input.id.as_deref() {
        validate_plain_value(id, 320, "rule id")?;
    }
    input.description = input
        .description
        .take()
        .map(|description| description.trim().to_string())
        .filter(|description| !description.is_empty());
    if let Some(description) = input.description.as_deref() {
        validate_plain_value(description, 1_000, "description")?;
    }
    if input.trigger_type != "on_receive" {
        return Err("unsupported trigger_type".to_string());
    }
    input.priority = input.priority.clamp(-1000, 1000);
    let config = input
        .trigger_config
        .as_object()
        .ok_or_else(|| "trigger_config must be an object".to_string())?;
    let supported = [
        "from_contains",
        "to_contains",
        "subject_contains",
        "body_contains",
    ];
    let has_condition = supported.iter().any(|key| {
        config
            .get(*key)
            .and_then(serde_json::Value::as_str)
            .is_some_and(|value| !value.trim().is_empty())
    });
    if !has_condition {
        return Err("rule requires at least one condition".to_string());
    }
    for key in supported {
        if let Some(value) = config.get(key).and_then(serde_json::Value::as_str) {
            validate_plain_value(value.trim(), 500, key)?;
        }
    }
    if input.actions.is_empty() || input.actions.len() > 20 {
        return Err("rule requires between 1 and 20 actions".to_string());
    }
    for action in &mut input.actions {
        *action = action.trim().to_string();
        validate_action(action)?;
    }
    let parsed = satchel_core::rules::parse_actions(&input.actions).unwrap_or_default();
    let count = |kind: fn(&RuleAction) -> bool| parsed.iter().filter(|action| kind(action)).count();
    if count(|action| matches!(action, RuleAction::MoveToFolder(_))) > 1 {
        return Err("backend.errors.rules.actionsInvalid".to_string());
    }
    if count(|action| matches!(action, RuleAction::Forward { .. })) > 1 {
        return Err("backend.errors.rules.actionsInvalid".to_string());
    }
    input.account_id = input
        .account_id
        .take()
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty());
    if let Some(account_id) = input.account_id.as_deref() {
        validate_plain_value(account_id, 320, "account_id")?;
    }
    Ok(())
}

/// What a rule names in the database: its account, the folder it moves to
/// (one of that account's, not the Inbox), and a forward address that is not
/// one of Satchel's own.
async fn validate_rule_targets(pool: &SqlitePool, input: &UpsertRuleInput) -> Result<(), String> {
    if let Some(account_id) = input.account_id.as_deref() {
        let exists: Option<i64> = sqlx::query_scalar("SELECT 1 FROM accounts WHERE id = ?")
            .bind(account_id)
            .fetch_optional(pool)
            .await
            .map_err(|error| error.to_string())?;
        if exists.is_none() {
            return Err("backend.errors.accountNotFound".to_string());
        }
    }
    for action in satchel_core::rules::parse_actions(&input.actions).unwrap_or_default() {
        match action {
            RuleAction::MoveToFolder(folder_id) => {
                let Some(account_id) = input.account_id.as_deref() else {
                    return Err("backend.errors.rules.actionsInvalid".to_string());
                };
                let kind: Option<String> = sqlx::query_scalar(
                    "SELECT folder_type FROM folders WHERE id = ? AND account_id = ?",
                )
                .bind(&folder_id)
                .bind(account_id)
                .fetch_optional(pool)
                .await
                .map_err(|error| error.to_string())?;
                match kind {
                    None => return Err("backend.errors.folderAccountMismatch".to_string()),
                    Some(kind) if NOT_A_MOVE_TARGET.contains(&kind.as_str()) => {
                        return Err("backend.errors.rules.actionsInvalid".to_string())
                    }
                    Some(_) => {}
                }
            }
            RuleAction::Forward { to, .. } if is_own_address(pool, &to).await? => {
                return Err("backend.errors.rules.actionsInvalid".to_string());
            }
            _ => {}
        }
    }
    Ok(())
}

fn validate_plain_value(value: &str, max_len: usize, field: &str) -> Result<(), String> {
    if value.chars().count() > max_len {
        return Err(format!("{field} is too long"));
    }
    if value.chars().any(char::is_control) {
        return Err(format!("{field} contains control characters"));
    }
    Ok(())
}

fn validate_action(action: &str) -> Result<(), String> {
    let valid = if let Some(state) = action.strip_prefix("set_state:") {
        WorkflowState::from_db_value(state).is_some()
    } else if let Some(priority) = action.strip_prefix("set_priority:") {
        priority
            .parse::<i64>()
            .is_ok_and(|value| (0..=3).contains(&value))
    } else if let Some(category) = action.strip_prefix("set_category:") {
        matches!(
            category,
            "personal" | "updates" | "promotions" | "social" | "none"
        )
    } else if let Some(tag) = action.strip_prefix("add_tag:") {
        !tag.trim().is_empty() && tag.chars().count() <= 80 && !tag.chars().any(char::is_control)
    } else {
        match RuleAction::parse(action) {
            Some(RuleAction::MarkRead | RuleAction::Star) => true,
            Some(RuleAction::MoveToFolder(folder)) => {
                folder.chars().count() <= 320 && !folder.chars().any(char::is_control)
            }
            Some(RuleAction::Forward { to, .. }) => {
                if !satchel_core::rules::is_forward_address(&to) {
                    return Err("backend.errors.rules.actionsInvalid".to_string());
                }
                true
            }
            _ => false,
        }
    };
    valid
        .then_some(())
        .ok_or_else(|| format!("unsupported rule action: {action}"))
}

fn matches_rule(
    trigger_type: &str,
    trigger_config: &serde_json::Value,
    ctx: &ThreadRuleContext,
) -> bool {
    if trigger_type != "on_receive" {
        return false;
    }
    let recipients = format!(
        "{} {}",
        ctx.to_addresses.as_deref().unwrap_or_default(),
        ctx.cc_addresses.as_deref().unwrap_or_default()
    );
    let candidates = [
        (
            "from_contains",
            ctx.from_address.as_deref().unwrap_or_default(),
        ),
        ("to_contains", recipients.as_str()),
        (
            "subject_contains",
            ctx.subject.as_deref().unwrap_or_default(),
        ),
        (
            "body_contains",
            ctx.body_text
                .as_deref()
                .or(ctx.snippet.as_deref())
                .unwrap_or_default(),
        ),
    ];
    let results = candidates
        .into_iter()
        .filter_map(|(key, haystack)| {
            trigger_config
                .get(key)
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|needle| !needle.is_empty())
                .map(|needle| haystack.to_lowercase().contains(&needle.to_lowercase()))
        })
        .collect::<Vec<_>>();
    if results.is_empty() {
        return false;
    }
    if trigger_config
        .get("match")
        .and_then(serde_json::Value::as_str)
        == Some("any")
    {
        results.into_iter().any(|matched| matched)
    } else {
        results.into_iter().all(|matched| matched)
    }
}

async fn apply_action(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    thread_id: &str,
    action: &str,
) -> Result<(), String> {
    let now = chrono::Utc::now().to_rfc3339();
    if let Some(next_state) = action.strip_prefix("set_state:") {
        let next_state = WorkflowState::from_db_value(next_state)
            .ok_or_else(|| "invalid workflow state".to_string())?;
        let previous =
            sqlx::query_scalar::<_, String>("SELECT workflow_state FROM threads WHERE id = ?")
                .bind(thread_id)
                .fetch_one(&mut **tx)
                .await
                .map_err(|err| err.to_string())?;
        sqlx::query(
            "UPDATE threads SET workflow_state = ?, board_column = ?, workflow_changed_at = ?, updated_at = ? WHERE id = ?",
        )
        .bind(next_state.as_db_value())
        .bind(next_state.as_db_value())
        .bind(&now)
        .bind(&now)
        .bind(thread_id)
        .execute(&mut **tx)
        .await
        .map_err(|err| err.to_string())?;
        sqlx::query(
            "INSERT INTO workflow_events (id, thread_id, from_state, to_state, reason, created_at) VALUES (?, ?, ?, ?, 'rule', ?)",
        )
        .bind(format!("wf-{}", uuid::Uuid::new_v4()))
        .bind(thread_id)
        .bind(previous)
        .bind(next_state.as_db_value())
        .bind(&now)
        .execute(&mut **tx)
        .await
        .map_err(|err| err.to_string())?;
        return Ok(());
    }
    if let Some(priority) = action.strip_prefix("set_priority:") {
        let priority = priority.parse::<i64>().map_err(|err| err.to_string())?;
        sqlx::query(
            "UPDATE threads
             SET priority = ?, priority_source = 'manual_thread', updated_at = ?
             WHERE id = ?",
        )
        .bind(priority)
        .bind(&now)
        .bind(thread_id)
        .execute(&mut **tx)
        .await
        .map_err(|err| err.to_string())?;
        return Ok(());
    }
    if let Some(category) = action.strip_prefix("set_category:") {
        let category = (category != "none").then_some(category);
        sqlx::query(
            "UPDATE threads
             SET category = ?, category_source = 'manual_thread', updated_at = ?
             WHERE id = ?",
        )
        .bind(category)
        .bind(&now)
        .bind(thread_id)
        .execute(&mut **tx)
        .await
        .map_err(|err| err.to_string())?;
        return Ok(());
    }
    if action == "mark_read" {
        satchel_core::mail_actions::set_flags_in_transaction(
            tx,
            &SyncQueue::new(),
            &[thread_id.to_string()],
            Some(true),
            None,
            None,
        )
        .await?;
        return Ok(());
    }
    if let Some(tag) = action.strip_prefix("add_tag:") {
        let tag = tag.trim().to_lowercase();
        let tag_id = sqlx::query_scalar::<_, String>(
            "SELECT id FROM tags WHERE LOWER(name) = LOWER(?) ORDER BY id LIMIT 1",
        )
        .bind(&tag)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|err| err.to_string())?
        .unwrap_or_else(|| format!("tag-{}", uuid::Uuid::new_v4()));
        sqlx::query(
            "INSERT INTO tags (id, name, is_system, sort_order) VALUES (?, ?, 0, 0) ON CONFLICT(id) DO NOTHING",
        )
        .bind(&tag_id)
        .bind(&tag)
        .execute(&mut **tx)
        .await
        .map_err(|err| err.to_string())?;
        sqlx::query(
            "INSERT INTO thread_tags (thread_id, tag_id) VALUES (?, ?) ON CONFLICT(thread_id, tag_id) DO NOTHING",
        )
        .bind(thread_id)
        .bind(tag_id)
        .execute(&mut **tx)
        .await
        .map_err(|err| err.to_string())?;
        return Ok(());
    }
    Err(format!("unsupported rule action: {action}"))
}

#[cfg(test)]
mod tests {
    use super::{matches_rule, validate_action, ThreadRuleContext};
    use sqlx::sqlite::SqlitePoolOptions;
    use sqlx::SqlitePool;

    const CONFIG: &str = r#"{"from_contains":"billing@"}"#;
    const ACTIONS: &str = r#"["add_tag:billing"]"#;

    /// One IMAP account, one rule, one newly received inbox message waiting
    /// for the ingest rules, and the account's server running that rule.
    async fn seeded_with_server_rule() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("../crates/satchel-core/src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
        for statement in [
            "INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
             VALUES ('acc', 'Me', 'me@example.com', 'imap', '{}', '', '2026-01-01', '2026-01-01')",
            "INSERT INTO folders (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at)
             VALUES ('inbox', 'acc', 'INBOX', 'INBOX', 'inbox', 'INBOX', '2026-01-01', '2026-01-01')",
            "INSERT INTO threads (id, account_id, subject, last_message_date, workflow_state, board_column, created_at, updated_at)
             VALUES ('th', 'acc', 'Invoice', '2026-06-01T10:00:00Z', 'inbox', 'inbox', '2026-01-01', '2026-01-01')",
            "INSERT INTO messages (id, account_id, folder_id, thread_id, from_address, to_addresses, subject, date, received_at, created_at, updated_at)
             VALUES ('msg', 'acc', 'inbox', 'th', 'billing@example.com', '[]', 'Invoice', '2026-06-01T10:00:00Z', '2026-06-01T10:00:05Z', '2026-01-01', '2026-01-01')",
            "INSERT INTO ingest_effects (message_id, thread_id, account_id, run_rules, created_at)
             VALUES ('msg', 'th', 'acc', 1, '2026-06-01')",
        ] {
            sqlx::query(statement).execute(&pool).await.unwrap();
        }
        sqlx::query(
            "INSERT INTO rules (id, name, is_active, priority, trigger_type, trigger_config, actions, trigger_count, created_at, updated_at)
             VALUES ('rule-1', 'Billing', 1, 0, 'on_receive', ?, ?, 0, '2026-01-01', '2026-01-01')",
        )
        .bind(CONFIG)
        .bind(ACTIONS)
        .execute(&pool)
        .await
        .unwrap();
        let fingerprint =
            satchel_core::server_rules::compiler::rule_fingerprint("on_receive", CONFIG, ACTIONS);
        sqlx::query(
            "INSERT INTO managesieve_accounts (account_id, enabled, server_active, pushed_rules, updated_at)
             VALUES ('acc', 1, 1, ?, '2026-01-01')",
        )
        .bind(serde_json::json!({ "rule-1": fingerprint }).to_string())
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    #[tokio::test]
    async fn a_rule_the_server_also_runs_still_runs_locally() {
        let pool = seeded_with_server_rule().await;
        super::run_pending_ingest_rules(&pool, &[("msg".into(), "th".into())])
            .await
            .unwrap();
        let count: i64 = sqlx::query_scalar("SELECT trigger_count FROM rules WHERE id = 'rule-1'")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 1);
        let tagged: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM thread_tags")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(tagged, 1);
    }

    /// One IMAP account with an Inbox message from billing@ that waits for
    /// the ingest rules, a custom folder, and `rules` as `(id, actions)`.
    async fn seeded(rules: &[(&str, &str, Option<&str>)]) -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("../crates/satchel-core/src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
        for statement in [
            "INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
             VALUES ('acc', 'Me', 'me@example.test', 'imap', '{}', '', '2026-01-01', '2026-01-01')",
            "INSERT INTO account_aliases (id, account_id, email, created_at, updated_at) VALUES ('alias', 'acc', 'alias@example.test', '2026-01-01', '2026-01-01')",
            "INSERT INTO folders (id, account_id, name, full_path, folder_type, remote_id, uidvalidity, created_at, updated_at)
             VALUES ('inbox', 'acc', 'INBOX', 'INBOX', 'inbox', 'INBOX', 7, '2026-01-01', '2026-01-01')",
            "INSERT INTO folders (id, account_id, name, full_path, folder_type, remote_id, uidvalidity, created_at, updated_at)
             VALUES ('billing', 'acc', 'Billing', 'Billing', 'custom', 'Billing', 8, '2026-01-01', '2026-01-01')",
            "INSERT INTO folders (id, account_id, name, full_path, folder_type, remote_id, uidvalidity, created_at, updated_at)
             VALUES ('archive', 'acc', 'Archive', 'Archive', 'archive', 'Archive', 9, '2026-01-01', '2026-01-01')",
            "INSERT INTO threads (id, account_id, subject, last_message_date, workflow_state, board_column, created_at, updated_at)
             VALUES ('th', 'acc', 'Invoice', '2026-06-01T10:00:00Z', 'inbox', 'inbox', '2026-01-01', '2026-01-01')",
            "INSERT INTO messages (id, account_id, folder_id, thread_id, message_id_header, from_address, to_addresses, subject, date, received_at, remote_id, remote_uid, remote_uidvalidity, created_at, updated_at)
             VALUES ('msg', 'acc', 'inbox', 'th', '<invoice@example.test>', 'billing@example.test', '[]', 'Invoice', '2026-06-01T10:00:00Z', '2026-06-01T10:00:05Z', '41', 41, 7, '2026-01-01', '2026-01-01')",
            "INSERT INTO ingest_effects (message_id, thread_id, account_id, run_rules, created_at)
             VALUES ('msg', 'th', 'acc', 1, '2026-06-01')",
        ] {
            sqlx::query(statement).execute(&pool).await.unwrap();
        }
        for (index, (id, actions, account)) in rules.iter().enumerate() {
            sqlx::query(
                "INSERT INTO rules (id, name, is_active, priority, trigger_type, trigger_config, actions, account_id, trigger_count, created_at, updated_at)
                 VALUES (?, ?, 1, ?, 'on_receive', ?, ?, ?, 0, '2026-01-01', '2026-01-01')",
            )
            .bind(id)
            .bind(format!("Rule {id}"))
            .bind(100 - index as i64)
            .bind(CONFIG)
            .bind(actions)
            .bind(account)
            .execute(&pool)
            .await
            .unwrap();
        }
        pool
    }

    async fn run(pool: &SqlitePool) {
        super::run_pending_ingest_rules(pool, &[("msg".into(), "th".into())])
            .await
            .unwrap();
    }

    async fn count(pool: &SqlitePool, sql: &str) -> i64 {
        sqlx::query_scalar(sql).fetch_one(pool).await.unwrap()
    }

    const FORWARDS: &str = "SELECT COUNT(*) FROM sync_queue WHERE action_type = 'rule_forward'";

    #[tokio::test]
    async fn a_forward_is_queued_once_whatever_runs_the_rules_again() {
        let pool = seeded(&[("fwd", r#"["forward:books@example.test"]"#, None)]).await;
        run(&pool).await;
        assert_eq!(count(&pool, FORWARDS).await, 1);
        let outcome = outcome(&pool).await;
        assert_eq!(outcome, "queued");
        // Arriving again (a second copy of the same mail under another id).
        sqlx::query("UPDATE messages SET id = 'msg-copy' WHERE id = 'msg'")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO ingest_effects (message_id, thread_id, account_id, run_rules, created_at)
             VALUES ('msg-copy', 'th', 'acc', 1, '2026-06-02')",
        )
        .execute(&pool)
        .await
        .unwrap();
        super::run_pending_ingest_rules(&pool, &[("msg-copy".into(), "th".into())])
            .await
            .unwrap();
        assert_eq!(count(&pool, FORWARDS).await, 1, "keyed by Message-ID");
    }

    #[tokio::test]
    async fn applying_rules_to_existing_mail_forwards_nothing() {
        let pool = seeded(&[("fwd", r#"["forward:books@example.test", "star"]"#, None)]).await;
        let applied = super::run_rules_for_thread_inner(&pool, "th")
            .await
            .unwrap();
        assert_eq!(applied, vec!["Rule fwd: star".to_string()]);
        assert_eq!(count(&pool, FORWARDS).await, 0);
    }

    #[tokio::test]
    async fn a_forward_recorded_on_another_account_is_not_repeated() {
        let pool = seeded(&[("fwd", r#"["forward:books@example.test", "star"]"#, None)]).await;
        sqlx::query(
            "INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
             VALUES ('acc-2', 'Other', 'other@example.test', 'imap', '{}', '', '2026-01-01', '2026-01-01')",
        )
        .execute(&pool)
        .await
        .unwrap();
        // The same mail arrived on the second account first and went out.
        sqlx::query(
            "INSERT INTO rule_forwards (account_id, rule_token, message_key, message_id, outcome, created_at, updated_at)
             VALUES ('acc-2', ?, '<invoice@example.test>', 'other-msg', 'outbox', '2026-06-01', '2026-06-01')",
        )
        .bind(satchel_core::rules::rule_token("fwd"))
        .execute(&pool)
        .await
        .unwrap();
        run(&pool).await;
        assert_eq!(count(&pool, FORWARDS).await, 0);
        assert_eq!(
            count(&pool, "SELECT is_starred FROM messages WHERE id = 'msg'").await,
            1,
            "the idempotent rest of the rule still runs"
        );
    }

    async fn fingerprint_of(pool: &SqlitePool, rule_id: &str) -> String {
        let actions: String = sqlx::query_scalar("SELECT actions FROM rules WHERE id = ?")
            .bind(rule_id)
            .fetch_one(pool)
            .await
            .unwrap();
        satchel_core::server_rules::compiler::rule_fingerprint("on_receive", CONFIG, &actions)
    }

    /// The server runs the rule as it is now, forwarding to `to`, since `since`.
    async fn server_runs(pool: &SqlitePool, rule_id: &str, since: &str) {
        server_forwards(pool, rule_id, CONFIG, "books@example.test", since).await;
    }

    async fn server_forwards(
        pool: &SqlitePool,
        rule_id: &str,
        config: &str,
        to: &str,
        since: &str,
    ) {
        let fingerprint = fingerprint_of(pool, rule_id).await;
        sqlx::query(
            "INSERT INTO managesieve_accounts (account_id, enabled, server_active, pushed_rules, pushed_rules_since, pushed_forwards, updated_at)
             VALUES ('acc', 1, 1, ?, ?, ?, '2026-01-01')",
        )
        .bind(serde_json::json!({ rule_id: fingerprint }).to_string())
        .bind(serde_json::json!({ rule_id: since }).to_string())
        .bind(
            serde_json::json!({ rule_id: {
                "trigger_type": "on_receive", "trigger_config": config, "to": to, "since": since
            }})
            .to_string(),
        )
        .execute(pool)
        .await
        .unwrap();
    }

    async fn outcome(pool: &SqlitePool) -> String {
        sqlx::query_scalar("SELECT outcome FROM rule_forwards WHERE account_id = 'acc'")
            .fetch_one(pool)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn a_push_that_failed_or_is_in_flight_keeps_both_versions_the_servers() {
        // v1 was verified; the push of v2 broke off after it may have written
        // (or is still running): `server_active` is off, both may run.
        let pool = seeded(&[("fwd", r#"["forward:books@example.test"]"#, None)]).await;
        let v1 = satchel_core::server_rules::compiler::rule_fingerprint(
            "on_receive",
            CONFIG,
            r#"["forward:old@example.test"]"#,
        );
        let v2 = fingerprint_of(&pool, "fwd").await;
        sqlx::query(
            "INSERT INTO managesieve_accounts (account_id, enabled, server_active, pushed_rules, pushed_rules_since, pushed_forwards, possibly_live_rules, updated_at)
             VALUES ('acc', 1, 0, ?, ?, ?, ?, '2026-01-01')",
        )
        .bind(serde_json::json!({ "fwd": v1 }).to_string())
        .bind(serde_json::json!({ "fwd": "2026-01-01T00:00:00+00:00" }).to_string())
        .bind(
            serde_json::json!({ "fwd": {
                "trigger_type": "on_receive", "trigger_config": CONFIG,
                "to": "old@example.test", "since": "2026-01-01T00:00:00+00:00"
            }})
            .to_string(),
        )
        .bind(
            serde_json::json!([{
                "rule_id": "fwd", "fingerprint": v2,
                "since": "2026-05-01T00:00:00+00:00", "script": "h2",
                "forward": {
                    "trigger_type": "on_receive", "trigger_config": CONFIG,
                    "to": "books@example.test", "since": "2026-05-01T00:00:00+00:00"
                }
            }])
            .to_string(),
        )
        .execute(&pool)
        .await
        .unwrap();
        run(&pool).await;
        assert_eq!(
            count(&pool, FORWARDS).await,
            0,
            "the new version may run there"
        );
        assert_eq!(outcome(&pool).await, "skipped_server_live");
    }

    #[tokio::test]
    async fn an_edit_that_keeps_the_forward_leaves_it_with_the_server() {
        let pool = seeded(&[("fwd", r#"["forward:books@example.test"]"#, None)]).await;
        server_runs(&pool, "fwd", "2026-05-01T00:00:00+00:00").await;
        // A pin added since: another rule version, the same forward.
        sqlx::query("UPDATE rules SET actions = '[\"star\", \"forward:books@example.test\"]'")
            .execute(&pool)
            .await
            .unwrap();
        run(&pool).await;
        assert_eq!(count(&pool, FORWARDS).await, 0);
        assert_eq!(outcome(&pool).await, "skipped_server_live");
    }

    #[tokio::test]
    async fn mail_that_matches_the_trigger_still_live_there_is_the_servers() {
        let pool = seeded(&[("fwd", r#"["forward:books@example.test"]"#, None)]).await;
        // The server still runs an older trigger the mail matches as well.
        server_forwards(
            &pool,
            "fwd",
            r#"{"subject_contains":"invoice"}"#,
            "books@example.test",
            "2026-05-01T00:00:00+00:00",
        )
        .await;
        run(&pool).await;
        assert_eq!(count(&pool, FORWARDS).await, 0);
        assert_eq!(outcome(&pool).await, "skipped_server_live");
    }

    #[tokio::test]
    async fn mail_the_live_trigger_does_not_match_is_forwarded_here() {
        let pool = seeded(&[("fwd", r#"["forward:books@example.test"]"#, None)]).await;
        server_forwards(
            &pool,
            "fwd",
            r#"{"subject_contains":"newsletter"}"#,
            "books@example.test",
            "2026-05-01T00:00:00+00:00",
        )
        .await;
        run(&pool).await;
        assert_eq!(count(&pool, FORWARDS).await, 1);
    }

    #[tokio::test]
    async fn a_server_mark_on_older_mail_is_the_servers_too() {
        let pool = seeded(&[("fwd", r#"["forward:books@example.test"]"#, None)]).await;
        // Live from July; this mail is from June, but the server marked it.
        server_runs(&pool, "fwd", "2026-07-01T00:00:00+00:00").await;
        sqlx::query(
            "INSERT INTO rule_server_marks (account_id, message_key, created_at)
             VALUES ('acc', '<invoice@example.test>', '2026-06-01')",
        )
        .execute(&pool)
        .await
        .unwrap();
        run(&pool).await;
        assert_eq!(count(&pool, FORWARDS).await, 0);
        assert_eq!(outcome(&pool).await, "server");
    }

    #[tokio::test]
    async fn a_mark_from_another_rule_does_not_stop_a_local_one() {
        let pool = seeded(&[("fwd", r#"["forward:books@example.test"]"#, None)]).await;
        sqlx::query(
            "INSERT INTO rule_server_marks (account_id, message_key, created_at)
             VALUES ('acc', '<invoice@example.test>', '2026-06-01')",
        )
        .execute(&pool)
        .await
        .unwrap();
        run(&pool).await;
        assert_eq!(
            count(&pool, FORWARDS).await,
            1,
            "this rule does not run on the server"
        );
    }

    async fn handed_to_the_outbox(pool: &SqlitePool, how_many: i64, minutes_ago: i64) {
        let at = (chrono::Utc::now() - chrono::Duration::minutes(minutes_ago)).to_rfc3339();
        for index in 0..how_many {
            sqlx::query(
                "INSERT INTO rule_forwards (account_id, rule_token, message_key, message_id, outcome, created_at, updated_at)
                 VALUES ('acc', ?, ?, 'm', 'outbox', ?, ?)",
            )
            .bind(satchel_core::rules::rule_token("fwd"))
            .bind(format!("<earlier-{minutes_ago}-{index}@example.test>"))
            .bind(&at)
            .bind(&at)
            .execute(pool)
            .await
            .unwrap();
        }
    }

    async fn paused(pool: &SqlitePool) -> bool {
        sqlx::query_scalar::<_, Option<String>>(
            "SELECT forward_paused_at FROM rules WHERE id = 'fwd'",
        )
        .fetch_one(pool)
        .await
        .unwrap()
        .is_some()
    }

    async fn rearrive(pool: &SqlitePool, id: &str) {
        sqlx::query("UPDATE messages SET id = ?, message_id_header = ? WHERE thread_id = 'th'")
            .bind(id)
            .bind(format!("<{id}@example.test>"))
            .execute(pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO ingest_effects (message_id, thread_id, account_id, run_rules, created_at)
             VALUES (?, 'th', 'acc', 1, '2026-06-02')",
        )
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
        super::run_pending_ingest_rules(pool, &[(id.into(), "th".into())])
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn too_many_forwards_within_an_hour_pause_the_rule_until_resumed() {
        let pool = seeded(&[("fwd", r#"["forward:books@example.test"]"#, None)]).await;
        // Skipped forwards (the loop guard stopped them) are no flood.
        let recent = (chrono::Utc::now() - chrono::Duration::minutes(10)).to_rfc3339();
        for index in 0..30 {
            sqlx::query(
                "INSERT INTO rule_forwards (account_id, rule_token, message_key, message_id, outcome, created_at, updated_at)
                 VALUES ('acc', ?, ?, 'm', 'skipped_automatic', ?, ?)",
            )
            .bind(satchel_core::rules::rule_token("fwd"))
            .bind(format!("<guarded-{index}@example.test>"))
            .bind(&recent)
            .bind(&recent)
            .execute(&pool)
            .await
            .unwrap();
        }
        handed_to_the_outbox(&pool, satchel_core::rules::FORWARDS_PER_HOUR, 10).await;
        run(&pool).await;
        assert_eq!(count(&pool, FORWARDS).await, 0);
        assert!(paused(&pool).await);
        let listed = super::list_rules_inner(&pool).await.unwrap();
        assert!(listed[0].forward_paused_at.is_some(), "the editor shows it");

        // Saving the rule does not resume it.
        let rule = listed[0].clone();
        super::upsert_rule_inner(
            &pool,
            super::UpsertRuleInput {
                id: Some(rule.id),
                name: rule.name,
                description: rule.description,
                trigger_type: rule.trigger_type,
                trigger_config: rule.trigger_config,
                actions: rule.actions,
                is_active: rule.is_active,
                priority: rule.priority,
                account_id: rule.account_id,
            },
        )
        .await
        .unwrap();
        assert!(paused(&pool).await, "only the explicit resume does");

        // Resumed: the hour starts again, the earlier forwards do not count.
        super::resume_rule_forwarding_inner(&pool, "fwd")
            .await
            .unwrap();
        assert!(!paused(&pool).await);
        rearrive(&pool, "after-resume").await;
        assert_eq!(
            count(&pool, FORWARDS).await,
            1,
            "not re-tripped by the old window"
        );
        assert!(!paused(&pool).await);
    }

    #[tokio::test]
    async fn nothing_in_spam_is_forwarded() {
        let pool = seeded(&[("fwd", r#"["forward:books@example.test"]"#, None)]).await;
        sqlx::query(
            "INSERT INTO folders (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at)
             VALUES ('junk', 'acc', 'Junk', 'Junk', 'spam', 'Junk', '2026-01-01', '2026-01-01')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("UPDATE messages SET folder_id = 'junk'")
            .execute(&pool)
            .await
            .unwrap();
        run(&pool).await;
        assert_eq!(count(&pool, FORWARDS).await, 0);
        assert_eq!(outcome(&pool).await, "skipped_spam");
    }

    #[tokio::test]
    async fn mail_that_arrived_while_the_server_ran_the_rule_is_left_to_it() {
        // No keyword (a server that drops custom flags): still not sent twice.
        let pool = seeded(&[("fwd", r#"["forward:books@example.test"]"#, None)]).await;
        server_runs(&pool, "fwd", "2026-05-01T00:00:00+00:00").await;
        run(&pool).await;
        assert_eq!(count(&pool, FORWARDS).await, 0);
        let outcome = outcome(&pool).await;
        assert_eq!(outcome, "skipped_server_live");
    }

    #[tokio::test]
    async fn mail_from_before_the_server_had_the_rule_is_forwarded_here() {
        let pool = seeded(&[("fwd", r#"["forward:books@example.test"]"#, None)]).await;
        server_runs(&pool, "fwd", "2026-07-01T00:00:00+00:00").await;
        run(&pool).await;
        assert_eq!(count(&pool, FORWARDS).await, 1);
    }

    #[tokio::test]
    async fn a_new_address_the_server_does_not_have_yet_forwards_here() {
        let pool = seeded(&[("fwd", r#"["forward:books@example.test"]"#, None)]).await;
        server_runs(&pool, "fwd", "2026-05-01T00:00:00+00:00").await;
        sqlx::query("UPDATE rules SET actions = '[\"forward:other@example.test\"]'")
            .execute(&pool)
            .await
            .unwrap();
        run(&pool).await;
        assert_eq!(count(&pool, FORWARDS).await, 1);
    }

    #[tokio::test]
    async fn a_forward_to_an_own_address_never_goes_out() {
        let pool = seeded(&[("fwd", r#"["forward:ALIAS+lists@example.test"]"#, None)]).await;
        run(&pool).await;
        assert_eq!(count(&pool, FORWARDS).await, 0);
        let outcome = outcome(&pool).await;
        assert_eq!(outcome, "skipped_own_address");
    }

    #[tokio::test]
    async fn the_first_matching_move_wins_after_the_flags() {
        let pool = seeded(&[
            (
                "first",
                r#"["star", "move_to_folder:billing"]"#,
                Some("acc"),
            ),
            (
                "second",
                r#"["move_to_folder:archive", "mark_read"]"#,
                Some("acc"),
            ),
        ])
        .await;
        run(&pool).await;
        let (folder, pending, read, starred): (String, Option<String>, i64, i64) = sqlx::query_as(
            "SELECT folder_id, pending_folder_id, is_read, is_starred FROM messages WHERE id = 'msg'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(folder, "inbox", "stays until the server confirms the move");
        assert_eq!(pending.as_deref(), Some("billing"));
        assert_eq!((read, starred), (1, 1), "both rules' flags apply");
        // Flags first, then the move: the UIDs they name are the Inbox's.
        let kinds: Vec<String> = sqlx::query_scalar(
            "SELECT json_extract(payload, '$.kind') FROM sync_queue WHERE action_type = 'remote_mutation' ORDER BY rowid",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(
            kinds.last().map(String::as_str),
            Some("workflow_move"),
            "{kinds:?}"
        );
        assert!(
            kinds[..kinds.len() - 1].iter().all(|kind| kind == "flags"),
            "{kinds:?}"
        );
    }

    #[tokio::test]
    async fn a_rule_for_another_account_does_not_run() {
        let pool = seeded(&[("star", r#"["star"]"#, Some("someone-else"))]).await;
        run(&pool).await;
        assert_eq!(
            count(&pool, "SELECT is_starred FROM messages WHERE id = 'msg'").await,
            0
        );
        assert_eq!(
            count(&pool, "SELECT trigger_count FROM rules WHERE id = 'star'").await,
            0
        );
    }

    #[tokio::test]
    async fn a_move_needs_a_folder_of_the_rules_account_and_never_the_inbox() {
        let pool = seeded(&[]).await;
        let input = |actions: &[&str], account: Option<&str>| super::UpsertRuleInput {
            id: None,
            name: "Move".into(),
            description: None,
            trigger_type: "on_receive".into(),
            trigger_config: serde_json::json!({"from_contains": "billing@"}),
            actions: actions.iter().map(|action| action.to_string()).collect(),
            is_active: true,
            priority: 0,
            account_id: account.map(String::from),
        };
        let error = |result: Result<super::RuleDto, String>| result.unwrap_err();
        assert_eq!(
            error(super::upsert_rule_inner(&pool, input(&["move_to_folder:billing"], None)).await),
            "backend.errors.rules.actionsInvalid"
        );
        assert_eq!(
            error(
                super::upsert_rule_inner(&pool, input(&["move_to_folder:inbox"], Some("acc")))
                    .await
            ),
            "backend.errors.rules.actionsInvalid",
            "moving into the Inbox would run the rules again"
        );
        assert_eq!(
            error(
                super::upsert_rule_inner(&pool, input(&["move_to_folder:elsewhere"], Some("acc")))
                    .await
            ),
            "backend.errors.folderAccountMismatch"
        );
        assert_eq!(
            error(
                super::upsert_rule_inner(
                    &pool,
                    input(
                        &["move_to_folder:billing", "move_to_folder:archive"],
                        Some("acc")
                    )
                )
                .await
            ),
            "backend.errors.rules.actionsInvalid"
        );
        assert_eq!(
            error(super::upsert_rule_inner(&pool, input(&["forward:me@example.test"], None)).await),
            "backend.errors.rules.actionsInvalid"
        );
        assert_eq!(
            error(
                super::upsert_rule_inner(&pool, input(&["forward:me+tag@example.test"], None))
                    .await
            ),
            "backend.errors.rules.actionsInvalid",
            "a plus-alias of an own address"
        );
        assert_eq!(
            error(super::upsert_rule_inner(&pool, input(&["forward:not an address"], None)).await),
            "backend.errors.rules.actionsInvalid"
        );
        let saved = super::upsert_rule_inner(
            &pool,
            input(
                &[
                    "star",
                    "move_to_folder:billing",
                    "forward_no_copy:books@example.test",
                ],
                Some("acc"),
            ),
        )
        .await
        .unwrap();
        assert_eq!(saved.account_id.as_deref(), Some("acc"));
        let listed = super::list_rules_inner(&pool).await.unwrap();
        assert_eq!(listed[0].actions.len(), 3);
        assert_eq!(listed[0].account_id.as_deref(), Some("acc"));
    }

    fn context() -> ThreadRuleContext {
        ThreadRuleContext {
            subject: Some("Rechnung Juli".to_string()),
            snippet: Some("Bitte prüfen".to_string()),
            from_address: Some("billing@example.com".to_string()),
            to_addresses: Some("[\"me@example.com\"]".to_string()),
            cc_addresses: Some("[]".to_string()),
            body_text: Some("Zahlbar bis Freitag".to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn rule_conditions_support_all_and_any_matching() {
        assert!(matches_rule(
            "on_receive",
            &serde_json::json!({
                "match": "all",
                "from_contains": "billing@",
                "subject_contains": "rechnung"
            }),
            &context(),
        ));
        assert!(matches_rule(
            "on_receive",
            &serde_json::json!({
                "match": "any",
                "from_contains": "nobody",
                "body_contains": "freitag"
            }),
            &context(),
        ));
        assert!(!matches_rule(
            "on_receive",
            &serde_json::json!({"from_contains": "nobody"}),
            &context(),
        ));
    }

    #[test]
    fn actions_reject_unknown_or_unsafe_values() {
        assert!(validate_action("set_state:done").is_ok());
        assert!(validate_action("add_tag:rechnung").is_ok());
        assert!(validate_action("set_priority:9").is_err());
        assert!(validate_action("add_tag:bad\nvalue").is_err());
        assert!(validate_action("shell:rm").is_err());
        assert!(validate_action("star").is_ok());
        assert!(validate_action("move_to_folder:folder-1").is_ok());
        assert!(validate_action("move_to_folder:").is_err());
        assert!(validate_action("forward:books@example.test").is_ok());
        assert!(validate_action("forward_no_copy:books@example.test").is_ok());
        assert!(validate_action("forward:Books <books@example.test>").is_err());
    }
}
