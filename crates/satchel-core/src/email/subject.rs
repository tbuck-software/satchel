/// Shared with the composer, which preserves prefix kind and correspondent spelling.
pub const REPLY_PREFIXES: &[&str] = &include!("reply-prefixes.json");
pub const FORWARD_PREFIXES: &[&str] = &include!("forward-prefixes.json");

/// Strip localized reply/forward chains while retaining the subject's display case.
pub fn normalized_subject(subject: &str) -> String {
    let mut rest = subject.trim();
    while let Some((label, tail)) = rest.split_once(':') {
        let mut token = label.trim();
        if token.ends_with(']') || token.ends_with(')') {
            let opener = if token.ends_with(']') { '[' } else { '(' };
            let Some((prefix, counter)) = token.split_once(opener) else {
                break;
            };
            let digits = counter[..counter.len() - 1].trim();
            if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                break;
            }
            token = prefix.trim();
        }
        let lower = token.to_lowercase();
        if !REPLY_PREFIXES.contains(&lower.as_str()) && !FORWARD_PREFIXES.contains(&lower.as_str())
        {
            break;
        }
        rest = tail.trim_start();
    }
    rest.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Threading compares the same display subject without case distinctions.
pub fn subject_key(subject: &str) -> String {
    normalized_subject(subject).to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_localized_prefix_has_the_same_display_and_threading_base() {
        for prefix in REPLY_PREFIXES.iter().chain(FORWARD_PREFIXES) {
            let subject = format!("{prefix}: Fw: SV: RE[2]: AW(3):  Quarterly  Plan ");
            assert_eq!(normalized_subject(&subject), "Quarterly Plan");
            assert_eq!(subject_key(&subject), "quarterly plan");
        }
    }

    #[test]
    fn preserves_unknown_labels_and_invalid_counters() {
        for value in [
            "R: Update",
            "Ref: Update",
            "Wichtig: lesen",
            "Re[x]: Update",
            "Re[2): Update",
        ] {
            assert_eq!(normalized_subject(value), value);
        }
        assert_eq!(normalized_subject("  RE :\u{a0}Facture  "), "Facture");
        assert_eq!(normalized_subject("Re: AW:"), "");
    }

    #[test]
    fn normalization_is_idempotent() {
        let once = normalized_subject("Odp: vs: Antw: Re: Über den Plan");
        assert_eq!(once, "Über den Plan");
        assert_eq!(normalized_subject(&once), once);
        assert_eq!(subject_key(&once), "über den plan");
    }
}
