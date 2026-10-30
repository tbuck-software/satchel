//! Decode decrypted MIME without treating extracted content as a valid signature.
use crate::providers::imap::{
    check_complete_mime, parse_raw_message_body, IncompleteMime, ParsedBody,
};

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    NestingLimit,
    RemainingEncryption,
    MalformedSigned,
    /// A multipart entity without a boundary or without any part.
    MalformedMime,
    /// A part that would reach neither the body nor the attachment list.
    UndecodedPart,
    InvalidSignedContent(String),
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NestingLimit => f.write_str("S/MIME nesting limit exceeded"),
            Self::RemainingEncryption => f.write_str("S/MIME content is still encrypted"),
            Self::MalformedSigned => f.write_str("Malformed S/MIME signed MIME structure"),
            Self::MalformedMime => f.write_str("Malformed MIME structure in decrypted S/MIME"),
            Self::UndecodedPart => {
                f.write_str("Decrypted S/MIME contains a part that cannot be shown")
            }
            Self::InvalidSignedContent(error) => f.write_str(error),
        }
    }
}

impl std::error::Error for DecodeError {}

impl From<IncompleteMime> for DecodeError {
    fn from(gap: IncompleteMime) -> Self {
        match gap {
            IncompleteMime::NestingLimit => Self::NestingLimit,
            IncompleteMime::MalformedMultipart => Self::MalformedMime,
            IncompleteMime::MalformedSigned => Self::MalformedSigned,
            IncompleteMime::RemainingEncryption => Self::RemainingEncryption,
            IncompleteMime::UndecodedPart => Self::UndecodedPart,
        }
    }
}

/// A decrypted entity has no other copy: every part must reach the reader.
fn parse_entity(bytes: &[u8]) -> Result<ParsedBody, DecodeError> {
    check_complete_mime(bytes)?;
    Ok(parse_raw_message_body(bytes))
}

/// Keep bytes intact until the MIME parser decodes each part's charset and CTE.
/// Opaque signed-data must be unwrapped separately from signature verification:
/// a damaged signature can still contain readable mail, but never gains trust.
pub fn parse_decrypted_body(bytes: &[u8]) -> Result<DecryptedBody, DecodeError> {
    let mut body = parse_entity(bytes)?;
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
            Ok(content) => body = parse_entity(&content)?,
            // multipart/signed can retain its readable entity even when the
            // signature structure is broken. Verification still receives it.
            Err(_) if body.text.is_some() || body.html.is_some() => {
                return Ok(finish(body, signatures));
            }
            Err(error) => return Err(DecodeError::InvalidSignedContent(error)),
        }
    }
    Err(DecodeError::NestingLimit)
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
