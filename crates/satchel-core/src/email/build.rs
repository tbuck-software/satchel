//! RFC-5322/2045 message builder for outgoing mail.
//!
//! Produces a complete RFC822 string suitable for Gmail-API `messages.send`,
//! IMAP `APPEND` (drafts), and similar callers. SMTP transport via lettre is handled
//! separately in `providers/imap.rs::send` for back-pressure and TLS reasons.

use crate::providers::traits::{OutgoingAttachment, OutgoingMessage};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BuildError {
    /// The attachment data does not decode; sending it would deliver garbage.
    #[error("attachment `{filename}` is not valid base64")]
    InvalidAttachment { filename: String },
}

impl BuildError {
    /// The `{key, params}` form stored in queue rows and shown by the frontend.
    pub fn encoded(&self) -> String {
        match self {
            Self::InvalidAttachment { filename } => serde_json::json!({
                "key": "backend.errors.attachmentNotBase64",
                "params": { "filename": filename },
            })
            .to_string(),
        }
    }
}

/// Build a complete RFC822 message string. `from_address` and `from_name` arguments
/// override the values on `OutgoingMessage` when provided (so callers without an
/// alias can pass the account email directly).
pub fn build_rfc822(
    message: &OutgoingMessage,
    fallback_from_address: Option<&str>,
    fallback_from_name: Option<&str>,
) -> Result<String, BuildError> {
    build_rfc822_inner(message, fallback_from_address, fallback_from_name, false)
}

/// Like `build_rfc822` but includes a `Bcc:` header. Use ONLY for submission
/// endpoints that derive recipients from headers and strip Bcc on delivery
/// (Gmail `messages.send`). Never feed this to raw SMTP DATA — most MTAs pass
/// the header through verbatim and every recipient would see the Bcc list.
pub fn build_rfc822_with_bcc(
    message: &OutgoingMessage,
    fallback_from_address: Option<&str>,
    fallback_from_name: Option<&str>,
) -> Result<String, BuildError> {
    build_rfc822_inner(message, fallback_from_address, fallback_from_name, true)
}

/// Add a `Bcc:` header to the outer header block of an already finished
/// message. Same rule as `build_rfc822_with_bcc`: ONLY for submission endpoints
/// that derive recipients from the headers and strip Bcc on delivery (Gmail
/// `messages.send`, Graph `/me/sendMail`). Never on the raw SMTP path.
///
/// Exists for the crypto transforms: their base message must stay Bcc-free, so
/// the header goes on afterwards, outside the signed or encrypted entity.
pub fn with_bcc_delivery_header(rfc822: &str, bcc: &[String]) -> String {
    if bcc.is_empty() {
        return rfc822.to_string();
    }
    let header = fold_header("Bcc", &sanitize_header_text(&bcc.join(", ")));
    if let Some((headers, body)) = rfc822.split_once("\r\n\r\n") {
        return format!("{headers}\r\n{header}\r\n\r\n{body}");
    }
    if let Some((headers, body)) = rfc822.split_once("\n\n") {
        return format!("{headers}\n{header}\n\n{body}");
    }
    // No separator at all: the whole string is a header block.
    format!("{}\r\n{header}\r\n", rfc822.trim_end_matches(['\r', '\n']))
}

fn build_rfc822_inner(
    message: &OutgoingMessage,
    fallback_from_address: Option<&str>,
    fallback_from_name: Option<&str>,
    include_bcc_header: bool,
) -> Result<String, BuildError> {
    let from_address = message
        .from_address
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .or(fallback_from_address)
        .unwrap_or("");
    let from_name = message
        .from_name
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .or(fallback_from_name);

    let mut headers: Vec<String> = Vec::new();
    headers.push(fold_header(
        "From",
        &format_mailbox(from_name, from_address),
    ));
    if !message.to.is_empty() {
        headers.push(fold_header("To", &mailbox_list(message, &message.to)));
    }
    if !message.cc.is_empty() {
        headers.push(fold_header("Cc", &mailbox_list(message, &message.cc)));
    }
    if include_bcc_header && !message.bcc.is_empty() {
        headers.push(fold_header("Bcc", &mailbox_list(message, &message.bcc)));
    }
    if let Some(reply_to) = message.reply_to.as_deref().filter(|s| !s.trim().is_empty()) {
        headers.push(fold_header("Reply-To", &sanitize_header_text(reply_to)));
    }
    headers.push(fold_header(
        "Subject",
        &encode_header_value(&message.subject),
    ));
    headers.push(format!("Date: {}", chrono::Utc::now().to_rfc2822()));
    let message_id = message
        .message_id_header
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| format!("<{}@satchel.local>", uuid::Uuid::new_v4()));
    headers.push(format!("Message-ID: {message_id}"));
    if let Some(rr) = message
        .request_read_receipt_to
        .as_deref()
        .filter(|s| !s.trim().is_empty())
    {
        headers.push(format!("Disposition-Notification-To: <{rr}>"));
        headers.push(format!("Return-Receipt-To: <{rr}>"));
    }

    // Importance markers. Three headers because no single one is universally
    // understood: X-Priority is the classic (Outlook/Thunderbird), Importance
    // is RFC 4021, X-MSMail-Priority is what Outlook itself writes.
    if let Some(priority) = message
        .priority
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("normal"))
    {
        let (x_priority, importance) = if priority.eq_ignore_ascii_case("high") {
            ("1 (Highest)", "high")
        } else {
            ("5 (Lowest)", "low")
        };
        headers.push(format!("X-Priority: {x_priority}"));
        headers.push(format!("Importance: {importance}"));
        headers.push(format!(
            "X-MSMail-Priority: {}",
            if importance == "high" { "High" } else { "Low" }
        ));
    }
    if let Some(in_reply_to) = &message.in_reply_to {
        headers.push(fold_header(
            "In-Reply-To",
            &sanitize_header_text(in_reply_to),
        ));
    }
    if !message.references.is_empty() {
        headers.push(fold_header(
            "References",
            &sanitize_header_text(&message.references.join(" ")),
        ));
    }
    headers.push("MIME-Version: 1.0".to_string());

    let body_section = build_body_section(message)?;
    for header in body_section.headers {
        match header.split_once(": ") {
            Some((name, value)) => headers.push(fold_header(name, value)),
            None => headers.push(header),
        }
    }

    let mut output = headers.join("\r\n");
    output.push_str("\r\n\r\n");
    output.push_str(&body_section.body);
    Ok(output)
}

