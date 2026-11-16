//! Repeating tasks: which rules Satchel can advance itself, and the next
//! occurrence of those.
//!
//! Completing a repeating VTODO by writing `STATUS:COMPLETED` onto it ends the
//! series for every client. For the rules people make in Apple Reminders,
//! Tasks.org or Thunderbird — every n days, weeks (on one weekday), months or
//! years, optionally limited by COUNT or UNTIL — completing instead moves the
//! due date (and DTSTART) to the next occurrence and keeps the task open, as
//! those apps do. Anything richer (BYMONTHDAY, BYSETPOS, several weekdays,
//! EXDATE, overridden instances, no due date to step from) is "complex": the
//! UI asks before it completes the whole series.
use chrono::{Datelike, Duration, Months, NaiveDate, NaiveDateTime, TimeZone, Utc, Weekday};

use crate::email::calendar::{local_to_utc, normalize_tzid};

/// `tasks.due_tz` for a DUE written in UTC (`…Z`): it repeats at that UTC time.
pub const ZONE_UTC: &str = "UTC";
/// `tasks.due_tz` for a floating DUE (no `Z`, no TZID): local to whoever reads it.
pub const ZONE_FLOATING: &str = "floating";

/// The clock a repeating date-time keeps its wall time on.
///
/// `DUE;TZID=America/New_York:…T090000` repeats at 09:00 New York time
/// wherever Satchel runs; a floating time at 09:00 where the user is; a UTC
/// stamp at the same UTC time; a due time Satchel set itself at the user's
/// local time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zone {
    Host,
    Utc,
    Named(chrono_tz::Tz),
}

impl Zone {
    /// From `tasks.due_tz`: None (Satchel's own value) and floating step on
    /// the host clock; an unknown TZID falls back to it too.
    pub fn from_stored(due_tz: Option<&str>) -> Self {
        match due_tz {
            None => Self::Host,
            Some(ZONE_FLOATING) => Self::Host,
            Some(ZONE_UTC) => Self::Utc,
            Some(tzid) => normalize_tzid(tzid)
                .parse::<chrono_tz::Tz>()
                .map_or(Self::Host, Self::Named),
        }
    }

    fn local(self, instant: chrono::DateTime<Utc>) -> NaiveDateTime {
        match self {
            Self::Host => instant.with_timezone(&chrono::Local).naive_local(),
            Self::Utc => instant.naive_utc(),
            Self::Named(tz) => instant.with_timezone(&tz).naive_local(),
        }
    }

    fn utc(self, local: NaiveDateTime) -> Option<chrono::DateTime<Utc>> {
        match self {
            Self::Host => local_to_utc(local),
            Self::Utc => Some(Utc.from_utc_datetime(&local)),
            Self::Named(tz) => tz
                .from_local_datetime(&local)
                .earliest()
                // A wall time in the spring-forward gap: the hour after it.
                .or_else(|| {
                    tz.from_local_datetime(&(local + Duration::hours(1)))
                        .earliest()
                })
                .map(|instant| instant.with_timezone(&Utc)),
        }
    }
}

