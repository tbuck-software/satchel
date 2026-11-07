//! iCalendar VTODO: read the fields Satchel shows, write new resources, and
//! patch existing ones without losing what another client put there.
//!
//! A patch replaces a property only when its value changed. An untouched due
//! date keeps its `TZID`, an untouched alarm keeps its relative trigger, and
//! properties Satchel has no field for (CATEGORIES, RELATED-TO, EMAIL alarms,
//! …) pass through unchanged. RRULE is read so a repeating task can advance
//! instead of ending its series (`super::recurrence`).
use chrono::{DateTime, Duration, NaiveDate, Utc};

use crate::email::calendar::{local_to_utc, normalize_ics_dt, normalize_tzid};
use crate::tasks::recurrence::{ZONE_FLOATING, ZONE_UTC};

/// The fields of a task that live in its iCalendar resource.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TaskFields {
    pub uid: String,
    pub title: String,
    pub notes: Option<String>,
    /// `YYYY-MM-DD` or RFC 3339 UTC (`YYYY-MM-DDTHH:MM:SSZ`).
    pub due_at: Option<String>,
    /// RFC 3339 UTC.
    pub reminder_at: Option<String>,
    /// 0 none, 1 highest … 9 lowest.
    pub priority: u8,
    /// RFC 3339 UTC.
    pub completed_at: Option<String>,
    /// RFC 5322 Message-ID of the mail the task was made from.
    pub message_id: Option<String>,
    /// DTSTART, in the same two forms as `due_at`.
    pub start_at: Option<String>,
    /// How DUE was written, for stepping a repeating task on the right clock:
    /// its TZID, `UTC` for a `Z` stamp, `floating`, or None for a date.
    pub due_tz: Option<String>,
    /// The RRULE value, verbatim.
    pub rrule: Option<String>,
    /// EXDATE, RDATE or overridden instances next to the RRULE: a series
    /// Satchel does not step itself.
    pub rrule_extras: bool,
}

const MESSAGE_ID_PROPERTY: &str = "X-SATCHEL-MESSAGE-ID";

/// One content line: name, raw parameter text (with the leading `;`), value.
#[derive(Debug, Clone)]
struct Line {
    name: String,
    params: String,
    value: String,
}

impl Line {
    fn parse(raw: &str) -> Option<Self> {
        let colon = separator(raw)?;
        let header = &raw[..colon];
        let (name, params) = match header.find(';') {
            Some(index) => (&header[..index], &header[index..]),
            None => (header, ""),
        };
        Some(Self {
            name: name.trim().to_ascii_uppercase(),
            params: params.to_string(),
            value: raw[colon + 1..].to_string(),
        })
    }

    fn param(&self, key: &str) -> Option<&str> {
        self.params.split(';').skip(1).find_map(|param| {
            let (name, value) = param.split_once('=')?;
            name.trim()
                .eq_ignore_ascii_case(key)
                .then(|| value.trim().trim_matches('"'))
        })
    }
}

/// The value separator; quoted parameters may contain colons.
fn separator(line: &str) -> Option<usize> {
    let mut quoted = false;
    line.char_indices().find_map(|(index, ch)| {
        if ch == '"' {
            quoted = !quoted;
        }
        (ch == ':' && !quoted).then_some(index)
    })
}

fn unfold(raw: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in raw.replace("\r\n", "\n").split('\n') {
        if (line.starts_with(' ') || line.starts_with('\t')) && !out.is_empty() {
            out.last_mut().expect("checked").push_str(&line[1..]);
        } else if !line.is_empty() {
            out.push(line.to_string());
        }
    }
    out
}

/// Fold to 75 octets per RFC 5545 §3.1, never inside a UTF-8 sequence.
fn fold(line: &str) -> String {
    let mut out = String::with_capacity(line.len() + line.len() / 70 * 3);
    let mut width = 0;
    for ch in line.chars() {
        let len = ch.len_utf8();
        if width + len > 75 {
            out.push_str("\r\n ");
            width = 1;
        }
        out.push(ch);
        width += len;
    }
    out
}

fn escape_text(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace(';', "\\;")
        .replace(',', "\\,")
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\n', "\\n")
}

