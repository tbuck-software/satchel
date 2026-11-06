//! Done and Snooze as the mail server stores them (ADR 0009).
//!
//! IMAP carries them as keywords, Gmail as labels. Both are plain names, so
//! the snooze wake time travels inside a name of its own:
//!
//! | State           | IMAP keywords                                   | Gmail labels                                   |
//! |-----------------|-------------------------------------------------|------------------------------------------------|
//! | Done            | `$Satchel-Done`                                 | `Satchel/Done`                                 |
//! | Snoozed until t | `$Satchel-Snoozed` `$Satchel-Wake-20261107T080000Z` | `Satchel/Snoozed` `Satchel/Wake/20261107T080000Z` |
//!
//! Done also moves the mail to Archive, as before. The keyword is what other
//! clients and devices can see on top of that.
//!
//! Everything here is pure: the mapping both ways, the thread-level reading
//! of a set of copies, and the rule for applying what the server says.

use chrono::{DateTime, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};

use super::states::WorkflowState;

pub const IMAP_KEYWORD_DONE: &str = "$Satchel-Done";
pub const IMAP_KEYWORD_SNOOZED: &str = "$Satchel-Snoozed";
pub const IMAP_KEYWORD_WAKE_PREFIX: &str = "$Satchel-Wake-";
/// Every keyword Satchel owns starts with this; anything else on a message
/// belongs to the person or another client and is never touched.
pub const IMAP_KEYWORD_PREFIX: &str = "$Satchel-";

pub const GMAIL_LABEL_DONE: &str = "Satchel/Done";
pub const GMAIL_LABEL_SNOOZED: &str = "Satchel/Snoozed";
pub const GMAIL_LABEL_WAKE_PREFIX: &str = "Satchel/Wake/";
/// The label namespace Satchel owns. Labels in it are workflow markers, not
/// folders, and never show up as one.
pub const GMAIL_LABEL_PREFIX: &str = "Satchel/";

const WAKE_FORMAT: &str = "%Y%m%dT%H%M%SZ";

/// What the server holds for a thread's workflow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub enum WorkflowMarker {
    /// Neither Done nor Snoozed: Inbox, or a state the server cannot express.
    None,
    Done,
    Snoozed(DateTime<Utc>),
}

impl WorkflowMarker {
    /// The marker a local state is pushed as. A snooze without a readable
    /// wake time has nothing to carry and is pushed as no marker.
    pub fn for_state(state: WorkflowState, snooze_until: Option<&str>) -> Self {
        match state {
            WorkflowState::Done => Self::Done,
            WorkflowState::Snoozed => snooze_until
                .and_then(parse_rfc3339)
                .map(|wake| Self::Snoozed(truncate_to_second(wake)))
                .unwrap_or(Self::None),
            _ => Self::None,
        }
    }

    /// `''`, `done` or `snoozed:<RFC 3339>`, as the `remote_workflow_marker`
    /// columns store it.
    pub fn as_db_value(&self) -> String {
        match self {
            Self::None => String::new(),
            Self::Done => "done".into(),
            Self::Snoozed(wake) => format!("snoozed:{}", wake.to_rfc3339()),
        }
    }

    pub fn from_db_value(value: &str) -> Option<Self> {
        let value = value.trim();
        if value.is_empty() {
            return Some(Self::None);
        }
        if value == "done" {
            return Some(Self::Done);
        }
        value
            .strip_prefix("snoozed:")
            .and_then(parse_rfc3339)
            .map(Self::Snoozed)
    }

    pub fn imap_keywords(&self) -> Vec<String> {
        match self {
            Self::None => Vec::new(),
            Self::Done => vec![IMAP_KEYWORD_DONE.into()],
            Self::Snoozed(wake) => vec![
                IMAP_KEYWORD_SNOOZED.into(),
                format!("{IMAP_KEYWORD_WAKE_PREFIX}{}", wake.format(WAKE_FORMAT)),
            ],
        }
    }

