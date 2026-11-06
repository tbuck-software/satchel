//! Done and Snooze as the mail server stores them (ADR 0009).
//!
//! | State           | IMAP keywords                               | Gmail labels                                       |
//! |-----------------|---------------------------------------------|----------------------------------------------------|
//! | Done            | `$Satchel-Done`                             | `Satchel/Done`                                     |
//! | Snoozed until t | `$Satchel-Snoozed` (+ `$Satchel-Snooze2`)   | `Satchel/Snoozed` `Satchel/Wake/20261107T080000Z`  |
//!
//! IMAP has a **fixed vocabulary of three keywords**, never one per time.
//! Servers bound the keywords a mailbox can hold: Dovecot's Maildir encodes
//! only 26 in the file name (letters a–z, the rest live in its index), and
//! Cyrus has a hard limit of 128 user flags per mailbox that are never
//! recycled once used. A keyword per wake time would exhaust either.
//! So the wake time does not travel on IMAP; `$Satchel-Snooze2` only tells
//! one snooze from the next (it toggles with each new snooze), which is what
//! a wake on another device needs to clear exactly the snooze it knew.
//!
//! A device that sees an IMAP snooze it did not set adopts it with
//! [`ADOPTED_SNOOZE_FALLBACK`] as its wake time; the device that set it
//! clears the keywords when it wakes the thread. The exact wake time belongs
//! to the hub's snooze alarm (ADR 0009). Gmail labels can be deleted again,
//! so Gmail carries the time as a label and removes it after it has passed.
//!
//! Done also moves the mail to Archive, as before.

use chrono::{DateTime, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};

use super::states::WorkflowState;

pub const IMAP_KEYWORD_DONE: &str = "$Satchel-Done";
pub const IMAP_KEYWORD_SNOOZED: &str = "$Satchel-Snoozed";
pub const IMAP_KEYWORD_SNOOZE_SECOND: &str = "$Satchel-Snooze2";
/// Every keyword Satchel ever writes. Anything else on a message belongs
/// to the person or another client and is never touched.
pub const IMAP_KEYWORDS: [&str; 3] = [
    IMAP_KEYWORD_DONE,
    IMAP_KEYWORD_SNOOZED,
    IMAP_KEYWORD_SNOOZE_SECOND,
];

pub const GMAIL_LABEL_DONE: &str = "Satchel/Done";
pub const GMAIL_LABEL_SNOOZED: &str = "Satchel/Snoozed";
pub const GMAIL_LABEL_WAKE_PREFIX: &str = "Satchel/Wake/";

/// How long a device keeps a snooze it learned from IMAP, where the wake
/// time does not travel. Short on purpose: waking a thread a little early is
/// a nuisance, hiding mail for days because the snoozing device went away
/// is worse. The snoozing device clears the keyword at the real time.
pub const ADOPTED_SNOOZE_FALLBACK: chrono::Duration = chrono::Duration::hours(24);

const WAKE_FORMAT: &str = "%Y%m%dT%H%M%SZ";

/// A snooze as the server holds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Snooze {
    /// Known when this device set the snooze, and always on Gmail. `None`
    /// for an IMAP snooze set elsewhere.
    pub wake: Option<DateTime<Utc>>,
    /// 1 or 2; a new IMAP snooze takes the other one than the last seen.
    pub generation: u8,
}

/// What the server holds for a thread's workflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub enum WorkflowMarker {
    /// Neither Done nor Snoozed: Inbox, or a state the server cannot express.
    None,
    Done,
    Snoozed(Snooze),
}

impl WorkflowMarker {
    /// The marker a local state is pushed as. A snooze without a readable
    /// wake time has nothing to carry and is pushed as no marker.
    pub fn for_state(state: WorkflowState, snooze_until: Option<&str>) -> Self {
        match state {
            WorkflowState::Done => Self::Done,
            WorkflowState::Snoozed => snooze_until
                .and_then(parse_rfc3339)
                .map(|wake| {
                    Self::Snoozed(Snooze {
                        wake: Some(truncate_to_second(wake)),
                        generation: 1,
                    })
                })
                .unwrap_or(Self::None),
            _ => Self::None,
        }
    }

