use exports::satchel::plugin::plugin::Guest;
use serde::{Deserialize, Serialize};

wit_bindgen::generate!({
    path: "../../plugin-sdk/wit",
    world: "satchel-plugin",
});

struct SatchelPgpPlugin;

impl Guest for SatchelPgpPlugin {
    fn run_command(command_id: String, payload: String) -> Result<String, String> {
        if command_id == "transform-message" {
            let input: MessageTransformInput =
                serde_json::from_str(&payload).map_err(|err| format!("invalid payload: {err}"))?;
            let output = transform_pgp_message(&input);
            return serde_json::to_string(&output)
                .map_err(|err| format!("failed to encode output: {err}"));
        }

        Err(format!("command not implemented: {command_id}"))
    }
}

export!(SatchelPgpPlugin);

#[derive(Debug, Deserialize)]
struct MessageTransformInput {
    subject: String,
    snippet: String,
    body_text: String,
    body_html: Option<String>,
    has_pgp_private_key: bool,
}

#[derive(Debug, Serialize, Default)]
struct MessageTransformOutput {
    subject: Option<String>,
    snippet: Option<String>,
    body_text: Option<String>,
    body_html: Option<String>,
    labels: Vec<String>,
    pgp_state: Option<String>,
    pgp_error_detail: Option<String>,
    pgp_signer_name: Option<String>,
    pgp_signer_fingerprint: Option<String>,
}

fn transform_pgp_message(input: &MessageTransformInput) -> MessageTransformOutput {
    let text = input.body_text.trim();
    if text.is_empty() {
        return MessageTransformOutput::default();
    }

    if text.contains("-----BEGIN PGP SIGNED MESSAGE-----") {
        if let Some(plain) = extract_clearsigned_content(text) {
            let snippet = first_non_empty_line(&plain).unwrap_or_else(|| input.snippet.clone());
            return MessageTransformOutput {
                snippet: Some(snippet),
                body_text: Some(plain),
                labels: vec!["pgp-signed".to_string()],
                ..MessageTransformOutput::default()
            };
        }
    }

    if text.contains("-----BEGIN PGP MESSAGE-----") {
        let state = if input.has_pgp_private_key {
            "key_available_locked"
        } else {
            "encrypted_no_key"
        };

        return MessageTransformOutput {
            labels: vec!["pgp-encrypted".to_string()],
            pgp_state: Some(state.to_string()),
            ..MessageTransformOutput::default()
        };
    }

    MessageTransformOutput {
        subject: Some(input.subject.clone()),
        body_html: input.body_html.clone(),
        ..MessageTransformOutput::default()
    }
}

fn extract_clearsigned_content(text: &str) -> Option<String> {
    let start_marker = "-----BEGIN PGP SIGNED MESSAGE-----";
    let sig_marker = "-----BEGIN PGP SIGNATURE-----";
    let start = text.find(start_marker)?;
    let after_start = &text[start + start_marker.len()..];

    let body_start = if let Some(pos) = after_start.find("\n\n") {
        pos + 2
    } else if let Some(pos) = after_start.find("\r\n\r\n") {
        pos + 4
    } else {
        return None;
    };

    let content_and_sig = &after_start[body_start..];
    let sig_pos = content_and_sig.find(sig_marker)?;
    let signed_text = &content_and_sig[..sig_pos];

    let normalized = signed_text
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .lines()
        .map(|line| line.strip_prefix("- ").unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string();

    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

fn first_non_empty_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.to_string())
}
