//! PGP/MIME wrappers per RFC 3156.
//!
//! Deliberately not inline ("`-----BEGIN PGP MESSAGE-----` pasted into the
//! body"): inline PGP loses attachments, mangles encodings, and is displayed as
//! noise by clients that do not understand it. RFC 3156 keeps the signed entity
//! a normal MIME part, so a signed mail stays readable everywhere.

use super::signed_mime::{self, SignedMimeError, SignedProtocol};

const MAX_SIGNED_BUNDLE_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedSignedMessage {
    pub signed_entity: Vec<u8>,
    pub armored_signature: String,
}

/// Split a complete RFC822 message into delivery headers and the MIME entity
/// (content headers + body) that PGP operates on.
///
/// Signing must cover the content headers — a signature over the body alone
/// would let an attacker change the Content-Type without invalidating it.
fn split_message(rfc822: &str) -> (Vec<String>, String) {
    let (header_block, body) = rfc822
        .split_once("\r\n\r\n")
        .or_else(|| rfc822.split_once("\n\n"))
        .unwrap_or((rfc822, ""));

    let (content_headers, delivery_headers): (Vec<String>, Vec<String>) =
        super::build::split_logical_headers(header_block)
            .into_iter()
            .partition(|header| super::build::is_content_header(header));

    let mut entity = content_headers
        .into_iter()
        // MIME-Version belongs on the outermost message, not on the inner part.
        .filter(|h| !h.to_ascii_lowercase().starts_with("mime-version:"))
        .collect::<Vec<_>>()
        .join("\r\n");
    if entity.is_empty() {
        entity.push_str("Content-Type: text/plain; charset=utf-8");
    }
    entity.push_str("\r\n\r\n");
    entity.push_str(&normalize_crlf(body));

    (delivery_headers, entity)
}

/// RFC 3156 §5: the signed data must use canonical CRLF line endings, or
/// verification fails on the receiving side.
fn normalize_crlf(text: &str) -> String {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\n', "\r\n")
}

fn boundary(tag: &str) -> String {
    format!("=_satchel_{tag}_{}", uuid::Uuid::new_v4().simple())
}

/// The exact bytes a detached signature must be computed over.
pub fn signable_entity(rfc822: &str) -> String {
    split_message(rfc822).1
}

/// Wrap a message and its detached signature as `multipart/signed`.
pub fn build_signed(rfc822: &str, armored_signature: &str) -> String {
    let (delivery_headers, entity) = split_message(rfc822);
    let boundary = boundary("sig");

    let mut out = delivery_headers.join("\r\n");
    out.push_str("\r\nMIME-Version: 1.0\r\n");
    out.push_str(&format!(
        "Content-Type: multipart/signed; micalg=pgp-sha256; \
         protocol=\"application/pgp-signature\"; boundary=\"{boundary}\"\r\n"
    ));
    out.push_str("\r\nThis is an OpenPGP/MIME signed message (RFC 3156).\r\n");

    out.push_str(&format!("--{boundary}\r\n"));
    out.push_str(&entity);

    out.push_str(&format!("\r\n--{boundary}\r\n"));
    out.push_str("Content-Type: application/pgp-signature; name=\"signature.asc\"\r\n");
    out.push_str("Content-Description: OpenPGP digital signature\r\n");
    out.push_str("Content-Disposition: attachment; filename=\"signature.asc\"\r\n\r\n");
    out.push_str(&normalize_crlf(armored_signature.trim()));
    out.push_str(&format!("\r\n--{boundary}--\r\n"));
    out
}

/// Wrap ciphertext as `multipart/encrypted`.
pub fn build_encrypted(rfc822: &str, armored_ciphertext: &str) -> String {
    let (delivery_headers, _) = split_message(rfc822);
    let boundary = boundary("enc");

    let mut out = delivery_headers.join("\r\n");
    out.push_str("\r\nMIME-Version: 1.0\r\n");
    out.push_str(&format!(
        "Content-Type: multipart/encrypted; \
         protocol=\"application/pgp-encrypted\"; boundary=\"{boundary}\"\r\n"
    ));
    out.push_str("\r\nThis is an OpenPGP/MIME encrypted message (RFC 3156).\r\n");

    out.push_str(&format!("--{boundary}\r\n"));
    out.push_str("Content-Type: application/pgp-encrypted\r\n");
    out.push_str("Content-Description: PGP/MIME version identification\r\n\r\n");
    out.push_str("Version: 1\r\n");

    out.push_str(&format!("\r\n--{boundary}\r\n"));
    out.push_str("Content-Type: application/octet-stream; name=\"encrypted.asc\"\r\n");
    out.push_str("Content-Description: OpenPGP encrypted message\r\n");
    out.push_str("Content-Disposition: inline; filename=\"encrypted.asc\"\r\n\r\n");
    out.push_str(&normalize_crlf(armored_ciphertext.trim()));
    out.push_str(&format!("\r\n--{boundary}--\r\n"));
    out
}