struct BodySection {
    headers: Vec<String>,
    body: String,
}

fn build_body_section(message: &OutgoingMessage) -> Result<BodySection, BuildError> {
    let has_html = message
        .body_html
        .as_deref()
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);
    let has_attachments = !message.attachments.is_empty();

    if !has_attachments {
        if !has_html {
            return Ok(BodySection {
                headers: vec![
                    "Content-Type: text/plain; charset=UTF-8".to_string(),
                    "Content-Transfer-Encoding: quoted-printable".to_string(),
                ],
                body: encode_quoted_printable(&message.body),
            });
        }

        // alternative: text + html
        let boundary = make_boundary("alt");
        let mut body = String::new();
        push_alternative(
            &mut body,
            &boundary,
            &message.body,
            message.body_html.as_deref(),
        );
        body.push_str(&format!("--{boundary}--\r\n"));
        return Ok(BodySection {
            headers: vec![format!(
                "Content-Type: multipart/alternative; boundary=\"{boundary}\""
            )],
            body,
        });
    }

    let (inline, attached): (Vec<&OutgoingAttachment>, Vec<&OutgoingAttachment>) =
        message.attachments.iter().partition(|att| att.inline);

    // RFC 2387: a `cid:` reference only resolves inside the multipart/related
    // that holds both the HTML and the part it names.
    if inline.is_empty() {
        let mixed_boundary = make_boundary("mixed");
        let mut body = String::new();
        push_alternative_section(&mut body, &mixed_boundary, message);
        for attachment in attached {
            push_attachment(&mut body, &mixed_boundary, attachment)?;
        }
        body.push_str(&format!("--{mixed_boundary}--\r\n"));
        return Ok(BodySection {
            headers: vec![format!(
                "Content-Type: multipart/mixed; boundary=\"{mixed_boundary}\""
            )],
            body,
        });
    }

    let related_boundary = make_boundary("related");
    let related_content_type = format!(
        "multipart/related; type=\"multipart/alternative\"; boundary=\"{related_boundary}\""
    );
    let mut related = String::new();
    push_alternative_section(&mut related, &related_boundary, message);
    for attachment in inline {
        push_attachment(&mut related, &related_boundary, attachment)?;
    }
    related.push_str(&format!("--{related_boundary}--\r\n"));

    if attached.is_empty() {
        return Ok(BodySection {
            headers: vec![format!("Content-Type: {related_content_type}")],
            body: related,
        });
    }

    // A real attachment belongs beside the related entity, not inside it:
    // everything within the related layer is part of the displayed document.
    let mixed_boundary = make_boundary("mixed");
    let mut body = String::new();
    body.push_str(&format!("--{mixed_boundary}\r\n"));
    body.push_str(&format!("Content-Type: {related_content_type}\r\n\r\n"));
    body.push_str(&related);
    body.push_str("\r\n");
    for attachment in attached {
        push_attachment(&mut body, &mixed_boundary, attachment)?;
    }
    body.push_str(&format!("--{mixed_boundary}--\r\n"));

    Ok(BodySection {
        headers: vec![format!(
            "Content-Type: multipart/mixed; boundary=\"{mixed_boundary}\""
        )],
        body,
    })
}

/// Write the `multipart/alternative` part that carries the message text.
fn push_alternative_section(out: &mut String, outer_boundary: &str, message: &OutgoingMessage) {
    let alt_boundary = make_boundary("alt");
    out.push_str(&format!("--{outer_boundary}\r\n"));
    out.push_str(&format!(
        "Content-Type: multipart/alternative; boundary=\"{alt_boundary}\"\r\n\r\n"
    ));
    push_alternative(
        out,
        &alt_boundary,
        &message.body,
        message.body_html.as_deref(),
    );
    out.push_str(&format!("--{alt_boundary}--\r\n\r\n"));
}

fn push_alternative(out: &mut String, boundary: &str, body_text: &str, body_html: Option<&str>) {
    out.push_str(&format!("--{boundary}\r\n"));
    out.push_str("Content-Type: text/plain; charset=UTF-8\r\n");
    out.push_str("Content-Transfer-Encoding: quoted-printable\r\n\r\n");
    out.push_str(&encode_quoted_printable(body_text));
    out.push_str("\r\n");

    if let Some(html) = body_html.filter(|s| !s.trim().is_empty()) {
        out.push_str(&format!("--{boundary}\r\n"));
        out.push_str("Content-Type: text/html; charset=UTF-8\r\n");
        out.push_str("Content-Transfer-Encoding: quoted-printable\r\n\r\n");
        out.push_str(&encode_quoted_printable(html));
        out.push_str("\r\n");
    }
}

fn push_attachment(
    out: &mut String,
    boundary: &str,
    attachment: &OutgoingAttachment,
) -> Result<(), BuildError> {
    let data = attachment_base64(&attachment.content_base64).ok_or_else(|| {
        BuildError::InvalidAttachment {
            filename: attachment.filename.clone(),
        }
    })?;
    out.push_str(&format!("--{boundary}\r\n"));
    let mime = if attachment.mime_type.trim().is_empty() {
        "application/octet-stream"
    } else {
        attachment.mime_type.as_str()
    };
    out.push_str(&format!(
        "Content-Type: {mime}{}\r\n",
        header_parameter("name", &attachment.filename)
    ));
    out.push_str("Content-Transfer-Encoding: base64\r\n");
    let disposition = if attachment.inline {
        "inline"
    } else {
        "attachment"
    };
    out.push_str(&format!(
        "Content-Disposition: {disposition}{}\r\n",
        header_parameter("filename", &attachment.filename)
    ));
    if let Some(cid) = attachment.content_id.as_deref().filter(|s| !s.is_empty()) {
        out.push_str(&format!("Content-ID: <{cid}>\r\n"));
    }
    out.push_str("\r\n");
    out.push_str(&chunk_base64(&data));
    out.push_str("\r\n");
    Ok(())
}

