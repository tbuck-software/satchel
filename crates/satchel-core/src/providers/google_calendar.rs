//! Read-only Google Calendar sync over the Calendar API.
//!
//! Mirrors every calendar a Gmail account has visible in the Google Calendar
//! sidebar into `calendar_events` (`source = 'google'`). One-way by design:
//! rows are replaced wholesale per sync window, so local edits to Google
//! events are neither possible nor silently lost — the UI treats them as
//! read-only. CalDAV accounts (GroupWise & Co.) are mirrored separately by
//! `providers::dav::sync_groupware_account` into the same table.

use serde::Deserialize;
use sqlx::SqlitePool;

use crate::utils::oauth::{access_token_from_account_config, OAuthProvider};

const API_BASE: &str = "https://www.googleapis.com/calendar/v3";
/// How far back and forward the mirror reaches.
const WINDOW_PAST_DAYS: i64 = 90;
const WINDOW_FUTURE_DAYS: i64 = 365;

#[derive(Debug, Deserialize)]
struct CalendarListPage {
    #[serde(default)]
    items: Vec<CalendarListEntry>,
    #[serde(rename = "nextPageToken")]
    next_page_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CalendarListEntry {
    id: String,
    summary: Option<String>,
    #[serde(rename = "summaryOverride")]
    summary_override: Option<String>,
    #[serde(default)]
    primary: bool,
    /// Mirrors the visibility checkbox in the Google Calendar sidebar.
    #[serde(default)]
    selected: bool,
}

#[derive(Debug, Deserialize)]
struct EventsPage {
    #[serde(default)]
    items: Vec<GoogleEvent>,
    #[serde(rename = "nextPageToken")]
    next_page_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GoogleEvent {
    id: String,
    #[serde(rename = "iCalUID")]
    ical_uid: Option<String>,
    status: Option<String>,
    summary: Option<String>,
    description: Option<String>,
    location: Option<String>,
    start: Option<GoogleEventTime>,
    end: Option<GoogleEventTime>,
    #[serde(default)]
    attendees: Vec<GoogleAttendee>,
}

#[derive(Debug, Deserialize)]
struct GoogleEventTime {
    /// All-day events carry `date` (YYYY-MM-DD), timed ones `dateTime`.
    date: Option<String>,
    #[serde(rename = "dateTime")]
    date_time: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GoogleAttendee {
    email: Option<String>,
}

struct MappedEvent {
    id: String,
    calendar_id: String,
    uid: String,
    title: String,
    description: Option<String>,
    location: Option<String>,
    starts_at: String,
    ends_at: String,
    all_day: bool,
    attendees_json: String,
}

/// Sync every visible Google calendar of one Gmail account. Returns the
/// number of events now mirrored locally.
pub async fn sync_account(pool: &SqlitePool, account_id: &str) -> Result<usize, String> {
    let config = crate::utils::oauth::fresh_account_config_for_provider(
        pool,
        account_id,
        OAuthProvider::Gmail,
    )
    .await
    .map_err(|err| err.encoded())?;
    let token = access_token_from_account_config(OAuthProvider::Gmail, &config)
        .await
        .map_err(|err| err.encoded())?;
    let client = crate::utils::http::client();

    let now = chrono::Utc::now();
    let time_min = (now - chrono::Duration::days(WINDOW_PAST_DAYS)).to_rfc3339();
    let time_max = (now + chrono::Duration::days(WINDOW_FUTURE_DAYS)).to_rfc3339();

    let calendars = select_calendars(fetch_calendar_list(&client, &token).await?);

    let mut events: Vec<MappedEvent> = Vec::new();
    for calendar in &calendars {
        let mut page_token: Option<String> = None;
        loop {
            let page = fetch_events_page(
                &client,
                &token,
                &calendar.id,
                &time_min,
                &time_max,
                page_token.as_deref(),
            )
            .await?;
            for item in page.items {
                if let Some(mapped) = map_event(account_id, &calendar.id, item) {
                    events.push(mapped);
                }
            }
            match page.next_page_token {
                Some(next) if !next.is_empty() => page_token = Some(next),
                _ => break,
            }
        }
    }

    let events = dedupe_same_occurrence(events);
    replace_google_mirror(pool, account_id, &calendars, &events).await?;
    Ok(events.len())
}

/// The calendars worth mirroring: whatever is ticked visible in the Google
/// Calendar sidebar, with the primary calendar first (it wins UID dedupe) and
/// as fallback when nothing is selected at all.
fn select_calendars(entries: Vec<CalendarListEntry>) -> Vec<CalendarListEntry> {
    let mut picked: Vec<CalendarListEntry> = entries
        .into_iter()
        .filter(|entry| entry.selected || entry.primary)
        .collect();
    picked.sort_by_key(|entry| !entry.primary);
    if picked.is_empty() {
        picked.push(CalendarListEntry {
            id: "primary".to_string(),
            summary: Some("Hauptkalender".to_string()),
            summary_override: None,
            primary: true,
            selected: true,
        });
    }
    picked
}

/// The same occurrence can appear in several calendars when an invite lands
/// both in the personal and a shared calendar. Recurring instances share one
/// iCalUID, so their start time is part of the key and must not be discarded.
fn dedupe_same_occurrence(events: Vec<MappedEvent>) -> Vec<MappedEvent> {
    let mut seen = std::collections::HashSet::new();
    events
        .into_iter()
        .filter(|event| seen.insert((event.uid.clone(), event.starts_at.clone())))
        .collect()
}

async fn fetch_calendar_list(
    client: &reqwest::Client,
    token: &str,
) -> Result<Vec<CalendarListEntry>, String> {
    let mut entries = Vec::new();
    let mut page_token: Option<String> = None;
    loop {
        let mut url = reqwest::Url::parse(&format!("{API_BASE}/users/me/calendarList"))
            .map_err(|err| err.to_string())?;
        if let Some(page_token) = page_token.as_deref() {
            url.query_pairs_mut().append_pair("pageToken", page_token);
        }
        let page: CalendarListPage = get_json(client, token, url).await?;
        entries.extend(page.items);
        match page.next_page_token {
            Some(next) if !next.is_empty() => page_token = Some(next),
            _ => break,
        }
    }
    Ok(entries)
}

async fn fetch_events_page(
    client: &reqwest::Client,
    token: &str,
    calendar_id: &str,
    time_min: &str,
    time_max: &str,
    page_token: Option<&str>,
) -> Result<EventsPage, String> {
    let mut url = reqwest::Url::parse(&format!("{API_BASE}/calendars/x/events"))
        .map_err(|err| err.to_string())?;
    // Calendar ids contain `@` and `#`; path_segments_mut percent-encodes them.
    url.path_segments_mut()
        .map_err(|_| "invalid calendar URL".to_string())?
        .pop()
        .pop()
        .push(calendar_id)
        .push("events");
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("singleEvents", "true");
        query.append_pair("maxResults", "250");
        query.append_pair("timeMin", time_min);
        query.append_pair("timeMax", time_max);
        if let Some(page_token) = page_token {
            query.append_pair("pageToken", page_token);
        }
    }
    get_json(client, token, url).await
}

async fn get_json<T: serde::de::DeserializeOwned>(
    client: &reqwest::Client,
    token: &str,
    url: reqwest::Url,
) -> Result<T, String> {
    let response = client
        .get(url)
        .bearer_auth(token)
        .send()
        .await
        .map_err(|err| format!("google calendar request failed: {err}"))?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(explain_api_error(status, &body));
    }
    response
        .json()
        .await
        .map_err(|err| format!("google calendar response parse failed: {err}"))
}

/// These errors surface in the UI, so they explain the fix, not just the code.
fn explain_api_error(status: reqwest::StatusCode, body: &str) -> String {
    if status.as_u16() == 403 && body.contains("accessNotConfigured") {
        let base =
            "Die Google Calendar API ist im Google-Cloud-Projekt dieser App nicht aktiviert.";
        // Google's error body carries the project-specific activation link;
        // pass it through so nobody has to dig the project number out of the
        // client id.
        return match extract_console_url(body) {
            Some(url) => format!("{base} Aktivieren unter: {url}"),
            None => base.to_string(),
        };
    }
    if status.as_u16() == 403 || status.as_u16() == 401 {
        return "Der Kalender-Zugriff fehlt. Bitte das Google-Konto in den Einstellungen neu verbinden."
            .to_string();
    }
    format!("google calendar sync failed with status {status}: {body}")
}

fn extract_console_url(body: &str) -> Option<&str> {
    let start = body.find("https://console.developers.google.com")?;
    let rest = &body[start..];
    let end = rest
        .find(|c: char| c.is_whitespace() || c == '"' || c == '\\')
        .unwrap_or(rest.len());
    Some(&rest[..end])
}

fn map_event(account_id: &str, calendar_id: &str, event: GoogleEvent) -> Option<MappedEvent> {
    if event.status.as_deref() == Some("cancelled") {
        return None;
    }
    let start = event.start.as_ref()?;
    let end = event.end.as_ref();
    let all_day = start.date.is_some();
    let starts_at = normalize_time(start)?;
    let ends_at = end
        .and_then(normalize_time)
        .unwrap_or_else(|| starts_at.clone());

    Some(MappedEvent {
        // Event ids are only unique per calendar, so the calendar is part of
        // the row id.
        id: format!("gcal-{account_id}-{calendar_id}-{}", event.id),
        calendar_id: calendar_id.to_string(),
        uid: event.ical_uid.unwrap_or(event.id),
        title: event
            .summary
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "(ohne Titel)".to_string()),
        description: event.description.filter(|value| !value.trim().is_empty()),
        location: event.location.filter(|value| !value.trim().is_empty()),
        starts_at,
        ends_at,
        all_day,
        attendees_json: serde_json::to_string(
            &event
                .attendees
                .into_iter()
                .filter_map(|attendee| attendee.email)
                .collect::<Vec<_>>(),
        )
        .unwrap_or_else(|_| "[]".to_string()),
    })
}

