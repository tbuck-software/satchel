//! Lossless updates of the editor's supported VEVENT fields.
//!
//! Keep property spelling, parameters, folding and child components from the
//! stored resource. Never round-trip a DAV resource through a reduced DTO.
use super::{normalize_attendees, EventInput, SaveError};
use crate::db::calendar::CalendarEventRecord;
use chrono::{DateTime, Utc};
use std::collections::BTreeMap;

const UNSUPPORTED: &str = "backend.errors.calendarEditUnsupported";
fn unsupported() -> SaveError {
    SaveError::Validation(UNSUPPORTED)
}

fn unfolded(line: &str) -> String {
    line.replace("\r\n ", "").replace("\r\n\t", "")
}

fn separator(line: &str) -> Option<usize> {
    // Quoted parameters may contain colons.
    let mut quoted = false;
    line.char_indices().find_map(|(index, ch)| {
        if ch == '"' {
            quoted = !quoted;
        }
        (ch == ':' && !quoted).then_some(index)
    })
}

fn property(line: &str) -> (String, String) {
    let line = unfolded(line);
    let Some(colon) = separator(&line) else {
        return (String::new(), String::new());
    };
    (
        line[..colon]
            .split(';')
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase(),
        line[colon + 1..].to_string(),
    )
}

fn with_value(key: &str, value: &str, fields: &[usize], lines: &[String]) -> String {
    let header = fields
        .iter()
        .find_map(|&index| {
            let line = unfolded(&lines[index]);
            (property(&line).0 == key)
                .then(|| line[..separator(&line).expect("property has a separator")].to_string())
        })
        .unwrap_or_else(|| key.to_string());
    format!("{header}:{value}")
}

fn datetime_value(
    key: &str,
    instant: DateTime<Utc>,
    all_day: bool,
    fields: &[usize],
    lines: &[String],
) -> Result<String, SaveError> {
    if all_day {
        return Ok(instant.format("%Y%m%d").to_string());
    }
    if let Some(line) = fields
        .iter()
        .find_map(|&index| (property(&lines[index]).0 == key).then(|| unfolded(&lines[index])))
    {
        let colon = separator(&line).ok_or_else(unsupported)?;
        let tzid = line[..colon].split(';').skip(1).find_map(|param| {
            let (name, value) = param.split_once('=')?;
            name.eq_ignore_ascii_case("TZID")
                .then_some(value.trim_matches('"'))
        });
        if let Some(tzid) = tzid {
            // Embedded definitions can differ from the system database even
            // when they use an IANA name. Preserve them without reinterpreting.
            if lines.iter().any(|line| {
                let (key, value) = property(line);
                key == "BEGIN" && value == "VTIMEZONE"
            }) {
                return Err(unsupported());
            }
            let zone: chrono_tz::Tz = tzid.parse().map_err(|_| unsupported())?;
            if line[colon + 1..].ends_with('Z') {
                return Err(unsupported());
            }
            return Ok(instant
                .with_timezone(&zone)
                .format("%Y%m%dT%H%M%S")
                .to_string());
        }
        // A floating time has no stable UTC interpretation for other viewers.
        if !line[colon + 1..].ends_with('Z') {
            return Err(unsupported());
        }
    }
    Ok(instant.format("%Y%m%dT%H%M%SZ").to_string())
}