    /// The snooze generation a new push takes after `last_seen`: the other
    /// one, so every device can tell this snooze from the previous one.
    pub fn after(self, last_seen: &WorkflowMarker) -> Self {
        match self {
            Self::Snoozed(snooze) => Self::Snoozed(Snooze {
                generation: match last_seen {
                    Self::Snoozed(Snooze { generation: 1, .. }) => 2,
                    _ => 1,
                },
                ..snooze
            }),
            other => other,
        }
    }

    /// `''`, `done` or `snoozed:<generation>:<RFC 3339 or empty>`.
    pub fn as_db_value(&self) -> String {
        match self {
            Self::None => String::new(),
            Self::Done => "done".into(),
            Self::Snoozed(snooze) => format!(
                "snoozed:{}:{}",
                snooze.generation,
                snooze.wake.map(|at| at.to_rfc3339()).unwrap_or_default()
            ),
        }
    }

    /// The stored column value: `NULL` for no marker, so a mailbox that
    /// never saw one is never written.
    pub fn as_column(&self) -> Option<String> {
        Some(self.as_db_value()).filter(|value| !value.is_empty())
    }

    pub fn from_db_value(value: &str) -> Option<Self> {
        let value = value.trim();
        if value.is_empty() {
            return Some(Self::None);
        }
        if value == "done" {
            return Some(Self::Done);
        }
        let rest = value.strip_prefix("snoozed:")?;
        let (generation, wake) = rest.split_once(':')?;
        let generation = generation
            .parse::<u8>()
            .ok()
            .filter(|g| matches!(g, 1 | 2))?;
        let wake = if wake.is_empty() {
            None
        } else {
            Some(parse_rfc3339(wake)?)
        };
        Some(Self::Snoozed(Snooze { wake, generation }))
    }

    /// Read a stored column: `NULL` and anything unreadable are no marker.
    pub fn from_column(value: Option<&str>) -> Self {
        value.and_then(Self::from_db_value).unwrap_or(Self::None)
    }

    pub fn imap_keywords(&self) -> Vec<String> {
        match self {
            Self::None => Vec::new(),
            Self::Done => vec![IMAP_KEYWORD_DONE.into()],
            Self::Snoozed(Snooze { generation: 2, .. }) => vec![
                IMAP_KEYWORD_SNOOZED.into(),
                IMAP_KEYWORD_SNOOZE_SECOND.into(),
            ],
            Self::Snoozed(_) => vec![IMAP_KEYWORD_SNOOZED.into()],
        }
    }

    pub fn gmail_labels(&self) -> Vec<String> {
        match self {
            Self::None => Vec::new(),
            Self::Done => vec![GMAIL_LABEL_DONE.into()],
            Self::Snoozed(snooze) => {
                let mut labels = vec![GMAIL_LABEL_SNOOZED.to_string()];
                if let Some(wake) = snooze.wake {
                    labels.push(format!(
                        "{GMAIL_LABEL_WAKE_PREFIX}{}",
                        wake.format(WAKE_FORMAT)
                    ));
                }
                labels
            }
        }
    }
}

impl From<WorkflowMarker> for String {
    fn from(value: WorkflowMarker) -> Self {
        value.as_db_value()
    }
}

impl TryFrom<String> for WorkflowMarker {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::from_db_value(&value).ok_or_else(|| format!("unknown workflow marker {value:?}"))
    }
}

/// Whether two markers describe the same server state. An IMAP echo of our
/// own snooze has no wake time; it is the same snooze when its generation
/// matches.
pub fn same_on_server(a: &WorkflowMarker, b: &WorkflowMarker) -> bool {
    match (a, b) {
        (WorkflowMarker::Snoozed(a), WorkflowMarker::Snoozed(b)) => {
            a.generation == b.generation
                && (a.wake.is_none() || b.wake.is_none() || a.wake == b.wake)
        }
        _ => a == b,
    }
}

