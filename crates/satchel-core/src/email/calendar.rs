//! Detection + persistence of calendar invitations (RFC 5545 / iCalendar) carried as
//! email body parts or `text/calendar` attachments. Used by the sync ingest pipeline
//! to surface invites as native RSVP cards instead of raw text.

use super::text::strip_html_tags;
use icalendar::{Calendar, CalendarComponent, Component, EventLike};

const ICS_BEGIN: &str = "BEGIN:VCALENDAR";
const ICS_END: &str = "END:VCALENDAR";

/// Try to extract an iCalendar block from the message bodies. Returns the raw
/// VCALENDAR text on success.
pub fn extract_ics_block(body_text: Option<&str>, body_html: Option<&str>) -> Option<String> {
    if let Some(text) = body_text {
        if let Some(block) = extract_block(text) {
            return Some(block);
        }
    }
    if let Some(html) = body_html {
        // Strip rough HTML tags so that text/calendar embedded in HTML wrappers is found.
        let stripped = strip_html_tags(html);
        if let Some(block) = extract_block(&stripped) {
            return Some(block);
        }
    }
    None
}

fn extract_block(haystack: &str) -> Option<String> {
    let start = haystack.find(ICS_BEGIN)?;
    let end_rel = haystack[start..].find(ICS_END)?;
    let end = start + end_rel + ICS_END.len();
    Some(haystack[start..end].to_string())
}

#[derive(Debug, Clone)]
pub struct ParsedInvite {
    pub uid: String,
    pub organizer_email: Option<String>,
    pub organizer_name: Option<String>,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub location: Option<String>,
    pub starts_at: Option<String>,
    pub ends_at: Option<String>,
    pub all_day: bool,
    pub sequence: Option<i64>,
    pub method: Option<String>,
    /// Value of RECURRENCE-ID for an overridden occurrence. Absence identifies
    /// the recurring master and therefore means an RSVP applies to the series.
    pub recurrence_id: Option<String>,
    pub recurring: bool,
    /// A master RRULE without COUNT or UNTIL is intentionally unbounded. The UI
    /// must make that scope explicit before sending a series-wide response.
    pub recurrence_unbounded: bool,
    /// True for METHOD:CANCEL or STATUS:CANCELLED — the organizer called the
    /// meeting off. Previously indistinguishable from a fresh invitation.
    pub cancelled: bool,
    pub raw_ics: String,
}

pub fn parse_invite(raw_ics: &str) -> Option<ParsedInvite> {
    let calendar = raw_ics.parse::<Calendar>().ok()?;

    let method = extract_method_from_raw(raw_ics);

    for component in calendar.components.iter() {
        if let CalendarComponent::Event(event) = component {
            let uid = event.get_uid().unwrap_or("").to_string();
            if uid.is_empty() {
                continue;
            }

            // icalendar exposes property_value() but strips parameters like CN=,
            // so we re-parse the raw ORGANIZER line to recover the display name.
            let (organizer_email, organizer_name) = parse_organizer_from_raw(raw_ics);
            let summary = event.get_summary().map(str::to_string);
            let description = event.get_description().map(str::to_string);
            let location = event.get_location().map(str::to_string);
            let (starts_at, ends_at, all_day) = parse_event_times(raw_ics);
            let sequence = event
                .property_value("SEQUENCE")
                .and_then(|s| s.parse::<i64>().ok());
            let recurrence_id = event
                .property_value("RECURRENCE-ID")
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            let rrule = event.property_value("RRULE");
            let recurring = rrule.is_some()
                || event.property_value("RDATE").is_some()
                || recurrence_id.is_some();
            let recurrence_unbounded = recurrence_id.is_none()
                && rrule.is_some_and(|rule| {
                    !rule.split(';').any(|part| {
                        part.split_once('=').is_some_and(|(key, _)| {
                            key.eq_ignore_ascii_case("COUNT") || key.eq_ignore_ascii_case("UNTIL")
                        })
                    })
                });

            let cancelled = method
                .as_deref()
                .is_some_and(|m| m.eq_ignore_ascii_case("CANCEL"))
                || event
                    .property_value("STATUS")
                    .is_some_and(|s| s.eq_ignore_ascii_case("CANCELLED"));

            return Some(ParsedInvite {
                uid,
                organizer_email,
                organizer_name,
                summary,
                description,
                location,
                starts_at,
                ends_at,
                all_day,
                sequence,
                method: method.clone(),
                recurrence_id,
                recurring,
                recurrence_unbounded,
                cancelled,
                raw_ics: raw_ics.to_string(),
            });
        }
    }
    None
}

