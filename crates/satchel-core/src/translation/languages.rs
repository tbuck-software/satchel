//! Languages without a network round trip: DeepL's target list for when the
//! service cannot be asked, and a cheap guess at what a message is written in.
//!
//! The guess only decides whether Translate is worth offering. Once a message
//! has gone to an engine, the language the engine detected is the one shown.

use super::TranslationLanguage;

/// DeepL's text-translation targets as of 2026-11. Used before a key exists
/// and when `/v3/languages` cannot be read; the live list wins otherwise.
pub const DEEPL_TARGET_LANGUAGES: &[&str] = &[
    "AR", "BG", "CS", "DA", "DE", "EL", "EN-GB", "EN-US", "ES", "ET", "FI", "FR", "HE", "HU", "ID",
    "IT", "JA", "KO", "LT", "LV", "NB", "NL", "PL", "PT-BR", "PT-PT", "RO", "RU", "SK", "SL", "SV",
    "TH", "TR", "UK", "VI", "ZH-HANS", "ZH-HANT",
];

pub fn deepl_fallback_languages() -> Vec<TranslationLanguage> {
    DEEPL_TARGET_LANGUAGES
        .iter()
        .map(|code| TranslationLanguage {
            code: (*code).to_owned(),
            name: None,
        })
        .collect()
}

/// How much of a body the guess reads. Enough for a confident answer; a
/// newsletter's tenth paragraph changes nothing.
const SAMPLE_CHARS: usize = 2_000;

/// The language a plain-text body is most likely written in, as an upper
/// case ISO 639-1 code, or `None` when the guess is not reliable or names a
/// language no engine here translates from.
///
/// Quoted history (`>` lines) is skipped: a German reply to an English mail
/// is a German mail.
pub fn detect_language(text: &str) -> Option<&'static str> {
    let mut sample = String::with_capacity(SAMPLE_CHARS);
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('>') {
            continue;
        }
        if sample.chars().count() >= SAMPLE_CHARS {
            break;
        }
        sample.push_str(line);
        sample.push('\n');
    }
    let info = whatlang::detect(&sample)?;
    if !info.is_reliable() {
        return None;
    }
    iso_639_1(info.lang().code())
}

/// whatlang names languages in ISO 639-3; engines want ISO 639-1.
fn iso_639_1(code: &str) -> Option<&'static str> {
    Some(match code {
        "ara" => "AR",
        "bul" => "BG",
        "ces" => "CS",
        "dan" => "DA",
        "deu" => "DE",
        "ell" => "EL",
        "eng" => "EN",
        "spa" => "ES",
        "est" => "ET",
        "fin" => "FI",
        "fra" => "FR",
        "heb" => "HE",
        "hun" => "HU",
        "ind" => "ID",
        "ita" => "IT",
        "jpn" => "JA",
        "kor" => "KO",
        "lit" => "LT",
        "lav" => "LV",
        "nob" => "NB",
        "nld" => "NL",
        "pol" => "PL",
        "por" => "PT",
        "ron" => "RO",
        "rus" => "RU",
        "slk" => "SK",
        "slv" => "SL",
        "swe" => "SV",
        "tha" => "TH",
        "tur" => "TR",
        "ukr" => "UK",
        "vie" => "VI",
        "cmn" => "ZH",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_common_languages() {
        assert_eq!(
            detect_language(
                "Hallo zusammen, anbei die Unterlagen für die Besprechung am Montag. \
                 Bitte gebt mir bis Freitag Bescheid, ob der Termin für euch passt."
            ),
            Some("DE")
        );
        assert_eq!(
            detect_language(
                "Hi everyone, attached are the documents for Monday's meeting. \
                 Please let me know by Friday whether the time works for you."
            ),
            Some("EN")
        );
        assert_eq!(
            detect_language(
                "Bonjour à tous, vous trouverez ci-joint les documents pour la réunion \
                 de lundi. Merci de me dire avant vendredi si l'horaire vous convient."
            ),
            Some("FR")
        );
    }

    #[test]
    fn quoted_history_does_not_count() {
        let reply = "Danke, das passt mir gut. Ich komme am Montag um zehn Uhr vorbei \
                     und bringe die Unterlagen mit.\n\n\
                     > Hi everyone, attached are the documents for Monday's meeting.\n\
                     > Please let me know by Friday whether the time works for you.\n\
                     > Thanks a lot and have a nice weekend, see you all next week.";
        assert_eq!(detect_language(reply), Some("DE"));
    }

    #[test]
    fn too_little_text_is_no_answer() {
        assert_eq!(detect_language(""), None);
        assert_eq!(detect_language("ok"), None);
    }

    #[test]
    fn fallback_list_is_in_engine_shape() {
        for language in deepl_fallback_languages() {
            assert_eq!(
                super::super::normalize_language(&language.code).as_deref(),
                Some(language.code.as_str())
            );
        }
    }
}
