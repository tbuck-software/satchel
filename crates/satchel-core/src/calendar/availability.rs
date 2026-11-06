//! Free time: the account owner's busy intervals from the local calendar
//! mirror, and the free windows inside working hours that "insert my
//! availability" and the event editor's "next time everyone is free" are
//! built from.
//!
//! Working hours are wall-clock times in one IANA zone. Every day is placed
//! in that zone on its own, so a window keeps its 09:00–17:00 across a DST
//! change; a start that falls into a spring-forward gap moves to the first
//! minute that exists, and an end inside a repeated hour takes the later one.

use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use sqlx::{FromRow, SqlitePool};

/// Slot boundaries snap to this grid, so a meeting ending 10:50 frees 11:00.
const GRID_MINUTES: i64 = 15;
/// "Next working days" never scans further than this, whatever the hours say.
const MAX_SCAN_DAYS: i64 = 62;
/// Upper bound on the occurrences one recurring event contributes to a range.
const MAX_OCCURRENCES: usize = 500;

/// One busy interval. Tentative time is shown hatched in the editor; for free
/// slot computation it counts as busy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Busy {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub tentative: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AvailabilityError {
    #[error("working hours are invalid")]
    WorkingHours,
    #[error("duration is invalid")]
    Duration,
    #[error("range is invalid")]
    Range,
    #[error("time zone is unknown")]
    TimeZone,
}

impl AvailabilityError {
    pub fn key(&self) -> &'static str {
        match self {
            Self::WorkingHours => "backend.errors.availabilityWorkingHoursInvalid",
            Self::Duration => "backend.errors.availabilityDurationInvalid",
            Self::Range => "backend.errors.availabilityRangeInvalid",
            Self::TimeZone => "backend.errors.availabilityTimeZoneInvalid",
        }
    }
}

/// Wall-clock working hours on a set of weekdays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkingHours {
    /// Minutes after local midnight, `0..end_minute`.
    pub start_minute: u32,
    /// Minutes after local midnight, up to 1440 for "until midnight".
    pub end_minute: u32,
    /// Monday first.
    pub weekdays: [bool; 7],
}

impl WorkingHours {
    /// `weekdays` are ISO numbers, 1 = Monday … 7 = Sunday.
    pub fn new(
        start_minute: u32,
        end_minute: u32,
        weekdays: &[u8],
    ) -> Result<Self, AvailabilityError> {
        if start_minute >= end_minute || end_minute > 24 * 60 {
            return Err(AvailabilityError::WorkingHours);
        }
        let mut days = [false; 7];
        for day in weekdays {
            let index = usize::from(*day)
                .checked_sub(1)
                .filter(|index| *index < 7)
                .ok_or(AvailabilityError::WorkingHours)?;
            days[index] = true;
        }
        if !days.contains(&true) {
            return Err(AvailabilityError::WorkingHours);
        }
        Ok(Self {
            start_minute,
            end_minute,
            weekdays: days,
        })
    }

    fn works_on(&self, date: NaiveDate) -> bool {
        self.weekdays[date.weekday().num_days_from_monday() as usize]
    }

    /// The working window of `date` in `zone`, as instants.
    fn window(&self, zone: Tz, date: NaiveDate) -> (DateTime<Utc>, DateTime<Utc>) {
        let start = wall_clock(date, self.start_minute);
        let end = wall_clock(date, self.end_minute);
        (to_utc(zone, start, false), to_utc(zone, end, true))
    }
}

/// A day's free windows, `date` in the working-hours zone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FreeDay {
    pub date: NaiveDate,
    pub windows: Vec<(DateTime<Utc>, DateTime<Utc>)>,
}

pub fn parse_zone(name: &str) -> Result<Tz, AvailabilityError> {
    name.trim()
        .parse::<Tz>()
        .map_err(|_| AvailabilityError::TimeZone)
}

fn wall_clock(date: NaiveDate, minute: u32) -> NaiveDateTime {
    let midnight = date.and_hms_opt(0, 0, 0).expect("midnight exists");
    midnight + Duration::minutes(i64::from(minute))
}