fn parse_rfc3339(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value.trim())
        .ok()
        .map(|value| value.with_timezone(&Utc))
}

fn truncate_to_second(value: DateTime<Utc>) -> DateTime<Utc> {
    DateTime::from_timestamp(value.timestamp(), 0).unwrap_or(value)
}

/// The wake time a Gmail label names, if it is one of Satchel's.
pub fn gmail_wake_label_time(name: &str) -> Option<DateTime<Utc>> {
    let time = name.trim().strip_prefix(GMAIL_LABEL_WAKE_PREFIX)?;
    NaiveDateTime::parse_from_str(time, WAKE_FORMAT)
        .ok()
        .map(|value| value.and_utc())
}

/// Read a message's marker from its IMAP keywords (compared without case,
/// RFC 3501). Done wins over a snooze left behind on the same copy.
pub fn marker_from_imap_keywords<'a>(
    keywords: impl IntoIterator<Item = &'a str>,
) -> WorkflowMarker {
    let (mut done, mut snoozed, mut second) = (false, false, false);
    for keyword in keywords {
        let keyword = keyword.trim();
        done |= keyword.eq_ignore_ascii_case(IMAP_KEYWORD_DONE);
        snoozed |= keyword.eq_ignore_ascii_case(IMAP_KEYWORD_SNOOZED);
        second |= keyword.eq_ignore_ascii_case(IMAP_KEYWORD_SNOOZE_SECOND);
    }
    if done {
        WorkflowMarker::Done
    } else if snoozed {
        WorkflowMarker::Snoozed(Snooze {
            wake: None,
            generation: if second { 2 } else { 1 },
        })
    } else {
        WorkflowMarker::None
    }
}

/// Read a message's marker from the names of its Gmail labels.
pub fn marker_from_gmail_labels<'a>(names: impl IntoIterator<Item = &'a str>) -> WorkflowMarker {
    let (mut done, mut snoozed) = (false, false);
    let mut wake = None::<DateTime<Utc>>;
    for name in names {
        let name = name.trim();
        if name == GMAIL_LABEL_DONE {
            done = true;
        } else if name == GMAIL_LABEL_SNOOZED {
            snoozed = true;
        } else if let Some(at) = gmail_wake_label_time(name) {
            wake = Some(wake.map_or(at, |current| current.max(at)));
        }
    }
    if done {
        WorkflowMarker::Done
    } else if snoozed {
        WorkflowMarker::Snoozed(Snooze {
            wake,
            generation: 1,
        })
    } else {
        WorkflowMarker::None
    }
}

/// Whether an IMAP keyword is one Satchel writes.
pub fn is_satchel_imap_keyword(keyword: &str) -> bool {
    IMAP_KEYWORDS
        .iter()
        .any(|ours| ours.eq_ignore_ascii_case(keyword.trim()))
}

/// Whether a Gmail label is one of Satchel's markers. Exact names only:
/// `Satchel/` is an ordinary label namespace, and `Satchel/Invoices` belongs
/// to the person.
pub fn is_satchel_gmail_label(name: &str) -> bool {
    let name = name.trim();
    name == GMAIL_LABEL_DONE || name == GMAIL_LABEL_SNOOZED || gmail_wake_label_time(name).is_some()
}

/// One stored copy of a thread's message, as the thread reading needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkerCopy {
    pub in_inbox: bool,
    pub marker: WorkflowMarker,
}

