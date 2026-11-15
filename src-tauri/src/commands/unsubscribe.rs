//! One-click unsubscribe via the RFC 8058 `List-Unsubscribe` / `List-Unsubscribe-Post`
//! headers. Supports both the HTTP POST flow (preferred, no UI dance) and the
//! mailto: fallback (sends an email to the unsub address once the user has
//! confirmed the exact recipient, subject and body).
//!
//! The POST target comes straight out of an incoming mail, so it is attacker
//! chosen. Everything in `plan_unsubscribe` exists to keep that target from
//! becoming a request to an address only this machine can reach.

use std::time::Duration;

use reqwest::Url;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::commands::error::{CommandError, CommandResult};
use crate::sync::outbox::build_send_action;
use crate::AppState;

/// A one-click unsubscribe is a fire-and-forget POST; without a deadline a
/// silent target keeps the request (and the user's spinner) alive forever.
const UNSUBSCRIBE_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/unsubscribe/", optional_fields = nullable))]
pub struct UnsubscribeInput {
    pub message_id: String,
    /// The mail the user confirmed. It is sent only if it still equals what
    /// the message's header yields; nothing else in the plan runs.
    pub confirmed_mail: Option<UnsubscribeMail>,
}

/// The mail a `mailto:` unsubscribe sends, exactly as shown to the user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/unsubscribe/")
)]
pub struct UnsubscribeMail {
    pub to: String,
    pub subject: String,
    pub body: String,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/unsubscribe/")
)]
pub struct UnsubscribeResult {
    pub method: String,
    pub target: String,
    /// Set when the next step is a mail: nothing was sent, and the caller
    /// confirms this mail by passing it back as `confirmed_mail`.
    pub confirm_mail: Option<UnsubscribeMail>,
}

/// A step the header allows us to take, in the order we are willing to take it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum UnsubscribeAction {
    OneClickPost(Url),
    Mailto(UnsubscribeMail),
    OpenInBrowser(Url),
}

const MAILTO_SUBJECT_MAX_CHARS: usize = 200;
const MAILTO_BODY_MAX_CHARS: usize = 2000;
const MAILTO_DEFAULT_TEXT: &str = "unsubscribe";

fn target_blocked() -> CommandError {
    CommandError::new("backend.errors.unsubscribeTargetBlocked")
}

#[tauri::command]
pub async fn unsubscribe(
    state: tauri::State<'_, AppState>,
    pool: tauri::State<'_, SqlitePool>,
    input: UnsubscribeInput,
) -> CommandResult<UnsubscribeResult> {
    let row = satchel_core::db::messages::MessageReadRepository::new(pool.inner())
        .unsubscribe_headers(&input.message_id)
        .await
        .map_err(CommandError::unexpected)?;
    let Some(headers) = row else {
        return Err(CommandError::new(
            "backend.errors.unsubscribeMessageNotFound",
        ));
    };
    let Some(list_unsub) = headers.list_unsubscribe else {
        return Err(CommandError::new("backend.errors.unsubscribeHeaderMissing"));
    };

    let plan = plan_unsubscribe(&list_unsub, headers.list_unsubscribe_post.as_deref()).await?;
    if let Some(confirmed) = input.confirmed_mail {
        let mail = confirmed_mailto(&plan, &confirmed)?;
        let outgoing = crate::commands::messages::OutgoingMessageInput {
            to: vec![mail.to.clone()],
            subject: mail.subject.clone(),
            body: mail.body.clone(),
            ..Default::default()
        };
        let mut action = build_send_action(headers.account_id, outgoing, None)?;
        // build_send_action marks immediate sends as 'sending' because the
        // send_message command executes them in-line right after persisting.
        // Here nobody executes the action — it must be 'pending' so the outbox
        // worker claims it, otherwise the unsubscribe mail never leaves.
        action.status = "pending".into();
        state.queue.enqueue(pool.inner(), action).await?;
        return Ok(UnsubscribeResult {
            method: "mailto".into(),
            target: mail.to.clone(),
            confirm_mail: None,
        });
    }

    let mut last_error = None;
    for action in plan {
        match action {
            UnsubscribeAction::OneClickPost(url) => match post_one_click(&url).await {
                Ok(true) => {
                    return Ok(UnsubscribeResult {
                        method: "http-one-click".into(),
                        target: url.to_string(),
                        confirm_mail: None,
                    })
                }
                // A non-2xx answer — a redirect included, since the client no
                // longer follows them — is not an unsubscribe. Try the next route.
                Ok(false) => {}
                Err(err) => last_error = Some(err),
            },
            UnsubscribeAction::Mailto(mail) => {
                return Ok(UnsubscribeResult {
                    method: "mailto".into(),
                    target: mail.to.clone(),
                    confirm_mail: Some(mail),
                });
            }
            UnsubscribeAction::OpenInBrowser(url) => {
                webbrowser::open(url.as_str()).map_err(|err| {
                    CommandError::new("backend.errors.unsubscribeBrowserOpenFailed")
                        .with_param("message", err.to_string())
                })?;
                return Ok(UnsubscribeResult {
                    method: "browser".into(),
                    target: url.to_string(),
                    confirm_mail: None,
                });
            }
        }
    }

    Err(last_error.unwrap_or_else(|| CommandError::new("backend.errors.unsubscribeNoTarget")))
}

