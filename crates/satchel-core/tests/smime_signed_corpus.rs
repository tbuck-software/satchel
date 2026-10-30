//! S/MIME `multipart/signed` mail through the parser incoming mail takes and
//! the S/MIME engine. A signature only counts for the bytes that are displayed:
//! the legitimate fixture must verify over exactly its displayed entity, the
//! forged ones never over what they display.

use satchel_core::email::signed_mime::{
    display_entity, stored_signed_body, EntityDisplay, StoredSignedBody,
};
use satchel_core::providers::imap::{parse_header_value, parse_raw_message_body, ParsedBody};
use satchel_core::providers::traits::MessageTrust;
use satchel_core::utils::smime_engine;

const SIGNED: &[u8] = include_bytes!("corpus/smime-signed/detached-signed.eml");
const SIGNED_HTML: &[u8] = include_bytes!("corpus/smime-signed/detached-signed-html.eml");
const OPAQUE: &[u8] = include_bytes!("corpus/smime-signed/opaque-signed.eml");
const FORGED: &[(&str, &[u8])] = &[
    (
        "forged-quoted-boundary",
        include_bytes!("corpus/smime-signed/forged-quoted-boundary.eml"),
    ),
    (
        "forged-preamble-delimiter",
        include_bytes!("corpus/smime-signed/forged-preamble-delimiter.eml"),
    ),
    (
        "forged-padded-boundary",
        include_bytes!("corpus/smime-signed/forged-padded-boundary.eml"),
    ),
    (
        "forged-bare-cr",
        include_bytes!("corpus/smime-signed/forged-bare-cr.eml"),
    ),
    (
        "forged-missing-protocol",
        include_bytes!("corpus/smime-signed/forged-missing-protocol.eml"),
    ),
    (
        "forged-html-link-swapped",
        include_bytes!("corpus/smime-signed/forged-html-link-swapped.eml"),
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
fn detached_smime_html_verifies_over_the_displayed_entity() {
    let parsed = parse_raw_message_body(SIGNED_HTML);
    assert!(parsed.trust.root_crypto && parsed.trust.signed_binding);
    assert!(parsed
        .html
        .as_deref()
        .is_some_and(|html| html.contains("https://bank.example/invoice/42")));
    let bundle = parsed.smime_signed.as_deref().unwrap();
    assert!(signature_holds(bundle));
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
        // Neither does a signature found further down: it is no root's.
        if parsed.smime_signed.is_none() {
            assert!(!parsed.trust.root_crypto, "{name}");
        }
    }
}

#[test]
fn a_stored_row_shows_exactly_the_entity_its_bundle_signs() {
    for raw in [SIGNED, SIGNED_HTML] {
        let parsed = parse_raw_message_body(raw);
        let bundle = parsed.smime_signed.as_deref().unwrap();
        let entity = StoredSignedBody::Entity(EntityDisplay {
            text: parsed.text.clone(),
            html: parsed.html.clone(),
        });
        assert_eq!(
            stored_signed_body(bundle, &parsed.trust, true),
            Ok(StoredSignedBody::Stored)
        );
        assert_eq!(
            stored_signed_body(bundle, &parsed.trust, false),
            Ok(entity.clone())
        );
        let legacy = MessageTrust {
            signed_binding: false,
            ..parsed.trust.clone()
        };
        assert_eq!(stored_signed_body(bundle, &legacy, true), Ok(entity));
    }

    // A row an older parser stored for the forged root: shown from its
    // bundle, the genuine HTML with the genuine link.
    let (_, forged) = FORGED
        .iter()
        .find(|(name, _)| *name == "forged-html-link-swapped")
        .unwrap();
    let forged = std::str::from_utf8(forged).unwrap();
    let nested = forged
        .find("--OUT\r\nContent-Type: multipart/signed")
        .unwrap()
        + 7;
    let bundle = &forged.as_bytes()[nested..forged.rfind("\r\n--OUT--").unwrap()];
    let trust: MessageTrust = serde_json::from_str(r#"{"root_crypto":true}"#).unwrap();
    let Ok(StoredSignedBody::Entity(shown)) = stored_signed_body(bundle, &trust, true) else {
        panic!("legacy row not shown from its bundle");
    };
    let html = shown.html.unwrap_or_default();
    assert!(html.contains("https://bank.example/invoice/42"), "{html}");
    assert!(!html.contains("evil.example"));
}

#[test]
fn opaque_smime_shows_the_content_the_signature_holds() {
    let parsed = parse_raw_message_body(OPAQUE);
    assert!(parsed.trust.root_crypto);
    let bundle = parsed.smime_signed.as_deref().unwrap();
    assert_eq!(
        stored_signed_body(bundle, &parsed.trust, false),
        Ok(StoredSignedBody::Encapsulated)
    );
    let result = smime_engine::verify(bundle).unwrap();
    assert!(result.valid, "{:?}", result.error);
    let content = result.content.expect("verified content");
    assert_eq!(
        display_entity(&content).text.as_deref(),
        Some("The opaque contract text is signed as a whole.")
    );

    // A changed byte inside the content is no longer what was signed.
    let tampered = bundle
        .windows(6)
        .position(|window| window == b"opaque")
        .map(|at| {
            let mut copy = bundle.to_vec();
            copy[at] = b'O';
            copy
        })
        .unwrap();
    assert!(!smime_engine::verify(&tampered).unwrap().valid);
}

/// Opaque signed content that reads like a detached bundle is still opaque:
/// what it shows comes from the engine, never from the DER bytes around it.
#[test]
fn opaque_content_that_reads_like_a_detached_bundle_stays_encapsulated() {
    let keys = smime_engine::generate_keypair(&smime_engine::GenerateSmimeRequest {
        name: "Alice",
        email: "alice@example.test",
        passphrase: None,
    })
    .unwrap();
    let content = "Content-Type: multipart/signed; protocol=\"application/pkcs7-signature\"; boundary=\"I\"\r\n\r\n\
        --I\r\nContent-Type: text/plain\r\n\r\nInner text\r\n\
        --I\r\nContent-Type: application/pkcs7-signature\r\n\r\nAAAA\r\n--I--\r\n";
    let signed = smime_engine::sign(
        &keys.pem_private_key,
        &keys.pem_certificate,
        None,
        content.as_bytes(),
    )
    .unwrap();
    let message = [
        b"From: alice@example.test\r\n".as_slice(),
        signed.as_slice(),
    ]
    .concat();
    let parsed = parse_raw_message_body(&message);
    let bundle = parsed.smime_signed.as_deref().expect("opaque bundle");
    assert!(parsed.trust.root_crypto && !parsed.trust.signed_binding);
    assert!(!bundle.starts_with(b"Content-Type:"));
    for has_stored_body in [false, true] {
        assert_eq!(
            stored_signed_body(bundle, &parsed.trust, has_stored_body),
            Ok(StoredSignedBody::Encapsulated)
        );
    }
}
