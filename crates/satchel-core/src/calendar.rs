//! Calendar event application service. No window or mail-worker dependencies.
use crate::db::accounts::{AccountError, AccountRepository};
use crate::db::calendar::{
    CalendarError, CalendarEventRecord, CalendarEventWrite, CalendarRepository,
};
use crate::drafts::outgoing::{OutgoingAttachmentInput, OutgoingMessageInput};
use crate::drafts::send_queue::{build_send_action, SendActionPayload};
use crate::sync::queue::{QueuedAction, SyncQueue};
use sqlx::SqlitePool;

mod ics;

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
    #[error("remote event saved but local mirror failed: {0}")]
    RemoteCommittedLocalFailed(#[source] CalendarError),
    #[error("calendar invitation could not be queued: {0}")]
    Invitation(String),
}

#[derive(Debug, Clone)]
pub struct EventInput {
    pub id: Option<String>,
    /// The editor's original revision. Legacy callers may omit this precondition.
    pub expected_updated_at: Option<String>,
    pub account_id: String,
    pub title: String,
    pub description: Option<String>,
    pub location: Option<String>,
    /// YYYY-MM-DD for all-day events; RFC3339 for timed events.
    pub starts_at: String,
    /// Exclusive end in the same representation as starts_at.
    pub ends_at: String,
    pub all_day: bool,
    pub attendees: Vec<String>,
    pub send_invites: bool,
}

struct Invitation {
    pub account_id: String,
    pub attendees: Vec<String>,
    pub title: String,
    pub description: Option<String>,
    pub ics: String,
}

/// Remote writes precede the mirror. A remote error leaves local state alone;
/// after remote success a mirror failure requires a sync before retrying.
/// Local events and their invitation queue entries commit atomically. The
/// existing mail worker owns delivery and retry after the transaction commits.
pub async fn save_event(
    pool: &SqlitePool,
    queue: &SyncQueue,
    input: EventInput,
) -> Result<CalendarEventRecord, SaveError> {
    let starts_at = parse_calendar_time(&input.starts_at, input.all_day)?;
    let ends_at = parse_calendar_time(&input.ends_at, input.all_day)?;
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
    let existing = repository.event_edit_state(&input.account_id, &id).await?;
    let uid = existing
        .as_ref()
        .map(|row| row.event.uid.clone())
        .unwrap_or_else(|| format!("{}@satchel.local", uuid::Uuid::new_v4()));
    let attendees = normalize_attendees(&input.attendees);
    if input.id.is_some() && existing.is_none() {
        return Err(CalendarError::EventNotFound.into());
    }
    let ics = if let Some(existing) = &existing {
        if input
            .expected_updated_at
            .as_deref()
            .is_some_and(|version| version != existing.event.updated_at.as_str())
        {
            return Err(CalendarError::EventChanged.into());
        }
        if !matches!(existing.event.source.as_str(), "local" | "caldav") {
            return Err(SaveError::Validation(
                "backend.errors.calendarEditUnsupported",
            ));
        }
        let original = existing.raw_ics.as_deref().ok_or(SaveError::Validation(
            "backend.errors.calendarEditUnsupported",
        ))?;
        ics::patch_event(
            original,
            &existing.event,
            &input,
            starts_at,
            ends_at,
            &account.email,
        )?
    } else {
        build_request_ics(
            &uid,
            title,
            input.description.as_deref(),
            input.location.as_deref(),
            starts_at,
            ends_at,
            input.all_day,
            &account.email,
            account.display_name.as_deref(),
            &attendees,
        )
    };
    let existing_remote = existing.as_ref().filter(|row| row.event.source == "caldav");
    if existing_remote.is_some_and(|row| {
        row.remote_href
            .as_deref()
            .is_none_or(|value| value.trim().is_empty())
            || row
                .remote_etag
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
    }) {
        return Err(SaveError::Validation(
            "backend.errors.calendarRemoteUnavailable",
        ));
    }
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
    if existing_remote.is_some() && remote.is_none() {
        return Err(SaveError::Validation(
            "backend.errors.calendarRemoteUnavailable",
        ));
    }
    let source = if remote.is_some() { "caldav" } else { "local" };
    let attendees_json = existing
        .as_ref()
        .map(|row| row.event.attendees_json.clone())
        .unwrap_or_else(|| serde_json::to_string(&attendees).expect("string list serializes"));
    let starts_at_text = if input.all_day {
        starts_at.format("%Y-%m-%d").to_string()
    } else {
        starts_at.to_rfc3339()
    };
    let ends_at_text = if input.all_day {
        ends_at.format("%Y-%m-%d").to_string()
    } else {
        ends_at.to_rfc3339()
    };
    let invitation = if remote.is_none() && input.send_invites && !attendees.is_empty() {
        Some(
            build_invitation(Invitation {
                account_id: input.account_id.clone(),
                attendees,
                title: title.to_string(),
                description: input.description.clone(),
                ics: invitation_ics(&ics),
            })
            .map_err(SaveError::Invitation)?,
        )
    } else {
        None
    };
    let mut transaction = pool.begin_with("BEGIN IMMEDIATE").await.map_err(|error| {
        if remote.is_some() {
            SaveError::RemoteCommittedLocalFailed(error.into())
        } else {
            SaveError::Calendar(error.into())
        }
    })?;
    let result: Result<CalendarEventRecord, CalendarError> = async {
        if let Some(existing) = &existing {
            repository
                .require_event_version(&mut transaction, existing)
                .await?;
        }
        let event = repository
            .upsert_event_in_transaction(
                &mut transaction,
                CalendarEventWrite {
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
                },
            )
            .await?;

        if let Some(action) = invitation.as_ref() {
            queue
                .enqueue_in_transaction(&mut transaction, action)
                .await
                .map_err(|error| CalendarError::Persistence(sqlx::Error::Protocol(error)))?;
        }
        Ok(event)
    }
    .await;
    let event = result.map_err(|error| {
        if remote.is_some() {
            SaveError::RemoteCommittedLocalFailed(error)
        } else {
            SaveError::Calendar(error)
        }
    })?;
    transaction.commit().await.map_err(|error| {
        if remote.is_some() {
            SaveError::RemoteCommittedLocalFailed(error.into())
        } else {
            SaveError::Calendar(error.into())
        }
    })?;
    if invitation.is_some() {
        queue.wake_worker();
    }
    Ok(event)
}

