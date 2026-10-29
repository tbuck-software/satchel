use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::{future::Future, pin::Pin};

use crate::commands::error::{CommandError, CommandResult};
use crate::commands::messages::{OutgoingAttachmentInput, OutgoingMessageInput};
use crate::db::accounts::AccountRepository;
use crate::db::calendar::{CalendarError, CalendarRepository};
pub use crate::db::calendar::{
    CalendarEventRecord as CalendarEventDto, CalendarInviteRecord as CalendarInviteDto,
    CalendarSourceRecord as CalendarSourceDto,
};
use crate::email::calendar::{build_invite_reply_ics, extract_ics_block, parse_invite};
use crate::sync::outbox::build_idempotent_send_action;
use crate::AppState;

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/calendar/", optional_fields = nullable))]
pub struct CalendarEventInput {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub expected_updated_at: Option<String>,
    pub account_id: String,
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub location: Option<String>,
    pub starts_at: String,
    pub ends_at: String,
    #[serde(default)]
    pub all_day: bool,
    #[serde(default)]
    pub attendees: Vec<String>,
    #[serde(default)]
    pub send_invites: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/calendar/", optional_fields = nullable))]
pub struct InviteResponse {
    pub invite_id: String,
    /// "accepted" | "declined" | "tentative"
    pub response: String,
    #[serde(default)]
    pub comment: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/calendar/", optional_fields = nullable))]
pub struct ImportIcsInput {
    /// Optional account_id to anchor the invite to. If omitted, the first active
    /// account is used (works fine for the common single-account case).
    #[serde(default)]
    pub account_id: Option<String>,
    /// Either the raw ICS text or a base64-encoded file content.
    pub raw: String,
    #[serde(default)]
    pub source_filename: Option<String>,
}

impl From<CalendarError> for CommandError {
    fn from(error: CalendarError) -> Self {
        let key = match error {
            CalendarError::InviteNotFound => "backend.errors.calendarInviteNotFound",
            CalendarError::EventOwnership => "backend.errors.calendarEventOwnership",
            CalendarError::EventNotFound => "backend.errors.calendarEventNotFound",
            CalendarError::EventChanged => "backend.errors.calendarEventChanged",
            CalendarError::Persistence(error) => {
                tracing::error!(%error, "calendar persistence failed");
                "backend.errors.calendarPersistenceFailed"
            }
        };
        Self::new(key)
    }
}

impl From<satchel_core::calendar::SaveError> for CommandError {
    fn from(error: satchel_core::calendar::SaveError) -> Self {
        use satchel_core::calendar::SaveError;
        match error {
            SaveError::Validation(key) => Self::new(key),
            SaveError::Account(error) => error.into(),
            SaveError::Calendar(error) => error.into(),
            SaveError::Remote(_) => Self::new("backend.errors.calendarRemoteWriteFailed"),
            SaveError::RemoteCommittedLocalFailed(error) => {
                tracing::error!(%error, "calendar remote write succeeded but local persistence failed");
                Self::new("backend.errors.calendarRemoteSavedSyncRequired")
            }
            SaveError::Invitation(error) => {
                tracing::error!(%error, "calendar invitation could not be built");
                Self::new("backend.errors.calendarPersistenceFailed")
            }
        }
    }
}

#[tauri::command]
pub async fn import_ics(
    pool: tauri::State<'_, SqlitePool>,
    input: ImportIcsInput,
) -> CommandResult<CalendarInviteDto> {
    let raw = decode_ics_payload(&input.raw)?;
    let block = extract_ics_block(Some(&raw), None)
        .ok_or_else(|| CommandError::new("backend.errors.calendarPayloadInvalid"))?;
    let parsed = parse_invite(&block)
        .ok_or_else(|| CommandError::new("backend.errors.calendarPayloadInvalid"))?;

    let account_id = match input.account_id.as_deref() {
        Some(id) if !id.is_empty() => id.to_string(),
        _ => AccountRepository::new(pool.inner())
            .first_active_id()
            .await?
            .ok_or_else(|| CommandError::new("backend.errors.noActiveAccount"))?,
    };

    let dto = CalendarRepository::new(pool.inner())
        .upsert_invite(&account_id, None, &parsed)
        .await?;

    Ok(with_current_recurrence_metadata(dto))
}

fn decode_ics_payload(raw: &str) -> CommandResult<String> {
    let trimmed = raw.trim();
    if trimmed.starts_with("BEGIN:VCALENDAR") || trimmed.contains("\nBEGIN:VCALENDAR") {
        return Ok(trimmed.to_string());
    }
    // Fall back to base64 decode (drag-drop / file-import payloads).
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(trimmed.as_bytes())
        .map_err(|_| CommandError::new("backend.errors.calendarPayloadInvalid"))?;
    String::from_utf8(bytes).map_err(|_| CommandError::new("backend.errors.calendarPayloadInvalid"))
}

#[tauri::command]
pub async fn import_ics_path(
    pool: tauri::State<'_, SqlitePool>,
    path: String,
) -> CommandResult<CalendarInviteDto> {
    crate::commands::file_limits::guard_import_size(std::path::Path::new(&path)).await?;
    let bytes = tokio::fs::read(&path).await.map_err(|error| {
        tracing::warn!(%error, %path, "calendar file could not be read");
        CommandError::new("backend.errors.calendarFileReadFailed")
    })?;
    let raw = String::from_utf8(bytes)
        .map_err(|_| CommandError::new("backend.errors.calendarPayloadInvalid"))?;
    import_ics(
        pool,
        ImportIcsInput {
            account_id: None,
            raw,
            source_filename: Some(path),
        },
    )
    .await
}

#[tauri::command]
pub async fn list_invites(
    pool: tauri::State<'_, crate::db::ReadPool>,
    account_id: Option<String>,
) -> CommandResult<Vec<CalendarInviteDto>> {
    let rows = CalendarRepository::new(pool.inner().as_ref())
        .list_invites(account_id.as_deref())
        .await?;
    Ok(rows
        .into_iter()
        .map(with_current_recurrence_metadata)
        .collect())
}

#[tauri::command]
pub async fn list_calendar_events(
    pool: tauri::State<'_, crate::db::ReadPool>,
    account_id: Option<String>,
) -> CommandResult<Vec<CalendarEventDto>> {
    CalendarRepository::new(pool.inner().as_ref())
        .list_events(account_id.as_deref())
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn list_calendar_sources(
    pool: tauri::State<'_, crate::db::ReadPool>,
    account_id: Option<String>,
) -> CommandResult<Vec<CalendarSourceDto>> {
    CalendarRepository::new(pool.inner().as_ref())
        .list_sources(account_id.as_deref())
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn create_calendar_event(
    state: tauri::State<'_, AppState>,
    pool: tauri::State<'_, SqlitePool>,
    input: CalendarEventInput,
) -> CommandResult<CalendarEventDto> {
    satchel_core::calendar::save_event(
        pool.inner(),
        &state.queue,
        satchel_core::calendar::EventInput {
            id: input.id,
            expected_updated_at: input.expected_updated_at,
            account_id: input.account_id,
            title: input.title,
            description: input.description,
            location: input.location,
            starts_at: input.starts_at,
            ends_at: input.ends_at,
            all_day: input.all_day,
            attendees: input.attendees,
            send_invites: input.send_invites,
        },
    )
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn delete_calendar_event(
    pool: tauri::State<'_, SqlitePool>,
    id: String,
) -> CommandResult<()> {
    satchel_core::calendar::delete_event(pool.inner(), &id)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn respond_to_invite(
    state: tauri::State<'_, AppState>,
    pool: tauri::State<'_, SqlitePool>,
    input: InviteResponse,
) -> CommandResult<()> {
    queue_invite_response(state.inner(), pool.inner(), input).await
}

/// Persist one logical RSVP for background delivery. The deterministic queue
/// id and Message-ID make duplicate UI events, retries and restarts harmless.
pub async fn queue_invite_response(
    state: &AppState,
    pool: &SqlitePool,
    input: InviteResponse,
) -> CommandResult<()> {
    let repository = CalendarRepository::new(pool);
    let invite = repository.get_invite(&input.invite_id).await?;

    let normalized = match input.response.as_str() {
        "accepted" | "yes" | "ACCEPTED" => "ACCEPTED",
        "declined" | "no" | "DECLINED" => "DECLINED",
        "tentative" | "maybe" | "TENTATIVE" => "TENTATIVE",
        _ => return Err(CommandError::new("backend.errors.calendarResponseInvalid")),
    };

    if invite.cancelled == 1 {
        return Err(CommandError::new("backend.errors.calendarInviteCancelled"));
    }

    let rsvp_string = match normalized {
        "ACCEPTED" => "accepted",
        "DECLINED" => "declined",
        "TENTATIVE" => "tentative",
        _ => "needs-action",
    };
    if invite.rsvp_status == rsvp_string {
        return Ok(());
    }

    let parsed = parse_invite(&invite.raw_ics)
        .ok_or_else(|| CommandError::new("backend.errors.calendarInviteInvalid"))?;
    if parsed.uid != invite.uid {
        return Err(CommandError::new("backend.errors.calendarInviteInvalid"));
    }

    let account = AccountRepository::new(pool)
        .identity(&invite.account_id)
        .await?;

    let reply_ics = build_invite_reply_ics(
        &parsed,
        &account.email,
        account.display_name.as_deref(),
        normalized,
    )
    .map_err(|error| {
        tracing::warn!(%error, invite_id = %invite.id, "calendar reply could not be built");
        CommandError::new("backend.errors.calendarInviteInvalid")
    })?;

    let organizer = invite
        .organizer_email
        .clone()
        .ok_or_else(|| CommandError::new("backend.errors.calendarOrganizerMissing"))?;

    let summary = invite
        .summary
        .clone()
        .unwrap_or_else(|| "Calendar Invitation".to_string());

    let prefix = match normalized {
        "ACCEPTED" => "Accepted",
        "DECLINED" => "Declined",
        "TENTATIVE" => "Tentatively Accepted",
        _ => "Replied",
    };

    let body = format!(
        "{prefix}: {summary}\n\n{}",
        input.comment.unwrap_or_default()
    );
    let ics_b64 = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        reply_ics.as_bytes(),
    );

    let outgoing_input = OutgoingMessageInput {
        to: vec![organizer],
        subject: format!("{prefix}: {summary}"),
        body,
        attachments: vec![OutgoingAttachmentInput {
            filename: "invite.ics".into(),
            mime_type: "text/calendar; method=REPLY; charset=UTF-8".into(),
            content_base64: ics_b64,
            content_id: None,
            inline: false,
        }],
        ..Default::default()
    };

    let idempotency_key = format!(
        "calendar-rsvp|{}|{}|{}|{}|{}",
        invite.account_id,
        invite.uid,
        parsed.recurrence_id.as_deref().unwrap_or("master"),
        invite.sequence.unwrap_or(0),
        normalized,
    );
    let action =
        build_idempotent_send_action(invite.account_id.clone(), outgoing_input, &idempotency_key)
            .map_err(|error| {
            tracing::error!(%error, invite_id = %invite.id, "calendar reply could not be queued");
            CommandError::new("backend.errors.calendarPersistenceFailed")
        })?;
    state.queue.enqueue(pool, action).await.map_err(|error| {
        tracing::error!(%error, invite_id = %invite.id, "calendar reply queue write failed");
        CommandError::new("backend.errors.calendarPersistenceFailed")
    })?;

    repository
        .update_rsvp_status(&input.invite_id, rsvp_string)
        .await?;

    Ok(())
}

fn with_current_recurrence_metadata(mut invite: CalendarInviteDto) -> CalendarInviteDto {
    if let Some(parsed) = parse_invite(&invite.raw_ics) {
        invite.recurrence_id = parsed.recurrence_id;
        invite.recurring = if parsed.recurring { 1 } else { 0 };
        invite.recurrence_unbounded = if parsed.recurrence_unbounded { 1 } else { 0 };
    }
    invite
}

#[cfg(test)]
fn build_reply_ics(
    invite: &CalendarInviteDto,
    account_email: &str,
    account_name: Option<&str>,
    partstat: &str,
) -> Result<String, String> {
    let parsed = parse_invite(&invite.raw_ics)
        .ok_or_else(|| "stored invitation no longer contains a VEVENT".to_string())?;
    build_invite_reply_ics(&parsed, account_email, account_name, partstat)
}

#[derive(Debug, Clone, Default, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/calendar/")
)]
pub struct GoogleCalendarSyncSummary {
    pub accounts_checked: usize,
    pub events_synced: usize,
    pub errors: Vec<String>,
}

/// Mirror the visible Google calendars of every active Gmail account (or one
/// account) into `calendar_events`. Per-account failures land in `errors`
/// instead of aborting the rest — one revoked token must not hide the other
/// accounts' calendars.
#[tauri::command]
pub async fn sync_google_calendar(
    pool: tauri::State<'_, SqlitePool>,
    account_id: Option<String>,
) -> CommandResult<GoogleCalendarSyncSummary> {
    sync_google_calendar_with(
        pool.inner(),
        account_id.as_deref(),
        |pool, account_id, config| {
            Box::pin(crate::providers::google_calendar::sync_account(
                pool, account_id, config,
            ))
        },
    )
    .await
}

type CalendarSyncFuture<'a> = Pin<Box<dyn Future<Output = Result<usize, String>> + Send + 'a>>;

async fn sync_google_calendar_with<F>(
    pool: &SqlitePool,
    account_id: Option<&str>,
    sync_account: F,
) -> CommandResult<GoogleCalendarSyncSummary>
where
    F: for<'a> Fn(
        &'a SqlitePool,
        &'a str,
        &'a crate::commands::accounts::AccountConfig,
    ) -> CalendarSyncFuture<'a>,
{
    let accounts = AccountRepository::new(pool)
        .list_active_gmail_calendar_accounts(account_id)
        .await?;

    let mut summary = GoogleCalendarSyncSummary::default();
    for account in accounts {
        summary.accounts_checked += 1;
        let config = match AccountRepository::new(pool)
            .load_provider_config(&account.id)
            .await
        {
            Ok(Some(config)) => config.config,
            Ok(None) => {
                tracing::warn!(account_id = %account.id, "google calendar account disappeared before sync");
                summary
                    .errors
                    .push(google_calendar_sync_error(&account.email));
                continue;
            }
            Err(error) => {
                tracing::warn!(account_id = %account.id, %error, "google calendar credentials could not be loaded");
                summary
                    .errors
                    .push(google_calendar_sync_error(&account.email));
                continue;
            }
        };
        if crate::utils::oauth::google_scope_granted(
            &config,
            crate::utils::oauth::GOOGLE_CALENDAR_SCOPE,
        ) == Some(false)
        {
            continue;
        }
        match sync_account(pool, &account.id, &config).await {
            Ok(count) => summary.events_synced += count,
            Err(error) => {
                tracing::warn!(account_id = %account.id, %error, "google calendar sync failed");
                summary
                    .errors
                    .push(google_calendar_sync_error(&account.email));
            }
        }
    }
    Ok(summary)
}

fn google_calendar_sync_error(account: &str) -> String {
    CommandError::new("backend.errors.googleCalendarSyncFailed")
        .with_param("account", account)
        .encoded()
}

#[cfg(test)]
mod tests {
    use super::*;
    use icalendar::Calendar;
    use sqlx::sqlite::SqlitePoolOptions;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    fn invite() -> CalendarInviteDto {
        CalendarInviteDto {
            id: "inv-1".into(),
            message_id: Some("m-1".into()),
            account_id: "acc".into(),
            uid: "abc-123@example.com".into(),
            organizer_email: Some("alice@example.com".into()),
            organizer_name: Some("Alice".into()),
            summary: Some("Wochenplanung".into()),
            description: None,
            location: None,
            starts_at: Some("2026-03-15T13:30:00Z".into()),
            ends_at: Some("2026-03-15T14:00:00Z".into()),
            all_day: 0,
            rsvp_status: "needs-action".into(),
            raw_ics: "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:abc-123@example.com\r\nORGANIZER;CN=Alice:mailto:alice@example.com\r\nSUMMARY:Wochenplanung\r\nDTSTART:20260315T133000Z\r\nDTEND:20260315T140000Z\r\nSEQUENCE:0\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n".into(),
            sequence: Some(0),
            method: Some("REQUEST".into()),
            cancelled: 0,
            recurrence_id: None,
            recurring: 0,
            recurrence_unbounded: 0,
            created_at: "2026-03-01T00:00:00Z".into(),
            updated_at: "2026-03-01T00:00:00Z".into(),
        }
    }