/// Build the minimal RFC 5546 METHOD:REPLY for exactly the invitation parsed
/// above. A recurring master intentionally has no RECURRENCE-ID (series-wide
/// reply); an overridden occurrence preserves the original property including
/// its TZID parameters. RRULE is optional in a REPLY and omitting it prevents a
/// receiver from expanding the response into one component per occurrence.
pub fn build_invite_reply_ics(
    invite: &ParsedInvite,
    account_email: &str,
    account_name: Option<&str>,
    partstat: &str,
) -> Result<String, String> {
    let calendar = invite
        .raw_ics
        .parse::<Calendar>()
        .map_err(|err| format!("failed to parse stored ICS: {err}"))?;
    let unfolded = unfold_ics(&invite.raw_ics);
    let raw_event = first_event_block(&unfolded);

    let has_source_event = calendar.components.iter().any(|component| {
        let CalendarComponent::Event(event) = component else {
            return false;
        };
        if event.get_uid().unwrap_or("") != invite.uid {
            return false;
        }
        let recurrence_id = event
            .property_value("RECURRENCE-ID")
            .map(str::trim)
            .filter(|value| !value.is_empty());
        recurrence_id == invite.recurrence_id.as_deref()
    });
    if !has_source_event {
        return Err(
            "stored invitation no longer contains the selected recurrence component".to_string(),
        );
    }

    // `Calendar::new()` injects its own VERSION/PRODID/CALSCALE values. Build
    // the envelope explicitly so each required property occurs exactly once.
    let mut reply = Calendar::empty();
    reply.append_property(("VERSION", "2.0"));
    reply.append_property(("PRODID", "-//Satchel//EN"));
    reply.append_property(("CALSCALE", "GREGORIAN"));
    reply.append_property(("METHOD", "REPLY"));

    let mut event = icalendar::Event::new();
    event.uid(&invite.uid);
    if let Some(sequence) = invite.sequence {
        event.append_property(("SEQUENCE", sequence.to_string().as_str()));
    }
    if let Some(organizer) = invite.organizer_email.as_deref() {
        event.append_property(("ORGANIZER", format!("mailto:{organizer}").as_str()));
    }

    if invite.recurrence_id.is_some() {
        let recurrence_line = find_property_line(raw_event, "RECURRENCE-ID")
            .ok_or_else(|| "recurring instance has no RECURRENCE-ID property".to_string())?;
        let (header, value) = recurrence_line
            .split_once(':')
            .ok_or_else(|| "invalid RECURRENCE-ID property".to_string())?;
        let mut property = icalendar::Property::new("RECURRENCE-ID", value.trim());
        for parameter in header.split(';').skip(1) {
            if let Some((key, value)) = parameter.split_once('=') {
                property.append_parameter((key.trim(), value.trim()));
            }
        }
        event.append_property(property);
    }

    let mut attendee =
        icalendar::Property::new("ATTENDEE", format!("mailto:{account_email}").as_str());
    attendee.append_parameter(("PARTSTAT", partstat));
    attendee.append_parameter(("CUTYPE", "INDIVIDUAL"));
    attendee.append_parameter(("ROLE", "REQ-PARTICIPANT"));
    if let Some(name) = account_name.filter(|name| !name.is_empty()) {
        attendee.append_parameter(("CN", name));
    }
    event.append_property(attendee);
    event.append_property((
        "DTSTAMP",
        chrono::Utc::now()
            .format("%Y%m%dT%H%M%SZ")
            .to_string()
            .as_str(),
    ));

    reply.push(event);
    Ok(reply.to_string())
}

fn find_property_line<'a>(event: &'a str, name: &str) -> Option<&'a str> {
    event.lines().find(|line| {
        let line = line.trim();
        line.get(..name.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(name))
            && matches!(line[name.len()..].chars().next(), Some(';') | Some(':'))
    })
}