/// RFC3339 UTC, matching what the ICS pipeline stores.
fn normalize_time(time: &GoogleEventTime) -> Option<String> {
    if let Some(date_time) = time.date_time.as_deref() {
        let parsed = chrono::DateTime::parse_from_rfc3339(date_time).ok()?;
        return Some(
            parsed
                .with_timezone(&chrono::Utc)
                .format("%Y-%m-%dT%H:%M:%SZ")
                .to_string(),
        );
    }
    // All-day boundaries are plain calendar dates; store them as UTC midnight
    // the same way VALUE=DATE from ICS attachments is stored.
    let date = time.date.as_deref()?;
    let parsed = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()?;
    Some(format!("{}T00:00:00Z", parsed.format("%Y-%m-%d")))
}

/// The mirror is authoritative for `source='google'`: replace the account's
/// rows in one transaction so deletions on the Google side disappear locally.
fn calendar_name(calendar: &CalendarListEntry) -> &str {
    calendar
        .summary_override
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            calendar
                .summary
                .as_deref()
                .filter(|value| !value.trim().is_empty())
        })
        .unwrap_or(&calendar.id)
}

async fn replace_google_mirror(
    pool: &SqlitePool,
    account_id: &str,
    calendars: &[CalendarListEntry],
    events: &[MappedEvent],
) -> Result<(), String> {
    use crate::db::calendar::{GoogleCalendarEvent, GoogleCalendarSource};
    let sources = calendars
        .iter()
        .map(|calendar| GoogleCalendarSource {
            remote_id: &calendar.id,
            name: calendar_name(calendar),
        })
        .collect::<Vec<_>>();
    let events = events
        .iter()
        .map(|event| GoogleCalendarEvent {
            id: &event.id,
            uid: &event.uid,
            title: &event.title,
            description: event.description.as_deref(),
            location: event.location.as_deref(),
            starts_at: &event.starts_at,
            ends_at: &event.ends_at,
            all_day: event.all_day,
            attendees_json: &event.attendees_json,
            calendar_id: &event.calendar_id,
        })
        .collect::<Vec<_>>();
    crate::db::calendar::CalendarRepository::new(pool)
        .replace_google_mirror(account_id, &sources, &events)
        .await
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timed(date_time: &str) -> GoogleEventTime {
        GoogleEventTime {
            date: None,
            date_time: Some(date_time.to_string()),
        }
    }

    #[test]
    fn surfaces_the_activation_link_from_googles_error_body() {
        let body = r#"{"error":{"code":403,"message":"Google Calendar API has not been used in project 1050804431145 before or it is disabled. Enable it by visiting https://console.developers.google.com/apis/api/calendar-json.googleapis.com/overview?project=1050804431145 then retry.","status":"PERMISSION_DENIED","details":[{"reason":"accessNotConfigured"}]}}"#;
        let message = explain_api_error(reqwest::StatusCode::FORBIDDEN, body);
        assert!(
            message.contains(
                "https://console.developers.google.com/apis/api/calendar-json.googleapis.com/overview?project=1050804431145"
            ),
            "message should carry the activation link: {message}"
        );
    }

    #[test]
    fn converts_offset_times_to_utc() {
        assert_eq!(
            normalize_time(&timed("2026-07-14T16:00:00+02:00")).as_deref(),
            Some("2026-07-14T14:00:00Z")
        );
    }

    #[test]
    fn stores_all_day_dates_at_utc_midnight() {
        let time = GoogleEventTime {
            date: Some("2026-07-14".to_string()),
            date_time: None,
        };
        assert_eq!(
            normalize_time(&time).as_deref(),
            Some("2026-07-14T00:00:00Z")
        );
    }

    #[test]
    fn drops_cancelled_events_and_events_without_start() {
        let cancelled = GoogleEvent {
            id: "a".into(),
            ical_uid: None,
            status: Some("cancelled".into()),
            summary: Some("Weg".into()),
            description: None,
            location: None,
            start: Some(timed("2026-07-14T16:00:00Z")),
            end: None,
            attendees: vec![],
        };
        assert!(map_event("acc", "primary", cancelled).is_none());

        let startless = GoogleEvent {
            id: "b".into(),
            ical_uid: None,
            status: None,
            summary: None,
            description: None,
            location: None,
            start: None,
            end: None,
            attendees: vec![],
        };
        assert!(map_event("acc", "primary", startless).is_none());
    }

    #[test]
    fn maps_titles_uids_and_attendees() {
        let event = GoogleEvent {
            id: "evt-1".into(),
            ical_uid: Some("uid-1@google.com".into()),
            status: Some("confirmed".into()),
            summary: Some("  Planung  ".into()),
            description: Some("Agenda".into()),
            location: Some("Raum 1".into()),
            start: Some(timed("2026-07-14T16:00:00+02:00")),
            end: Some(timed("2026-07-14T17:00:00+02:00")),
            attendees: vec![
                GoogleAttendee {
                    email: Some("a@example.com".into()),
                },
                GoogleAttendee { email: None },
            ],
        };
        let mapped = map_event("acc-1", "team@group.calendar.google.com", event).expect("maps");
        assert_eq!(mapped.id, "gcal-acc-1-team@group.calendar.google.com-evt-1");
        assert_eq!(mapped.calendar_id, "team@group.calendar.google.com");
        assert_eq!(mapped.uid, "uid-1@google.com");
        assert_eq!(mapped.title, "Planung");
        assert_eq!(mapped.starts_at, "2026-07-14T14:00:00Z");
        assert_eq!(mapped.ends_at, "2026-07-14T15:00:00Z");
        assert!(!mapped.all_day);
        assert_eq!(mapped.attendees_json, r#"["a@example.com"]"#);
    }

    fn entry(id: &str, primary: bool, selected: bool) -> CalendarListEntry {
        CalendarListEntry {
            id: id.into(),
            summary: None,
            summary_override: None,
            primary,
            selected,
        }
    }

    #[test]
    fn mirrors_the_sidebar_selection_with_primary_first() {
        let picked = select_calendars(vec![
            entry("holidays@group.v.calendar.google.com", false, false),
            entry("team@group.calendar.google.com", false, true),
            entry("me@example.com", true, true),
        ]);
        let ids: Vec<&str> = picked.iter().map(|entry| entry.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["me@example.com", "team@group.calendar.google.com"]
        );
    }

    #[test]
    fn falls_back_to_the_primary_calendar_when_nothing_is_selected() {
        let picked = select_calendars(vec![entry("hidden@example.com", false, false)]);
        let ids: Vec<&str> = picked.iter().map(|entry| entry.id.as_str()).collect();
        assert_eq!(ids, vec!["primary"]);
    }

    #[test]
    fn prefers_the_user_overridden_calendar_name() {
        let calendar: CalendarListEntry = serde_json::from_str(
            r#"{"id":"fixture-family@group.calendar.google.com","summary":"Familie","summaryOverride":"Privat"}"#,
        )
        .expect("calendar list entry");
        assert_eq!(calendar_name(&calendar), "Privat");
    }

    #[test]
    fn deduplicates_the_same_invite_across_calendars() {
        let make = |cal: &str| {
            map_event(
                "acc",
                cal,
                GoogleEvent {
                    id: "evt".into(),
                    ical_uid: Some("uid-shared@google.com".into()),
                    status: None,
                    summary: Some("Planung".into()),
                    description: None,
                    location: None,
                    start: Some(timed("2026-07-14T16:00:00Z")),
                    end: Some(timed("2026-07-14T17:00:00Z")),
                    attendees: vec![],
                },
            )
            .expect("maps")
        };
        let deduped = dedupe_same_occurrence(vec![
            make("primary"),
            make("team@group.calendar.google.com"),
        ]);
        assert_eq!(deduped.len(), 1);
        assert_eq!(deduped[0].calendar_id, "primary");
    }

    #[test]
    fn keeps_every_instance_of_a_recurring_event() {
        let make = |id: &str, start: &str, end: &str| {
            map_event(
                "acc",
                "work@group.calendar.google.com",
                GoogleEvent {
                    id: id.into(),
                    ical_uid: Some("shift-series@google.com".into()),
                    status: None,
                    summary: Some("Arbeitsplan".into()),
                    description: None,
                    location: None,
                    start: Some(timed(start)),
                    end: Some(timed(end)),
                    attendees: vec![],
                },
            )
            .expect("maps")
        };
        let deduped = dedupe_same_occurrence(vec![
            make("instance-1", "2026-08-10T08:00:00Z", "2026-08-10T16:00:00Z"),
            make("instance-2", "2026-08-11T08:00:00Z", "2026-08-11T16:00:00Z"),
        ]);
        assert_eq!(deduped.len(), 2);
    }

    #[test]
    fn events_url_percent_encodes_calendar_ids() {
        let mut url = reqwest::Url::parse(&format!("{API_BASE}/calendars/x/events")).unwrap();
        url.path_segments_mut()
            .unwrap()
            .pop()
            .pop()
            .push("team#contacts@group.v.calendar.google.com")
            .push("events");
        assert_eq!(
            url.as_str(),
            "https://www.googleapis.com/calendar/v3/calendars/team%23contacts@group.v.calendar.google.com/events"
        );
    }
}
