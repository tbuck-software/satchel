//! Attachment names from mail: strip invisible formatting that can disguise
//! the final extension, and reduce the name to a plain basename.

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

/// Windows device names, reserved with any extension (`nul.txt`).
const RESERVED_DEVICE_NAMES: &[&str] = &[
    "con",
    "prn",
    "aux",
    "nul",
    "conin$",
    "conout$",
    "com0",
    "com1",
    "com2",
    "com3",
    "com4",
    "com5",
    "com6",
    "com7",
    "com8",
    "com9",
    "com\u{b9}",
    "com\u{b2}",
    "com\u{b3}",
    "lpt0",
    "lpt1",
    "lpt2",
    "lpt3",
    "lpt4",
    "lpt5",
    "lpt6",
    "lpt7",
    "lpt8",
    "lpt9",
    "lpt\u{b9}",
    "lpt\u{b2}",
    "lpt\u{b3}",
];

/// A name that is safe as a single path component on every platform: no
/// directory part, no separators, not hidden, no trailing dots or spaces.
pub fn safe_attachment_name(value: &str) -> String {
    let stripped = strip_filename_controls(value);
    let base = stripped
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
        .chars()
        .map(|ch| match ch {
            ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            other => other,
        })
        .collect::<String>();
    let base = base
        .trim_start_matches(|ch: char| ch == '.' || ch.is_whitespace())
        .trim_end_matches(|ch: char| ch == '.' || ch.is_whitespace());
    if base.is_empty() {
        return "attachment".to_string();
    }
    let stem = base.split('.').next().unwrap_or_default().trim_end();
    if RESERVED_DEVICE_NAMES
        .iter()
        .any(|reserved| stem.eq_ignore_ascii_case(reserved))
    {
        return format!("_{base}");
    }
    base.to_string()
}

pub fn deserialize_filename<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<String, D::Error> {
    let value = <String as serde::Deserialize>::deserialize(deserializer)?;
    Ok(safe_attachment_name(&value))
}

pub fn serialize_filename<S: serde::Serializer>(
    value: &str,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&safe_attachment_name(value))
}

pub fn serialize_optional_filename<S: serde::Serializer>(
    value: &Option<String>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match value {
        Some(value) => serializer.serialize_some(&safe_attachment_name(value)),
        None => serializer.serialize_none(),
    }
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

    #[test]
    fn attachment_names_reduce_to_a_plain_basename() {
        use super::safe_attachment_name as safe;
        assert_eq!(safe("/Users/x/Library/LaunchAgents/a.plist"), "a.plist");
        assert_eq!(safe("..\\..\\Startup\\run.bat"), "run.bat");
        assert_eq!(safe("C:evil.txt"), "C_evil.txt");
        assert_eq!(safe("Meeting 10:30.pdf"), "Meeting 10_30.pdf");
        assert_eq!(safe(".."), "attachment");
        assert_eq!(safe("a/.."), "attachment");
        assert_eq!(safe(".bashrc"), "bashrc");
        assert_eq!(safe("invoice.exe. . "), "invoice.exe");
        assert_eq!(safe("dir/"), "attachment");
        assert_eq!(safe("   "), "attachment");
        assert_eq!(safe("CON"), "_CON");
        assert_eq!(safe("nul.txt"), "_nul.txt");
        assert_eq!(safe("Com1 .tar.gz"), "_Com1 .tar.gz");
        assert_eq!(safe("console.log"), "console.log");
        assert_eq!(safe("Größe\u{202e}fdp.pdf"), "Größefdp.pdf");
    }

    #[test]
    fn serialized_names_are_basenames() {
        #[derive(serde::Serialize, serde::Deserialize)]
        struct Row {
            #[serde(
                serialize_with = "super::serialize_filename",
                deserialize_with = "super::deserialize_filename"
            )]
            name: String,
            #[serde(serialize_with = "super::serialize_optional_filename")]
            optional: Option<String>,
        }
        let json = serde_json::to_string(&Row {
            name: "../a.txt".into(),
            optional: Some("x/y.pdf".into()),
        })
        .unwrap();
        assert_eq!(json, r#"{"name":"a.txt","optional":"y.pdf"}"#);
        let row: Row = serde_json::from_str(r#"{"name":"/tmp/b.txt","optional":null}"#).unwrap();
        assert_eq!(row.name, "b.txt");
    }
}
