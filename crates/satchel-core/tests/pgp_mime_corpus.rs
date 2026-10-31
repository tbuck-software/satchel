//! PGP/MIME signed mail from real client shapes, run through the same parser
//! incoming mail takes and verified against the GnuPG fixture key.
//!
//! The invariant under test: a signature only counts for the bytes that are
//! displayed. Every fixture that verifies must display exactly its verified
//! entity; the forged fixtures must never verify over what they display.

use satchel_core::email::pgp_mime::parse_signed_message;
use satchel_core::email::signed_mime::{
    is_signature_media_type, stored_signed_body, EntityDisplay, StoredSignedBody,
};
use satchel_core::providers::imap::{parse_raw_message_body, ParsedBody};
use satchel_core::providers::traits::MessageTrust;
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
    Forged {
        name: "forged-unrecognised-protocol",
        raw: include_bytes!("corpus/pgp-mime/forged-unrecognised-protocol.eml"),
        outcome: Outcome::NotSigned,
    },
    Forged {
        name: "forged-missing-protocol",
        raw: include_bytes!("corpus/pgp-mime/forged-missing-protocol.eml"),
        outcome: Outcome::NotSigned,
    },
    Forged {
        name: "forged-duplicate-signature-type",
        raw: include_bytes!("corpus/pgp-mime/forged-duplicate-signature-type.eml"),
        outcome: Outcome::NotSigned,
    },
];

const SIGNATURE_PART_ATTACHMENT: &[u8] =
    include_bytes!("corpus/pgp-mime/forged-signature-part-attachment.eml");

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
fn the_signature_part_is_never_an_attachment_even_with_a_filename() {
    let parsed = parse_raw_message_body(SIGNATURE_PART_ATTACHMENT);
    assert!(parsed.trust.root_crypto);
    let (_, verdict) = verify(parsed.pgp_signed.as_deref().unwrap()).unwrap();
    assert!(matches!(verdict, DetachedVerification::Valid { .. }));
    // Its headers are not signed, so nothing they name is offered.
    for attachment in &parsed.attachments {
        assert!(
            !is_signature_media_type(&attachment.content_type)
                && attachment.filename.as_deref() != Some("Invoice.pdf"),
            "{attachment:?}"
        );
    }
}

#[test]
fn a_signed_part_inside_another_container_never_signs_the_root() {
    let genuine = LEGITIMATE
        .iter()
        .find(|fixture| fixture.name == "mutt-crlf")
        .unwrap();
    let text = std::str::from_utf8(genuine.raw).unwrap();
    let signed = &text[text.find("Content-Type: multipart/signed").unwrap()..];
    let mixed = format!(
        "Content-Type: multipart/mixed; boundary=\"OUT\"\r\n\r\n--OUT\r\n\
         Content-Type: text/plain\r\n\r\nForged text.\r\n--OUT\r\n{signed}\r\n--OUT--\r\n"
    );
    let parsed = parse_raw_message_body(mixed.as_bytes());
    // The nested signature stays, for a partial state; the root is not signed.
    assert!(parsed.pgp_signed.is_some());
    assert!(!parsed.trust.root_crypto);
    assert!(!parsed.trust.signed_binding);
}

