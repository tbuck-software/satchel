//! Which rows of a recipient list become messages, and what each one says.
//!
//! The preview and the job both come from here, so what the user steps
//! through is exactly what the job will send.

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};

use super::template::{placeholders, render, RenderContext};

/// Most messages one job may send. Consumer Gmail allows about 500 a day;
/// a bigger list is a newsletter and belongs in a newsletter tool.
pub const MAX_JOB_RECIPIENTS: usize = 500;

/// Subject and body of the message every recipient gets a copy of.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "core/mail_merge/", optional_fields = nullable))]
pub struct MergeTemplate {
    pub subject: String,
    pub body: String,
    #[serde(default)]
    pub body_html: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "core/mail_merge/", optional_fields = nullable))]
pub struct MergeRecipientInput {
    /// Where the row came from: a CSV line, or none for a contact.
    #[serde(default)]
    pub line: Option<u32>,
    pub values: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "core/mail_merge/"))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RowIssue {
    /// The address cell is empty or not one address.
    InvalidEmail,
    /// The address already appeared in an earlier row.
    Duplicate,
    /// Placeholders with neither a value nor a fallback.
    MissingValues { fields: Vec<String> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "core/mail_merge/", optional_fields = nullable))]
pub struct PlannedRow {
    /// Index in the input list.
    pub index: u32,
    pub line: Option<u32>,
    /// The normalized address, or the raw cell when it is not one.
    pub email: String,
    pub issue: Option<RowIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "core/mail_merge/"))]
pub struct RenderedMessage {
    pub subject: String,
    pub body: String,
    pub body_html: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "core/mail_merge/"))]
pub struct MergePlan {
    pub rows: Vec<PlannedRow>,
    /// Rows that will be sent.
    pub ready: u32,
    /// Placeholders naming no field of the list. They render as their
    /// fallback or block the row, so a typo shows up before sending.
    pub unknown_fields: Vec<String>,
}

/// One address, lower-cased in the domain, or `None`. A cell like
/// `Anna <anna@x.de>` counts; two addresses in one cell do not.
pub fn normalize_address(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let candidate = match (trimmed.rfind('<'), trimmed.ends_with('>')) {
        (Some(open), true) => &trimmed[open + 1..trimmed.len() - 1],
        _ => trimmed,
    }
    .trim();
    if candidate.is_empty()
        || candidate.len() > 254
        || candidate.chars().any(|c| {
            c.is_whitespace()
                || c.is_control()
                || matches!(
                    c,
                    '<' | '>' | ',' | ';' | '"' | '(' | ')' | '[' | ']' | '\\'
                )
        })
    {
        return None;
    }
    let (local, domain) = candidate.rsplit_once('@')?;
    if local.is_empty() || local.contains('@') || local.starts_with('.') || local.ends_with('.') {
        return None;
    }
    let labels: Vec<&str> = domain.split('.').collect();
    if labels.len() < 2
        || labels
            .iter()
            .any(|label| label.is_empty() || label.starts_with('-') || label.ends_with('-'))
    {
        return None;
    }
    Some(format!("{local}@{}", domain.to_ascii_lowercase()))
}

/// Checks every row against the template. Rows keep their order; issues are
/// per row, so a bad row is skipped and listed instead of stopping the job.
pub fn plan(
    template: &MergeTemplate,
    fields: &[String],
    email_field: &str,
    rows: &[MergeRecipientInput],
) -> MergePlan {
    let mut seen = HashSet::new();
    let mut planned = Vec::with_capacity(rows.len());
    let mut ready = 0;
    for (index, row) in rows.iter().enumerate() {
        let raw = row
            .values
            .get(email_field)
            .map(String::as_str)
            .unwrap_or("");
        let (email, issue) = match normalize_address(raw) {
            None => (raw.trim().to_string(), Some(RowIssue::InvalidEmail)),
            Some(email) => {
                if !seen.insert(email.to_ascii_lowercase()) {
                    (email, Some(RowIssue::Duplicate))
                } else {
                    let missing = missing_fields(template, &row.values);
                    let issue = (!missing.is_empty())
                        .then_some(RowIssue::MissingValues { fields: missing });
                    (email, issue)
                }
            }
        };
        if issue.is_none() {
            ready += 1;
        }
        planned.push(PlannedRow {
            index: index as u32,
            line: row.line,
            email,
            issue,
        });
    }
    MergePlan {
        rows: planned,
        ready,
        unknown_fields: unknown_fields(template, fields),
    }
}

fn missing_fields(template: &MergeTemplate, values: &BTreeMap<String, String>) -> Vec<String> {
    let mut missing: Vec<String> = Vec::new();
    let parts = [
        (template.subject.as_str(), RenderContext::Header),
        (template.body.as_str(), RenderContext::Text),
        (
            template.body_html.as_deref().unwrap_or(""),
            RenderContext::Html,
        ),
    ];
    for (text, context) in parts {
        if let Err(fields) = render(text, values, context) {
            for field in fields {
                if !missing.iter().any(|name| name.eq_ignore_ascii_case(&field)) {
                    missing.push(field);
                }
            }
        }
    }
    missing
}

