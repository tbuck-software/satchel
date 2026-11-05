//! Turns one message into the set of tokens the classifier scores.
//!
//! Tokens are namespaced strings (`from-domain:example.com`, `subj:invoice`,
//! `auth:dkim=fail`, plain body words) hashed to 64 bits. The model stores the
//! hash only: it keeps the table an integer-keyed b-tree, and a training ledger
//! row is a compact blob instead of a word list. At the sizes the model is
//! bounded to, a 64-bit collision is not a practical concern.
//!
//! The hash is FNV-1a and must never change: every stored count is keyed by it.

use std::collections::HashSet;

/// A message is scored by at most this many distinct tokens. Header tokens are
/// produced first, so a huge body cannot crowd them out.
pub const MAX_TOKENS_PER_MESSAGE: usize = 1_000;

/// Body text beyond this many bytes adds nothing a spam filter needs and costs
/// time on every newsletter.
const MAX_BODY_BYTES: usize = 64 * 1024;

const MIN_WORD_CHARS: usize = 3;
const MAX_WORD_CHARS: usize = 24;

/// The headers, beside the stored columns, that carry spam evidence. Only these
/// are kept from the raw header block at ingest.
pub const HINT_HEADERS: &[&str] = &[
    "x-spam-flag",
    "x-spam-status",
    "x-spam-level",
    "x-spam-score",
    "x-mailer",
    "user-agent",
    "list-id",
    "precedence",
    "content-type",
];

/// Everything the tokenizer reads. Built either from a provider envelope at
/// ingest or from a stored `messages` row when training later.
#[derive(Debug, Clone, Default)]
pub struct SpamFeatures {
    pub account_id: String,
    pub from_address: String,
    pub from_name: Option<String>,
    pub reply_to: Option<String>,
    pub subject: String,
    pub body_text: Option<String>,
    pub body_html: Option<String>,
    pub recipient_count: usize,
    pub has_attachments: bool,
    pub has_list_unsubscribe: bool,
    pub auth_spf: Option<String>,
    pub auth_dkim: Option<String>,
    pub auth_dmarc: Option<String>,
    /// `Name: value` lines for [`HINT_HEADERS`], as kept at ingest.
    pub hint_headers: Option<String>,
}

/// Stable 64-bit FNV-1a, stored as SQLite's signed INTEGER.
pub fn token_hash(token: &str) -> i64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET;
    for byte in token.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(PRIME);
    }
    hash as i64
}

/// Little-endian i64s, the form a training ledger row keeps.
pub fn encode_tokens(tokens: &[i64]) -> Vec<u8> {
    tokens
        .iter()
        .flat_map(|token| token.to_le_bytes())
        .collect()
}

pub fn decode_tokens(blob: &[u8]) -> Vec<i64> {
    blob.chunks_exact(8)
        .map(|chunk| {
            let mut bytes = [0_u8; 8];
            bytes.copy_from_slice(chunk);
            i64::from_le_bytes(bytes)
        })
        .collect()
}

/// Keep only the hint headers from a raw header block, unfolded, one per line.
pub fn extract_hint_headers(raw: &str) -> Option<String> {
    let mut kept = Vec::new();
    let mut current: Option<String> = None;
    let flush = |current: &mut Option<String>, kept: &mut Vec<String>| {
        if let Some(line) = current.take() {
            let name = line
                .split_once(':')
                .map(|(name, _)| name.trim().to_ascii_lowercase());
            if name.is_some_and(|name| HINT_HEADERS.contains(&name.as_str())) {
                kept.push(line);
            }
        }
    };
    for line in raw.lines() {
        if line.is_empty() {
            break;
        }
        if line.starts_with([' ', '\t']) {
            if let Some(current) = current.as_mut() {
                current.push(' ');
                current.push_str(line.trim());
            }
            continue;
        }
        flush(&mut current, &mut kept);
        current = Some(line.trim_end().to_string());
    }
    flush(&mut current, &mut kept);
    (!kept.is_empty()).then(|| kept.join("\n"))
}