/// The base64 payload without whitespace or a `data:…;base64,` prefix, if it decodes.
fn attachment_base64(raw: &str) -> Option<String> {
    use base64::Engine;
    let trimmed = raw.trim_start();
    let payload = match trimmed.strip_prefix("data:") {
        Some(url) => url.split_once(";base64,")?.1,
        None => trimmed,
    };
    let stripped: String = payload.chars().filter(|c| !c.is_whitespace()).collect();
    base64::engine::general_purpose::STANDARD
        .decode(&stripped)
        .ok()
        .map(|_| stripped)
}

fn make_boundary(kind: &str) -> String {
    format!("=_satchel_{}_{}", kind, uuid::Uuid::new_v4().simple())
}

fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '"' | '\\' | '\r' | '\n' => '_',
            other => other,
        })
        .collect()
}

fn chunk_base64(raw: &str) -> String {
    // RFC 2045: base64 lines should not exceed 76 characters.
    let stripped: String = raw.chars().filter(|c| !c.is_whitespace()).collect();
    let mut out = String::with_capacity(stripped.len() + stripped.len() / 76);
    let mut chars = stripped.chars().peekable();
    let mut col = 0;
    while let Some(ch) = chars.next() {
        out.push(ch);
        col += 1;
        if col >= 76 {
            out.push_str("\r\n");
            col = 0;
        }
        // No special handling needed at end; final newline is appended by caller.
        let _ = chars.peek();
    }
    out
}

/// RFC 5322 §2.1.1: a line SHOULD stay within 78 octets and MUST stay within
/// 998, both excluding the CRLF.
const FOLD_TARGET_OCTETS: usize = 78;

/// Fold `Name: value` onto continuation lines per RFC 5322 §2.2.3.
///
/// Breaks only at whitespace that is already in the value and keeps that
/// whitespace as the continuation's leading WSP, so dropping the inserted CRLF
/// restores the value byte for byte. A token longer than the target is left
/// whole on its own line — splitting a message id or an encoded word changes
/// what it means (RFC 2047 splitting is a separate job).
///
/// Order matters for signed mail: this runs while the message is being built,
/// before S/MIME signs the bytes and before PGP splits them into the signed
/// entity. Nothing may re-fold an already built message — that would rewrite
/// the very bytes the signature covers.
fn fold_header(name: &str, value: &str) -> String {
    let line = format!("{name}: {value}");
    if line.len() <= FOLD_TARGET_OCTETS {
        return line;
    }

    let mut out = String::with_capacity(line.len() + line.len() / FOLD_TARGET_OCTETS * 3);
    let mut used = 0usize;
    let mut rest = line.as_str();
    while !rest.is_empty() {
        // One chunk is the whitespace run in front of a token plus the token.
        let token_start = rest
            .find(|ch: char| ch != ' ' && ch != '\t')
            .unwrap_or(rest.len());
        let (space, tail) = rest.split_at(token_start);
        let token_end = tail.find([' ', '\t']).unwrap_or(tail.len());
        let (token, next) = tail.split_at(token_end);

        let chunk = space.len() + token.len();
        if used > 0 && !space.is_empty() && !token.is_empty() && used + chunk > FOLD_TARGET_OCTETS {
            out.push_str("\r\n");
            used = 0;
        }
        out.push_str(space);
        out.push_str(token);
        used += chunk;
        rest = next;
    }
    out
}

/// Split a header block into logical headers, the inverse of [`fold_header`]:
/// a line starting with SP or HTAB continues the header above it (RFC 5322
/// §2.2.3). Filtering per line instead drops a folded header's name-carrying
/// first line and leaves its continuations grafted onto whatever precedes them.
///
/// Reads up to the first empty line, so a whole message may be passed in.
pub fn split_logical_headers(header_block: &str) -> Vec<String> {
    let mut headers: Vec<String> = Vec::new();
    for line in header_block.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            break;
        }
        match headers.last_mut() {
            Some(previous) if line.starts_with([' ', '\t']) => {
                previous.push_str("\r\n");
                previous.push_str(line);
            }
            _ => headers.push(line.to_string()),
        }
    }
    headers
}

/// Whether a logical header describes the entity's content rather than its
/// delivery. These belong to the MIME part a crypto transform replaces or
/// signs, so they must not be copied onto the outer message.
pub fn is_content_header(header: &str) -> bool {
    let lower = header.to_ascii_lowercase();
    lower.starts_with("content-type:")
        || lower.starts_with("content-transfer-encoding:")
        || lower.starts_with("content-disposition:")
        || lower.starts_with("content-id:")
        || lower.starts_with("mime-version:")
}

/// Strip CR/LF (and other ASCII control chars) from text destined for a header
/// line. Without this, a subject decoded from an incoming mail's RFC2047 header
/// (e.g. "Foo\r\nBcc: attacker@evil.com") would inject attacker-controlled
/// headers into the victim's reply.
fn sanitize_header_text(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_control() && ch != '\t' {
                ' '
            } else {
                ch
            }
        })
        .collect()
}

