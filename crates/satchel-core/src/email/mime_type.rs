/// Keep only a valid RFC media type; parameters never enter attachment metadata.
pub fn normalize_mime_type(value: &str) -> String {
    let media_type = value.split(';').next().unwrap_or_default().trim();
    let valid_token = |token: &str| {
        !token.is_empty()
            && token
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte))
    };
    match media_type.split_once('/') {
        Some((kind, subtype)) if valid_token(kind) && valid_token(subtype) => {
            media_type.to_ascii_lowercase()
        }
        _ => "application/octet-stream".to_string(),
    }
}

pub fn is_image_mime_type(value: &str) -> bool {
    let normalized = normalize_mime_type(value);
    normalized.starts_with("image/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_markup_in_media_types() {
        for value in [
            r#"image/png"><link rel=preconnect href=https://u1.t.example>"#,
            "image/",
            "/png",
            "image/png/extra",
            "image/p ng",
            "image/png\r\nX: y",
        ] {
            assert_eq!(normalize_mime_type(value), "application/octet-stream");
            assert!(!is_image_mime_type(value));
        }
        assert_eq!(
            normalize_mime_type(" IMAGE/SVG+XML; charset=utf-8"),
            "image/svg+xml"
        );
    }
}