/// What the server says about a whole thread.
///
/// - Inbox copies decide when there are any: the thread is snoozed only if
///   every Inbox copy carries the snooze. A reply that arrived after the
///   snooze has no keyword and brings the thread back, on every device.
/// - Without Inbox copies (one's own sent mail, Archive), any snoozed copy
///   counts.
/// - Done counts only outside the Inbox. Mail someone moved back to the Inbox
///   in another client keeps its keyword, and the Inbox copy is what the
///   person meant (the same rule as `reconcile_external_workflow_states`).
pub fn thread_marker(copies: &[MarkerCopy]) -> WorkflowMarker {
    let has_inbox = copies.iter().any(|copy| copy.in_inbox);
    let primary = copies
        .iter()
        .filter(|copy| !has_inbox || copy.in_inbox)
        .map(|copy| copy.marker)
        .collect::<Vec<_>>();
    let snoozes = primary
        .iter()
        .filter_map(|marker| match marker {
            WorkflowMarker::Snoozed(snooze) => Some(*snooze),
            _ => None,
        })
        .collect::<Vec<_>>();
    let snoozed = if has_inbox {
        !snoozes.is_empty() && snoozes.len() == primary.len()
    } else {
        !snoozes.is_empty()
    };
    if snoozed {
        return WorkflowMarker::Snoozed(Snooze {
            wake: snoozes.iter().filter_map(|snooze| snooze.wake).max(),
            generation: snoozes
                .iter()
                .map(|snooze| snooze.generation)
                .max()
                .unwrap_or(1),
        });
    }
    if !has_inbox && primary.contains(&WorkflowMarker::Done) {
        return WorkflowMarker::Done;
    }
    WorkflowMarker::None
}

/// The local state a server marker asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PullDecision {
    /// Nothing to change locally; remember the marker as seen.
    Keep,
    /// Set this state (and snooze time), then remember the marker as seen.
    Apply {
        state: WorkflowState,
        snooze_until: Option<DateTime<Utc>>,
    },
}

