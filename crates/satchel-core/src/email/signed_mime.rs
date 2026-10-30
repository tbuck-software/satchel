//! The two parts of a `multipart/signed` body (RFC 1847), split once.
//!
//! A signature only vouches for the bytes the reader is shown. Display and
//! verification therefore take the signed entity from this one splitter, and
//! it refuses any structure that another MIME reader could split differently:
//! a signature part known only by a secondary header or named twice, a
//! parameter list that parses two ways, a delimiter-like line outside the
//! signed entity. A refused structure carries no signature state at all.

use std::borrow::Cow;

use crate::providers::imap::{extract_header_param, parse_header_value, parse_raw_header_value};
use crate::providers::traits::MessageTrust;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignedProtocol {
    OpenPgp,
    Smime,
}

impl SignedProtocol {
    /// The protocol a `multipart/signed` content type declares.
    pub fn declared(content_type: &str) -> Option<Self> {
        Self::named(&extract_header_param(content_type, "protocol")?)
    }

    fn named(protocol: &str) -> Option<Self> {
        match protocol.to_ascii_lowercase().as_str() {
            "application/pgp-signature" => Some(Self::OpenPgp),
            "application/pkcs7-signature" | "application/x-pkcs7-signature" => Some(Self::Smime),
            _ => None,
        }
    }

    /// Whether a signature part of this media type belongs to the protocol.
    /// The two S/MIME spellings are interchangeable in practice.
    fn accepts(self, media_type: &str) -> bool {
        match self {
            Self::OpenPgp => media_type == "application/pgp-signature",
            Self::Smime => matches!(
                media_type,
                "application/pkcs7-signature" | "application/x-pkcs7-signature"
            ),
        }
    }
}

/// Exact byte ranges of the body; nothing is trimmed or re-encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SignedParts<'a> {
    pub protocol: SignedProtocol,
    /// The signed entity, headers included. The line break before the next
    /// delimiter belongs to that delimiter (RFC 2046 §5.1.1), not to the entity.
    pub entity: &'a [u8],
    /// The signature part, headers included.
    pub signature: &'a [u8],
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SignedMimeError {
    #[error("not a multipart/signed entity")]
    NotSigned,
    #[error("multipart/signed declares no supported protocol")]
    UnsupportedProtocol,
    #[error("multipart/signed has no boundary")]
    MissingBoundary,
    #[error("multipart/signed parameters do not parse unambiguously")]
    AmbiguousParameters,
    #[error("multipart/signed has {0} body parts instead of two")]
    PartCount(usize),
    #[error("a line outside the signed entity starts like a delimiter")]
    AmbiguousDelimiter,
    #[error("the signature part is not unambiguously of the declared protocol")]
    SignatureType,
    #[error("the signed message exceeds the size limit")]
    TooLarge,
    #[error("the signature part holds no armored OpenPGP signature")]
    MissingArmor,
    #[error("the S/MIME signed entity has a carriage return outside a line break")]
    BareCarriageReturn,
}

