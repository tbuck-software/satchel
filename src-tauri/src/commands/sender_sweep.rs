//! Per-sender cleanup: the cleanup view's sender list, the threads a one-shot
//! sweep files, and the recurring sweep rules the scheduler runs.
//!
//! One-shot sweeps are not commands of their own. The frontend asks which
//! threads a sweep takes and files them through `transition_threads` and
//! `trash_threads` in batches, so the optimistic update, the receipt and the
//! Undo toast are the ones every other Done and Delete has.

use serde::Deserialize;
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter, Manager};

use crate::commands::error::{CommandError, CommandResult};
use crate::AppState;
use satchel_core::db::sender_sweep::{
    CleanupSender, SenderSweepError, SenderSweepRepository, SweepAction, SweepRuleRecord,
    SweepRuleUpsert, SweepSelection, SweepSummary, SweepTarget, ThreadSweepSender,
};
use satchel_core::mail_actions::sweep::SweepGate;

/// The cleanup view looks back this far unless told otherwise.
const DEFAULT_WINDOW_DAYS: i64 = 90;
const DEFAULT_SENDER_LIMIT: i64 = 50;

#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/sender_sweep/", optional_fields = nullable))]
pub struct ListCleanupSendersInput {
    #[serde(default)]
    pub days: Option<i64>,
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub account_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/sender_sweep/", optional_fields = nullable))]
pub struct SenderSweepTargetsInput {
    pub account_id: String,
    pub sender_email: String,
    #[serde(default)]
    pub keep_latest: bool,
    #[serde(default)]
    pub older_than_days: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/sender_sweep/", optional_fields = nullable))]
pub struct UpsertSweepRuleInput {
    pub account_id: String,
    pub sender_email: String,
    /// `archive`, `trash` or `move`.
    pub action: String,
    #[serde(default)]
    pub older_than_days: Option<i64>,
    #[serde(default)]
    pub keep_latest: bool,
    #[serde(default)]
    pub target_folder_id: Option<String>,
}

pub(crate) fn sender_sweep_error_key(error: SenderSweepError) -> String {
    match error {
        SenderSweepError::AccountIdEmpty => "backend.errors.senderSweepAccountInvalid",
        SenderSweepError::SenderEmailInvalid => "backend.errors.senderSweepSenderInvalid",
        SenderSweepError::OwnAddress => "backend.errors.senderSweepOwnAddress",
        SenderSweepError::DaysInvalid => "backend.errors.senderSweepDaysInvalid",
        SenderSweepError::TrashNeedsAge => "backend.errors.senderSweepTrashNeedsAge",
        SenderSweepError::ActionInvalid => "backend.errors.senderSweepActionInvalid",
        SenderSweepError::FolderInvalid => "backend.errors.senderSweepFolderInvalid",
        SenderSweepError::NotFound => "backend.errors.senderSweepRuleNotFound",
        SenderSweepError::Persistence(error) => {
            tracing::error!(error = ?error, "sender sweep persistence failed");
            "backend.errors.senderSweepPersistenceFailed"
        }
    }
    .to_owned()
}

impl From<SenderSweepError> for CommandError {
    fn from(error: SenderSweepError) -> Self {
        Self::new(sender_sweep_error_key(error))
    }
}

#[tauri::command]
pub async fn list_cleanup_senders(
    pool: tauri::State<'_, crate::db::ReadPool>,
    input: ListCleanupSendersInput,
) -> CommandResult<Vec<CleanupSender>> {
    list_cleanup_senders_inner(pool.inner().as_ref(), input, chrono::Utc::now()).await
}

pub(crate) async fn list_cleanup_senders_inner(
    pool: &SqlitePool,
    input: ListCleanupSendersInput,
    now: chrono::DateTime<chrono::Utc>,
) -> CommandResult<Vec<CleanupSender>> {
    let days = input.days.unwrap_or(DEFAULT_WINDOW_DAYS).clamp(1, 3650);
    let since = (now - chrono::Duration::days(days)).to_rfc3339();
    Ok(SenderSweepRepository::new(pool)
        .top_inbox_senders(
            &since,
            input.account_id.as_deref(),
            input.limit.unwrap_or(DEFAULT_SENDER_LIMIT),
        )
        .await?)
}

#[tauri::command]
pub async fn list_sender_sweep_targets(
    pool: tauri::State<'_, crate::db::ReadPool>,
    input: SenderSweepTargetsInput,
) -> CommandResult<Vec<SweepTarget>> {
    let older_than_days =
        satchel_core::db::sender_sweep::validate_older_than_days(input.older_than_days)?;
    let selection =
        SweepSelection::for_rule(older_than_days, input.keep_latest, chrono::Utc::now());
    Ok(SenderSweepRepository::new(pool.inner().as_ref())
        .targets(&input.account_id, &input.sender_email, &selection)
        .await?)
}

/// How much a sweep would take and what it leaves alone, without the ids:
/// the dialog asks again on every toggle.
#[tauri::command]
pub async fn sender_sweep_summary(
    pool: tauri::State<'_, crate::db::ReadPool>,
    input: SenderSweepTargetsInput,
) -> CommandResult<SweepSummary> {
    let older_than_days =
        satchel_core::db::sender_sweep::validate_older_than_days(input.older_than_days)?;
    let selection =
        SweepSelection::for_rule(older_than_days, input.keep_latest, chrono::Utc::now());
    Ok(SenderSweepRepository::new(pool.inner().as_ref())
        .plan(&input.account_id, &input.sender_email, &selection)
        .await?
        .summary())
}

/// Who started a thread, for a sweep opened from a list row or the keyboard.
#[tauri::command]
pub async fn thread_sweep_sender(
    pool: tauri::State<'_, crate::db::ReadPool>,
    thread_id: String,
) -> CommandResult<Option<ThreadSweepSender>> {
    Ok(SenderSweepRepository::new(pool.inner().as_ref())
        .thread_sender(thread_id.trim())
        .await?)
}

#[tauri::command]
pub async fn list_sweep_rules(
    pool: tauri::State<'_, crate::db::ReadPool>,
) -> CommandResult<Vec<SweepRuleRecord>> {
    Ok(SenderSweepRepository::new(pool.inner().as_ref())
        .list_rules()
        .await?)
}

/// Saves the rule and runs it once right away, so "from now on" includes the
/// mail that already qualifies; the scheduler repeats it from then on.
#[tauri::command]
pub async fn upsert_sweep_rule(
    app: AppHandle,
    pool: tauri::State<'_, SqlitePool>,
    input: UpsertSweepRuleInput,
) -> CommandResult<SweepRuleRecord> {
    let rule = upsert_sweep_rule_inner(pool.inner(), input).await?;
    let pool = pool.inner().clone();
    let rule_id = rule.id.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        let gate = app.state::<SweepGate>();
        // Through the gate: a scheduler pass in progress finishes first, so
        // the two never file the same threads or overwrite each other's run.
        match gate
            .run_rule_now(&pool, &state.queue, &rule_id, chrono::Utc::now())
            .await
        {
            Ok(outcome) => emit_swept(&app, outcome.swept),
            Err(error) => {
                tracing::warn!(target: "satchel::sweep", rule = %rule_id, %error, "first sweep run failed")
            }
        }
    });
    Ok(rule)
}