/// `"Anna Meier" <a@x.de>, b@y.de` from bare addresses and the names beside them.
fn mailbox_list(message: &OutgoingMessage, addresses: &[String]) -> String {
    addresses
        .iter()
        .map(|address| {
            let name = message
                .recipient_names
                .get(&address.to_ascii_lowercase())
                .map(String::as_str);
            format_mailbox(name, address)
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn format_mailbox(name: Option<&str>, address: &str) -> String {
    let address = sanitize_header_text(address);
    match name {
        Some(name) if !name.trim().is_empty() => {
            let needs_encoding = name.chars().any(|c| !c.is_ascii() || c == '"');
            if needs_encoding {
                format!("{} <{}>", encode_header_value(name), address)
            } else {
                format!(
                    "\"{}\" <{}>",
                    sanitize_header_text(name).replace('"', "\\\""),
                    address
                )
            }
        }
        _ => address,
    }
}

/// How many input bytes go into one encoded word.
///
/// RFC 2047 §2 caps an encoded word at 75 characters including `=?UTF-8?B?`
/// and `?=`. 42 bytes base64-encode to 56, so a word measures 68 — short enough
/// that `Subject: ` plus one word still clears the 78-octet fold target and the
/// header folds between words rather than right after the colon.
const ENCODED_WORD_INPUT_BYTES: usize = 42;

/// Encode a header value as one or more RFC 2047 encoded words, joined by a
/// space that [`fold_header`] turns into the `CRLF SP` the RFC asks for.
///
/// A decoder drops whitespace between adjacent encoded words, so a value split
/// across two words arrives whole — as long as the split lands on a character
/// boundary, which is why the chunking walks characters, not bytes.
fn encode_header_value(value: &str) -> String {
    if value.is_ascii() {
        return sanitize_header_text(value);
    }

    let sanitized = sanitize_header_text(value);
    let mut words: Vec<String> = Vec::new();
    let mut chunk = String::new();
    for ch in sanitized.chars() {
        if chunk.len() + ch.len_utf8() > ENCODED_WORD_INPUT_BYTES {
            words.push(encoded_word(&chunk));
            chunk.clear();
        }
        chunk.push(ch);
    }
    if !chunk.is_empty() || words.is_empty() {
        words.push(encoded_word(&chunk));
    }
    words.join(" ")
}

fn encoded_word(chunk: &str) -> String {
    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(chunk.as_bytes());
    format!("=?UTF-8?B?{encoded}?=")
}

/// Percent-encode a parameter value per RFC 2231 §4. Everything outside the
/// attribute character set is escaped, which is stricter than required and
/// never wrong.
fn percent_encode_parameter(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// Render one MIME header parameter, ASCII-clean in every case.
///
/// A header may not carry raw 8-bit bytes, so a non-ASCII value goes into the
/// RFC 2231 extended form. The quoted ASCII form stays beside it: clients that
/// never learned `filename*` would otherwise show no name at all.
fn header_parameter(name: &str, value: &str) -> String {
    let sanitized = sanitize_filename(value);
    if sanitized.is_ascii() {
        return format!("; {name}=\"{sanitized}\"");
    }
    let fallback: String = sanitized
        .chars()
        .map(|ch| if ch.is_ascii() { ch } else { '_' })
        .collect();
    format!(
        "; {name}=\"{fallback}\"; {name}*=UTF-8''{}",
        percent_encode_parameter(&sanitized)
    )
}

/// RFC 2045 §6.7 rule 5: a quoted-printable line, soft break included, stays
/// within 76 characters.
const MAX_QUOTED_PRINTABLE_LINE: usize = 76;

/// Quoted-printable encoder.
///
/// Works line by line because the two rules that are easy to get wrong are
/// properties of a line, not of a byte: whitespace may not end one (rule 3 —
/// a relay is allowed to strip it, and for PGP that turns a valid signature
/// into an invalid one), and `From ` may not start one.
fn encode_quoted_printable(value: &str) -> String {
    let normalized = value.replace("\r\n", "\n").replace('\r', "\n");
    let mut out = String::with_capacity(value.len());
    for (index, line) in normalized.split('\n').enumerate() {
        if index > 0 {
            out.push_str("\r\n");
        }
        push_quoted_printable_line(&mut out, line);
    }
    out
}

fn push_quoted_printable_line(out: &mut String, line: &str) {
    let mut tokens: Vec<String> = line
        .bytes()
        .map(|byte| {
            if matches!(byte, 0..=8 | 11..=12 | 14..=31 | 61 | 127..=255) {
                format!("={byte:02X}")
            } else {
                (byte as char).to_string()
            }
        })
        .collect();

    // Anything that stores mail as mbox reads a body line beginning with
    // "From " as the start of the next message.
    if line.starts_with("From ") {
        tokens[0] = "=46".to_string();
    }
    if let Some(last) = tokens.last_mut() {
        match last.as_str() {
            " " => *last = "=20".to_string(),
            "\t" => *last = "=09".to_string(),
            _ => {}
        }
    }

    let mut line_len = 0usize;
    for (index, token) in tokens.iter().enumerate() {
        // One character is held back for the soft break that may follow.
        let needs_break = if matches!(token.as_str(), " " | "\t") {
            // A soft break may not follow whitespace either: break in front of
            // it instead, so it never ends up last.
            let next = tokens.get(index + 1).map_or(0, String::len);
            line_len + token.len() + next + 1 > MAX_QUOTED_PRINTABLE_LINE
        } else {
            line_len + token.len() + 1 > MAX_QUOTED_PRINTABLE_LINE
        };
        if needs_break && line_len > 0 {
            out.push_str("=\r\n");
            line_len = 0;
        }
        out.push_str(token);
        line_len += token.len();
    }
}

#[cfg(test)]
mod tests {
    use super::{
        build_rfc822, build_rfc822_with_bcc, chunk_base64, encode_quoted_printable, fold_header,
        is_content_header, split_logical_headers, with_bcc_delivery_header, BuildError,
        FOLD_TARGET_OCTETS,
    };
    use crate::providers::traits::{OutgoingAttachment, OutgoingMessage};
    use crate::utils::mime::{decode_mime_encoded_words, decode_quoted_printable_bytes};

    /// RFC 5322 §2.1.1: the hard ceiling, CRLF excluded.
    const HARD_LIMIT_OCTETS: usize = 998;

    fn references(count: usize) -> Vec<String> {
        (0..count)
            .map(|index| format!("<thread-{index:03}-9f2c4b8a1d6e4f0b93c7@mail.example.test>"))
            .collect()
    }

    /// A reply deep in a German thread: long subject, 40 references, 20
    /// recipients, one attachment.
    fn realistic_reply() -> OutgoingMessage {
        OutgoingMessage {
            from_address: Some("max.mustermann@beispiel.test".to_owned()),
            from_name: Some("Max Mustermann (Geschäftsführung)".to_owned()),
            to: (0..20)
                .map(|index| {
                    format!("empfaenger.nummer.{index:02}@sehr-langer-mandantenname.beispiel.test")
                })
                .collect(),
            cc: vec!["verteiler-projektleitung@beispiel.test".to_owned()],
            bcc: vec!["revision@beispiel.test".to_owned()],
            subject: "Rückfrage zur Angebotskalkulation und Terminplanung "
                .repeat(6)
                .trim_end()
                .to_owned(),
            body: "Guten Tag,\n\nanbei die überarbeitete Fassung.\n\nViele Grüße".to_owned(),
            body_html: Some("<p>Guten Tag,</p><p>anbei die überarbeitete Fassung.</p>".to_owned()),
            in_reply_to: Some("<thread-039-9f2c4b8a1d6e4f0b93c7@mail.example.test>".to_owned()),
            references: references(40),
            attachments: vec![OutgoingAttachment {
                filename: "Angebotskalkulation.pdf".to_owned(),
                mime_type: "application/pdf".to_owned(),
                content_base64: {
                    use base64::Engine;
                    base64::engine::general_purpose::STANDARD.encode("Angebot".repeat(400))
                },
                content_id: None,
                inline: false,
            }],
            ..Default::default()
        }
    }

    fn message_with_bcc() -> OutgoingMessage {
        OutgoingMessage {
            to: vec!["her@example.test".to_owned()],
            cc: vec!["team@example.test".to_owned()],
            bcc: vec!["chef@example.test".to_owned()],
            subject: "quarterly figures".to_owned(),
            body: "plain text".to_owned(),
            ..Default::default()
        }
    }

    #[test]
    fn only_the_header_addressed_builder_emits_bcc() {
        let message = message_with_bcc();

        let smtp = build_rfc822(&message, Some("me@example.test"), None).unwrap();
        assert!(!smtp.contains("chef@example.test"));

        let api = build_rfc822_with_bcc(&message, Some("me@example.test"), None).unwrap();
        assert!(api.contains("Bcc: chef@example.test"));
    }

    #[test]
    fn the_bcc_delivery_header_stays_outside_the_signed_entity() {
        let signed = "From: me@example.test\r\nTo: her@example.test\r\n\
                      Content-Type: multipart/signed; boundary=\"b\"\r\n\r\n\
                      --b\r\nContent-Type: text/plain\r\n\r\nsigned body\r\n--b--\r\n";

        let out = with_bcc_delivery_header(signed, &["chef@example.test".to_owned()]);
        let (headers, body) = out.split_once("\r\n\r\n").expect("header/body split");

        assert!(headers.contains("Bcc: chef@example.test"));
        assert!(body.contains("signed body"));
        assert!(
            !body.contains("chef@example.test"),
            "the signed entity must not change, or the signature stops verifying"
        );
    }

    #[test]
    fn the_bcc_delivery_header_cannot_smuggle_a_second_header() {
        let out = with_bcc_delivery_header(
            "From: me@example.test\r\n\r\nbody",
            &["chef@example.test\r\nBcc: attacker@evil.test".to_owned()],
        );

        assert_eq!(
            out.lines().filter(|line| line.starts_with("Bcc:")).count(),
            1
        );
    }

    #[test]
    fn an_empty_bcc_list_leaves_the_message_untouched() {
        let original = "From: me@example.test\r\n\r\nbody";
        assert_eq!(with_bcc_delivery_header(original, &[]), original);
    }

    #[test]
    fn folds_a_long_references_header() {
        let value = references(40).join(" ");
        let folded = fold_header("References", &value);
        let lines: Vec<&str> = folded.split("\r\n").collect();

        assert!(lines.len() > 1, "40 references have to fold");
        assert!(lines[0].starts_with("References: "));
        for line in &lines[1..] {
            assert!(
                line.starts_with(' '),
                "continuation must start with SP: {line:?}"
            );
        }
        for line in &lines {
            assert!(line.len() <= FOLD_TARGET_OCTETS, "line too long: {line:?}");
            // A break inside a message id would leave an unbalanced pair.
            assert_eq!(line.matches('<').count(), line.matches('>').count());
        }
    }

    #[test]
    fn folding_round_trips() {
        let value = references(40).join(" ");
        let folded = fold_header("References", &value);

        assert!(folded.contains("\r\n "));
        // RFC 5322 §2.2.3 unfolding: drop the CRLF, keep the whitespace.
        assert_eq!(folded.replace("\r\n", ""), format!("References: {value}"));
    }

    #[test]
    fn does_not_split_an_oversized_token() {
        let oversized = format!("<{}@mail.example.test>", "z".repeat(200));
        let value = format!("<kurz@mail.example.test> {oversized} <auch-kurz@mail.example.test>");
        let folded = fold_header("References", &value);

        assert!(
            folded.split("\r\n").any(|line| line.trim() == oversized),
            "the oversized message id must stay whole on its own line"
        );
        assert_eq!(folded.replace("\r\n", ""), format!("References: {value}"));
    }

    #[test]
    fn a_continuation_line_belongs_to_the_header_that_started_it() {
        let block = "References: <a@x.test>\r\n <b@x.test>\r\n\
                     Content-Type: multipart/mixed;\r\n\tboundary=\"b\"\r\n\
                     Subject: after\r\n\r\nbody";

        let headers = split_logical_headers(block);

        assert_eq!(
            headers,
            vec![
                "References: <a@x.test>\r\n <b@x.test>",
                "Content-Type: multipart/mixed;\r\n\tboundary=\"b\"",
                "Subject: after",
            ]
        );
        assert_eq!(
            headers.iter().filter(|h| is_content_header(h)).count(),
            1,
            "the folded Content-Type is one header, not two"
        );
    }

    #[test]
    fn dropping_a_folded_header_takes_its_continuations_with_it() {
        let block = "References: <a@x.test>\r\n <b@x.test>\r\n\
                     Content-Type: multipart/mixed;\r\n boundary=\"b\"\r\n";

        let kept: Vec<String> = split_logical_headers(block)
            .into_iter()
            .filter(|header| !is_content_header(header))
            .collect();

        assert_eq!(kept, vec!["References: <a@x.test>\r\n <b@x.test>"]);
        // The line-wise predecessor of this filter kept ` boundary="b"`, which
        // then read as a continuation of References.
        assert!(!kept.join("\r\n").contains("boundary"));
    }

    #[test]
    fn built_content_headers_fold_like_every_other_header() {
        let message = OutgoingMessage {
            to: vec!["her@example.test".to_owned()],
            body: "plain".to_owned(),
            body_html: Some("<p>plain</p>".to_owned()),
            ..Default::default()
        };

        let raw = build_rfc822(&message, Some("me@example.test"), None).unwrap();
        let (headers, _) = raw.split_once("\r\n\r\n").expect("header/body split");
        let content_type = split_logical_headers(headers)
            .into_iter()
            .find(|header| header.to_ascii_lowercase().starts_with("content-type:"))
            .expect("a Content-Type header");

        assert!(
            content_type.contains("\r\n "),
            "the boundary pushes this header past the fold target: {content_type:?}"
        );
        for line in content_type.split("\r\n") {
            assert!(line.len() <= FOLD_TARGET_OCTETS, "line too long: {line:?}");
        }
        assert_eq!(
            content_type.replace("\r\n", ""),
            format!(
                "Content-Type: multipart/alternative; boundary=\"{}\"",
                content_type
                    .split_once("boundary=\"")
                    .and_then(|(_, tail)| tail.split_once('"'))
                    .expect("a boundary")
                    .0
            )
        );
    }

    /// RFC 2047 §2: an encoded word, delimiters included, stays within this.
    const MAX_ENCODED_WORD: usize = 75;

    fn header_value(rfc822: &str, name: &str) -> String {
        let (headers, _) = rfc822.split_once("\r\n\r\n").expect("header/body split");
        split_logical_headers(headers)
            .into_iter()
            .find(|header| {
                header
                    .to_ascii_lowercase()
                    .starts_with(&name.to_ascii_lowercase())
            })
            .unwrap_or_else(|| panic!("no {name} header in {rfc822:?}"))
    }

    fn encoded_words(header: &str) -> Vec<String> {
        header
            .split(|ch: char| ch.is_whitespace())
            .filter(|word| word.starts_with("=?") && word.ends_with("?="))
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn splits_long_encoded_words() {
        let subject = "Rückfrage zur Angebotsposition vier, Lieferung im März ".repeat(4);
        let message = OutgoingMessage {
            to: vec!["her@example.test".to_owned()],
            subject: subject.trim_end().to_owned(),
            body: "kurz".to_owned(),
            ..Default::default()
        };

        let header = header_value(
            &build_rfc822(&message, Some("me@example.test"), None).unwrap(),
            "Subject:",
        );
        let words = encoded_words(&header);

        assert!(words.len() > 1, "one word cannot hold 200 characters");
        for word in &words {
            assert!(
                word.len() <= MAX_ENCODED_WORD,
                "encoded word of {} characters: {word}",
                word.len()
            );
        }
    }

    #[test]
    fn encoded_word_round_trips() {
        // The second subject puts a two-byte character on the 42-byte chunk
        // boundary: splitting there would tear the character in half.
        for subject in [
            "Rückfrage zur Angebotsposition vier, Lieferung im März ".repeat(4),
            format!("{}ä{}", "a".repeat(41), "b".repeat(50)),
            "Größe".to_owned(),
        ] {
            let message = OutgoingMessage {
                to: vec!["her@example.test".to_owned()],
                subject: subject.clone(),
                body: "kurz".to_owned(),
                ..Default::default()
            };

            let header = header_value(
                &build_rfc822(&message, Some("me@example.test"), None).unwrap(),
                "Subject:",
            );
            let value = header
                .strip_prefix("Subject: ")
                .expect("subject prefix")
                .replace("\r\n", "");

            assert_eq!(decode_mime_encoded_words(&value), subject.trim());
        }
    }

    #[test]
    fn a_non_ascii_display_name_leaves_no_raw_bytes_in_from() {
        let message = OutgoingMessage {
            from_name: Some("Jürgen Baumgartner-Schilling, Geschäftsführung".to_owned()),
            to: vec!["her@example.test".to_owned()],
            body: "kurz".to_owned(),
            ..Default::default()
        };

        let header = header_value(
            &build_rfc822(&message, Some("juergen@beispiel.test"), None).unwrap(),
            "From:",
        );

        assert!(header.is_ascii(), "raw UTF-8 in a header: {header:?}");
        for word in encoded_words(&header) {
            assert!(word.len() <= MAX_ENCODED_WORD, "{word}");
        }
        assert!(header.ends_with("<juergen@beispiel.test>"));
    }

    fn message_with_attachment(filename: &str) -> OutgoingMessage {
        OutgoingMessage {
            to: vec!["her@example.test".to_owned()],
            body: "anbei".to_owned(),
            attachments: vec![OutgoingAttachment {
                filename: filename.to_owned(),
                mime_type: "application/pdf".to_owned(),
                content_base64: "QW5nZWJvdA==".to_owned(),
                content_id: None,
                inline: false,
            }],
            ..Default::default()
        }
    }

    #[test]
    fn encodes_non_ascii_filenames_per_rfc2231() {
        let raw = build_rfc822(
            &message_with_attachment("Angebot Größe März.pdf"),
            Some("me@example.test"),
            None,
        )
        .unwrap();

        let disposition = raw
            .split("\r\n")
            .find(|line| line.starts_with("Content-Disposition:"))
            .expect("a Content-Disposition");
        assert!(disposition.is_ascii(), "raw 8-bit bytes: {disposition:?}");
        assert!(disposition.contains("filename*=UTF-8''Angebot%20Gr%C3%B6%C3%9Fe%20M%C3%A4rz.pdf"));
        assert!(
            disposition.contains("filename=\"Angebot Gr__e M_rz.pdf\""),
            "clients without filename* still need a name: {disposition:?}"
        );

        let content_type = raw
            .split("\r\n")
            .find(|line| line.starts_with("Content-Type: application/pdf"))
            .expect("the attachment's Content-Type");
        assert!(content_type.is_ascii(), "{content_type:?}");
        assert!(content_type.contains("name*=UTF-8''"));
    }

    #[test]
    fn keeps_ascii_filenames_verbatim() {
        let raw = build_rfc822(
            &message_with_attachment("report.pdf"),
            Some("me@example.test"),
            None,
        )
        .unwrap();

        assert!(raw.contains("Content-Disposition: attachment; filename=\"report.pdf\"\r\n"));
        assert!(raw.contains("Content-Type: application/pdf; name=\"report.pdf\"\r\n"));
        assert!(!raw.contains("filename*="));
    }

    #[test]
    fn encodes_trailing_space_at_hard_line_break() {
        // RFC 3676 signature delimiter: the space is the whole point.
        let encoded = encode_quoted_printable("Viele Grüße\n-- \nMax");

        assert!(encoded.contains("--=20\r\n"), "{encoded:?}");
        assert!(!encoded.contains("-- \r\n"));
    }

    #[test]
    fn encodes_trailing_tab_before_soft_break() {
        let line = format!("{}\t{}", "a".repeat(70), "b".repeat(70));
        let encoded = encode_quoted_printable(&line);

        for line in encoded.split("\r\n") {
            let content = line.strip_suffix('=').unwrap_or(line);
            assert!(
                !content.ends_with(' ') && !content.ends_with('\t'),
                "whitespace at a line end does not survive a relay: {line:?}"
            );
            assert!(line.len() <= 76, "line of {} characters", line.len());
        }
        assert_eq!(
            String::from_utf8(decode_quoted_printable_bytes(encoded.as_bytes())).unwrap(),
            line
        );
    }

    #[test]
    fn escapes_from_at_line_start() {
        let encoded = encode_quoted_printable("From here on\nnot From here\n");

        assert!(encoded.starts_with("=46rom here on\r\n"), "{encoded:?}");
        assert!(encoded.contains("not From here"), "only at a line start");
    }

    #[test]
    fn quoted_printable_round_trips() {
        let source = "Guten Tag Herr Müller,\n\nanbei die Übersicht.   \n\
                      From dort aus geht es weiter.\t\n\
                      Ein sehr langer Absatz ohne Umbruch, damit der weiche Zeilenumbruch                       auch wirklich einmal greift und die Grüße am Ende noch ankommen.\n-- \nMax";

        let encoded = encode_quoted_printable(source);
        let decoded = String::from_utf8(decode_quoted_printable_bytes(encoded.as_bytes()))
            .expect("utf-8 round trip");

        assert_eq!(decoded, source.replace('\n', "\r\n"));
        for line in encoded.split("\r\n") {
            assert!(
                line.len() <= 76,
                "line of {} characters: {line:?}",
                line.len()
            );
        }
    }

    fn with_attachment_data(content_base64: &str) -> OutgoingMessage {
        let mut file = attachment("angebot.pdf", false, None);
        file.content_base64 = content_base64.to_owned();
        OutgoingMessage {
            to: vec!["kunde@example.test".into()],
            subject: "Angebot".into(),
            body: "Anbei.".into(),
            attachments: vec![file],
            ..OutgoingMessage::default()
        }
    }

    fn attachment_part(raw: &str) -> String {
        let (_, part) = raw
            .split_once("filename=\"angebot.pdf\"\r\n\r\n")
            .expect("attachment part");
        part.split("\r\n--").next().unwrap().to_owned()
    }

    #[test]
    fn recipient_display_names_are_quoted_or_encoded_in_the_header() {
        let mut message = OutgoingMessage {
            to: vec!["a.meier@kunde.de".into(), "bare@kunde.de".into()],
            cc: vec!["m@x.de".into(), "morgan@x.de".into()],
            subject: "Termin".into(),
            body: "Hallo".into(),
            ..OutgoingMessage::default()
        };
        for (address, name) in [
            ("a.meier@kunde.de", "Anna Meier"),
            ("m@x.de", "Anna Müller"),
            ("morgan@x.de", "Morgan, Alex"),
        ] {
            message.recipient_names.insert(address.into(), name.into());
        }

        let raw = build_rfc822(&message, Some("me@example.test"), None).unwrap();
        let header = |name: &str| {
            let (_, rest) = raw.split_once(&format!("\r\n{name}: ")).expect(name);
            rest.split("\r\n")
                .take_while(|line| !line.is_empty())
                .enumerate()
                .take_while(|(index, line)| *index == 0 || line.starts_with(' '))
                .map(|(_, line)| line.trim_start())
                .collect::<Vec<_>>()
                .join(" ")
        };

        assert_eq!(
            header("To"),
            "\"Anna Meier\" <a.meier@kunde.de>, bare@kunde.de"
        );
        let cc = header("Cc");
        assert!(cc.is_ascii(), "{cc}");
        assert!(cc.contains("\"Morgan, Alex\" <morgan@x.de>"), "{cc}");
        assert_eq!(
            decode_mime_encoded_words(&cc),
            "Anna Müller <m@x.de>, \"Morgan, Alex\" <morgan@x.de>"
        );
    }

    #[test]
    fn invalid_attachment_base64_is_an_error_not_a_message() {
        let error = build_rfc822(
            &with_attachment_data("not*base64!"),
            Some("me@example.test"),
            None,
        )
        .unwrap_err();

        assert_eq!(
            error,
            BuildError::InvalidAttachment {
                filename: "angebot.pdf".into()
            }
        );
        assert!(error
            .encoded()
            .contains("backend.errors.attachmentNotBase64"));
    }

    #[test]
    fn a_data_url_prefix_never_reaches_the_attachment() {
        let plain = build_rfc822(
            &with_attachment_data("QW5nZWJvdA=="),
            Some("me@example.test"),
            None,
        )
        .unwrap();
        let data_url = build_rfc822(
            &with_attachment_data("data:application/pdf;base64,QW5nZWJvdA=="),
            Some("me@example.test"),
            None,
        )
        .unwrap();

        assert_eq!(attachment_part(&data_url), attachment_part(&plain));
        assert!(!data_url.contains("data:application/pdf"));
    }

    #[test]
    fn valid_base64_with_line_breaks_is_chunked_as_before() {
        use base64::Engine;
        let encoded = base64::engine::general_purpose::STANDARD.encode(vec![7u8; 300]);
        let wrapped = encoded
            .as_bytes()
            .chunks(60)
            .map(|line| std::str::from_utf8(line).unwrap())
            .collect::<Vec<_>>()
            .join("\n");

        let raw = build_rfc822(
            &with_attachment_data(&wrapped),
            Some("me@example.test"),
            None,
        )
        .unwrap();

        assert_eq!(attachment_part(&raw), chunk_base64(&encoded));
    }

    fn attachment(filename: &str, inline: bool, content_id: Option<&str>) -> OutgoingAttachment {
        OutgoingAttachment {
            filename: filename.to_owned(),
            mime_type: if inline {
                "image/png".to_owned()
            } else {
                "application/pdf".to_owned()
            },
            content_base64: "QW5nZWJvdA==".to_owned(),
            content_id: content_id.map(str::to_owned),
            inline,
        }
    }

    fn message_with(attachments: Vec<OutgoingAttachment>) -> OutgoingMessage {
        OutgoingMessage {
            to: vec!["her@example.test".to_owned()],
            subject: "Angebot".to_owned(),
            body: "Guten Tag".to_owned(),
            body_html: Some("<p>Guten Tag <img src=\"cid:logo\"></p>".to_owned()),
            attachments,
            ..Default::default()
        }
    }

    /// The order in which the multipart types appear in the built message.
    fn multipart_order(raw: &str) -> Vec<&str> {
        raw.split("\r\n")
            .filter_map(|line| {
                let value = line.trim_start().strip_prefix("Content-Type: ")?;
                [
                    "multipart/mixed",
                    "multipart/related",
                    "multipart/alternative",
                ]
                .into_iter()
                .find(|kind| value.starts_with(kind))
            })
            .collect()
    }

    #[test]
    fn inline_attachment_produces_multipart_related() {
        let raw = build_rfc822(
            &message_with(vec![attachment("logo.png", true, Some("logo"))]),
            Some("me@example.test"),
            None,
        )
        .unwrap();

        assert_eq!(
            multipart_order(&raw),
            vec!["multipart/related", "multipart/alternative"]
        );
        assert!(raw.contains("Content-ID: <logo>"));
        assert!(raw.contains("cid:logo"));
    }

    #[test]
    fn mixed_wraps_related_when_a_real_attachment_is_present() {
        let raw = build_rfc822(
            &message_with(vec![
                attachment("logo.png", true, Some("logo")),
                attachment("Angebot.pdf", false, None),
            ]),
            Some("me@example.test"),
            None,
        )
        .unwrap();

        assert_eq!(
            multipart_order(&raw),
            vec![
                "multipart/mixed",
                "multipart/related",
                "multipart/alternative"
            ]
        );
        // The real attachment sits beside the related entity, not inside it.
        let related_boundary = raw
            .split_once("multipart/related; type=\"multipart/alternative\"; boundary=\"")
            .and_then(|(_, tail)| tail.split_once('"'))
            .expect("a related boundary")
            .0
            .to_owned();
        let (_, after_related_close) = raw
            .split_once(&format!("--{related_boundary}--"))
            .expect("the related part closes");
        assert!(after_related_close.contains("filename=\"Angebot.pdf\""));
    }

    #[test]
    fn no_related_layer_without_inline_attachments() {
        let raw = build_rfc822(
            &message_with(vec![attachment("Angebot.pdf", false, None)]),
            Some("me@example.test"),
            None,
        )
        .unwrap();

        assert_eq!(
            multipart_order(&raw),
            vec!["multipart/mixed", "multipart/alternative"]
        );
        assert!(!raw.contains("multipart/related"));
    }

    #[test]
    fn an_inline_attachment_without_a_content_id_still_gets_the_related_layer() {
        let raw = build_rfc822(
            &message_with(vec![attachment("logo.png", true, None)]),
            Some("me@example.test"),
            None,
        )
        .unwrap();

        assert_eq!(
            multipart_order(&raw),
            vec!["multipart/related", "multipart/alternative"]
        );
        assert!(!raw.contains("Content-ID:"));
    }

    #[test]
    fn no_line_exceeds_998_octets() {
        let message = realistic_reply();
        let smtp = build_rfc822(&message, Some("max@beispiel.test"), None).unwrap();
        let api = build_rfc822_with_bcc(&message, Some("max@beispiel.test"), None).unwrap();
        let delivered = with_bcc_delivery_header(&smtp, &message.bcc);

        for rendering in [&smtp, &api, &delivered] {
            for line in rendering.split("\r\n") {
                assert!(
                    line.len() <= HARD_LIMIT_OCTETS,
                    "line of {} octets: {}…",
                    line.len(),
                    line.chars().take(60).collect::<String>()
                );
            }
        }
        assert!(smtp.contains("\r\n <thread-"), "references have to fold");
    }

    #[test]
    fn folding_survives_the_pgp_entity_split() {
        // The folding happens while building, so the signature covers the folded
        // bytes; re-folding afterwards would break verification.
        let raw = build_rfc822(&realistic_reply(), Some("max@beispiel.test"), None).unwrap();
        let entity = crate::email::pgp_mime::signable_entity(&raw);
        let signed = crate::email::pgp_mime::build_signed(&raw, "-----BEGIN PGP SIGNATURE-----");

        assert!(
            signed.contains(&entity),
            "the signed entity must reappear byte for byte in the message"
        );
        assert!(
            signed.contains("\r\n <thread-"),
            "folded delivery headers must survive the split"
        );
    }

    #[test]
    fn a_folded_bcc_delivery_header_stays_a_single_header() {
        let bcc: Vec<String> = (0..12)
            .map(|index| format!("verdeckt.{index:02}@sehr-langer-mandantenname.beispiel.test"))
            .collect();

        let out = with_bcc_delivery_header("From: me@example.test\r\n\r\nbody", &bcc);
        let (headers, _) = out.split_once("\r\n\r\n").expect("header/body split");
        let lines: Vec<&str> = headers.split("\r\n").collect();

        assert_eq!(
            lines.iter().filter(|line| line.starts_with("Bcc:")).count(),
            1
        );
        assert!(
            lines.iter().any(|line| line.starts_with(' ')),
            "has to fold"
        );
        for line in &lines {
            assert!(line.len() <= HARD_LIMIT_OCTETS);
        }
    }
}
