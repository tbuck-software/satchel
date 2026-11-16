//! Robinson–Fisher combining over per-token probabilities, the scheme
//! SpamBayes uses and Thunderbird's junk filter is modelled on.
//!
//! Each token gets Graham's ratio of how often it appeared in spam versus ham,
//! shrunk toward 0.5 by Robinson's prior so a token seen once does not decide
//! anything. The strongest clues are combined with Fisher's chi-squared test,
//! once for spam evidence and once for ham evidence; the score is where the
//! two land between 0 (ham) and 1 (spam). Two strong opposing signals give a
//! score near 0.5 — "unsure" — instead of a confident wrong answer.
//!
//! The gates around the score are deliberately strict. A false positive costs
//! a wanted mail; a missed spam costs a keypress. So:
//!
//! - nothing is judged before the model knows [`MIN_TRAINED_PER_CLASS`]
//!   messages of each class in a ratio no worse than [`MAX_CLASS_RATIO`];
//! - a token counts as a clue only after [`MIN_CLUE_SIGHTINGS`] sightings,
//!   scaled to the smaller class, so the absence of a word from thirty hams
//!   is not read as proof against it next to a thousand spams;
//! - Move needs a far higher score, a larger model, and a verdict that holds
//!   with every clue seen fewer than [`MIN_MOVE_CLUE_SIGHTINGS`] times left
//!   out ([`Score::move_confident`]).

/// Robinson's `s`: how much weight the prior gets against observed counts.
const UNKNOWN_WORD_STRENGTH: f64 = 0.45;
/// Robinson's `x`: the probability assumed for a token never seen.
const UNKNOWN_WORD_PROB: f64 = 0.5;
/// Tokens closer to 0.5 than this carry no information and are ignored.
const MINIMUM_PROB_STRENGTH: f64 = 0.1;
/// At most this many clues are combined, strongest first.
const MAX_DISCRIMINATORS: usize = 150;

/// At or above: likely spam, marked.
pub const SPAM_CUTOFF: f64 = 0.90;
/// At or above, with the other Move gates: moved to Spam.
pub const MOVE_CUTOFF: f64 = 0.99;
/// At or below: ham. Between the two the filter says nothing.
pub const HAM_CUTOFF: f64 = 0.20;
/// Below this many trained messages of either class nothing is judged.
pub const MIN_TRAINED_PER_CLASS: i64 = 50;
/// Below this many of either class Move behaves like Mark.
pub const MIN_MOVE_TRAINED_PER_CLASS: i64 = 200;
/// The larger class may be at most this many times the smaller one.
pub const MAX_CLASS_RATIO: i64 = 4;
/// A token seen fewer times than this is not a clue.
pub const MIN_CLUE_SIGHTINGS: f64 = 2.0;
/// Every clue a Move rests on was seen at least this often.
pub const MIN_MOVE_CLUE_SIGHTINGS: f64 = 3.0;

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
    pub fn balanced(self) -> bool {
        let (small, large) = (self.spam.min(self.ham), self.spam.max(self.ham));
        small > 0 && large <= small.saturating_mul(MAX_CLASS_RATIO)
    }

    /// Enough of both classes, in proportion, to give any verdict.
    pub fn ready(self) -> bool {
        self.spam >= MIN_TRAINED_PER_CLASS && self.ham >= MIN_TRAINED_PER_CLASS && self.balanced()
    }

    /// Enough of both classes to move mail on its own.
    pub fn move_ready(self) -> bool {
        self.spam >= MIN_MOVE_TRAINED_PER_CLASS
            && self.ham >= MIN_MOVE_TRAINED_PER_CLASS
            && self.balanced()
    }

    /// How many more `(spam, ham)` examples it takes to reach `floor` of each
    /// with the classes in proportion.
    pub fn needed(self, floor: i64) -> (i64, i64) {
        let spam = self.spam.max(0);
        let ham = self.ham.max(0);
        let spam_target = floor.max((ham + MAX_CLASS_RATIO - 1) / MAX_CLASS_RATIO);
        let ham_target = floor.max((spam + MAX_CLASS_RATIO - 1) / MAX_CLASS_RATIO);
        ((spam_target - spam).max(0), (ham_target - ham).max(0))
    }
}

/// What the evidence behind one token amounts to: sightings scaled to the
/// smaller class, so a lopsided model does not inflate one side.
pub fn token_sightings(spam_count: i64, ham_count: i64, totals: ModelTotals) -> f64 {
    let spam_total = totals.spam.max(1) as f64;
    let ham_total = totals.ham.max(1) as f64;
    let smaller = spam_total.min(ham_total);
    spam_count.max(0) as f64 * smaller / spam_total + ham_count.max(0) as f64 * smaller / ham_total
}

/// The smoothed spam probability of one token.
pub fn token_probability(spam_count: i64, ham_count: i64, totals: ModelTotals) -> f64 {
    let n = token_sightings(spam_count, ham_count, totals);
    if n == 0.0 {
        return UNKNOWN_WORD_PROB;
    }
    let spam_ratio = spam_count.max(0) as f64 / totals.spam.max(1) as f64;
    let ham_ratio = ham_count.max(0) as f64 / totals.ham.max(1) as f64;
    let raw = if spam_ratio + ham_ratio == 0.0 {
        UNKNOWN_WORD_PROB
    } else {
        spam_ratio / (spam_ratio + ham_ratio)
    };
    (UNKNOWN_WORD_STRENGTH * UNKNOWN_WORD_PROB + n * raw) / (UNKNOWN_WORD_STRENGTH + n)
}

