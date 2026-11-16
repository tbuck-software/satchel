//! Free/busy for the event editor and free slots for "insert my
//! availability". Both are queries: nothing here writes to the calendar, and
//! people whose time nobody will tell come back as unknown, never as an error.

use serde::Deserialize;
use sqlx::SqlitePool;

use crate::commands::error::{CommandError, CommandResult};
use satchel_core::calendar::availability::{AvailabilityError, WorkingHours};
use satchel_core::calendar::free_busy::{
    self, Editing, FreeBusyError, FreeBusyReport, FreeSlotsReport, Participants,
};

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/availability/", optional_fields = nullable))]
pub struct FreeBusyInput {
    /// The account the event belongs to; it is asked about the attendees.
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub attendees: Vec<String>,
    /// RFC 3339 bounds, end exclusive.
    pub starts_at: String,
    pub ends_at: String,
    /// IANA zone whole-day events are placed in.
    pub time_zone: String,
    /// Calendar keys that do not count towards the owner's availability.
    #[serde(default)]
    pub excluded_calendars: Vec<String>,
    #[serde(default)]
    pub editing: Option<EditingEventInput>,
}

/// An existing event open in the editor: its own time does not count against it.
#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/availability/", optional_fields = nullable))]
pub struct EditingEventInput {
    pub event_id: String,
    /// The saved slot, RFC 3339. Attendee busy time exactly this slot is the event.
    #[serde(default)]
    pub starts_at: Option<String>,
    #[serde(default)]
    pub ends_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/availability/")
)]
pub struct WorkingHoursInput {
    /// Minutes after local midnight.
    pub start_minute: u32,
    /// Minutes after local midnight, up to 1440.
    pub end_minute: u32,
    /// ISO weekdays, 1 = Monday … 7 = Sunday.
    pub weekdays: Vec<u8>,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/availability/", optional_fields = nullable))]
pub struct FreeSlotsInput {
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub attendees: Vec<String>,
    /// RFC 3339; nothing earlier is offered.
    pub from: String,
    pub working_days: u32,
    pub duration_minutes: u32,
    pub working_hours: WorkingHoursInput,
    pub time_zone: String,
    #[serde(default)]
    pub excluded_calendars: Vec<String>,
    #[serde(default)]
    pub editing: Option<EditingEventInput>,
}

impl From<AvailabilityError> for CommandError {
    fn from(error: AvailabilityError) -> Self {
        Self::new(error.key())
    }
}

impl From<FreeBusyError> for CommandError {
    fn from(error: FreeBusyError) -> Self {
        match error {
            FreeBusyError::Invalid(error) => error.into(),
            FreeBusyError::Persistence(error) => {
                tracing::error!(%error, "free/busy persistence failed");
                Self::new("backend.errors.calendarPersistenceFailed")
            }
        }
    }
}

fn editing(input: Option<&EditingEventInput>) -> CommandResult<Option<Editing<'_>>> {
    let Some(input) = input else {
        return Ok(None);
    };
    Ok(Some(Editing {
        event_id: &input.event_id,
        starts_at: input.starts_at.as_deref().map(instant).transpose()?,
        ends_at: input.ends_at.as_deref().map(instant).transpose()?,
    }))
}

fn instant(raw: &str) -> CommandResult<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(raw)
        .map(|time| time.with_timezone(&chrono::Utc))
        .map_err(|_| AvailabilityError::Range.into())
}

/// Busy time of the owner and of every attendee over a range.
#[tauri::command]
pub async fn query_free_busy(
    pool: tauri::State<'_, SqlitePool>,
    input: FreeBusyInput,
) -> CommandResult<FreeBusyReport> {
    let participants = Participants {
        account_id: input.account_id.as_deref(),
        attendees: &input.attendees,
        excluded_calendars: &input.excluded_calendars,
        editing: editing(input.editing.as_ref())?,
    };
    free_busy::query(
        pool.inner(),
        &participants,
        instant(&input.starts_at)?,
        instant(&input.ends_at)?,
        &input.time_zone,
    )
    .await
    .map_err(Into::into)
}

/// Free windows within working hours over the next working days.
#[tauri::command]
pub async fn find_free_slots(
    pool: tauri::State<'_, SqlitePool>,
    input: FreeSlotsInput,
) -> CommandResult<FreeSlotsReport> {
    let hours = WorkingHours::new(
        input.working_hours.start_minute,
        input.working_hours.end_minute,
        &input.working_hours.weekdays,
    )?;
    let participants = Participants {
        account_id: input.account_id.as_deref(),
        attendees: &input.attendees,
        excluded_calendars: &input.excluded_calendars,
        editing: editing(input.editing.as_ref())?,
    };
    free_busy::free_slots(
        pool.inner(),
        &participants,
        instant(&input.from)?,
        input.working_days,
        input.duration_minutes,
        &hours,
        &input.time_zone,
    )
    .await
    .map_err(Into::into)
}
