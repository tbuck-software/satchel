//! Signature placement is shared by queued, immediate, HTML and plain sends.
use std::sync::LazyLock;

use regex::Regex;
use scraper::{ElementRef, Html, Selector};

static HISTORY_INTRO: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:On .+ wrote:|Am .+ schrieb .+:|---------- (?:Forwarded message|Weitergeleitete Nachricht) ----------)$")
        .expect("composer history header regex")
});
static BLOCKQUOTE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)<blockquote\b[^>]*>").expect("blockquote opening tag regex"));
static QUOTE_SELECTOR: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse("blockquote").expect("blockquote selector"));

fn text_history_start(body: &str) -> usize {
    let mut offset = 0;
    for line in body.split_inclusive('\n') {
        if HISTORY_INTRO.is_match(line.trim()) {
            return offset;
        }
        offset += line.len();
    }
    body.len()
}

fn html_history_start(html: &str) -> usize {
    // Keep the original HTML bytes, including CID attributes and inline styles.
    // Parse each candidate in an inert tree to distinguish generated history
    // from ordinary blockquotes used in the writer's own message.
    for opening in BLOCKQUOTE.find_iter(html) {
        let fragment = Html::parse_fragment(&html[opening.start()..]);
        let Some(quote) = fragment.select(&QUOTE_SELECTOR).next() else {
            continue;
        };
        let Some(first) = quote.children().find_map(ElementRef::wrap) else {
            continue;
        };
        if HISTORY_INTRO.is_match(first.text().collect::<String>().trim()) {
            return opening.start();
        }
    }
    html.len()
}

pub(super) fn with_signature(
    body: &str,
    body_html: Option<&str>,
    signature_id: &str,
    signature_text: &str,
    signature_html: Option<&str>,
) -> (String, Option<String>) {
    let text = if signature_text.trim().is_empty() {
        body.to_owned()
    } else {
        let (writing, history) = body.split_at(text_history_start(body));
        let signed = format!("{}\n\n-- \n{signature_text}", writing.trim_end());
        if history.is_empty() {
            signed
        } else {
            format!("{}\n\n{history}", signed.trim_end())
        }
    };
    // No HTML body is the composer's plain-text contract. A rich signature
    // must not silently turn such a message into multipart/alternative.
    let html = body_html.map(|html| {
        let (writing, history) = html.split_at(html_history_start(html));
        let signature = signature_html
            .map(str::to_owned)
            .unwrap_or_else(|| escape_html(signature_text).replace('\n', "<br>"));
        format!(
            "{writing}<div data-satchel-signature=\"{}\">{signature}</div>{history}",
            escape_html(signature_id)
        )
    });
    (text, html)
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::with_signature;
    use satchel_core::email::build::build_rfc822;
    use satchel_core::providers::imap::parse_raw_message_body;
    use satchel_core::providers::traits::OutgoingMessage;

    fn assert_order(body: &str) {
        assert!(body.find("My answer").unwrap() < body.find("Kind regards").unwrap());
        assert!(body.find("Kind regards").unwrap() < body.find("Older mail").unwrap());
    }

    #[test]
    fn composer_signature_precedes_reply_and_forward_history_in_sent_mime() {
        for intro in [
            "On Monday, Ada wrote:",
            "Am Montag schrieb Ada:",
            "---------- Forwarded message ----------",
            "---------- Weitergeleitete Nachricht ----------",
        ] {
            let body = format!("My answer\n\n{intro}\n> Older mail");
            let html =
                format!("<p>My answer</p><blockquote><p>{intro}</p><p>Older mail</p></blockquote>");
            let (body, html) = with_signature(
                &body,
                Some(&html),
                "sig",
                "Kind regards",
                Some("<p>Kind regards</p>"),
            );
            assert_order(&body);
            assert_order(html.as_deref().unwrap());
            let mime = build_rfc822(
                &OutgoingMessage {
                    from_address: Some("me@example.test".into()),
                    to: vec!["ada@example.test".into()],
                    subject: "Reply".into(),
                    body,
                    body_html: html,
                    ..Default::default()
                },
                None,
                None,
            )
            .unwrap();
            let parsed = parse_raw_message_body(mime.as_bytes());
            assert_order(parsed.text.as_deref().unwrap());
            assert_order(parsed.html.as_deref().unwrap());
        }
    }

    #[test]
    fn composer_signature_preserves_plain_text_and_unquoted_messages() {
        let (body, html) = with_signature(
            "My answer\n\nOn Monday, Ada wrote:\n> Older mail",
            None,
            "sig",
            "Kind regards",
            Some("<p>Rich signature</p>"),
        );
        assert_order(&body);
        assert!(html.is_none());
        let (body, html) = with_signature(
            "My answer",
            Some("<p>My answer</p>"),
            "sig",
            "Kind regards",
            None,
        );
        assert_eq!(body, "My answer\n\n-- \nKind regards");
        assert_eq!(
            html.as_deref(),
            Some("<p>My answer</p><div data-satchel-signature=\"sig\">Kind regards</div>")
        );
    }

    #[test]
    fn composer_signature_skips_authored_quotes_and_preserves_inline_assets() {
        let html = "<blockquote><p>My answer</p></blockquote><blockquote><p>On Monday, Ada wrote:</p><p>Older mail</p><img src=\"cid:old\"></blockquote>";
        let (_, html) = with_signature(
            "My answer\n\nOn Monday, Ada wrote:\n> Older mail",
            Some(html),
            "sig",
            "Kind regards",
            Some("<p>Kind regards<img src=\"cid:logo\"></p>"),
        );
        let html = html.unwrap();
        assert_order(&html);
        assert!(html.contains("src=\"cid:logo\""));
        assert!(html.contains("src=\"cid:old\""));
    }
}
