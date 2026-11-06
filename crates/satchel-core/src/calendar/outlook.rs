//! Event writes for Outlook calendars: Graph first, then the mirror.

use sqlx::SqlitePool;

use super::{normalize_attendees, EventInput, SaveError};
use crate::db::calendar::{
    outlook_event_write, CalendarError, CalendarEventEditState, CalendarEventRecord,
    CalendarEventSyncState, CalendarRepository,
};
use crate::providers::outlook::calendar::{
    self as graph, EventFields, MirroredEventView, WriteError, WrittenEvent,
};

/// Message key for a write refused because the event moved on in Outlook.
const CHANGED_REMOTELY: &str = "backend.errors.outlookEventChangedRemotely";
const READ_ONLY: &str = "backend.errors.calendarEventReadOnly";

/// Translate a refused write. When the event changed or vanished in Outlook,
/// the account's calendar is synced before answering, so that reopening the
/// event shows what is there now and a retry has a current version to guard.
async fn remote_error(pool: &SqlitePool, account_id: &str, error: WriteError) -> SaveError {
    match error {
        WriteError::NotGranted => {
            SaveError::Validation("backend.errors.outlookCalendarAccessMissing")
        }
        WriteError::Forbidden => SaveError::Validation(READ_ONLY),
        WriteError::Changed | WriteError::Gone => {
            if let Err(error) = graph::sync_account(pool, account_id).await {
                tracing::warn!(%error, account_id, "Outlook calendar resync after a conflict failed");
            }
            SaveError::Validation(CHANGED_REMOTELY)
        }
        WriteError::Remote(error) => {
            tracing::warn!(%error, "Outlook calendar write failed");
            SaveError::Remote(error.to_string())
        }
    }
}

fn fields<'a>(
    input: &'a EventInput,
    title: &'a str,
    starts_at: chrono::DateTime<chrono::Utc>,
    ends_at: chrono::DateTime<chrono::Utc>,
) -> EventFields<'a> {
    EventFields {
        title,
        description: input.description.as_deref(),
        location: input.location.as_deref(),
        starts_at,
        ends_at,
        all_day: input.all_day,
    }
}

/// A new event in the account's default Outlook calendar. Exchange mails the
/// invitations itself, so attendees go along only when invitations are wanted.
pub(super) async fn save_new(
    pool: &SqlitePool,
    input: &EventInput,
    title: &str,
    starts_at: chrono::DateTime<chrono::Utc>,
    ends_at: chrono::DateTime<chrono::Utc>,
    attendees: &[String],
) -> Result<CalendarEventRecord, SaveError> {
    let invite = if input.send_invites { attendees } else { &[] };
    let written = match graph::create_event(
        pool,
        &input.account_id,
        &fields(input, title, starts_at, ends_at),
        invite,
    )
    .await
    {
        Ok(written) => written,
        Err(error) => return Err(remote_error(pool, &input.account_id, error).await),
    };
    mirror(pool, &input.account_id, &written, None).await
}

/// Changes to a mirrored Outlook event. Guests stay as they are: changing them
/// means scheduling messages this form does not offer, as for CalDAV.
pub(super) async fn save_edit(
    pool: &SqlitePool,
    input: &EventInput,
    existing: &CalendarEventEditState,
    title: &str,
    starts_at: chrono::DateTime<chrono::Utc>,
    ends_at: chrono::DateTime<chrono::Utc>,
) -> Result<CalendarEventRecord, SaveError> {
    let event = &existing.event;
    if event.read_only == 1 {
        return Err(SaveError::Validation(READ_ONLY));
    }
    let old_attendees: Vec<String> =
        serde_json::from_str(&event.attendees_json).unwrap_or_default();
    if normalize_attendees(&old_attendees) != normalize_attendees(&input.attendees) {
        return Err(SaveError::Validation(
            "backend.errors.calendarEditUnsupported",
        ));
    }
    let remote_id = existing
        .remote_href
        .as_deref()
        .filter(|value| !value.trim().is_empty());
    let calendar_id = event
        .collection_href
        .as_deref()
        .filter(|value| !value.trim().is_empty());
    let (Some(remote_id), Some(calendar_id)) = (remote_id, calendar_id) else {
        return Err(SaveError::Validation(
            "backend.errors.calendarRemoteUnavailable",
        ));
    };
    let view = MirroredEventView {
        title: &event.title,
        description: event.description.as_deref(),
        location: event.location.as_deref(),
        starts_at: &event.starts_at,
        ends_at: &event.ends_at,
        all_day: event.all_day == 1,
        rich_description: event.rich_description == 1,
    };
    let fields = fields(input, title, starts_at, ends_at);
    if graph::changes_rich_description(&view, &fields) {
        return Err(SaveError::Validation(
            "backend.errors.calendarRichDescriptionReadOnly",
        ));
    }
    let patch = graph::patch_body(&view, &fields);
    if patch.as_object().is_some_and(|fields| fields.is_empty()) {
        return Ok(event.clone());
    }
    let written = match graph::update_event(
        pool,
        &input.account_id,
        calendar_id,
        remote_id,
        existing.remote_etag.as_deref(),
        &patch,
    )
    .await
    {
        Ok(written) => written,
        Err(error) => return Err(remote_error(pool, &input.account_id, error).await),
    };
    mirror(pool, &input.account_id, &written, Some(existing)).await
}

/// Store what Graph returned. After Graph accepted the write, a local failure
/// asks for a sync rather than a retry that would apply it twice.
async fn mirror(
    pool: &SqlitePool,
    account_id: &str,
    written: &WrittenEvent,
    expected: Option<&CalendarEventEditState>,
) -> Result<CalendarEventRecord, SaveError> {
    let repository = CalendarRepository::new(pool);
    let result: Result<CalendarEventRecord, CalendarError> = async {
        let mut transaction = pool.begin_with("BEGIN IMMEDIATE").await?;
        if let Some(expected) = expected {
            repository
                .require_event_version(&mut transaction, expected)
                .await?;
        }
        let row = written.event.as_row();
        let event = repository
            .upsert_event_in_transaction(
                &mut transaction,
                outlook_event_write(account_id, &written.calendar_id, &row),
            )
            .await?;
        transaction.commit().await?;
        Ok(event)
    }
    .await;
    result.map_err(SaveError::RemoteCommittedLocalFailed)
}

/// Delete in Outlook first; the caller removes the mirrored row afterwards.
pub(super) async fn delete(
    pool: &SqlitePool,
    remote: &CalendarEventSyncState,
) -> Result<(), SaveError> {
    if remote.read_only == 1 {
        return Err(SaveError::Validation(READ_ONLY));
    }
    let remote_id = remote
        .remote_href
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or(SaveError::Validation(
            "backend.errors.calendarRemoteUnavailable",
        ))?;
    match graph::delete_event(
        pool,
        &remote.account_id,
        remote_id,
        remote.remote_etag.as_deref(),
    )
    .await
    {
        Ok(()) => Ok(()),
        Err(error) => Err(remote_error(pool, &remote.account_id, error).await),
    }
}
