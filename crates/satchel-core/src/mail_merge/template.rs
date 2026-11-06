//! `{{FirstName}}` placeholders and their fallbacks (`{{FirstName|there}}`).
//!
//! A field is looked up without regard to case or surrounding spaces, so a
//! hand-typed `{{ firstname }}` still finds the `FirstName` column. A value
//! that is empty counts as missing: "Dear ," is the mistake mail merge exists
//! to prevent. Missing without a fallback is reported, not rendered.

use std::collections::BTreeMap;

/// Where the rendered text goes, which decides how a value is escaped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderContext {
    /// Plain-text body: values go in as they are.
    Text,
    /// HTML body: values are escaped. A fallback is part of the template and
    /// is already HTML.
    Html,
    /// Subject line: line breaks and control characters become spaces, so a
    /// value cannot start a new header.
    Header,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placeholder {
    pub name: String,
    pub fallback: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Piece<'a> {
    Text(&'a str),
    Token(Placeholder),
}

/// Splits a template into literal text and placeholders. Anything that does
/// not close, or has nothing between the braces, stays literal text.
fn pieces(template: &str) -> Vec<Piece<'_>> {
    let mut pieces = Vec::new();
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            break;
        };
        let inner = &after[..end];
        if inner.contains("{{") || inner.trim().is_empty() {
            // `{{ {{Name}}` – the inner pair is the token.
            let skip = start + 2;
            pieces.push(Piece::Text(&rest[..skip]));
            rest = &rest[skip..];
            continue;
        }
        let (name, fallback) = match inner.split_once('|') {
            Some((name, fallback)) => (name, Some(fallback.to_string())),
            None => (inner, None),
        };
        let name = name.trim();
        if name.is_empty() {
            let skip = start + 2;
            pieces.push(Piece::Text(&rest[..skip]));
            rest = &rest[skip..];
            continue;
        }
        if start > 0 {
            pieces.push(Piece::Text(&rest[..start]));
        }
        pieces.push(Piece::Token(Placeholder {
            name: name.to_string(),
            fallback,
        }));
        rest = &after[end + 2..];
    }
    if !rest.is_empty() {
        pieces.push(Piece::Text(rest));
    }
    pieces
}

/// Every placeholder in the template, in order of appearance.
pub fn placeholders(template: &str) -> Vec<Placeholder> {
    pieces(template)
        .into_iter()
        .filter_map(|piece| match piece {
            Piece::Token(token) => Some(token),
            Piece::Text(_) => None,
        })
        .collect()
}

/// The value a row holds for `name`, matched without case and outer spaces.
pub fn lookup<'a>(values: &'a BTreeMap<String, String>, name: &str) -> Option<&'a str> {
    let wanted = name.trim();
    values
        .iter()
        .find(|(field, _)| field.trim().eq_ignore_ascii_case(wanted))
        .map(|(_, value)| value.as_str())
        .filter(|value| !value.trim().is_empty())
}

/// Renders the template for one row. `Err` lists the fields that had neither
/// a value nor a fallback, each once, in order of appearance.
pub fn render(
    template: &str,
    values: &BTreeMap<String, String>,
    context: RenderContext,
) -> Result<String, Vec<String>> {
    let mut out = String::with_capacity(template.len());
    let mut missing: Vec<String> = Vec::new();
    for piece in pieces(template) {
        match piece {
            Piece::Text(text) => out.push_str(text),
            Piece::Token(token) => match lookup(values, &token.name) {
                Some(value) => out.push_str(&escape(value.trim(), context)),
                None => match token.fallback {
                    Some(fallback) => out.push_str(&clean_fallback(&fallback, context)),
                    None => {
                        if !missing
                            .iter()
                            .any(|name| name.eq_ignore_ascii_case(&token.name))
                        {
                            missing.push(token.name.clone());
                        }
                    }
                },
            },
        }
    }
    if missing.is_empty() {
        Ok(out)
    } else {
        Err(missing)
    }
}

fn escape(value: &str, context: RenderContext) -> String {
    match context {
        RenderContext::Text => value.to_string(),
        RenderContext::Header => single_line(value),
        RenderContext::Html => {
            let mut escaped = String::with_capacity(value.len());
            for character in value.chars() {
                match character {
                    '&' => escaped.push_str("&amp;"),
                    '<' => escaped.push_str("&lt;"),
                    '>' => escaped.push_str("&gt;"),
                    '"' => escaped.push_str("&quot;"),
                    '\'' => escaped.push_str("&#39;"),
                    '\n' => escaped.push_str("<br>"),
                    other => escaped.push(other),
                }
            }
            escaped
        }
    }
}

fn clean_fallback(fallback: &str, context: RenderContext) -> String {
    match context {
        RenderContext::Header => single_line(fallback),
        _ => fallback.to_string(),
    }
}

fn single_line(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    }

    #[test]
    fn fills_fields_regardless_of_case_and_spacing() {
        let values = row(&[("FirstName", "Anna"), ("Company", "ACME")]);
        assert_eq!(
            render(
                "Hi {{FirstName}} from {{ company }}!",
                &values,
                RenderContext::Text
            ),
            Ok("Hi Anna from ACME!".into())
        );
    }

    #[test]
    fn uses_the_fallback_for_a_missing_or_empty_value() {
        let values = row(&[("FirstName", "  ")]);
        assert_eq!(
            render("Hello {{FirstName|there}},", &values, RenderContext::Text),
            Ok("Hello there,".into())
        );
        // An empty fallback is a deliberate "leave it out".
        assert_eq!(
            render("Hello{{Title|}} you", &values, RenderContext::Text),
            Ok("Hello you".into())
        );
    }

    #[test]
    fn lists_each_missing_field_once() {
        let values = row(&[("Email", "a@x.de")]);
        assert_eq!(
            render(
                "{{FirstName}} {{LastName}} {{firstname}} {{Email}}",
                &values,
                RenderContext::Text
            ),
            Err(vec!["FirstName".into(), "LastName".into()])
        );
    }

    #[test]
    fn escapes_values_for_html_but_not_the_template() {
        let values = row(&[("Name", "<b>Tom & \"Jerry\"</b>")]);
        assert_eq!(
            render(
                "<p>{{Name}} {{Missing|&amp; co}}</p>",
                &values,
                RenderContext::Html
            ),
            Ok("<p>&lt;b&gt;Tom &amp; &quot;Jerry&quot;&lt;/b&gt; &amp; co</p>".into())
        );
    }

    #[test]
    fn keeps_a_value_from_breaking_the_subject_header() {
        let values = row(&[("Name", "Anna\r\nBcc: evil@example.com")]);
        assert_eq!(
            render("Offer for {{Name}}", &values, RenderContext::Header),
            Ok("Offer for Anna  Bcc: evil@example.com".into())
        );
    }

    #[test]
    fn leaves_broken_or_empty_braces_alone() {
        let values = row(&[("Name", "Anna")]);
        assert_eq!(
            render("{{}} {{ {{Name}} {{Name", &values, RenderContext::Text),
            Ok("{{}} {{ Anna {{Name".into())
        );
        assert_eq!(
            placeholders("{{A|x}} and {{ B }}"),
            vec![
                Placeholder {
                    name: "A".into(),
                    fallback: Some("x".into())
                },
                Placeholder {
                    name: "B".into(),
                    fallback: None
                },
            ]
        );
    }
}
