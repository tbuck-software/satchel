const SOURCE_BODY_MARKER_ATTRIBUTE: &str = "data-satchel-source-body";

/// Schemes a link may still carry after sanitizing.
const LINK_URL_SCHEMES: &[&str] = &["http", "https", "mailto"];
/// Schemes that resolve inside the message or the app, never on the network.
const LOCAL_URL_SCHEMES: &[&str] = &["cid", "data", "blob", "asset"];
/// Schemes an image source may carry.
const IMAGE_URL_SCHEMES: &[&str] = &["http", "https", "cid", "data", "blob", "asset"];

/// Returns the lowercased scheme of a URL reference, or `None` when it carries
/// none. ASCII whitespace and control characters are ignored the way a browser
/// ignores them, so `java&#9;script:` reads as a scheme and not as a path.
fn url_scheme(value: &str) -> Option<String> {
    let mut scheme = String::new();
    for ch in value.chars() {
        if ch.is_ascii_whitespace() || ch.is_ascii_control() {
            continue;
        }
        if ch == ':' {
            return if scheme.is_empty() {
                None
            } else {
                Some(scheme)
            };
        }
        let valid = if scheme.is_empty() {
            ch.is_ascii_alphabetic()
        } else {
            ch.is_ascii_alphanumeric() || matches!(ch, '+' | '-' | '.')
        };
        if !valid {
            return None;
        }
        scheme.push(ch.to_ascii_lowercase());
    }
    None
}

/// A reference without a scheme is relative and has no scheme to abuse, which
/// is also what ammonia does with one today.
fn scheme_is_allowed(value: &str, allowed: &[&str]) -> bool {
    match url_scheme(value) {
        Some(scheme) => allowed.contains(&scheme.as_str()),
        None => true,
    }
}

/// `srcset` is a comma-separated candidate list, and ammonia does not count it
/// as a URL attribute at all — nothing else checks its schemes.
fn srcset_is_allowed(value: &str) -> bool {
    value
        .split(',')
        .filter_map(|candidate| candidate.split_ascii_whitespace().next())
        .all(|url| scheme_is_allowed(url, IMAGE_URL_SCHEMES))
}

/// Whether one `url(...)` payload resolves inside the message. An allowlist,
/// not a denylist: a comment or a stray quote hiding the scheme reads as
/// "not local" and the reference is dropped.
fn css_url_is_local(inner: &str) -> bool {
    let trimmed = inner.trim();
    let unquoted = trimmed
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .or_else(|| {
            trimmed
                .strip_prefix('\'')
                .and_then(|rest| rest.strip_suffix('\''))
        })
        .unwrap_or(trimmed)
        .trim();
    if unquoted.starts_with('#') {
        return true;
    }
    match url_scheme(unquoted) {
        Some(scheme) => LOCAL_URL_SCHEMES.contains(&scheme.as_str()),
        None => false,
    }
}

fn replace_case_insensitive(css: &str, needle: &str, replacement: &str) -> String {
    let haystack = css.to_ascii_lowercase();
    let mut out = String::with_capacity(css.len());
    let mut cursor = 0usize;
    while let Some(found) = haystack[cursor..].find(needle) {
        let start = cursor + found;
        out.push_str(&css[cursor..start]);
        out.push_str(replacement);
        cursor = start + needle.len();
    }
    out.push_str(&css[cursor..]);
    out
}

fn strip_at_imports(css: &str) -> String {
    let haystack = css.to_ascii_lowercase();
    let mut out = String::with_capacity(css.len());
    let mut cursor = 0usize;
    while let Some(found) = haystack[cursor..].find("@import") {
        let start = cursor + found;
        out.push_str(&css[cursor..start]);
        let rest = &haystack[start..];
        cursor = match rest.find([';', '}']) {
            Some(end) if rest.as_bytes()[end] == b'}' => start + end,
            Some(end) => start + end + 1,
            None => css.len(),
        };
    }
    out.push_str(&css[cursor..]);
    out
}

fn neutralize_css_urls(css: &str) -> String {
    let haystack = css.to_ascii_lowercase();
    let mut out = String::with_capacity(css.len());
    let mut cursor = 0usize;
    while let Some(found) = haystack[cursor..].find("url(") {
        let start = cursor + found;
        let open = start + "url(".len();
        let Some(close) = haystack[open..].find(')') else {
            // An unterminated url() swallows the rest of the stylesheet.
            out.push_str(&css[cursor..start]);
            out.push_str("none");
            return out;
        };
        let close = open + close;
        out.push_str(&css[cursor..start]);
        if css_url_is_local(&css[open..close]) {
            out.push_str(&css[start..=close]);
        } else {
            out.push_str("none");
        }
        cursor = close + 1;
    }
    out.push_str(&css[cursor..]);
    out
}

/// Ammonia hands CSS through untouched, so the sender's rules reach the
/// document verbatim. Strip the constructs that turn a stylesheet into a
/// network client or a script host and leave the rest, so layout survives.
fn sanitize_css(css: &str) -> String {
    let css = strip_at_imports(css);
    let css = neutralize_css_urls(&css);
    // Renamed rather than cut out: the declaration stays syntactically whole
    // and the CSS parser drops it, and no replacement contains the token it
    // replaces, so a second pass has nothing left to find.
    let css = replace_case_insensitive(&css, "expression(", "-satchel-blocked-fn(");
    let css = replace_case_insensitive(&css, "behavior:", "-satchel-blocked:");
    replace_case_insensitive(&css, "-moz-binding", "-satchel-blocked-binding")
}

