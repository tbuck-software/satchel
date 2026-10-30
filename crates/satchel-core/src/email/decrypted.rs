//! Decode decrypted MIME without treating extracted content as a valid signature.
//!
//! S/MIME envelopes and OpenPGP plaintexts that declare MIME both end here: a
//! decrypted entity has no second copy, so every part must reach the reader.
use crate::providers::imap::{
    decode_complete_mime, CompleteMime, DetachedSignature, IncompleteMime, ShownLeaf,
};
use crate::providers::traits::RemoteAttachment;

/// What the reader shows of a decrypted entity.
#[derive(Debug, Default)]
pub struct DisplayedBody {
    /// The text of each shown part, in document order. Parts without text,
    /// such as an HTML image, are left out.
    pub text_parts: Vec<String>,
    /// `text_parts` joined by [`PART_SEPARATOR`]; `None` when no part is shown.
    pub text: Option<String>,
    /// Sanitized HTML of every shown part, when any of them is HTML. A text
    /// part keeps its line breaks inside it.
    pub html: Option<String>,
    pub attachments: Vec<RemoteAttachment>,
}

/// Between two shown parts in the text view.
pub const PART_SEPARATOR: &str = "\n\n";

pub struct DecryptedBody {
    pub body: DisplayedBody,
    /// Exact S/MIME signed payloads, each with whether it covers the complete
    /// displayed body.
    pub signatures: Vec<(Vec<u8>, bool)>,
    /// OpenPGP `multipart/signed` bundles, with the same flag.
    pub pgp_signatures: Vec<(Vec<u8>, bool)>,
}

/// Why a decrypted entity cannot be shown. The protocol is the caller's: the
/// app maps each variant to an S/MIME or OpenPGP message key.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DecodeError {
    #[error("decrypted content exceeds the nesting limit")]
    NestingLimit,
    #[error("decrypted content is still encrypted")]
    RemainingEncryption,
    #[error("malformed signed MIME structure in decrypted content")]
    MalformedSigned,
    /// A multipart entity without a boundary or without any part.
    #[error("malformed MIME structure in decrypted content")]
    MalformedMime,
    /// Opaque signed-data below the root, which nothing would unwrap.
    #[error("decrypted content contains a part that cannot be shown")]
    UndecodedPart,
    #[error("unreadable signed content: {0}")]
    InvalidSignedContent(String),
}

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

/// Keep bytes intact until the MIME parser decodes each part's charset and CTE.
/// Opaque signed-data must be unwrapped separately from signature verification:
/// a damaged signature can still contain readable mail, but never gains trust.
pub fn parse_decrypted_body(bytes: &[u8]) -> Result<DecryptedBody, DecodeError> {
    let mut mime = decode_complete_mime(bytes)?;
    let mut signatures = Vec::new();
    let mut pgp_signatures = Vec::new();
    for _ in 0..10 {
        for signature in std::mem::take(&mut mime.root_signatures) {
            match signature {
                DetachedSignature::Smime(bundle) => signatures.push((bundle, true)),
                DetachedSignature::Pgp(bundle) => pgp_signatures.push((bundle, true)),
            }
        }
        let Some(payload) = mime.opaque_signed.take() else {
            // Below the root a signature covers one part, beside unsigned ones.
            for signature in std::mem::take(&mut mime.nested_signatures) {
                match signature {
                    DetachedSignature::Smime(bundle) => signatures.push((bundle, false)),
                    DetachedSignature::Pgp(bundle) => pgp_signatures.push((bundle, false)),
                }
            }
            return Ok(DecryptedBody {
                body: display(mime),
                signatures,
                pgp_signatures,
            });
        };
        let content = crate::utils::smime_engine::signed_content(&payload)
            .map_err(DecodeError::InvalidSignedContent);
        signatures.push((payload, true));
        mime = decode_complete_mime(&content?)?;
    }
    Err(DecodeError::NestingLimit)
}

fn display(mime: CompleteMime) -> DisplayedBody {
    // An HTML-only part is in both views; sanitize it once.
    let mut sanitized = std::collections::HashMap::<String, String>::new();
    let mut sanitize = |html: &str| -> String {
        sanitized
            .entry(html.to_string())
            .or_insert_with(|| crate::email::sanitizer::sanitize_email_html(html))
            .clone()
    };
    let text_parts = mime
        .text
        .iter()
        .map(|leaf| match leaf {
            ShownLeaf::Text(text) => text.clone(),
            ShownLeaf::Html(html) => crate::utils::mime::decode_html_entities(
                &crate::email::text::strip_html_tags(&sanitize(html)),
            ),
        })
        .filter(|text| !text.trim().is_empty())
        .collect::<Vec<_>>();
    let html = mime
        .html
        .iter()
        .any(|leaf| matches!(leaf, ShownLeaf::Html(_)))
        .then(|| {
            mime.html
                .iter()
                .map(|leaf| match leaf {
                    ShownLeaf::Html(html) => sanitize(html),
                    ShownLeaf::Text(text) => format!(
                        "<div style=\"white-space: pre-wrap\">{}</div>",
                        escape_html_text(text)
                    ),
                })
                .collect::<String>()
        });
    DisplayedBody {
        text: (!mime.text.is_empty()).then(|| text_parts.join(PART_SEPARATOR)),
        text_parts,
        html,
        attachments: mime.attachments,
    }
}

fn escape_html_text(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
