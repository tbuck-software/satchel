//! S/MIME `multipart/signed` mail through the parser incoming mail takes and
//! the S/MIME engine. A signature only counts for the bytes that are displayed:
//! the legitimate fixture must verify over exactly its displayed entity, the
//! forged ones never over what they display.

use satchel_core::providers::imap::{parse_header_value, parse_raw_message_body, ParsedBody};
use satchel_core::utils::smime_engine;

const SIGNED: &[u8] = include_bytes!("corpus/smime-signed/detached-signed.eml");
const FORGED: &[(&str, &[u8])] = &[
    (
        "forged-quoted-boundary",
        include_bytes!("corpus/smime-signed/forged-quoted-boundary.eml"),
    ),
    (
        "forged-preamble-delimiter",
        include_bytes!("corpus/smime-signed/forged-preamble-delimiter.eml"),
    ),
];

/// Whether the engine accepts the signature maths over the bundle. Chain trust
/// is a separate question; the fixture's certificate is self-signed.
fn signature_holds(bundle: &[u8]) -> bool {
    smime_engine::verify(bundle).is_ok_and(|result| result.valid)
}

fn shown(parsed: &ParsedBody) -> (Option<&str>, Option<&str>) {
    (parsed.text.as_deref(), parsed.html.as_deref())
}

#[test]
fn detached_smime_signature_verifies_over_the_displayed_entity() {
    let parsed = parse_raw_message_body(SIGNED);
    assert!(parsed.trust.root_crypto);
    assert_eq!(
        parsed.text.as_deref(),
        Some("The contract is ready for signature on Monday.")
    );
    let bundle = parsed.smime_signed.as_deref().expect("signed bundle kept");
    let result = smime_engine::verify(bundle).expect("verifiable bundle");
    assert!(result.valid, "{:?}", result.error);
    assert_eq!(
        result
            .signer
            .and_then(|signer| signer.subject_email)
            .as_deref(),
        Some("smime-fixture@example.test")
    );
}

#[test]
fn forged_smime_mail_never_verifies_over_what_it_displays() {
    for (name, raw) in FORGED {
        let parsed = parse_raw_message_body(raw);
        if let Some(bundle) = parsed.smime_signed.as_deref() {
            assert!(
                !signature_holds(bundle),
                "{name}: a valid S/MIME signature is shown over {:?}",
                shown(&parsed)
            );
        }
        // The same bytes as a bundle stored before the display path refused
        // them must not verify either.
        let (header, body) = std::str::from_utf8(raw)
            .unwrap()
            .split_once("\r\n\r\n")
            .unwrap();
        let content_type = parse_header_value(header, "Content-Type").unwrap();
        let stored = format!("Content-Type: {content_type}\r\n\r\n{body}");
        assert!(!signature_holds(stored.as_bytes()), "{name}: stored bundle");
    }
}
