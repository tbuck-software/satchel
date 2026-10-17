use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::commands::error::{CommandError, CommandResult};
use satchel_core::db::templates::{TemplateError, TemplateRepository, TemplateUpsertRequest};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/templates/")
)]
pub struct TemplateInput {
    pub id: Option<String>,
    #[serde(default)]
    pub account_id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub subject: Option<String>,
    #[serde(default)]
    pub body_text: String,
    #[serde(default)]
    pub body_html: Option<String>,
    #[serde(default)]
    pub shortcut: Option<String>,
    #[serde(default)]
    pub cc: Vec<String>,
    #[serde(default)]
    pub bcc: Vec<String>,
    #[serde(default)]
    pub attachments_json: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/templates/")
)]
pub struct TemplateDto {
    pub id: String,
    pub account_id: Option<String>,
    pub name: String,
    pub subject: Option<String>,
    pub body_text: String,
    pub body_html: Option<String>,
    pub shortcut: Option<String>,
    pub cc_json: Option<String>,
    pub bcc_json: Option<String>,
    pub attachments_json: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl From<satchel_core::db::templates::TemplateRecord> for TemplateDto {
    fn from(record: satchel_core::db::templates::TemplateRecord) -> Self {
        Self {
            id: record.id,
            account_id: record.account_id,
            name: record.name,
            subject: record.subject,
            body_text: record.body_text,
            body_html: record.body_html,
            shortcut: record.shortcut,
            cc_json: encode_recipients(record.cc),
            bcc_json: encode_recipients(record.bcc),
            attachments_json: record.attachments_json,
            created_at: record.created_at,
            updated_at: record.updated_at,
        }
    }
}

impl From<TemplateInput> for TemplateUpsertRequest {
    fn from(input: TemplateInput) -> Self {
        Self {
            id: input.id,
            account_id: input.account_id,
            name: input.name,
            subject: input.subject,
            body_text: input.body_text,
            body_html: input.body_html,
            shortcut: input.shortcut,
            cc: input.cc,
            bcc: input.bcc,
            attachments_json: input.attachments_json,
        }
    }
}

impl From<TemplateError> for CommandError {
    fn from(error: TemplateError) -> Self {
        match error {
            TemplateError::NotFound => Self::new("backend.errors.templateNotFound"),
            TemplateError::Persistence(error) => {
                tracing::error!(error = ?error, "template persistence failed");
                Self::new("backend.errors.templatePersistenceFailed")
            }
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/templates/", optional_fields = nullable))]
pub struct RenderTemplateInput {
    pub template_id: String,
    #[serde(default)]
    pub variables: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/templates/")
)]
pub struct RenderedTemplate {
    pub subject: Option<String>,
    pub body_text: String,
    pub body_html: Option<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub attachments_json: Option<String>,
}

#[tauri::command]
pub async fn list_templates(
    pool: tauri::State<'_, crate::db::ReadPool>,
    account_id: Option<String>,
) -> CommandResult<Vec<TemplateDto>> {
    TemplateRepository::new(pool.inner().as_ref())
        .list(account_id.as_deref())
        .await
        .map(|rows| rows.into_iter().map(Into::into).collect())
        .map_err(Into::into)
}

#[tauri::command]
pub async fn upsert_template(
    pool: tauri::State<'_, SqlitePool>,
    input: TemplateInput,
) -> CommandResult<TemplateDto> {
    TemplateRepository::new(pool.inner())
        .upsert(input.into())
        .await
        .map(Into::into)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn delete_template(pool: tauri::State<'_, SqlitePool>, id: String) -> CommandResult<()> {
    TemplateRepository::new(pool.inner())
        .delete(&id)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn render_template(
    pool: tauri::State<'_, crate::db::ReadPool>,
    input: RenderTemplateInput,
) -> CommandResult<RenderedTemplate> {
    let row = TemplateRepository::new(pool.inner().as_ref())
        .get(&input.template_id)
        .await
        .map_err(CommandError::from)?;

    let mut vars = input.variables;
    let now = chrono::Local::now();
    vars.entry("date".into())
        .or_insert_with(|| now.format("%Y-%m-%d").to_string());
    vars.entry("time".into())
        .or_insert_with(|| now.format("%H:%M").to_string());

    Ok(RenderedTemplate {
        subject: row.subject.map(|s| substitute(&s, &vars)),
        body_text: substitute(&row.body_text, &vars),
        body_html: row.body_html.map(|s| substitute(&s, &vars)),
        cc: row.cc.unwrap_or_default(),
        bcc: row.bcc.unwrap_or_default(),
        attachments_json: row.attachments_json,
    })
}

fn substitute(template: &str, vars: &HashMap<String, String>) -> String {
    let mut out = template.to_string();
    for (key, value) in vars {
        let placeholder = format!("{{{{{key}}}}}");
        out = out.replace(&placeholder, value);
    }
    out
}

fn encode_recipients(recipients: Option<Vec<String>>) -> Option<String> {
    recipients.map(|values| serde_json::to_string(&values).unwrap_or_else(|_| "[]".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repository_errors_use_stable_keys_without_sql_details() {
        assert_eq!(
            CommandError::from(TemplateError::NotFound).key,
            "backend.errors.templateNotFound"
        );

        let detail = "near SELECT: secret template SQL";
        let error = CommandError::from(TemplateError::Persistence(sqlx::Error::Protocol(
            detail.into(),
        )));
        let serialized = serde_json::to_value(error).expect("command error JSON");
        assert_eq!(
            serialized,
            serde_json::json!({ "key": "backend.errors.templatePersistenceFailed" })
        );
        assert!(!serialized.to_string().contains(detail));
    }
}