struct TokenSet {
    seen: HashSet<i64>,
    ordered: Vec<i64>,
}

impl TokenSet {
    fn new() -> Self {
        Self {
            seen: HashSet::new(),
            ordered: Vec::new(),
        }
    }

    fn push(&mut self, token: impl AsRef<str>) {
        if self.ordered.len() >= MAX_TOKENS_PER_MESSAGE {
            return;
        }
        let hash = token_hash(token.as_ref());
        if self.seen.insert(hash) {
            self.ordered.push(hash);
        }
    }

    fn full(&self) -> bool {
        self.ordered.len() >= MAX_TOKENS_PER_MESSAGE
    }
}

/// The distinct tokens of one message, in a stable order.
pub fn tokenize(features: &SpamFeatures) -> Vec<i64> {
    let mut tokens = TokenSet::new();
    for token in header_tokens(features) {
        tokens.push(token);
    }
    let body = body_source(features);
    for url_token in url_tokens(&body) {
        tokens.push(url_token);
    }
    for word in words(&body) {
        if tokens.full() {
            break;
        }
        tokens.push(word);
    }
    tokens.ordered
}

/// The readable tokens before hashing. Tests and diagnostics read these.
pub fn header_tokens(features: &SpamFeatures) -> Vec<String> {
    let mut out = Vec::new();
    if !features.account_id.trim().is_empty() {
        out.push(format!("acct:{}", features.account_id.trim()));
    }
    let from = features.from_address.trim().to_lowercase();
    let from_domain = domain_of(&from);
    if !from.is_empty() {
        out.push(format!("from:{from}"));
    }
    if let Some(domain) = from_domain.as_deref() {
        out.push(format!("from-domain:{domain}"));
        if let Some(base) = registrable(domain) {
            if base != domain {
                out.push(format!("from-domain:{base}"));
            }
        }
        if domain.chars().filter(char::is_ascii_digit).count() >= 3 {
            out.push("from-domain-digits".into());
        }
    } else {
        out.push("from:none".into());
    }
    if let Some(name) = features.from_name.as_deref() {
        for word in words(name) {
            out.push(format!("from-name:{word}"));
        }
        if name.contains('@') {
            out.push("from-name-has-address".into());
        }
    }
    if let Some(reply_to) = features.reply_to.as_deref() {
        let reply_to = reply_to.trim().to_lowercase();
        if let Some(domain) = domain_of(&reply_to) {
            out.push(format!("reply-to-domain:{domain}"));
            if from_domain.as_deref() != Some(domain.as_str()) {
                out.push("reply-to-differs".into());
            }
        }
    }
    let subject = features.subject.trim();
    if subject.is_empty() {
        out.push("subj:empty".into());
    }
    for word in words(subject) {
        out.push(format!("subj:{word}"));
    }
    let letters = subject.chars().filter(|c| c.is_alphabetic()).count();
    let upper = subject.chars().filter(|c| c.is_uppercase()).count();
    if letters >= 8 && upper * 10 >= letters * 6 {
        out.push("subj:shouting".into());
    }
    if subject.contains('!') {
        out.push("subj:exclamation".into());
    }
    if subject.contains('$') || subject.contains('€') {
        out.push("subj:currency".into());
    }
    for (name, value) in [
        ("spf", features.auth_spf.as_deref()),
        ("dkim", features.auth_dkim.as_deref()),
        ("dmarc", features.auth_dmarc.as_deref()),
    ] {
        let value = value.map(str::trim).filter(|value| !value.is_empty());
        out.push(format!(
            "auth:{name}={}",
            value
                .map(str::to_ascii_lowercase)
                .as_deref()
                .unwrap_or("none")
        ));
    }
    out.push(format!(
        "rcpt:{}",
        match features.recipient_count {
            0 => "0",
            1 => "1",
            2..=5 => "few",
            _ => "many",
        }
    ));
    if features.has_attachments {
        out.push("has-attachment".into());
    }
    if features.has_list_unsubscribe {
        out.push("has-list-unsubscribe".into());
    }
    let has_text = features
        .body_text
        .as_deref()
        .is_some_and(|text| !text.trim().is_empty());
    let html = features.body_html.as_deref().unwrap_or("");
    if !html.is_empty() {
        out.push(
            if has_text {
                "html:alternative"
            } else {
                "html:only"
            }
            .into(),
        );
        let lower = html.to_ascii_lowercase();
        if lower.contains("<img") {
            out.push("html:img".into());
        }
        if lower.contains("display:none") || lower.contains("display: none") {
            out.push("html:hidden-text".into());
        }
    }
    if let Some(headers) = features.hint_headers.as_deref() {
        out.extend(hint_tokens(headers));
    }
    out
}