    pub fn gmail_labels(&self) -> Vec<String> {
        match self {
            Self::None => Vec::new(),
            Self::Done => vec![GMAIL_LABEL_DONE.into()],
            Self::Snoozed(wake) => vec![
                GMAIL_LABEL_SNOOZED.into(),
                format!("{GMAIL_LABEL_WAKE_PREFIX}{}", wake.format(WAKE_FORMAT)),
            ],
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

fn parse_rfc3339(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value.trim())
        .ok()
        .map(|value| value.with_timezone(&Utc))
}

fn truncate_to_second(value: DateTime<Utc>) -> DateTime<Utc> {
    DateTime::from_timestamp(value.timestamp(), 0).unwrap_or(value)
}

fn parse_wake(value: &str) -> Option<DateTime<Utc>> {
    NaiveDateTime::parse_from_str(value, WAKE_FORMAT)
        .ok()
        .map(|value| value.and_utc())
}

/// Read a message's marker from its IMAP keywords. Keywords compare without
/// case (RFC 3501). Done wins over a snooze left behind on the same copy; a
/// snooze needs both its keywords, and the latest wake time if several.
pub fn marker_from_imap_keywords<'a>(
    keywords: impl IntoIterator<Item = &'a str>,
) -> WorkflowMarker {
    marker_from_names(
        keywords,
        IMAP_KEYWORD_DONE,
        IMAP_KEYWORD_SNOOZED,
        IMAP_KEYWORD_WAKE_PREFIX,
    )
}

/// Read a message's marker from the names of its Gmail labels.
pub fn marker_from_gmail_labels<'a>(names: impl IntoIterator<Item = &'a str>) -> WorkflowMarker {
    marker_from_names(
        names,
        GMAIL_LABEL_DONE,
        GMAIL_LABEL_SNOOZED,
        GMAIL_LABEL_WAKE_PREFIX,
    )
}

fn marker_from_names<'a>(
    names: impl IntoIterator<Item = &'a str>,
    done: &str,
    snoozed: &str,
    wake_prefix: &str,
) -> WorkflowMarker {
    let mut is_done = false;
    let mut is_snoozed = false;
    let mut wake = None::<DateTime<Utc>>;
    for name in names {
        let name = name.trim();
        if name.eq_ignore_ascii_case(done) {
            is_done = true;
        } else if name.eq_ignore_ascii_case(snoozed) {
            is_snoozed = true;
        } else if name.len() > wake_prefix.len()
            && name.is_char_boundary(wake_prefix.len())
            && name[..wake_prefix.len()].eq_ignore_ascii_case(wake_prefix)
        {
            if let Some(at) = parse_wake(&name[wake_prefix.len()..].to_ascii_uppercase()) {
                wake = Some(wake.map_or(at, |current| current.max(at)));
            }
        }
    }
    match (is_done, is_snoozed, wake) {
        (true, _, _) => WorkflowMarker::Done,
        (false, true, Some(wake)) => WorkflowMarker::Snoozed(wake),
        _ => WorkflowMarker::None,
    }
}

/// Whether an IMAP keyword is one of Satchel's.
pub fn is_satchel_imap_keyword(keyword: &str) -> bool {
    keyword.len() >= IMAP_KEYWORD_PREFIX.len()
        && keyword.is_char_boundary(IMAP_KEYWORD_PREFIX.len())
        && keyword[..IMAP_KEYWORD_PREFIX.len()].eq_ignore_ascii_case(IMAP_KEYWORD_PREFIX)
}

/// Whether a Gmail label name is one of Satchel's workflow markers.
pub fn is_satchel_gmail_label(name: &str) -> bool {
    let name = name.trim();
    name.len() > GMAIL_LABEL_PREFIX.len()
        && name.is_char_boundary(GMAIL_LABEL_PREFIX.len())
        && name[..GMAIL_LABEL_PREFIX.len()].eq_ignore_ascii_case(GMAIL_LABEL_PREFIX)
}

