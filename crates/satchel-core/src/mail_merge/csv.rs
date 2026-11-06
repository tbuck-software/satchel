//! Recipient lists from CSV files, as spreadsheets export them.
//!
//! Excel writes `;` in German locales and `,` elsewhere, LibreOffice asks, and
//! older Excel saves Windows-1252 unless told otherwise. The header row names
//! the fields; nothing else about the file is assumed.

use std::collections::BTreeMap;

use serde::Serialize;

/// Larger files are not recipient lists. The job limit is far below this.
pub const MAX_CSV_BYTES: usize = 5 * 1024 * 1024;
/// Rows read from one file. More than the job limit, so the user sees that the
/// list is too long instead of a silently cut one.
pub const MAX_CSV_ROWS: usize = 5000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "core/mail_merge/"))]
#[serde(rename_all = "snake_case")]
pub enum CsvEncoding {
    Utf8,
    Utf8Bom,
    Utf16,
    Windows1252,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "core/mail_merge/"))]
pub struct CsvRow {
    /// 1-based line in the file where the row starts, for "row 14 is invalid".
    pub line: u32,
    pub values: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "core/mail_merge/"))]
pub struct ParsedCsv {
    pub fields: Vec<String>,
    pub rows: Vec<CsvRow>,
    /// The column that holds addresses, when one could be told from the
    /// header or the values. The user can pick another.
    pub email_field: Option<String>,
    /// `,`, `;` or a tab.
    pub delimiter: String,
    pub encoding: CsvEncoding,
    /// More rows than [`MAX_CSV_ROWS`] were in the file; the rest was not read.
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CsvError {
    #[error("the file is empty")]
    Empty,
    #[error("the file is too large")]
    TooLarge,
    #[error("the file has a header row but no recipients")]
    NoRows,
    #[error("a quoted value is not closed")]
    UnterminatedQuote { line: u32 },
}

/// Decodes the bytes, finds the delimiter, and reads header and rows.
pub fn parse_csv(bytes: &[u8]) -> Result<ParsedCsv, CsvError> {
    if bytes.len() > MAX_CSV_BYTES {
        return Err(CsvError::TooLarge);
    }
    let (text, encoding) = decode(bytes);
    if text.trim().is_empty() {
        return Err(CsvError::Empty);
    }
    let delimiter = detect_delimiter(&text);
    let records = split_records(&text, delimiter)?;
    let mut records = records
        .into_iter()
        .filter(|(_, cells)| cells.iter().any(|cell| !cell.trim().is_empty()));
    let Some((_, header)) = records.next() else {
        return Err(CsvError::Empty);
    };
    let fields = field_names(&header);

    let mut rows = Vec::new();
    let mut truncated = false;
    for (line, cells) in records {
        if rows.len() == MAX_CSV_ROWS {
            truncated = true;
            break;
        }
        let values = fields
            .iter()
            .enumerate()
            .map(|(index, field)| {
                let value = cells.get(index).map(|cell| cell.trim()).unwrap_or("");
                (field.clone(), value.to_string())
            })
            .collect();
        rows.push(CsvRow { line, values });
    }
    if rows.is_empty() {
        return Err(CsvError::NoRows);
    }
    let email_field = detect_email_field(&fields, &rows);
    Ok(ParsedCsv {
        fields,
        rows,
        email_field,
        delimiter: delimiter.to_string(),
        encoding,
        truncated,
    })
}

fn decode(bytes: &[u8]) -> (String, CsvEncoding) {
    if let Some(rest) = bytes.strip_prefix(b"\xEF\xBB\xBF") {
        return (
            String::from_utf8_lossy(rest).into_owned(),
            CsvEncoding::Utf8Bom,
        );
    }
    if bytes.starts_with(b"\xFF\xFE") || bytes.starts_with(b"\xFE\xFF") {
        // Excel's "Unicode text" export. The BOM picks the byte order.
        let (text, _, _) = encoding_rs::UTF_16LE.decode(bytes);
        return (text.into_owned(), CsvEncoding::Utf16);
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => (text.to_string(), CsvEncoding::Utf8),
        Err(_) => {
            let (text, _, _) = encoding_rs::WINDOWS_1252.decode(bytes);
            (text.into_owned(), CsvEncoding::Windows1252)
        }
    }
}

/// The candidate that splits the header row into the most cells. A header
/// such as `Name;Firma, Abteilung;E-Mail` still picks `;`: commas inside a
/// field name are rarer than the delimiter between every pair of them.
fn detect_delimiter(text: &str) -> char {
    let header = first_record_text(text);
    let mut best = (',', 0usize);
    for candidate in [',', ';', '\t'] {
        let count = count_unquoted(header, candidate);
        if count > best.1 {
            best = (candidate, count);
        }
    }
    best.0
}

fn first_record_text(text: &str) -> &str {
    let mut quoted = false;
    for (index, character) in text.char_indices() {
        match character {
            '"' => quoted = !quoted,
            '\n' | '\r' if !quoted => return &text[..index],
            _ => {}
        }
    }
    text
}

fn count_unquoted(text: &str, needle: char) -> usize {
    let mut quoted = false;
    let mut count = 0;
    for character in text.chars() {
        if character == '"' {
            quoted = !quoted;
        } else if character == needle && !quoted {
            count += 1;
        }
    }
    count
}

/// RFC 4180 records: quoted cells may hold the delimiter, doubled quotes and
/// line breaks. `\r\n`, `\n` and a lone `\r` all end a record.
fn split_records(text: &str, delimiter: char) -> Result<Vec<(u32, Vec<String>)>, CsvError> {
    let mut records = Vec::new();
    let mut cells = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    let mut line: u32 = 1;
    let mut record_line: u32 = 1;
    let mut quote_line: u32 = 1;
    let mut chars = text.chars().peekable();
    while let Some(character) = chars.next() {
        if quoted {
            match character {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    cell.push('"');
                }
                '"' => quoted = false,
                '\n' => {
                    line += 1;
                    cell.push('\n');
                }
                '\r' => {
                    if chars.peek() == Some(&'\n') {
                        chars.next();
                    }
                    line += 1;
                    cell.push('\n');
                }
                other => cell.push(other),
            }
            continue;
        }
        match character {
            '"' if cell.trim().is_empty() => {
                cell.clear();
                quoted = true;
                quote_line = line;
            }
            c if c == delimiter => cells.push(std::mem::take(&mut cell)),
            '\n' | '\r' => {
                if character == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                cells.push(std::mem::take(&mut cell));
                records.push((record_line, std::mem::take(&mut cells)));
                line += 1;
                record_line = line;
            }
            other => cell.push(other),
        }
    }
    if quoted {
        return Err(CsvError::UnterminatedQuote { line: quote_line });
    }
    if !cell.is_empty() || !cells.is_empty() {
        cells.push(cell);
        records.push((record_line, cells));
    }
    Ok(records)
}