fn extract_method_from_raw(raw: &str) -> Option<String> {
    for line in raw.lines() {
        let trimmed = line.trim_start_matches('\u{feff}').trim();
        if let Some(rest) = trimmed.strip_prefix("METHOD:") {
            let value = rest.trim().to_string();
            if !value.is_empty() {
                return Some(value);
            }
        }
    }
    None
}

fn parse_organizer_from_raw(raw_ics: &str) -> (Option<String>, Option<String>) {
    let unfolded = unfold_ics(raw_ics);
    for line in first_event_block(&unfolded).lines() {
        let trimmed = line.trim();
        if !trimmed.to_ascii_uppercase().starts_with("ORGANIZER") {
            continue;
        }
        // Split into "ORGANIZER[;params]" and "value"
        let Some((header, value)) = trimmed.split_once(':') else {
            continue;
        };
        let mut name: Option<String> = None;
        for param in header.split(';').skip(1) {
            if let Some(rest) = param.trim().strip_prefix("CN=") {
                let cleaned = rest.trim_matches('"').to_string();
                if !cleaned.is_empty() {
                    name = Some(cleaned);
                }
            } else if let Some(rest) = param.trim().strip_prefix("cn=") {
                let cleaned = rest.trim_matches('"').to_string();
                if !cleaned.is_empty() {
                    name = Some(cleaned);
                }
            }
        }
        let email = value
            .trim()
            .trim_start_matches("mailto:")
            .trim_start_matches("MAILTO:")
            .trim_start_matches("Mailto:")
            .to_string();
        let email = if email.is_empty() { None } else { Some(email) };
        return (email, name);
    }
    (None, None)
}

/// A DTSTART/DTEND line split into its TZID parameter and raw value.
struct IcsDateTime<'a> {
    tzid: Option<&'a str>,
    value: &'a str,
}

/// Find a date-time property in the raw ICS, preserving its parameters.
///
/// `icalendar`'s `property_value()` strips parameters, which silently discards
/// TZID — the difference between "14:30 Berlin" and "14:30 UTC".
fn find_dt_property<'a>(unfolded: &'a str, name: &str) -> Option<IcsDateTime<'a>> {
    for line in unfolded.lines() {
        let trimmed = line.trim();
        let upper = trimmed.to_ascii_uppercase();
        if !upper.starts_with(name) {
            continue;
        }
        // Must be followed by a parameter or the value separator, so DTSTAMP
        // does not match a search for DTSTART.
        let after = trimmed[name.len()..].chars().next();
        if !matches!(after, Some(';') | Some(':')) {
            continue;
        }
        let (header, value) = trimmed.split_once(':')?;
        let tzid = header.split(';').skip(1).find_map(|param| {
            let param = param.trim();
            let (key, val) = param.split_once('=')?;
            key.eq_ignore_ascii_case("TZID")
                .then(|| val.trim_matches('"'))
        });
        return Some(IcsDateTime {
            tzid,
            value: value.trim(),
        });
    }
    None
}

/// The first VEVENT block of the unfolded ICS.
///
/// DTSTART also appears inside VTIMEZONE transition rules, where it carries
/// the historical epoch of the DST rule (usually 1970). Scanning the whole
/// file made such invites start in 1970 while the end date stayed correct.
fn first_event_block(unfolded: &str) -> &str {
    let Some(begin) = unfolded.find("BEGIN:VEVENT") else {
        return unfolded;
    };
    let rest = &unfolded[begin..];
    match rest.find("END:VEVENT") {
        Some(end) => &rest[..end],
        None => rest,
    }
}

fn unfold_ics(raw: &str) -> String {
    let mut unfolded = String::with_capacity(raw.len());
    for line in raw.lines() {
        if line.starts_with(' ') || line.starts_with('\t') {
            unfolded.push_str(&line[1..]);
        } else {
            if !unfolded.is_empty() {
                unfolded.push('\n');
            }
            unfolded.push_str(line);
        }
    }
    unfolded
}

fn parse_event_times(raw_ics: &str) -> (Option<String>, Option<String>, bool) {
    let unfolded = unfold_ics(raw_ics);
    let event = first_event_block(&unfolded);
    let start = find_dt_property(event, "DTSTART");
    let end = find_dt_property(event, "DTEND");

    let all_day = start
        .as_ref()
        .map(|dt| dt.value.len() == 8 && dt.value.chars().all(|c| c.is_ascii_digit()))
        .unwrap_or(false);

    (
        start.and_then(|dt| normalize_ics_dt(dt.value, dt.tzid, all_day)),
        end.and_then(|dt| normalize_ics_dt(dt.value, dt.tzid, all_day)),
        all_day,
    )
}