fn hint_tokens(headers: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in headers.lines() {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim().to_ascii_lowercase();
        let value = value.trim();
        let lower = value.to_ascii_lowercase();
        match name.as_str() {
            "x-spam-flag" => out.push(format!(
                "hdr:x-spam-flag:{}",
                lower.split_whitespace().next().unwrap_or("none")
            )),
            "x-spam-status" => {
                let verdict = lower
                    .split([',', ' '])
                    .find(|part| !part.is_empty())
                    .unwrap_or("none");
                out.push(format!("hdr:x-spam-status:{verdict}"));
                // SpamAssassin names the rules that fired: `tests=A,B,C`.
                if let Some(start) = lower.find("tests=") {
                    let tests = &lower[start + "tests=".len()..];
                    for test in tests
                        .split([',', ' '])
                        .take_while(|part| !part.contains('='))
                        .filter(|part| !part.is_empty())
                        .take(32)
                    {
                        out.push(format!("hdr:x-spam-test:{test}"));
                    }
                }
            }
            "x-spam-level" => {
                let stars = value.chars().filter(|c| *c == '*').count();
                out.push(format!("hdr:x-spam-level:{}", stars.min(10)));
            }
            "x-spam-score" => {
                if let Ok(score) = lower.split_whitespace().next().unwrap_or("").parse::<f64>() {
                    let bucket = score.floor().clamp(-5.0, 15.0) as i64;
                    out.push(format!("hdr:x-spam-score:{bucket}"));
                }
            }
            "x-mailer" | "user-agent" => {
                if let Some(word) = words(value).into_iter().next() {
                    out.push(format!("hdr:mailer:{word}"));
                }
            }
            "list-id" => {
                let id = lower
                    .rsplit_once('<')
                    .map(|(_, id)| id.trim_end_matches('>'))
                    .unwrap_or(&lower);
                out.push(format!("hdr:list-id:{}", id.trim()));
            }
            "precedence" => out.push(format!("hdr:precedence:{}", lower.trim())),
            "content-type" => {
                let kind = lower.split(';').next().unwrap_or("").trim();
                if !kind.is_empty() {
                    out.push(format!("hdr:content-type:{kind}"));
                }
            }
            _ => {}
        }
    }
    out
}

/// Plain text to read words from: the text part, else the HTML without tags.
fn body_source(features: &SpamFeatures) -> String {
    let text = features
        .body_text
        .as_deref()
        .filter(|text| !text.trim().is_empty());
    let source = match text {
        Some(text) => text.to_string(),
        None => strip_tags(features.body_html.as_deref().unwrap_or("")),
    };
    truncate_at_char_boundary(source, MAX_BODY_BYTES)
}

fn truncate_at_char_boundary(mut value: String, max: usize) -> String {
    if value.len() > max {
        let mut cut = max;
        while !value.is_char_boundary(cut) {
            cut -= 1;
        }
        value.truncate(cut);
    }
    value
}