/// The display a legacy row's stored bundle signs, next to the body an older
/// parser stored for the root: the forged first part.
fn legacy_row_of(forged: &[u8]) -> (Vec<u8>, MessageTrust) {
    let text = std::str::from_utf8(forged).unwrap();
    let nested = text
        .find("--OUT\r\nContent-Type: multipart/signed")
        .unwrap()
        + 7;
    let end = text.rfind("\r\n--OUT--").unwrap();
    // Older rows carry no binding marker.
    let trust = serde_json::from_str(r#"{"root_crypto":true}"#).unwrap();
    (text.as_bytes()[nested..end].to_vec(), trust)
}

#[test]
fn a_stored_row_shows_exactly_the_entity_its_bundle_signs() {
    for fixture in LEGITIMATE {
        let parsed = parse_raw_message_body(fixture.raw);
        assert!(parsed.trust.signed_binding, "{}", fixture.name);
        let bundle = parsed.pgp_signed.as_deref().unwrap();
        let entity = StoredSignedBody::Entity(EntityDisplay {
            text: parsed.text.clone(),
            html: parsed.html.clone(),
        });
        // Written with its bundle by the strict splitter: the body stands.
        assert_eq!(
            stored_signed_body(bundle, &parsed.trust, true),
            Ok(StoredSignedBody::Stored),
            "{}",
            fixture.name
        );
        // Evicted: shown from the bundle, as ingest displayed it.
        assert_eq!(
            stored_signed_body(bundle, &parsed.trust, false),
            Ok(entity.clone()),
            "{}",
            fixture.name
        );
        // Stored before the marker existed: never the stored body.
        let legacy = MessageTrust {
            signed_binding: false,
            ..parsed.trust.clone()
        };
        assert_eq!(
            stored_signed_body(bundle, &legacy, true),
            Ok(entity),
            "{}",
            fixture.name
        );
    }

    for forged in ["forged-missing-protocol", "forged-unrecognised-protocol"] {
        let raw = FORGED
            .iter()
            .find(|fixture| fixture.name == forged)
            .unwrap();
        let (bundle, trust) = legacy_row_of(raw.raw);
        let Ok(StoredSignedBody::Entity(shown)) = stored_signed_body(&bundle, &trust, true) else {
            panic!("{forged}: legacy row not shown from its bundle");
        };
        let text = shown.text.unwrap_or_default();
        assert!(
            text.starts_with("The fix is on the branch."),
            "{forged}: {text:?}"
        );
        assert!(!text.contains("Forged"), "{forged}");
    }
}

/// The same messages as the plaintext of an encrypted one, which has no
/// second copy. The decrypted decoder takes the entity from the splitter the
/// display path uses: what it shows is what the bundle it hands the verifier
/// yields, and its files are the signed entity's, bytes included.
#[test]
fn decrypted_signed_content_verifies_over_exactly_what_is_displayed() {
    use satchel_core::email::decrypted::{parse_decrypted_body, DecodeError};
    use satchel_core::providers::imap::signed_entity_attachments;
    // Files by name. The attachment scan of synced mail also lists a body
    // part labelled `inline` (Mutt's text) without a name; the decrypted
    // decoder shows that part as text instead.
    let files = |attachments: &[satchel_core::providers::traits::RemoteAttachment]| {
        attachments
            .iter()
            .filter(|attachment| attachment.filename.is_some())
            .map(|attachment| {
                (
                    attachment.filename.clone(),
                    attachment.content_type.clone(),
                    attachment.content_base64.clone(),
                )
            })
            .collect::<Vec<_>>()
    };
    for fixture in LEGITIMATE {
        let ordinary = parse_raw_message_body(fixture.raw);
        let stored = ordinary.pgp_signed.as_deref().unwrap();
        let (entity, _) = verify(stored).unwrap();

        let decrypted = parse_decrypted_body(fixture.raw)
            .unwrap_or_else(|error| panic!("{}: {error}", fixture.name));
        assert!(decrypted.signatures.is_empty(), "{}", fixture.name);
        let [(bundle, true)] = &decrypted.pgp_signatures[..] else {
            panic!(
                "{}: {} OpenPGP signatures",
                fixture.name,
                decrypted.pgp_signatures.len()
            );
        };
        let (verified, verdict) =
            verify(bundle).unwrap_or_else(|| panic!("{}: rebuilt bundle rejected", fixture.name));
        assert_eq!(
            verified, entity,
            "{}: the verifier reads another entity",
            fixture.name
        );
        assert_eq!(
            verdict,
            DetachedVerification::Valid {
                signer_fingerprint: GPG_FIXTURE_FINGERPRINT.to_string()
            },
            "{}",
            fixture.name
        );
        let text = decrypted.body.text.as_deref().unwrap_or_default();
        assert!(
            text.contains(fixture.shows),
            "{}: displayed {text:?}",
            fixture.name
        );
        assert_eq!(
            files(&decrypted.body.attachments),
            files(&signed_entity_attachments(stored).unwrap()),
            "{}: files outside the signed entity",
            fixture.name
        );
    }

    for forged in FORGED {
        match parse_decrypted_body(forged.raw) {
            Err(error) => assert_eq!(error, DecodeError::MalformedSigned, "{}", forged.name),
            Ok(decrypted) => {
                for (bundle, _) in &decrypted.pgp_signatures {
                    assert!(
                        !verify(bundle).is_some_and(|(_, verdict)| matches!(
                            verdict,
                            DetachedVerification::Valid { .. }
                        )),
                        "{}: decrypted forged content verifies",
                        forged.name
                    );
                }
            }
        }
    }
}