/// Rewrite parsed CSS before ammonia sees the final serialized markup.
fn sanitize_style_blocks(document: &mut scraper::Html) {
    let selector = scraper::Selector::parse("style").expect("static style selector");
    let replacements = document
        .select(&selector)
        .flat_map(|style| {
            style.children().filter_map(|child| {
                child
                    .value()
                    .as_text()
                    .map(|text| (child.id(), sanitize_css(text)))
            })
        })
        .collect::<Vec<_>>();
    for (id, css) in replacements {
        if let Some(mut node) = document.tree.get_mut(id) {
            if let scraper::Node::Text(text) = node.value() {
                text.text = css.into();
            }
        }
    }
}

/// A second look at an attribute the email rules kept: `(element, attribute,
/// kept value)` → the value to write instead, or `None` to keep it.
type AttributeRewrite = Box<dyn Fn(&str, &str, &str) -> Option<String> + Send + Sync>;

fn build_email_html_sanitizer<'a>(allow_source_body_marker: bool) -> ammonia::Builder<'a> {
    build_email_html_sanitizer_rewriting(allow_source_body_marker, None)
}

fn build_email_html_sanitizer_rewriting<'a>(
    allow_source_body_marker: bool,
    rewrite: Option<AttributeRewrite>,
) -> ammonia::Builder<'a> {
    let mut builder = ammonia::Builder::default();
    builder
        // ammonia's defaults carry every table tag but `tfoot`; without it the
        // footer rows of a forwarded table lose their place.
        .add_tags(&["body", "font", "head", "html", "style", "tfoot"])
        .add_clean_content_tags(&["title"])
        .rm_clean_content_tags(&["style"])
        .add_generic_attributes(&[
            "align",
            "background",
            "bgcolor",
            "border",
            "cellpadding",
            "cellspacing",
            "class",
            "color",
            "dir",
            "face",
            "height",
            "id",
            "name",
            "role",
            "style",
            "valign",
            "width",
        ])
        .add_generic_attribute_prefixes(&["aria-", "data-"])
        .add_tag_attributes("a", &["name"])
        .add_tag_attributes("img", &["src", "srcset"])
        .add_tag_attributes("font", &["size"])
        // Ammonia drops an attribute whose scheme is missing here before the
        // filter below ever sees it, so the local schemes have to pass this
        // gate; narrowing them down per attribute is the filter's job.
        .add_url_schemes(&["cid", "data", "blob", "asset"])
        .attribute_filter(move |element, attribute, value| {
            if attribute == SOURCE_BODY_MARKER_ATTRIBUTE && !allow_source_body_marker {
                return None;
            }
            let keep = match (element, attribute) {
                ("img", "src") => scheme_is_allowed(value, IMAGE_URL_SCHEMES),
                ("img", "srcset") => srcset_is_allowed(value),
                (_, "src") | (_, "href") => scheme_is_allowed(value, LINK_URL_SCHEMES),
                _ => true,
            };
            if !keep {
                return None;
            }
            let kept: std::borrow::Cow<'_, str> = if attribute == "style" {
                sanitize_css(value).into()
            } else {
                value.into()
            };
            match rewrite
                .as_ref()
                .and_then(|rewrite| rewrite(element, attribute, &kept))
            {
                Some(replaced) => Some(replaced.into()),
                None => Some(kept),
            }
        })
        .link_rel(Some("noopener noreferrer"));
    builder
}