/// Convert an iCalendar date-time to RFC3339 UTC.
///
/// Three forms per RFC 5545 §3.3.5, all of which used to be treated as UTC:
///   - `20260315T143000Z`             — already UTC
///   - `TZID=Europe/Berlin:20260315T143000` — local to that zone
///   - `20260315T143000`              — floating; local to the viewer
///
/// Ignoring TZID shifted appointments by the zone's offset (one or two hours
/// for most European senders), which is worse than showing nothing.
fn normalize_ics_dt(value: &str, tzid: Option<&str>, all_day: bool) -> Option<String> {
    use chrono::{NaiveDate, TimeZone};

    if all_day && value.len() == 8 {
        return Some(format!(
            "{}-{}-{}T00:00:00Z",
            &value[0..4],
            &value[4..6],
            &value[6..8],
        ));
    }

    if value.len() < 15 {
        return None;
    }
    let date_part = &value[0..8];
    let time_part = &value[9..15];
    if !date_part.chars().all(|c| c.is_ascii_digit())
        || !time_part.chars().all(|c| c.is_ascii_digit())
    {
        return None;
    }

    let naive = NaiveDate::from_ymd_opt(
        date_part[0..4].parse().ok()?,
        date_part[4..6].parse().ok()?,
        date_part[6..8].parse().ok()?,
    )?
    .and_hms_opt(
        time_part[0..2].parse().ok()?,
        time_part[2..4].parse().ok()?,
        time_part[4..6].parse().ok()?,
    )?;

    // Trailing Z wins over any TZID: an explicit UTC stamp is unambiguous.
    if value.ends_with('Z') {
        return Some(format_utc(chrono::Utc.from_utc_datetime(&naive)));
    }

    if let Some(tzid) = tzid {
        if let Some(utc) = resolve_in_zone(tzid, naive) {
            return Some(format_utc(utc));
        }
        // Unknown zone: fall through to floating rather than silently
        // pretending the wall-clock time is UTC.
    }

    // Floating time is defined as local to whoever is looking at it.
    local_to_utc(naive).map(format_utc)
}