/// A message's score and what it allows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Score {
    pub score: f64,
    pub verdict: Verdict,
    /// At least [`MOVE_CUTOFF`], and still there with every clue seen fewer
    /// than [`MIN_MOVE_CLUE_SIGHTINGS`] times taken out. The model size gate
    /// ([`ModelTotals::move_ready`]) is the caller's.
    pub move_confident: bool,
}

/// Score a message from the counts of its tokens.
///
/// `counts` holds `(spam_count, ham_count)` for every token of the message,
/// unknown ones as `(0, 0)`.
pub fn evaluate(counts: &[(i64, i64)], totals: ModelTotals) -> Score {
    let value = combine(counts, totals, MIN_CLUE_SIGHTINGS);
    let move_confident =
        value >= MOVE_CUTOFF && combine(counts, totals, MIN_MOVE_CLUE_SIGHTINGS) >= MOVE_CUTOFF;
    Score {
        score: value,
        verdict: Verdict::from_score(value),
        move_confident,
    }
}

/// The score alone: 0 is ham, 1 is spam.
pub fn score(counts: &[(i64, i64)], totals: ModelTotals) -> f64 {
    combine(counts, totals, MIN_CLUE_SIGHTINGS)
}

fn combine(counts: &[(i64, i64)], totals: ModelTotals, min_sightings: f64) -> f64 {
    let mut clues: Vec<f64> = counts
        .iter()
        .filter(|(spam, ham)| token_sightings(*spam, *ham, totals) >= min_sightings)
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
    fn robinson_prior_damps_rare_tokens() {
        let twice = token_probability(2, 0, TOTALS);
        let often = token_probability(40, 0, TOTALS);
        // (0.45 · 0.5 + 2 · 1) / (0.45 + 2): leaning spam, not certain.
        assert!((twice - 0.908).abs() < 0.001, "{twice}");
        assert!(often > 0.98, "{often}");
        assert!(token_probability(0, 40, TOTALS) < 0.02);
    }

    #[test]
    fn tokens_seen_once_are_not_clues() {
        // Three once-seen spam-only tokens used to score 0.95 on a 10/10 model.
        let thin = ModelTotals { spam: 10, ham: 10 };
        assert_eq!(score(&[(1, 0), (1, 0), (1, 0)], thin), 0.5);
        assert_eq!(score(&[(1, 0); 40], TOTALS), 0.5);
    }

    #[test]
    fn strong_clues_decide_and_mixed_clues_are_unsure() {
        let spammy = vec![(40, 0); 20];
        let hammy = vec![(0, 40); 20];
        assert_eq!(evaluate(&spammy, TOTALS).verdict, Verdict::Spam);
        assert_eq!(evaluate(&hammy, TOTALS).verdict, Verdict::Ham);
        let mixed: Vec<_> = spammy.iter().chain(hammy.iter()).copied().collect();
        assert_eq!(evaluate(&mixed, TOTALS).verdict, Verdict::Unsure);
    }

    #[test]
    fn a_lopsided_model_does_not_convict_ordinary_words() {
        // Ordinary words in 30 of 1000 spams and in none of 30 hams scored
        // 1.000 before sightings were scaled to the smaller class.
        let skewed = ModelTotals {
            spam: 1_000,
            ham: 30,
        };
        assert!(!skewed.ready());
        let ordinary = vec![(30, 0); 25];
        let value = score(&ordinary, skewed);
        assert!(value < SPAM_CUTOFF, "{value}");
    }

    #[test]
    fn move_needs_clues_seen_three_times() {
        let totals = ModelTotals {
            spam: 300,
            ham: 300,
        };
        let strong = vec![(60, 0); 12];
        assert!(evaluate(&strong, totals).move_confident);
        // The same score carried by twice-seen clues marks but does not move.
        let thin = vec![(2, 0); 60];
        let thin_score = evaluate(&thin, totals);
        assert_eq!(thin_score.verdict, Verdict::Spam);
        assert!(!thin_score.move_confident, "{thin_score:?}");
    }

    #[test]
    fn gates_bound_the_verdicts() {
        assert_eq!(Verdict::from_score(SPAM_CUTOFF), Verdict::Spam);
        assert_eq!(Verdict::from_score(SPAM_CUTOFF - 0.01), Verdict::Unsure);
        assert_eq!(Verdict::from_score(HAM_CUTOFF), Verdict::Ham);
        assert_eq!(Verdict::from_score(HAM_CUTOFF + 0.01), Verdict::Unsure);
        assert!(!ModelTotals { spam: 10, ham: 10 }.ready());
        assert!(!ModelTotals { spam: 49, ham: 100 }.ready());
        assert!(
            !ModelTotals { spam: 50, ham: 201 }.ready(),
            "worse than 4:1"
        );
        assert!(ModelTotals { spam: 50, ham: 200 }.ready());
        assert!(!ModelTotals {
            spam: 199,
            ham: 400
        }
        .move_ready());
        assert!(ModelTotals {
            spam: 200,
            ham: 400
        }
        .move_ready());
    }

    #[test]
    fn needed_counts_the_floor_and_the_ratio() {
        assert_eq!(ModelTotals { spam: 10, ham: 0 }.needed(50), (40, 50));
        assert_eq!(ModelTotals { spam: 60, ham: 400 }.needed(50), (40, 0));
        assert_eq!(ModelTotals { spam: 60, ham: 70 }.needed(50), (0, 0));
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
