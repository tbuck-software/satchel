//! Free/busy for scheduling: who is busy when, for the account owner and for
//! the people they invite.
//!
//! The owner's time always comes from the local calendar mirror
//! ([`availability::own_busy`]). Other people's time is asked of the account
//! the event belongs to: Google's `freeBusy.query`, Microsoft Graph's
//! `getSchedule`, or a CalDAV server's scheduling outbox (RFC 6638) when it
//! advertises `calendar-auto-schedule`. The existing calendar grants cover
//! all three (`calendar.readonly`, `Calendars.ReadWrite`); nothing here asks
//! for a new permission or touches mail sign-in.
//!
//! Nobody's free/busy is an error. An account without calendar access, a
//! provider that will not answer for an address, or a failed request all come
//! back as "unknown" for the people concerned, and the editor shows them so.
//!
//! Answers are cached for a few minutes per account, address and day range;
//! a different range asks again.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration as StdDuration, Instant};

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use sqlx::SqlitePool;

use super::availability::{self, AvailabilityError, Busy, WorkingHours};

/// How long an answer from a provider is reused.
const ANSWER_TTL: StdDuration = StdDuration::from_secs(5 * 60);
/// A failed request is retried sooner than an answer is refreshed.
const FAILURE_TTL: StdDuration = StdDuration::from_secs(60);
/// The editor shows a day, "next free" looks two weeks ahead; nothing needs more.
const MAX_RANGE_DAYS: i64 = 62;
/// More people than this in one request is not a meeting the strip can show.
const MAX_ATTENDEES: usize = 50;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "core/calendar/free_busy/")
)]
pub struct BusyBlock {
    /// RFC 3339, UTC.
    pub starts_at: String,
    pub ends_at: String,
    /// Accepted with a maybe, or marked tentative by the provider.
    pub tentative: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "core/calendar/free_busy/")
)]
pub struct AttendeeFreeBusy {
    /// Lower-case address as it was asked for.
    pub address: String,
    /// False when nobody would say: no calendar access, a provider without
    /// free/busy, an address it does not know, or a failed request.
    pub known: bool,
    pub busy: Vec<BusyBlock>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "core/calendar/free_busy/")
)]
pub struct FreeBusyReport {
    /// The account owner's busy time across every calendar that counts.
    pub own: Vec<BusyBlock>,
    pub attendees: Vec<AttendeeFreeBusy>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "core/calendar/free_busy/")
)]
pub struct FreeWindow {
    pub starts_at: String,
    pub ends_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "core/calendar/free_busy/")
)]
pub struct FreeDay {
    /// `YYYY-MM-DD` in `FreeSlotsReport::time_zone`.
    pub date: String,
    pub windows: Vec<FreeWindow>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "core/calendar/free_busy/")
)]
pub struct FreeSlotsReport {
    /// The IANA zone the days and working hours were read in.
    pub time_zone: String,
    pub days: Vec<FreeDay>,
    /// Attendees whose time could not be taken into account.
    pub unknown: Vec<String>,
}

/// What the free/busy and free-slot queries share.
#[derive(Debug, Clone)]
pub struct Participants<'a> {
    /// The account the event belongs to; it is asked about the attendees.
    pub account_id: Option<&'a str>,
    pub attendees: &'a [String],
    /// Calendar keys ([`availability::calendar_key`]) that do not count
    /// towards the owner's availability.
    pub excluded_calendars: &'a [String],
}