/// The stored classification; `None` means the task does not repeat.
pub const SIMPLE: &str = "simple";
pub const COMPLEX: &str = "complex";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Frequency {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimpleRule {
    frequency: Frequency,
    interval: u32,
    count: Option<u32>,
    until: Option<String>,
    weekday: Option<Weekday>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recurrence {
    None,
    Simple(SimpleRule),
    Complex,
}

impl Recurrence {
    /// The value of `tasks.recurrence`.
    pub fn stored(&self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::Simple(_) => Some(SIMPLE),
            Self::Complex => Some(COMPLEX),
        }
    }
}

fn weekday(value: &str) -> Option<Weekday> {
    Some(match value {
        "MO" => Weekday::Mon,
        "TU" => Weekday::Tue,
        "WE" => Weekday::Wed,
        "TH" => Weekday::Thu,
        "FR" => Weekday::Fri,
        "SA" => Weekday::Sat,
        "SU" => Weekday::Sun,
        _ => return None,
    })
}

/// Classify an RRULE. `extras` is set when the resource also carries EXDATE,
/// RDATE or overridden instances; `due` is the value a step starts from.
pub fn classify(rrule: Option<&str>, extras: bool, due: Option<&str>) -> Recurrence {
    let Some(rrule) = rrule.map(str::trim).filter(|value| !value.is_empty()) else {
        return Recurrence::None;
    };
    if extras || due.is_none() {
        return Recurrence::Complex;
    }
    let mut frequency = None;
    let mut interval = 1;
    let mut count = None;
    let mut until = None;
    let mut by_day = None;
    let mut week_start: Option<String> = None;
    for part in rrule.split(';').filter(|part| !part.is_empty()) {
        let Some((key, value)) = part.split_once('=') else {
            return Recurrence::Complex;
        };
        let value = value.trim().to_ascii_uppercase();
        match key.trim().to_ascii_uppercase().as_str() {
            "FREQ" => {
                frequency = Some(match value.as_str() {
                    "DAILY" => Frequency::Daily,
                    "WEEKLY" => Frequency::Weekly,
                    "MONTHLY" => Frequency::Monthly,
                    "YEARLY" => Frequency::Yearly,
                    _ => return Recurrence::Complex,
                });
            }
            "INTERVAL" => match value.parse::<u32>() {
                Ok(value) if value >= 1 => interval = value,
                _ => return Recurrence::Complex,
            },
            "COUNT" => match value.parse::<u32>() {
                Ok(value) if value >= 1 => count = Some(value),
                _ => return Recurrence::Complex,
            },
            "UNTIL" => {
                if until_bound(&value).is_none() {
                    return Recurrence::Complex;
                }
                until = Some(value);
            }
            "BYDAY" => match weekday(&value) {
                Some(day) => by_day = Some(day),
                None => return Recurrence::Complex,
            },
            // The week start only matters for rules Satchel does not step.
            // The week start shapes only rules with weeks between occurrences
            // and a weekday; Satchel counts such weeks from Monday.
            "WKST" => week_start = Some(value),
            _ => return Recurrence::Complex,
        }
    }
    let Some(frequency) = frequency else {
        return Recurrence::Complex;
    };
    if by_day.is_some() && frequency != Frequency::Weekly {
        return Recurrence::Complex;
    }
    if by_day.is_some() && interval > 1 && week_start.as_deref().is_some_and(|day| day != "MO") {
        return Recurrence::Complex;
    }
    Recurrence::Simple(SimpleRule {
        frequency,
        interval,
        count,
        until,
        weekday: by_day,
    })
}

/// UNTIL as either a date or a UTC instant.
enum Bound {
    Date(NaiveDate),
    Instant(chrono::DateTime<Utc>),
}

fn until_bound(value: &str) -> Option<Bound> {
    if value.len() == 8 {
        return NaiveDate::parse_from_str(value, "%Y%m%d")
            .ok()
            .map(Bound::Date);
    }
    let naive = NaiveDateTime::parse_from_str(value.trim_end_matches('Z'), "%Y%m%dT%H%M%S").ok()?;
    Some(if value.ends_with('Z') {
        Bound::Instant(Utc.from_utc_datetime(&naive))
    } else {
        Bound::Instant(local_to_utc(naive)?)
    })
}

/// A due or start value: a date, or a date-time as the wall clock of the
/// series' zone shows it.
#[derive(Debug, Clone, Copy)]
enum When {
    Date(NaiveDate),
    Local(NaiveDateTime),
}

fn parse_when(value: &str, zone: Zone) -> Option<When> {
    if value.len() == 10 {
        return NaiveDate::parse_from_str(value, "%Y-%m-%d")
            .ok()
            .map(When::Date);
    }
    let instant = chrono::DateTime::parse_from_rfc3339(value).ok()?;
    Some(When::Local(zone.local(instant.with_timezone(&Utc))))
}

fn format_when(value: When, zone: Zone) -> Option<String> {
    Some(match value {
        When::Date(date) => date.format("%Y-%m-%d").to_string(),
        When::Local(local) => zone.utc(local)?.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
    })
}

impl When {
    fn date(self) -> NaiveDate {
        match self {
            Self::Date(date) => date,
            Self::Local(local) => local.date(),
        }
    }

    /// The same wall-clock time `days` later, so 09:00 stays 09:00 across a
    /// daylight-saving change.
    fn shifted(self, days: i64) -> Self {
        match self {
            Self::Date(date) => Self::Date(date + Duration::days(days)),
            Self::Local(local) => Self::Local(local + Duration::days(days)),
        }
    }
}

fn add_months(date: NaiveDate, months: u32) -> Option<NaiveDate> {
    // RFC 5545: a month without the day (the 31st, 29 February) is skipped,
    // not clamped.
    let day = date.day();
    let mut step = months;
    for _ in 0..48 {
        let candidate = date.with_day(1)?.checked_add_months(Months::new(step))?;
        if let Some(found) = candidate.with_day(day) {
            return Some(found);
        }
        step += months;
    }
    None
}

impl SimpleRule {
    fn next_date(&self, from: NaiveDate) -> Option<NaiveDate> {
        let interval = self.interval;
        match self.frequency {
            Frequency::Daily => Some(from + Duration::days(i64::from(interval))),
            Frequency::Weekly => match self.weekday {
                Some(day) if day != from.weekday() => {
                    let ahead = (7 + day.num_days_from_monday() as i64
                        - from.weekday().num_days_from_monday() as i64)
                        % 7;
                    let next = from + Duration::days(ahead);
                    // Landing in a later week starts the next interval there.
                    let later_week = next.weekday().num_days_from_monday()
                        < from.weekday().num_days_from_monday();
                    Some(if later_week {
                        next + Duration::weeks(i64::from(interval) - 1)
                    } else {
                        next
                    })
                }
                _ => Some(from + Duration::weeks(i64::from(interval))),
            },
            Frequency::Monthly => add_months(from, interval),
            Frequency::Yearly => add_months(from, interval.checked_mul(12)?),
        }
    }

    fn ends_before(&self, next: When, zone: Zone) -> bool {
        match self.until.as_deref().and_then(until_bound) {
            None => false,
            Some(Bound::Date(until)) => next.date() > until,
            Some(Bound::Instant(until)) => match next {
                When::Date(date) => date > zone.local(until).date(),
                When::Local(local) => zone.utc(local).is_some_and(|instant| instant > until),
            },
        }
    }
}

/// The task after one completion of a simple rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Advance {
    pub due_at: String,
    pub start_at: Option<String>,
    pub reminder_at: Option<String>,
    /// The rule with COUNT lowered by the occurrence just completed.
    pub rrule: String,
}

