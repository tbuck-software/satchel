//! Tauri-free persistence for calendar invitations, events and remote mirrors.

use serde::Serialize;
use sqlx::{FromRow, SqlitePool};
use thiserror::Error;

use crate::email::calendar::ParsedInvite;

const INVITE_SELECT: &str =
    "SELECT id, message_id, account_id, uid, organizer_email, organizer_name, summary,
            description, location, starts_at, ends_at, all_day, rsvp_status, raw_ics,
            sequence, method, cancelled, NULLIF(recurrence_id, '') AS recurrence_id,
            recurring, recurrence_unbounded, created_at, updated_at
     FROM calendar_invites";

const EVENT_SELECT: &str =
    "SELECT id, account_id, uid, title, description, location, starts_at, ends_at,
            all_day, attendees_json, source, collection_href, created_at, updated_at
     FROM calendar_events";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, FromRow)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "core/db/calendar/"))]
pub struct CalendarInviteRecord {
    pub id: String,
    pub message_id: Option<String>,
    pub account_id: String,
    pub uid: String,
    pub organizer_email: Option<String>,
    pub organizer_name: Option<String>,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub location: Option<String>,
    pub starts_at: Option<String>,
    pub ends_at: Option<String>,
    pub all_day: i64,
    pub rsvp_status: String,
    pub raw_ics: String,
    pub sequence: Option<i64>,
    pub method: Option<String>,
    pub cancelled: i64,
    pub recurrence_id: Option<String>,
    pub recurring: i64,
    pub recurrence_unbounded: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, FromRow)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "core/db/calendar/"))]
pub struct CalendarEventRecord {
    pub id: String,
    pub account_id: String,
    pub uid: String,
    pub title: String,
    pub description: Option<String>,
    pub location: Option<String>,
    pub starts_at: String,
    pub ends_at: String,
    pub all_day: i64,
    pub attendees_json: String,
    pub source: String,
    pub collection_href: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, FromRow)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "core/db/calendar/"))]
pub struct CalendarSourceRecord {
    pub account_id: String,
    pub source: String,
    pub remote_id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct CalendarEventSyncState {
    pub account_id: String,
    pub uid: String,
    pub source: String,
    pub remote_href: Option<String>,
    pub remote_etag: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
pub struct CalendarEventEditState {
    #[sqlx(flatten)]
    pub event: CalendarEventRecord,
    pub raw_ics: Option<String>,
    pub remote_href: Option<String>,
    pub remote_etag: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CalendarEventWrite<'a> {
    pub id: &'a str,
    pub account_id: &'a str,
    pub uid: &'a str,
    pub title: &'a str,
    pub description: Option<&'a str>,
    pub location: Option<&'a str>,
    pub starts_at: &'a str,
    pub ends_at: &'a str,
    pub all_day: bool,
    pub attendees_json: &'a str,
    pub source: &'a str,
    pub remote_href: Option<&'a str>,
    pub remote_etag: Option<&'a str>,
    pub collection_href: Option<&'a str>,
    pub raw_ics: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub struct GoogleCalendarSource<'a> {
    pub remote_id: &'a str,
    pub name: &'a str,
}

#[derive(Debug, Clone)]
pub struct GoogleCalendarEvent<'a> {
    pub id: &'a str,
    pub uid: &'a str,
    pub title: &'a str,
    pub description: Option<&'a str>,
    pub location: Option<&'a str>,
    pub starts_at: &'a str,
    pub ends_at: &'a str,
    pub all_day: bool,
    pub attendees_json: &'a str,
    pub calendar_id: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct UpcomingCalendarRecord {
    pub title: String,
    pub starts_at: String,
    pub ends_at: String,
    pub all_day: bool,
    pub location: Option<String>,
    pub raw_ics: Option<String>,
}

#[derive(Debug, Error)]
pub enum CalendarError {
    #[error("calendar invitation was not found")]
    InviteNotFound,
    #[error("calendar event belongs to another account")]
    EventOwnership,
    #[error("calendar event was not found")]
    EventNotFound,
    #[error("calendar event changed during editing")]
    EventChanged,
    #[error("calendar persistence failed")]
    Persistence(#[source] sqlx::Error),
}

impl From<sqlx::Error> for CalendarError {
    fn from(error: sqlx::Error) -> Self {
        Self::Persistence(error)
    }
}

/// Owns calendar persistence. The calendar application service coordinates
/// ICS editing, provider calls and invitation queue transactions.
pub struct CalendarRepository<'a> {
    pool: &'a SqlitePool,
}

impl<'a> CalendarRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn list_invites(
        &self,
        account_id: Option<&str>,
    ) -> Result<Vec<CalendarInviteRecord>, CalendarError> {
        match account_id {
            Some(account_id) => sqlx::query_as::<_, CalendarInviteRecord>(&format!(
                "{INVITE_SELECT} WHERE account_id = ? ORDER BY starts_at ASC"
            ))
            .bind(account_id)
            .fetch_all(self.pool)
            .await
            .map_err(Into::into),
            None => sqlx::query_as::<_, CalendarInviteRecord>(&format!(
                "{INVITE_SELECT} ORDER BY starts_at ASC"
            ))
            .fetch_all(self.pool)
            .await
            .map_err(Into::into),
        }
    }

