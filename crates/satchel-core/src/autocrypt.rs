//! Autocrypt Level 1: the `Autocrypt:` header, peer state and the
//! encryption recommendation.
//!
//! Spec: <https://docs.autocrypt.org/level1.html>. Section names in the doc
//! comments below are that page's headings. Gossip (`Autocrypt-Gossip`) is not
//! implemented: it only ever feeds the "discourage" path.
use base64::Engine as _;
use chrono::{DateTime, Duration, Utc};

/// "MUST NOT send an Autocrypt header above 10 KiB in size"; receivers
/// "SHOULD treat such headers as invalid".
pub const MAX_HEADER_BYTES: usize = 10 * 1024;

/// A peer whose last Autocrypt header is this much older than its last mail is
/// only "discourage" ("Recommendation", step 4).
const STALE_AFTER_DAYS: i64 = 35;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreferEncrypt {
    Mutual,
    NoPreference,
}

impl PreferEncrypt {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mutual => "mutual",
            Self::NoPreference => "nopreference",
        }
    }

    pub fn parse(value: &str) -> Self {
        if value.trim().eq_ignore_ascii_case("mutual") {
            Self::Mutual
        } else {
            Self::NoPreference
        }
    }
}

/// One valid `Autocrypt:` header ("The Autocrypt Header").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutocryptHeader {
    pub addr: String,
    pub prefer_encrypt: PreferEncrypt,
    /// Binary Transferable Public Key.
    pub keydata: Vec<u8>,
}

/// Canonical form of an address for peer lookup.
pub fn canonical_addr(addr: &str) -> String {
    addr.trim().to_ascii_lowercase()
}

/// Parse one header value. `None` for anything the spec calls invalid: no
/// `addr` or `keydata`, `keydata` not last, an unknown critical attribute,
/// undecodable key data, or a header over 10 KiB.
pub fn parse_header(value: &str) -> Option<AutocryptHeader> {
    if value.len() > MAX_HEADER_BYTES {
        return None;
    }
    let mut addr = None;
    let mut prefer_encrypt = PreferEncrypt::NoPreference;
    let mut keydata = None;
    for attribute in value.split(';').map(str::trim).filter(|a| !a.is_empty()) {
        // "the `keydata` attribute MUST be the last attribute in this header".
        if keydata.is_some() {
            return None;
        }
        let (name, attribute_value) = attribute.split_once('=')?;
        let name = name.trim().to_ascii_lowercase();
        let attribute_value = attribute_value.trim();
        match name.as_str() {
            "addr" => addr = Some(canonical_addr(attribute_value)),
            "prefer-encrypt" => prefer_encrypt = PreferEncrypt::parse(attribute_value),
            "keydata" => {
                let compact: String = attribute_value
                    .chars()
                    .filter(|c| !c.is_whitespace())
                    .collect();
                keydata = Some(
                    base64::engine::general_purpose::STANDARD
                        .decode(compact)
                        .ok()?,
                );
            }
            // Non-critical: "SHOULD ignore any unsupported non-critical attributes".
            other if other.starts_with('_') => {}
            // Critical and unknown: the "entire `Autocrypt` header" is invalid.
            _ => return None,
        }
    }
    let addr = addr.filter(|addr| addr.contains('@'))?;
    let keydata = keydata.filter(|data| !data.is_empty())?;
    Some(AutocryptHeader {
        addr,
        prefer_encrypt,
        keydata,
    })
}

/// The one header a message speaks with ("Updating Autocrypt Peer State"):
/// headers whose `addr` differs from the From address are invalid, and "if
/// more than one valid header exists, this SHOULD be treated as an error, and
/// all `Autocrypt` headers discarded as invalid".
pub fn select_header(values: &[String], from_addr: &str) -> Option<AutocryptHeader> {
    let from = canonical_addr(from_addr);
    let mut valid = values
        .iter()
        .filter_map(|value| parse_header(value))
        .filter(|header| header.addr == from);
    let first = valid.next()?;
    if valid.next().is_some() {
        return None;
    }
    Some(first)
}

/// Our own header ("Header injection in outbound mail"): `addr`, then
/// `prefer-encrypt=mutual` when we prefer it, then `keydata` last. Key data is
/// split into space-separated runs so the header can fold; whitespace inside
/// `keydata` is not part of the value.
pub fn format_header(addr: &str, prefer_encrypt: PreferEncrypt, keydata: &[u8]) -> String {
    let encoded = base64::engine::general_purpose::STANDARD.encode(keydata);
    let folded = encoded
        .as_bytes()
        .chunks(76)
        .map(|chunk| std::str::from_utf8(chunk).unwrap_or_default())
        .collect::<Vec<_>>()
        .join(" ");
    match prefer_encrypt {
        PreferEncrypt::Mutual => {
            format!(
                "addr={}; prefer-encrypt=mutual; keydata={folded}",
                canonical_addr(addr)
            )
        }
        PreferEncrypt::NoPreference => format!("addr={}; keydata={folded}", canonical_addr(addr)),
    }
}