/// Split a `multipart/signed` body under the given content type.
pub fn split<'a>(content_type: &str, body: &'a [u8]) -> Result<SignedParts<'a>, SignedMimeError> {
    let media_type = content_type.split(';').next().unwrap_or_default().trim();
    if !media_type.eq_ignore_ascii_case("multipart/signed") {
        return Err(SignedMimeError::NotSigned);
    }
    let protocol =
        SignedProtocol::declared(content_type).ok_or(SignedMimeError::UnsupportedProtocol)?;
    let boundary =
        extract_header_param(content_type, "boundary").ok_or(SignedMimeError::MissingBoundary)?;
    // A reader that honours quoting must arrive at the same values, or it
    // finds different delimiters in the same body.
    let strict_boundary = unambiguous_param(content_type, "boundary")?;
    let strict_protocol = unambiguous_param(content_type, "protocol")?;
    if strict_boundary.as_deref() != Some(boundary.as_str())
        || strict_protocol.as_deref().and_then(SignedProtocol::named) != Some(protocol)
    {
        return Err(SignedMimeError::AmbiguousParameters);
    }

    let marker = format!("--{boundary}");
    let (delimiters, near_misses) = delimiter_lines(body, marker.as_bytes());
    let opening: Vec<&Delimiter> = delimiters.iter().filter(|line| !line.closing).collect();
    if opening.len() != 2 {
        return Err(SignedMimeError::PartCount(opening.len()));
    }
    let (first, second) = (opening[0], opening[1]);
    let entity = first.next_line..part_end(body, first.next_line, second.start);
    let signature_end = delimiters
        .iter()
        .find(|line| line.closing)
        .map_or(body.len(), |close| {
            part_end(body, second.next_line, close.start)
        });

    // RFC 2046 lets a reader take any line that begins with the delimiter as
    // one. Inside the entity such a line is the signer's, and covered; anywhere
    // else it would let that reader see other parts than this one.
    if near_misses
        .iter()
        .any(|start| !(entity.start..second.start).contains(start))
    {
        return Err(SignedMimeError::AmbiguousDelimiter);
    }

    // OpenSSL folds every CR in front of a line break into that break before
    // hashing, so `\r\r\n` verifies as `\r\n` while a reader splits it
    // differently. Such an entity is not shown with a signature.
    if protocol == SignedProtocol::Smime && has_bare_carriage_return(&body[entity.clone()]) {
        return Err(SignedMimeError::BareCarriageReturn);
    }

    let signature = &body[second.next_line..signature_end];
    let (signature_header, _) = entity_header_and_body(signature);
    let signature_header = String::from_utf8_lossy(signature_header);
    // A reader that takes another of several types sees another part, one
    // that may name itself an attachment.
    if header_count(&signature_header, "Content-Type") > 1 {
        return Err(SignedMimeError::SignatureType);
    }
    let signature_type = parse_header_value(&signature_header, "Content-Type").unwrap_or_default();
    let signature_media = signature_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if !protocol.accepts(&signature_media) {
        return Err(SignedMimeError::SignatureType);
    }

    Ok(SignedParts {
        protocol,
        entity: &body[entity],
        signature,
    })
}

/// Split a stored bundle: a `Content-Type` header, an empty line, the body.
///
/// The header is read as written, without decoding encoded words again: the
/// display path decoded it once before writing it, and a second pass could
/// turn it into a different boundary.
pub fn split_bundle(bundle: &[u8]) -> Result<SignedParts<'_>, SignedMimeError> {
    let (header, body) = entity_header_and_body(bundle);
    let content_type = parse_raw_header_value(&String::from_utf8_lossy(header), "Content-Type")
        .ok_or(SignedMimeError::NotSigned)?;
    split(&content_type, body)
}

/// The text and HTML a signed entity displays.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EntityDisplay {
    pub text: Option<String>,
    pub html: Option<String>,
}

/// What a signed entity displays, decoded the way ingest decodes the body of a
/// `multipart/signed` message.
pub fn display_entity(entity: &[u8]) -> EntityDisplay {
    let shown = crate::providers::imap::display_signed_entity(entity, 1);
    EntityDisplay {
        text: shown.text,
        html: shown.html,
    }
}

/// Where the body shown under a stored row's signature comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoredSignedBody {
    /// The row's own body, which the strict splitter wrote with the bundle.
    Stored,
    /// The bundle's signed entity: the bytes verification reads.
    Entity(EntityDisplay),
    /// Opaque S/MIME: the content travels inside the signature, and only
    /// the engine's verification extracts it.
    Encapsulated,
}