/// Place a wall-clock time in `zone`. A time inside a spring-forward gap
/// moves to the first minute after it; a repeated hour takes the earlier or,
/// with `late`, the later instant.
pub fn to_utc(zone: Tz, local: NaiveDateTime, late: bool) -> DateTime<Utc> {
    match zone.from_local_datetime(&local) {
        chrono::LocalResult::Single(time) => time.with_timezone(&Utc),
        chrono::LocalResult::Ambiguous(early, later) => {
            if late { later } else { early }.with_timezone(&Utc)
        }
        chrono::LocalResult::None => (1..=24 * 60)
            .find_map(|minute| {
                zone.from_local_datetime(&(local + Duration::minutes(minute)))
                    .earliest()
            })
            .map_or_else(
                || Utc.from_utc_datetime(&local),
                |time| time.with_timezone(&Utc),
            ),
    }
}

/// Sorted, non-overlapping intervals covering every busy block.
pub fn merge(busy: &[Busy]) -> Vec<(DateTime<Utc>, DateTime<Utc>)> {
    let mut sorted = busy
        .iter()
        .filter(|block| block.end > block.start)
        .map(|block| (block.start, block.end))
        .collect::<Vec<_>>();
    sorted.sort();
    let mut merged: Vec<(DateTime<Utc>, DateTime<Utc>)> = Vec::with_capacity(sorted.len());
    for (start, end) in sorted {
        match merged.last_mut() {
            Some(last) if start <= last.1 => last.1 = last.1.max(end),
            _ => merged.push((start, end)),
        }
    }
    merged
}

fn round_up(time: DateTime<Utc>) -> DateTime<Utc> {
    let grid = GRID_MINUTES * 60;
    let seconds = time.timestamp();
    let rounded = (seconds + grid - 1).div_euclid(grid) * grid;
    DateTime::from_timestamp(rounded, 0).unwrap_or(time)
}

fn round_down(time: DateTime<Utc>) -> DateTime<Utc> {
    let grid = GRID_MINUTES * 60;
    let rounded = time.timestamp().div_euclid(grid) * grid;
    DateTime::from_timestamp(rounded, 0).unwrap_or(time)
}

/// Free windows of at least `duration` inside the working hours of the next
/// `working_days` working days from `from`. Today counts as one of them only
/// while enough of its working time is left; a working day without a free
/// window still counts and is returned empty.
pub fn free_days(
    busy: &[Busy],
    zone: Tz,
    hours: &WorkingHours,
    from: DateTime<Utc>,
    working_days: u32,
    duration: Duration,
) -> Result<Vec<FreeDay>, AvailabilityError> {
    if duration <= Duration::zero() || duration > Duration::hours(24) {
        return Err(AvailabilityError::Duration);
    }
    if working_days == 0 || i64::from(working_days) > MAX_SCAN_DAYS {
        return Err(AvailabilityError::Range);
    }
    let merged = merge(busy);
    let earliest = round_up(from);
    let first_date = from.with_timezone(&zone).date_naive();
    let mut days = Vec::new();
    for offset in 0..MAX_SCAN_DAYS {
        if days.len() == working_days as usize {
            break;
        }
        let date = first_date + Duration::days(offset);
        if !hours.works_on(date) {
            continue;
        }
        let (day_start, day_end) = hours.window(zone, date);
        let start = day_start.max(earliest);
        if day_end - start < duration {
            // Today's working time is over, or the hours are shorter than
            // the meeting: the day does not count.
            continue;
        }
        days.push(FreeDay {
            date,
            windows: gaps(&merged, start, day_end, duration),
        });
    }
    Ok(days)
}