/// What Satchel knows about one peer ("Peers").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerState {
    pub last_seen: DateTime<Utc>,
    pub autocrypt_timestamp: Option<DateTime<Utc>>,
    /// Armored certificate from the peer's latest valid header.
    pub public_key: Option<String>,
    pub fingerprint: Option<String>,
    pub prefer_encrypt: PreferEncrypt,
}

/// A header's key, already checked and armored by the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerKey {
    pub armored: String,
    pub fingerprint: String,
    pub prefer_encrypt: PreferEncrypt,
}

/// "the `Date` header, or the time of receipt if that date is in the future
/// or unavailable".
pub fn effective_date(date: Option<DateTime<Utc>>, received: DateTime<Utc>) -> DateTime<Utc> {
    match date {
        Some(date) if date <= received => date,
        _ => received,
    }
}

/// "Updating Autocrypt Peer State", steps 1–6. `None` in, `None` out when the
/// message has no header: a peer that never sent one has no state worth
/// keeping, and its recommendation is "disable" either way.
pub fn update_peer(
    state: Option<PeerState>,
    effective: DateTime<Utc>,
    header: Option<PeerKey>,
) -> Option<PeerState> {
    let Some(mut state) = state else {
        let key = header?;
        return Some(PeerState {
            last_seen: effective,
            autocrypt_timestamp: Some(effective),
            public_key: Some(key.armored),
            fingerprint: Some(key.fingerprint),
            prefer_encrypt: key.prefer_encrypt,
        });
    };
    // 1. "If the effective date is older than autocrypt_timestamp, stop."
    if state
        .autocrypt_timestamp
        .is_some_and(|timestamp| effective < timestamp)
    {
        return Some(state);
    }
    // 2. Move last_seen forward.
    if effective > state.last_seen {
        state.last_seen = effective;
    }
    // 3. No header: stop.
    let Some(key) = header else {
        return Some(state);
    };
    // 4–6.
    state.autocrypt_timestamp = Some(effective);
    state.public_key = Some(key.armored);
    state.fingerprint = Some(key.fingerprint);
    state.prefer_encrypt = key.prefer_encrypt;
    Some(state)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Recommendation {
    Disable,
    Discourage,
    Available,
    Encrypt,
}

impl Recommendation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disable => "disable",
            Self::Discourage => "discourage",
            Self::Available => "available",
            Self::Encrypt => "encrypt",
        }
    }
}

/// "Recommendation" for one recipient. Keys Satchel could not use never got
/// stored, so a stored key counts as usable; there is no gossip key.
pub fn recommend(
    peer: Option<&PeerState>,
    own_prefer_encrypt: PreferEncrypt,
    reply_to_encrypted: bool,
) -> Recommendation {
    let Some(peer) = peer else {
        return Recommendation::Disable;
    };
    if peer.public_key.is_none() {
        return Recommendation::Disable;
    }
    let stale = peer
        .autocrypt_timestamp
        .is_none_or(|timestamp| peer.last_seen - timestamp > Duration::days(STALE_AFTER_DAYS));
    let preliminary = if stale {
        Recommendation::Discourage
    } else {
        Recommendation::Available
    };
    // "Final recommendation phase", step 5.
    if reply_to_encrypted {
        return Recommendation::Encrypt;
    }
    if preliminary == Recommendation::Available
        && peer.prefer_encrypt == PreferEncrypt::Mutual
        && own_prefer_encrypt == PreferEncrypt::Mutual
    {
        return Recommendation::Encrypt;
    }
    preliminary
}

