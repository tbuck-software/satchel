#![no_main]
use libfuzzer_sys::fuzz_target;
use satchel_core::email::sanitizer::{
    sanitize_email_html, sanitize_html, sanitize_stored_email_html,
};

fuzz_target!(|data: &[u8]| {
    let input = String::from_utf8_lossy(data);
    let email = sanitize_email_html(&input);
    for output in [
        sanitize_html(&input),
        sanitize_stored_email_html(&email),
        email,
    ] {
        let document = scraper::Html::parse_document(&output);
        for element in document.select(&scraper::Selector::parse("*").unwrap()) {
            // Reparsing catches markup reopened by CSS cleanup after </style>.
            assert!(!matches!(
                element.value().name(),
                "script" | "link" | "meta" | "iframe" | "object" | "embed"
            ));
            for (name, value) in element.value().attrs() {
                assert!(!name.to_ascii_lowercase().starts_with("on"));
                if name == "srcset" {
                    for candidate in value.split(',') {
                        let url = candidate.split_ascii_whitespace().next().unwrap_or("");
                        assert!(!url.to_ascii_lowercase().starts_with("javascript:"));
                    }
                }
                if matches!(
                    name,
                    "href" | "src" | "action" | "formaction" | "xlink:href"
                ) {
                    let compact: String = value
                        .chars()
                        .filter(|c| !c.is_ascii_control() && !c.is_ascii_whitespace())
                        .collect();
                    assert!(!compact.to_ascii_lowercase().starts_with("javascript:"));
                }
            }
        }
        for style in document.select(&scraper::Selector::parse("style").unwrap()) {
            assert!(!style.inner_html().to_ascii_lowercase().contains("</style>"));
        }
    }
});
