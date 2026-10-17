use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

use crate::commands::error::CommandResult;
use crate::sync::queue::SyncQueue;
use crate::workflow::states::WorkflowState;

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
}

#[derive(Debug, FromRow)]
struct ThreadRuleContext {
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
    let rows = sqlx::query_as::<_, RuleRow>(
        "SELECT id, name, description, trigger_type, trigger_config, actions, is_active, priority
         FROM rules
         ORDER BY priority DESC, created_at ASC",
    )
    .fetch_all(pool)
    .await
    .map_err(|err| err.to_string())?;
    Ok(rows.into_iter().map(to_rule_dto).collect())
}

#[tauri::command]
pub async fn upsert_rule(
    pool: tauri::State<'_, SqlitePool>,
    input: UpsertRuleInput,
) -> CommandResult<RuleDto> {
    Ok(upsert_rule_inner(pool.inner(), input).await?)
}

pub async fn upsert_rule_inner(
    pool: &SqlitePool,
    mut input: UpsertRuleInput,
) -> Result<RuleDto, String> {
    validate_rule(&mut input)?;
    let now = chrono::Utc::now().to_rfc3339();
    let id = input
        .id
        .clone()
        .unwrap_or_else(|| format!("rule-{}", uuid::Uuid::new_v4()));
    let trigger_config =
        serde_json::to_string(&input.trigger_config).map_err(|err| err.to_string())?;
    let actions = serde_json::to_string(&input.actions).map_err(|err| err.to_string())?;

    sqlx::query(
        "INSERT INTO rules (id, name, description, is_active, priority, trigger_type, trigger_config, actions, last_triggered, trigger_count, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, NULL, 0, ?, ?)
         ON CONFLICT(id) DO UPDATE SET
             name = excluded.name,
             description = excluded.description,
             is_active = excluded.is_active,
             priority = excluded.priority,
             trigger_type = excluded.trigger_type,
             trigger_config = excluded.trigger_config,
             actions = excluded.actions,
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
            "SELECT subject, snippet, from_address, to_addresses, cc_addresses, body_text FROM messages WHERE id = ? AND thread_id = ?"
        ).bind(message_id).bind(thread_id).fetch_one(&mut **tx).await.map_err(|error| error.to_string())?
    } else {
        sqlx::query_as::<_, ThreadRuleContext>(
        "SELECT
           threads.subject,
           threads.snippet,
           (SELECT from_address FROM messages WHERE thread_id = threads.id ORDER BY date DESC, id DESC LIMIT 1) AS from_address,
           (SELECT to_addresses FROM messages WHERE thread_id = threads.id ORDER BY date DESC, id DESC LIMIT 1) AS to_addresses,
           (SELECT cc_addresses FROM messages WHERE thread_id = threads.id ORDER BY date DESC, id DESC LIMIT 1) AS cc_addresses,
           (SELECT body_text FROM messages WHERE thread_id = threads.id ORDER BY date DESC, id DESC LIMIT 1) AS body_text
         FROM threads WHERE id = ? LIMIT 1",
    )
    .bind(thread_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|err| err.to_string())?
    .ok_or_else(|| "thread not found".to_string())?
    };

    let rows = sqlx::query_as::<_, RuleRow>(
        "SELECT id, name, description, trigger_type, trigger_config, actions, is_active, priority
         FROM rules WHERE is_active = 1 ORDER BY priority DESC, created_at ASC",
    )
    .fetch_all(&mut **tx)
    .await
    .map_err(|err| err.to_string())?;

    let mut applied = Vec::new();
    for row in rows {
        let trigger_config =
            serde_json::from_str::<serde_json::Value>(&row.trigger_config).unwrap_or_default();
        if !matches_rule(&row.trigger_type, &trigger_config, &ctx) {
            continue;
        }
        let actions = serde_json::from_str::<Vec<String>>(&row.actions).unwrap_or_default();
        for action in actions {
            apply_action(tx, thread_id, &action).await?;
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
    Ok(applied)
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
        action == "mark_read"
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

    fn context() -> ThreadRuleContext {
        ThreadRuleContext {
            subject: Some("Rechnung Juli".to_string()),
            snippet: Some("Bitte prüfen".to_string()),
            from_address: Some("billing@example.com".to_string()),
            to_addresses: Some("[\"me@example.com\"]".to_string()),
            cc_addresses: Some("[]".to_string()),
            body_text: Some("Zahlbar bis Freitag".to_string()),
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
    }
}
