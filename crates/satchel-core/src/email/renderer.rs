use super::sanitizer::sanitize_html;

pub fn prepare_for_render(input: &str) -> String {
    let safe = sanitize_html(input);
    safe.replace("<a ", "<a target=\"_blank\" rel=\"noopener noreferrer\" ")
}