/// One stored copy of a thread's message, as the thread reading needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkerCopy {
    pub in_inbox: bool,
    /// `None` when the server has not reported this copy yet.
    pub marker: Option<WorkflowMarker>,
}

/// What the server says about a whole thread, or `None` when it has not
/// reported enough to say anything.
///
/// - Inbox copies decide when there are any: the thread is snoozed only if
///   every reported Inbox copy carries the snooze. A reply that arrived
///   after the snooze has no keyword and brings the thread back, on every
///   device, without anyone cleaning up the older copies.
/// - Done counts only outside the Inbox. Mail someone moved back to the Inbox
///   in another client keeps its keyword, and the Inbox copy is what the
///   person meant (the same rule as `reconcile_external_workflow_states`).
pub fn thread_marker(copies: &[MarkerCopy]) -> Option<WorkflowMarker> {
    let has_inbox = copies.iter().any(|copy| copy.in_inbox);
    let primary = copies
        .iter()
        .filter(|copy| !has_inbox || copy.in_inbox)
        .filter_map(|copy| copy.marker.as_ref())
        .collect::<Vec<_>>();
    if primary.is_empty() {
        return None;
    }
    let mut wake = None::<DateTime<Utc>>;
    let all_snoozed = primary.iter().all(|marker| match marker {
        WorkflowMarker::Snoozed(at) => {
            wake = Some(wake.map_or(*at, |current| current.max(*at)));
            true
        }
        _ => false,
    });
    if all_snoozed {
        if let Some(wake) = wake {
            return Some(WorkflowMarker::Snoozed(wake));
        }
    }
    if !has_inbox && primary.contains(&&WorkflowMarker::Done) {
        return Some(WorkflowMarker::Done);
    }
    Some(WorkflowMarker::None)
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
/// - Snoozed until a time that has passed: back to Inbox if it is snoozed
///   here, otherwise nothing.
/// - Done: Done (the mail is already outside the Inbox on the server).
/// - No marker after a snooze: someone woke it; back to Inbox if snoozed here.
/// - No marker after Done: nothing. Where the mail lies decides that, and
///   folder reconciliation already handles it.
pub fn pull_decision(
    last_seen: Option<&WorkflowMarker>,
    server: &WorkflowMarker,
    local_state: WorkflowState,
    local_snooze_until: Option<&str>,
    now: DateTime<Utc>,
) -> PullDecision {
    let last_seen = last_seen.unwrap_or(&WorkflowMarker::None);
    if last_seen == server {
        return PullDecision::Keep;
    }
    match server {
        WorkflowMarker::Snoozed(wake) if *wake > now => {
            // Keywords carry whole seconds; the local time may not.
            let same = local_state == WorkflowState::Snoozed
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
        WorkflowMarker::Snoozed(_) | WorkflowMarker::None => {
            let woke = matches!(server, WorkflowMarker::Snoozed(_))
                || matches!(last_seen, WorkflowMarker::Snoozed(_));
            if woke && local_state == WorkflowState::Snoozed {
                PullDecision::Apply {
                    state: WorkflowState::Inbox,
                    snooze_until: None,
                }
            } else {
                PullDecision::Keep
            }
        }
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

    #[test]
    fn states_map_to_markers_and_back() {
        assert_eq!(
            WorkflowMarker::for_state(WorkflowState::Done, None),
            WorkflowMarker::Done
        );
        assert_eq!(
            WorkflowMarker::for_state(WorkflowState::Snoozed, Some("2026-11-07T09:00:00+01:00")),
            WorkflowMarker::Snoozed(at(8))
        );
        // Sub-second precision cannot travel in a keyword; drop it up front so
        // the echo of our own push compares equal.
        assert_eq!(
            WorkflowMarker::for_state(WorkflowState::Snoozed, Some("2026-11-07T08:00:00.731Z")),
            WorkflowMarker::Snoozed(at(8))
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
            WorkflowMarker::Snoozed(at(8)),
        ] {
            assert_eq!(
                WorkflowMarker::from_db_value(&marker.as_db_value()),
                Some(marker)
            );
        }
        assert_eq!(WorkflowMarker::from_db_value("snoozed:later"), None);
    }

    #[test]
    fn imap_keywords_round_trip() {
        let snoozed = WorkflowMarker::Snoozed(at(8));
        assert_eq!(
            snoozed.imap_keywords(),
            vec!["$Satchel-Snoozed", "$Satchel-Wake-20261107T080000Z"]
        );
        assert_eq!(WorkflowMarker::Done.imap_keywords(), vec!["$Satchel-Done"]);
        assert!(WorkflowMarker::None.imap_keywords().is_empty());
        for marker in [WorkflowMarker::None, WorkflowMarker::Done, snoozed] {
            let keywords = marker.imap_keywords();
            assert_eq!(
                marker_from_imap_keywords(keywords.iter().map(String::as_str)),
                marker
            );
        }
    }

    #[test]
    fn imap_keywords_are_read_without_case_and_ignore_foreign_ones() {
        assert_eq!(
            marker_from_imap_keywords([
                "$label1",
                "$satchel-snoozed",
                "$SATCHEL-WAKE-20261107t080000z",
                "NonJunk"
            ]),
            WorkflowMarker::Snoozed(at(8))
        );
        // A snooze without its time, or a time without the snooze, says nothing.
        assert_eq!(
            marker_from_imap_keywords(["$Satchel-Snoozed"]),
            WorkflowMarker::None
        );
        assert_eq!(
            marker_from_imap_keywords(["$Satchel-Wake-20261107T080000Z"]),
            WorkflowMarker::None
        );
        assert_eq!(
            marker_from_imap_keywords(["$Satchel-Snoozed", "$Satchel-Wake-soon"]),
            WorkflowMarker::None
        );
        // Two wake keywords (two devices raced): the later one.
        assert_eq!(
            marker_from_imap_keywords([
                "$Satchel-Snoozed",
                "$Satchel-Wake-20261107T080000Z",
                "$Satchel-Wake-20261107T100000Z"
            ]),
            WorkflowMarker::Snoozed(at(10))
        );
        assert_eq!(
            marker_from_imap_keywords([
                "$Satchel-Snoozed",
                "$Satchel-Wake-20261107T080000Z",
                "$Satchel-Done"
            ]),
            WorkflowMarker::Done
        );
        assert!(is_satchel_imap_keyword("$satchel-done"));
        assert!(!is_satchel_imap_keyword("$Satch"));
        assert!(!is_satchel_imap_keyword("$Junk"));
    }

    #[test]
    fn gmail_labels_round_trip() {
        let snoozed = WorkflowMarker::Snoozed(at(8));
        assert_eq!(
            snoozed.gmail_labels(),
            vec!["Satchel/Snoozed", "Satchel/Wake/20261107T080000Z"]
        );
        assert_eq!(WorkflowMarker::Done.gmail_labels(), vec!["Satchel/Done"]);
        for marker in [WorkflowMarker::None, WorkflowMarker::Done, snoozed] {
            let labels = marker.gmail_labels();
            assert_eq!(
                marker_from_gmail_labels(labels.iter().map(String::as_str)),
                marker
            );
        }
        assert_eq!(
            marker_from_gmail_labels(["Projects", "satchel/done"]),
            WorkflowMarker::Done
        );
        assert!(is_satchel_gmail_label("Satchel/Wake/20261107T080000Z"));
        assert!(!is_satchel_gmail_label("Satchel"));
        assert!(!is_satchel_gmail_label("Satchelbags/Done"));
    }

    fn copy(in_inbox: bool, marker: Option<WorkflowMarker>) -> MarkerCopy {
        MarkerCopy { in_inbox, marker }
    }

    #[test]
    fn a_thread_is_snoozed_only_while_every_inbox_copy_is() {
        let snoozed = Some(WorkflowMarker::Snoozed(at(8)));
        assert_eq!(
            thread_marker(&[
                copy(true, snoozed.clone()),
                copy(false, Some(WorkflowMarker::None))
            ]),
            Some(WorkflowMarker::Snoozed(at(8)))
        );
        // The reply that arrived afterwards carries no keyword.
        assert_eq!(
            thread_marker(&[
                copy(true, snoozed.clone()),
                copy(true, Some(WorkflowMarker::None))
            ]),
            Some(WorkflowMarker::None)
        );
        // A copy not reported yet neither confirms nor contradicts.
        assert_eq!(
            thread_marker(&[copy(true, snoozed.clone()), copy(true, None)]),
            Some(WorkflowMarker::Snoozed(at(8)))
        );
        assert_eq!(thread_marker(&[copy(true, None)]), None);
        assert_eq!(thread_marker(&[]), None);
        // A snoozed thread of one's own sent mail has no Inbox copy at all.
        assert_eq!(
            thread_marker(&[copy(false, snoozed)]),
            Some(WorkflowMarker::Snoozed(at(8)))
        );
    }

    #[test]
    fn done_counts_only_outside_the_inbox() {
        assert_eq!(
            thread_marker(&[
                copy(false, Some(WorkflowMarker::Done)),
                copy(false, Some(WorkflowMarker::None))
            ]),
            Some(WorkflowMarker::Done)
        );
        assert_eq!(
            thread_marker(&[copy(true, Some(WorkflowMarker::Done))]),
            Some(WorkflowMarker::None)
        );
        assert_eq!(
            thread_marker(&[copy(true, None), copy(false, Some(WorkflowMarker::Done))]),
            None
        );
    }

    fn now() -> DateTime<Utc> {
        at(7)
    }

    #[test]
    fn an_unchanged_server_marker_never_overrides_local_state() {
        // Todo has no server form; the server's silence must not reset it.
        assert_eq!(
            pull_decision(
                None,
                &WorkflowMarker::None,
                WorkflowState::Todo,
                None,
                now()
            ),
            PullDecision::Keep
        );
        assert_eq!(
            pull_decision(
                Some(&WorkflowMarker::Done),
                &WorkflowMarker::Done,
                WorkflowState::Inbox,
                None,
                now()
            ),
            PullDecision::Keep
        );
    }

    #[test]
    fn a_snooze_from_elsewhere_is_adopted_with_its_wake_time() {
        assert_eq!(
            pull_decision(
                None,
                &WorkflowMarker::Snoozed(at(8)),
                WorkflowState::Inbox,
                None,
                now()
            ),
            PullDecision::Apply {
                state: WorkflowState::Snoozed,
                snooze_until: Some(at(8))
            }
        );
        // Re-snoozed elsewhere to a later time.
        assert_eq!(
            pull_decision(
                Some(&WorkflowMarker::Snoozed(at(8))),
                &WorkflowMarker::Snoozed(at(10)),
                WorkflowState::Snoozed,
                Some("2026-11-07T08:00:00+00:00"),
                now()
            ),
            PullDecision::Apply {
                state: WorkflowState::Snoozed,
                snooze_until: Some(at(10))
            }
        );
        assert_eq!(
            pull_decision(
                None,
                &WorkflowMarker::Snoozed(at(8)),
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
                None,
                &WorkflowMarker::Snoozed(at(6)),
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
                None,
                &WorkflowMarker::Snoozed(at(6)),
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
                Some(&WorkflowMarker::Snoozed(at(8))),
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
                Some(&WorkflowMarker::Done),
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
                None,
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