/// The full message to encrypt when signing and encrypting together.
///
/// RFC 3156 §6.2 nests sign-then-encrypt: the signed multipart becomes the
/// plaintext of the encrypted one, so the signature stays hidden from anyone
/// who cannot decrypt.
pub fn signed_entity_for_encryption(rfc822: &str, armored_signature: &str) -> String {
    let signed = build_signed(rfc822, armored_signature);
    // Strip delivery headers: only the MIME part is encrypted.
    let (_, entity) = split_message(&signed);
    entity
}

/// Recover the signed entity and armored detached signature from an RFC 3156
/// `multipart/signed` bundle.
///
/// The entity comes from the same splitter the display path uses, so it is
/// byte for byte the entity the reader is shown. It is not normalized: even a
/// harmless line-ending rewrite would invalidate a binary signature.
pub fn parse_signed_message(bundle: &[u8]) -> Result<ParsedSignedMessage, SignedMimeError> {
    if bundle.len() > MAX_SIGNED_BUNDLE_BYTES {
        return Err(SignedMimeError::TooLarge);
    }
    let parts = signed_mime::split_bundle(bundle)?;
    if parts.protocol != SignedProtocol::OpenPgp {
        return Err(SignedMimeError::UnsupportedProtocol);
    }

    const BEGIN: &[u8] = b"-----BEGIN PGP SIGNATURE-----";
    const END: &[u8] = b"-----END PGP SIGNATURE-----";
    // Only the signature part's own body: armor further down the message is
    // not this part's signature.
    let (_, signature_body) = signed_mime::entity_header_and_body(parts.signature);
    let armor_start =
        memchr::memmem::find(signature_body, BEGIN).ok_or(SignedMimeError::MissingArmor)?;
    let armor_end = memchr::memmem::find(&signature_body[armor_start..], END)
        .map(|offset| armor_start + offset + END.len())
        .ok_or(SignedMimeError::MissingArmor)?;
    let armored_signature = String::from_utf8(signature_body[armor_start..armor_end].to_vec())
        .map_err(|_| SignedMimeError::MissingArmor)?;

    Ok(ParsedSignedMessage {
        signed_entity: parts.entity.to_vec(),
        armored_signature,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MSG: &str = "From: me@example.com\r\nTo: you@example.com\r\nSubject: Test\r\nMIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nHallo Welt\r\n";

    #[test]
    fn captured_valid_pgp_entities_are_partial_or_opaque_when_embedded() {
        use crate::utils::pgp_engine;
        let key = pgp_engine::generate_keypair(&pgp_engine::GenerateKeyRequest {
            uid: "Bob <bob@bank.example>",
            algorithm: "default",
            expires_in: "0",
            passphrase: None,
        })
        .unwrap();
        let signature =
            pgp_engine::sign_detached(&key.private_armored, "", signable_entity(MSG).as_bytes())
                .unwrap();
        let signed = build_signed(MSG, &signature);
        let ciphertext = pgp_engine::encrypt_to(
            std::slice::from_ref(&key.public_armored),
            b"Secret captured message",
        )
        .unwrap();
        let encrypted = build_encrypted(MSG, &ciphertext);
        for (entity, is_signed) in [(&signed, true), (&encrypted, false)] {
            let attack = format!("From: Bob <bob@bank.example>\r\nContent-Type: multipart/mixed; boundary=attack\r\n\r\n--attack\r\nContent-Type: text/html\r\n\r\n<p>Please wire EUR 10000</p>\r\n--attack\r\n{entity}\r\n--attack--\r\n");
            let parsed = crate::providers::imap::parse_raw_message_body(attack.as_bytes());
            assert!(!parsed.trust.root_crypto);
            assert!(!parsed.trust.root_encryption);
            assert!(parsed.html.as_deref().unwrap().contains("wire"));
            if is_signed {
                let signed = parse_signed_message(parsed.pgp_signed.as_deref().unwrap()).unwrap();
                assert!(matches!(
                    pgp_engine::verify_detached(
                        &key.public_armored,
                        &signed.signed_entity,
                        &signed.armored_signature
                    )
                    .unwrap(),
                    pgp_engine::DetachedVerification::Valid { .. }
                ));
            }
        }
    }

    #[test]
    fn signable_entity_keeps_content_headers_but_drops_delivery_headers() {
        let entity = signable_entity(MSG);
        assert!(entity.starts_with("Content-Type: text/plain"));
        assert!(entity.contains("Hallo Welt"));
        // A signature over the body alone would not protect the Content-Type.
        assert!(!entity.contains("Subject:"));
        assert!(!entity.contains("From:"));
        assert!(!entity.contains("MIME-Version:"));
    }

    #[test]
    fn signed_message_is_a_conformant_multipart_signed() {
        let out = build_signed(
            MSG,
            "-----BEGIN PGP SIGNATURE-----\nabc\n-----END PGP SIGNATURE-----",
        );

        assert!(out.contains("Content-Type: multipart/signed"));
        assert!(out.contains("protocol=\"application/pgp-signature\""));
        assert!(out.contains("micalg=pgp-sha256"));
        assert!(out.contains("Content-Type: application/pgp-signature"));
        // Delivery headers survive so the message can actually be routed.
        assert!(out.contains("From: me@example.com"));
        assert!(out.contains("Subject: Test"));
        // The signed body stays readable to clients without PGP.
        assert!(out.contains("Hallo Welt"));
    }

    #[test]
    fn encrypted_message_hides_the_body_but_keeps_routing_headers() {
        let out = build_encrypted(
            MSG,
            "-----BEGIN PGP MESSAGE-----\nxyz\n-----END PGP MESSAGE-----",
        );

        assert!(out.contains("Content-Type: multipart/encrypted"));
        assert!(out.contains("protocol=\"application/pgp-encrypted\""));
        assert!(out.contains("Version: 1"));
        assert!(out.contains("-----BEGIN PGP MESSAGE-----"));
        assert!(out.contains("To: you@example.com"));
        // The plaintext must not survive anywhere in the output.
        assert!(!out.contains("Hallo Welt"));
    }

    #[test]
    fn boundaries_are_unique_per_message() {
        let a = build_signed(MSG, "sig");
        let b = build_signed(MSG, "sig");
        assert_ne!(a, b, "each message needs its own boundary");
    }

    #[test]
    fn signed_data_uses_canonical_crlf() {
        let lf_only =
            "From: me@example.com\nSubject: Test\nContent-Type: text/plain\n\nZeile1\nZeile2\n";
        let entity = signable_entity(lf_only);
        assert!(entity.contains("Zeile1\r\nZeile2"));
        assert!(!entity.contains("Zeile1\nZeile2"));
    }

    #[test]
    fn nested_sign_then_encrypt_keeps_the_signature_inside() {
        let entity = signed_entity_for_encryption(MSG, "SIGNATURE");
        assert!(entity.starts_with("Content-Type: multipart/signed"));
        // Routing headers belong on the outer encrypted message only.
        assert!(!entity.contains("Subject: Test"));
    }

    #[test]
    fn handles_a_message_without_explicit_content_headers() {
        let bare = "From: me@example.com\r\nSubject: X\r\n\r\nnur text\r\n";
        let entity = signable_entity(bare);
        assert!(entity.starts_with("Content-Type: text/plain"));
        assert!(entity.contains("nur text"));
    }

    #[test]
    fn signed_message_round_trips_into_exact_verification_parts() {
        let signature = "-----BEGIN PGP SIGNATURE-----\nabc\n-----END PGP SIGNATURE-----";
        let signed = build_signed(MSG, signature);
        let parsed = parse_signed_message(signed.as_bytes()).expect("parse signed message");

        assert_eq!(parsed.signed_entity, signable_entity(MSG).as_bytes());
        assert_eq!(parsed.armored_signature, normalize_crlf(signature));
    }

    #[test]
    fn malformed_signed_message_is_rejected() {
        assert!(parse_signed_message(b"Content-Type: multipart/signed\r\n\r\nbody").is_err());
    }
}