/// Several recipients: any "disable" disables, all "encrypt" encrypts, any
/// "discourage" discourages, otherwise "available".
pub fn aggregate(recommendations: &[Recommendation]) -> Recommendation {
    if recommendations.is_empty() || recommendations.contains(&Recommendation::Disable) {
        return Recommendation::Disable;
    }
    if recommendations
        .iter()
        .all(|r| *r == Recommendation::Encrypt)
    {
        return Recommendation::Encrypt;
    }
    if recommendations.contains(&Recommendation::Discourage) {
        return Recommendation::Discourage;
    }
    Recommendation::Available
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 3, day, 12, 0, 0).unwrap()
    }

    fn key(name: &str, prefer: PreferEncrypt) -> PeerKey {
        PeerKey {
            armored: format!("key-{name}"),
            fingerprint: format!("FP-{name}"),
            prefer_encrypt: prefer,
        }
    }

    #[test]
    fn parses_a_header_and_ignores_non_critical_attributes() {
        let header =
            parse_header("addr=Lena@Example.test; prefer-encrypt=mutual; _x=1; keydata=AAEC AwQ=")
                .unwrap();
        assert_eq!(header.addr, "lena@example.test");
        assert_eq!(header.prefer_encrypt, PreferEncrypt::Mutual);
        assert_eq!(header.keydata, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn rejects_what_the_spec_calls_invalid() {
        // Unknown critical attribute.
        assert!(parse_header("addr=a@example.test; color=blue; keydata=AAEC").is_none());
        // keydata not last.
        assert!(parse_header("addr=a@example.test; keydata=AAEC; _x=1").is_none());
        // Missing parts.
        assert!(parse_header("addr=a@example.test").is_none());
        assert!(parse_header("keydata=AAEC").is_none());
        // Undecodable or oversized.
        assert!(parse_header("addr=a@example.test; keydata=***").is_none());
        let big = format!(
            "addr=a@example.test; keydata={}",
            "A".repeat(MAX_HEADER_BYTES)
        );
        assert!(parse_header(&big).is_none());
        // An unknown prefer-encrypt value is "nopreference", not an error.
        let header = parse_header("addr=a@example.test; prefer-encrypt=yes; keydata=AAEC").unwrap();
        assert_eq!(header.prefer_encrypt, PreferEncrypt::NoPreference);
    }

    #[test]
    fn select_header_needs_exactly_one_header_for_the_sender() {
        let lena = "addr=lena@example.test; keydata=AAEC".to_string();
        let other = "addr=other@example.test; keydata=AAEC".to_string();
        assert!(select_header(std::slice::from_ref(&lena), "LENA@example.test").is_some());
        assert!(select_header(std::slice::from_ref(&other), "lena@example.test").is_none());
        assert!(select_header(&[lena.clone(), other], "lena@example.test").is_some());
        assert!(select_header(&[lena.clone(), lena], "lena@example.test").is_none());
    }

    #[test]
    fn format_header_round_trips_and_puts_keydata_last() {
        let keydata: Vec<u8> = (0..200u8).collect();
        let value = format_header("Me@Example.test", PreferEncrypt::Mutual, &keydata);
        assert!(value.starts_with("addr=me@example.test; prefer-encrypt=mutual; keydata="));
        let parsed = parse_header(&value).unwrap();
        assert_eq!(parsed.keydata, keydata);
        assert!(
            !format_header("me@example.test", PreferEncrypt::NoPreference, &keydata)
                .contains("prefer-encrypt")
        );
    }

    #[test]
    fn effective_date_never_lies_in_the_future() {
        assert_eq!(effective_date(Some(at(1)), at(2)), at(1));
        assert_eq!(effective_date(Some(at(3)), at(2)), at(2));
        assert_eq!(effective_date(None, at(2)), at(2));
    }

    #[test]
    fn peer_state_follows_the_update_steps() {
        assert_eq!(update_peer(None, at(1), None), None);

        let first = update_peer(None, at(2), Some(key("a", PreferEncrypt::Mutual))).unwrap();
        assert_eq!(first.public_key.as_deref(), Some("key-a"));
        assert_eq!(first.autocrypt_timestamp, Some(at(2)));

        // An older message changes nothing (step 1).
        let older = update_peer(
            Some(first.clone()),
            at(1),
            Some(key("b", PreferEncrypt::Mutual)),
        );
        assert_eq!(older, Some(first.clone()));

        // A newer one without a header only moves last_seen (steps 2–3).
        let plain = update_peer(Some(first.clone()), at(5), None).unwrap();
        assert_eq!(plain.last_seen, at(5));
        assert_eq!(plain.public_key.as_deref(), Some("key-a"));
        assert_eq!(plain.autocrypt_timestamp, Some(at(2)));

        // A newer header replaces key and preference (steps 4–6).
        let next = update_peer(
            Some(plain),
            at(6),
            Some(key("c", PreferEncrypt::NoPreference)),
        )
        .unwrap();
        assert_eq!(next.public_key.as_deref(), Some("key-c"));
        assert_eq!(next.prefer_encrypt, PreferEncrypt::NoPreference);
        assert_eq!(next.autocrypt_timestamp, Some(at(6)));
    }

    #[test]
    fn recommendation_matches_the_spec() {
        use PreferEncrypt::*;
        use Recommendation::*;
        assert_eq!(recommend(None, Mutual, false), Disable);

        let mutual = update_peer(None, at(1), Some(key("a", Mutual))).unwrap();
        assert_eq!(recommend(Some(&mutual), Mutual, false), Encrypt);
        assert_eq!(recommend(Some(&mutual), NoPreference, false), Available);

        let indifferent = update_peer(None, at(1), Some(key("a", NoPreference))).unwrap();
        assert_eq!(recommend(Some(&indifferent), Mutual, false), Available);
        assert_eq!(recommend(Some(&indifferent), Mutual, true), Encrypt);

        // Mail without Autocrypt for more than 35 days: discourage.
        let mut stale = mutual.clone();
        stale.last_seen = mutual.last_seen + Duration::days(36);
        assert_eq!(recommend(Some(&stale), Mutual, false), Discourage);
        assert_eq!(recommend(Some(&stale), Mutual, true), Encrypt);
    }

    #[test]
    fn several_recipients_aggregate_by_the_earliest_rule() {
        use Recommendation::*;
        assert_eq!(aggregate(&[Encrypt, Disable]), Disable);
        assert_eq!(aggregate(&[Encrypt, Encrypt]), Encrypt);
        assert_eq!(aggregate(&[Encrypt, Discourage, Available]), Discourage);
        assert_eq!(aggregate(&[Encrypt, Available]), Available);
        assert_eq!(aggregate(&[]), Disable);
    }
}