/// Decides what the header permits before anything leaves the process.
///
/// The one-click POST is the only step that acts without the user seeing where
/// it goes, so it is the only one held to the full target check. A plain-http
/// URL is deliberately demoted to the browser rather than POSTed.
async fn plan_unsubscribe(
    list_unsub: &str,
    post_header: Option<&str>,
) -> Result<Vec<UnsubscribeAction>, CommandError> {
    let url = first_url(list_unsub);
    let one_click = post_header.is_some_and(|value| value.contains("List-Unsubscribe=One-Click"));
    let mut plan = Vec::new();

    if one_click {
        if let Some(url) = url.as_ref() {
            if is_public_https_target(url).is_ok()
                && resolves_to_public_addresses_only(url).await.is_ok()
            {
                plan.push(UnsubscribeAction::OneClickPost(url.clone()));
            }
        }
    }

    let mut mailto_error = None;
    if let Some(mailto) = first_mailto(list_unsub) {
        match parse_mailto(&mailto) {
            Ok(mail) => plan.push(UnsubscribeAction::Mailto(mail)),
            Err(err) => mailto_error = Some(err),
        }
    }

    if let Some(url) = url {
        // Handing the URL to the browser is visible to the user, so the scheme
        // and port stay open here — but an address only this machine can reach
        // is still the attacker's target, not the user's.
        match is_public_host(&url) {
            Ok(()) => plan.push(UnsubscribeAction::OpenInBrowser(url)),
            Err(err) if plan.is_empty() => return Err(err),
            Err(_) => {}
        }
    }

    if plan.is_empty() {
        return Err(
            mailto_error.unwrap_or_else(|| CommandError::new("backend.errors.unsubscribeNoTarget"))
        );
    }

    Ok(plan)
}

/// The mail to send for a confirmed unsubscribe: the header must still yield
/// exactly the mail the user saw.
fn confirmed_mailto(
    plan: &[UnsubscribeAction],
    confirmed: &UnsubscribeMail,
) -> Result<UnsubscribeMail, CommandError> {
    plan.iter()
        .find_map(|action| match action {
            UnsubscribeAction::Mailto(mail) if mail == confirmed => Some(mail.clone()),
            _ => None,
        })
        .ok_or_else(|| CommandError::new("backend.errors.unsubscribeConfirmationStale"))
}

/// Sends the one-click POST. `true` means the list server accepted it.
async fn post_one_click(url: &Url) -> Result<bool, CommandError> {
    let client = satchel_core::utils::public_http::client(url, UNSUBSCRIBE_TIMEOUT)
        .await
        .map_err(|_| target_blocked())?;

    let response = client
        .post(url.clone())
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body("List-Unsubscribe=One-Click")
        .send()
        .await
        .map_err(|err| {
            CommandError::new("backend.errors.unsubscribePostFailed")
                .with_param("message", err.to_string())
        })?;

    Ok(response.status().is_success())
}

fn is_public_https_target(url: &Url) -> Result<(), CommandError> {
    satchel_core::utils::public_http::is_public_https_target(url).map_err(|_| target_blocked())
}

fn is_public_host(url: &Url) -> Result<(), CommandError> {
    satchel_core::utils::public_http::is_public_host(url).map_err(|_| target_blocked())
}