/// Trimmed header names. An empty one becomes `Column3`, a repeated one
/// `Email_2`, so every field can still be told apart in a placeholder.
fn field_names(header: &[String]) -> Vec<String> {
    let mut names: Vec<String> = Vec::with_capacity(header.len());
    for (index, raw) in header.iter().enumerate() {
        let base = sanitize_field_name(raw);
        let base = if base.is_empty() {
            format!("Column{}", index + 1)
        } else {
            base
        };
        let mut name = base.clone();
        let mut suffix = 2;
        while names
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(&name))
        {
            name = format!("{base}_{suffix}");
            suffix += 1;
        }
        names.push(name);
    }
    names
}

/// Characters a `{{…}}` token cannot carry are dropped from the name.
fn sanitize_field_name(raw: &str) -> String {
    raw.chars()
        .filter(|character| !matches!(character, '{' | '}' | '|') && !character.is_control())
        .collect::<String>()
        .trim()
        .to_string()
}

const EMAIL_HEADERS: &[&str] = &[
    "email",
    "e-mail",
    "emailaddress",
    "e-mailaddress",
    "e-mailadresse",
    "emailadresse",
    "mail",
    "mailadresse",
    "primaryemail",
    "emailaddress1",
];

fn normalized_header(name: &str) -> String {
    name.chars()
        .filter(|character| !character.is_whitespace() && *character != '_')
        .flat_map(char::to_lowercase)
        .collect()
}