pub(crate) async fn upsert_sweep_rule_inner(
    pool: &SqlitePool,
    input: UpsertSweepRuleInput,
) -> CommandResult<SweepRuleRecord> {
    let action =
        SweepAction::from_db_value(input.action.trim()).ok_or(SenderSweepError::ActionInvalid)?;
    Ok(SenderSweepRepository::new(pool)
        .upsert_rule(SweepRuleUpsert {
            account_id: input.account_id,
            sender_email: input.sender_email,
            action,
            older_than_days: input.older_than_days,
            keep_latest: input.keep_latest,
            target_folder_id: input.target_folder_id,
        })
        .await?)
}

#[tauri::command]
pub async fn delete_sweep_rule(
    pool: tauri::State<'_, SqlitePool>,
    rule_id: String,
) -> CommandResult<()> {
    if !SenderSweepRepository::new(pool.inner())
        .delete_rule(rule_id.trim())
        .await?
    {
        return Err(SenderSweepError::NotFound.into());
    }
    Ok(())
}

/// Lists refresh the threads a sweep filed behind the user's back.
pub(crate) fn emit_swept(app: &AppHandle, thread_ids: Vec<String>) {
    if thread_ids.is_empty() {
        return;
    }
    let _ = app.emit(
        "thread:updated",
        serde_json::json!({ "thread_ids": thread_ids }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("pool");
        sqlx::migrate!("../crates/satchel-core/src/db/migrations")
            .run(&pool)
            .await
            .expect("migrations");
        pool
    }

    #[tokio::test]
    async fn an_unknown_action_is_a_message_key() {
        let pool = pool().await;
        let error = upsert_sweep_rule_inner(
            &pool,
            UpsertSweepRuleInput {
                account_id: "a".into(),
                sender_email: "news@shop.example".into(),
                action: "explode".into(),
                older_than_days: None,
                keep_latest: false,
                target_folder_id: None,
            },
        )
        .await
        .unwrap_err();
        assert_eq!(
            serde_json::to_value(&error).unwrap()["key"],
            "backend.errors.senderSweepActionInvalid"
        );
    }

    #[tokio::test]
    async fn the_sender_list_defaults_to_ninety_days() {
        let pool = pool().await;
        let senders = list_cleanup_senders_inner(
            &pool,
            ListCleanupSendersInput::default(),
            chrono::Utc::now(),
        )
        .await
        .unwrap();
        assert!(senders.is_empty());
    }
}