async fn resolves_to_public_addresses_only(url: &Url) -> Result<(), CommandError> {
    satchel_core::utils::public_http::resolves_to_public_addresses_only(url)
        .await
        .map_err(|_| target_blocked())
}

fn first_url(header: &str) -> Option<Url> {
    header
        .split(',')
        .map(str::trim)
        .filter_map(|part| Url::parse(part.trim_start_matches('<').trim_end_matches('>')).ok())
        .find(|url| matches!(url.scheme(), "http" | "https"))
}

/// The first `<mailto:…>` entry, without brackets and scheme. RFC 2369 puts
/// every entry in angle brackets, so commas inside one do not split it.
fn first_mailto(header: &str) -> Option<String> {
    let mut rest = header;
    while let Some(open) = rest.find('<') {
        let after = &rest[open + 1..];
        let close = after.find('>')?;
        let entry = after[..close].trim();
        if let Some(scheme) = entry.get(..7).filter(|s| s.eq_ignore_ascii_case("mailto:")) {
            return Some(entry[scheme.len()..].to_string());
        }
        rest = &after[close + 1..];
    }
    None
}

/// Parses the `mailto:` body into the one mail it may send. `to`, `cc` and
/// `bcc` fields are ignored; the address part must be exactly one address.
fn parse_mailto(value: &str) -> Result<UnsubscribeMail, CommandError> {
    let (address, query) = value.split_once('?').unwrap_or((value, ""));
    // `+` is literal in an address (`leave+123@list.example`).
    let to = percent_decode(address, false).trim().to_string();
    if !is_single_address(&to) {
        return Err(CommandError::new("backend.errors.unsubscribeMailtoInvalid"));
    }

    let mut subject = None;
    let mut body = None;
    for kv in query.split('&') {
        if let Some((key, val)) = kv.split_once('=') {
            match key.to_ascii_lowercase().as_str() {
                "subject" if subject.is_none() => subject = Some(percent_decode(val, true)),
                "body" if body.is_none() => body = Some(percent_decode(val, true)),
                _ => {}
            }
        }
    }

    let subject = subject
        .map(|value| single_line(&value))
        .map(|value| truncate_chars(&value, MAILTO_SUBJECT_MAX_CHARS))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| MAILTO_DEFAULT_TEXT.into());
    let body = body
        .map(|value| clean_body(&value))
        .map(|value| truncate_chars(&value, MAILTO_BODY_MAX_CHARS))
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| MAILTO_DEFAULT_TEXT.into());

    Ok(UnsubscribeMail { to, subject, body })
}

/// One plain `local@domain`: dot-atom local part, dotted domain, nothing that
/// a recipient parser could read as a second address.
fn is_single_address(value: &str) -> bool {
    let Some((local, domain)) = value.split_once('@') else {
        return false;
    };
    if local.is_empty() || local.len() > 64 || domain.is_empty() || value.len() > 254 {
        return false;
    }
    let atext = |ch: char| ch.is_ascii_alphanumeric() || "!#$%&'*+-/=?^_`{|}~".contains(ch);
    let dot_atom = |part: &str, allowed: &dyn Fn(char) -> bool| {
        part.split('.')
            .all(|atom| !atom.is_empty() && atom.chars().all(allowed))
    };
    let label = |ch: char| ch.is_alphanumeric() || ch == '-';
    dot_atom(local, &atext)
        && domain.contains('.')
        && dot_atom(domain, &label)
        && domain
            .split('.')
            .all(|part| !part.starts_with('-') && !part.ends_with('-'))
}

fn single_line(value: &str) -> String {
    value
        .split(|ch: char| ch.is_control())
        .filter(|part| !part.trim().is_empty())
        .map(str::trim)
        .collect::<Vec<_>>()
        .join(" ")
}

fn clean_body(value: &str) -> String {
    value
        .replace("\r\n", "\n")
        .chars()
        .map(|ch| if ch == '\r' { '\n' } else { ch })
        .filter(|ch| !ch.is_control() || matches!(ch, '\n' | '\t'))
        .collect()
}