#[derive(Debug, thiserror::Error)]
pub enum FreeBusyError {
    #[error(transparent)]
    Invalid(#[from] AvailabilityError),
    #[error("free/busy persistence failed")]
    Persistence(#[from] sqlx::Error),
}

pub async fn query(
    pool: &SqlitePool,
    participants: &Participants<'_>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    zone: &str,
) -> Result<FreeBusyReport, FreeBusyError> {
    check_range(start, end)?;
    let zone = availability::parse_zone(zone)?;
    let own =
        availability::own_busy(pool, start, end, zone, participants.excluded_calendars).await?;
    let attendees = attendee_busy(pool, participants, start, end).await?;
    Ok(FreeBusyReport {
        own: own.iter().map(block_record).collect(),
        attendees: attendees
            .into_iter()
            .map(|(address, busy)| AttendeeFreeBusy {
                address,
                known: busy.is_some(),
                busy: busy.unwrap_or_default().iter().map(block_record).collect(),
            })
            .collect(),
    })
}

/// Free windows within working hours for the owner and every attendee whose
/// time is known, over the next `working_days` working days from `from`.
pub async fn free_slots(
    pool: &SqlitePool,
    participants: &Participants<'_>,
    from: DateTime<Utc>,
    working_days: u32,
    duration_minutes: u32,
    hours: &WorkingHours,
    zone_name: &str,
) -> Result<FreeSlotsReport, FreeBusyError> {
    let zone = availability::parse_zone(zone_name)?;
    if !(5..=24 * 60).contains(&duration_minutes) {
        return Err(AvailabilityError::Duration.into());
    }
    if working_days == 0 || working_days > 20 {
        return Err(AvailabilityError::Range.into());
    }
    // Working days skip weekends and the like; a calendar week per five
    // working days, plus slack, always covers them.
    let span = Duration::days(i64::from(working_days.div_ceil(5)) * 7 + 3);
    let end = from + span;
    let mut busy =
        availability::own_busy(pool, from, end, zone, participants.excluded_calendars).await?;
    let mut unknown = Vec::new();
    for (address, answer) in attendee_busy(pool, participants, from, end).await? {
        match answer {
            Some(blocks) => busy.extend(blocks),
            None => unknown.push(address),
        }
    }
    let days = availability::free_days(
        &busy,
        zone,
        hours,
        from,
        working_days,
        Duration::minutes(i64::from(duration_minutes)),
    )?;
    Ok(FreeSlotsReport {
        time_zone: zone.name().to_string(),
        days: days
            .into_iter()
            .map(|day| FreeDay {
                date: day.date.format("%Y-%m-%d").to_string(),
                windows: day
                    .windows
                    .into_iter()
                    .map(|(start, end)| FreeWindow {
                        starts_at: utc_text(start),
                        ends_at: utc_text(end),
                    })
                    .collect(),
            })
            .collect(),
        unknown,
    })
}

fn check_range(start: DateTime<Utc>, end: DateTime<Utc>) -> Result<(), AvailabilityError> {
    if end <= start || end - start > Duration::days(MAX_RANGE_DAYS) {
        return Err(AvailabilityError::Range);
    }
    Ok(())
}

fn utc_text(time: DateTime<Utc>) -> String {
    time.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

fn block_record(block: &Busy) -> BusyBlock {
    BusyBlock {
        starts_at: utc_text(block.start),
        ends_at: utc_text(block.end),
        tentative: block.tentative,
    }
}

/// Lower-cased, de-duplicated addresses in the order given, without the
/// owner's own (they are the "you" row) and without anything that is not one.
pub fn normalize_addresses(input: &[String], own: &[String]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    input
        .iter()
        .map(|value| value.trim().trim_matches(['<', '>']).to_ascii_lowercase())
        .filter(|value| {
            value.contains('@') && !own.iter().any(|mine| mine.eq_ignore_ascii_case(value))
        })
        .filter(|value| seen.insert(value.clone()))
        .take(MAX_ATTENDEES)
        .collect()
}

/// Which provider answers for an account's attendees.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    Google,
    Outlook,
    CalDav,
    None,
}

type CacheKey = (String, String, i64, i64);

struct CacheEntry {
    at: Instant,
    ttl: StdDuration,
    busy: Option<Vec<Busy>>,
}

static CACHE: LazyLock<Mutex<HashMap<CacheKey, CacheEntry>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Forget every cached answer. Tests start from an empty cache.
pub fn clear_cache() {
    if let Ok(mut cache) = CACHE.lock() {
        cache.clear();
    }
}

/// Provider requests cover whole UTC days, so the editor's day steps and
/// "next free" from a moved slot reuse the same answers.
fn day_range(start: DateTime<Utc>, end: DateTime<Utc>) -> (DateTime<Utc>, DateTime<Utc>) {
    let day = 86_400;
    let from = start.timestamp().div_euclid(day) * day;
    let to = (end.timestamp() + day - 1).div_euclid(day) * day;
    (
        DateTime::from_timestamp(from, 0).unwrap_or(start),
        DateTime::from_timestamp(to, 0).unwrap_or(end),
    )
}

async fn attendee_busy(
    pool: &SqlitePool,
    participants: &Participants<'_>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<Vec<(String, Option<Vec<Busy>>)>, sqlx::Error> {
    let own: Vec<String> = sqlx::query_scalar("SELECT email FROM accounts")
        .fetch_all(pool)
        .await?;
    let addresses = normalize_addresses(participants.attendees, &own);
    if addresses.is_empty() {
        return Ok(Vec::new());
    }
    let Some(account_id) = participants.account_id.filter(|id| !id.is_empty()) else {
        return Ok(addresses
            .into_iter()
            .map(|address| (address, None))
            .collect());
    };
    let (from, to) = day_range(start, end);
    let key = |address: &str| {
        (
            account_id.to_string(),
            address.to_string(),
            from.timestamp(),
            to.timestamp(),
        )
    };

    let mut answers: HashMap<String, Option<Vec<Busy>>> = HashMap::new();
    if let Ok(mut cache) = CACHE.lock() {
        cache.retain(|_, entry| entry.at.elapsed() < entry.ttl);
        for address in &addresses {
            if let Some(entry) = cache.get(&key(address)) {
                answers.insert(address.clone(), entry.busy.clone());
            }
        }
    }
    let missing = addresses
        .iter()
        .filter(|address| !answers.contains_key(*address))
        .cloned()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        let (fetched, ttl) = match ask_provider(pool, account_id, &missing, from, to).await {
            Ok(known) => (known, ANSWER_TTL),
            Err(error) => {
                tracing::warn!(account_id, %error, "free/busy lookup failed; attendees shown as unknown");
                (HashMap::new(), FAILURE_TTL)
            }
        };
        let mut cache = CACHE.lock().ok();
        for address in missing {
            let busy = fetched.get(&address).cloned();
            if let Some(cache) = cache.as_mut() {
                cache.insert(
                    key(&address),
                    CacheEntry {
                        at: Instant::now(),
                        ttl,
                        busy: busy.clone(),
                    },
                );
            }
            answers.insert(address, busy);
        }
    }
    Ok(addresses
        .into_iter()
        .map(|address| {
            let busy = answers.remove(&address).flatten().map(|blocks| {
                blocks
                    .into_iter()
                    .filter(|block| block.end > start && block.start < end)
                    .collect()
            });
            (address, busy)
        })
        .collect())
}

async fn source_of(pool: &SqlitePool, account_id: &str) -> Result<Source, String> {
    let provider: Option<String> = sqlx::query_scalar("SELECT provider FROM accounts WHERE id = ?")
        .bind(account_id)
        .fetch_optional(pool)
        .await
        .map_err(|error| error.to_string())?;
    let Some(provider) = provider else {
        return Ok(Source::None);
    };
    if provider == crate::utils::oauth::OAuthProvider::Gmail.as_str() {
        return Ok(Source::Google);
    }
    if provider == crate::utils::oauth::OAuthProvider::Outlook.as_str() {
        return Ok(Source::Outlook);
    }
    let connection = crate::db::accounts::AccountRepository::new(pool)
        .calendar_connection(account_id)
        .await
        .map_err(|error| error.to_string())?;
    Ok(if crate::providers::dav::has_caldav(&connection.config) {
        Source::CalDav
    } else {
        Source::None
    })
}

/// The busy time of every address the provider answered for. An address
/// missing from the map is unknown.
async fn ask_provider(
    pool: &SqlitePool,
    account_id: &str,
    addresses: &[String],
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<HashMap<String, Vec<Busy>>, String> {
    match source_of(pool, account_id).await? {
        Source::Google => {
            crate::providers::google_calendar::free_busy(pool, account_id, addresses, start, end)
                .await
        }
        Source::Outlook => {
            crate::providers::outlook::calendar::schedules(pool, account_id, addresses, start, end)
                .await
        }
        Source::CalDav => {
            let connection = crate::db::accounts::AccountRepository::new(pool)
                .calendar_connection(account_id)
                .await
                .map_err(|error| error.to_string())?;
            crate::providers::dav::free_busy(
                &connection.config,
                &connection.email,
                addresses,
                start,
                end,
            )
            .await
        }
        Source::None => Ok(HashMap::new()),
    }
}

#[cfg(test)]
mod tests;