fn escape_html_attribute(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Ammonia parses fragments in a synthetic div, so document-level body
/// attributes disappear even though the tag itself is allow-listed. Preserve
/// the two presentation attributes the frontend deliberately carries onto its
/// isolated wrapper. The marker is sanitized independently and prepended after
/// the untrusted message has been cleaned, making its first position trustworthy
/// even when a sender uses the same data attribute.
fn sanitized_source_body_marker(input: &str) -> Option<String> {
    let document = scraper::Html::parse_document(input);
    let selector = scraper::Selector::parse("body").expect("static body selector is valid");
    let body = document.select(&selector).next();
    let class_name = body.and_then(|element| element.attr("class"));
    let style = body.and_then(|element| element.attr("style"));
    if class_name.is_none() && style.is_none() {
        return None;
    }

    let mut marker = String::from(r#"<div data-satchel-source-body="on""#);
    if let Some(class_name) = class_name {
        marker.push_str(r#" class=""#);
        marker.push_str(&escape_html_attribute(class_name));
        marker.push('"');
    }
    if let Some(style) = style {
        marker.push_str(r#" style=""#);
        marker.push_str(&escape_html_attribute(style));
        marker.push('"');
    }
    marker.push_str("></div>");

    Some(build_email_html_sanitizer(true).clean(&marker).to_string())
}

/// Attributes whose whole value is a reference: replaced even when it is not a
/// web address (`cid:`, `data:`, a relative path). A translation engine needs
/// none of them, and in mail they carry per-recipient tracking tokens
/// (ADR 0018).
const WITHHELD_ATTRIBUTES: &[&str] = &["src", "srcset", "href", "background", "poster", "cite"];

/// Attributes whose value means nothing to a translation, withheld whole as
/// well: `data-*` (often JSON with escaped URLs), `id`, `name`, `summary`.
fn withheld_wholesale(attribute: &str) -> bool {
    WITHHELD_ATTRIBUTES.contains(&attribute)
        || attribute.starts_with("data-")
        || matches!(attribute, "id" | "name" | "summary")
}

const TOKEN_END: &str = "-end";

/// The references [`withhold_references`] took out, keyed by a nonce drawn
/// for this request: a placeholder the mail itself contains, or one an engine
/// invents, does not match and is never "restored" to a real address.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WithheldReferences {
    nonce: String,
    /// Whole attribute values, unescaped; restored with attribute escaping.
    attributes: Vec<String>,
    /// Serialized substrings (CSS `url(…)` contents, addresses in text,
    /// other attributes and style blocks); restored as they were.
    raw: Vec<String>,
}

impl WithheldReferences {
    pub fn len(&self) -> usize {
        self.attributes.len() + self.raw.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn prefix(&self) -> String {
        format!("#satchel-{}-", self.nonce)
    }

    fn raw_token(&self, index: usize) -> String {
        format!("{}r{index}{TOKEN_END}", self.prefix())
    }
}

/// A nonce the input does not already contain.
fn fresh_nonce(input: &str) -> String {
    loop {
        let nonce = format!("{:012x}", rand::random::<u64>() & 0xffff_ffff_ffff);
        if !input.contains(&nonce) {
            return nonce;
        }
    }
}

/// Stored mail HTML with every reference replaced by a placeholder
/// (`#satchel-<nonce>-a3-end`), for sending to an engine that translates
/// markup. Withheld are:
///
/// - the whole value of `src`, `srcset`, `href`, `background`, `poster`,
///   `cite`, every `data-*`, `id`, `name` and `summary`, whatever it holds
///   (in-page anchors `#…` in `href` stay);
/// - the contents of every CSS `url(…)`, in style attributes and blocks;
/// - every address anywhere else: other attributes (`alt`, `title`), CSS
///   strings such as `image-set('…')`, and the text itself, link text
///   included. An address starts with `scheme://` (also JSON-escaped,
///   `scheme:\/\/`), `//host.tld`, `www.` or `mailto:`.
///
/// Host names written without one of those prefixes, and e-mail addresses in
/// the text, stay: they are part of what the mail says. The result is the
/// email sanitizer's output, so nothing the sanitizer drops goes out either.
pub fn withhold_references(input: &str) -> (String, WithheldReferences) {
    use std::sync::{Arc, Mutex};
    let nonce = fresh_nonce(input);
    let attributes = Arc::new(Mutex::new(Vec::<String>::new()));
    let collected = Arc::clone(&attributes);
    let attribute_nonce = nonce.clone();
    let rewrite: AttributeRewrite = Box::new(move |_element, attribute, value| {
        if !withheld_wholesale(attribute) {
            return None;
        }
        let trimmed = value.trim();
        if trimmed.is_empty() || (attribute == "href" && trimmed.starts_with('#')) {
            return None;
        }
        let mut values = collected
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        values.push(value.to_owned());
        Some(format!(
            "#satchel-{attribute_nonce}-a{}{TOKEN_END}",
            values.len() - 1
        ))
    });
    let cleaned = build_email_html_sanitizer_rewriting(true, Some(rewrite))
        .clean(input)
        .to_string();
    let mut withheld = WithheldReferences {
        nonce,
        attributes: std::mem::take(
            &mut *attributes
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        ),
        raw: Vec::new(),
    };
    let html = withhold_css_urls(&cleaned, &mut withheld);
    let html = withhold_addresses(&html, &mut withheld);
    (html, withheld)
}

/// A plain-text body with every address replaced by a placeholder, by the
/// same rules as the text of an HTML body. [`restore_references`] puts them
/// back byte for byte.
pub fn withhold_text_references(text: &str) -> (String, WithheldReferences) {
    let mut withheld = WithheldReferences {
        nonce: fresh_nonce(text),
        ..Default::default()
    };
    let withheld_text = withhold_addresses(text, &mut withheld);
    (withheld_text, withheld)
}

/// Every `url(…)` in style attributes and style blocks. The serialized text
/// inside the parentheses is kept as it is, quotes and escaping included, so
/// putting it back restores the exact bytes.
fn withhold_css_urls(html: &str, withheld: &mut WithheldReferences) -> String {
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len());
    let mut cursor = 0;
    while let Some(found) = lower[cursor..].find("url(") {
        let open = cursor + found + "url(".len();
        let Some(close) = html[open..].find(')').map(|offset| open + offset) else {
            break;
        };
        let inner = &html[open..close];
        let bare = inner
            .trim()
            .trim_matches(|c| c == '"' || c == '\'')
            .trim_start_matches("&quot;");
        out.push_str(&html[cursor..open]);
        if bare.is_empty() || bare.starts_with('#') {
            out.push_str(inner);
        } else {
            out.push_str(&withheld.raw_token(withheld.raw.len()));
            withheld.raw.push(inner.to_owned());
        }
        cursor = close;
    }
    out.push_str(&html[cursor..]);
    out
}

/// `//` or its JSON-escaped form `\/\/`, and how many bytes it takes.
fn double_slash(rest: &[u8]) -> Option<usize> {
    if rest.starts_with(b"//") {
        Some(2)
    } else if rest.starts_with(br"\/\/") {
        Some(4)
    } else {
        None
    }
}

/// Where a web or mail address starts at byte `at`, if one does.
fn address_starts_at(bytes: &[u8], at: usize) -> bool {
    let rest = &bytes[at..];
    let lower =
        |len: usize| -> Vec<u8> { rest.iter().take(len).map(u8::to_ascii_lowercase).collect() };
    let prev = at.checked_sub(1).map(|index| bytes[index]);
    let at_boundary =
        prev.is_none_or(|byte| !(byte.is_ascii_alphanumeric() || b"+-._@".contains(&byte)));
    if at_boundary && (lower(7) == b"mailto:" || lower(4) == b"www.") {
        return true;
    }
    // `//host`: protocol-relative, a host with a dot right after the
    // slashes. Not the `//` of `scheme://`, which the scheme branch below
    // takes as a whole, and not `1//2` or `and//or` in running text.
    if let Some(slashes) = double_slash(rest) {
        if prev.is_none_or(|byte| {
            !(byte.is_ascii_alphanumeric() || byte == b':' || byte == b'/' || byte == b'\\')
        }) {
            let host: Vec<u8> = rest[slashes..]
                .iter()
                .copied()
                .take_while(|byte| byte.is_ascii_alphanumeric() || b".-".contains(byte))
                .collect();
            if host.first().is_some_and(u8::is_ascii_alphanumeric)
                && host
                    .windows(2)
                    .any(|pair| pair[0] == b'.' && pair[1].is_ascii_alphanumeric())
            {
                return true;
            }
        }
    }
    if !at_boundary || !rest.first().is_some_and(u8::is_ascii_alphabetic) {
        return false;
    }
    let scheme = rest
        .iter()
        .take(32)
        .take_while(|byte| byte.is_ascii_alphanumeric() || b"+.-".contains(byte))
        .count();
    rest[scheme..].starts_with(b":") && double_slash(&rest[scheme + 1..]).is_some()
}

/// The length of a tag that does not end an address when it sits inside one:
/// `<wbr>`, `<wbr/>`, `<wbr />`, or an empty inline element such as
/// `<span></span>`. Senders split long links with them.
fn inert_tag_len(rest: &str) -> Option<usize> {
    let lower = rest.get(..rest.len().min(64))?.to_ascii_lowercase();
    for wbr in ["<wbr>", "<wbr/>", "<wbr />"] {
        if lower.starts_with(wbr) {
            return Some(wbr.len());
        }
    }
    let name_len = lower[1..]
        .bytes()
        .take_while(u8::is_ascii_alphanumeric)
        .count();
    if name_len == 0 {
        return None;
    }
    let name = &lower[1..1 + name_len];
    let open_end = lower.find('>')?;
    if lower[1 + name_len..open_end].contains('<') {
        return None;
    }
    let close = format!("</{name}>");
    lower[open_end + 1..]
        .starts_with(&close)
        .then_some(open_end + 1 + close.len())
}

/// Where the address that starts at `start` ends: at ASCII white space, `>`,
/// `"`, `&nbsp;`, `&quot;`, or a `<` that is not an inert tag. Trailing
/// sentence punctuation stays in the text, except a closing bracket that
/// pairs with an opening one inside the address.
fn address_end(text: &str, start: usize) -> usize {
    let bytes = text.as_bytes();
    let mut end = start;
    while end < bytes.len() {
        let byte = bytes[end];
        if byte.is_ascii_whitespace() || byte == b'>' || byte == b'"' {
            break;
        }
        if byte == b'&' && (text[end..].starts_with("&nbsp;") || text[end..].starts_with("&quot;"))
        {
            break;
        }
        if byte == b'<' {
            match inert_tag_len(&text[end..]) {
                Some(len) => {
                    end += len;
                    continue;
                }
                None => break,
            }
        }
        end += 1;
    }
    while end > start {
        let last = bytes[end - 1];
        let opener = match last {
            b')' => Some(b'('),
            b']' => Some(b'['),
            b'}' => Some(b'{'),
            b'.' | b',' | b';' | b':' | b'!' | b'?' | b'\'' => None,
            _ => break,
        };
        if let Some(opener) = opener {
            let span = &bytes[start..end];
            let opened = span.iter().filter(|&&b| b == opener).count();
            let closed = span.iter().filter(|&&b| b == last).count();
            if opened >= closed {
                break;
            }
        }
        end -= 1;
    }
    end
}

/// Every web or mail address left in the serialized HTML or plain text,
/// wherever it sits.
fn withhold_addresses(html: &str, withheld: &mut WithheldReferences) -> String {
    let bytes = html.as_bytes();
    let mut out = String::with_capacity(html.len());
    let mut cursor = 0;
    let mut at = 0;
    while at < bytes.len() {
        if !html.is_char_boundary(at) || !address_starts_at(bytes, at) {
            at += 1;
            continue;
        }
        let end = address_end(html, at);
        if end <= at || !html.is_char_boundary(end) {
            at += 1;
            continue;
        }
        out.push_str(&html[cursor..at]);
        out.push_str(&withheld.raw_token(withheld.raw.len()));
        withheld.raw.push(html[at..end].to_owned());
        cursor = end;
        at = end;
    }
    out.push_str(&html[cursor..]);
    out
}

/// Puts the withheld references back into a translation of
/// [`withhold_references`]' or [`withhold_text_references`]' output, in one
/// pass. Only this request's placeholders match, exactly as written (an
/// uppercased or renumbered one stays inert); ones the engine dropped stay
/// dropped. Run the email sanitizer over restored HTML before anything
/// renders it.
pub fn restore_references(text: &str, withheld: &WithheldReferences) -> String {
    let prefix = withheld.prefix();
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;
    while let Some(found) = text[cursor..].find(&prefix) {
        let at = cursor + found;
        out.push_str(&text[cursor..at]);
        let rest = &text[at + prefix.len()..];
        let parsed = rest.as_bytes().first().and_then(|kind| {
            let digits = rest[1..].bytes().take_while(u8::is_ascii_digit).count();
            let index: usize = rest.get(1..1 + digits)?.parse().ok()?;
            rest[1 + digits..].starts_with(TOKEN_END).then_some((
                *kind,
                index,
                1 + digits + TOKEN_END.len(),
            ))
        });
        let replacement = parsed.and_then(|(kind, index, consumed)| match kind {
            b'a' => withheld
                .attributes
                .get(index)
                .map(|value| (escape_html_attribute(value), consumed)),
            b'r' => withheld
                .raw
                .get(index)
                .map(|value| (value.clone(), consumed)),
            _ => None,
        });
        match replacement {
            Some((value, consumed)) => {
                out.push_str(&value);
                cursor = at + prefix.len() + consumed;
            }
            None => {
                out.push_str(&prefix);
                cursor = at + prefix.len();
            }
        }
    }
    out.push_str(&text[cursor..]);
    out
}

pub fn sanitize_html(input: &str) -> String {
    ammonia::Builder::default()
        .link_rel(Some("noopener noreferrer"))
        .clean(input)
        .to_string()
}

pub fn sanitize_email_html(input: &str) -> String {
    sanitize_email_html_inner(input, false)
}

/// Recheck stored HTML without losing its sanitized body presentation marker.
pub fn sanitize_stored_email_html(input: &str) -> String {
    sanitize_email_html_inner(input, true)
}

fn sanitize_email_html_inner(input: &str, preserve_marker: bool) -> String {
    let mut document = scraper::Html::parse_document(input);
    let mut marker = sanitized_source_body_marker(input);
    if preserve_marker && marker.is_none() {
        let selector =
            scraper::Selector::parse("body > div:first-child").expect("static marker selector");
        let stored_marker = document
            .select(&selector)
            .next()
            .filter(|element| {
                element.attr(SOURCE_BODY_MARKER_ATTRIBUTE) == Some("on")
                    && element.inner_html().is_empty()
            })
            .map(|element| (element.id(), element.html()));
        if let Some((id, html)) = stored_marker {
            marker = Some(build_email_html_sanitizer(true).clean(&html).to_string());
            if let Some(mut node) = document.tree.get_mut(id) {
                node.detach();
            }
        }
    }
    sanitize_style_blocks(&mut document);
    let content = build_email_html_sanitizer(false)
        .clean(&document.html())
        .to_string();
    if let Some(marker) = marker {
        // The marker is ours; strip sender markers first, then clean the complete result.
        build_email_html_sanitizer(true)
            .clean(&format!("{marker}{content}"))
            .to_string()
    } else {
        content
    }
}

pub async fn sanitize_optional_email_html(
    input: Option<String>,
) -> Result<Option<String>, tokio::task::JoinError> {
    let Some(input) = input else {
        return Ok(None);
    };
    tokio::task::spawn_blocking(move || {
        let sanitized = sanitize_email_html(&input);
        (!sanitized.trim().is_empty()).then_some(sanitized)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::{sanitize_email_html, sanitize_html, SOURCE_BODY_MARKER_ATTRIBUTE};

    #[test]
    fn output_pass_preserves_ingested_body_presentation() {
        let ingested = sanitize_email_html(
            r#"<body class="campaign" style="font-family:serif"><p>Hi</p></body>"#,
        );
        let read = super::sanitize_stored_email_html(&ingested);
        let reread = super::sanitize_stored_email_html(&read);
        let selector = scraper::Selector::parse("[data-satchel-source-body]").unwrap();
        for output in [ingested, read, reread] {
            let document = scraper::Html::parse_document(&output);
            let markers = document.select(&selector).collect::<Vec<_>>();
            assert_eq!(markers.len(), 1);
            assert_eq!(markers[0].attr("class"), Some("campaign"));
            assert_eq!(markers[0].attr("style"), Some("font-family:serif"));
            assert!(output.contains("<p>Hi</p>"));
        }
    }

    #[test]
    fn css_cleanup_cannot_create_markup_after_sanitizing() {
        let payload = r#"<p>Hi</p><style></sty@import;le><link rel="preconnect" href="https://u123.t.example"><meta http-equiv="refresh" content="0;url=https://evil.example/login"></style>"#;
        for raw in [
            payload.to_string(),
            payload.replace("@import;", "url()"),
            payload.replace("@import;", "expression("),
            payload.replace("@import;", "behavior:"),
            payload.replace("@import;", "-moz-binding"),
        ] {
            let output = sanitize_email_html(&raw);
            let document = scraper::Html::parse_document(&output);
            let selector = scraper::Selector::parse("link, meta").unwrap();
            assert_eq!(document.select(&selector).count(), 0, "{output}");
        }
    }

    #[test]
    fn removes_scripts_event_handlers_and_js_urls() {
        let raw = r#"
            <div onclick="steal()">
                <script>alert(1)</script>
                <a href="javascript:alert(1)">bad</a>
                <img src="x" onerror="hack()">
            </div>
        "#;

        let sanitized = sanitize_html(raw);

        assert!(!sanitized.contains("<script"));
        assert!(!sanitized.contains("onclick"));
        assert!(!sanitized.contains("onerror"));
        assert!(!sanitized.contains("javascript:"));
    }

    #[test]
    fn preserves_safe_markup() {
        let raw = r#"<p>Hello <strong>team</strong> <a href="https://example.com">open</a></p>"#;

        let sanitized = sanitize_html(raw);

        assert!(sanitized.contains("<strong>team</strong>"));
        assert!(sanitized.contains("https://example.com"));
        assert!(sanitized.contains("rel=\"noopener noreferrer\""));
    }

    #[test]
    fn preserves_table_markup_and_its_inline_presentation() {
        // The composer's own tables, and what other clients send: structure,
        // spans, and borders and padding carried inline or as attributes.
        let raw = r#"<table border="1" cellpadding="6" cellspacing="0" style="border-collapse:collapse;border:1px solid #c3c7cc"><thead><tr><th style="border:1px solid #c3c7cc;padding:6px 10px;text-align:left" scope="col">Item</th><th>Qty</th></tr></thead><tbody><tr><td colspan="2" rowspan="1" align="right" valign="top" width="120"><p style="margin:0">Apples</p></td></tr></tbody><tfoot><tr><td>Total</td><td>12</td></tr></tfoot></table>"#;

        let sanitized = sanitize_email_html(raw);

        for kept in [
            r#"border="1""#,
            r#"cellpadding="6""#,
            r#"cellspacing="0""#,
            "border-collapse:collapse",
            "padding:6px 10px",
            r#"scope="col""#,
            r#"colspan="2""#,
            r#"align="right""#,
            r#"valign="top""#,
            r#"width="120""#,
            r#"<p style="margin:0">"#,
            "<thead>",
            "<tfoot>",
        ] {
            assert!(sanitized.contains(kept), "{kept} missing from {sanitized}");
        }
    }

    #[test]
    fn preserves_email_css_and_classes() {
        let raw = r##"
            <style>
                .dark-logo { display: none; }
            </style>
            <table class="wrapper" style="background:#000" bgcolor="#000" data-track="hero">
                <tr>
                    <td>
                        <img class="dark-logo" src="https://example.com/logo-dark.png" style="display:none" />
                    </td>
                </tr>
            </table>
        "##;

        let sanitized = sanitize_email_html(raw);

        assert!(sanitized.contains("<style>"));
        assert!(sanitized.contains("class=\"wrapper\""));
        assert!(sanitized.contains("style=\"background:#000\""));
        assert!(sanitized.contains("bgcolor=\"#000\""));
        assert!(sanitized.contains("data-track=\"hero\""));
        assert!(sanitized.contains("class=\"dark-logo\""));
        assert!(sanitized.contains("src=\"https://example.com/logo-dark.png\""));
        assert!(sanitized.contains("display:none"));
    }

    #[test]
    fn preserves_body_presentation_in_a_trusted_marker() {
        let raw = r#"
            <html>
                <body class="campaign" style="margin:0;font-family:Arial,sans-serif" onclick="steal()">
                    <p>Visible message body</p>
                </body>
            </html>
        "#;

        let sanitized = sanitize_email_html(raw);

        assert!(sanitized.starts_with("<div data-satchel-source-body=\"on\""));
        assert!(sanitized.contains("class=\"campaign\""));
        assert!(sanitized.contains("font-family:Arial,sans-serif"));
        assert!(!sanitized.contains("onclick"));
        assert!(sanitized.contains("Visible message body"));
    }

    #[test]
    fn reserves_the_source_body_marker_for_sanitizer_output() {
        let sanitized = sanitize_email_html(
            r#"<div data-satchel-source-body="on" style="font-family:serif">Sender content</div>"#,
        );

        assert!(!sanitized.contains(SOURCE_BODY_MARKER_ATTRIBUTE));
        assert!(sanitized.contains("Sender content"));
    }

    #[test]
    fn leaves_empty_and_fragment_inputs_without_an_internal_marker() {
        assert_eq!(sanitize_email_html(""), "");
        assert_eq!(sanitize_email_html("<p>Fragment</p>"), "<p>Fragment</p>");
    }

    #[test]
    fn removes_document_titles_without_exposing_them_as_body_text() {
        let raw = r#"
            <!doctype html>
            <html>
                <head><title>Duplicate message subject</title></head>
                <body><p>Visible message body</p></body>
            </html>
        "#;

        let sanitized = sanitize_email_html(raw);

        assert!(!sanitized.contains("Duplicate message subject"));
        assert!(sanitized.contains("Visible message body"));
    }

    #[test]
    fn preserves_mail_client_reply_section_markers() {
        let raw = r#"<div name="messageReplySection">On 10 July, Mike wrote:<blockquote>Earlier message</blockquote></div>"#;

        let sanitized = sanitize_email_html(raw);

        assert!(sanitized.contains("name=\"messageReplySection\""));
    }

    #[test]
    fn preserves_local_email_image_sources() {
        let raw = r#"
            <img src="cid:image001.jpg@ABC" />
            <img src="data:image/png;base64,abc123" />
            <img src="blob:tauri://localhost/abc123" />
        "#;

        let sanitized = sanitize_email_html(raw);

        assert!(sanitized.contains("src=\"cid:image001.jpg@ABC\""));
        assert!(sanitized.contains("src=\"data:image/png;base64,abc123\""));
        assert!(sanitized.contains("src=\"blob:tauri://localhost/abc123\""));
    }

    #[test]
    fn email_sanitizer_still_removes_scripts_and_handlers() {
        let raw = r#"
            <style>.safe { color: white; }</style>
            <div class="safe" onclick="steal()">
                <script>alert(1)</script>
                <img src="x" onerror="hack()" style="display:block" />
                <a href="javascript:alert(1)">bad</a>
            </div>
        "#;

        let sanitized = sanitize_email_html(raw);

        assert!(sanitized.contains("<style>"));
        assert!(!sanitized.contains("<script"));
        assert!(!sanitized.contains("onclick"));
        assert!(!sanitized.contains("onerror"));
        assert!(!sanitized.contains("javascript:"));
    }

    #[test]
    fn data_urls_survive_only_in_image_sources() {
        let raw = r#"
            <a href="data:text/html;base64,PHNjcmlwdD5hbGVydCgxKTwvc2NyaXB0Pg==">Rechnung</a>
            <img src="data:image/png;base64,abc123" />
        "#;

        let sanitized = sanitize_email_html(raw);

        assert!(!sanitized.contains("data:text/html"));
        assert!(sanitized.contains("Rechnung"));
        assert!(sanitized.contains("src=\"data:image/png;base64,abc123\""));
    }

    #[test]
    fn links_keep_only_web_and_mail_schemes() {
        let raw = r#"
            <a href="https://example.com/a">web</a>
            <a href="mailto:team@example.com">mail</a>
            <a href="ftp://example.com/a">ftp</a>
            <a href="cid:image001">cid</a>
            <a href="blob:tauri://localhost/abc">blob</a>
            <a href="asset://localhost/etc/passwd">asset</a>
        "#;

        let sanitized = sanitize_email_html(raw);

        assert!(sanitized.contains("href=\"https://example.com/a\""));
        assert!(sanitized.contains("href=\"mailto:team@example.com\""));
        assert!(!sanitized.contains("ftp://"));
        assert!(!sanitized.contains("href=\"cid:"));
        assert!(!sanitized.contains("href=\"blob:"));
        assert!(!sanitized.contains("asset://"));
    }

    #[test]
    fn image_srcset_drops_foreign_schemes() {
        let allowed = sanitize_email_html(
            r#"<img srcset="https://example.com/a.png 1x, cid:b@c 2x" src="https://example.com/a.png" />"#,
        );
        assert!(allowed.contains("srcset="));

        let denied = sanitize_email_html(
            r#"<img srcset="javascript:alert(1) 1x" src="https://example.com/a.png" />"#,
        );
        assert!(!denied.contains("srcset="));
        assert!(!denied.contains("javascript:"));
    }

    #[test]
    fn style_blocks_lose_remote_urls_and_imports() {
        let raw = r##"
            <style>
                @import url("https://tracker.example/evil.css");
                input[value^="a"] { background: url(https://tracker.example/a); }
                .logo { background: url(cid:logo001@satchel); }
                .ie { width: expression(alert(1)); behavior: url(#default#time2); }
                .moz { -moz-binding: url(https://tracker.example/x.xml#z); }
                .hidden { background: url(/*x*/"https://tracker.example/b"); }
            </style>
            <p>Body</p>
        "##;

        let sanitized = sanitize_email_html(raw);

        assert!(!sanitized.contains("@import"));
        assert!(!sanitized.contains("tracker.example"));
        assert!(!sanitized.contains("expression("));
        assert!(!sanitized.contains("behavior:"));
        assert!(!sanitized.contains("-moz-binding"));
        assert!(sanitized.contains("url(cid:logo001@satchel)"));
        assert!(sanitized.contains("input[value^=\"a\"]"));
        assert!(sanitized.contains("Body"));
    }

    #[test]
    fn style_attributes_lose_remote_urls() {
        let sanitized = sanitize_email_html(
            r#"<div style="background:#000;background-image:url(https://tracker.example/a)">x</div>"#,
        );

        assert!(sanitized.contains("background:#000"));
        assert!(!sanitized.contains("tracker.example"));
    }

    #[test]
    fn dangerous_tags_are_removed() {
        let cases = [
            (
                r#"<form action="https://evil.example"><input name="a" /></form>"#,
                "<form",
            ),
            (
                r#"<meta http-equiv="refresh" content="0;url=https://evil.example" />"#,
                "http-equiv",
            ),
            (r#"<base href="https://evil.example/" />"#, "<base"),
            (r#"<iframe src="https://evil.example"></iframe>"#, "<iframe"),
            (
                r#"<object data="https://evil.example"></object>"#,
                "<object",
            ),
            (
                r#"<svg><use href="https://evil.example#x" /></svg>"#,
                "<svg",
            ),
        ];

        for (raw, forbidden) in cases {
            let sanitized = sanitize_email_html(raw);
            assert!(
                !sanitized.contains(forbidden),
                "{forbidden} survived sanitizing of {raw}: {sanitized}"
            );
        }
    }
}

#[cfg(test)]
mod withheld_reference_tests {
    use super::*;

    /// One mail with an address in every place one can hide.
    const PROBE: &str = r##"<div data-satchel-source-body="on"></div><style>.hero { background: url("cid:hero@x"); } .b { background-image: image-set('https://t.example/set.png?u=4711' 1x); }</style><p style="background-image: url(cid:bg@x); content: image-set(url(cid:i@x) 1x)">Hello <a href="https://click.example/r?u=4711&amp;l=7">https://click.example/r?u=4711&amp;l=7</a> <a href="#top">top</a> <a href="mailto:me@example.test">write to me@example.test</a>, see www.example.org/offer?u=4711. or //cdn.example/x?u=4711</p><img src="https://t.example/open.gif?u=4711" srcset="https://t.example/a.png?u=4711 1x" alt="Logo from https://t.example/logo?u=4711" title="See http://t.example/t?u=4711"><span data-src="https://t.example/d.png?u=4711" data-href="//t.example/h?u=4711">Data</span><img src="cid:logo@x"><table background="https://t.example/t.png?u=4711"><tr><td>x</td></tr></table>"##;

    /// Independent of the scanner under test: anything that looks like an
    /// address, a tracker host or the token.
    fn leaks(sent: &str) -> Vec<&'static str> {
        let lower = sent.to_ascii_lowercase();
        [
            "http",
            "www.",
            "mailto:",
            "//",
            "example.org",
            "t.example",
            "click.example",
            "cdn.example",
            "u=4711",
            "cid:",
        ]
        .into_iter()
        .filter(|needle| lower.contains(needle))
        .collect()
    }

    #[test]
    fn no_address_leaves_with_the_html() {
        let stored = sanitize_stored_email_html(PROBE);
        let (sent, _) = withhold_references(&stored);
        let request = crate::translation::TranslationRequest {
            content: crate::translation::TranslationContent::Html(sent.clone()),
            source_language: None,
            target_language: "DE".into(),
        };
        let body = String::from_utf8(crate::translation::deepl::translate_body(&request).unwrap())
            .unwrap();
        assert_eq!(leaks(&body), Vec::<&str>::new(), "{body}");
        // What is still sent: the words, the in-page anchor, plain e-mail
        // addresses in the text, and the presentation marker.
        for kept in [
            "Hello",
            "href=\"#top\"",
            "me@example.test",
            "Logo from",
            "Data",
            "data-satchel-source-body",
        ] {
            assert!(sent.contains(kept), "{kept} missing in {sent}");
        }
    }

    #[test]
    fn restoring_returns_the_exact_bytes() {
        let stored = sanitize_stored_email_html(PROBE);
        let (sent, withheld) = withhold_references(&stored);
        assert!(!withheld.is_empty());
        assert_eq!(restore_references(&sent, &withheld), stored);
        let translated = sent.replace("Hello", "Hallo");
        assert_eq!(
            sanitize_stored_email_html(&restore_references(&translated, &withheld)),
            stored.replace("Hello", "Hallo")
        );
    }

    #[test]
    fn a_placeholder_from_another_request_is_not_restored() {
        let (_, withheld) = withhold_references(r#"<img src="https://t.example/x.gif">"#);
        // What a mail (or an engine) could write: the right shape, but
        // without this request's nonce.
        let forged =
            r##"<a href="#satchel-000000000000-a0-end">x</a> #satchel-000000000000-r0-end"##;
        let restored = restore_references(forged, &withheld);
        assert!(!restored.contains("t.example"), "{restored}");
        assert_eq!(restored, forged);
        let script = sanitize_stored_email_html(&restore_references(
            r#"<img src="javascript:alert(1)">"#,
            &withheld,
        ));
        assert!(!script.contains("javascript"));
    }

    #[test]
    fn every_request_draws_its_own_nonce() {
        let html = r#"<img src="https://t.example/x.gif">"#;
        let (first, _) = withhold_references(html);
        let (second, _) = withhold_references(html);
        assert_ne!(first, second);
    }

    fn withheld_text(text: &str) -> (String, WithheldReferences) {
        withhold_text_references(text)
    }

    #[test]
    fn addresses_are_taken_whole_however_they_are_written() {
        for (input, address) in [
            (
                "see https://x.example/p(1)?u=tok now",
                "https://x.example/p(1)?u=tok",
            ),
            (
                "see https://x.example/?a[]=tok now",
                "https://x.example/?a[]=tok",
            ),
            (
                "see https://x.example/o'b?u=tok now",
                "https://x.example/o'b?u=tok",
            ),
            (
                "see http://[2001:db8::1]/x?u=tok now",
                "http://[2001:db8::1]/x?u=tok",
            ),
            (
                "see https://x.example/a<wbr>b?u=tok now",
                "https://x.example/a<wbr>b?u=tok",
            ),
            (
                "see https://<wbr>host.example/x?u=tok now",
                "https://<wbr>host.example/x?u=tok",
            ),
            (
                "see https://x.example/a<wbr/>b?u=tok now",
                "https://x.example/a<wbr/>b?u=tok",
            ),
            (
                "see https://x.example/a<span></span>b?u=tok now",
                "https://x.example/a<span></span>b?u=tok",
            ),
            (
                "(see https://x.example/p(1)?u=tok).",
                "https://x.example/p(1)?u=tok",
            ),
            (
                "[https://click.example/r?u=tok]",
                "https://click.example/r?u=tok",
            ),
            (
                "json https:\\/\\/x.example\\/y?u=tok here",
                "https:\\/\\/x.example\\/y?u=tok",
            ),
            (
                "json \\/\\/cdn.example\\/z?u=tok here",
                "\\/\\/cdn.example\\/z?u=tok",
            ),
        ] {
            let (sent, withheld) = withheld_text(input);
            assert_eq!(withheld.raw, [address], "{input}");
            assert!(!sent.contains("tok"), "{input} -> {sent}");
            assert_eq!(restore_references(&sent, &withheld), input);
        }
    }

    #[test]
    fn an_address_ends_where_the_markup_does() {
        let (sent, withheld) = withheld_text(
            "<a>https://x.example/r?u=tok</a> and &quot;https://y.example/q&quot; done",
        );
        assert_eq!(
            withheld.raw,
            ["https://x.example/r?u=tok", "https://y.example/q"]
        );
        assert!(sent.contains("</a> and &quot;"));
    }

    #[test]
    fn identifiers_and_data_attributes_go_whole() {
        let html = r#"<div id="user-4711" data-track='{"u":"https:\/\/t.example\/p?u=4711"}'><a name="anchor-4711">x</a><table summary="report for 4711"><tr><td>y</td></tr></table></div>"#;
        let stored = sanitize_stored_email_html(html);
        let (sent, withheld) = withhold_references(&stored);
        assert!(!sent.contains("4711"), "{sent}");
        assert!(!sent.contains("t.example"), "{sent}");
        assert_eq!(restore_references(&sent, &withheld), stored);
    }

    #[test]
    fn an_altered_placeholder_stays_inert() {
        let (sent, withheld) = withheld_text("go to https://x.example/a?u=tok now");
        let upper = sent.to_ascii_uppercase();
        assert!(!restore_references(&upper, &withheld).contains("x.example"));
        let renumbered = sent.replace("-r0-", "-r7-");
        assert_eq!(restore_references(&renumbered, &withheld), renumbered);
    }

    #[test]
    fn a_plain_text_newsletter_keeps_no_address() {
        let text = "Hello Ana,\n\nOur offer [https://click.example/r?u=4711&l=7] ends Friday.\nUnsubscribe: <https://list.example/u?u=4711>\nOr write to me@example.test or mailto:me@example.test\nSee www.example.org/x?u=4711.";
        let (sent, withheld) = withheld_text(text);
        assert_eq!(leaks(&sent), Vec::<&str>::new(), "{sent}");
        assert!(sent.contains("me@example.test"));
        assert_eq!(restore_references(&sent, &withheld), text);
    }

    #[test]
    fn addresses_are_found_where_they_are_and_nowhere_else() {
        let mut withheld = WithheldReferences {
            nonce: "n".into(),
            ..Default::default()
        };
        let out = withhold_addresses(
            "Mail me@example.test or visit https://x.example/a?b=1&amp;c=2. Done; ratio 1//2, a//b and ftp://f.example",
            &mut withheld,
        );
        assert_eq!(
            withheld.raw,
            ["https://x.example/a?b=1&amp;c=2", "ftp://f.example"]
        );
        assert!(out.contains("me@example.test"));
        assert!(out.contains("#satchel-n-r0-end. Done"));
        assert!(out.contains("1//2"));
    }
}
