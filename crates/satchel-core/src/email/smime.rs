//! Decode decrypted MIME without treating extracted content as a valid signature.
use crate::providers::imap::{parse_raw_message_body, ParsedBody};

/// The outbox's sign-then-encrypt transform, independent of credential storage.
pub fn protect_message(
    bytes: &[u8],
    signing: Option<(&str, &str, Option<&str>)>,
    recipients: Option<&[String]>,
) -> Result<Vec<u8>, String> {
    let signed = match signing {
        Some((key, cert, passphrase)) => {
            crate::utils::smime_engine::sign(key, cert, passphrase, bytes)?
        }
        None => bytes.to_vec(),
    };
    match recipients {
        Some(certs) => crate::utils::smime_engine::encrypt(certs, &signed),
        None => Ok(signed),
    }
}

pub struct DecryptedBody {
    pub body: ParsedBody,
    /// Exact signed payload and whether it covers the complete displayed body.
    pub signatures: Vec<(Vec<u8>, bool)>,
}

/// Keep bytes intact until the MIME parser decodes each part's charset and CTE.
/// Opaque signed-data must be unwrapped separately from signature verification:
/// a damaged signature can still contain readable mail, but never gains trust.
pub fn parse_decrypted_body(bytes: &[u8]) -> Result<DecryptedBody, String> {
    let mut body = parse_raw_message_body(bytes);
    let mut signatures = Vec::new();
    for _ in 0..10 {
        let Some(payload) = body.smime_signed.take() else {
            return Ok(finish(body, signatures));
        };
        let root = body.trust.root_crypto;
        if !root {
            // A signed attachment cannot replace an unsigned cover note.
            signatures.push((payload, false));
            return Ok(finish(body, signatures));
        }
        let content = crate::utils::smime_engine::signed_content(&payload);
        signatures.push((payload, true));
        match content {
            Ok(content) => body = parse_raw_message_body(&content),
            // multipart/signed can retain its readable entity even when the
            // signature structure is broken. Verification still receives it.
            Err(_) if body.text.is_some() || body.html.is_some() => {
                return Ok(finish(body, signatures));
            }
            Err(error) => return Err(error),
        }
    }
    Err("S/MIME nesting limit exceeded".into())
}

fn finish(mut body: ParsedBody, signatures: Vec<(Vec<u8>, bool)>) -> DecryptedBody {
    if let Some(html) = body.html.take() {
        let sanitized = crate::email::sanitizer::sanitize_email_html(&html);
        if body.text.is_none() {
            body.text = Some(crate::utils::mime::decode_html_entities(
                &crate::email::text::strip_html_tags(&sanitized),
            ));
        }
        body.html = Some(sanitized);
    }
    DecryptedBody { body, signatures }
}