fn unescape_text(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('n' | 'N') => out.push('\n'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

fn ics_utc(value: &str) -> Option<String> {
    let instant = DateTime::parse_from_rfc3339(value).ok()?;
    Some(
        instant
            .with_timezone(&Utc)
            .format("%Y%m%dT%H%M%SZ")
            .to_string(),
    )
}

fn date_value(line: &Line) -> Option<String> {
    let value = line.value.trim();
    let is_date = line
        .param("VALUE")
        .is_some_and(|kind| kind.eq_ignore_ascii_case("DATE"))
        || (value.len() == 8 && value.chars().all(|ch| ch.is_ascii_digit()));
    if is_date {
        let date = NaiveDate::parse_from_str(value.get(..8)?, "%Y%m%d").ok()?;
        return Some(date.format("%Y-%m-%d").to_string());
    }
    normalize_ics_dt(value, line.param("TZID"), false)
}

/// The instant a due date stands for: a date means the start of that day where
/// the user is, which is when a relative alarm on it is meant to count from.
fn due_instant(due: &str) -> Option<DateTime<Utc>> {
    if due.len() == 10 {
        let date = NaiveDate::parse_from_str(due, "%Y-%m-%d").ok()?;
        return local_to_utc(date.and_hms_opt(0, 0, 0)?);
    }
    DateTime::parse_from_rfc3339(due)
        .ok()
        .map(|value| value.with_timezone(&Utc))
}

fn is_date(value: &str) -> bool {
    value.len() == 10
}

/// DTSTART only stands next to DUE in the same form and not after it
/// (RFC 5545 §3.8.2.3); otherwise it is dropped rather than made invalid.
fn consistent_start(start: Option<&str>, due: Option<&str>) -> Option<String> {
    let start = start?;
    if let Some(due) = due {
        if is_date(start) != is_date(due) {
            return None;
        }
        if due_instant(start)
            .zip(due_instant(due))
            .is_some_and(|(start, due)| start > due)
        {
            return None;
        }
    }
    Some(start.to_string())
}

/// How a date-time property is written, kept when its value changes.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Form {
    Utc,
    Floating,
    Zone(String),
}

impl Form {
    fn of(line: &Line) -> Self {
        match line.param("TZID") {
            Some(tzid) => Self::Zone(tzid.to_string()),
            None if line.value.trim().ends_with('Z') => Self::Utc,
            None => Self::Floating,
        }
    }

    /// From `TaskFields::due_tz`, for a property the resource did not have.
    fn stored(due_tz: Option<&str>) -> Self {
        match due_tz {
            None | Some(ZONE_UTC) => Self::Utc,
            Some(ZONE_FLOATING) => Self::Floating,
            Some(tzid) => Self::Zone(tzid.to_string()),
        }
    }
}

/// How a DUE line was written, as `TaskFields::due_tz` records it.
fn due_zone(line: &Line) -> Option<String> {
    let value = line.value.trim();
    if line
        .param("VALUE")
        .is_some_and(|kind| kind.eq_ignore_ascii_case("DATE"))
        || value.len() == 8
    {
        return None;
    }
    Some(match Form::of(line) {
        Form::Utc => ZONE_UTC.to_string(),
        Form::Floating => ZONE_FLOATING.to_string(),
        Form::Zone(tzid) => tzid,
    })
}

/// `NAME;VALUE=DATE:…`, or a date-time in the property's own form: in its
/// TZID (so a moved due date keeps its zone), floating (wall time where the
/// user is, as it was read), or UTC.
fn date_line(name: &str, value: &str, form: &Form) -> Option<String> {
    if is_date(value) {
        let date = NaiveDate::parse_from_str(value, "%Y-%m-%d").ok()?;
        return Some(format!("{name};VALUE=DATE:{}", date.format("%Y%m%d")));
    }
    let instant = DateTime::parse_from_rfc3339(value)
        .ok()?
        .with_timezone(&Utc);
    match form {
        Form::Zone(tzid) => {
            if let Ok(tz) = normalize_tzid(tzid).parse::<chrono_tz::Tz>() {
                return Some(format!(
                    "{name};TZID={tzid}:{}",
                    instant.with_timezone(&tz).format("%Y%m%dT%H%M%S")
                ));
            }
        }
        Form::Floating => {
            return Some(format!(
                "{name}:{}",
                instant
                    .with_timezone(&chrono::Local)
                    .format("%Y%m%dT%H%M%S")
            ));
        }
        Form::Utc => {}
    }
    Some(format!("{name}:{}", instant.format("%Y%m%dT%H%M%SZ")))
}

/// `[+-]P[nW][nD][T[nH][nM][nS]]` (RFC 5545 §3.3.6).
fn parse_duration(value: &str) -> Option<Duration> {
    let value = value.trim();
    let (negative, rest) = match value.as_bytes().first()? {
        b'-' => (true, &value[1..]),
        b'+' => (false, &value[1..]),
        _ => (false, value),
    };
    let rest = rest.strip_prefix('P').or_else(|| rest.strip_prefix('p'))?;
    let mut total = Duration::zero();
    let mut number = String::new();
    let mut in_time = false;
    for ch in rest.chars() {
        match ch.to_ascii_uppercase() {
            'T' => in_time = true,
            digit if digit.is_ascii_digit() => number.push(digit),
            unit => {
                let amount: i64 = number.parse().ok()?;
                number.clear();
                total += match (unit, in_time) {
                    ('W', false) => Duration::weeks(amount),
                    ('D', false) => Duration::days(amount),
                    ('H', true) => Duration::hours(amount),
                    ('M', true) => Duration::minutes(amount),
                    ('S', true) => Duration::seconds(amount),
                    _ => return None,
                };
            }
        }
    }
    if !number.is_empty() {
        return None;
    }
    Some(if negative { -total } else { total })
}

/// The parts of the first VTODO: its own lines and its VALARM blocks.
struct Todo {
    /// Index of `END:VTODO` in the unfolded lines.
    end: usize,
    /// Top-level property line indexes, alarms excluded.
    properties: Vec<usize>,
    /// `(begin, end)` line indexes of each VALARM, inclusive.
    alarms: Vec<(usize, usize)>,
}

fn locate(lines: &[String]) -> Option<Todo> {
    let begin = lines
        .iter()
        .position(|line| line.eq_ignore_ascii_case("BEGIN:VTODO"))?;
    let mut depth = 0usize;
    let mut properties = Vec::new();
    let mut alarms = Vec::new();
    let mut alarm_start = None;
    for (index, line) in lines.iter().enumerate().skip(begin + 1) {
        let upper = line.to_ascii_uppercase();
        if upper == "END:VTODO" && depth == 0 {
            return Some(Todo {
                end: index,
                properties,
                alarms,
            });
        }
        if upper.starts_with("BEGIN:") {
            if depth == 0 && upper == "BEGIN:VALARM" {
                alarm_start = Some(index);
            }
            depth += 1;
        } else if upper.starts_with("END:") {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                if let Some(start) = alarm_start.take() {
                    alarms.push((start, index));
                }
            }
        } else if depth == 0 {
            properties.push(index);
        }
    }
    None
}