/// The free stretches of `[start, end)` between `merged` busy intervals,
/// snapped to the grid and at least `duration` long.
fn gaps(
    merged: &[(DateTime<Utc>, DateTime<Utc>)],
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    duration: Duration,
) -> Vec<(DateTime<Utc>, DateTime<Utc>)> {
    let mut windows = Vec::new();
    let mut cursor = start;
    let mut push = |from: DateTime<Utc>, to: DateTime<Utc>| {
        let (from, to) = (round_up(from), round_down(to));
        if to - from >= duration {
            windows.push((from, to));
        }
    };
    for (busy_start, busy_end) in merged {
        if *busy_end <= cursor {
            continue;
        }
        if *busy_start >= end {
            break;
        }
        if *busy_start > cursor {
            push(cursor, (*busy_start).min(end));
        }
        cursor = cursor.max(*busy_end);
        if cursor >= end {
            break;
        }
    }
    if cursor < end {
        push(cursor, end);
    }
    windows
}

/// Whether an iCalendar event leaves its time free. TRANSP decides when it is
/// there; without it a whole-day event is free (birthdays, holidays, "remote
/// today"), a timed one busy, as RFC 5545 and every common client have it. A
/// cancelled event takes no time either.
pub fn ics_transparent(raw: &str, all_day: bool) -> bool {
    let mut unfolded: Vec<String> = Vec::new();
    for line in raw.lines() {
        let line = line.trim_end_matches('\r');
        if (line.starts_with(' ') || line.starts_with('\t')) && !unfolded.is_empty() {
            unfolded.last_mut().expect("checked").push_str(&line[1..]);
        } else {
            unfolded.push(line.to_string());
        }
    }
    let mut in_event = false;
    let mut nested = 0_usize;
    let mut transp = None;
    for line in &unfolded {
        let upper = line.trim().to_ascii_uppercase();
        if upper == "BEGIN:VEVENT" {
            in_event = true;
            continue;
        }
        if !in_event {
            continue;
        }
        if upper == "END:VEVENT" {
            break;
        }
        // VALARM and friends carry no TRANSP of the event's own.
        if upper.starts_with("BEGIN:") {
            nested += 1;
            continue;
        }
        if upper.starts_with("END:") {
            nested = nested.saturating_sub(1);
            continue;
        }
        if nested > 0 {
            continue;
        }
        let (name, value) = match upper.split_once(':') {
            Some((name, value)) => (name.split(';').next().unwrap_or(name), value.trim()),
            None => continue,
        };
        match name {
            "STATUS" if value == "CANCELLED" => return true,
            "TRANSP" => transp = Some(value == "TRANSPARENT"),
            _ => {}
        }
    }
    transp.unwrap_or(all_day)
}

/// The calendar key the frontend's source list and its availability switches
/// use: `google:<calendar id>`, `outlook:<calendar id>`, else the source.
/// Mirrors `calendarSourceKey` in `src/utils/calendarGrid.ts`.
pub fn calendar_key(source: &str, collection: Option<&str>) -> String {
    match source {
        "google" => format!("google:{}", collection.unwrap_or("primary")),
        "outlook" => format!("outlook:{}", collection.unwrap_or("default")),
        other => other.to_string(),
    }
}

#[derive(Debug, FromRow)]
struct OwnEventRow {
    source: String,
    collection_href: Option<String>,
    starts_at: String,
    ends_at: String,
    all_day: bool,
    raw_ics: Option<String>,
}

#[derive(Debug, FromRow)]
struct OwnInviteRow {
    starts_at: String,
    ends_at: String,
    all_day: bool,
    rsvp_status: String,
}

