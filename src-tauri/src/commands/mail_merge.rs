//! Mail merge: read a recipient list, preview each personalised copy, and
//! start, steer and inspect the jobs that send them.
//!
//! Rendering lives in `satchel_core::mail_merge`; the preview the user steps
//! through and the message the runner hands to the outbox come from the same
//! function. Sending is the runner's (`sync::mail_merge_runner`), through the
//! ordinary outbox.

use std::collections::BTreeMap;

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tauri::{AppHandle, State};

use crate::commands::error::{CommandError, CommandResult};
use crate::commands::messages::{
    apply_selected_signature, ensure_attachments_within_limit, resolve_sender,
    OutgoingMessageInput,
};
use crate::AppState;
use satchel_core::db::smime::SmimeRepositoryError;
use satchel_core::mail_merge::{
    csv::{parse_csv, CsvError, ParsedCsv},
    job::{
        self, default_delay_seconds, merge_template, MailMergeError, MailMergeJobDetail,
        MailMergeJobSummary, NewMailMergeJob,
    },
    plan::{
        plan, render_message, MergePlan, MergeRecipientInput, MergeTemplate, RenderedMessage,
        MAX_JOB_RECIPIENTS,
    },
};

pub(crate) fn mail_merge_error(error: MailMergeError) -> CommandError {
    match error {
        MailMergeError::NotFound => CommandError::new("backend.errors.mailMergeJobNotFound"),
        MailMergeError::NoRecipients => CommandError::new("backend.errors.mailMergeNoRecipients"),
        MailMergeError::TooManyRecipients { limit } => {
            CommandError::new("backend.errors.mailMergeTooManyRecipients")
                .with_param("limit", limit as u64)
        }
        MailMergeError::DelayInvalid => CommandError::new("backend.errors.mailMergeDelayInvalid"),
        MailMergeError::InvalidState => CommandError::new("backend.errors.mailMergeInvalidState"),
        error @ (MailMergeError::Queue(_)
        | MailMergeError::Persistence(_)
        | MailMergeError::Encoding(_)) => {
            tracing::error!(error = ?error, "mail merge persistence failed");
            CommandError::new("backend.errors.mailMergePersistenceFailed")
        }
    }
}

impl From<MailMergeError> for CommandError {
    fn from(error: MailMergeError) -> Self {
        mail_merge_error(error)
    }
}