/// How a stored row shows what its bundle signs.
///
/// Only a row the strict splitter wrote together with its bundle keeps its
/// stored body. Any other root-signed row, stored before the splitter existed
/// or with its body evicted since, shows the entity decoded from the bundle,
/// so what is displayed is what is verified. A signature below the root only
/// ever reaches a partial state; such a row keeps the body of the whole
/// message.
pub fn stored_signed_body(
    bundle: &[u8],
    trust: &MessageTrust,
    has_stored_body: bool,
) -> Result<StoredSignedBody, SignedMimeError> {
    if !trust.root_crypto {
        return Ok(StoredSignedBody::Stored);
    }
    match split_bundle(bundle) {
        Err(SignedMimeError::NotSigned) => Ok(StoredSignedBody::Encapsulated),
        Err(error) => Err(error),
        Ok(_) if trust.signed_binding && has_stored_body => Ok(StoredSignedBody::Stored),
        Ok(parts) => Ok(StoredSignedBody::Entity(display_entity(parts.entity))),
    }
}

/// The media types a signature part carries.
pub fn is_signature_media_type(content_type: &str) -> bool {
    let media_type = content_type.split(';').next().unwrap_or_default().trim();
    [
        "application/pgp-signature",
        "application/pkcs7-signature",
        "application/x-pkcs7-signature",
    ]
    .iter()
    .any(|signature| media_type.eq_ignore_ascii_case(signature))
}

/// Whether a part of a message under `root_content_type` is, by its type, a
/// signature part: one of the signature types, or the root's declared
/// `protocol`. Its headers are not signed.
pub fn is_signature_part(content_type: &str, root_content_type: &str) -> bool {
    let media_type = content_type.split(';').next().unwrap_or_default().trim();
    is_signature_media_type(media_type)
        || extract_header_param(root_content_type, "protocol")
            .is_some_and(|protocol| protocol.trim().eq_ignore_ascii_case(media_type))
}

/// How many header fields of this name the block holds; continuation lines
/// belong to the field above them.
fn header_count(header: &str, name: &str) -> usize {
    header
        .lines()
        .filter(|line| !line.starts_with([' ', '\t']))
        .filter(|line| {
            line.split_once(':')
                .is_some_and(|(field, _)| field.trim().eq_ignore_ascii_case(name))
        })
        .count()
}

fn has_bare_carriage_return(entity: &[u8]) -> bool {
    memchr::memchr_iter(b'\r', entity).any(|index| entity.get(index + 1) != Some(&b'\n'))
}

/// Rebuild an S/MIME `multipart/signed` bundle around exactly the parts this
/// module splits, with a boundary that appears nowhere inside them, so the
/// S/MIME engine's own MIME parser cannot find other parts. Payloads that are
/// not `multipart/signed` (opaque `application/pkcs7-mime`, bare DER or PEM)
/// pass through unchanged.
pub fn bind_smime_bundle(payload: &[u8]) -> Result<Cow<'_, [u8]>, SignedMimeError> {
    let parts = match split_bundle(payload) {
        Ok(parts) => parts,
        Err(SignedMimeError::NotSigned) => return Ok(Cow::Borrowed(payload)),
        Err(error) => return Err(error),
    };
    if parts.protocol != SignedProtocol::Smime {
        return Err(SignedMimeError::UnsupportedProtocol);
    }
    let boundary = (0_u32..)
        .map(|attempt| format!("satchel-signed-{attempt}"))
        .find(|candidate| {
            [parts.entity, parts.signature]
                .iter()
                .all(|part| memchr::memmem::find(part, candidate.as_bytes()).is_none())
        })
        .unwrap_or_default();
    let mut bound = format!(
        "Content-Type: multipart/signed; protocol=\"application/pkcs7-signature\"; \
         boundary=\"{boundary}\"\r\n\r\n--{boundary}\r\n"
    )
    .into_bytes();
    bound.extend_from_slice(parts.entity);
    bound.extend_from_slice(format!("\r\n--{boundary}\r\n").as_bytes());
    bound.extend_from_slice(parts.signature);
    bound.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    Ok(Cow::Owned(bound))
}

