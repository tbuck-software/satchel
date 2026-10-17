//! One-click unsubscribe via the RFC 8058 `List-Unsubscribe` / `List-Unsubscribe-Post`
//! headers. Supports both the HTTP POST flow (preferred, no UI dance) and the
//! mailto: fallback (sends an email to the unsub address).
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
}

/// A step the header allows us to take, in the order we are willing to take it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum UnsubscribeAction {
    OneClickPost(Url),
    Mailto(String),
    OpenInBrowser(Url),
}

fn target_blocked() -> CommandError {
    CommandError::new("backend.errors.unsubscribeTargetBlocked")
}

#[tauri::command]
pub async fn unsubscribe(
    state: tauri::State<'_, AppState>,
    pool: tauri::State<'_, SqlitePool>,
    input: UnsubscribeInput,
) -> CommandResult<UnsubscribeResult> {
    let row: Option<(String, Option<String>, Option<String>, String)> = sqlx::query_as(
        "SELECT account_id, list_unsubscribe, list_unsubscribe_post, from_address
         FROM messages WHERE id = ?",
    )
    .bind(&input.message_id)
    .fetch_optional(pool.inner())
    .await
    .map_err(CommandError::unexpected)?;
    let Some((account_id, list_unsub, list_unsub_post, _from)) = row else {
        return Err(CommandError::new(
            "backend.errors.unsubscribeMessageNotFound",
        ));
    };
    let Some(list_unsub) = list_unsub else {
        return Err(CommandError::new("backend.errors.unsubscribeHeaderMissing"));
    };

    let plan = plan_unsubscribe(&list_unsub, list_unsub_post.as_deref()).await?;
    let mut last_error = None;
    for action in plan {
        match action {
            UnsubscribeAction::OneClickPost(url) => match post_one_click(&url).await {
                Ok(true) => {
                    return Ok(UnsubscribeResult {
                        method: "http-one-click".into(),
                        target: url.to_string(),
                    })
                }
                // A non-2xx answer — a redirect included, since the client no
                // longer follows them — is not an unsubscribe. Try the next route.
                Ok(false) => {}
                Err(err) => last_error = Some(err),
            },
            UnsubscribeAction::Mailto(mailto) => {
                let parsed = parse_mailto(&mailto);
                let outgoing = crate::commands::messages::OutgoingMessageInput {
                    to: vec![parsed.to.clone()],
                    subject: parsed.subject.unwrap_or_else(|| "unsubscribe".into()),
                    body: parsed.body.unwrap_or_else(|| "unsubscribe".into()),
                    ..Default::default()
                };
                let mut action = build_send_action(account_id, outgoing, None)?;
                // build_send_action marks immediate sends as 'sending' because the
                // send_message command executes them in-line right after persisting.
                // Here nobody executes the action — it must be 'pending' so the outbox
                // worker claims it, otherwise the unsubscribe mail never leaves.
                action.status = "pending".into();
                state.queue.enqueue(pool.inner(), action).await?;
                return Ok(UnsubscribeResult {
                    method: "mailto".into(),
                    target: parsed.to,
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

    if let Some(mailto) = first_mailto(list_unsub) {
        plan.push(UnsubscribeAction::Mailto(mailto));
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
        return Err(CommandError::new("backend.errors.unsubscribeNoTarget"));
    }

    Ok(plan)
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

fn first_mailto(header: &str) -> Option<String> {
    header
        .split(',')
        .map(str::trim)
        .find(|part| part.to_ascii_lowercase().starts_with("<mailto:"))
        .map(|p| {
            p.trim_start_matches('<')
                .trim_end_matches('>')
                .trim_start_matches("mailto:")
                .to_string()
        })
}

#[derive(Debug, Default)]
struct MailtoParts {
    to: String,
    subject: Option<String>,
    body: Option<String>,
}

fn parse_mailto(value: &str) -> MailtoParts {
    let (address, query) = value.split_once('?').unwrap_or((value, ""));
    let mut parts = MailtoParts {
        to: address.to_string(),
        ..Default::default()
    };
    for kv in query.split('&') {
        if let Some((key, val)) = kv.split_once('=') {
            let decoded = urlencoding_decode(val);
            match key.to_ascii_lowercase().as_str() {
                "subject" => parts.subject = Some(decoded),
                "body" => parts.body = Some(decoded),
                _ => {}
            }
        }
    }
    parts
}

fn urlencoding_decode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '+' {
            out.push(' ');
        } else if ch == '%' {
            let h1 = chars.next();
            let h2 = chars.next();
            if let (Some(h1), Some(h2)) = (h1, h2) {
                if let Ok(byte) = u8::from_str_radix(&format!("{h1}{h2}"), 16) {
                    out.push(byte as char);
                    continue;
                }
            }
            out.push('%');
            if let Some(h1) = h1 {
                out.push(h1);
            }
            if let Some(h2) = h2 {
                out.push(h2);
            }
        } else {
            out.push(ch);
        }
    }
    out
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
            vec![UnsubscribeAction::Mailto("leave@list.example".into())]
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
        let parts = parse_mailto("leave@list.example?subject=Un%20sub&body=stop+it");
        assert_eq!(parts.to, "leave@list.example");
        assert_eq!(parts.subject.as_deref(), Some("Un sub"));
        assert_eq!(parts.body.as_deref(), Some("stop it"));
    }
}