    /// Undo RFC 5545 §3.1 line folding, which splits long properties across
    /// physical lines with a leading space.
    fn unfold(ics: &str) -> String {
        let mut out = String::with_capacity(ics.len());
        for line in ics.lines() {
            if line.starts_with(' ') || line.starts_with('\t') {
                out.push_str(&line[1..]);
            } else {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(line);
            }
        }
        out
    }

    /// The attendee line used to be built with ":" between parameters, making
    /// the whole string the property value. Organizers could not match the
    /// attendee, so every acceptance and decline was silently discarded.
    #[test]
    fn reply_puts_partstat_in_a_parameter_not_the_value() {
        let raw = build_reply_ics(&invite(), "me@example.com", Some("Me"), "ACCEPTED")
            .expect("builds reply");
        let ics = unfold(&raw);

        let attendee = ics
            .lines()
            .find(|line| line.starts_with("ATTENDEE"))
            .expect("reply contains an ATTENDEE line");

        let (params, value) = attendee.split_once(':').expect("attendee has a value");
        assert!(
            params.contains("PARTSTAT=ACCEPTED"),
            "PARTSTAT must be a parameter, got: {attendee}"
        );
        assert!(
            params.contains("CN=Me"),
            "CN must be a parameter: {attendee}"
        );
        assert_eq!(
            value, "mailto:me@example.com",
            "the property value must be the bare mailto URI"
        );
    }