/// Every busy interval of the account owner within `[start, end)`, across all
/// accounts' calendars except the `excluded` calendar keys. Whole-day events
/// cover their dates in `zone`. Invitations answered yes or maybe that no
/// calendar mirrors yet count too; a maybe is tentative.
pub async fn own_busy(
    pool: &SqlitePool,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    zone: Tz,
    excluded: &[String],
) -> Result<Vec<Busy>, sqlx::Error> {
    // Whole-day rows are dates; a day of margin keeps every zone's midnight
    // in the candidate set, and the exact cut happens below.
    let from = (start - Duration::days(1)).to_rfc3339();
    let to = (end + Duration::days(1)).to_rfc3339();
    let rows = sqlx::query_as::<_, OwnEventRow>(
        "SELECT source, collection_href, starts_at, ends_at, all_day, raw_ics
         FROM calendar_events
         WHERE transparent = 0
           AND ((datetime(ends_at) >= datetime(?) AND datetime(starts_at) <= datetime(?))
                OR (raw_ics IS NOT NULL
                    AND instr(upper(substr(raw_ics, instr(raw_ics, 'BEGIN:VEVENT'))), 'RRULE:') > 0))",
    )
    .bind(&from)
    .bind(&to)
    .fetch_all(pool)
    .await?;

    let invitation = crate::email::calendar::invitation_method_sql("invites.method");
    let invites = sqlx::query_as::<_, OwnInviteRow>(&format!(
        "SELECT invites.starts_at AS starts_at, invites.ends_at AS ends_at,
                invites.all_day = 1 AS all_day, invites.rsvp_status AS rsvp_status
         FROM calendar_invites invites
         WHERE invites.starts_at IS NOT NULL AND invites.ends_at IS NOT NULL
           AND invites.rsvp_status IN ('accepted', 'tentative')
           AND COALESCE(invites.cancelled, 0) = 0
           AND {invitation}
           AND datetime(invites.ends_at) >= datetime(?) AND datetime(invites.starts_at) <= datetime(?)
           AND NOT EXISTS (
               SELECT 1 FROM calendar_events events
               WHERE events.account_id = invites.account_id AND events.uid = invites.uid
           )",
    ))
    .bind(&from)
    .bind(&to)
    .fetch_all(pool)
    .await?;

    let mut busy = Vec::new();
    let mut keep = |block: Option<Busy>| {
        if let Some(block) = block.filter(|block| block.end > start && block.start < end) {
            busy.push(block);
        }
    };
    for row in rows {
        if excluded.contains(&calendar_key(&row.source, row.collection_href.as_deref())) {
            continue;
        }
        let occurrences = row
            .raw_ics
            .as_deref()
            .map(|raw| {
                crate::email::calendar::upcoming_recurrence_occurrences(
                    raw,
                    start - Duration::days(1),
                    MAX_OCCURRENCES,
                )
            })
            .unwrap_or_default();
        if occurrences.is_empty() {
            keep(block(
                &row.starts_at,
                &row.ends_at,
                row.all_day,
                zone,
                false,
            ));
        } else {
            for occurrence in occurrences {
                keep(block(
                    &occurrence.starts_at,
                    &occurrence.ends_at,
                    row.all_day,
                    zone,
                    false,
                ));
            }
        }
    }
    for invite in invites {
        keep(block(
            &invite.starts_at,
            &invite.ends_at,
            invite.all_day,
            zone,
            invite.rsvp_status == "tentative",
        ));
    }
    busy.sort_by_key(|block| (block.start, block.end));
    Ok(busy)
}

/// One stored event as an interval. Whole-day boundaries are dates (or the
/// legacy UTC-midnight form of one) and become local midnights in `zone`.
fn block(starts_at: &str, ends_at: &str, all_day: bool, zone: Tz, tentative: bool) -> Option<Busy> {
    let (start, end) = if all_day {
        let start = NaiveDate::parse_from_str(starts_at.get(..10)?, "%Y-%m-%d").ok()?;
        let end = NaiveDate::parse_from_str(ends_at.get(..10)?, "%Y-%m-%d")
            .ok()
            .filter(|end| *end > start)
            .unwrap_or(start + Duration::days(1));
        (
            to_utc(zone, wall_clock(start, 0), false),
            to_utc(zone, wall_clock(end, 0), false),
        )
    } else {
        let start = DateTime::parse_from_rfc3339(starts_at)
            .ok()?
            .with_timezone(&Utc);
        let end = DateTime::parse_from_rfc3339(ends_at)
            .ok()
            .map(|end| end.with_timezone(&Utc))
            .filter(|end| *end > start)?;
        (start, end)
    };
    Some(Busy {
        start,
        end,
        tentative,
    })
}

#[cfg(test)]
mod tests;