fn unknown_fields(template: &MergeTemplate, fields: &[String]) -> Vec<String> {
    let mut unknown: Vec<String> = Vec::new();
    let all = [
        template.subject.as_str(),
        template.body.as_str(),
        template.body_html.as_deref().unwrap_or(""),
    ];
    for text in all {
        for placeholder in placeholders(text) {
            let known = fields
                .iter()
                .any(|field| field.trim().eq_ignore_ascii_case(&placeholder.name));
            if !known
                && !unknown
                    .iter()
                    .any(|name| name.eq_ignore_ascii_case(&placeholder.name))
            {
                unknown.push(placeholder.name);
            }
        }
    }
    unknown
}

/// The message one row receives. `Err` lists the fields that block it.
pub fn render_message(
    template: &MergeTemplate,
    values: &BTreeMap<String, String>,
) -> Result<RenderedMessage, Vec<String>> {
    let missing = missing_fields(template, values);
    if !missing.is_empty() {
        return Err(missing);
    }
    // `missing_fields` rendered each part already; none of these can fail.
    let subject = render(&template.subject, values, RenderContext::Header)?;
    let body = render(&template.body, values, RenderContext::Text)?;
    let body_html = template
        .body_html
        .as_deref()
        .map(|html| render(html, values, RenderContext::Html))
        .transpose()?;
    Ok(RenderedMessage {
        subject,
        body,
        body_html,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(pairs: &[(&str, &str)]) -> MergeRecipientInput {
        MergeRecipientInput {
            line: None,
            values: pairs
                .iter()
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect(),
        }
    }

    fn template(subject: &str, body: &str) -> MergeTemplate {
        MergeTemplate {
            subject: subject.into(),
            body: body.into(),
            body_html: Some(format!("<p>{body}</p>")),
        }
    }

    #[test]
    fn accepts_one_address_per_cell() {
        assert_eq!(
            normalize_address(" a.b@Example.COM "),
            Some("a.b@example.com".into())
        );
        assert_eq!(
            normalize_address("Anna <anna@x.de>"),
            Some("anna@x.de".into())
        );
        for bad in [
            "",
            "anna",
            "a@b",
            "a@b.",
            "a @b.de",
            "a@b.de, c@d.de",
            "@x.de",
            "a@-x.de",
        ] {
            assert_eq!(normalize_address(bad), None, "{bad}");
        }
    }

    #[test]
    fn skips_invalid_duplicate_and_incomplete_rows_and_keeps_the_rest() {
        let fields = vec!["Email".to_string(), "FirstName".to_string()];
        let rows = vec![
            input(&[("Email", "anna@x.de"), ("FirstName", "Anna")]),
            input(&[("Email", "not-an-address"), ("FirstName", "Ben")]),
            input(&[("Email", "ANNA@X.DE"), ("FirstName", "Anna again")]),
            input(&[("Email", "cem@x.de"), ("FirstName", "")]),
        ];
        let plan = plan(
            &template("Hi {{FirstName}}", "Dear {{FirstName}}"),
            &fields,
            "Email",
            &rows,
        );
        assert_eq!(plan.ready, 1);
        assert_eq!(plan.rows[0].issue, None);
        assert_eq!(plan.rows[1].issue, Some(RowIssue::InvalidEmail));
        assert_eq!(plan.rows[2].issue, Some(RowIssue::Duplicate));
        assert_eq!(
            plan.rows[3].issue,
            Some(RowIssue::MissingValues {
                fields: vec!["FirstName".into()]
            })
        );
        assert!(plan.unknown_fields.is_empty());
    }

    #[test]
    fn a_fallback_lets_an_incomplete_row_through() {
        let fields = vec!["Email".to_string(), "FirstName".to_string()];
        let rows = vec![input(&[("Email", "cem@x.de")])];
        let plan = plan(
            &template("Hi", "Dear {{FirstName|friend}}"),
            &fields,
            "Email",
            &rows,
        );
        assert_eq!(plan.ready, 1);
        let rendered = render_message(
            &template("Hi", "Dear {{FirstName|friend}}"),
            &rows[0].values,
        )
        .expect("render");
        assert_eq!(rendered.body, "Dear friend");
        assert_eq!(rendered.body_html.as_deref(), Some("<p>Dear friend</p>"));
    }

    #[test]
    fn names_placeholders_that_match_no_field() {
        let fields = vec!["Email".to_string(), "FirstName".to_string()];
        let plan = plan(
            &template("{{Frist|x}}", "{{firstname}} {{Company}}"),
            &fields,
            "Email",
            &[],
        );
        assert_eq!(
            plan.unknown_fields,
            vec!["Frist".to_string(), "Company".to_string()]
        );
    }
}
