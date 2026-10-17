use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use encoding_rs::{Encoding, UTF_8};

const WINDOWS_1252: &str = "windows-1252";

/// How the items in an address list are represented when they reach the
/// shared normalizer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AddressListDecodeStrategy {
    /// The surrounding header parser already decoded MIME encoded-words.
    AlreadyDecoded,
    /// Decode MIME encoded-words in each list item before normalizing it.
    MimeEncodedWords,
}

/// Normalize comma-separated address-list items while preserving provider
/// specific decoding semantics at the call site.
pub(crate) fn parse_address_list_tokens(
    raw: &str,
    strategy: AddressListDecodeStrategy,
) -> Vec<String> {
    raw.split(',')
        .filter_map(|item| {
            let decoded = match strategy {
                AddressListDecodeStrategy::AlreadyDecoded => item.trim().to_string(),
                AddressListDecodeStrategy::MimeEncodedWords => {
                    decode_mime_encoded_words(item.trim())
                }
            };
            let value = extract_email_address(&decoded).unwrap_or(decoded);
            (!value.is_empty()).then_some(value)
        })
        .fold(Vec::new(), |mut acc, value| {
            if !acc.iter().any(|existing| existing == &value) {
                acc.push(value);
            }
            acc
        })
}

pub fn decode_mime_encoded_words(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut offset = 0usize;

    while offset < input.len() {
        let rest = &input[offset..];
        if let Some((decoded, consumed)) = try_decode_encoded_word(rest) {
            result.push_str(&decoded);
            offset += consumed;

            // RFC 2047: whitespace between adjacent encoded words is ignored.
            let mut gap = offset;
            while gap < input.len() {
                let ch = input[gap..]
                    .chars()
                    .next()
                    .expect("char boundary while scanning MIME header");
                if matches!(ch, ' ' | '\t' | '\r' | '\n') {
                    gap += ch.len_utf8();
                } else {
                    break;
                }
            }
            if input[gap..].starts_with("=?") {
                offset = gap;
            }
            continue;
        }

        let ch = rest
            .chars()
            .next()
            .expect("char boundary while copying MIME header");
        result.push(ch);
        offset += ch.len_utf8();
    }

    result.trim().to_string()
}

pub fn decode_html_entities(input: &str) -> String {
    if !input.contains('&') {
        return input.to_string();
    }

    let mut output = String::with_capacity(input.len());
    let mut rest = input;

    while let Some(entity_start) = rest.find('&') {
        output.push_str(&rest[..entity_start]);
        let candidate = &rest[entity_start..];
        let Some(entity_end) = candidate.find(';') else {
            output.push_str(candidate);
            return output;
        };

        let entity = &candidate[1..entity_end];
        if let Some(decoded) = decode_html_entity(entity) {
            output.push(decoded);
            rest = &candidate[entity_end + 1..];
        } else {
            output.push_str(&candidate[..=entity_end]);
            rest = &candidate[entity_end + 1..];
        }
    }

    output.push_str(rest);
    output
}

fn decode_html_entity(entity: &str) -> Option<char> {
    match entity {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some('\u{00a0}'),
        _ => {
            let numeric = entity.strip_prefix('#')?;
            let value = if let Some(hex) = numeric
                .strip_prefix('x')
                .or_else(|| numeric.strip_prefix('X'))
            {
                u32::from_str_radix(hex, 16).ok()?
            } else {
                numeric.parse::<u32>().ok()?
            };
            char::from_u32(value)
        }
    }
}

/// Lenient quoted-printable decoder: drops soft line breaks (`=\r\n`, `=\n`)
/// and expands `=XX` hex escapes. Invalid or truncated escape sequences are
/// copied through verbatim, matching the tolerance real-world mail needs.
pub fn decode_quoted_printable_bytes(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    let mut index = 0usize;
    while index < input.len() {
        if input[index] == b'=' {
            if index + 1 < input.len() && input[index + 1] == b'\n' {
                index += 2;
                continue;
            }
            if index + 2 < input.len() && input[index + 1] == b'\r' && input[index + 2] == b'\n' {
                index += 3;
                continue;
            }
            if index + 2 < input.len() {
                if let (Some(high), Some(low)) = (
                    quoted_printable_hex(input[index + 1]),
                    quoted_printable_hex(input[index + 2]),
                ) {
                    out.push((high << 4) | low);
                    index += 3;
                    continue;
                }
            }
        }

        out.push(input[index]);
        index += 1;
    }
    out
}

