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
        MailMergeError::ReplyNotAllowed => {
            CommandError::new("backend.errors.mailMergeReplyNotAllowed")
        }
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
        CsvError::TooManyColumns { limit } => {
            CommandError::new("backend.errors.mailMergeCsvTooManyColumns")
                .with_param("limit", limit as u64)
        }
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
    /// The composer's signature choice, so the preview shows the signature
    /// `create_mail_merge_job` appends. Without a variant there is none.
    #[serde(default)]
    pub signature: Option<MergeSignatureInput>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/mail_merge/", optional_fields = nullable))]
pub struct MergeSignatureInput {
    #[serde(default)]
    pub from_address: Option<String>,
    #[serde(default)]
    pub signature_id: Option<String>,
    #[serde(default)]
    pub signature_variant: Option<String>,
    #[serde(default)]
    pub signature_override_text: Option<String>,
    #[serde(default)]
    pub signature_override_html: Option<String>,
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
    let mut input = input;
    if let (Some(account_id), Some(signature)) =
        (input.account_id.as_deref(), input.signature.take())
    {
        input.template = with_signature(pool, account_id, &input.template, signature).await?;
    }
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

/// The template with the signature `create_mail_merge_job` would append.
async fn with_signature(
    pool: &SqlitePool,
    account_id: &str,
    template: &MergeTemplate,
    signature: MergeSignatureInput,
) -> CommandResult<MergeTemplate> {
    let mut message = OutgoingMessageInput {
        subject: template.subject.clone(),
        body: template.body.clone(),
        body_html: template.body_html.clone(),
        from_address: signature.from_address,
        signature_id: signature.signature_id,
        signature_variant: signature.signature_variant,
        signature_override_text: signature.signature_override_text,
        signature_override_html: signature.signature_override_html,
        ..OutgoingMessageInput::default()
    };
    apply_selected_signature(pool, account_id, &mut message).await?;
    Ok(merge_template(&message))
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
    /// The client's token for this start. Sent again after a lost answer, it
    /// returns the job already made instead of a second one.
    #[serde(default)]
    pub request_id: Option<String>,
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
        request_id,
    } = input;
    let mut message = message;
    // A merge is a new message; the conversation's quote and In-Reply-To
    // would reach people who were never part of it.
    if message.thread_id.is_some() || message.reply_to_message_id.is_some() {
        return Err(MailMergeError::ReplyNotAllowed.into());
    }
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
    ensure_crypto_ready(&app, &state, pool, &account_id, &message, &recipients).await?;

