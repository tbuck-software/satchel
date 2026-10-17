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

#[cfg(test)]
mod tests {
    use super::strip_html_tags;

    #[test]
    fn strip_html_tags_only_removes_tags_and_keeps_text_layout_for_callers() {
        assert_eq!(strip_html_tags("<p>A</p><p>B</p>"), "AB");
    }
}