/// Drop tags, `<style>`/`<script>` contents and entities; keep `href` targets
/// so URL tokens still see the links.
fn strip_tags(html: &str) -> String {
    let html = truncate_at_char_boundary(html.to_string(), MAX_BODY_BYTES * 2);
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len() / 2);
    let mut index = 0;
    while index < html.len() {
        let rest = &html[index..];
        if rest.starts_with('<') {
            let lower_rest = &lower[index..];
            for skipped in ["style", "script"] {
                if lower_rest.starts_with(&format!("<{skipped}")) {
                    if let Some(end) = lower_rest.find(&format!("</{skipped}")) {
                        index += end;
                    }
                }
            }
            let rest = &html[index..];
            let end = rest.find('>').map(|end| end + 1).unwrap_or(rest.len());
            let tag = &rest[..end];
            if let Some(start) = tag.to_ascii_lowercase().find("href=") {
                let target = tag[start + 5..]
                    .trim_start_matches(['"', '\''])
                    .split(['"', '\'', ' ', '>'])
                    .next()
                    .unwrap_or("");
                out.push(' ');
                out.push_str(target);
            }
            out.push(' ');
            index += end;
        } else if rest.starts_with('&') {
            let end = rest
                .find(';')
                .filter(|end| *end <= 10)
                .map(|end| end + 1)
                .unwrap_or(1);
            out.push(' ');
            index += end;
        } else {
            let next = rest.find(['<', '&']).unwrap_or(rest.len());
            out.push_str(&rest[..next]);
            index += next;
        }
    }
    out
}

fn url_tokens(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let lower = body.to_ascii_lowercase();
    let mut search = 0;
    while let Some(found) = lower[search..].find("http") {
        let start = search + found;
        let rest = &lower[start..];
        let after_scheme = rest
            .strip_prefix("https://")
            .or_else(|| rest.strip_prefix("http://"));
        search = start + 4;
        let Some(after_scheme) = after_scheme else {
            continue;
        };
        let host = after_scheme
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '.' || c == '-'))
            .next()
            .unwrap_or("")
            .trim_matches('.');
        if host.is_empty() {
            continue;
        }
        if host.chars().all(|c| c.is_ascii_digit() || c == '.') {
            out.push("url:ip-address".into());
        } else {
            out.push(format!("url:{host}"));
            if let Some(base) = registrable(host) {
                if base != host {
                    out.push(format!("url:{base}"));
                }
            }
        }
        if out.len() >= 64 {
            break;
        }
    }
    out
}

/// Lowercase words between 3 and 24 characters. Longer runs collapse into a
/// length class, the way SpamBayes keeps base64 and tracking ids from
/// flooding the model with one-off tokens.
fn words(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for raw in text.split(|c: char| !(c.is_alphanumeric() || c == '\'' || c == '-')) {
        let word = raw.trim_matches(['\'', '-']);
        let count = word.chars().count();
        if count < MIN_WORD_CHARS {
            continue;
        }
        if word.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        if count > MAX_WORD_CHARS {
            let first = word.chars().next().unwrap_or('_').to_lowercase();
            out.push(format!("skip:{first}:{}", count / 10 * 10));
            continue;
        }
        out.push(word.to_lowercase());
    }
    out
}

fn domain_of(address: &str) -> Option<String> {
    let address = address.trim().trim_matches(['<', '>']);
    let (_, domain) = address.rsplit_once('@')?;
    let domain = domain.trim().trim_end_matches('>').to_ascii_lowercase();
    (!domain.is_empty()).then_some(domain)
}