    let job_id = job::create_job(
        pool,
        NewMailMergeJob {
            account_id,
            message,
            source_label: source_label.trim().chars().take(200).collect(),
            delay_seconds,
            request_id,
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

/// What the outbox will need for every copy, checked before the first one
/// leaves: the signing key or certificate unlocked, a key for every
/// recipient, and the sender's own key, since the Sent copy is encrypted to
/// it too. Finding the gap at copy 40 would leave a job half sent. A key
/// removed or locked mid-job still fails that copy in the outbox; nothing
/// goes out unencrypted.
async fn ensure_crypto_ready(
    app: &AppHandle,
    state: &AppState,
    pool: &SqlitePool,
    account_id: &str,
    message: &OutgoingMessageInput,
    recipients: &[(String, BTreeMap<String, String>)],
) -> CommandResult<()> {
    let pgp = message.pgp_sign_with_key_id.is_some() || message.pgp_encrypt_to_recipients;
    let smime = message.smime_sign_with_cert_id.is_some() || message.smime_encrypt_to_recipients;
    if !pgp && !smime {
        return Ok(());
    }
    if pgp && smime {
        return Err(CommandError::new("backend.errors.pgpSmimeConflict"));
    }
    let own_address = own_address(pool, account_id, message).await?;
    let emails: Vec<String> = recipients.iter().map(|(email, _)| email.clone()).collect();

    if let Some(key_id) = message.pgp_sign_with_key_id.as_deref() {
        crate::commands::pgp::load_private_signing_key(app, state, pool, key_id)
            .await
            .map_err(|_| CommandError::new("backend.errors.pgpSigningKeyUnavailable"))?;
    }
    if message.pgp_encrypt_to_recipients {
        let missing = crate::commands::pgp::missing_recipient_public_keys(pool, app, &emails)
            .await
            .map_err(CommandError::unexpected)?;
        missing_keys(&missing)?;
        let own = crate::commands::pgp::missing_recipient_public_keys(
            pool,
            app,
            std::slice::from_ref(&own_address),
        )
        .await
        .map_err(CommandError::unexpected)?;
        if !own.is_empty() {
            return Err(CommandError::new("backend.errors.pgpOwnKeyMissing"));
        }
    }

    if let Some(cert_id) = message.smime_sign_with_cert_id.as_deref() {
        crate::commands::smime::load_private_signing_material(app, state, pool, cert_id)
            .await
            .map_err(|_| CommandError::new("backend.errors.mailMergeSmimeSigningUnavailable"))?;
    }
    if message.smime_encrypt_to_recipients {
        match crate::commands::smime::resolve_recipient_certs(pool, &emails).await {
            Ok(_) => {}
            Err(SmimeRepositoryError::MissingRecipients { recipients })
            | Err(SmimeRepositoryError::AmbiguousRecipients { recipients }) => {
                missing_keys(&recipients)?
            }
            Err(error) => return Err(CommandError::from(error)),
        }
        match crate::commands::smime::resolve_recipient_certs(pool, &[own_address]).await {
            Ok(_) => {}
            Err(SmimeRepositoryError::MissingRecipients { .. }) => {
                return Err(CommandError::new(
                    "backend.errors.smimeOwnCertificateMissing",
                ))
            }
            Err(error) => return Err(CommandError::from(error)),
        }
    }
    Ok(())
}

/// The address the copies leave from, as the outbox resolves `From`.
async fn own_address(
    pool: &SqlitePool,
    account_id: &str,
    message: &OutgoingMessageInput,
) -> CommandResult<String> {
    if let Some(address) = message
        .from_address
        .as_deref()
        .map(str::trim)
        .filter(|address| !address.is_empty())
    {
        return Ok(address.to_string());
    }
    sqlx::query_scalar::<_, String>("SELECT email FROM accounts WHERE id = ?")
        .bind(account_id)
        .fetch_optional(pool)
        .await
        .map_err(CommandError::unexpected)?
        .ok_or_else(|| CommandError::new("backend.errors.accountNotFound"))
}

fn missing_keys(missing: &[String]) -> CommandResult<()> {
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
             VALUES ('acc', '', 'me@example.test', 'gmail', '{}', '', '', '')",
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
                rows: vec![row("a@example.test", "Anna"), row("b@example.test", "")],
                index: Some(1),
                signature: Some(MergeSignatureInput {
                    signature_variant: Some("none".into()),
                    ..MergeSignatureInput::default()
                }),
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
            &[
                row("A@EXAMPLE.TEST", "Anna"),
                row("bad", "Ben"),
                row("c@example.test", ""),
            ],
        )
        .unwrap();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].0, "A@example.test");
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

    #[tokio::test]
    async fn refuses_a_wide_csv_and_names_the_limit() {
        let header = (0..=200)
            .map(|i| format!("F{i}"))
            .collect::<Vec<_>>()
            .join(",");
        let error = parse_mail_merge_csv(ParseMailMergeCsvInput {
            content_base64: base64::engine::general_purpose::STANDARD
                .encode(format!("{header}\n1\n")),
        })
        .await
        .unwrap_err();
        assert_eq!(error.key, "backend.errors.mailMergeCsvTooManyColumns");
        assert_eq!(error.params.get("limit"), Some(&serde_json::json!(200)));
    }

    #[test]
    fn a_reply_is_refused_with_its_own_key() {
        assert_eq!(
            mail_merge_error(MailMergeError::ReplyNotAllowed).key,
            "backend.errors.mailMergeReplyNotAllowed"
        );
    }
}
