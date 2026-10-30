//! PGP/MIME signed mail from real client shapes, run through the same parser
//! incoming mail takes and verified against the GnuPG fixture key.
//!
//! The invariant under test: a signature only counts for the bytes that are
//! displayed. Every fixture that verifies must display exactly its verified
//! entity; the forged fixtures must never verify over what they display.

use satchel_core::email::pgp_mime::parse_signed_message;
use satchel_core::email::sanitizer::sanitize_email_html;
use satchel_core::email::signed_mime::{stored_body_matches, SignedMimeError};
use satchel_core::providers::imap::{parse_raw_message_body, ParsedBody};
use satchel_core::utils::pgp_engine::{self, DetachedVerification};

const GPG_FIXTURE_PUBLIC: &str = include_str!("corpus/pgp/gpg-public.asc");
const GPG_FIXTURE_FINGERPRINT: &str = "C29160B0BADC8557E5FEF30C5A4E7D912785BF5C";

struct Legitimate {
    name: &'static str,
    raw: &'static [u8],
    shows: &'static str,
}

const LEGITIMATE: &[Legitimate] = &[
    Legitimate {
        name: "thunderbird-mixed-attachment",
        raw: include_bytes!("corpus/pgp-mime/thunderbird-mixed-attachment.eml"),
        shows: "The quarterly figures are attached. Grüße.",
    },
    Legitimate {
        name: "apple-mail-gpgtools",
        raw: include_bytes!("corpus/pgp-mime/apple-mail-gpgtools.eml"),
        shows: "The train leaves at nine.",
    },
    Legitimate {
        name: "mutt-lf",
        raw: include_bytes!("corpus/pgp-mime/mutt-lf.eml"),
        shows: "The fix is on the branch.",
    },
    Legitimate {
        name: "mutt-crlf",
        raw: include_bytes!("corpus/pgp-mime/mutt-crlf.eml"),
        shows: "The fix is on the branch.",
    },
    Legitimate {
        name: "preamble-and-epilogue",
        raw: include_bytes!("corpus/pgp-mime/preamble-and-epilogue.eml"),
        shows: "Notes from today: we ship on Friday. Grüße.",
    },
    Legitimate {
        name: "nested-prefix-boundary",
        raw: include_bytes!("corpus/pgp-mime/nested-prefix-boundary.eml"),
        shows: "Plain rendering of the signed note.",
    },
];

/// How a forged message must end up.
#[derive(Debug, PartialEq)]
enum Outcome {
    /// Shown, but the signature does not hold over what is shown.
    Invalid,
    /// Refused as signed mail: no bundle, no signature state.
    NotSigned,
}

struct Forged {
    name: &'static str,
    raw: &'static [u8],
    outcome: Outcome,
}

const FORGED: &[Forged] = &[
    Forged {
        name: "forged-preamble-delimiter",
        raw: include_bytes!("corpus/pgp-mime/forged-preamble-delimiter.eml"),
        outcome: Outcome::Invalid,
    },
    Forged {
        name: "forged-quoted-boundary",
        raw: include_bytes!("corpus/pgp-mime/forged-quoted-boundary.eml"),
        outcome: Outcome::NotSigned,
    },
];

/// What the reader sees, reduced to what a signature has to cover.
fn shown(parsed: &ParsedBody) -> (Option<&str>, Option<&str>) {
    (parsed.text.as_deref(), parsed.html.as_deref())
}

/// The verified entity and the verdict over it, or `None` when the bundle is
/// rejected before any key is consulted.
fn verify(bundle: &[u8]) -> Option<(Vec<u8>, DetachedVerification)> {
    let signed = parse_signed_message(bundle).ok()?;
    let verdict = pgp_engine::verify_detached(
        GPG_FIXTURE_PUBLIC,
        &signed.signed_entity,
        &signed.armored_signature,
    )
    .expect("fixture signature parses");
    Some((signed.signed_entity, verdict))
}

