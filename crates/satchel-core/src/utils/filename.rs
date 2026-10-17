//! Remove invisible formatting that can disguise a file's final extension.

pub fn strip_filename_controls(value: &str) -> String {
    value
        .chars()
        .filter(|ch| {
            !ch.is_control()
                && !matches!(*ch as u32,
                    0x00ad | 0x0600..=0x0605 | 0x061c | 0x06dd | 0x070f | 0x0890..=0x0891 |
                    0x08e2 | 0x180e | 0x200b..=0x200f | 0x202a..=0x202e | 0x2060..=0x2064 |
                    0x2066..=0x206f | 0xfeff | 0xfff9..=0xfffb | 0x110bd | 0x110cd |
                    0x13430..=0x1343f | 0x1bca0..=0x1bca3 | 0x1d173..=0x1d17a |
                    0xe0001 | 0xe0020..=0xe007f
                )
        })
        .collect()
}

pub fn deserialize_filename<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<String, D::Error> {
    let value = <String as serde::Deserialize>::deserialize(deserializer)?;
    Ok(strip_filename_controls(&value))
}

pub fn serialize_filename<S: serde::Serializer>(
    value: &str,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&strip_filename_controls(value))
}

#[cfg(test)]
mod tests {
    #[test]
    fn strips_bidi_and_formatting_without_losing_visible_unicode() {
        assert_eq!(
            super::strip_filename_controls("Rechnung\u{202e}fdp.html"),
            "Rechnungfdp.html"
        );
        assert_eq!(
            super::strip_filename_controls("Größe\u{200b}\u{2066}\u{feff}\u{e0001}.pdf"),
            "Größe.pdf"
        );
    }
}