    #[test]
    fn reply_is_a_well_formed_reply_calendar() {
        let raw =
            build_reply_ics(&invite(), "me@example.com", None, "DECLINED").expect("builds reply");
        let ics = unfold(&raw);

        assert!(ics.contains("METHOD:REPLY"));
        assert!(ics.contains("UID:abc-123@example.com"));
        let organizer = ics
            .lines()
            .find(|line| line.starts_with("ORGANIZER"))
            .expect("reply contains an ORGANIZER line");
        assert_eq!(
            organizer.split_once(':').map(|(_, value)| value),
            Some("mailto:alice@example.com")
        );
        // DTSTAMP is mandatory in an iTIP REPLY.
        assert!(ics.contains("DTSTAMP:"), "reply must carry DTSTAMP");
        assert!(raw.parse::<Calendar>().is_ok(), "reply must re-parse");
    }

    #[test]
    fn reply_round_trips_each_participation_status() {
        for status in ["ACCEPTED", "DECLINED", "TENTATIVE"] {
            let ics = unfold(
                &build_reply_ics(&invite(), "me@example.com", None, status).expect("builds reply"),
            );
            assert!(
                ics.contains(&format!("PARTSTAT={status}")),
                "missing PARTSTAT={status}"
            );
        }
    }

    #[test]
    fn persistence_errors_use_a_stable_redacted_key() {
        let detail = "no such table: calendar_events";
        let error = CommandError::from(CalendarError::Persistence(sqlx::Error::Protocol(
            detail.into(),
        )));
        let serialized = serde_json::to_string(&error).unwrap();

        assert_eq!(error.key, "backend.errors.calendarPersistenceFailed");
        assert!(!serialized.contains(detail));
    }

