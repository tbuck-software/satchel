//! Calendar event application service. No window or mail-worker dependencies.
use crate::db::accounts::{AccountError, AccountRepository};
use crate::db::calendar::{
    CalendarError, CalendarEventRecord, CalendarEventWrite, CalendarRepository,
};
use sqlx::SqlitePool;

#[derive(Debug, thiserror::Error)]
pub enum SaveError {
    #[error("{0}")]
    Validation(&'static str),
    #[error(transparent)]
    Account(#[from] AccountError),
    #[error(transparent)]
    Calendar(#[from] CalendarError),
    #[error("CalDAV write failed: {0}")]
    Remote(String),
}

#[derive(Debug, Clone)]
pub struct EventInput {
    pub id: Option<String>,
    pub account_id: String,
    pub title: String,
    pub description: Option<String>,
    pub location: Option<String>,
    pub starts_at: String,
    pub ends_at: String,
    pub all_day: bool,
    pub attendees: Vec<String>,
    pub send_invites: bool,
}

pub struct Invitation {
    pub account_id: String,
    pub attendees: Vec<String>,
    pub title: String,
    pub description: Option<String>,
    pub ics: String,
}

pub struct SavedEvent {
    pub event: CalendarEventRecord,
    pub invitation: Option<Invitation>,
}

/// DAV succeeds before the local mirror changes. Mail delivery is a separate
/// durable queue operation owned by the application shell.
pub async fn save_event(pool: &SqlitePool, input: EventInput) -> Result<SavedEvent, SaveError> {
    let starts_at = parse_calendar_time(&input.starts_at, "starts_at")?;
    let ends_at = parse_calendar_time(&input.ends_at, "ends_at")?;
    if ends_at <= starts_at {
        return Err(SaveError::Validation(
            "backend.errors.calendarEventEndInvalid",
        ));
    }
    let title = input.title.trim();
    if title.is_empty() {
        return Err(SaveError::Validation(
            "backend.errors.calendarTitleRequired",
        ));
    }

    let account = AccountRepository::new(pool)
        .calendar_connection(&input.account_id)
        .await?;

    let id = input
        .id
        .clone()
        .unwrap_or_else(|| format!("cal-{}", uuid::Uuid::new_v4()));
    let repository = CalendarRepository::new(pool);
    let existing = repository.event_sync_state(&input.account_id, &id).await?;
    let uid = existing
        .as_ref()
        .map(|row| row.uid.clone())
        .unwrap_or_else(|| format!("{}@satchel.local", uuid::Uuid::new_v4()));
    let attendees = normalize_attendees(&input.attendees);
    let server_attendees: &[String] = if input.send_invites { &attendees } else { &[] };
    let ics = build_request_ics(
        &uid,
        title,
        input.description.as_deref(),
        input.location.as_deref(),
        starts_at,
        ends_at,
        input.all_day,
        &account.email,
        account.display_name.as_deref(),
        server_attendees,
    );
    let existing_remote = existing.as_ref().filter(|row| row.source == "caldav");
    let remote = crate::providers::dav::put_calendar_event(
        &account.config,
        &uid,
        &ics,
        existing_remote.and_then(|row| row.remote_href.as_deref()),
        existing_remote.and_then(|row| row.remote_etag.as_deref()),
    )
    .await
    .map_err(|error| {
        tracing::warn!(%error, account_id = %input.account_id, "CalDAV event write failed");
        SaveError::Remote(error)
    })?;
    let source = if remote.is_some() { "caldav" } else { "local" };
    let stored_attendees = if remote.is_some() {
        server_attendees
    } else {
        attendees.as_slice()
    };
    let attendees_json =
        serde_json::to_string(stored_attendees).unwrap_or_else(|_| "[]".to_string());

    let starts_at_text = starts_at.to_rfc3339();
    let ends_at_text = ends_at.to_rfc3339();
    let event = repository
        .upsert_event(CalendarEventWrite {
            id: &id,
            account_id: &input.account_id,
            uid: &uid,
            title,
            description: input.description.as_deref(),
            location: input.location.as_deref(),
            starts_at: &starts_at_text,
            ends_at: &ends_at_text,
            all_day: input.all_day,
            attendees_json: &attendees_json,
            source,
            remote_href: remote.as_ref().map(|value| value.href.as_str()),
            remote_etag: remote.as_ref().and_then(|value| value.etag.as_deref()),
            collection_href: remote.as_ref().map(|value| value.collection_href.as_str()),
            raw_ics: Some(&ics),
        })
        .await?;

    let invitation =
        (remote.is_none() && input.send_invites && !attendees.is_empty()).then(|| Invitation {
            account_id: input.account_id.clone(),
            attendees,
            title: title.to_string(),
            description: input.description.clone(),
            ics,
        });
    Ok(SavedEvent { event, invitation })
}

fn parse_calendar_time(
    raw: &str,
    _field: &str,
) -> Result<chrono::DateTime<chrono::Utc>, SaveError> {
    chrono::DateTime::parse_from_rfc3339(raw)
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .map_err(|_| SaveError::Validation("backend.errors.calendarTimeInvalid"))
}

fn normalize_attendees(input: &[String]) -> Vec<String> {
    let mut out = input
        .iter()
        .map(|value| value.trim().trim_matches(['<', '>']).to_ascii_lowercase())
        .filter(|value| value.contains('@'))
        .collect::<Vec<_>>();
    out.sort();
    out.dedup();
    out
}

fn build_request_ics(
    uid: &str,
    title: &str,
    description: Option<&str>,
    location: Option<&str>,
    starts_at: chrono::DateTime<chrono::Utc>,
    ends_at: chrono::DateTime<chrono::Utc>,
    all_day: bool,
    organizer_email: &str,
    organizer_name: Option<&str>,
    attendees: &[String],
) -> String {
    let dtstamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
    let mut ics = String::new();
    ics.push_str("BEGIN:VCALENDAR\r\n");
    ics.push_str("PRODID:-//Satchel//EN\r\n");
    ics.push_str("VERSION:2.0\r\n");
    ics.push_str("METHOD:REQUEST\r\n");
    ics.push_str("BEGIN:VEVENT\r\n");
    ics.push_str(&format!("UID:{}\r\n", escape_ics(uid)));
    ics.push_str(&format!("DTSTAMP:{dtstamp}\r\n"));
    if all_day {
        ics.push_str(&format!(
            "DTSTART;VALUE=DATE:{}\r\n",
            starts_at.format("%Y%m%d")
        ));
        ics.push_str(&format!(
            "DTEND;VALUE=DATE:{}\r\n",
            ends_at.format("%Y%m%d")
        ));
    } else {
        ics.push_str(&format!(
            "DTSTART:{}\r\n",
            starts_at.format("%Y%m%dT%H%M%SZ")
        ));
        ics.push_str(&format!("DTEND:{}\r\n", ends_at.format("%Y%m%dT%H%M%SZ")));
    }
    ics.push_str(&format!("SUMMARY:{}\r\n", escape_ics(title)));
    if let Some(description) = description.filter(|value| !value.trim().is_empty()) {
        ics.push_str(&format!("DESCRIPTION:{}\r\n", escape_ics(description)));
    }
    if let Some(location) = location.filter(|value| !value.trim().is_empty()) {
        ics.push_str(&format!("LOCATION:{}\r\n", escape_ics(location)));
    }
    let cn = organizer_name.unwrap_or(organizer_email);
    ics.push_str(&format!(
        "ORGANIZER;CN={}:mailto:{}\r\n",
        escape_ics(cn),
        organizer_email
    ));
    for attendee in attendees {
        ics.push_str(&format!(
            "ATTENDEE;ROLE=REQ-PARTICIPANT;PARTSTAT=NEEDS-ACTION;RSVP=TRUE:mailto:{}\r\n",
            attendee
        ));
    }
    ics.push_str("END:VEVENT\r\n");
    ics.push_str("END:VCALENDAR\r\n");
    ics
}

fn escape_ics(input: &str) -> String {
    input
        .replace('\\', "\\\\")
        .replace(';', "\\;")
        .replace(',', "\\,")
        .replace('\n', "\\n")
        .replace('\r', "")
}
