/// Strip tags without imposing a provider-specific HTML whitespace policy.
pub(crate) fn strip_html_tags(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut inside_tag = false;
    for ch in input.chars() {
        match ch {
            '<' => inside_tag = true,
            '>' => inside_tag = false,
            _ if !inside_tag => output.push(ch),
            _ => {}
        }
    }
    output
}

/// The text an HTML document shows, its layout kept. Stripping only the tags
/// would leave the contents of elements that show nothing behind: a kept
/// `<style>` block turns into CSS in the text view.
pub fn visible_html_text(html: &str) -> String {
    let visible = ammonia::Builder::default()
        .add_clean_content_tags(&["title"])
        .clean(html)
        .to_string();
    strip_html_tags(&visible)
}

#[cfg(test)]
mod tests {
    use super::{strip_html_tags, visible_html_text};

    #[test]
    fn strip_html_tags_only_removes_tags_and_keeps_text_layout_for_callers() {
        assert_eq!(strip_html_tags("<p>A</p><p>B</p>"), "AB");
    }

    #[test]
    fn visible_text_leaves_out_what_no_element_shows() {
        assert_eq!(
            visible_html_text("<style>p{color:red}</style><title>T</title><p>A</p>\n<p>B</p>"),
            "A\nB"
        );
        assert_eq!(visible_html_text("<style>p{color:red}</style>").trim(), "");
    }
}