/// Headers and body of a MIME part: the headers end at the first empty line,
/// whatever its line ending. A part that opens with an empty line has none.
pub fn entity_header_and_body(part: &[u8]) -> (&[u8], &[u8]) {
    let mut line_start = 0;
    while line_start < part.len() {
        let line_end = memchr::memchr(b'\n', &part[line_start..])
            .map_or(part.len(), |offset| line_start + offset + 1);
        let line = &part[line_start..line_end];
        if matches!(line, b"\n" | b"\r\n") {
            let header_end = line_start.saturating_sub(1);
            let header_end = if header_end > 0 && part[header_end - 1] == b'\r' {
                header_end - 1
            } else {
                header_end
            };
            return (&part[..header_end], &part[line_end..]);
        }
        line_start = line_end;
    }
    (b"", part)
}

struct Delimiter {
    start: usize,
    next_line: usize,
    closing: bool,
}

/// Delimiter lines up to and including the close delimiter (RFC 2046): the
/// delimiter at the start of a line, optionally `--` for the close, optional
/// transport padding, then the line break. Also returns where a line begins
/// with the delimiter but goes on with something else.
fn delimiter_lines(body: &[u8], marker: &[u8]) -> (Vec<Delimiter>, Vec<usize>) {
    let mut delimiters = Vec::new();
    let mut near_misses = Vec::new();
    for start in memchr::memmem::find_iter(body, marker) {
        if start != 0 && body[start - 1] != b'\n' {
            continue;
        }
        let after = start + marker.len();
        let closing = body[after..].starts_with(b"--");
        match line_end_after_padding(body, if closing { after + 2 } else { after }) {
            Some(next_line) => {
                delimiters.push(Delimiter {
                    start,
                    next_line,
                    closing,
                });
                if closing {
                    break;
                }
            }
            None => near_misses.push(start),
        }
    }
    (delimiters, near_misses)
}

fn line_end_after_padding(body: &[u8], from: usize) -> Option<usize> {
    let mut index = from;
    while matches!(body.get(index), Some(b' ' | b'\t')) {
        index += 1;
    }
    match body.get(index) {
        None => Some(body.len()),
        Some(b'\n') => Some(index + 1),
        Some(b'\r') if body.get(index + 1) == Some(&b'\n') => Some(index + 2),
        _ => None,
    }
}

/// The end of a part whose next delimiter starts at `delimiter`: the line
/// break in front of the delimiter is the delimiter's.
fn part_end(body: &[u8], start: usize, delimiter: usize) -> usize {
    let mut end = delimiter;
    if end > start && body[end - 1] == b'\n' {
        end -= 1;
        if end > start && body[end - 1] == b'\r' {
            end -= 1;
        }
    }
    end
}

/// The value of a parameter as a reader that honours quoting reads it
/// (RFC 2045 §5.1). A repeated parameter, an RFC 2231 form of it, a comment or
/// a broken quote anywhere in the list makes it ambiguous: readers disagree
/// about all of them.
fn unambiguous_param(content_type: &str, name: &str) -> Result<Option<String>, SignedMimeError> {
    let parameters = parameters(content_type).ok_or(SignedMimeError::AmbiguousParameters)?;
    let extended = format!("{name}*");
    let mut matching = parameters
        .into_iter()
        .filter(|(key, _)| key == name || key.starts_with(&extended));
    let first = matching.next();
    if matching.next().is_some() {
        return Err(SignedMimeError::AmbiguousParameters);
    }
    match first {
        Some((key, _)) if key != name => Err(SignedMimeError::AmbiguousParameters),
        Some((_, value)) => Ok(Some(value)),
        None => Ok(None),
    }
}

