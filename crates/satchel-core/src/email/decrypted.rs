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

/// What the ID of every attachment row for a part of decrypted content, PGP
/// or S/MIME, starts with. The `att-pgp-` form predates S/MIME decoding and
/// stays: rows, cache file names and open attachment views are keyed by it.
const ATTACHMENT_ID_PREFIX: &str = "att-pgp-";

/// The stored ID of a part found inside decrypted content.
pub fn attachment_id(message_id: &str, remote_part_id: &str) -> String {
    format!("{ATTACHMENT_ID_PREFIX}{message_id}-{remote_part_id}")
}

/// Whether an attachment row holds a part of decrypted content. Such a part
/// exists nowhere else: the server copy of the message holds it encrypted, so
/// it can be read again only by decrypting the message again.
pub fn is_attachment_id(id: &str) -> bool {
    id.starts_with(ATTACHMENT_ID_PREFIX)
}

/// A `LIKE` pattern for [`is_attachment_id`].
pub fn attachment_id_pattern() -> String {
    format!("{ATTACHMENT_ID_PREFIX}%")
}

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

#[cfg(test)]
mod tests {
    use super::parse_decrypted_body;
    use crate::email::pgp_mime::parse_signed_message;
    use crate::utils::pgp_engine::{verify_detached, DetachedVerification};

    const GPG_PUBLIC: &str = include_str!("../../tests/corpus/pgp/gpg-public.asc");

    fn verifies(bundle: &[u8]) -> bool {
        let signed = parse_signed_message(bundle).unwrap();
        matches!(
            verify_detached(GPG_PUBLIC, &signed.signed_entity, &signed.armored_signature),
            Ok(DetachedVerification::Valid { .. })
        )
    }

    #[test]
    fn attachment_ids_keep_their_persisted_form() {
        let id = super::attachment_id("msg-1", "mime-2");
        assert_eq!(id, "att-pgp-msg-1-mime-2");
        assert!(super::is_attachment_id(&id));
        assert!(!super::is_attachment_id("att-imap-msg-1-mime-2"));
    }

    /// The verifier splits the signed bundle again with its own parser. What
    /// it checks must be the entity the reader is shown, byte for byte.
    #[test]
    fn a_signature_counts_only_for_the_entity_that_is_shown() {
        // The fixture's own signed entity, as decrypted plaintext, still verifies.
        let mail = include_bytes!("../../tests/corpus/pgp/signed-covered-block.eml");
        let start = memchr::memmem::find(mail, b"Content-Type: multipart/signed").unwrap();
        let decoded = parse_decrypted_body(&mail[start..]).unwrap();
        let [(bundle, true)] = &decoded.pgp_signatures[..] else {
            panic!("signatures {:?}", decoded.pgp_signatures.len());
        };
        assert!(verifies(bundle));

        let decoded = parse_decrypted_body(include_bytes!(
            "../../tests/corpus/decrypted/signed-entity-in-preamble.eml"
        ))
        .unwrap();
        assert_eq!(
            decoded.body.text_parts,
            ["Shown text, not covered by the signature."]
        );
        let [(bundle, true)] = &decoded.pgp_signatures[..] else {
            panic!("signatures {:?}", decoded.pgp_signatures.len());
        };
        let checked = parse_signed_message(bundle).unwrap().signed_entity;
        assert!(
            String::from_utf8_lossy(&checked).contains("Shown text"),
            "verified {:?}",
            String::from_utf8_lossy(&checked)
        );
        assert!(!verifies(bundle), "a signature over another entity");
    }
}