fn detect_email_field(fields: &[String], rows: &[CsvRow]) -> Option<String> {
    if let Some(field) = fields
        .iter()
        .find(|field| EMAIL_HEADERS.contains(&normalized_header(field).as_str()))
    {
        return Some(field.clone());
    }
    // No telling header: the first column whose filled values are addresses.
    fields
        .iter()
        .find(|field| {
            let mut filled = rows
                .iter()
                .take(50)
                .filter_map(|row| row.values.get(*field))
                .filter(|value| !value.is_empty())
                .peekable();
            filled.peek().is_some()
                && filled.all(|value| super::plan::normalize_address(value).is_some())
        })
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(parsed: &ParsedCsv, row: usize, field: &str) -> String {
        parsed.rows[row]
            .values
            .get(field)
            .cloned()
            .unwrap_or_default()
    }

    #[test]
    fn reads_comma_separated_utf8() {
        let parsed = parse_csv(b"FirstName,Email\nAnna,anna@example.com\nBen,ben@example.com\n")
            .expect("parse");
        assert_eq!(parsed.fields, ["FirstName", "Email"]);
        assert_eq!(parsed.delimiter, ",");
        assert_eq!(parsed.encoding, CsvEncoding::Utf8);
        assert_eq!(parsed.rows.len(), 2);
        assert_eq!(value(&parsed, 1, "FirstName"), "Ben");
        assert_eq!(parsed.email_field.as_deref(), Some("Email"));
    }

    #[test]
    fn reads_semicolons_and_the_utf8_bom_excel_writes() {
        let parsed =
            parse_csv("\u{feff}Vorname;Nachname;E-Mail\r\nJürgen;Groß;j@example.de\r\n".as_bytes())
                .expect("parse");
        assert_eq!(parsed.delimiter, ";");
        assert_eq!(parsed.encoding, CsvEncoding::Utf8Bom);
        assert_eq!(parsed.fields, ["Vorname", "Nachname", "E-Mail"]);
        assert_eq!(value(&parsed, 0, "Nachname"), "Groß");
        assert_eq!(parsed.email_field.as_deref(), Some("E-Mail"));
    }

    #[test]
    fn falls_back_to_windows_1252() {
        // "Jürgen;Müller" as Windows-1252: ü is 0xFC, not valid UTF-8 alone.
        let bytes = b"Name;Mail\nJ\xFCrgen M\xFCller;j@example.de\n";
        let parsed = parse_csv(bytes).expect("parse");
        assert_eq!(parsed.encoding, CsvEncoding::Windows1252);
        assert_eq!(value(&parsed, 0, "Name"), "Jürgen Müller");
        assert_eq!(parsed.email_field.as_deref(), Some("Mail"));
    }

    #[test]
    fn reads_utf16_with_tabs() {
        let text = "Name\tEmail\r\nZoë\tz@example.com\r\n";
        let mut bytes = vec![0xFF, 0xFE];
        for unit in text.encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        let parsed = parse_csv(&bytes).expect("parse");
        assert_eq!(parsed.encoding, CsvEncoding::Utf16);
        assert_eq!(parsed.delimiter, "\t");
        assert_eq!(value(&parsed, 0, "Name"), "Zoë");
    }

    #[test]
    fn quoted_cells_keep_delimiters_quotes_and_line_breaks() {
        let parsed = parse_csv(
            b"Name,Company,Email\n\"Meier, Anna\",\"ACME \"\"Tools\"\"\nBerlin\",a@x.de\nBen,,b@x.de\n",
        )
        .expect("parse");
        assert_eq!(parsed.rows.len(), 2);
        assert_eq!(value(&parsed, 0, "Name"), "Meier, Anna");
        assert_eq!(value(&parsed, 0, "Company"), "ACME \"Tools\"\nBerlin");
        // The second record starts on line 4: the quoted break counts.
        assert_eq!(parsed.rows[1].line, 4);
        assert_eq!(value(&parsed, 1, "Company"), "");
    }

    #[test]
    fn names_empty_and_repeated_headers_and_skips_blank_lines() {
        let parsed =
            parse_csv(b"Email,,email,{Name}\n\na@x.de,1,b@x.de,Anna\n;;;\n").expect("parse");
        assert_eq!(parsed.fields, ["Email", "Column2", "email_2", "Name"]);
        // `;;;` is one non-empty cell under a comma delimiter, so it stays a row.
        assert_eq!(parsed.rows.len(), 2);
        assert_eq!(parsed.rows[0].line, 3);
        assert_eq!(value(&parsed, 0, "Name"), "Anna");
    }

    #[test]
    fn finds_the_address_column_by_its_values() {
        let parsed = parse_csv(b"Who,Contact\nAnna,anna@example.com\nBen,\n").expect("parse");
        assert_eq!(parsed.email_field.as_deref(), Some("Contact"));
        let none = parse_csv(b"Who,Where\nAnna,Berlin\n").expect("parse");
        assert_eq!(none.email_field, None);
    }

    #[test]
    fn rejects_empty_headless_and_broken_files() {
        assert_eq!(parse_csv(b"  \n"), Err(CsvError::Empty));
        assert_eq!(parse_csv(b"Name,Email\n"), Err(CsvError::NoRows));
        assert_eq!(
            parse_csv(b"Name,Email\n\"Anna,a@x.de\n"),
            Err(CsvError::UnterminatedQuote { line: 2 })
        );
        assert_eq!(
            parse_csv(&vec![b'a'; MAX_CSV_BYTES + 1]),
            Err(CsvError::TooLarge)
        );
    }

    #[test]
    fn stops_reading_at_the_row_limit() {
        let mut text = String::from("Email\n");
        for index in 0..=MAX_CSV_ROWS {
            text.push_str(&format!("p{index}@example.com\n"));
        }
        let parsed = parse_csv(text.as_bytes()).expect("parse");
        assert_eq!(parsed.rows.len(), MAX_CSV_ROWS);
        assert!(parsed.truncated);
    }
}