    #[tokio::test]
    async fn google_sync_skips_ungranted_accounts_and_continues_after_bad_credentials() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::db::init(&pool).await.unwrap();
        for (id, email, config, created_at) in [
            (
                "broken",
                "broken@example.test",
                "enc:v1:not-valid-ciphertext",
                "2026-01-01",
            ),
            (
                "working",
                "working@example.test",
                r#"{"oauth":{"access_token":"test"}}"#,
                "2026-01-02",
            ),
            (
                "mail-only",
                "mail@example.test",
                r#"{"oauth":{"access_token":"mail", "scope":"https://www.googleapis.com/auth/gmail.modify"}}"#,
                "2026-01-03",
            ),
        ] {
            satchel_core::utils::crypto::seed_master_key_for_tests();
            let config = if config.starts_with("enc:") {
                config.to_string()
            } else {
                crate::commands::accounts::encrypt_account_config(id, config).unwrap()
            };
            sqlx::query(
                "INSERT INTO accounts
                    (id, display_name, email, provider, config, credentials_ref, sync_state,
                     color, is_active, created_at, updated_at)
                 VALUES (?, ?, ?, 'gmail', ?, '', '{}', '#123456', 1, ?, ?)",
            )
            .bind(id)
            .bind(id)
            .bind(email)
            .bind(config)
            .bind(created_at)
            .bind(created_at)
            .execute(&pool)
            .await
            .unwrap();
        }

        let attempts = Arc::new(AtomicUsize::new(0));
        let attempts_for_sync = Arc::clone(&attempts);
        let summary = sync_google_calendar_with(&pool, None, move |_pool, account_id, config| {
            let attempts = Arc::clone(&attempts_for_sync);
            let account_id = account_id.to_string();
            let config = config.to_string();
            Box::pin(async move {
                attempts.fetch_add(1, Ordering::SeqCst);
                assert_eq!(account_id, "working");
                assert!(config.contains("access_token"));
                Ok(7)
            })
        })
        .await
        .unwrap();

        assert_eq!(attempts.load(Ordering::SeqCst), 1);
        assert_eq!(summary.accounts_checked, 3);
        assert_eq!(summary.events_synced, 7);
        assert_eq!(summary.errors.len(), 1);
        assert!(summary.errors[0].contains("backend.errors.googleCalendarSyncFailed"));
        assert!(!summary.errors[0].contains("ciphertext"));
    }
}
