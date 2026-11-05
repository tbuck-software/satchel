//! Robinson–Fisher combining over per-token probabilities, the scheme
//! SpamBayes uses and Thunderbird's junk filter is modelled on.
//!
//! Each token gets Graham's ratio of how often it appeared in spam versus ham,
//! shrunk toward 0.5 by Robinson's prior so a token seen once does not decide
//! anything. The strongest clues are combined with Fisher's chi-squared test,
//! once for spam evidence and once for ham evidence; the score is where the
//! two land between 0 (ham) and 1 (spam). Two strong opposing signals give a
//! score near 0.5 — "unsure" — instead of a confident wrong answer.

/// Robinson's `s`: how much weight the prior gets against observed counts.
const UNKNOWN_WORD_STRENGTH: f64 = 0.45;
/// Robinson's `x`: the probability assumed for a token never seen.
const UNKNOWN_WORD_PROB: f64 = 0.5;
/// Tokens closer to 0.5 than this carry no information and are ignored.
const MINIMUM_PROB_STRENGTH: f64 = 0.1;
/// At most this many clues are combined, strongest first.
const MAX_DISCRIMINATORS: usize = 150;

/// At or above: likely spam.
pub const SPAM_CUTOFF: f64 = 0.90;
/// At or below: ham. Between the two the filter says nothing.
pub const HAM_CUTOFF: f64 = 0.20;
/// Below this many trained messages of either class the filter does not
/// classify: a model that has seen three spams calls everything spam.
pub const MIN_TRAINED_PER_CLASS: i64 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Spam,
    Unsure,
    Ham,
}

impl Verdict {
    pub fn from_score(score: f64) -> Self {
        if score >= SPAM_CUTOFF {
            Self::Spam
        } else if score <= HAM_CUTOFF {
            Self::Ham
        } else {
            Self::Unsure
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Spam => "spam",
            Self::Unsure => "unsure",
            Self::Ham => "ham",
        }
    }
}

/// How many messages of each class the model was trained on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ModelTotals {
    pub spam: i64,
    pub ham: i64,
}

impl ModelTotals {
    pub fn ready(self) -> bool {
        self.spam >= MIN_TRAINED_PER_CLASS && self.ham >= MIN_TRAINED_PER_CLASS
    }
}

/// The smoothed spam probability of one token.
pub fn token_probability(spam_count: i64, ham_count: i64, totals: ModelTotals) -> f64 {
    let spam_count = spam_count.max(0) as f64;
    let ham_count = ham_count.max(0) as f64;
    let n = spam_count + ham_count;
    if n == 0.0 {
        return UNKNOWN_WORD_PROB;
    }
    let spam_ratio = spam_count / totals.spam.max(1) as f64;
    let ham_ratio = ham_count / totals.ham.max(1) as f64;
    let raw = if spam_ratio + ham_ratio == 0.0 {
        UNKNOWN_WORD_PROB
    } else {
        spam_ratio / (spam_ratio + ham_ratio)
    };
    (UNKNOWN_WORD_STRENGTH * UNKNOWN_WORD_PROB + n * raw) / (UNKNOWN_WORD_STRENGTH + n)
}

/// Score a message from the counts of its tokens: 0 is ham, 1 is spam.
///
/// `counts` holds `(spam_count, ham_count)` for every token of the message,
/// unknown ones as `(0, 0)`.
pub fn score(counts: &[(i64, i64)], totals: ModelTotals) -> f64 {
    let mut clues: Vec<f64> = counts
        .iter()
        .map(|(spam, ham)| token_probability(*spam, *ham, totals))
        .filter(|probability| (probability - 0.5).abs() >= MINIMUM_PROB_STRENGTH)
        .collect();
    if clues.is_empty() {
        return 0.5;
    }
    clues.sort_by(|a, b| {
        (b - 0.5)
            .abs()
            .partial_cmp(&(a - 0.5).abs())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    clues.truncate(MAX_DISCRIMINATORS);
    let clamp = |p: f64| p.clamp(1e-9, 1.0 - 1e-9);
    let ln_ham: f64 = clues.iter().map(|p| clamp(*p).ln()).sum();
    let ln_spam: f64 = clues.iter().map(|p| (1.0 - clamp(*p)).ln()).sum();
    let degrees = 2 * clues.len();
    let spam = 1.0 - chi2q(-2.0 * ln_spam, degrees);
    let ham = 1.0 - chi2q(-2.0 * ln_ham, degrees);
    ((spam - ham + 1.0) / 2.0).clamp(0.0, 1.0)
}

/// Probability that a chi-squared variable with `degrees` (even) degrees of
/// freedom is at least `x2`.
fn chi2q(x2: f64, degrees: usize) -> f64 {
    let m = x2 / 2.0;
    let mut term = (-m).exp();
    let mut sum = term;
    for i in 1..degrees / 2 {
        term *= m / i as f64;
        sum += term;
    }
    sum.min(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOTALS: ModelTotals = ModelTotals { spam: 50, ham: 50 };

    #[test]
    fn unknown_and_balanced_tokens_say_nothing() {
        assert_eq!(score(&[], TOTALS), 0.5);
        assert_eq!(score(&[(0, 0), (0, 0)], TOTALS), 0.5);
        assert_eq!(score(&[(10, 10)], TOTALS), 0.5);
    }

    #[test]
    fn robinson_prior_damps_a_single_sighting() {
        let once = token_probability(1, 0, TOTALS);
        let often = token_probability(40, 0, TOTALS);
        // (0.45 · 0.5 + 1 · 1) / (0.45 + 1): leaning spam, not certain.
        assert!((once - 0.845).abs() < 0.001, "{once}");
        assert!(often > 0.98, "{often}");
        assert!(token_probability(0, 40, TOTALS) < 0.02);
    }

    #[test]
    fn strong_clues_decide_and_mixed_clues_are_unsure() {
        let spammy = vec![(40, 0); 20];
        let hammy = vec![(0, 40); 20];
        assert_eq!(Verdict::from_score(score(&spammy, TOTALS)), Verdict::Spam);
        assert_eq!(Verdict::from_score(score(&hammy, TOTALS)), Verdict::Ham);
        let mixed: Vec<_> = spammy.iter().chain(hammy.iter()).copied().collect();
        assert_eq!(Verdict::from_score(score(&mixed, TOTALS)), Verdict::Unsure);
    }

    #[test]
    fn class_sizes_normalize_counts() {
        // Seen in 10 of 10 spams and 10 of 1000 hams: a strong spam clue.
        let totals = ModelTotals {
            spam: 10,
            ham: 1_000,
        };
        assert!(token_probability(10, 10, totals) > 0.95);
    }

    #[test]
    fn thresholds_bound_the_verdicts() {
        assert_eq!(Verdict::from_score(SPAM_CUTOFF), Verdict::Spam);
        assert_eq!(Verdict::from_score(SPAM_CUTOFF - 0.01), Verdict::Unsure);
        assert_eq!(Verdict::from_score(HAM_CUTOFF), Verdict::Ham);
        assert_eq!(Verdict::from_score(HAM_CUTOFF + 0.01), Verdict::Unsure);
        assert!(!ModelTotals { spam: 9, ham: 100 }.ready());
        assert!(ModelTotals { spam: 10, ham: 10 }.ready());
    }

    #[test]
    fn many_clues_do_not_underflow() {
        let spammy = vec![(500, 1); 1_000];
        let value = score(
            &spammy,
            ModelTotals {
                spam: 500,
                ham: 500,
            },
        );
        assert!(value.is_finite() && value > 0.99, "{value}");
    }
}