/// `name=value` pairs after the media type, names lower-cased, quoted strings
/// unescaped. `None` for anything a strict reader would not accept.
fn parameters(content_type: &str) -> Option<Vec<(String, String)>> {
    let bytes = content_type.as_bytes();
    let mut parameters = Vec::new();
    let Some(media_end) = memchr::memchr(b';', bytes) else {
        return Some(parameters);
    };
    if bytes[..media_end]
        .iter()
        .any(|byte| matches!(byte, b'"' | b'('))
    {
        return None;
    }
    let mut index = media_end + 1;
    while index < bytes.len() {
        if matches!(bytes[index], b' ' | b'\t' | b';') {
            index += 1;
            continue;
        }
        let name_start = index;
        while index < bytes.len() && !matches!(bytes[index], b'=' | b';') {
            if matches!(bytes[index], b'"' | b'(') {
                return None;
            }
            index += 1;
        }
        let name = content_type[name_start..index].trim().to_ascii_lowercase();
        if index == bytes.len() || bytes[index] == b';' {
            // Valueless segment; senders do write `; ;`.
            continue;
        }
        index += 1;
        while matches!(bytes.get(index), Some(b' ' | b'\t')) {
            index += 1;
        }
        let value = if bytes.get(index) == Some(&b'"') {
            index += 1;
            let mut value = Vec::new();
            loop {
                match *bytes.get(index)? {
                    b'"' => {
                        index += 1;
                        break;
                    }
                    b'\\' => {
                        value.push(*bytes.get(index + 1)?);
                        index += 2;
                    }
                    byte => {
                        value.push(byte);
                        index += 1;
                    }
                }
            }
            while matches!(bytes.get(index), Some(b' ' | b'\t')) {
                index += 1;
            }
            if index < bytes.len() && bytes[index] != b';' {
                return None;
            }
            String::from_utf8(value).ok()?
        } else {
            let value_start = index;
            while index < bytes.len() && bytes[index] != b';' {
                if matches!(bytes[index], b'"' | b'(') {
                    return None;
                }
                index += 1;
            }
            content_type[value_start..index].trim().to_string()
        };
        parameters.push((name, value));
    }
    Some(parameters)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PGP: &str =
        "multipart/signed; micalg=pgp-sha256; protocol=\"application/pgp-signature\"; boundary=\"b\"";
    const ENTITY: &str = "Content-Type: text/plain\r\n\r\nSigned text\r\n";
    const SIGNATURE: &str = "Content-Type: application/pgp-signature\r\n\r\n\
                             -----BEGIN PGP SIGNATURE-----\r\nAAAA\r\n-----END PGP SIGNATURE-----\r\n";

    fn body(eol: &str) -> String {
        format!("--b{eol}{ENTITY}{eol}--b{eol}{SIGNATURE}{eol}--b--{eol}").replace("\r\n", eol)
    }

    #[test]
    fn splits_exact_entity_bytes_for_crlf_and_lf() {
        for eol in ["\r\n", "\n"] {
            let body = body(eol);
            let parts = split(PGP, body.as_bytes()).unwrap();
            assert_eq!(parts.protocol, SignedProtocol::OpenPgp);
            assert_eq!(parts.entity, ENTITY.replace("\r\n", eol).as_bytes());
            assert_eq!(parts.signature, SIGNATURE.replace("\r\n", eol).as_bytes());
        }
    }

    #[test]
    fn preamble_epilogue_and_transport_padding_are_not_part_of_the_entity() {
        let body = format!(
            "preamble line\r\n--b \t\r\n{ENTITY}\r\n--b\r\n{SIGNATURE}\r\n--b-- \r\nepilogue\r\n--b\r\n"
        );
        let parts = split(PGP, body.as_bytes()).unwrap();
        assert_eq!(parts.entity, ENTITY.as_bytes());
    }

    #[test]
    fn a_delimiter_inside_a_line_is_text() {
        let body = format!(
            "x--b\r\nContent-Type: text/plain\r\n\r\nPreamble\r\n--b\r\n{ENTITY}\r\n--b\r\n{SIGNATURE}\r\n--b--\r\n"
        );
        let parts = split(PGP, body.as_bytes()).unwrap();
        assert_eq!(parts.entity, ENTITY.as_bytes());
        assert_eq!(parts.signature, SIGNATURE.as_bytes());
    }

    #[test]
    fn exactly_two_parts() {
        let one = format!("--b\r\n{ENTITY}\r\n--b--\r\n");
        let three =
            format!("--b\r\n{ENTITY}\r\n--b\r\n{ENTITY}\r\n--b\r\n{SIGNATURE}\r\n--b--\r\n");
        assert_eq!(
            split(PGP, one.as_bytes()),
            Err(SignedMimeError::PartCount(1))
        );
        assert_eq!(
            split(PGP, three.as_bytes()),
            Err(SignedMimeError::PartCount(3))
        );
        // A missing close delimiter leaves the signature running to the end.
        let unclosed = format!("--b\r\n{ENTITY}\r\n--b\r\n{SIGNATURE}");
        assert_eq!(
            split(PGP, unclosed.as_bytes()).unwrap().signature,
            SIGNATURE.as_bytes()
        );
    }

    #[test]
    fn the_signature_part_is_known_by_its_content_type_only() {
        let disguised = "Content-Type: text/plain\r\n\
                         Content-Description: application/pgp-signature\r\n\r\ntext";
        let body = format!("--b\r\n{ENTITY}\r\n--b\r\n{disguised}\r\n--b--\r\n");
        assert_eq!(
            split(PGP, body.as_bytes()),
            Err(SignedMimeError::SignatureType)
        );

        let smime = PGP.replace("application/pgp-signature", "application/pkcs7-signature");
        assert_eq!(
            split(&smime, self::body("\r\n").as_bytes()),
            Err(SignedMimeError::SignatureType)
        );
        // A reader that takes the second type sees another part entirely.
        for doubled in [
            "Content-Type: application/pgp-signature\r\nContent-Type: application/pdf",
            "Content-Type: application/pgp-signature\r\ncontent-type:application/pgp-signature",
        ] {
            let body = format!("--b\r\n{ENTITY}\r\n--b\r\n{doubled}\r\n\r\nsig\r\n--b--\r\n");
            assert_eq!(
                split(PGP, body.as_bytes()),
                Err(SignedMimeError::SignatureType),
                "{doubled:?}"
            );
        }
        let folded = "Content-Disposition: attachment;\r\n\tfilename=signature.asc\r\n\
                      Content-Type: application/pgp-signature;\r\n\tname=signature.asc\r\n\r\nsig";
        let body = format!("--b\r\n{ENTITY}\r\n--b\r\n{folded}\r\n--b--\r\n");
        assert!(split(PGP, body.as_bytes()).is_ok());
    }

    #[test]
    fn a_carriage_return_outside_a_line_break_is_refused_for_smime() {
        let smime = PGP.replace("application/pgp-signature", "application/pkcs7-signature");
        let signature = "Content-Type: application/pkcs7-signature\r\n\r\nMIAG";
        for entity in [
            "Content-Type: text/plain\r\n\r\r\nText",
            "Content-Type: text/plain\r\n\r\nText\rMore",
            "Content-Type: text/plain\r\n\r\nText\r",
        ] {
            let body = format!("--b\r\n{entity}\r\n--b\r\n{signature}\r\n--b--\r\n");
            assert_eq!(
                split(&smime, body.as_bytes()),
                Err(SignedMimeError::BareCarriageReturn),
                "{entity:?}"
            );
        }
        // OpenPGP verifies the entity bytes as they are.
        let entity = "Content-Type: text/plain\r\n\r\nText\rMore";
        let body = format!("--b\r\n{entity}\r\n--b\r\n{SIGNATURE}\r\n--b--\r\n");
        assert_eq!(
            split(PGP, body.as_bytes()).unwrap().entity,
            entity.as_bytes()
        );
    }

    #[test]
    fn delimiter_like_lines_count_only_inside_the_entity() {
        let nested = "Content-Type: multipart/mixed; boundary=\"b2\"\r\n\r\n--b2\r\n\
                      Content-Type: text/plain\r\n\r\nx\r\n--b2--\r\n";
        let body = format!("--b\r\n{nested}\r\n--b\r\n{SIGNATURE}\r\n--b--\r\n");
        assert_eq!(
            split(PGP, body.as_bytes()).unwrap().entity,
            nested.as_bytes()
        );

        for misplaced in [
            format!("--bx\r\n--b\r\n{ENTITY}\r\n--b\r\n{SIGNATURE}\r\n--b--\r\n"),
            format!("--b\r\n{ENTITY}\r\n--b\r\n{SIGNATURE}\r\n--bx\r\n\r\n--b--\r\n"),
        ] {
            assert_eq!(
                split(PGP, misplaced.as_bytes()),
                Err(SignedMimeError::AmbiguousDelimiter)
            );
        }
    }

    #[test]
    fn parameters_must_read_the_same_to_every_reader() {
        let body = body("\r\n");
        for content_type in [
            "multipart/signed; protocol=\"application/pgp-signature\"; boundary=\"b;x=1\"",
            "multipart/signed; protocol=\"application/pgp-signature\"; boundary=b; boundary=c",
            "multipart/signed; protocol=\"application/pgp-signature\"; boundary=b; boundary*=c",
            "multipart/signed; protocol=\"application/pgp-signature\"; boundary='b'",
            "multipart/signed; protocol=\"application/pgp-signature\"; boundary=\"b \"",
            "multipart/signed; protocol=\"application/pgp-signature\"; boundary=b (comment)",
            "multipart/signed; protocol=\"application/pgp-signature\"; boundary=\"b",
            "multipart/signed; protocol=\"application/pgp-signature\"; protocol=application/pkcs7-signature; boundary=b",
        ] {
            assert_eq!(
                split(content_type, body.as_bytes()),
                Err(SignedMimeError::AmbiguousParameters),
                "{content_type}"
            );
        }
        for content_type in [
            "multipart/signed; protocol=\"application/pgp-signature\"; ; boundary=b",
            "Multipart/Signed;\tBoundary=\"b\"; Protocol=\"Application/PGP-Signature\"; micalg=pgp-sha256",
        ] {
            assert!(split(content_type, body.as_bytes()).is_ok(), "{content_type}");
        }
    }

    #[test]
    fn a_stored_bundle_is_read_back_without_decoding_again() {
        let content_type = format!("{PGP}; x=\"=?utf-8?q?a?=\"");
        let bundle = format!("Content-Type: {content_type}\r\n\r\n{}", body("\r\n"));
        let parts = split_bundle(bundle.as_bytes()).unwrap();
        assert_eq!(parts.entity, ENTITY.as_bytes());
        assert_eq!(
            split_bundle(b"Content-Type: text/plain\r\n\r\nhello"),
            Err(SignedMimeError::NotSigned)
        );
    }

    #[test]
    fn entity_headers_end_at_the_first_empty_line_of_either_ending() {
        assert_eq!(
            entity_header_and_body(b"A: 1\nB: 2\r\n\r\nbody\n\nmore"),
            (&b"A: 1\nB: 2"[..], &b"body\n\nmore"[..])
        );
        assert_eq!(
            entity_header_and_body(b"\r\nno headers\r\n\r\nx"),
            (&b""[..], &b"no headers\r\n\r\nx"[..])
        );
        assert_eq!(
            entity_header_and_body(b"only text"),
            (&b""[..], &b"only text"[..])
        );
    }

    #[test]
    fn smime_bundles_are_rebuilt_around_the_split_parts() {
        let content_type = PGP.replace("application/pgp-signature", "application/pkcs7-signature");
        let signature = "Content-Type: application/pkcs7-signature\r\n\r\nMIAG\r\n";
        let body = format!("--b\r\n{ENTITY}\r\n--b\r\n{signature}\r\n--b--\r\n");
        let bundle = format!("Content-Type: {content_type}\r\n\r\n{body}");
        let bound = bind_smime_bundle(bundle.as_bytes()).unwrap();
        let reread = split_bundle(&bound).unwrap();
        assert_eq!(reread.entity, ENTITY.as_bytes());
        assert_eq!(reread.signature, signature.as_bytes());

        let opaque = b"\x30\x82\x01\x00not mime";
        assert!(matches!(bind_smime_bundle(opaque), Ok(Cow::Borrowed(_))));
        let pgp = format!("Content-Type: {PGP}\r\n\r\n{}", self::body("\r\n"));
        assert_eq!(
            bind_smime_bundle(pgp.as_bytes()),
            Err(SignedMimeError::UnsupportedProtocol)
        );
    }
}