fn property(lines: &[String], todo: &Todo, name: &str) -> Option<Line> {
    todo.properties
        .iter()
        .filter_map(|&index| Line::parse(&lines[index]))
        .find(|line| line.name == name)
}

/// The alarms that stand for "remind me": DISPLAY, or no ACTION at all. AUDIO
/// and EMAIL alarms belong to other clients and are left as they are.
fn display_alarms(lines: &[String], todo: &Todo) -> Vec<(usize, usize)> {
    todo.alarms
        .iter()
        .copied()
        .filter(|&(begin, end)| {
            lines[begin + 1..end]
                .iter()
                .filter_map(|line| Line::parse(line))
                .find(|line| line.name == "ACTION")
                .is_none_or(|line| line.value.trim().eq_ignore_ascii_case("DISPLAY"))
        })
        .collect()
}

/// The reminder the DISPLAY alarms give for this due date and start.
fn reminder_from_alarms(
    lines: &[String],
    todo: &Todo,
    due: Option<&str>,
    start: Option<&str>,
) -> Option<String> {
    for (begin, end) in display_alarms(lines, todo) {
        let Some(trigger) = lines[begin + 1..end]
            .iter()
            .filter_map(|line| Line::parse(line))
            .find(|line| line.name == "TRIGGER")
        else {
            continue;
        };
        let absolute = trigger
            .param("VALUE")
            .is_some_and(|kind| kind.eq_ignore_ascii_case("DATE-TIME"));
        if absolute {
            if let Some(value) = normalize_ics_dt(trigger.value.trim(), None, false) {
                return Some(value);
            }
            continue;
        }
        let Some(offset) = parse_duration(&trigger.value) else {
            continue;
        };
        // RELATED=START counts from DTSTART; a task without one is anchored
        // on its due date, as Thunderbird and Nextcloud do.
        let related_end = trigger
            .param("RELATED")
            .is_some_and(|value| value.eq_ignore_ascii_case("END"));
        let anchor = if related_end { due } else { start.or(due) };
        if let Some(base) = anchor.and_then(due_instant) {
            return Some(format_utc(base + offset));
        }
    }
    None
}