pub(super) fn patch_event(
    raw: &str,
    old: &CalendarEventRecord,
    input: &EventInput,
    starts_at: DateTime<Utc>,
    ends_at: DateTime<Utc>,
    organizer: &str,
) -> Result<String, SaveError> {
    let mut lines: Vec<String> = Vec::new();
    for line in raw.lines() {
        if line.starts_with([' ', '\t']) {
            let previous = lines.last_mut().ok_or_else(unsupported)?;
            previous.push_str("\r\n");
            previous.push_str(line);
        } else {
            lines.push(line.to_string());
        }
    }
    let mut stack: Vec<String> = Vec::new();
    let mut masters = Vec::new();
    let mut events = Vec::new();
    let mut fields: Vec<usize> = Vec::new();
    let mut recurrent = false;
    for (index, line) in lines.iter().enumerate() {
        let (key, value) = property(line);
        match key.as_str() {
            "BEGIN" => {
                if stack.is_empty() && value != "VCALENDAR" {
                    return Err(unsupported());
                }
                if value == "VEVENT" {
                    if stack.as_slice() != ["VCALENDAR"] {
                        return Err(unsupported());
                    }
                    fields.clear();
                }
                stack.push(value);
            }
            "END" => {
                if stack.last() != Some(&value) {
                    return Err(unsupported());
                }
                if value == "VEVENT" {
                    let uid = fields
                        .iter()
                        .find_map(|&i| {
                            let (key, value) = property(&lines[i]);
                            (key == "UID").then_some(value)
                        })
                        .ok_or_else(unsupported)?;
                    if uid != old.uid {
                        return Err(unsupported());
                    }
                    if !fields
                        .iter()
                        .any(|&i| property(&lines[i]).0 == "RECURRENCE-ID")
                    {
                        masters.push((fields.clone(), index));
                    }
                    events.push(index);
                }
                stack.pop();
            }
            _ if stack.as_slice() == ["VCALENDAR", "VEVENT"] => {
                recurrent |= matches!(
                    key.as_str(),
                    "RRULE" | "RDATE" | "EXRULE" | "EXDATE" | "RECURRENCE-ID"
                );
                fields.push(index);
            }
            _ => (),
        }
    }
    if !stack.is_empty()
        || masters.len() != 1
        || events.first() != masters.first().map(|(_, end)| end)
    {
        return Err(unsupported());
    }
    let (fields, end) = masters.pop().ok_or_else(unsupported)?;
    let mut seen = std::collections::HashSet::new();
    for &index in &fields {
        let (key, value) = property(&lines[index]);
        if matches!(
            key.as_str(),
            "UID"
                | "DTSTART"
                | "DTEND"
                | "DURATION"
                | "SUMMARY"
                | "DESCRIPTION"
                | "LOCATION"
                | "SEQUENCE"
                | "DTSTAMP"
                | "ORGANIZER"
        ) && !seen.insert(key.clone())
        {
            return Err(unsupported());
        }
        if key == "ORGANIZER"
            && !value
                .to_ascii_lowercase()
                .strip_prefix("mailto:")
                .is_some_and(|email| email.eq_ignore_ascii_case(organizer))
        {
            return Err(unsupported());
        }
    }
    let old_start = super::parse_calendar_time(&old.starts_at, old.all_day == 1)?;
    let old_end = super::parse_calendar_time(&old.ends_at, old.all_day == 1)?;
    // The grid substitutes an hour for missing/zero ends. That fallback is
    // not the original interval and must never overwrite DURATION semantics.
    if old_end <= old_start {
        return Err(unsupported());
    }
    let time_changed =
        old_start != starts_at || old_end != ends_at || (old.all_day == 1) != input.all_day;
    // Moving a series requires a recurrence-aware editor. Likewise, changing
    // participants requires cancellation/update scheduling beyond this form.
    let old_attendees: Vec<serde_json::Value> =
        serde_json::from_str(&old.attendees_json).map_err(|_| unsupported())?;
    let old_attendees = old_attendees
        .iter()
        .filter_map(|value| {
            value
                .as_str()
                .or_else(|| value.get("email").and_then(|email| email.as_str()))
        })
        .map(str::to_string)
        .collect::<Vec<_>>();
    if ((recurrent || seen.contains("DURATION")) && time_changed)
        || normalize_attendees(&old_attendees) != normalize_attendees(&input.attendees)
    {
        return Err(unsupported());
    }
    let mut replacements: BTreeMap<&str, Option<String>> = BTreeMap::new();
    if input.title.trim() != old.title {
        replacements.insert(
            "SUMMARY",
            Some(with_value(
                "SUMMARY",
                &super::escape_ics(input.title.trim()),
                &fields,
                &lines,
            )),
        );
    }
    for (key, new, old) in [
        ("DESCRIPTION", &input.description, &old.description),
        ("LOCATION", &input.location, &old.location),
    ] {
        if new != old {
            replacements.insert(
                key,
                new.as_ref()
                    .filter(|value| !value.trim().is_empty())
                    .map(|value| with_value(key, &super::escape_ics(value), &fields, &lines)),
            );
        }
    }
    if time_changed {
        for (key, value) in [("DTSTART", starts_at), ("DTEND", ends_at)] {
            let value = if (old.all_day == 1) == input.all_day {
                datetime_value(key, value, input.all_day, &fields, &lines)?
            } else if input.all_day {
                value.format("%Y%m%d").to_string()
            } else {
                value.format("%Y%m%dT%H%M%SZ").to_string()
            };
            let line = if (old.all_day == 1) == input.all_day {
                if input.all_day && !fields.iter().any(|&index| property(&lines[index]).0 == key) {
                    format!("{key};VALUE=DATE:{value}")
                } else {
                    with_value(key, &value, &fields, &lines)
                }
            } else {
                // Switching DATE/DATE-TIME changes VALUE intentionally; any
                // additional parameters need an editor that can represent them.
                for &index in &fields {
                    let original = unfolded(&lines[index]);
                    if property(&original).0 == key {
                        let header =
                            &original[..separator(&original).expect("property has a separator")];
                        if header.split(';').skip(1).any(|param| {
                            !param.eq_ignore_ascii_case("VALUE=DATE")
                                && !param.eq_ignore_ascii_case("VALUE=DATE-TIME")
                        }) {
                            return Err(unsupported());
                        }
                    }
                }
                if input.all_day {
                    format!("{key};VALUE=DATE:{value}")
                } else {
                    format!("{key}:{value}")
                }
            };
            replacements.insert(key, Some(line));
        }
    }

    if replacements.is_empty() {
        return Ok(raw.to_string());
    }
    let sequence = fields.iter().find_map(|&i| {
        let (key, value) = property(&lines[i]);
        (key == "SEQUENCE").then_some(value)
    });
    let sequence = sequence
        .as_deref()
        .unwrap_or("0")
        .parse::<u32>()
        .ok()
        .and_then(|v| v.checked_add(1))
        .ok_or_else(unsupported)?;
    replacements.insert(
        "SEQUENCE",
        Some(with_value(
            "SEQUENCE",
            &sequence.to_string(),
            &fields,
            &lines,
        )),
    );
    replacements.insert(
        "DTSTAMP",
        Some(with_value(
            "DTSTAMP",
            &Utc::now().format("%Y%m%dT%H%M%SZ").to_string(),
            &fields,
            &lines,
        )),
    );
    // Missing properties belong before child components such as VALARM.
    let insertion = fields.last().map_or(end, |index| index + 1);
    let mut output = Vec::new();
    for (index, line) in lines.into_iter().enumerate() {
        if index == insertion {
            output.extend(replacements.values().filter_map(Clone::clone));
        }
        if fields.binary_search(&index).is_ok() {
            let (key, _) = property(&line);
            if let Some(replacement) = replacements.remove(key.as_str()) {
                if let Some(line) = replacement {
                    output.push(line);
                }
                continue;
            }
        }
        output.push(line);
    }
    Ok(format!("{}\r\n", output.join("\r\n")))
}