    pub async fn list_events(
        &self,
        account_id: Option<&str>,
    ) -> Result<Vec<CalendarEventRecord>, CalendarError> {
        match account_id {
            Some(account_id) => sqlx::query_as::<_, CalendarEventRecord>(&format!(
                "{EVENT_SELECT} WHERE account_id = ? ORDER BY starts_at ASC"
            ))
            .bind(account_id)
            .fetch_all(self.pool)
            .await
            .map_err(Into::into),
            None => sqlx::query_as::<_, CalendarEventRecord>(&format!(
                "{EVENT_SELECT} ORDER BY starts_at ASC"
            ))
            .fetch_all(self.pool)
            .await
            .map_err(Into::into),
        }
    }

    pub async fn list_sources(
        &self,
        account_id: Option<&str>,
    ) -> Result<Vec<CalendarSourceRecord>, CalendarError> {
        let select = "SELECT account_id, source, remote_id, name FROM calendar_sources";
        match account_id {
            Some(account_id) => sqlx::query_as::<_, CalendarSourceRecord>(&format!(
                "{select} WHERE account_id = ? ORDER BY name COLLATE NOCASE ASC"
            ))
            .bind(account_id)
            .fetch_all(self.pool)
            .await
            .map_err(Into::into),
            None => sqlx::query_as::<_, CalendarSourceRecord>(&format!(
                "{select} ORDER BY name COLLATE NOCASE ASC"
            ))
            .fetch_all(self.pool)
            .await
            .map_err(Into::into),
        }
    }

    pub async fn get_invite(&self, id: &str) -> Result<CalendarInviteRecord, CalendarError> {
        sqlx::query_as::<_, CalendarInviteRecord>(&format!("{INVITE_SELECT} WHERE id = ?"))
            .bind(id)
            .fetch_optional(self.pool)
            .await?
            .ok_or(CalendarError::InviteNotFound)
    }

    pub async fn find_invite(
        &self,
        account_id: &str,
        uid: &str,
        recurrence_id: Option<&str>,
    ) -> Result<CalendarInviteRecord, CalendarError> {
        sqlx::query_as::<_, CalendarInviteRecord>(&format!(
            "{INVITE_SELECT}
             WHERE account_id = ? AND uid = ? AND recurrence_id = ? LIMIT 1"
        ))
        .bind(account_id)
        .bind(uid)
        .bind(recurrence_id.unwrap_or(""))
        .fetch_optional(self.pool)
        .await?
        .ok_or(CalendarError::InviteNotFound)
    }

    pub async fn upsert_invite(
        &self,
        account_id: &str,
        message_id: Option<&str>,
        invite: &ParsedInvite,
    ) -> Result<CalendarInviteRecord, CalendarError> {
        let mut connection = self.pool.acquire().await?;
        Self::upsert_invite_on_connection(&mut connection, account_id, message_id, invite).await?;
        drop(connection);
        self.find_invite(account_id, &invite.uid, invite.recurrence_id.as_deref())
            .await
    }