fn format_utc(value: DateTime<Utc>) -> String {
    value.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// Read the first VTODO. `None` when the resource holds none (an event in a
/// mixed collection) or it has no UID.
pub fn parse(raw: &str) -> Option<TaskFields> {
    let lines = unfold(raw);
    let todo = locate(&lines)?;
    let text = |name: &str| {
        property(&lines, &todo, name)
            .map(|line| unescape_text(line.value.trim()))
            .filter(|value| !value.trim().is_empty())
    };
    let uid = text("UID")?;
    let due_at = property(&lines, &todo, "DUE").and_then(|line| date_value(&line));
    let start_at = property(&lines, &todo, "DTSTART").and_then(|line| date_value(&line));
    let status = text("STATUS").map(|value| value.to_ascii_uppercase());
    let completed_at = property(&lines, &todo, "COMPLETED")
        .and_then(|line| normalize_ics_dt(line.value.trim(), line.param("TZID"), false))
        .or_else(|| {
            matches!(status.as_deref(), Some("COMPLETED" | "CANCELLED")).then(|| {
                ["LAST-MODIFIED", "DTSTAMP"]
                    .iter()
                    .find_map(|name| {
                        property(&lines, &todo, name)
                            .and_then(|line| normalize_ics_dt(line.value.trim(), None, false))
                    })
                    // A fixed stand-in: `now` would make the task look changed
                    // on every read.
                    .unwrap_or_else(|| "1970-01-01T00:00:00Z".to_string())
            })
        });
    let priority = text("PRIORITY")
        .and_then(|value| value.trim().parse::<u8>().ok())
        .filter(|value| *value <= 9)
        .unwrap_or(0);
    let overridden = lines
        .iter()
        .filter(|line| line.eq_ignore_ascii_case("BEGIN:VTODO"))
        .count()
        > 1;
    let rrule_extras = overridden
        || todo.properties.iter().any(|&index| {
            Line::parse(&lines[index])
                .is_some_and(|line| matches!(line.name.as_str(), "EXDATE" | "RDATE"))
        });
    Some(TaskFields {
        reminder_at: reminder_from_alarms(&lines, &todo, due_at.as_deref(), start_at.as_deref()),
        due_tz: property(&lines, &todo, "DUE").and_then(|line| due_zone(&line)),
        start_at,
        rrule: property(&lines, &todo, "RRULE")
            .map(|line| line.value.trim().to_string())
            .filter(|value| !value.is_empty()),
        rrule_extras,
        uid,
        title: text("SUMMARY").unwrap_or_default(),
        notes: text("DESCRIPTION"),
        due_at,
        priority,
        completed_at,
        message_id: text(MESSAGE_ID_PROPERTY),
    })
}

fn alarm_lines(reminder: &str, title: &str) -> Vec<String> {
    let Some(trigger) = ics_utc(reminder) else {
        return Vec::new();
    };
    vec![
        "BEGIN:VALARM".to_string(),
        "ACTION:DISPLAY".to_string(),
        format!("DESCRIPTION:{}", escape_text(title)),
        format!("TRIGGER;VALUE=DATE-TIME:{trigger}"),
        "END:VALARM".to_string(),
    ]
}

fn status_lines(fields: &TaskFields) -> Vec<String> {
    match fields.completed_at.as_deref().and_then(ics_utc) {
        Some(completed) => vec![
            "STATUS:COMPLETED".to_string(),
            format!("COMPLETED:{completed}"),
            "PERCENT-COMPLETE:100".to_string(),
        ],
        None => vec!["STATUS:NEEDS-ACTION".to_string()],
    }
}

fn now_stamp() -> String {
    Utc::now().format("%Y%m%dT%H%M%SZ").to_string()
}

/// A new resource for a task that has none on the server yet.
pub fn build(fields: &TaskFields) -> String {
    let stamp = now_stamp();
    let mut lines = vec![
        "BEGIN:VCALENDAR".to_string(),
        "VERSION:2.0".to_string(),
        "PRODID:-//Satchel//Tasks//EN".to_string(),
        "BEGIN:VTODO".to_string(),
        format!("UID:{}", fields.uid),
        format!("DTSTAMP:{stamp}"),
        format!("CREATED:{stamp}"),
        format!("LAST-MODIFIED:{stamp}"),
        format!("SUMMARY:{}", escape_text(&fields.title)),
    ];
    if let Some(notes) = fields.notes.as_deref().filter(|value| !value.is_empty()) {
        lines.push(format!("DESCRIPTION:{}", escape_text(notes)));
    }
    if let Some(start) = consistent_start(fields.start_at.as_deref(), fields.due_at.as_deref())
        .and_then(|start| date_line("DTSTART", &start, &Form::stored(fields.due_tz.as_deref())))
    {
        lines.push(start);
    }
    if let Some(due) = fields
        .due_at
        .as_deref()
        .and_then(|due| date_line("DUE", due, &Form::stored(fields.due_tz.as_deref())))
    {
        lines.push(due);
    }
    if let Some(rrule) = fields.rrule.as_deref() {
        lines.push(format!("RRULE:{rrule}"));
    }
    if fields.priority > 0 {
        lines.push(format!("PRIORITY:{}", fields.priority));
    }
    lines.extend(status_lines(fields));
    if let Some(message_id) = fields.message_id.as_deref() {
        lines.push(format!("{MESSAGE_ID_PROPERTY}:{}", escape_text(message_id)));
    }
    if let Some(reminder) = fields.reminder_at.as_deref() {
        lines.extend(alarm_lines(reminder, &fields.title));
    }
    lines.push("END:VTODO".to_string());
    lines.push("END:VCALENDAR".to_string());
    join(&lines)
}

fn join(lines: &[String]) -> String {
    let mut out = String::new();
    for line in lines {
        out.push_str(&fold(line));
        out.push_str("\r\n");
    }
    out
}

/// Apply `fields` to a stored resource, replacing only what changed.
///
/// Falls back to [`build`] when the stored text holds no VTODO.
pub fn patch(original: &str, fields: &TaskFields) -> String {
    let lines = unfold(original);
    let (Some(todo), Some(before)) = (locate(&lines), parse(original)) else {
        return build(fields);
    };
    let mut drop = vec![false; lines.len()];
    let mut added: Vec<String> = Vec::new();
    let replace = |drop: &mut Vec<bool>,
                   added: &mut Vec<String>,
                   names: &[&str],
                   replacement: Vec<String>| {
        for &index in &todo.properties {
            if let Some(line) = Line::parse(&lines[index]) {
                if names.contains(&line.name.as_str()) {
                    drop[index] = true;
                }
            }
        }
        added.extend(replacement);
    };

    if fields.title != before.title {
        replace(
            &mut drop,
            &mut added,
            &["SUMMARY"],
            vec![format!("SUMMARY:{}", escape_text(&fields.title))],
        );
    }
    if fields.notes != before.notes {
        let line = fields
            .notes
            .as_deref()
            .filter(|value| !value.is_empty())
            .map(|notes| format!("DESCRIPTION:{}", escape_text(notes)));
        replace(
            &mut drop,
            &mut added,
            &["DESCRIPTION"],
            line.into_iter().collect(),
        );
    }
    let due_form = property(&lines, &todo, "DUE")
        .map(|line| Form::of(&line))
        .unwrap_or_else(|| Form::stored(fields.due_tz.as_deref()));
    let start_form = property(&lines, &todo, "DTSTART")
        .map(|line| Form::of(&line))
        .unwrap_or_else(|| due_form.clone());
    if fields.due_at != before.due_at {
        // DUE and DURATION are exclusive.
        replace(
            &mut drop,
            &mut added,
            &["DUE", "DURATION"],
            fields
                .due_at
                .as_deref()
                .and_then(|due| date_line("DUE", due, &due_form))
                .into_iter()
                .collect(),
        );
    }
    if fields.due_at != before.due_at || fields.start_at != before.start_at {
        let start = consistent_start(fields.start_at.as_deref(), fields.due_at.as_deref());
        if start != before.start_at {
            replace(
                &mut drop,
                &mut added,
                &["DTSTART"],
                start
                    .as_deref()
                    .and_then(|start| date_line("DTSTART", start, &start_form))
                    .into_iter()
                    .collect(),
            );
        }
    }
    if fields.rrule != before.rrule {
        let line = fields
            .rrule
            .as_deref()
            .map(|rrule| format!("RRULE:{rrule}"));
        replace(
            &mut drop,
            &mut added,
            &["RRULE"],
            line.into_iter().collect(),
        );
    }
    if fields.priority != before.priority {
        let line = (fields.priority > 0).then(|| format!("PRIORITY:{}", fields.priority));
        replace(
            &mut drop,
            &mut added,
            &["PRIORITY"],
            line.into_iter().collect(),
        );
    }
    if fields.completed_at != before.completed_at {
        replace(
            &mut drop,
            &mut added,
            &["STATUS", "COMPLETED", "PERCENT-COMPLETE"],
            status_lines(fields),
        );
    }
    // A repeating task that moved on starts its next occurrence afresh.
    let advanced = fields.rrule.is_some()
        && fields.completed_at.is_none()
        && before.completed_at.is_none()
        && fields.due_at != before.due_at;
    if advanced {
        let in_progress = todo.properties.iter().any(|&index| {
            Line::parse(&lines[index]).is_some_and(|line| {
                (line.name == "STATUS" && line.value.trim().eq_ignore_ascii_case("IN-PROCESS"))
                    || line.name == "PERCENT-COMPLETE"
            })
        });
        if in_progress {
            replace(
                &mut drop,
                &mut added,
                &["STATUS", "PERCENT-COMPLETE"],
                vec!["STATUS:NEEDS-ACTION".to_string()],
            );
        }
    }
    if fields.message_id != before.message_id {
        let line = fields
            .message_id
            .as_deref()
            .map(|value| format!("{MESSAGE_ID_PROPERTY}:{}", escape_text(value)));
        replace(
            &mut drop,
            &mut added,
            &[MESSAGE_ID_PROPERTY],
            line.into_iter().collect(),
        );
    }
    let changed = !added.is_empty() || drop.iter().any(|value| *value);
    // A relative alarm that still gives the new reminder (the task moved and
    // the reminder with it) stays as it is, offset and RELATED included.
    let start = consistent_start(fields.start_at.as_deref(), fields.due_at.as_deref());
    let reminder_changed = fields.reminder_at != before.reminder_at
        && reminder_from_alarms(&lines, &todo, fields.due_at.as_deref(), start.as_deref())
            != fields.reminder_at;
    if !changed && !reminder_changed {
        return join(&lines);
    }
    replace(
        &mut drop,
        &mut added,
        &["LAST-MODIFIED", "DTSTAMP"],
        vec![
            format!("DTSTAMP:{}", now_stamp()),
            format!("LAST-MODIFIED:{}", now_stamp()),
        ],
    );
    if reminder_changed {
        for (begin, end) in display_alarms(&lines, &todo) {
            for flag in &mut drop[begin..=end] {
                *flag = true;
            }
        }
        if let Some(reminder) = fields.reminder_at.as_deref() {
            added.extend(alarm_lines(reminder, &fields.title));
        }
    }

    let mut out = Vec::with_capacity(lines.len() + added.len());
    for (index, line) in lines.iter().enumerate() {
        if index == todo.end {
            out.append(&mut added);
        }
        if !drop[index] {
            out.push(line.clone());
        }
    }
    join(&out)
}

/// A UTC instant for a date-only due value, for sorting and "due today".
pub fn due_sort_key(due: &str) -> Option<DateTime<Utc>> {
    due_instant(due)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NEXTCLOUD: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Nextcloud Tasks v0.16\r\nBEGIN:VTODO\r\nUID:abc-123\r\nCREATED:20261101T090000Z\r\nLAST-MODIFIED:20261101T090000Z\r\nDTSTAMP:20261101T090000Z\r\nSUMMARY:Send the report\\, finally\r\nDESCRIPTION:Line one\\nLine two\r\nDUE;TZID=Europe/Berlin:20261110T170000\r\nPRIORITY:1\r\nCATEGORIES:Work\r\nRRULE:FREQ=WEEKLY\r\nSTATUS:NEEDS-ACTION\r\nBEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER;RELATED=END:-PT30M\r\nDESCRIPTION:Default\r\nEND:VALARM\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";

    #[test]
    fn reads_fields_timezone_and_relative_alarm() {
        let task = parse(NEXTCLOUD).unwrap();
        assert_eq!(task.uid, "abc-123");
        assert_eq!(task.title, "Send the report, finally");
        assert_eq!(task.notes.as_deref(), Some("Line one\nLine two"));
        // 17:00 in Berlin in November is 16:00 UTC.
        assert_eq!(task.due_at.as_deref(), Some("2026-11-10T16:00:00Z"));
        assert_eq!(task.reminder_at.as_deref(), Some("2026-11-10T15:30:00Z"));
        assert_eq!(task.priority, 1);
        assert_eq!(task.completed_at, None);
    }

    #[test]
    fn reads_date_due_absolute_alarm_and_completion() {
        let raw = "BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:x\r\nSUMMARY:Pay rent\r\nDUE;VALUE=DATE:20261201\r\nSTATUS:COMPLETED\r\nCOMPLETED:20261130T101500Z\r\nBEGIN:VALARM\r\nTRIGGER;VALUE=DATE-TIME:20261130T080000Z\r\nACTION:DISPLAY\r\nEND:VALARM\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";
        let task = parse(raw).unwrap();
        assert_eq!(task.due_at.as_deref(), Some("2026-12-01"));
        assert_eq!(task.reminder_at.as_deref(), Some("2026-11-30T08:00:00Z"));
        assert_eq!(task.completed_at.as_deref(), Some("2026-11-30T10:15:00Z"));
    }

    #[test]
    fn ignores_events_and_resources_without_uid() {
        assert!(parse(
            "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:e\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
        )
        .is_none());
        assert!(parse(
            "BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nSUMMARY:x\r\nEND:VTODO\r\nEND:VCALENDAR\r\n"
        )
        .is_none());
    }

    #[test]
    fn build_round_trips() {
        let fields = TaskFields {
            uid: "new@satchel.local".into(),
            title: "Reply to Anna; with numbers, please".into(),
            notes: Some("Two\nlines".into()),
            due_at: Some("2026-11-12T09:30:00Z".into()),
            reminder_at: Some("2026-11-12T09:00:00Z".into()),
            priority: 5,
            completed_at: None,
            message_id: Some("<m1@example.test>".into()),
            start_at: None,
            due_tz: Some(ZONE_UTC.into()),
            rrule: Some("FREQ=WEEKLY;COUNT=3".into()),
            rrule_extras: false,
        };
        let raw = build(&fields);
        assert!(raw.contains("DUE:20261112T093000Z\r\n"));
        assert!(raw.contains("TRIGGER;VALUE=DATE-TIME:20261112T090000Z\r\n"));
        assert_eq!(parse(&raw).unwrap(), fields);
    }

    #[test]
    fn folds_long_lines_on_character_boundaries() {
        let fields = TaskFields {
            uid: "u".into(),
            title: "Ä".repeat(80),
            ..Default::default()
        };
        let raw = build(&fields);
        assert!(raw.split("\r\n").all(|line| line.len() <= 75));
        assert_eq!(parse(&raw).unwrap().title, "Ä".repeat(80));
    }

    #[test]
    fn completing_keeps_everything_else_verbatim() {
        let mut fields = parse(NEXTCLOUD).unwrap();
        fields.completed_at = Some("2026-11-06T12:00:00Z".into());
        let patched = patch(NEXTCLOUD, &fields);
        assert!(patched.contains("DUE;TZID=Europe/Berlin:20261110T170000\r\n"));
        assert!(patched.contains("TRIGGER;RELATED=END:-PT30M\r\n"));
        assert!(patched.contains("RRULE:FREQ=WEEKLY\r\n"));
        assert!(patched.contains("CATEGORIES:Work\r\n"));
        assert!(patched.contains("STATUS:COMPLETED\r\n"));
        assert!(patched.contains("COMPLETED:20261106T120000Z\r\n"));
        assert!(!patched.contains("NEEDS-ACTION"));
        assert_eq!(patched.matches("LAST-MODIFIED:").count(), 1);
        assert_eq!(parse(&patched).unwrap(), fields);
    }

    #[test]
    fn changing_the_reminder_replaces_the_alarm_only() {
        let mut fields = parse(NEXTCLOUD).unwrap();
        fields.reminder_at = Some("2026-11-10T07:00:00Z".into());
        let patched = patch(NEXTCLOUD, &fields);
        assert_eq!(patched.matches("BEGIN:VALARM").count(), 1);
        assert!(patched.contains("TRIGGER;VALUE=DATE-TIME:20261110T070000Z\r\n"));
        assert!(patched.contains("DUE;TZID=Europe/Berlin:20261110T170000\r\n"));

        fields.reminder_at = None;
        let cleared = patch(NEXTCLOUD, &fields);
        assert!(!cleared.contains("VALARM"));
    }

    #[test]
    fn moving_due_before_start_drops_the_start() {
        let raw = "BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:s\r\nSUMMARY:x\r\nDTSTART;VALUE=DATE:20261120\r\nDUE;VALUE=DATE:20261125\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";
        let mut fields = parse(raw).unwrap();
        fields.due_at = Some("2026-11-10".into());
        let patched = patch(raw, &fields);
        assert!(!patched.contains("DTSTART"));
        assert!(patched.contains("DUE;VALUE=DATE:20261110\r\n"));
    }

    #[test]
    fn an_unchanged_patch_is_the_same_resource() {
        let fields = parse(NEXTCLOUD).unwrap();
        let patched = patch(NEXTCLOUD, &fields);
        assert_eq!(patched, NEXTCLOUD);
    }

    #[test]
    fn reads_the_rule_and_notices_what_makes_it_complex() {
        let task = parse(NEXTCLOUD).unwrap();
        assert_eq!(task.rrule.as_deref(), Some("FREQ=WEEKLY"));
        assert!(!task.rrule_extras);
        let with_exdate = NEXTCLOUD.replace(
            "RRULE:FREQ=WEEKLY\r\n",
            "RRULE:FREQ=WEEKLY\r\nEXDATE;TZID=Europe/Berlin:20261117T170000\r\n",
        );
        assert!(parse(&with_exdate).unwrap().rrule_extras);
        let overridden = NEXTCLOUD.replace(
            "END:VCALENDAR",
            "BEGIN:VTODO\r\nUID:abc-123\r\nRECURRENCE-ID;TZID=Europe/Berlin:20261117T170000\r\nSUMMARY:Moved\r\nEND:VTODO\r\nEND:VCALENDAR",
        );
        assert!(parse(&overridden).unwrap().rrule_extras);
    }

    #[test]
    fn advancing_keeps_the_zone_and_lowers_the_count() {
        let raw = "BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:r\r\nSUMMARY:Bins out\r\nDTSTART;TZID=Europe/Berlin:20261110T070000\r\nDUE;TZID=Europe/Berlin:20261110T080000\r\nRRULE:FREQ=WEEKLY;COUNT=4\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";
        let mut fields = parse(raw).unwrap();
        assert_eq!(fields.start_at.as_deref(), Some("2026-11-10T06:00:00Z"));
        fields.due_at = Some("2026-11-17T07:00:00Z".into());
        fields.start_at = Some("2026-11-17T06:00:00Z".into());
        fields.rrule = Some("FREQ=WEEKLY;COUNT=3".into());
        let patched = patch(raw, &fields);
        assert!(
            patched.contains("DUE;TZID=Europe/Berlin:20261117T080000\r\n"),
            "{patched}"
        );
        assert!(patched.contains("DTSTART;TZID=Europe/Berlin:20261117T070000\r\n"));
        assert!(patched.contains("RRULE:FREQ=WEEKLY;COUNT=3\r\n"));
        assert!(!patched.contains("STATUS:COMPLETED"));
    }

    #[test]
    fn drops_a_start_that_would_disagree_with_the_due_date() {
        // A date-time start next to a date due is not allowed (RFC 5545 §3.8.2.3).
        let raw = "BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:s\r\nSUMMARY:x\r\nDTSTART:20261101T090000Z\r\nDUE:20261125T090000Z\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";
        let mut fields = parse(raw).unwrap();
        fields.due_at = Some("2026-11-26".into());
        let patched = patch(raw, &fields);
        assert!(!patched.contains("DTSTART"), "{patched}");
        assert!(patched.contains("DUE;VALUE=DATE:20261126\r\n"));

        let same_form = "BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:s\r\nSUMMARY:x\r\nDTSTART;VALUE=DATE:20261101\r\nDUE;VALUE=DATE:20261125\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";
        let mut fields = parse(same_form).unwrap();
        fields.due_at = Some("2026-11-28".into());
        assert!(patch(same_form, &fields).contains("DTSTART;VALUE=DATE:20261101\r\n"));
    }

    #[test]
    fn a_new_reminder_leaves_other_clients_alarms_alone() {
        let raw = "BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:a\r\nSUMMARY:x\r\nDUE:20261110T100000Z\r\nBEGIN:VALARM\r\nACTION:EMAIL\r\nATTENDEE:mailto:me@example.test\r\nSUMMARY:x\r\nDESCRIPTION:x\r\nTRIGGER:-P1D\r\nEND:VALARM\r\nBEGIN:VALARM\r\nACTION:DISPLAY\r\nDESCRIPTION:x\r\nTRIGGER:-PT15M\r\nEND:VALARM\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";
        let mut fields = parse(raw).unwrap();
        assert_eq!(fields.reminder_at.as_deref(), Some("2026-11-10T09:45:00Z"));
        fields.reminder_at = Some("2026-11-10T08:00:00Z".into());
        let patched = patch(raw, &fields);
        assert!(patched.contains("ACTION:EMAIL\r\n"));
        assert!(!patched.contains("TRIGGER:-PT15M"));
        assert!(patched.contains("TRIGGER;VALUE=DATE-TIME:20261110T080000Z\r\n"));
    }

    #[test]
    fn a_cancelled_task_without_timestamps_reads_the_same_every_time() {
        let raw = "BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:c\r\nSUMMARY:x\r\nSTATUS:CANCELLED\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";
        let first = parse(raw).unwrap();
        assert_eq!(first.completed_at.as_deref(), Some("1970-01-01T00:00:00Z"));
        assert_eq!(parse(raw).unwrap(), first);
    }

    /// Host-independent: run under `TZ=UTC` and `TZ=Europe/Berlin`.
    #[test]
    fn an_advanced_new_york_task_stays_at_nine_in_new_york() {
        use crate::tasks::recurrence::{advance, classify, Recurrence, Zone};
        let raw = "BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:ny\r\nSUMMARY:Standup notes\r\nDUE;TZID=America/New_York:20261028T090000\r\nRRULE:FREQ=WEEKLY\r\nBEGIN:VALARM\r\nACTION:DISPLAY\r\nDESCRIPTION:x\r\nTRIGGER;RELATED=END:-PT30M\r\nEND:VALARM\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";
        let mut fields = parse(raw).unwrap();
        assert_eq!(fields.due_tz.as_deref(), Some("America/New_York"));
        let Recurrence::Simple(rule) =
            classify(fields.rrule.as_deref(), false, fields.due_at.as_deref())
        else {
            panic!("weekly is simple");
        };
        let next = advance(
            &rule,
            "FREQ=WEEKLY",
            fields.due_at.as_deref().unwrap(),
            None,
            fields.reminder_at.as_deref(),
            Zone::from_stored(fields.due_tz.as_deref()),
        )
        .unwrap();
        fields.due_at = Some(next.due_at);
        fields.reminder_at = next.reminder_at;
        let patched = patch(raw, &fields);
        assert!(
            patched.contains("DUE;TZID=America/New_York:20261104T090000\r\n"),
            "{patched}"
        );
        // The alarm still sits 30 minutes before the (new) due time: untouched.
        assert!(
            patched.contains("TRIGGER;RELATED=END:-PT30M\r\n"),
            "{patched}"
        );
        assert!(!patched.contains("VALUE=DATE-TIME"));
    }

    #[test]
    fn a_floating_due_date_stays_floating_when_it_moves() {
        use crate::tasks::recurrence::{advance, classify, Recurrence, Zone};
        let raw = "BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:f\r\nSUMMARY:Water plants\r\nDTSTART:20261110T080000\r\nDUE:20261110T090000\r\nRRULE:FREQ=WEEKLY\r\nSTATUS:IN-PROCESS\r\nPERCENT-COMPLETE:40\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";
        let mut fields = parse(raw).unwrap();
        assert_eq!(fields.due_tz.as_deref(), Some(ZONE_FLOATING));
        let Recurrence::Simple(rule) =
            classify(fields.rrule.as_deref(), false, fields.due_at.as_deref())
        else {
            panic!("weekly is simple");
        };
        let next = advance(
            &rule,
            "FREQ=WEEKLY",
            fields.due_at.as_deref().unwrap(),
            fields.start_at.as_deref(),
            None,
            Zone::from_stored(fields.due_tz.as_deref()),
        )
        .unwrap();
        fields.due_at = Some(next.due_at);
        fields.start_at = next.start_at;
        let patched = patch(raw, &fields);
        assert!(patched.contains("DUE:20261117T090000\r\n"), "{patched}");
        assert!(patched.contains("DTSTART:20261117T080000\r\n"), "{patched}");
        // The next occurrence starts afresh.
        assert!(patched.contains("STATUS:NEEDS-ACTION\r\n"));
        assert!(!patched.contains("PERCENT-COMPLETE") && !patched.contains("IN-PROCESS"));
    }

    #[test]
    fn parses_durations() {
        assert_eq!(parse_duration("-PT15M"), Some(Duration::minutes(-15)));
        assert_eq!(parse_duration("P1DT2H"), Some(Duration::hours(26)));
        assert_eq!(parse_duration("P2W"), Some(Duration::weeks(2)));
        assert_eq!(parse_duration("PT"), Some(Duration::zero()));
        assert_eq!(parse_duration("P5"), None);
    }
}