/// The next occurrence, or `None` when the one completed was the last
/// (COUNT reached, or the next one falls after UNTIL): then the series ends.
///
/// Date-times step on `zone`'s wall clock (see [`Zone`]); the start and the
/// reminder move by the same number of days on the same clock, so a relative
/// alarm still lands where its offset puts it.
pub fn advance(
    rule: &SimpleRule,
    rrule: &str,
    due: &str,
    start: Option<&str>,
    reminder: Option<&str>,
    zone: Zone,
) -> Option<Advance> {
    if rule.count.is_some_and(|count| count <= 1) {
        return None;
    }
    let current = parse_when(due, zone)?;
    let next_date = rule.next_date(current.date())?;
    let days = (next_date - current.date()).num_days();
    let next = current.shifted(days);
    if rule.ends_before(next, zone) {
        return None;
    }
    let shift = |value: Option<&str>| -> Option<String> {
        value
            .and_then(|value| parse_when(value, zone))
            .and_then(|when| format_when(when.shifted(days), zone))
    };
    let rrule = match rule.count {
        Some(count) => rrule
            .split(';')
            .map(|part| {
                if part
                    .split_once('=')
                    .is_some_and(|(key, _)| key.trim().eq_ignore_ascii_case("COUNT"))
                {
                    format!("COUNT={}", count - 1)
                } else {
                    part.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join(";"),
        None => rrule.to_string(),
    };
    Some(Advance {
        due_at: format_when(next, zone)?,
        start_at: shift(start),
        reminder_at: shift(reminder),
        rrule,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn simple(rrule: &str) -> SimpleRule {
        match classify(Some(rrule), false, Some("2026-11-06")) {
            Recurrence::Simple(rule) => rule,
            other => panic!("{rrule} is {other:?}"),
        }
    }

    fn next_due(rrule: &str, due: &str) -> Option<String> {
        advance(&simple(rrule), rrule, due, None, None, Zone::Host).map(|next| next.due_at)
    }

    #[test]
    fn tells_simple_rules_from_complex_ones() {
        assert_eq!(classify(None, false, Some("2026-11-06")), Recurrence::None);
        for rule in [
            "FREQ=DAILY",
            "FREQ=WEEKLY;INTERVAL=2;BYDAY=FR",
            "FREQ=MONTHLY;COUNT=5",
            "FREQ=YEARLY;UNTIL=20301231",
            "FREQ=WEEKLY;WKST=MO;UNTIL=20270101T000000Z",
        ] {
            assert!(
                matches!(
                    classify(Some(rule), false, Some("2026-11-06")),
                    Recurrence::Simple(_)
                ),
                "{rule}"
            );
        }
        for rule in [
            "FREQ=WEEKLY;BYDAY=MO,WE",
            "FREQ=MONTHLY;BYMONTHDAY=-1",
            "FREQ=MONTHLY;BYDAY=1MO",
            "FREQ=MONTHLY;BYDAY=MO",
            "FREQ=HOURLY",
            "FREQ=YEARLY;BYSETPOS=1",
            "INTERVAL=2",
        ] {
            assert_eq!(
                classify(Some(rule), false, Some("2026-11-06")),
                Recurrence::Complex,
                "{rule}"
            );
        }
        assert_eq!(
            classify(Some("FREQ=DAILY"), true, Some("2026-11-06")),
            Recurrence::Complex
        );
        assert_eq!(
            classify(Some("FREQ=DAILY"), false, None),
            Recurrence::Complex
        );
    }

    #[test]
    fn steps_each_frequency() {
        assert_eq!(
            next_due("FREQ=DAILY;INTERVAL=3", "2026-11-06").as_deref(),
            Some("2026-11-09")
        );
        assert_eq!(
            next_due("FREQ=WEEKLY", "2026-11-06").as_deref(),
            Some("2026-11-13")
        );
        assert_eq!(
            next_due("FREQ=WEEKLY;INTERVAL=2", "2026-11-06").as_deref(),
            Some("2026-11-20")
        );
        assert_eq!(
            next_due("FREQ=MONTHLY", "2026-11-06").as_deref(),
            Some("2026-12-06")
        );
        assert_eq!(
            next_due("FREQ=YEARLY", "2028-02-29").as_deref(),
            Some("2032-02-29")
        );
        // The 31st skips months that have no 31st.
        assert_eq!(
            next_due("FREQ=MONTHLY", "2026-01-31").as_deref(),
            Some("2026-03-31")
        );
        // Due on a Thursday, repeating on Fridays: the coming Friday.
        assert_eq!(
            next_due("FREQ=WEEKLY;BYDAY=FR", "2026-11-05").as_deref(),
            Some("2026-11-06")
        );
        // Due on a Saturday, every other week on Friday: Friday of the week after next.
        assert_eq!(
            next_due("FREQ=WEEKLY;INTERVAL=2;BYDAY=FR", "2026-11-07").as_deref(),
            Some("2026-11-20")
        );
    }

    #[test]
    fn counts_down_and_stops_at_until() {
        let rule = "FREQ=DAILY;COUNT=3;WKST=MO";
        let next = advance(&simple(rule), rule, "2026-11-06", None, None, Zone::Host).unwrap();
        assert_eq!(next.rrule, "FREQ=DAILY;COUNT=2;WKST=MO");
        assert_eq!(next_due("FREQ=DAILY;COUNT=1", "2026-11-06"), None);
        assert_eq!(next_due("FREQ=WEEKLY;UNTIL=20261110", "2026-11-06"), None);
        assert_eq!(
            next_due("FREQ=WEEKLY;UNTIL=20261113", "2026-11-06").as_deref(),
            Some("2026-11-13")
        );
    }

    #[test]
    fn keeps_the_wall_clock_time_and_moves_start_and_reminder_along() {
        // 09:00 local before and after the end of daylight saving.
        let due = local_to_utc(
            NaiveDate::from_ymd_opt(2026, 10, 20)
                .unwrap()
                .and_hms_opt(9, 0, 0)
                .unwrap(),
        )
        .unwrap()
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string();
        let reminder = local_to_utc(
            NaiveDate::from_ymd_opt(2026, 10, 20)
                .unwrap()
                .and_hms_opt(8, 30, 0)
                .unwrap(),
        )
        .unwrap()
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string();
        let next = advance(
            &simple("FREQ=WEEKLY"),
            "FREQ=WEEKLY",
            &due,
            Some("2026-10-19"),
            Some(&reminder),
            Zone::Host,
        )
        .unwrap();
        let local = |value: &str| {
            chrono::DateTime::parse_from_rfc3339(value)
                .unwrap()
                .with_timezone(&chrono::Local)
                .naive_local()
        };
        assert_eq!(
            local(&next.due_at),
            NaiveDate::from_ymd_opt(2026, 10, 27)
                .unwrap()
                .and_hms_opt(9, 0, 0)
                .unwrap()
        );
        assert_eq!(
            local(next.reminder_at.as_deref().unwrap()).time(),
            chrono::NaiveTime::from_hms_opt(8, 30, 0).unwrap()
        );
        assert_eq!(next.start_at.as_deref(), Some("2026-10-26"));
    }

    /// The same answer whatever zone the host is in (run under `TZ=UTC` and
    /// `TZ=Europe/Berlin`): New York leaves daylight saving on 1 November,
    /// Berlin a week earlier, and the task stays at 09:00 New York time.
    #[test]
    fn steps_on_the_wall_clock_of_the_due_dates_zone() {
        let zone = Zone::from_stored(Some("America/New_York"));
        assert!(matches!(zone, Zone::Named(_)));
        // 28 Oct 2026 09:00 EDT = 13:00Z; 4 Nov 09:00 EST = 14:00Z.
        let next = advance(
            &simple("FREQ=WEEKLY"),
            "FREQ=WEEKLY",
            "2026-10-28T13:00:00Z",
            Some("2026-10-28T12:00:00Z"),
            Some("2026-10-28T12:30:00Z"),
            zone,
        )
        .unwrap();
        assert_eq!(next.due_at, "2026-11-04T14:00:00Z");
        assert_eq!(next.start_at.as_deref(), Some("2026-11-04T13:00:00Z"));
        assert_eq!(next.reminder_at.as_deref(), Some("2026-11-04T13:30:00Z"));
    }

    #[test]
    fn a_utc_stamp_repeats_at_the_same_utc_time() {
        let next = advance(
            &simple("FREQ=WEEKLY"),
            "FREQ=WEEKLY",
            "2026-10-28T09:00:00Z",
            None,
            None,
            Zone::from_stored(Some(ZONE_UTC)),
        )
        .unwrap();
        assert_eq!(next.due_at, "2026-11-04T09:00:00Z");
    }

    #[test]
    fn a_week_start_other_than_monday_with_weeks_between_is_not_stepped() {
        assert!(matches!(
            classify(
                Some("FREQ=WEEKLY;INTERVAL=2;BYDAY=TU;WKST=SU"),
                false,
                Some("2026-11-06")
            ),
            Recurrence::Complex
        ));
        assert!(matches!(
            classify(
                Some("FREQ=WEEKLY;BYDAY=TU;WKST=SU"),
                false,
                Some("2026-11-06")
            ),
            Recurrence::Simple(_)
        ));
    }
}