fn quoted_printable_hex(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

pub fn decode_bytes_with_charset(bytes: &[u8], charset: &str) -> String {
    let normalized = normalize_charset_label(charset);
    let decoder = Encoding::for_label(normalized.as_bytes()).unwrap_or(UTF_8);
    let (decoded, _, had_errors) = decoder.decode(bytes);
    if had_errors && (normalized == "utf-8" || normalized == "utf8") && !is_valid_utf8(bytes) {
        let fallback = Encoding::for_label(WINDOWS_1252.as_bytes()).unwrap_or(UTF_8);
        let (fallback_decoded, _, _) = fallback.decode(bytes);
        return fallback_decoded.into_owned();
    }
    let decoded = decoded.into_owned();
    if normalized != "utf-8" && normalized != "utf8" && is_valid_utf8(bytes) {
        let repaired = repair_likely_utf8_mojibake(&decoded);
        if repaired != decoded {
            return repaired;
        }
    }
    decoded
}

/// Decode with exactly the charset the MIME part declared.
///
/// Unlike `decode_bytes_with_charset`, this does not substitute Windows-1252
/// for malformed UTF-8 and does not repair apparent mojibake. Callers receive
/// whether replacement characters were necessary, and an unknown charset is
/// an explicit error rather than an implicit UTF-8 guess.
pub fn decode_bytes_with_declared_charset(
    bytes: &[u8],
    charset: &str,
) -> Result<(String, bool), String> {
    let normalized = normalize_charset_label(charset);
    let decoder = Encoding::for_label(normalized.as_bytes())
        .ok_or_else(|| format!("unknown declared charset `{charset}`"))?;
    let (decoded, _, had_errors) = decoder.decode(bytes);
    Ok((decoded.into_owned(), had_errors))
}

pub fn repair_likely_utf8_mojibake(input: &str) -> String {
    if utf8_mojibake_score(input) == 0 {
        return input.to_string();
    }

    let encoder = Encoding::for_label(WINDOWS_1252.as_bytes()).unwrap_or(UTF_8);
    let mut repaired = String::with_capacity(input.len());
    let mut encodable_segment = String::new();

    for character in input.chars() {
        let mut buffer = [0; 4];
        let encoded_character = character.encode_utf8(&mut buffer);
        let (_, _, had_errors) = encoder.encode(encoded_character);
        if had_errors {
            append_repaired_mojibake_segment(&mut repaired, &encodable_segment, encoder);
            encodable_segment.clear();
            repaired.push(character);
        } else {
            encodable_segment.push(character);
        }
    }
    append_repaired_mojibake_segment(&mut repaired, &encodable_segment, encoder);

    if repaired != input && utf8_mojibake_score(&repaired) < utf8_mojibake_score(input) {
        repaired
    } else {
        input.to_string()
    }
}

fn append_repaired_mojibake_segment(
    output: &mut String,
    segment: &str,
    encoder: &'static Encoding,
) {
    if segment.is_empty() || utf8_mojibake_score(segment) == 0 {
        output.push_str(segment);
        return;
    }

    let (bytes, _, had_errors) = encoder.encode(segment);
    if had_errors {
        output.push_str(segment);
        return;
    }

    let Ok(candidate) = std::str::from_utf8(&bytes) else {
        output.push_str(segment);
        return;
    };

    if utf8_mojibake_score(candidate) < utf8_mojibake_score(segment) {
        output.push_str(candidate);
    } else {
        output.push_str(segment);
    }
}

fn utf8_mojibake_score(input: &str) -> usize {
    const MARKERS: [&str; 10] = [
        "Ã",
        "Â",
        "â€",
        "â€™",
        "â€œ",
        "â€\u{009d}",
        "â€“",
        "â€”",
        "â€¦",
        "â€¢",
    ];

    MARKERS
        .iter()
        .map(|marker| input.matches(marker).count())
        .sum()
}

pub fn decode_rfc2231_value(value: &str) -> String {
    let trimmed = value.trim().trim_matches('"').trim_matches('\'');
    if let Some((charset, encoded)) = parse_rfc2231_extended_value(trimmed) {
        let bytes = percent_decode_to_bytes(encoded);
        return decode_bytes_with_charset(&bytes, charset);
    }
    decode_mime_encoded_words(trimmed)
}

pub fn decode_rfc2231_segments(segments: &[(usize, bool, String)]) -> Option<String> {
    if segments.is_empty() {
        return None;
    }
    let mut sorted = segments.to_vec();
    sorted.sort_by_key(|(index, _, _)| *index);
    if sorted.first().map(|(index, _, _)| *index) != Some(0) {
        return None;
    }
    let any_extended = sorted.iter().any(|(_, extended, _)| *extended);
    let combined = sorted
        .into_iter()
        .map(|(_, _, value)| {
            value
                .trim()
                .trim_matches('"')
                .trim_matches('\'')
                .to_string()
        })
        .collect::<String>();
    if any_extended {
        Some(decode_rfc2231_value(&combined))
    } else {
        Some(decode_mime_encoded_words(&combined))
    }
}

fn parse_rfc2231_extended_value(value: &str) -> Option<(&str, &str)> {
    let mut parts = value.splitn(3, '\'');
    let charset = parts.next()?.trim();
    let _language = parts.next()?;
    let encoded = parts.next()?;
    if charset.is_empty() {
        return None;
    }
    Some((charset, encoded))
}

fn percent_decode_to_bytes(value: &str) -> Vec<u8> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let (Some(high), Some(low)) =
                (from_hex(bytes[index + 1]), from_hex(bytes[index + 2]))
            {
                out.push((high << 4) | low);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    out
}

fn normalize_charset_label(charset: &str) -> String {
    match charset
        .trim()
        .trim_matches('"')
        .to_ascii_lowercase()
        .as_str()
    {
        "" => "utf-8".to_string(),
        "latin1" | "latin-1" => "iso-8859-1".to_string(),
        "utf8" => "utf-8".to_string(),
        "win1252" | "cp1252" => WINDOWS_1252.to_string(),
        other => other.to_string(),
    }
}

fn is_valid_utf8(bytes: &[u8]) -> bool {
    std::str::from_utf8(bytes).is_ok()
}

pub fn extract_email_address(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let (Some(start), Some(end)) = (trimmed.find('<'), trimmed.rfind('>')) {
        if end > start {
            let candidate = trimmed[start + 1..end].trim();
            if is_likely_email(candidate) {
                return Some(candidate.to_string());
            }
        }
    }

    for token in trimmed.split(|ch: char| ch.is_whitespace() || matches!(ch, ',' | ';')) {
        let candidate = token.trim_matches(is_address_wrapper);
        if is_likely_email(candidate) {
            return Some(candidate.to_string());
        }
    }

    None
}

/// Every address an address-list entry contains, in order.
///
/// `extract_email_address` answers "which address is this?" and stops at the
/// first hit. A composer entry may legitimately carry two, and dropping the
/// second is how a recipient disappears without a warning. Angle brackets win
/// when the entry has any: a display name is not an address, even when it looks
/// like one.
pub fn extract_email_addresses(raw: &str) -> Vec<String> {
    let bracketed = bracketed_addresses(raw);
    if bracketed.is_empty() {
        bare_addresses(raw)
    } else {
        bracketed
    }
}

/// Like [`extract_email_addresses`], keeping each address's display name.
///
/// `"Meier, Anna" <a@x.de>, b@y.de` → `[(Some("Meier, Anna"), "a@x.de"), (None, "b@y.de")]`.
pub fn extract_mailboxes(raw: &str) -> Vec<(Option<String>, String)> {
    let mut mailboxes: Vec<(Option<String>, String)> = Vec::new();
    for segment in top_level_segments(raw) {
        let bracketed = bracketed_addresses(&segment);
        let named = bracketed.len() == 1;
        let addresses = if bracketed.is_empty() {
            bare_addresses(&segment)
        } else {
            bracketed
        };
        for address in addresses {
            if mailboxes.iter().any(|(_, existing)| *existing == address) {
                continue;
            }
            let name = named
                .then(|| display_name_of(&segment))
                .flatten()
                .filter(|name| !name.eq_ignore_ascii_case(&address));
            mailboxes.push((name, address));
        }
    }
    mailboxes
}

/// Splits at `,` and `;` outside quotes and angle brackets.
fn top_level_segments(raw: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let (mut quoted, mut escaped, mut bracket) = (false, false, false);
    for character in raw.chars() {
        match character {
            _ if escaped => escaped = false,
            '\\' if quoted => escaped = true,
            '"' if !bracket => quoted = !quoted,
            '<' if !quoted => bracket = true,
            '>' if !quoted => bracket = false,
            ',' | ';' if !quoted && !bracket => {
                segments.push(std::mem::take(&mut current));
                continue;
            }
            _ => {}
        }
        current.push(character);
    }
    segments.push(current);
    segments
}

/// The phrase before `<`, unquoted; `None` when empty.
fn display_name_of(segment: &str) -> Option<String> {
    let phrase = segment.split('<').next()?.trim();
    let unquoted = phrase
        .strip_prefix('"')
        .and_then(|inner| inner.strip_suffix('"'))
        .map(|inner| inner.replace("\\\"", "\"").replace("\\\\", "\\"))
        .unwrap_or_else(|| phrase.to_string());
    let name = unquoted.trim();
    (!name.is_empty()).then(|| name.to_string())
}

fn bracketed_addresses(raw: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut quoted = false;
    let mut current: Option<String> = None;
    for character in raw.chars() {
        if character == '"' && current.is_none() {
            quoted = !quoted;
        } else if quoted {
            continue;
        } else if character == '<' {
            current = Some(String::new());
        } else if character == '>' {
            if let Some(candidate) = current.take() {
                push_unique(&mut found, candidate.trim());
            }
        } else if let Some(buffer) = current.as_mut() {
            buffer.push(character);
        }
    }
    found
}

fn bare_addresses(raw: &str) -> Vec<String> {
    let mut found = Vec::new();
    for token in raw.split(|ch: char| ch.is_whitespace() || matches!(ch, ',' | ';')) {
        push_unique(&mut found, token.trim_matches(is_address_wrapper));
    }
    found
}

fn is_address_wrapper(character: char) -> bool {
    matches!(
        character,
        '<' | '>' | '(' | ')' | '[' | ']' | '{' | '}' | '"' | '\'' | ':'
    )
}

fn push_unique(found: &mut Vec<String>, candidate: &str) {
    if is_likely_email(candidate) && !found.iter().any(|existing| existing == candidate) {
        found.push(candidate.to_string());
    }
}

pub fn mailbox_local_display_name(email: &str) -> String {
    let local = email.split('@').next().unwrap_or(email);
    local
        .split(['.', '_', '-'])
        .filter(|part| !part.is_empty())
        .map(title_case_segment)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn fallback_display_name(email: &str) -> String {
    let local_name = mailbox_local_display_name(email);
    if !local_name.is_empty() && !is_generic_mailbox_local_part(email) {
        return local_name;
    }

    if let Some(domain_name) = domain_display_name(email) {
        return domain_name;
    }

    if !local_name.is_empty() {
        return local_name;
    }

    "Unknown".to_string()
}

pub fn resolve_sender_display_name(raw_name: Option<&str>, email: &str) -> String {
    let decoded = raw_name
        .map(decode_mime_encoded_words)
        .unwrap_or_default()
        .trim()
        .to_string();

    if !decoded.is_empty()
        && !decoded.eq_ignore_ascii_case(email)
        && !looks_like_generic_sender_name(&decoded, email)
    {
        return decoded;
    }

    fallback_display_name(email)
}

/// Do two mailboxes name the same recipient?
///
/// RFC 5321 leaves the local part to the receiving host and folds only the
/// domain, so that is the one half compared case-insensitively. Used to bind a
/// signer identity to the `From:` mailbox, where a wrong `true` is a forged
/// sender.
pub fn mailbox_matches(left: &str, right: &str) -> bool {
    match (split_mailbox(left), split_mailbox(right)) {
        (Some((left_local, left_domain)), Some((right_local, right_domain))) => {
            left_local == right_local && left_domain.eq_ignore_ascii_case(right_domain)
        }
        _ => false,
    }
}

fn split_mailbox(raw: &str) -> Option<(&str, &str)> {
    let trimmed = raw.trim();
    let at = trimmed.rfind('@')?;
    let local = trimmed[..at].trim();
    let domain = trimmed[at + 1..].trim();
    (!local.is_empty() && !domain.is_empty()).then_some((local, domain))
}

fn is_likely_email(value: &str) -> bool {
    let at = value.find('@');
    match at {
        Some(index) if index > 0 && index + 1 < value.len() => {
            value[index + 1..].contains('.') || value[index + 1..].contains('-')
        }
        _ => false,
    }
}

fn try_decode_encoded_word(rest: &str) -> Option<(String, usize)> {
    // `=?charset?encoding?payload?=` — the terminating `?=` must be searched
    // within the payload only: in `=?UTF-8?Q?=5B...` the delimiter after `Q`
    // plus the payload's leading `=` already form a false `?=`.
    let mut parts = rest.strip_prefix("=?")?.splitn(3, '?');
    let charset = parts.next()?.trim();
    let encoding = parts.next()?.trim();
    let remainder = parts.next()?;
    let payload_end = remainder.find("?=")?;
    let payload = &remainder[..payload_end];
    let consumed = rest.len() - remainder.len() + payload_end + 2;

    if charset.is_empty() || encoding.is_empty() {
        return None;
    }

    let bytes = if encoding.eq_ignore_ascii_case("B") {
        let compact = payload
            .chars()
            .filter(|ch| !ch.is_ascii_whitespace())
            .collect::<String>();
        STANDARD.decode(compact).ok()?
    } else if encoding.eq_ignore_ascii_case("Q") {
        decode_rfc2047_q(payload)?
    } else {
        return None;
    };

    let charset_label = charset.split('*').next().unwrap_or(charset);
    let decoder = Encoding::for_label(charset_label.as_bytes()).unwrap_or(UTF_8);
    let (decoded, _, _) = decoder.decode(&bytes);
    Some((decoded.into_owned(), consumed))
}

fn decode_rfc2047_q(input: &str) -> Option<Vec<u8>> {
    let bytes = input.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0usize;

    while index < bytes.len() {
        match bytes[index] {
            b'_' => {
                output.push(b' ');
                index += 1;
            }
            b'=' => {
                if index + 2 >= bytes.len() {
                    return None;
                }
                let high = from_hex(bytes[index + 1])?;
                let low = from_hex(bytes[index + 2])?;
                output.push((high << 4) | low);
                index += 3;
            }
            value => {
                output.push(value);
                index += 1;
            }
        }
    }

    Some(output)
}

fn from_hex(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn looks_like_generic_sender_name(name: &str, email: &str) -> bool {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return true;
    }

    if trimmed.eq_ignore_ascii_case(email) {
        return true;
    }

    let local_name = mailbox_local_display_name(email);
    if !local_name.is_empty() && trimmed.eq_ignore_ascii_case(&local_name) {
        return is_generic_mailbox_local_part(email);
    }

    false
}

fn is_generic_mailbox_local_part(email: &str) -> bool {
    let local = email.split('@').next().unwrap_or(email);
    let tokens = local
        .split(['.', '_', '-'])
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();

    if tokens.is_empty() {
        return false;
    }

    tokens
        .iter()
        .all(|token| matches_generic_mailbox_token(token))
}

fn matches_generic_mailbox_token(token: &str) -> bool {
    matches!(
        token.to_ascii_lowercase().as_str(),
        "no" | "reply"
            | "noreply"
            | "donotreply"
            | "do"
            | "not"
            | "notifications"
            | "notification"
            | "notify"
            | "updates"
            | "update"
            | "mail"
            | "mailer"
            | "hello"
            | "team"
            | "contact"
            | "info"
            | "support"
            | "service"
            | "admin"
            | "bot"
    )
}

fn domain_display_name(email: &str) -> Option<String> {
    let domain = email.split('@').nth(1)?.trim();
    if domain.is_empty() {
        return None;
    }

    let labels = domain
        .split('.')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if labels.is_empty() {
        return None;
    }

    let brand = if labels.len() >= 3
        && labels[labels.len() - 1].len() == 2
        && labels[labels.len() - 2].len() <= 3
    {
        labels[labels.len() - 3]
    } else if labels.len() >= 2 {
        labels[labels.len() - 2]
    } else {
        labels[0]
    };

    let formatted = brand
        .split(['-', '_'])
        .filter(|part| !part.is_empty())
        .map(format_brand_segment)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");

    (!formatted.is_empty()).then_some(formatted)
}

fn title_case_segment(input: &str) -> String {
    let mut chars = input.chars();
    match chars.next() {
        Some(first) => {
            let mut label = first.to_uppercase().to_string();
            label.push_str(chars.as_str());
            label
        }
        None => String::new(),
    }
}

fn format_brand_segment(input: &str) -> String {
    match input.to_ascii_lowercase().as_str() {
        "github" => "GitHub".to_string(),
        "youtube" => "YouTube".to_string(),
        "linkedin" => "LinkedIn".to_string(),
        _ => title_case_segment(input),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        decode_bytes_with_charset, decode_bytes_with_declared_charset, decode_html_entities,
        decode_mime_encoded_words, decode_rfc2231_segments, decode_rfc2231_value,
        extract_email_address, extract_email_addresses, extract_mailboxes, fallback_display_name,
        mailbox_local_display_name, mailbox_matches, parse_address_list_tokens,
        repair_likely_utf8_mojibake, resolve_sender_display_name, AddressListDecodeStrategy,
    };

    #[test]
    fn mailbox_comparison_folds_the_domain_but_not_the_local_part() {
        assert!(mailbox_matches("board@Firma.DE", "board@firma.de"));
        assert!(!mailbox_matches("Board@firma.de", "board@firma.de"));
        assert!(!mailbox_matches(
            "board@firma.de",
            "board@firma.de.evil.test"
        ));
        assert!(!mailbox_matches("board@firma.de", "board"));
        assert!(!mailbox_matches("", ""));
    }

    #[test]
    fn address_list_tokens_deduplicate_and_fallback_without_dropping_items() {
        assert_eq!(
            parse_address_list_tokens(
                "Alice <alice@example.test>, alice@example.test, opaque, ,",
                AddressListDecodeStrategy::AlreadyDecoded,
            ),
            vec!["alice@example.test".to_string(), "opaque".to_string()]
        );
    }

    #[test]
    fn address_list_tokens_keep_the_provider_decoder_strategy_explicit() {
        let encoded = "=?UTF-8?Q?J=C3=B6rg?=";
        assert_eq!(
            parse_address_list_tokens(encoded, AddressListDecodeStrategy::MimeEncodedWords),
            vec!["Jörg".to_string()]
        );
        assert_eq!(
            parse_address_list_tokens(encoded, AddressListDecodeStrategy::AlreadyDecoded),
            vec![encoded.to_string()]
        );
    }

    #[test]
    fn decodes_utf8_base64_header() {
        let value = "=?UTF-8?B?QVc6IE1lc3NkYXRlbg==?=";
        assert_eq!(decode_mime_encoded_words(value), "AW: Messdaten");
    }

    #[test]
    fn decodes_utf8_q_header() {
        let value = "=?UTF-8?Q?Termin_heute=2Fgleich?=";
        assert_eq!(decode_mime_encoded_words(value), "Termin heute/gleich");
    }

    #[test]
    fn decodes_q_header_whose_payload_starts_with_hex_escape() {
        let value = "=?UTF-8?Q?=5BWichtige_Informationen=5D=C2=A0Deine_App_ist_von_=C3=84nderun?=\r\n =?UTF-8?Q?gen_an_den_Google_Play_Billing_Library-Anforderungen_betroffen?=";
        assert_eq!(
            decode_mime_encoded_words(value),
            "[Wichtige Informationen]\u{a0}Deine App ist von Änderungen an den Google Play Billing Library-Anforderungen betroffen"
        );
    }

    #[test]
    fn decodes_iso_8859_1_q_header() {
        assert_eq!(
            decode_mime_encoded_words("=?ISO-8859-1?Q?Gr=FC=DFe_aus_M=FCnchen?="),
            "Grüße aus München"
        );
    }

    #[test]
    fn decodes_lowercase_base64_header_with_padding() {
        assert_eq!(decode_mime_encoded_words("=?utf-8?b?w4Q=?="), "Ä");
    }

    #[test]
    fn decodes_header_with_rfc2231_language_tag() {
        assert_eq!(
            decode_mime_encoded_words("=?UTF-8*de?Q?Stra=C3=9Fe?="),
            "Straße"
        );
    }

    #[test]
    fn decodes_adjacent_words_with_mixed_charsets() {
        assert_eq!(
            decode_mime_encoded_words("=?ISO-8859-1?Q?Gr=FC=DFe?= =?UTF-8?Q?_und_T=C3=BCcken?="),
            "Grüße und Tücken"
        );
    }

    #[test]
    fn falls_back_to_utf8_for_unknown_charset() {
        assert_eq!(decode_mime_encoded_words("=?X-UNKNOWN?Q?=C3=84?="), "Ä");
    }

    #[test]
    fn keeps_malformed_encoded_words_verbatim() {
        assert_eq!(
            decode_mime_encoded_words("=?UTF-8?X?abc?="),
            "=?UTF-8?X?abc?="
        );
        assert_eq!(decode_mime_encoded_words("=?UTF-8?Q?abc"), "=?UTF-8?Q?abc");
    }

    #[test]
    fn decodes_html_entities_in_preview_text() {
        let value = "We hope you&#39;re enjoying Spark &amp; Satchel";
        assert_eq!(
            decode_html_entities(value),
            "We hope you're enjoying Spark & Satchel"
        );
    }

    #[test]
    fn decodes_hex_html_entities_in_preview_text() {
        let value = "Tom &quot;liked&quot; it &#x1F44D;";
        assert_eq!(decode_html_entities(value), "Tom \"liked\" it 👍");
    }

    #[test]
    fn extracts_email_from_display_name() {
        let value = "Max Mustermann <max@example.com>";
        assert_eq!(
            extract_email_address(value),
            Some("max@example.com".to_string())
        );
    }

    #[test]
    fn keeps_every_address_of_an_entry_that_holds_two() {
        assert_eq!(
            extract_email_addresses("a@x.de b@y.de"),
            vec!["a@x.de".to_string(), "b@y.de".to_string()]
        );
    }

    #[test]
    fn reads_a_display_name_with_a_comma_as_one_address() {
        assert_eq!(
            extract_email_addresses("\"Morgan, Alex\" <a.morgan@example.com>"),
            vec!["a.morgan@example.com".to_string()]
        );
    }

    #[test]
    fn ignores_an_address_shaped_display_name_when_brackets_are_present() {
        assert_eq!(
            extract_email_addresses("a@x.de <b@y.de>"),
            vec!["b@y.de".to_string()]
        );
    }

    #[test]
    fn keeps_display_names_beside_the_same_addresses() {
        let raw = "\"Meier, Anna\" <a.meier@kunde.de>, b@y.de; Anna Müller <m@x.de>";

        assert_eq!(
            extract_mailboxes(raw),
            vec![
                (
                    Some("Meier, Anna".to_string()),
                    "a.meier@kunde.de".to_string()
                ),
                (None, "b@y.de".to_string()),
                (Some("Anna Müller".to_string()), "m@x.de".to_string()),
            ]
        );
        let addresses: Vec<String> = extract_mailboxes(raw).into_iter().map(|(_, a)| a).collect();
        assert_eq!(addresses, ["a.meier@kunde.de", "b@y.de", "m@x.de"]);
        assert_eq!(
            extract_mailboxes("\"Say \\\"hi\\\"\" <h@x.de>"),
            vec![(Some("Say \"hi\"".to_string()), "h@x.de".to_string())]
        );
        assert_eq!(
            extract_mailboxes("a@x.de <a@x.de>"),
            vec![(None, "a@x.de".to_string())]
        );
    }

    #[test]
    fn reports_no_address_for_a_fragment() {
        assert!(extract_email_addresses("\"Buck").is_empty());
        assert!(extract_email_addresses("   ").is_empty());
    }

    #[test]
    fn formats_local_part_as_display_name() {
        assert_eq!(
            mailbox_local_display_name("no-reply@example.com"),
            "No Reply"
        );
    }

    #[test]
    fn falls_back_to_domain_brand_for_generic_mailboxes() {
        assert_eq!(fallback_display_name("notifications@github.com"), "GitHub");
        assert_eq!(fallback_display_name("no-reply@amazon.de"), "Amazon");
    }

    #[test]
    fn prefers_specific_sender_name_over_generic_mailbox_name() {
        assert_eq!(
            resolve_sender_display_name(Some("Amazon Spar Abo"), "no-reply@amazon.de"),
            "Amazon Spar Abo"
        );
        assert_eq!(
            resolve_sender_display_name(Some("Notifications"), "notifications@github.com"),
            "GitHub"
        );
    }

    #[test]
    fn decodes_rfc2231_percent_encoded_filename() {
        assert_eq!(
            decode_rfc2231_value("iso-8859-1''Gr%FC%DFe.txt"),
            "Grüße.txt"
        );
    }

    #[test]
    fn decodes_rfc2231_split_filename_segments() {
        let segments = vec![
            (1, true, "%DFe.txt".to_string()),
            (0, true, "iso-8859-1''Gr%FC".to_string()),
        ];
        assert_eq!(
            decode_rfc2231_segments(&segments),
            Some("Grüße.txt".to_string())
        );
    }

    #[test]
    fn falls_back_to_windows_1252_for_invalid_utf8_body() {
        assert_eq!(
            decode_bytes_with_charset(b"Freundliche Gr\xFC\xDFe", "utf-8"),
            "Freundliche Grüße"
        );
    }

    #[test]
    fn declared_charset_decoder_reports_invalid_utf8_without_guessing_windows_1252() {
        let (decoded, had_errors) =
            decode_bytes_with_declared_charset(b"Freundliche Gr\xFC\xDFe", "utf-8").unwrap();

        assert!(had_errors);
        assert_eq!(decoded, "Freundliche Gr��e");
    }

    #[test]
    fn declared_charset_decoder_rejects_unknown_labels() {
        assert!(decode_bytes_with_declared_charset(b"hello", "x-made-up-charset").is_err());
    }

    #[test]
    fn repairs_utf8_bytes_misdeclared_as_windows_1252() {
        assert_eq!(
            decode_bytes_with_charset("Überprüfen Sie Ihre Ratentypen".as_bytes(), "windows-1252"),
            "Überprüfen Sie Ihre Ratentypen"
        );
    }

    #[test]
    fn repairs_stored_utf8_mojibake() {
        assert_eq!(
            repair_likely_utf8_mojibake("ÃœberprÃ¼fen Sie Ihre Ratentypen"),
            "Überprüfen Sie Ihre Ratentypen"
        );
    }

    #[test]
    fn repairs_mojibake_without_dropping_other_unicode() {
        let input = "Dear customer,\nWeâ€™re writing to inform you.\nMattermost Server â€” patch release updates ✅";
        assert_eq!(
            repair_likely_utf8_mojibake(input),
            "Dear customer,\nWe’re writing to inform you.\nMattermost Server — patch release updates ✅"
        );
    }
}