/// Delete remotely first; a failed DAV precondition leaves the local event intact.
pub async fn delete_event(pool: &SqlitePool, id: &str) -> Result<(), SaveError> {
    let repository = CalendarRepository::new(pool);
    let remote = repository
        .event_sync_state_by_id(id)
        .await?
        .ok_or(CalendarError::EventNotFound)?;
    if remote.source == "caldav" {
        let href = remote
            .remote_href
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .ok_or(SaveError::Validation(
                "backend.errors.calendarRemoteUnavailable",
            ))?;
        if remote
            .remote_etag
            .as_deref()
            .is_none_or(|value| value.trim().is_empty())
        {
            return Err(SaveError::Validation(
                "backend.errors.calendarRemoteUnavailable",
            ));
        }
        {
            let account = AccountRepository::new(pool)
                .calendar_connection(&remote.account_id)
                .await?;
            crate::providers::dav::delete_calendar_event_remote(
                &account.config,
                href,
                remote.remote_etag.as_deref(),
            )
            .await
            .map_err(|error| {
                tracing::warn!(%error, account_id = %remote.account_id, "CalDAV event deletion failed");
                SaveError::Remote(error)
            })?;
        }
    }
    repository.delete_event(&remote.account_id, id).await?;
    Ok(())
}

fn build_invitation(invitation: Invitation) -> Result<QueuedAction, String> {
    use base64::Engine as _;
    use sha2::{Digest, Sha256};
    let digest = format!(
        "{:x}",
        Sha256::digest(format!("{}\0{}", invitation.account_id, invitation.ics).as_bytes())
    );
    let outgoing = OutgoingMessageInput {
        to: invitation.attendees,
        subject: invitation.title.clone(),
        body: invitation.description.unwrap_or(invitation.title),
        attachments: vec![OutgoingAttachmentInput {
            filename: "invite.ics".into(),
            mime_type: "text/calendar; method=REQUEST; charset=UTF-8".into(),
            content_base64: base64::engine::general_purpose::STANDARD.encode(invitation.ics),
            content_id: None,
            inline: false,
        }],
        ..Default::default()
    };
    let mut action = build_send_action(invitation.account_id, outgoing, None)?;
    let mut payload: SendActionPayload =
        serde_json::from_value(action.payload).map_err(|error| error.to_string())?;
    payload.message_id_header = Some(format!("<calendar-event-{digest}@satchel.local>"));
    action.payload = serde_json::to_value(payload).map_err(|error| error.to_string())?;
    action.id = format!("calendar-event-{digest}");
    action.status = "pending".into();
    Ok(action)
}

// A CalDAV resource has no METHOD; mail invitations explicitly use REQUEST.
fn invitation_ics(raw: &str) -> String {
    let mut lines = raw
        .lines()
        .filter(|line| !line.to_ascii_uppercase().starts_with("METHOD:"))
        .map(str::to_string)
        .collect::<Vec<_>>();
    lines.insert(1, "METHOD:REQUEST".to_string());
    format!("{}\r\n", lines.join("\r\n"))
}

fn parse_calendar_time(
    raw: &str,
    all_day: bool,
) -> Result<chrono::DateTime<chrono::Utc>, SaveError> {
    if all_day {
        // The editor sends a DATE. Legacy mirrors encode that DATE at UTC
        // midnight; take its written date without applying a timezone offset.
        let date = if raw.len() == 10 {
            raw
        } else {
            chrono::DateTime::parse_from_rfc3339(raw)
                .map_err(|_| SaveError::Validation("backend.errors.calendarTimeInvalid"))?;
            raw.get(..10)
                .ok_or(SaveError::Validation("backend.errors.calendarTimeInvalid"))?
        };
        return chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
            .ok()
            .and_then(|date| date.and_hms_opt(0, 0, 0))
            .map(|date| date.and_utc())
            .ok_or(SaveError::Validation("backend.errors.calendarTimeInvalid"));
    }
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
    let cn = organizer_name
        .unwrap_or(organizer_email)
        .replace('^', "^^")
        .replace('"', "^'")
        .replace(['\r', '\n'], "");
    ics.push_str(&format!(
        "ORGANIZER;CN=\"{cn}\":mailto:{organizer_email}\r\n"
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

#[cfg(test)]
mod tests;