fn truncate_chars(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

fn percent_decode(input: &str, plus_is_space: bool) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'+' && plus_is_space {
            out.push(b' ');
        } else if byte == b'%' {
            let hex = input.get(index + 1..index + 3);
            if let Some(value) = hex.and_then(|hex| u8::from_str_radix(hex, 16).ok()) {
                out.push(value);
                index += 3;
                continue;
            }
            out.push(byte);
        } else {
            out.push(byte);
        }
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(value: &str) -> Url {
        Url::parse(value).expect("test URL should parse")
    }

    #[test]
    fn one_click_targets_are_restricted_to_public_https() {
        assert!(is_public_https_target(&url("https://example.com/u")).is_ok());
        assert!(is_public_https_target(&url("https://example.com:443/u")).is_ok());

        for rejected in [
            // Plain http keeps the browser route instead of a silent POST.
            "http://example.com/u",
            "https://example.com:8080/u",
            "https://127.0.0.1/u",
            "https://[::1]/u",
            "https://[::ffff:127.0.0.1]/u",
            "https://10.1.2.3/u",
            "https://172.16.4.5/u",
            "https://192.168.1.1/u",
            "https://100.64.0.1/u",
            "https://169.254.169.254/latest/meta-data",
            "https://[fe80::1]/u",
            "https://[fd00::1]/u",
            "https://localhost/u",
            "https://api.localhost/u",
        ] {
            let Err(error) = is_public_https_target(&url(rejected)) else {
                panic!("{rejected} must not be a one-click POST target");
            };
            assert_eq!(error.key, "backend.errors.unsubscribeTargetBlocked");
        }
    }

    #[tokio::test]
    async fn resolution_rejects_every_non_routable_answer() {
        // Literal hosts resolve without a nameserver, so this stays offline.
        for rejected in [
            "https://127.0.0.1/u",
            "https://[::1]/u",
            "https://10.1.2.3/u",
        ] {
            let Err(error) = resolves_to_public_addresses_only(&url(rejected)).await else {
                panic!("{rejected} must not pass the resolution check");
            };
            assert_eq!(error.key, "backend.errors.unsubscribeTargetBlocked");
        }

        assert!(
            resolves_to_public_addresses_only(&url("https://93.184.215.14/u"))
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn one_click_unsubscribe_refuses_a_loopback_target() {
        // No mailto to fall back on, so the plan has to come back empty rather
        // than hold any step that touches the network.
        let error = plan_unsubscribe("<http://127.0.0.1:1/x>", Some("List-Unsubscribe=One-Click"))
            .await
            .expect_err("a loopback target must be refused outright");

        assert_eq!(error.key, "backend.errors.unsubscribeTargetBlocked");
    }

    #[tokio::test]
    async fn a_blocked_one_click_target_still_uses_the_mailto_route() {
        let plan = plan_unsubscribe(
            "<https://169.254.169.254/u>, <mailto:leave@list.example>",
            Some("List-Unsubscribe=One-Click"),
        )
        .await
        .expect("the mailto route stays available");

        assert_eq!(
            plan,
            vec![UnsubscribeAction::Mailto(UnsubscribeMail {
                to: "leave@list.example".into(),
                subject: "unsubscribe".into(),
                body: "unsubscribe".into(),
            })]
        );
    }

    #[tokio::test]
    async fn plain_http_falls_back_to_the_browser_instead_of_posting() {
        let plan = plan_unsubscribe(
            "<http://lists.example/u>",
            Some("List-Unsubscribe=One-Click"),
        )
        .await
        .expect("a public http target is browsable");

        assert_eq!(
            plan,
            vec![UnsubscribeAction::OpenInBrowser(url(
                "http://lists.example/u"
            ))]
        );
    }

    #[tokio::test]
    async fn a_public_one_click_target_is_planned_before_the_fallbacks() {
        let plan = plan_unsubscribe(
            "<https://93.184.215.14/u>, <mailto:leave@list.example>",
            Some("List-Unsubscribe=One-Click"),
        )
        .await
        .expect("a public https target is allowed");

        assert_eq!(
            plan.first(),
            Some(&UnsubscribeAction::OneClickPost(url(
                "https://93.184.215.14/u"
            )))
        );
    }

    #[tokio::test]
    async fn without_the_post_header_nothing_is_posted() {
        let plan = plan_unsubscribe("<https://93.184.215.14/u>", None)
            .await
            .expect("the browser route remains");

        assert!(plan
            .iter()
            .all(|action| !matches!(action, UnsubscribeAction::OneClickPost(_))));
    }

    #[test]
    fn mailto_parsing_survives_the_header_forms() {
        let mail = parse_mailto("leave@list.example?subject=Un%20sub&body=stop+it").unwrap();
        assert_eq!(mail.to, "leave@list.example");
        assert_eq!(mail.subject, "Un sub");
        assert_eq!(mail.body, "stop it");

        let mail = parse_mailto("leave+t0k3n@list.example").unwrap();
        assert_eq!(mail.to, "leave+t0k3n@list.example");
        assert_eq!(mail.subject, "unsubscribe");
        assert_eq!(mail.body, "unsubscribe");

        let mail = parse_mailto("leave%40list.example?subject=%C3%84nderung").unwrap();
        assert_eq!(mail.to, "leave@list.example");
        assert_eq!(mail.subject, "Änderung");
    }

    #[test]
    fn mailto_must_name_exactly_one_address() {
        for rejected in [
            "a@list.example;b@other.example",
            "a@list.example%3Bb@other.example",
            "a@list.example,b@other.example",
            "a@list.example%2Cb@other.example",
            "a@list.example%20b@other.example",
            "a@list.example%0Ab@other.example",
            "Name%20%3Ca@list.example%3E",
            "",
            "list.example",
            "a@localhost",
            "a@@list.example",
            "a@list..example",
        ] {
            let Err(error) = parse_mailto(rejected) else {
                panic!("{rejected} must not be accepted as one address");
            };
            assert_eq!(error.key, "backend.errors.unsubscribeMailtoInvalid");
        }
    }

    #[test]
    fn mailto_recipient_fields_in_the_query_are_ignored() {
        let mail = parse_mailto(
            "leave@list.example?to=x@other.example&cc=y@other.example&bcc=z@other.example&subject=go",
        )
        .unwrap();
        assert_eq!(mail.to, "leave@list.example");
        assert_eq!(mail.subject, "go");
    }

    #[test]
    fn mailto_subject_and_body_are_bounded() {
        let long = "x".repeat(5000);
        let mail = parse_mailto(&format!(
            "leave@list.example?subject=a%0D%0ABcc:%20y@other.example%0A{long}&body={long}"
        ))
        .unwrap();
        assert!(!mail.subject.contains(['\r', '\n']));
        assert!(mail.subject.starts_with("a Bcc: y@other.example x"));
        assert_eq!(mail.subject.chars().count(), MAILTO_SUBJECT_MAX_CHARS);
        assert_eq!(mail.body.chars().count(), MAILTO_BODY_MAX_CHARS);

        let mail = parse_mailto("leave@list.example?subject=%0A&body=%20").unwrap();
        assert_eq!(mail.subject, "unsubscribe");
        assert_eq!(mail.body, "unsubscribe");
    }

    #[test]
    fn the_mailto_entry_is_read_from_angle_brackets() {
        assert_eq!(
            first_mailto("<https://x.example/u>, <MAILTO:leave@list.example?subject=a,b>")
                .as_deref(),
            Some("leave@list.example?subject=a,b")
        );
        assert_eq!(first_mailto("<https://x.example/u>"), None);
    }

    #[tokio::test]
    async fn an_invalid_mailto_alone_is_refused_with_its_own_key() {
        let error = plan_unsubscribe("<mailto:a@list.example;b@other.example>", None)
            .await
            .expect_err("two recipients must not be planned");
        assert_eq!(error.key, "backend.errors.unsubscribeMailtoInvalid");
    }

    #[tokio::test]
    async fn only_the_mail_the_header_yields_can_be_confirmed() {
        let plan = plan_unsubscribe("<mailto:leave@list.example?subject=tok>", None)
            .await
            .unwrap();
        let shown = UnsubscribeMail {
            to: "leave@list.example".into(),
            subject: "tok".into(),
            body: "unsubscribe".into(),
        };
        assert_eq!(confirmed_mailto(&plan, &shown).unwrap(), shown);

        for altered in [
            UnsubscribeMail {
                to: "x@other.example".into(),
                ..shown.clone()
            },
            UnsubscribeMail {
                subject: "other".into(),
                ..shown.clone()
            },
            UnsubscribeMail {
                body: "other".into(),
                ..shown.clone()
            },
        ] {
            let error =
                confirmed_mailto(&plan, &altered).expect_err("altered mail must be refused");
            assert_eq!(error.key, "backend.errors.unsubscribeConfirmationStale");
        }
    }
}