/// The last two labels: `mail.shop.example.com` → `example.com`. Not a public
/// suffix list — close enough for a statistical feature.
fn registrable(domain: &str) -> Option<String> {
    let labels: Vec<&str> = domain.split('.').filter(|part| !part.is_empty()).collect();
    (labels.len() >= 2).then(|| labels[labels.len() - 2..].join("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn features() -> SpamFeatures {
        SpamFeatures {
            account_id: "acc".into(),
            from_address: "Deals@Mail.Shop.Example.com".into(),
            from_name: Some("Mega Deals".into()),
            subject: "WIN A FREE CRUISE NOW!".into(),
            body_text: Some("Click https://track.example.net/x?id=1 to claim your prize".into()),
            auth_dkim: Some("fail".into()),
            ..SpamFeatures::default()
        }
    }

    #[test]
    fn header_tokens_are_namespaced_and_normalized() {
        let tokens = header_tokens(&features());
        for expected in [
            "acct:acc",
            "from:deals@mail.shop.example.com",
            "from-domain:mail.shop.example.com",
            "from-domain:example.com",
            "from-name:mega",
            "subj:cruise",
            "subj:shouting",
            "subj:exclamation",
            "auth:dkim=fail",
            "auth:spf=none",
            "rcpt:0",
        ] {
            assert!(
                tokens.iter().any(|token| token == expected),
                "{expected} in {tokens:?}"
            );
        }
    }

    #[test]
    fn tokenize_dedupes_and_caps() {
        let mut long = features();
        long.body_text = Some(
            (0..5_000)
                .map(|n| format!("word{n}x"))
                .collect::<Vec<_>>()
                .join(" "),
        );
        let tokens = tokenize(&long);
        assert_eq!(tokens.len(), MAX_TOKENS_PER_MESSAGE);
        let unique: HashSet<_> = tokens.iter().collect();
        assert_eq!(unique.len(), tokens.len());
        // Headers come first and survive the cap.
        assert_eq!(tokens[0], token_hash("acct:acc"));
    }

    #[test]
    fn urls_and_html_feed_the_tokens() {
        let html = SpamFeatures {
            body_html: Some(
                "<style>.x{color:red}</style><p>Hello&nbsp;there <a href=\"https://evil.example.org/p\">here</a><img src=x></p>"
                    .into(),
            ),
            ..SpamFeatures::default()
        };
        let headers = header_tokens(&html);
        assert!(headers.contains(&"html:only".to_string()));
        assert!(headers.contains(&"html:img".to_string()));
        let tokens = tokenize(&html);
        assert!(tokens.contains(&token_hash("url:evil.example.org")));
        assert!(tokens.contains(&token_hash("url:example.org")));
        assert!(tokens.contains(&token_hash("hello")));
        assert!(!tokens.contains(&token_hash("color")));
    }

    #[test]
    fn hint_headers_are_extracted_unfolded_and_tokenized() {
        let raw = "From: a@b.c\r\nX-Spam-Status: Yes, score=7.1 required=5.0\r\n\ttests=BAYES_99,URIBL_BLACK autolearn=no\r\nX-Spam-Flag: YES\r\nSubject: hi\r\n\r\nbody X-Spam-Flag: NO";
        let kept = extract_hint_headers(raw).expect("hint headers");
        assert_eq!(kept.lines().count(), 2);
        let tokens = hint_tokens(&kept);
        assert!(tokens.contains(&"hdr:x-spam-flag:yes".to_string()));
        assert!(tokens.contains(&"hdr:x-spam-status:yes".to_string()));
        assert!(tokens.contains(&"hdr:x-spam-test:bayes_99".to_string()));
        assert!(tokens.contains(&"hdr:x-spam-test:uribl_black".to_string()));
        assert!(!tokens.iter().any(|token| token.contains("autolearn")));
    }

    #[test]
    fn blob_round_trips_and_hash_is_stable() {
        let tokens = vec![1, -1, i64::MAX, i64::MIN, 42];
        assert_eq!(decode_tokens(&encode_tokens(&tokens)), tokens);
        // FNV-1a of "" and "a" — pinned, because stored counts depend on them.
        assert_eq!(token_hash("") as u64, 0xcbf2_9ce4_8422_2325);
        assert_eq!(token_hash("a") as u64, 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn long_runs_collapse_into_length_classes() {
        let tokens = words("aGVsbG8gd29ybGQgdGhpcyBpcyBiYXNlNjQ= ok fine");
        assert_eq!(tokens, vec!["skip:a:30".to_string(), "fine".to_string()]);
    }
}