fn format_utc(dt: chrono::DateTime<chrono::Utc>) -> String {
    dt.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

fn resolve_in_zone(
    tzid: &str,
    naive: chrono::NaiveDateTime,
) -> Option<chrono::DateTime<chrono::Utc>> {
    use chrono::TimeZone;

    let zone: chrono_tz::Tz = normalize_tzid(tzid).parse().ok()?;
    // A wall-clock time can be ambiguous (DST fall-back) or nonexistent
    // (spring-forward). Prefer the earlier instant; never drop the event.
    match zone.from_local_datetime(&naive) {
        chrono::LocalResult::Single(dt) => Some(dt.with_timezone(&chrono::Utc)),
        chrono::LocalResult::Ambiguous(earlier, _) => Some(earlier.with_timezone(&chrono::Utc)),
        chrono::LocalResult::None => {
            // Skipped hour: shift forward until the clock exists again.
            let shifted = naive + chrono::Duration::hours(1);
            zone.from_local_datetime(&shifted)
                .earliest()
                .map(|dt| dt.with_timezone(&chrono::Utc))
        }
    }
}

/// Map common non-IANA zone names onto IANA identifiers.
///
/// Exchange and Outlook still emit Windows zone names, and some servers prefix
/// the identifier with a solidus per RFC 5545's "globally unique" form.
fn normalize_tzid(tzid: &str) -> String {
    let trimmed = tzid.trim().trim_start_matches('/');
    let candidate = trimmed
        .strip_prefix("Customized Time Zone/")
        .unwrap_or(trimmed);

    match candidate {
        "W. Europe Standard Time"
        | "Romance Standard Time"
        | "Central Europe Standard Time"
        | "Central European Standard Time" => "Europe/Berlin".to_string(),
        "GMT Standard Time" => "Europe/London".to_string(),
        "Eastern Standard Time" => "America/New_York".to_string(),
        "Central Standard Time" => "America/Chicago".to_string(),
        "Mountain Standard Time" => "America/Denver".to_string(),
        "Pacific Standard Time" => "America/Los_Angeles".to_string(),
        "UTC" | "GMT" => "UTC".to_string(),
        other => other.to_string(),
    }
}

fn local_to_utc(naive: chrono::NaiveDateTime) -> Option<chrono::DateTime<chrono::Utc>> {
    use chrono::TimeZone;
    chrono::Local
        .from_local_datetime(&naive)
        .earliest()
        .map(|dt| dt.with_timezone(&chrono::Utc))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecurrenceOccurrence {
    pub starts_at: String,
    pub ends_at: String,
}

/// Expand the common RFC 5545 recurrence forms used by CalDAV into upcoming
/// concrete instances. CalDAV returns a recurring resource once, with the
/// original DTSTART in the past; without this step a current series silently
/// disappears from compact views that only query future timestamps.
pub fn upcoming_recurrence_occurrences(
    raw_ics: &str,
    after: chrono::DateTime<chrono::Utc>,
    limit: usize,
) -> Vec<RecurrenceOccurrence> {
    use chrono::{Datelike, Duration};

    if limit == 0 {
        return Vec::new();
    }
    let unfolded = unfold_ics(raw_ics);
    let event = first_event_block(&unfolded);
    let Some(rrule) = event.lines().find_map(|line| {
        line.trim()
            .strip_prefix("RRULE:")
            .or_else(|| line.trim().strip_prefix("rrule:"))
    }) else {
        return Vec::new();
    };
    let Some(start_prop) = find_dt_property(event, "DTSTART") else {
        return Vec::new();
    };
    let all_day = start_prop.value.len() == 8;
    let Some(anchor) = parse_ics_naive(start_prop.value, all_day) else {
        return Vec::new();
    };
    let end_naive = find_dt_property(event, "DTEND")
        .and_then(|value| parse_ics_naive(value.value, all_day))
        .unwrap_or_else(|| {
            anchor
                + if all_day {
                    Duration::days(1)
                } else {
                    Duration::hours(1)
                }
        });
    let duration = end_naive - anchor;

    let mut freq = "";
    let mut interval = 1_i64;
    let mut count = None;
    let mut until = None;
    let mut by_days = Vec::new();
    let mut by_month_days = Vec::new();
    for part in rrule.split(';') {
        let Some((key, value)) = part.split_once('=') else {
            continue;
        };
        match key.to_ascii_uppercase().as_str() {
            "FREQ" => freq = value,
            "INTERVAL" => interval = value.parse::<i64>().unwrap_or(1).max(1),
            "COUNT" => count = value.parse::<usize>().ok(),
            "UNTIL" => {
                until = normalize_ics_dt(value, start_prop.tzid, value.len() == 8)
                    .and_then(|value| chrono::DateTime::parse_from_rfc3339(&value).ok())
                    .map(|value| value.with_timezone(&chrono::Utc));
            }
            "BYDAY" => {
                by_days.extend(value.split(',').filter_map(parse_by_day));
            }
            "BYMONTHDAY" => {
                by_month_days.extend(value.split(',').filter_map(|day| day.parse::<i32>().ok()));
            }
            _ => {}
        }
    }
    if !matches!(
        freq.to_ascii_uppercase().as_str(),
        "DAILY" | "WEEKLY" | "MONTHLY"
    ) {
        return Vec::new();
    }

    let anchor_date = anchor.date();
    let scan_end = after.date_naive() + Duration::days(730);
    let mut date = anchor_date;
    let mut matched = 0_usize;
    let mut occurrences = Vec::new();
    while date <= scan_end {
        let days = (date - anchor_date).num_days();
        let months = (date.year() - anchor_date.year()) as i64 * 12 + date.month() as i64
            - anchor_date.month() as i64;
        let matches = match freq.to_ascii_uppercase().as_str() {
            "DAILY" => days >= 0 && days % interval == 0,
            "WEEKLY" => {
                days >= 0
                    && (days / 7) % interval == 0
                    && if by_days.is_empty() {
                        date.weekday() == anchor_date.weekday()
                    } else {
                        by_days
                            .iter()
                            .any(|(_, weekday)| *weekday == date.weekday())
                    }
            }
            "MONTHLY" => {
                months >= 0
                    && months % interval == 0
                    && if !by_month_days.is_empty() {
                        by_month_days.iter().any(|day| *day == date.day() as i32)
                    } else if !by_days.is_empty() {
                        by_days.iter().any(|(ordinal, weekday)| {
                            *weekday == date.weekday()
                                && ordinal
                                    .is_none_or(|ordinal| matches_month_ordinal(date, ordinal))
                        })
                    } else {
                        date.day() == anchor_date.day()
                    }
            }
            _ => false,
        };

        if matches {
            matched += 1;
            if count.is_some_and(|count| matched > count) {
                break;
            }
            let candidate = date.and_time(anchor.time());
            if let Some(start) = recurrence_to_utc(candidate, &start_prop, all_day) {
                if until.is_some_and(|until| start > until) {
                    break;
                }
                let end_candidate = candidate + duration;
                let end = recurrence_to_utc(end_candidate, &start_prop, all_day)
                    .unwrap_or(start + duration);
                if end >= after {
                    occurrences.push(RecurrenceOccurrence {
                        starts_at: format_utc(start),
                        ends_at: format_utc(end),
                    });
                    if occurrences.len() == limit {
                        break;
                    }
                }
            }
        }
        let Some(next) = date.succ_opt() else {
            break;
        };
        date = next;
    }
    occurrences
}

fn parse_ics_naive(value: &str, all_day: bool) -> Option<chrono::NaiveDateTime> {
    use chrono::NaiveDate;
    if all_day && value.len() == 8 {
        return NaiveDate::parse_from_str(value, "%Y%m%d")
            .ok()?
            .and_hms_opt(0, 0, 0);
    }
    chrono::NaiveDateTime::parse_from_str(value.trim_end_matches('Z'), "%Y%m%dT%H%M%S").ok()
}

fn parse_by_day(value: &str) -> Option<(Option<i32>, chrono::Weekday)> {
    let value = value.trim().to_ascii_uppercase();
    if value.len() < 2 {
        return None;
    }
    let (ordinal, day) = value.split_at(value.len() - 2);
    let weekday = match day {
        "MO" => chrono::Weekday::Mon,
        "TU" => chrono::Weekday::Tue,
        "WE" => chrono::Weekday::Wed,
        "TH" => chrono::Weekday::Thu,
        "FR" => chrono::Weekday::Fri,
        "SA" => chrono::Weekday::Sat,
        "SU" => chrono::Weekday::Sun,
        _ => return None,
    };
    let ordinal = if ordinal.is_empty() {
        None
    } else {
        ordinal.parse::<i32>().ok()
    };
    Some((ordinal, weekday))
}

fn matches_month_ordinal(date: chrono::NaiveDate, ordinal: i32) -> bool {
    use chrono::Datelike;
    if ordinal > 0 {
        return ((date.day() - 1) / 7 + 1) as i32 == ordinal;
    }
    let next_month = if date.month() == 12 {
        chrono::NaiveDate::from_ymd_opt(date.year() + 1, 1, 1)
    } else {
        chrono::NaiveDate::from_ymd_opt(date.year(), date.month() + 1, 1)
    };
    let Some(last_day) = next_month.and_then(|date| date.pred_opt()) else {
        return false;
    };
    -(((last_day.day() - date.day()) / 7 + 1) as i32) == ordinal
}

fn recurrence_to_utc(
    naive: chrono::NaiveDateTime,
    original: &IcsDateTime<'_>,
    all_day: bool,
) -> Option<chrono::DateTime<chrono::Utc>> {
    use chrono::TimeZone;
    if all_day || original.value.ends_with('Z') {
        return Some(chrono::Utc.from_utc_datetime(&naive));
    }
    original
        .tzid
        .and_then(|tzid| resolve_in_zone(tzid, naive))
        .or_else(|| local_to_utc(naive))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_ICS: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nMETHOD:REQUEST\r\nPRODID:-//Test//EN\r\nBEGIN:VEVENT\r\nUID:abc-123@example.com\r\nORGANIZER;CN=Alice:mailto:alice@example.com\r\nSUMMARY:Wochenplanung\r\nLOCATION:Raum 4.2\r\nDTSTART:20260315T143000Z\r\nDTEND:20260315T153000Z\r\nSEQUENCE:0\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    #[test]
    fn extracts_block() {
        let found = extract_ics_block(Some(&format!("Hi\n{SAMPLE_ICS}\nbye")), None);
        assert!(found.is_some());
        assert!(found.unwrap().starts_with("BEGIN:VCALENDAR"));
    }

    #[test]
    fn parses_invite() {
        let parsed = parse_invite(SAMPLE_ICS).expect("parses");
        assert_eq!(parsed.uid, "abc-123@example.com");
        assert_eq!(parsed.organizer_email.as_deref(), Some("alice@example.com"));
        assert_eq!(parsed.organizer_name.as_deref(), Some("Alice"));
        assert_eq!(parsed.summary.as_deref(), Some("Wochenplanung"));
        assert_eq!(parsed.starts_at.as_deref(), Some("2026-03-15T14:30:00Z"));
        assert_eq!(parsed.method.as_deref(), Some("REQUEST"));
    }

    const TZ_ICS: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:tz-1@example.com\r\nSUMMARY:Standup\r\nDTSTART;TZID=Europe/Berlin:20260315T143000\r\nDTEND;TZID=Europe/Berlin:20260315T150000\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    const CANCEL_ICS: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nMETHOD:CANCEL\r\nBEGIN:VEVENT\r\nUID:abc-123@example.com\r\nSUMMARY:Wochenplanung\r\nDTSTART:20260315T143000Z\r\nSEQUENCE:1\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    #[test]
    fn honours_tzid_instead_of_pretending_local_time_is_utc() {
        let parsed = parse_invite(TZ_ICS).expect("parses");
        // 15 March 2026 is still CET (+1), so 14:30 Berlin is 13:30 UTC.
        // Treating the wall-clock time as UTC put every European invitation
        // an hour or two out.
        assert_eq!(parsed.starts_at.as_deref(), Some("2026-03-15T13:30:00Z"));
        assert_eq!(parsed.ends_at.as_deref(), Some("2026-03-15T14:00:00Z"));
    }

    #[test]
    fn applies_the_offset_in_force_on_the_event_date() {
        // Same zone, same wall-clock time, different side of the DST switch
        // (EU changes on 29 March 2026): +1 in March, +2 in July.
        let summer = TZ_ICS.replace("20260315", "20260715");
        assert_eq!(
            parse_invite(&summer).expect("parses").starts_at.as_deref(),
            Some("2026-07-15T12:30:00Z"),
        );

        let winter = TZ_ICS.replace("20260315", "20260115");
        assert_eq!(
            parse_invite(&winter).expect("parses").starts_at.as_deref(),
            Some("2026-01-15T13:30:00Z"),
        );
    }

    #[test]
    fn keeps_explicit_utc_stamps_unchanged() {
        let parsed = parse_invite(SAMPLE_ICS).expect("parses");
        assert_eq!(parsed.starts_at.as_deref(), Some("2026-03-15T14:30:00Z"));
    }

    #[test]
    fn maps_windows_zone_names_from_exchange() {
        let windows = TZ_ICS.replace("Europe/Berlin", "W. Europe Standard Time");
        let parsed = parse_invite(&windows).expect("parses");
        assert_eq!(parsed.starts_at.as_deref(), Some("2026-03-15T13:30:00Z"));
    }

    #[test]
    fn does_not_confuse_dtstamp_with_dtstart() {
        let with_stamp = TZ_ICS.replace(
            "BEGIN:VEVENT\r\n",
            "BEGIN:VEVENT\r\nDTSTAMP:20260101T000000Z\r\n",
        );
        let parsed = parse_invite(&with_stamp).expect("parses");
        assert_eq!(parsed.starts_at.as_deref(), Some("2026-03-15T13:30:00Z"));
    }

    #[test]
    fn ignores_dtstart_lines_inside_vtimezone_rules() {
        // Zoom/Exchange invites ship a VTIMEZONE block before the VEVENT whose
        // DST rules carry DTSTART:1970… lines. Those must never win over the
        // event's own start.
        let ics = concat!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nMETHOD:REQUEST\r\n",
            "BEGIN:VTIMEZONE\r\nTZID:Europe/Berlin\r\n",
            "BEGIN:DAYLIGHT\r\nDTSTART:19700329T020000\r\n",
            "TZOFFSETFROM:+0100\r\nTZOFFSETTO:+0200\r\nEND:DAYLIGHT\r\n",
            "BEGIN:STANDARD\r\nDTSTART:19701025T030000\r\n",
            "TZOFFSETFROM:+0200\r\nTZOFFSETTO:+0100\r\nEND:STANDARD\r\n",
            "END:VTIMEZONE\r\n",
            "BEGIN:VEVENT\r\nUID:tz-2@example.com\r\nSUMMARY:Roadmap\r\n",
            "DTSTART;TZID=Europe/Berlin:20260714T160000\r\n",
            "DTEND;TZID=Europe/Berlin:20260714T170000\r\n",
            "END:VEVENT\r\nEND:VCALENDAR\r\n",
        );
        let parsed = parse_invite(ics).expect("parses");
        assert_eq!(parsed.starts_at.as_deref(), Some("2026-07-14T14:00:00Z"));
        assert_eq!(parsed.ends_at.as_deref(), Some("2026-07-14T15:00:00Z"));
        assert!(!parsed.all_day);
    }

    #[test]
    fn falls_back_to_floating_time_for_an_unknown_zone() {
        let bogus = TZ_ICS.replace("Europe/Berlin", "Mars/Olympus_Mons");
        // Must still produce a time rather than dropping the event.
        assert!(parse_invite(&bogus).expect("parses").starts_at.is_some());
    }

    #[test]
    fn marks_cancellations_instead_of_showing_them_as_invitations() {
        let parsed = parse_invite(CANCEL_ICS).expect("parses");
        assert!(
            parsed.cancelled,
            "METHOD:CANCEL must mark the invite cancelled"
        );
        assert_eq!(parsed.method.as_deref(), Some("CANCEL"));
    }

    #[test]
    fn treats_status_cancelled_as_a_cancellation_too() {
        let ics = SAMPLE_ICS.replace("SEQUENCE:0", "STATUS:CANCELLED\r\nSEQUENCE:0");
        assert!(parse_invite(&ics).expect("parses").cancelled);
    }

    #[test]
    fn a_plain_request_is_not_cancelled() {
        assert!(!parse_invite(SAMPLE_ICS).expect("parses").cancelled);
    }

    #[test]
    fn handles_all_day_events() {
        let ics = SAMPLE_ICS
            .replace("DTSTART:20260315T143000Z", "DTSTART;VALUE=DATE:20260315")
            .replace("DTEND:20260315T153000Z", "DTEND;VALUE=DATE:20260316");
        let parsed = parse_invite(&ics).expect("parses");
        assert!(parsed.all_day);
        assert_eq!(parsed.starts_at.as_deref(), Some("2026-03-15T00:00:00Z"));
    }

    #[test]
    fn expands_a_biweekly_series_at_the_local_wall_clock_time() {
        let recurring = concat!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\n",
            "UID:series@example.com\r\nSUMMARY:Regeltermin\r\n",
            "DTSTART;TZID=Europe/Berlin:20250122T143000\r\n",
            "DTEND;TZID=Europe/Berlin:20250122T153000\r\n",
            "RRULE:FREQ=WEEKLY;INTERVAL=2;BYDAY=WE\r\n",
            "END:VEVENT\r\nEND:VCALENDAR\r\n",
        );
        let after = chrono::DateTime::parse_from_rfc3339("2026-08-04T12:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);

        let occurrences = upcoming_recurrence_occurrences(recurring, after, 2);

        assert_eq!(occurrences.len(), 2);
        assert_eq!(occurrences[0].starts_at, "2026-08-05T12:30:00Z");
        assert_eq!(occurrences[0].ends_at, "2026-08-05T13:30:00Z");
        assert_eq!(occurrences[1].starts_at, "2026-08-19T12:30:00Z");
    }

    #[test]
    fn respects_count_when_a_series_has_finished() {
        let recurring = concat!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\n",
            "UID:finished@example.com\r\nSUMMARY:Finished\r\n",
            "DTSTART:20260101T090000Z\r\nDTEND:20260101T100000Z\r\n",
            "RRULE:FREQ=WEEKLY;COUNT=2;BYDAY=TH\r\n",
            "END:VEVENT\r\nEND:VCALENDAR\r\n",
        );
        let after = chrono::DateTime::parse_from_rfc3339("2026-08-04T12:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);

        assert!(upcoming_recurrence_occurrences(recurring, after, 3).is_empty());
    }
}