fn csv_error(error: CsvError) -> CommandError {
    match error {
        CsvError::Empty => CommandError::new("backend.errors.mailMergeCsvEmpty"),
        CsvError::TooLarge => CommandError::new("backend.errors.mailMergeCsvTooLarge"),
        CsvError::NoRows => CommandError::new("backend.errors.mailMergeCsvNoRows"),
        CsvError::UnterminatedQuote { line } => {
            CommandError::new("backend.errors.mailMergeCsvUnterminatedQuote")
                .with_param("line", line)
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/mail_merge/")
)]
pub struct ParseMailMergeCsvInput {
    pub content_base64: String,
}

#[tauri::command]
pub async fn parse_mail_merge_csv(input: ParseMailMergeCsvInput) -> CommandResult<ParsedCsv> {
    // Measured on the encoded form: decoding a huge string only to reject it
    // would hold it twice.
    if input.content_base64.len() / 4 * 3 > satchel_core::mail_merge::csv::MAX_CSV_BYTES + 3 {
        return Err(csv_error(CsvError::TooLarge));
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(input.content_base64.trim())
        .map_err(|_| CommandError::new("backend.errors.mailMergeCsvUnreadable"))?;
    parse_csv(&bytes).map_err(csv_error)
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/mail_merge/", optional_fields = nullable))]
pub struct PreviewMailMergeInput {
    #[serde(default)]
    pub account_id: Option<String>,
    pub template: MergeTemplate,
    pub fields: Vec<String>,
    pub email_field: String,
    pub rows: Vec<MergeRecipientInput>,
    /// The row to render; out of range renders nothing.
    #[serde(default)]
    pub index: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/mail_merge/")
)]
pub struct MailMergePreview {
    pub plan: MergePlan,
    pub rendered: Option<RenderedMessage>,
    /// Fields that keep the previewed row from being sent.
    pub missing: Vec<String>,
    pub max_recipients: u32,
    /// The pace that suits the account's provider; 0 without an account.
    pub default_delay_seconds: i64,
}

#[tauri::command]
pub async fn preview_mail_merge(
    pool: State<'_, crate::db::ReadPool>,
    input: PreviewMailMergeInput,
) -> CommandResult<MailMergePreview> {
    preview_mail_merge_inner(pool.inner().as_ref(), input).await
}

pub(crate) async fn preview_mail_merge_inner(
    pool: &SqlitePool,
    input: PreviewMailMergeInput,
) -> CommandResult<MailMergePreview> {
    let plan = plan(
        &input.template,
        &input.fields,
        &input.email_field,
        &input.rows,
    );
    let (rendered, missing) = match input.index.and_then(|index| input.rows.get(index as usize)) {
        Some(row) => match render_message(&input.template, &row.values) {
            Ok(rendered) => (Some(rendered), Vec::new()),
            Err(missing) => (None, missing),
        },
        None => (None, Vec::new()),
    };
    let default_delay_seconds = match input.account_id.as_deref() {
        Some(account_id) => account_default_delay(pool, account_id).await?,
        None => 0,
    };
    Ok(MailMergePreview {
        plan,
        rendered,
        missing,
        max_recipients: MAX_JOB_RECIPIENTS as u32,
        default_delay_seconds,
    })
}

async fn account_default_delay(pool: &SqlitePool, account_id: &str) -> CommandResult<i64> {
    let account: Option<(String, String)> =
        sqlx::query_as("SELECT provider, email FROM accounts WHERE id = ?")
            .bind(account_id)
            .fetch_optional(pool)
            .await
            .map_err(CommandError::unexpected)?;
    let (provider, email) =
        account.ok_or_else(|| CommandError::new("backend.errors.accountNotFound"))?;
    Ok(default_delay_seconds(&provider, &email))
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/mail_merge/", optional_fields = nullable))]
pub struct CreateMailMergeJobInput {
    pub account_id: String,
    /// The composer's send input. To, Cc and Bcc are ignored: every
    /// recipient gets a copy addressed to them alone.
    pub message: OutgoingMessageInput,
    #[serde(default)]
    pub source_label: String,
    pub delay_seconds: i64,
    pub fields: Vec<String>,
    pub email_field: String,
    pub rows: Vec<MergeRecipientInput>,
}

#[tauri::command]
pub async fn create_mail_merge_job(
    app: AppHandle,
    state: State<'_, AppState>,
    pool: State<'_, SqlitePool>,
    input: CreateMailMergeJobInput,
) -> CommandResult<MailMergeJobSummary> {
    let pool = pool.inner();
    let CreateMailMergeJobInput {
        account_id,
        message,
        source_label,
        delay_seconds,
        fields,
        email_field,
        rows,
    } = input;
    let mut message = message;
    ensure_attachments_within_limit(&message.attachments)?;
    // The same sender rules as `send_message`: the chosen alias or the
    // account's own address, stored in the template so every copy, its queue
    // payload and its Sent copy carry it.
    message.from_address =
        Some(resolve_sender(pool, &account_id, message.from_address.as_deref()).await?);
    job::validate_delay(delay_seconds)?;

    // The signature joins the template once, so its placeholders render too.
    let draft_id = message.draft_id.take();
    apply_selected_signature(pool, &account_id, &mut message).await?;
    message.signature_variant = Some("none".into());
    message.signature_id = None;
    message.signature_override_text = None;
    message.signature_override_html = None;

    let recipients = ready_recipients(&merge_template(&message), &fields, &email_field, &rows)?;
    ensure_encryption_keys(&app, pool, &message, &recipients).await?;

    let job_id = job::create_job(
        pool,
        NewMailMergeJob {
            account_id,
            message,
            source_label: source_label.trim().chars().take(200).collect(),
            delay_seconds,
            recipients,
        },
        chrono::Utc::now(),
    )
    .await?;

    // The message now lives in the job; its draft would only send it twice.
    if let Some(draft_id) = draft_id {
        if let Err(error) = satchel_core::drafts::discard(pool, &state.queue, &draft_id).await {
            tracing::warn!(error = ?error, "mail merge draft cleanup failed");
        }
    }
    crate::sync::mail_merge_runner::wake();
    Ok(job::job_summary(pool, &job_id).await?)
}

/// The rows the plan lets through, each with its normalized address. A job
/// with nothing to send, or with more than the limit, is refused whole.
fn ready_recipients(
    template: &MergeTemplate,
    fields: &[String],
    email_field: &str,
    rows: &[MergeRecipientInput],
) -> CommandResult<Vec<(String, BTreeMap<String, String>)>> {
    let plan = plan(template, fields, email_field, rows);
    let ready: Vec<(String, BTreeMap<String, String>)> = plan
        .rows
        .into_iter()
        .filter(|row| row.issue.is_none())
        .map(|row| (row.email, rows[row.index as usize].values.clone()))
        .collect();
    if ready.is_empty() {
        return Err(MailMergeError::NoRecipients.into());
    }
    if ready.len() > MAX_JOB_RECIPIENTS {
        return Err(MailMergeError::TooManyRecipients {
            limit: MAX_JOB_RECIPIENTS,
        }
        .into());
    }
    Ok(ready)
}

/// With encryption on, every recipient needs a key before the first copy
/// leaves; finding the gap at message 40 would leave a job half sent.
async fn ensure_encryption_keys(
    app: &AppHandle,
    pool: &SqlitePool,
    message: &OutgoingMessageInput,
    recipients: &[(String, BTreeMap<String, String>)],
) -> CommandResult<()> {
    let emails: Vec<String> = recipients.iter().map(|(email, _)| email.clone()).collect();
    let missing = if message.pgp_encrypt_to_recipients {
        crate::commands::pgp::missing_recipient_public_keys(pool, app, &emails)
            .await
            .map_err(CommandError::unexpected)?
    } else if message.smime_encrypt_to_recipients {
        match crate::commands::smime::resolve_recipient_certs(pool, &emails).await {
            Ok(_) => Vec::new(),
            Err(SmimeRepositoryError::MissingRecipients { recipients })
            | Err(SmimeRepositoryError::AmbiguousRecipients { recipients }) => recipients,
            Err(error) => return Err(CommandError::from(error)),
        }
    } else {
        return Ok(());
    };
    if missing.is_empty() {
        return Ok(());
    }
    let shown: Vec<String> = missing.iter().take(5).cloned().collect();
    Err(CommandError::new("backend.errors.mailMergeMissingKeys")
        .with_param("count", missing.len() as u64)
        .with_param("addresses", shown.join(", ")))
}

#[tauri::command]
pub async fn list_mail_merge_jobs(
    pool: State<'_, crate::db::ReadPool>,
) -> CommandResult<Vec<MailMergeJobSummary>> {
    Ok(job::list_jobs(pool.inner().as_ref()).await?)
}

#[tauri::command]
pub async fn get_mail_merge_job(
    pool: State<'_, crate::db::ReadPool>,
    id: String,
) -> CommandResult<MailMergeJobDetail> {
    Ok(job::job_detail(pool.inner().as_ref(), &id).await?)
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/mail_merge/")
)]
#[serde(rename_all = "snake_case")]
pub enum MailMergeJobAction {
    Pause,
    Resume,
    Cancel,
    RetryFailed,
    Delete,
}

/// Pause, resume, cancel, retry the failed copies, or remove a finished job.
/// Answers with the job as it is afterwards; `None` once it is deleted.
#[tauri::command]
pub async fn control_mail_merge_job(
    pool: State<'_, SqlitePool>,
    id: String,
    action: MailMergeJobAction,
) -> CommandResult<Option<MailMergeJobSummary>> {
    control_mail_merge_job_inner(pool.inner(), &id, action).await
}

pub(crate) async fn control_mail_merge_job_inner(
    pool: &SqlitePool,
    id: &str,
    action: MailMergeJobAction,
) -> CommandResult<Option<MailMergeJobSummary>> {
    let now = chrono::Utc::now();
    match action {
        MailMergeJobAction::Pause => job::pause_job(pool, id, now).await?,
        MailMergeJobAction::Resume => job::resume_job(pool, id, now).await?,
        MailMergeJobAction::Cancel => job::cancel_job(pool, id, now).await?,
        MailMergeJobAction::RetryFailed => {
            job::retry_failed(pool, id, now).await?;
        }
        MailMergeJobAction::Delete => {
            job::delete_job(pool, id).await?;
            return Ok(None);
        }
    }
    crate::sync::mail_merge_runner::wake();
    Ok(Some(job::job_summary(pool, id).await?))
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn pool() -> SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("../crates/satchel-core/src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
             VALUES ('acc', '', 'me@gmail.com', 'gmail', '{}', '', '', '')",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    fn row(email: &str, first: &str) -> MergeRecipientInput {
        MergeRecipientInput {
            line: None,
            values: BTreeMap::from([
                ("Email".to_string(), email.to_string()),
                ("FirstName".to_string(), first.to_string()),
            ]),
        }
    }

    #[tokio::test]
    async fn previews_the_chosen_row_with_the_accounts_pace() {
        let pool = pool().await;
        let preview = preview_mail_merge_inner(
            &pool,
            PreviewMailMergeInput {
                account_id: Some("acc".into()),
                template: MergeTemplate {
                    subject: "Hi {{FirstName}}".into(),
                    body: "Dear {{FirstName}}".into(),
                    body_html: None,
                },
                fields: vec!["Email".into(), "FirstName".into()],
                email_field: "Email".into(),
                rows: vec![row("a@x.de", "Anna"), row("b@x.de", "")],
                index: Some(1),
            },
        )
        .await
        .unwrap();
        assert_eq!(preview.plan.ready, 1);
        assert_eq!(preview.rendered, None);
        assert_eq!(preview.missing, vec!["FirstName".to_string()]);
        assert_eq!(preview.default_delay_seconds, 15);
        assert_eq!(preview.max_recipients, 500);
    }

    #[test]
    fn keeps_only_ready_rows_and_refuses_an_empty_job() {
        let message: OutgoingMessageInput = serde_json::from_value(serde_json::json!({
            "to": [], "subject": "Hi {{FirstName}}", "body": "x",
        }))
        .unwrap();
        let fields = vec!["Email".to_string(), "FirstName".to_string()];
        let template = merge_template(&message);
        let ready = ready_recipients(
            &template,
            &fields,
            "Email",
            &[row("A@X.DE", "Anna"), row("bad", "Ben"), row("c@x.de", "")],
        )
        .unwrap();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].0, "A@x.de");
        let error =
            ready_recipients(&template, &fields, "Email", &[row("bad", "Ben")]).unwrap_err();
        assert_eq!(error.key, "backend.errors.mailMergeNoRecipients");
    }

    #[tokio::test]
    async fn steering_an_unknown_job_says_so() {
        let pool = pool().await;
        let error = control_mail_merge_job_inner(&pool, "nope", MailMergeJobAction::Pause)
            .await
            .unwrap_err();
        assert_eq!(error.key, "backend.errors.mailMergeJobNotFound");
    }
}