#[test]
fn client_shaped_signed_mail_verifies_over_exactly_what_is_displayed() {
    for fixture in LEGITIMATE {
        let parsed = parse_raw_message_body(fixture.raw);
        assert!(parsed.trust.root_crypto, "{}: root signature", fixture.name);
        let text = parsed.text.as_deref().unwrap_or_default();
        assert!(
            text.contains(fixture.shows),
            "{}: displayed {text:?}",
            fixture.name
        );

        let bundle = parsed.pgp_signed.as_deref().expect("signed bundle kept");
        let (entity, verdict) = verify(bundle)
            .unwrap_or_else(|| panic!("{}: well-formed signed message rejected", fixture.name));
        assert_eq!(
            verdict,
            DetachedVerification::Valid {
                signer_fingerprint: GPG_FIXTURE_FINGERPRINT.to_string()
            },
            "{}",
            fixture.name
        );
        let verified = parse_raw_message_body(&entity);
        assert_eq!(
            shown(&parsed),
            shown(&verified),
            "{}: displayed content is not the verified entity",
            fixture.name
        );
    }
}

#[test]
fn signed_attachments_stay_and_the_signature_part_is_not_one() {
    let parsed = parse_raw_message_body(LEGITIMATE[0].raw);
    let names: Vec<_> = parsed
        .attachments
        .iter()
        .map(|attachment| attachment.filename.as_deref().unwrap_or_default())
        .collect();
    assert_eq!(names, ["figures.csv"]);
}

#[test]
fn forged_signed_mail_never_verifies_over_what_it_displays() {
    for forged in FORGED {
        let parsed = parse_raw_message_body(forged.raw);
        let verdict = parsed.pgp_signed.as_deref().and_then(verify);
        if let Some((entity, DetachedVerification::Valid { .. })) = &verdict {
            assert_eq!(
                shown(&parsed),
                shown(&parse_raw_message_body(entity)),
                "{}: a valid signature is shown over text it does not cover",
                forged.name
            );
        }

        let outcome = match verdict {
            Some((_, DetachedVerification::Invalid)) => Outcome::Invalid,
            None if parsed.pgp_signed.is_none() => Outcome::NotSigned,
            other => panic!("{}: unexpected verdict {other:?}", forged.name),
        };
        assert_eq!(outcome, forged.outcome, "{}", forged.name);
        if outcome == Outcome::NotSigned {
            assert!(!parsed.trust.root_crypto, "{}", forged.name);
        } else {
            assert!(
                parsed
                    .text
                    .as_deref()
                    .is_some_and(|text| text.starts_with("Forged text")),
                "{}",
                forged.name
            );
        }
    }
}

#[test]
fn a_stored_row_keeps_its_signature_only_over_the_entity_it_shows() {
    for fixture in LEGITIMATE {
        let parsed = parse_raw_message_body(fixture.raw);
        let bundle = parsed.pgp_signed.as_deref().unwrap();
        // As ingest stores the row: the parsed text and the sanitized HTML.
        let html = parsed.html.as_deref().map(sanitize_email_html);
        assert_eq!(
            stored_body_matches(bundle, parsed.text.as_deref(), html.as_deref()),
            Ok(()),
            "{}",
            fixture.name
        );
        // Line endings and wrapping are not content.
        let reflowed = parsed
            .text
            .as_deref()
            .map(|text| text.replace('\n', "\r\n  "));
        assert_eq!(
            stored_body_matches(bundle, reflowed.as_deref(), html.as_deref()),
            Ok(()),
            "{}",
            fixture.name
        );
        // A row an older parser filled from another part keeps no verdict.
        assert_eq!(
            stored_body_matches(bundle, Some("Forged text"), html.as_deref()),
            Err(SignedMimeError::StoredBodyMismatch),
            "{}",
            fixture.name
        );
    }

    let nested = LEGITIMATE
        .iter()
        .find(|fixture| fixture.name == "nested-prefix-boundary")
        .unwrap();
    let parsed = parse_raw_message_body(nested.raw);
    assert_eq!(
        stored_body_matches(
            parsed.pgp_signed.as_deref().unwrap(),
            parsed.text.as_deref(),
            Some(&sanitize_email_html("<p>Forged text</p>"))
        ),
        Err(SignedMimeError::StoredBodyMismatch)
    );
}