    /// Publish a parsed mail attachment inside the caller's identity guard.
    pub async fn upsert_invite_on_connection(
        connection: &mut sqlx::SqliteConnection,
        account_id: &str,
        message_id: Option<&str>,
        invite: &ParsedInvite,
    ) -> Result<(), CalendarError> {
        let now = chrono::Utc::now().to_rfc3339();
        let recurrence_id = invite.recurrence_id.as_deref().unwrap_or("");
        let id = invite_id(account_id, &invite.uid, recurrence_id);
        sqlx::query(
            "INSERT INTO calendar_invites
                (id, message_id, account_id, uid, organizer_email, organizer_name, summary,
                 description, location, starts_at, ends_at, all_day, rsvp_status, raw_ics,
                 sequence, method, cancelled, recurrence_id, recurring, recurrence_unbounded,
                 created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'needs-action', ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(account_id, uid, recurrence_id) DO UPDATE SET
                message_id = excluded.message_id,
                organizer_email = excluded.organizer_email,
                organizer_name = excluded.organizer_name,
                summary = excluded.summary,
                description = excluded.description,
                location = excluded.location,
                starts_at = excluded.starts_at,
                ends_at = excluded.ends_at,
                all_day = excluded.all_day,
                raw_ics = excluded.raw_ics,
                sequence = excluded.sequence,
                method = excluded.method,
                recurring = excluded.recurring,
                recurrence_unbounded = excluded.recurrence_unbounded,
                cancelled = MAX(calendar_invites.cancelled, excluded.cancelled),
                updated_at = excluded.updated_at",
        )
        .bind(id)
        .bind(message_id)
        .bind(account_id)
        .bind(&invite.uid)
        .bind(invite.organizer_email.as_deref())
        .bind(invite.organizer_name.as_deref())
        .bind(invite.summary.as_deref())
        .bind(invite.description.as_deref())
        .bind(invite.location.as_deref())
        .bind(invite.starts_at.as_deref())
        .bind(invite.ends_at.as_deref())
        .bind(if invite.all_day { 1_i64 } else { 0_i64 })
        .bind(&invite.raw_ics)
        .bind(invite.sequence)
        .bind(invite.method.as_deref())
        .bind(if invite.cancelled { 1_i64 } else { 0_i64 })
        .bind(recurrence_id)
        .bind(if invite.recurring { 1_i64 } else { 0_i64 })
        .bind(if invite.recurrence_unbounded {
            1_i64
        } else {
            0_i64
        })
        .bind(&now)
        .bind(&now)
        .execute(connection)
        .await?;
        Ok(())
    }

    /// Keep the resource body, form projection and DAV precondition together.
    pub async fn event_edit_state(
        &self,
        account_id: &str,
        id: &str,
    ) -> Result<Option<CalendarEventEditState>, CalendarError> {
        let state = sqlx::query_as::<_, CalendarEventEditState>(
            "SELECT * FROM calendar_events WHERE id = ? LIMIT 1",
        )
        .bind(id)
        .fetch_optional(self.pool)
        .await?;
        match state {
            Some(state) if state.event.account_id != account_id => {
                Err(CalendarError::EventOwnership)
            }
            state => Ok(state),
        }
    }

    /// Recheck the snapshot after provider I/O, while holding the writer lock.
    pub async fn require_event_version(
        &self,
        transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        expected: &CalendarEventEditState,
    ) -> Result<(), CalendarError> {
        let unchanged: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM calendar_events WHERE id = ? AND account_id = ?
             AND updated_at = ? AND raw_ics IS ? AND remote_etag IS ?)",
        )
        .bind(&expected.event.id)
        .bind(&expected.event.account_id)
        .bind(&expected.event.updated_at)
        .bind(expected.raw_ics.as_deref())
        .bind(expected.remote_etag.as_deref())
        .fetch_one(&mut **transaction)
        .await?;
        if unchanged {
            Ok(())
        } else {
            Err(CalendarError::EventChanged)
        }
    }

    pub async fn event_sync_state(
        &self,
        account_id: &str,
        id: &str,
    ) -> Result<Option<CalendarEventSyncState>, CalendarError> {
        let state = sqlx::query_as::<_, CalendarEventSyncState>(
            "SELECT account_id, uid, source, remote_href, remote_etag
             FROM calendar_events WHERE id = ? LIMIT 1",
        )
        .bind(id)
        .fetch_optional(self.pool)
        .await?;
        match state {
            Some(state) if state.account_id != account_id => Err(CalendarError::EventOwnership),
            state => Ok(state),
        }
    }

    pub async fn event_sync_state_by_id(
        &self,
        id: &str,
    ) -> Result<Option<CalendarEventSyncState>, CalendarError> {
        sqlx::query_as::<_, CalendarEventSyncState>(
            "SELECT account_id, uid, source, remote_href, remote_etag
             FROM calendar_events WHERE id = ? LIMIT 1",
        )
        .bind(id)
        .fetch_optional(self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn upsert_event(
        &self,
        event: CalendarEventWrite<'_>,
    ) -> Result<CalendarEventRecord, CalendarError> {
        let mut connection = self.pool.acquire().await?;
        Self::write_event(&mut connection, event).await
    }

    pub async fn upsert_event_in_transaction(
        &self,
        transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        event: CalendarEventWrite<'_>,
    ) -> Result<CalendarEventRecord, CalendarError> {
        Self::write_event(transaction, event).await
    }

    async fn write_event(
        connection: &mut sqlx::SqliteConnection,
        event: CalendarEventWrite<'_>,
    ) -> Result<CalendarEventRecord, CalendarError> {
        // Providers may still supply legacy UTC-midnight mirrors of DATE
        // values. Normalize every writer, so a sync cannot reverse an edit's
        // representation and invalidate the editor revision without a change.
        let starts_at = if event.all_day {
            event.starts_at.get(..10).unwrap_or(event.starts_at)
        } else {
            event.starts_at
        };
        let ends_at = if event.all_day {
            event.ends_at.get(..10).unwrap_or(event.ends_at)
        } else {
            event.ends_at
        };
        let now = chrono::Utc::now().to_rfc3339();
        let result = sqlx::query(
            "INSERT INTO calendar_events
                (id, account_id, uid, title, description, location, starts_at, ends_at, all_day,
                 attendees_json, source, remote_href, remote_etag, collection_href, raw_ics,
                 created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
                uid = excluded.uid,
                title = excluded.title,
                description = excluded.description,
                location = excluded.location,
                starts_at = excluded.starts_at,
                ends_at = excluded.ends_at,
                all_day = excluded.all_day,
                attendees_json = excluded.attendees_json,
                source = excluded.source,
                remote_href = excluded.remote_href,
                remote_etag = excluded.remote_etag,
                collection_href = excluded.collection_href,
                raw_ics = excluded.raw_ics,
                updated_at = CASE WHEN
                    (calendar_events.uid, calendar_events.title, calendar_events.description,
                     calendar_events.location, calendar_events.starts_at, calendar_events.ends_at,
                     calendar_events.all_day, calendar_events.attendees_json, calendar_events.source,
                     calendar_events.remote_href, calendar_events.remote_etag,
                     calendar_events.collection_href, calendar_events.raw_ics)
                    IS (excluded.uid, excluded.title, excluded.description, excluded.location,
                        excluded.starts_at, excluded.ends_at, excluded.all_day,
                        excluded.attendees_json, excluded.source, excluded.remote_href,
                        excluded.remote_etag, excluded.collection_href, excluded.raw_ics)
                    THEN calendar_events.updated_at ELSE excluded.updated_at END
             WHERE calendar_events.account_id = excluded.account_id",
        )
        .bind(event.id)
        .bind(event.account_id)
        .bind(event.uid)
        .bind(event.title)
        .bind(event.description)
        .bind(event.location)
        .bind(starts_at)
        .bind(ends_at)
        .bind(if event.all_day { 1_i64 } else { 0_i64 })
        .bind(event.attendees_json)
        .bind(event.source)
        .bind(event.remote_href)
        .bind(event.remote_etag)
        .bind(event.collection_href)
        .bind(event.raw_ics)
        .bind(&now)
        .bind(&now)
        .execute(&mut *connection)
        .await?;
        if result.rows_affected() == 0 {
            return Err(CalendarError::EventOwnership);
        }
        sqlx::query_as::<_, CalendarEventRecord>(&format!(
            "{EVENT_SELECT} WHERE account_id = ? AND id = ?"
        ))
        .bind(event.account_id)
        .bind(event.id)
        .fetch_optional(&mut *connection)
        .await?
        .ok_or(CalendarError::EventNotFound)
    }

    pub async fn get_event(
        &self,
        account_id: &str,
        id: &str,
    ) -> Result<CalendarEventRecord, CalendarError> {
        sqlx::query_as::<_, CalendarEventRecord>(&format!(
            "{EVENT_SELECT} WHERE account_id = ? AND id = ?"
        ))
        .bind(account_id)
        .bind(id)
        .fetch_optional(self.pool)
        .await?
        .ok_or(CalendarError::EventNotFound)
    }

    pub async fn delete_event(&self, account_id: &str, id: &str) -> Result<(), CalendarError> {
        let result = sqlx::query("DELETE FROM calendar_events WHERE account_id = ? AND id = ?")
            .bind(account_id)
            .bind(id)
            .execute(self.pool)
            .await?;
        if result.rows_affected() == 0 {
            return Err(CalendarError::EventNotFound);
        }
        Ok(())
    }

    pub async fn update_rsvp_status(
        &self,
        invite_id: &str,
        status: &str,
    ) -> Result<(), CalendarError> {
        let result =
            sqlx::query("UPDATE calendar_invites SET rsvp_status = ?, updated_at = ? WHERE id = ?")
                .bind(status)
                .bind(chrono::Utc::now().to_rfc3339())
                .bind(invite_id)
                .execute(self.pool)
                .await?;
        if result.rows_affected() == 0 {
            return Err(CalendarError::InviteNotFound);
        }
        Ok(())
    }

    pub async fn replace_google_mirror(
        &self,
        account_id: &str,
        sources: &[GoogleCalendarSource<'_>],
        events: &[GoogleCalendarEvent<'_>],
    ) -> Result<(), CalendarError> {
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        sqlx::query("DELETE FROM calendar_events WHERE account_id = ? AND source = 'google'")
            .bind(account_id)
            .execute(&mut *transaction)
            .await?;
        sqlx::query("DELETE FROM calendar_sources WHERE account_id = ? AND source = 'google'")
            .bind(account_id)
            .execute(&mut *transaction)
            .await?;

        let now = chrono::Utc::now().to_rfc3339();
        for source in sources {
            sqlx::query(
                "INSERT INTO calendar_sources
                    (account_id, source, remote_id, name, updated_at)
                 VALUES (?, 'google', ?, ?, ?)",
            )
            .bind(account_id)
            .bind(source.remote_id)
            .bind(source.name)
            .bind(&now)
            .execute(&mut *transaction)
            .await?;
        }
        for event in events {
            sqlx::query(
                "INSERT INTO calendar_events
                    (id, account_id, uid, title, description, location, starts_at, ends_at,
                     all_day, attendees_json, source, collection_href, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'google', ?, ?, ?)",
            )
            .bind(event.id)
            .bind(account_id)
            .bind(event.uid)
            .bind(event.title)
            .bind(event.description)
            .bind(event.location)
            .bind(event.starts_at)
            .bind(event.ends_at)
            .bind(if event.all_day { 1_i64 } else { 0_i64 })
            .bind(event.attendees_json)
            .bind(event.calendar_id)
            .bind(&now)
            .bind(&now)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    pub async fn upsert_caldav_event(
        &self,
        account_id: &str,
        collection_href: &str,
        remote_href: &str,
        etag: Option<&str>,
        raw_ics: &str,
    ) -> Result<bool, CalendarError> {
        let Some(event) = crate::email::calendar::parse_invite(raw_ics) else {
            return Ok(false);
        };
        let Some(starts_at) = event.starts_at.as_deref() else {
            return Ok(false);
        };
        let ends_at = event.ends_at.as_deref().unwrap_or(starts_at);
        let id = stable_id("dav-event", &format!("{account_id}|{remote_href}"));
        let attendees_json = serde_json::to_string(&parse_ics_attendees(raw_ics))
            .unwrap_or_else(|_| "[]".to_string());
        self.upsert_event(CalendarEventWrite {
            id: &id,
            account_id,
            uid: &event.uid,
            title: event.summary.as_deref().unwrap_or("(ohne Titel)"),
            description: event.description.as_deref(),
            location: event.location.as_deref(),
            starts_at,
            ends_at,
            all_day: event.all_day,
            attendees_json: &attendees_json,
            source: "caldav",
            remote_href: Some(remote_href),
            remote_etag: etag,
            collection_href: Some(collection_href),
            raw_ics: Some(raw_ics),
        })
        .await?;
        Ok(true)
    }

    /// Include a one-day margin for DATE ends. The caller interprets those
    /// calendar dates in the viewer's timezone and filters the exact boundary.
    pub async fn upcoming(&self, now: &str) -> Result<Vec<UpcomingCalendarRecord>, CalendarError> {
        sqlx::query_as::<_, UpcomingCalendarRecord>(
            "SELECT title, starts_at, ends_at, all_day, location, raw_ics
             FROM (
                 SELECT title, starts_at, ends_at, all_day, location, raw_ics
                 FROM calendar_events
                 WHERE CASE WHEN all_day = 1 THEN datetime(substr(ends_at, 1, 10), '+1 day') ELSE datetime(ends_at) END >= datetime(?)
                    OR (
                        raw_ics IS NOT NULL
                        AND instr(upper(substr(raw_ics, instr(raw_ics, 'BEGIN:VEVENT'))), 'RRULE:') > 0
                    )
                 UNION ALL
                 SELECT COALESCE(NULLIF(TRIM(invites.summary), ''), 'Termin') AS title,
                        invites.starts_at, invites.ends_at, invites.all_day, invites.location,
                        NULL AS raw_ics
                 FROM calendar_invites invites
                 WHERE invites.starts_at IS NOT NULL
                   AND invites.ends_at IS NOT NULL
                   AND CASE WHEN invites.all_day = 1 THEN datetime(substr(invites.ends_at, 1, 10), '+1 day') ELSE datetime(invites.ends_at) END >= datetime(?)
                   AND COALESCE(invites.cancelled, 0) = 0
                   AND invites.rsvp_status != 'declined'
                   AND NOT EXISTS (
                       SELECT 1 FROM calendar_events events
                       WHERE events.account_id = invites.account_id AND events.uid = invites.uid
                   )
             ) upcoming",
        )
        .bind(now)
        .bind(now)
        .fetch_all(self.pool)
        .await
        .map_err(Into::into)
    }
}

fn invite_id(account_id: &str, uid: &str, recurrence_id: &str) -> String {
    crate::sync::provider_identity::provider_local_id(
        "inv",
        "calendar",
        &[account_id, uid, recurrence_id],
    )
}

fn stable_id(prefix: &str, value: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(value.as_bytes());
    let suffix = digest[..12]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("{prefix}-{suffix}")
}

fn parse_ics_attendees(raw_ics: &str) -> Vec<String> {
    let mut unfolded: Vec<String> = Vec::new();
    for line in raw_ics.replace("\r\n", "\n").lines() {
        if (line.starts_with(' ') || line.starts_with('\t')) && !unfolded.is_empty() {
            unfolded.last_mut().unwrap().push_str(&line[1..]);
        } else {
            unfolded.push(line.trim_end_matches('\r').to_string());
        }
    }
    let mut attendees = Vec::new();
    for line in unfolded {
        if !line.to_ascii_uppercase().starts_with("ATTENDEE") {
            continue;
        }
        let Some((_, value)) = line.split_once(':') else {
            continue;
        };
        let email = value
            .trim()
            .trim_start_matches("mailto:")
            .trim_start_matches("MAILTO:")
            .to_ascii_lowercase();
        if email.contains('@') {
            attendees.push(email);
        }
    }
    attendees.sort();
    attendees.dedup();
    attendees
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    const REQUEST: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:event@example.test\r\nORGANIZER;CN=Alice:mailto:alice@example.test\r\nSUMMARY:Planning\r\nDTSTART:20260910T100000Z\r\nDTEND:20260910T110000Z\r\nSEQUENCE:1\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
    const CANCEL: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nMETHOD:CANCEL\r\nBEGIN:VEVENT\r\nUID:event@example.test\r\nORGANIZER;CN=Alice:mailto:alice@example.test\r\nSUMMARY:Planning\r\nDTSTART:20260910T100000Z\r\nDTEND:20260910T110000Z\r\nSEQUENCE:2\r\nSTATUS:CANCELLED\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    async fn pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::db::init(&pool).await.unwrap();
        pool
    }

    async fn account(pool: &SqlitePool, id: &str, created_at: &str) {
        sqlx::query(
            "INSERT INTO accounts
                (id, display_name, email, provider, config, credentials_ref, sync_state,
                 color, is_active, created_at, updated_at)
             VALUES (?, ?, ?, 'imap', '{}', '', '{}', '#123456', 1, ?, ?)",
        )
        .bind(id)
        .bind(id)
        .bind(format!("{id}@example.test"))
        .bind(created_at)
        .bind(created_at)
        .execute(pool)
        .await
        .unwrap();
    }

    fn local_event<'a>(id: &'a str, account_id: &'a str, title: &'a str) -> CalendarEventWrite<'a> {
        CalendarEventWrite {
            id,
            account_id,
            uid: id,
            title,
            description: None,
            location: None,
            starts_at: "2026-09-10T10:00:00Z",
            ends_at: "2026-09-10T11:00:00Z",
            all_day: false,
            attendees_json: "[]",
            source: "local",
            remote_href: None,
            remote_etag: None,
            collection_href: None,
            raw_ics: None,
        }
    }

    #[tokio::test]
    async fn lists_calendar_records_with_original_scope_and_sorting() {
        let pool = pool().await;
        account(&pool, "b", "2026-01-02").await;
        account(&pool, "a", "2026-01-01").await;
        let repository = CalendarRepository::new(&pool);
        repository
            .upsert_event(local_event("late", "a", "Late"))
            .await
            .unwrap();
        let mut early = local_event("early", "b", "Early");
        early.starts_at = "2026-09-09T10:00:00Z";
        early.ends_at = "2026-09-09T11:00:00Z";
        repository.upsert_event(early).await.unwrap();
        sqlx::query(
            "INSERT INTO calendar_sources (account_id, source, remote_id, name, updated_at)
             VALUES ('a', 'google', 'z', 'Zulu', 'now'), ('a', 'google', 'a', 'alpha', 'now'),
                    ('b', 'google', 'b', 'Beta', 'now')",
        )
        .execute(&pool)
        .await
        .unwrap();

        assert_eq!(
            repository
                .list_events(None)
                .await
                .unwrap()
                .iter()
                .map(|event| event.id.as_str())
                .collect::<Vec<_>>(),
            vec!["early", "late"]
        );
        assert_eq!(repository.list_events(Some("a")).await.unwrap().len(), 1);
        assert_eq!(
            repository
                .list_sources(Some("a"))
                .await
                .unwrap()
                .iter()
                .map(|source| source.name.as_str())
                .collect::<Vec<_>>(),
            vec!["alpha", "Zulu"]
        );
    }

    #[tokio::test]
    async fn invitation_identity_preserves_a_legacy_id_and_rsvp_on_reimport() {
        let pool = pool().await;
        account(&pool, "account", "2026-01-01").await;
        sqlx::query("INSERT INTO calendar_invites (id, account_id, uid, rsvp_status, raw_ics, created_at, updated_at) VALUES ('inv-legacy-event', 'account', 'event@example.test', 'accepted', ?, '', '')")
            .bind(REQUEST).execute(&pool).await.unwrap();
        let repository = CalendarRepository::new(&pool);
        let revised = REQUEST.replace("SUMMARY:Planning", "SUMMARY:Revised planning");
        let request = crate::email::calendar::parse_invite(&revised).unwrap();
        let updated = repository
            .upsert_invite("account", None, &request)
            .await
            .unwrap();
        assert_eq!(updated.id, "inv-legacy-event");
        assert_eq!(updated.rsvp_status, "accepted");
        assert_eq!(updated.summary.as_deref(), Some("Revised planning"));
        assert_eq!(
            repository.get_invite("inv-legacy-event").await.unwrap(),
            updated
        );
        assert_eq!(
            repository
                .list_invites(Some("account"))
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn invitation_identity_distinguishes_punctuation_in_imported_ics_uids() {
        let pool = pool().await;
        account(&pool, "account", "2026-01-01").await;
        let repository = CalendarRepository::new(&pool);
        let mut ids = Vec::new();
        for uid in ["meeting/a", "meeting-a"] {
            let raw = REQUEST.replace("UID:event@example.test", &format!("UID:{uid}"));
            let request = crate::email::calendar::parse_invite(&raw).unwrap();
            assert_eq!(request.uid, uid);
            let invitation = repository
                .upsert_invite("account", None, &request)
                .await
                .unwrap();
            ids.push(invitation.id);
        }
        assert_ne!(ids[0], ids[1]);
        assert_eq!(
            repository
                .find_invite("account", "meeting/a", None)
                .await
                .unwrap()
                .id,
            ids[0]
        );
        assert_eq!(
            repository
                .find_invite("account", "meeting-a", None)
                .await
                .unwrap()
                .id,
            ids[1]
        );
        assert_eq!(
            repository
                .list_invites(Some("account"))
                .await
                .unwrap()
                .len(),
            2
        );
    }

    #[tokio::test]
    async fn invite_reimports_preserve_rsvp_and_cancellation_is_terminal() {
        let pool = pool().await;
        account(&pool, "account", "2026-01-01").await;
        let repository = CalendarRepository::new(&pool);
        let request = crate::email::calendar::parse_invite(REQUEST).unwrap();
        let first = repository
            .upsert_invite("account", None, &request)
            .await
            .unwrap();
        repository
            .update_rsvp_status(&first.id, "accepted")
            .await
            .unwrap();
        repository
            .upsert_invite("account", None, &request)
            .await
            .unwrap();
        assert_eq!(
            repository.get_invite(&first.id).await.unwrap().rsvp_status,
            "accepted"
        );

        let cancel = crate::email::calendar::parse_invite(CANCEL).unwrap();
        repository
            .upsert_invite("account", None, &cancel)
            .await
            .unwrap();
        repository
            .upsert_invite("account", None, &request)
            .await
            .unwrap();
        assert_eq!(repository.get_invite(&first.id).await.unwrap().cancelled, 1);

        let recurring =
            REQUEST.replace("SEQUENCE:1", "RECURRENCE-ID:20260917T100000Z\r\nSEQUENCE:1");
        let recurring = crate::email::calendar::parse_invite(&recurring).unwrap();
        let occurrence = repository
            .upsert_invite("account", None, &recurring)
            .await
            .unwrap();
        assert_ne!(occurrence.id, first.id);
        assert_eq!(
            repository
                .list_invites(Some("account"))
                .await
                .unwrap()
                .len(),
            2
        );
    }

    #[tokio::test]
    async fn event_updates_and_deletes_are_account_safe() {
        let pool = pool().await;
        account(&pool, "one", "2026-01-01").await;
        account(&pool, "two", "2026-01-02").await;
        let repository = CalendarRepository::new(&pool);
        repository
            .upsert_event(local_event("event", "one", "Original"))
            .await
            .unwrap();

        assert!(matches!(
            repository.event_sync_state("two", "event").await,
            Err(CalendarError::EventOwnership)
        ));
        assert!(matches!(
            repository
                .upsert_event(local_event("event", "two", "Stolen"))
                .await,
            Err(CalendarError::EventOwnership)
        ));
        assert_eq!(
            repository.get_event("one", "event").await.unwrap().title,
            "Original"
        );
        assert!(matches!(
            repository.delete_event("two", "event").await,
            Err(CalendarError::EventNotFound)
        ));
        repository.delete_event("one", "event").await.unwrap();
        assert!(matches!(
            repository.get_event("one", "event").await,
            Err(CalendarError::EventNotFound)
        ));
    }

    #[tokio::test]
    async fn google_mirror_replacement_rolls_back_as_one_change() {
        let pool = pool().await;
        account(&pool, "account", "2026-01-01").await;
        let repository = CalendarRepository::new(&pool);
        repository
            .replace_google_mirror(
                "account",
                &[GoogleCalendarSource {
                    remote_id: "old",
                    name: "Old",
                }],
                &[GoogleCalendarEvent {
                    id: "old-event",
                    uid: "old-event",
                    title: "Old",
                    description: None,
                    location: None,
                    starts_at: "2026-09-10T10:00:00Z",
                    ends_at: "2026-09-10T11:00:00Z",
                    all_day: false,
                    attendees_json: "[]",
                    calendar_id: "old",
                }],
            )
            .await
            .unwrap();
        let duplicate = GoogleCalendarSource {
            remote_id: "duplicate",
            name: "Duplicate",
        };
        assert!(repository
            .replace_google_mirror("account", &[duplicate.clone(), duplicate], &[])
            .await
            .is_err());
        assert_eq!(
            repository.list_sources(Some("account")).await.unwrap()[0].remote_id,
            "old"
        );
        assert_eq!(
            repository.list_events(Some("account")).await.unwrap()[0].id,
            "old-event"
        );
    }

    #[tokio::test]
    async fn caldav_projection_keeps_stable_identity_and_updates_etag() {
        let pool = pool().await;
        account(&pool, "account", "2026-01-01").await;
        let repository = CalendarRepository::new(&pool);
        let raw = REQUEST.replace(
            "ORGANIZER;CN=Alice:mailto:alice@example.test",
            "ORGANIZER;CN=Alice:mailto:alice@example.test\r\nATTENDEE:mailto:PERSON@EXAMPLE.TEST",
        );
        assert!(repository
            .upsert_caldav_event(
                "account",
                "/calendar/",
                "/calendar/event.ics",
                Some("one"),
                &raw
            )
            .await
            .unwrap());
        assert!(repository
            .upsert_caldav_event(
                "account",
                "/calendar/",
                "/calendar/event.ics",
                Some("two"),
                &raw
            )
            .await
            .unwrap());
        let stored: (i64, String, String) = sqlx::query_as(
            "SELECT COUNT(*), MAX(remote_etag), MAX(attendees_json) FROM calendar_events
             WHERE account_id = 'account' AND source = 'caldav'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            stored,
            (1, "two".into(), "[\"person@example.test\"]".into())
        );
    }

    #[tokio::test]
    async fn upcoming_selection_excludes_declines_cancellations_and_mirrored_invites() {
        let pool = pool().await;
        account(&pool, "account", "2026-01-01").await;
        let repository = CalendarRepository::new(&pool);
        let request = crate::email::calendar::parse_invite(REQUEST).unwrap();
        let invite = repository
            .upsert_invite("account", None, &request)
            .await
            .unwrap();
        assert_eq!(
            repository
                .upcoming("2026-09-01T00:00:00Z")
                .await
                .unwrap()
                .len(),
            1
        );
        repository
            .update_rsvp_status(&invite.id, "declined")
            .await
            .unwrap();
        assert!(repository
            .upcoming("2026-09-01T00:00:00Z")
            .await
            .unwrap()
            .is_empty());
        repository
            .update_rsvp_status(&invite.id, "accepted")
            .await
            .unwrap();
        repository
            .upsert_event(local_event("mirror", "account", "Planning"))
            .await
            .unwrap();
        sqlx::query("UPDATE calendar_events SET uid = ? WHERE id = 'mirror'")
            .bind(&request.uid)
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            repository
                .upcoming("2026-09-01T00:00:00Z")
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn persistence_errors_hide_database_detail() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        let error = CalendarRepository::new(&pool)
            .list_events(None)
            .await
            .unwrap_err();
        assert!(matches!(error, CalendarError::Persistence(_)));
        assert_eq!(error.to_string(), "calendar persistence failed");
        assert!(!error.to_string().contains("no such table"));
    }
}