/// The conflict rule. Edges, not levels: a server marker is applied only
/// when it differs from the one this device last pushed or saw (`last_seen`).
/// The caller has already skipped threads with a local change still queued,
/// so what reaches here is a change made elsewhere, and the newest one wins.
///
/// - Snoozed until a future time: snooze here too, until that time.
/// - Snoozed, time unknown (IMAP, set elsewhere): snooze here for
///   [`ADOPTED_SNOOZE_FALLBACK`] unless already snoozed.
/// - Snoozed until a time that has passed: back to Inbox if snoozed here.
/// - Done: Done (the mail is already outside the Inbox on the server).
/// - No marker after a snooze: someone woke it; back to Inbox if snoozed.
/// - No marker after Done: nothing. Where the mail lies decides that, and
///   folder reconciliation already handles it.
pub fn pull_decision(
    last_seen: &WorkflowMarker,
    server: &WorkflowMarker,
    local_state: WorkflowState,
    local_snooze_until: Option<&str>,
    now: DateTime<Utc>,
) -> PullDecision {
    if same_on_server(last_seen, server) {
        return PullDecision::Keep;
    }
    let snoozed_here = local_state == WorkflowState::Snoozed;
    let to_inbox = PullDecision::Apply {
        state: WorkflowState::Inbox,
        snooze_until: None,
    };
    match server {
        WorkflowMarker::Snoozed(Snooze {
            wake: Some(wake), ..
        }) if *wake > now => {
            // Keywords and labels carry whole seconds; the local time may not.
            let same = snoozed_here
                && local_snooze_until
                    .and_then(parse_rfc3339)
                    .map(truncate_to_second)
                    == Some(*wake);
            if same {
                PullDecision::Keep
            } else {
                PullDecision::Apply {
                    state: WorkflowState::Snoozed,
                    snooze_until: Some(*wake),
                }
            }
        }
        WorkflowMarker::Snoozed(Snooze { wake: Some(_), .. }) if snoozed_here => to_inbox,
        WorkflowMarker::Snoozed(Snooze { wake: Some(_), .. }) => PullDecision::Keep,
        WorkflowMarker::Snoozed(Snooze { wake: None, .. }) if snoozed_here => PullDecision::Keep,
        WorkflowMarker::Snoozed(Snooze { wake: None, .. }) => PullDecision::Apply {
            state: WorkflowState::Snoozed,
            snooze_until: Some(truncate_to_second(now + ADOPTED_SNOOZE_FALLBACK)),
        },
        WorkflowMarker::None if matches!(last_seen, WorkflowMarker::Snoozed(_)) && snoozed_here => {
            to_inbox
        }
        WorkflowMarker::None => PullDecision::Keep,
        WorkflowMarker::Done if local_state == WorkflowState::Done => PullDecision::Keep,
        WorkflowMarker::Done => PullDecision::Apply {
            state: WorkflowState::Done,
            snooze_until: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 7, hour, 0, 0).unwrap()
    }

    fn snoozed(wake: Option<DateTime<Utc>>, generation: u8) -> WorkflowMarker {
        WorkflowMarker::Snoozed(Snooze { wake, generation })
    }

    #[test]
    fn states_map_to_markers_and_back() {
        assert_eq!(
            WorkflowMarker::for_state(WorkflowState::Done, None),
            WorkflowMarker::Done
        );
        assert_eq!(
            WorkflowMarker::for_state(WorkflowState::Snoozed, Some("2026-11-07T09:00:00+01:00")),
            snoozed(Some(at(8)), 1)
        );
        // Sub-second precision cannot travel; drop it up front so the echo of
        // our own push compares equal.
        assert_eq!(
            WorkflowMarker::for_state(WorkflowState::Snoozed, Some("2026-11-07T08:00:00.731Z")),
            snoozed(Some(at(8)), 1)
        );
        for state in [
            WorkflowState::Inbox,
            WorkflowState::Todo,
            WorkflowState::Waiting,
            WorkflowState::Aside,
        ] {
            assert_eq!(WorkflowMarker::for_state(state, None), WorkflowMarker::None);
        }
        assert_eq!(
            WorkflowMarker::for_state(WorkflowState::Snoozed, Some("tomorrow")),
            WorkflowMarker::None
        );
        for marker in [
            WorkflowMarker::None,
            WorkflowMarker::Done,
            snoozed(Some(at(8)), 2),
            snoozed(None, 1),
        ] {
            assert_eq!(
                WorkflowMarker::from_db_value(&marker.as_db_value()),
                Some(marker)
            );
        }
        assert_eq!(WorkflowMarker::None.as_column(), None);
        assert_eq!(WorkflowMarker::from_column(None), WorkflowMarker::None);
        assert_eq!(WorkflowMarker::from_db_value("snoozed:3:"), None);
        assert_eq!(WorkflowMarker::from_db_value("snoozed:1:later"), None);
    }

    #[test]
    fn a_new_snooze_takes_the_other_generation() {
        let new = WorkflowMarker::for_state(WorkflowState::Snoozed, Some("2026-11-07T08:00:00Z"));
        assert_eq!(new.after(&WorkflowMarker::None), snoozed(Some(at(8)), 1));
        assert_eq!(new.after(&snoozed(None, 1)), snoozed(Some(at(8)), 2));
        assert_eq!(new.after(&snoozed(Some(at(6)), 2)), snoozed(Some(at(8)), 1));
        assert_eq!(
            WorkflowMarker::Done.after(&snoozed(None, 1)),
            WorkflowMarker::Done
        );
    }

    #[test]
    fn imap_has_a_fixed_vocabulary_of_three_keywords() {
        assert_eq!(WorkflowMarker::Done.imap_keywords(), vec!["$Satchel-Done"]);
        assert_eq!(
            snoozed(Some(at(8)), 1).imap_keywords(),
            vec!["$Satchel-Snoozed"]
        );
        assert_eq!(
            snoozed(Some(at(9)), 2).imap_keywords(),
            vec!["$Satchel-Snoozed", "$Satchel-Snooze2"]
        );
        // No time ever becomes a keyword: any number of snoozes uses only
        // these three names.
        let mut names = std::collections::BTreeSet::new();
        for hour in 0..24 {
            for generation in [1, 2] {
                names.extend(snoozed(Some(at(hour)), generation).imap_keywords());
            }
        }
        names.extend(WorkflowMarker::Done.imap_keywords());
        assert_eq!(names.len(), IMAP_KEYWORDS.len());

        assert_eq!(
            marker_from_imap_keywords([
                "$label1",
                "$satchel-snoozed",
                "$SATCHEL-SNOOZE2",
                "NonJunk"
            ]),
            snoozed(None, 2)
        );
        assert_eq!(
            marker_from_imap_keywords(["$Satchel-Snoozed"]),
            snoozed(None, 1)
        );
        assert_eq!(
            marker_from_imap_keywords(["$Satchel-Snooze2"]),
            WorkflowMarker::None
        );
        assert_eq!(
            marker_from_imap_keywords(["$Satchel-Snoozed", "$Satchel-Done"]),
            WorkflowMarker::Done
        );
        assert!(is_satchel_imap_keyword("$satchel-done"));
        assert!(!is_satchel_imap_keyword("$Satchel-Invoices"));
        assert!(!is_satchel_imap_keyword("$Junk"));
    }

    #[test]
    fn an_imap_echo_of_our_own_snooze_is_the_same_snooze() {
        assert!(same_on_server(&snoozed(Some(at(8)), 1), &snoozed(None, 1)));
        assert!(!same_on_server(&snoozed(Some(at(8)), 1), &snoozed(None, 2)));
        assert!(!same_on_server(
            &snoozed(Some(at(8)), 1),
            &snoozed(Some(at(9)), 1)
        ));
        assert!(!same_on_server(&WorkflowMarker::None, &snoozed(None, 1)));
    }

    #[test]
    fn gmail_labels_round_trip_and_ignore_the_persons_satchel_labels() {
        let marker = snoozed(Some(at(8)), 1);
        assert_eq!(
            marker.gmail_labels(),
            vec!["Satchel/Snoozed", "Satchel/Wake/20261107T080000Z"]
        );
        assert_eq!(WorkflowMarker::Done.gmail_labels(), vec!["Satchel/Done"]);
        for marker in [WorkflowMarker::None, WorkflowMarker::Done, marker] {
            let labels = marker.gmail_labels();
            assert_eq!(
                marker_from_gmail_labels(labels.iter().map(String::as_str)),
                marker
            );
        }
        assert_eq!(
            marker_from_gmail_labels(["Projects", "Satchel/Invoices"]),
            WorkflowMarker::None
        );
        assert!(is_satchel_gmail_label("Satchel/Wake/20261107T080000Z"));
        assert!(is_satchel_gmail_label("Satchel/Done"));
        assert!(!is_satchel_gmail_label("Satchel/Invoices"));
        assert!(!is_satchel_gmail_label("Satchel/Wake/soon"));
        assert!(!is_satchel_gmail_label("satchel/done"));
        assert!(!is_satchel_gmail_label("Satchel"));
    }

    fn copy(in_inbox: bool, marker: WorkflowMarker) -> MarkerCopy {
        MarkerCopy { in_inbox, marker }
    }

    #[test]
    fn a_thread_is_snoozed_only_while_every_inbox_copy_is() {
        let s = snoozed(Some(at(8)), 1);
        assert_eq!(
            thread_marker(&[copy(true, s), copy(false, WorkflowMarker::None)]),
            s
        );
        // The reply that arrived afterwards carries no keyword.
        assert_eq!(
            thread_marker(&[copy(true, s), copy(true, WorkflowMarker::None)]),
            WorkflowMarker::None
        );
        assert_eq!(thread_marker(&[]), WorkflowMarker::None);
        // A snoozed thread of one's own sent mail has no Inbox copy at all.
        assert_eq!(
            thread_marker(&[copy(false, s), copy(false, WorkflowMarker::None)]),
            s
        );
    }

    #[test]
    fn done_counts_only_outside_the_inbox() {
        assert_eq!(
            thread_marker(&[
                copy(false, WorkflowMarker::Done),
                copy(false, WorkflowMarker::None)
            ]),
            WorkflowMarker::Done
        );
        assert_eq!(
            thread_marker(&[copy(true, WorkflowMarker::Done)]),
            WorkflowMarker::None
        );
    }

    fn now() -> DateTime<Utc> {
        at(7)
    }

    #[test]
    fn an_unchanged_server_marker_never_overrides_local_state() {
        assert_eq!(
            pull_decision(
                &WorkflowMarker::None,
                &WorkflowMarker::None,
                WorkflowState::Todo,
                None,
                now()
            ),
            PullDecision::Keep
        );
        // Our own IMAP snooze, echoed without its time.
        assert_eq!(
            pull_decision(
                &snoozed(Some(at(8)), 1),
                &snoozed(None, 1),
                WorkflowState::Inbox,
                None,
                now()
            ),
            PullDecision::Keep
        );
    }

    #[test]
    fn a_gmail_snooze_from_elsewhere_is_adopted_with_its_wake_time() {
        assert_eq!(
            pull_decision(
                &WorkflowMarker::None,
                &snoozed(Some(at(8)), 1),
                WorkflowState::Inbox,
                None,
                now()
            ),
            PullDecision::Apply {
                state: WorkflowState::Snoozed,
                snooze_until: Some(at(8))
            }
        );
        assert_eq!(
            pull_decision(
                &WorkflowMarker::None,
                &snoozed(Some(at(8)), 1),
                WorkflowState::Snoozed,
                Some("2026-11-07T08:00:00.500Z"),
                now()
            ),
            PullDecision::Keep
        );
    }

    #[test]
    fn an_imap_snooze_from_elsewhere_is_adopted_for_the_fallback_time() {
        assert_eq!(
            pull_decision(
                &WorkflowMarker::None,
                &snoozed(None, 1),
                WorkflowState::Inbox,
                None,
                now()
            ),
            PullDecision::Apply {
                state: WorkflowState::Snoozed,
                snooze_until: Some(now() + ADOPTED_SNOOZE_FALLBACK)
            }
        );
        // A newer snooze elsewhere, while snoozed here: keep our own time.
        assert_eq!(
            pull_decision(
                &snoozed(Some(at(8)), 1),
                &snoozed(None, 2),
                WorkflowState::Snoozed,
                Some("2026-11-07T08:00:00Z"),
                now()
            ),
            PullDecision::Keep
        );
    }

    #[test]
    fn a_snooze_whose_time_has_passed_resolves_to_inbox() {
        assert_eq!(
            pull_decision(
                &WorkflowMarker::None,
                &snoozed(Some(at(6)), 1),
                WorkflowState::Snoozed,
                Some("2026-11-07T09:00:00Z"),
                now()
            ),
            PullDecision::Apply {
                state: WorkflowState::Inbox,
                snooze_until: None
            }
        );
        assert_eq!(
            pull_decision(
                &WorkflowMarker::None,
                &snoozed(Some(at(6)), 1),
                WorkflowState::Inbox,
                None,
                now()
            ),
            PullDecision::Keep
        );
    }

    #[test]
    fn removing_a_marker_elsewhere_wakes_but_does_not_reopen_done() {
        assert_eq!(
            pull_decision(
                &snoozed(Some(at(8)), 1),
                &WorkflowMarker::None,
                WorkflowState::Snoozed,
                Some("2026-11-07T08:00:00Z"),
                now()
            ),
            PullDecision::Apply {
                state: WorkflowState::Inbox,
                snooze_until: None
            }
        );
        assert_eq!(
            pull_decision(
                &WorkflowMarker::Done,
                &WorkflowMarker::None,
                WorkflowState::Done,
                None,
                now()
            ),
            PullDecision::Keep
        );
    }

    #[test]
    fn done_from_elsewhere_is_applied() {
        assert_eq!(
            pull_decision(
                &WorkflowMarker::None,
                &WorkflowMarker::Done,
                WorkflowState::Waiting,
                None,
                now()
            ),
            PullDecision::Apply {
                state: WorkflowState::Done,
                snooze_until: None
            }
        );
    }
}
