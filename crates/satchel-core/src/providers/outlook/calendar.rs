//! Outlook calendars over Microsoft Graph.
//!
//! Mirrors every calendar of an Outlook account into `calendar_events`
//! (`source = 'outlook'`), the way `providers::google_calendar` mirrors
//! Google's. Recurring series arrive expanded, one row per occurrence, because
//! the mirror reads each calendar's `calendarView` — the same shape Google's
//! `singleEvents=true` gives.
//!
//! The default calendar is mirrored incrementally through the one delta Graph
//! v1.0 documents, `/me/calendarView/delta`: its delta link is kept in
//! `calendar_sources.sync_cursor`, and a pass only applies what changed since.
//! A delta link is bound to the window it was started with, so the mirror
//! starts over once that window has drifted a week, and whenever Graph
//! declares the link expired. Every other calendar — and the default one, if
//! Graph refuses its delta — is listed whole on every pass, as Google's is. A
//! calendar Graph will not show (403/404: birthdays, revoked shares) is
//! skipped, not reported as the account failing.
//!
//! Writes go to Graph first and are mirrored from Graph's answer: an edit the
//! server refused never shows up locally as if it had worked. Graph v1.0 does
//! not document `If-Match` for events, so a write first reads the event's
//! `changeKey` and goes ahead only if it is the one the mirror last saw.

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sqlx::SqlitePool;

use super::batch::{send_throttled, send_waiting_out_throttling};
use crate::db::calendar::{
    CalendarRepository, EventFlags, OutlookCalendarDelta, OutlookCalendarEvent,
};
use crate::sync::errors::{http_failure, SyncError, SyncResult};
use crate::utils::oauth::{
    access_token_from_account_config, outlook_scope_granted, OAuthProvider, OUTLOOK_CALENDAR_SCOPE,
};

/// How far back and forward the mirror reaches; Google's window.
const WINDOW_PAST_DAYS: i64 = 90;
const WINDOW_FUTURE_DAYS: i64 = 365;
/// A delta link keeps the window it started with. Past this drift the mirror
/// starts over, so the far end keeps reaching a year ahead.
const WINDOW_DRIFT_DAYS: i64 = 7;
/// Page size: a header for the delta, which ignores `$top`; `$top` for a listing.
const PAGE_SIZE: u32 = 100;
/// The fields a listing asks for. `body` stays: the mirror shows the
/// description and must know whether it is formatted.
const LISTING_SELECT: &str = "id,iCalUId,changeKey,subject,body,location,start,end,isAllDay,\
isCancelled,attendees,isOrganizer,type,showAs,responseStatus";
/// Same ids as mail: an event moved between calendars keeps its id.
const PREFER_IMMUTABLE_IDS: &str = "IdType=\"ImmutableId\"";
/// Times come back as UTC, whatever zone the event was created in.
const PREFER_UTC: &str = "outlook.timezone=\"UTC\"";

#[cfg(test)]
thread_local! {
    static TEST_API_BASE: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

fn api_base() -> String {
    #[cfg(test)]
    if let Some(base) = TEST_API_BASE.with(|base| base.borrow().clone()) {
        return base;
    }
    super::graph_api_base()
}

#[derive(Debug, Deserialize)]
struct Page<T> {
    #[serde(default = "Vec::new")]
    value: Vec<T>,
    #[serde(rename = "@odata.nextLink")]
    next_link: Option<String>,
    #[serde(rename = "@odata.deltaLink")]
    delta_link: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct GraphCalendar {
    id: String,
    name: Option<String>,
    #[serde(rename = "isDefaultCalendar", default)]
    is_default: bool,
    /// False for read-only shares and subscribed calendars.
    #[serde(rename = "canEdit")]
    can_edit: Option<bool>,
}

impl GraphCalendar {
    fn can_edit(&self) -> bool {
        self.can_edit.unwrap_or(true)
    }
}

#[derive(Debug, Deserialize)]
struct GraphEvent {
    id: String,
    /// Delta tombstone: `{"id": …, "@removed": {"reason": "deleted"}}`.
    #[serde(rename = "@removed")]
    removed: Option<Value>,
    #[serde(rename = "changeKey")]
    change_key: Option<String>,
    #[serde(rename = "iCalUId")]
    ical_uid: Option<String>,
    subject: Option<String>,
    body: Option<GraphBody>,
    location: Option<GraphLocation>,
    start: Option<GraphDateTime>,
    end: Option<GraphDateTime>,
    #[serde(rename = "isAllDay", default)]
    is_all_day: bool,
    #[serde(rename = "isCancelled", default)]
    is_cancelled: bool,
    #[serde(default)]
    attendees: Vec<GraphAttendee>,
    /// Only the organizer may change a meeting; an attendee's copy is theirs
    /// to answer, not to edit. Absent counts as not organizing.
    #[serde(rename = "isOrganizer")]
    is_organizer: Option<bool>,
    /// `singleInstance`, `occurrence`, `exception` or `seriesMaster`.
    #[serde(rename = "type")]
    event_type: Option<String>,
    /// `free`, `tentative`, `busy`, `oof`, `workingElsewhere` or `unknown`.
    /// Absent counts as busy, Graph's default.
    #[serde(rename = "showAs")]
    show_as: Option<String>,
    /// The account's own answer to a meeting it attends.
    #[serde(rename = "responseStatus")]
    response_status: Option<GraphResponseStatus>,
}

#[derive(Debug, Deserialize)]
struct GraphResponseStatus {
    /// `none`, `organizer`, `tentativelyAccepted`, `accepted`, `declined`, …
    response: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GraphBody {
    #[serde(rename = "contentType")]
    content_type: Option<String>,
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GraphLocation {
    #[serde(rename = "displayName")]
    display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GraphDateTime {
    #[serde(rename = "dateTime")]
    date_time: String,
    #[serde(rename = "timeZone")]
    time_zone: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GraphAttendee {
    #[serde(rename = "emailAddress")]
    email_address: Option<GraphEmailAddress>,
}

#[derive(Debug, Deserialize)]
struct GraphEmailAddress {
    address: Option<String>,
}

/// An event as the mirror stores it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirroredEvent {
    pub id: String,
    pub remote_id: String,
    pub change_key: Option<String>,
    pub flags: EventFlags,
    pub uid: String,
    pub title: String,
    pub description: Option<String>,
    pub location: Option<String>,
    /// RFC3339 UTC for timed events, `YYYY-MM-DD` for all-day ones.
    pub starts_at: String,
    pub ends_at: String,
    pub all_day: bool,
    pub attendees_json: String,
}

impl MirroredEvent {
    pub fn as_row(&self) -> OutlookCalendarEvent<'_> {
        OutlookCalendarEvent {
            id: &self.id,
            remote_id: &self.remote_id,
            change_key: self.change_key.as_deref(),
            flags: self.flags,
            uid: &self.uid,
            title: &self.title,
            description: self.description.as_deref(),
            location: self.location.as_deref(),
            starts_at: &self.starts_at,
            ends_at: &self.ends_at,
            all_day: self.all_day,
            attendees_json: &self.attendees_json,
        }
    }
}

/// The cursor stored per calendar.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct DeltaCursor {
    delta_link: String,
    /// Start of the window the delta link was opened with.
    window_start: String,
}

/// Whether this account config is an Outlook sign-in that granted calendar
/// access. Mail-only Outlook accounts, and every other provider, are not.
pub fn calendar_granted(config_json: &str) -> bool {
    crate::utils::oauth::load_stored_oauth(config_json)
        .ok()
        .and_then(|stored| stored.provider)
        .is_some_and(|provider| provider.eq_ignore_ascii_case(OAuthProvider::Outlook.as_str()))
        && outlook_scope_granted(config_json, OUTLOOK_CALENDAR_SCOPE)
}

/// The calendar grant of the stored Outlook row, read before any refresh.
async fn stored_calendar_grant(pool: &SqlitePool, account_id: &str) -> Result<bool, String> {
    let stored: Option<String> =
        sqlx::query_scalar("SELECT config FROM accounts WHERE id = ? AND provider = ?")
            .bind(account_id)
            .bind(OAuthProvider::Outlook.as_str())
            .fetch_optional(pool)
            .await
            .map_err(|err| SyncError::Transient(err.to_string()).encoded())?;
    let Some(stored) = stored else {
        return Ok(false);
    };
    let config = crate::accounts::decrypt_account_config(account_id, &stored)
        .map_err(|err| SyncError::Permanent(err).encoded())?;
    Ok(outlook_scope_granted(&config, OUTLOOK_CALENDAR_SCOPE))
}

/// A Graph session for one account: client, base URL and a fresh token.
struct Graph {
    client: reqwest::Client,
    base: String,
    token: String,
}

impl Graph {
    /// `None` when the account has not granted calendar access — after the
    /// refresh, which may have added or withdrawn it.
    async fn connect(pool: &SqlitePool, account_id: &str) -> SyncResult<Option<Self>> {
        let config = crate::utils::oauth::fresh_account_config_for_provider(
            pool,
            account_id,
            OAuthProvider::Outlook,
        )
        .await?;
        if !outlook_scope_granted(&config, OUTLOOK_CALENDAR_SCOPE) {
            return Ok(None);
        }
        let token = access_token_from_account_config(OAuthProvider::Outlook, &config).await?;
        Ok(Some(Self {
            client: crate::utils::http::client(),
            base: api_base(),
            token,
        }))
    }

    fn url(&self, segments: &[&str]) -> SyncResult<reqwest::Url> {
        let mut url = reqwest::Url::parse(&self.base)
            .map_err(|err| SyncError::Permanent(format!("invalid Graph base: {err}")))?;
        url.path_segments_mut()
            .map_err(|_| SyncError::Permanent("invalid Graph base".into()))?
            .pop_if_empty()
            .extend(segments);
        Ok(url)
    }

    /// Graph's paging links are absolute. The bearer token goes nowhere but
    /// the Graph origin, under the API's own path.
    fn follow(&self, link: &str) -> SyncResult<reqwest::Url> {
        let leaves = || {
            SyncError::Permanent(format!(
                "outlook calendar link leaves the Graph API: {link}"
            ))
        };
        let base = reqwest::Url::parse(&self.base)
            .map_err(|err| SyncError::Permanent(format!("invalid Graph base: {err}")))?;
        let url = reqwest::Url::parse(link).map_err(|_| leaves())?;
        if url.origin() != base.origin() {
            return Err(leaves());
        }
        let root = base.path().trim_end_matches('/');
        let path = url.path();
        if !(path == root || path.starts_with(&format!("{root}/"))) {
            return Err(leaves());
        }
        Ok(url)
    }

    fn request(&self, method: reqwest::Method, url: reqwest::Url) -> reqwest::RequestBuilder {
        self.request_preferring(method, url, None)
    }

    /// One `Prefer` header carrying every preference, comma-separated.
    fn request_preferring(
        &self,
        method: reqwest::Method,
        url: reqwest::Url,
        extra: Option<&str>,
    ) -> reqwest::RequestBuilder {
        let mut prefer = format!("{PREFER_IMMUTABLE_IDS}, {PREFER_UTC}");
        if let Some(extra) = extra {
            prefer.push_str(", ");
            prefer.push_str(extra);
        }
        self.client
            .request(method, url)
            .bearer_auth(&self.token)
            .header("Prefer", prefer)
    }

    async fn calendars(&self) -> SyncResult<Vec<GraphCalendar>> {
        let mut url = self.url(&["me", "calendars"])?;
        url.query_pairs_mut()
            .append_pair("$select", "id,name,isDefaultCalendar,canEdit");
        let mut calendars = Vec::new();
        loop {
            let response = send_throttled(
                || self.request(reqwest::Method::GET, url.clone()),
                "calendar list",
            )
            .await?;
            let page: Page<GraphCalendar> = parse(response, "calendar list").await?;
            calendars.extend(page.value);
            match page.next_link {
                Some(next) => url = self.follow(&next)?,
                None => break,
            }
        }
        // The default calendar first: it is where new events go.
        calendars.sort_by_key(|calendar| !calendar.is_default);
        Ok(calendars)
    }

    async fn default_calendar_id(&self) -> SyncResult<String> {
        let mut url = self.url(&["me", "calendar"])?;
        url.query_pairs_mut().append_pair("$select", "id");
        let response = send_throttled(
            || self.request(reqwest::Method::GET, url.clone()),
            "default calendar",
        )
        .await?;
        let calendar: GraphCalendar = parse(response, "default calendar").await?;
        Ok(calendar.id)
    }
}

async fn parse<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
    label: &str,
) -> SyncResult<T> {
    response
        .json()
        .await
        .map_err(|err| SyncError::Transient(format!("outlook {label} parse failed: {err}")))
}

/// Sync every calendar of one Outlook account. Returns the number of events
/// now mirrored locally. An account without calendar access is skipped before
/// its credentials are refreshed: that failure belongs to mail sync.
pub async fn sync_account(pool: &SqlitePool, account_id: &str) -> Result<usize, String> {
    if !stored_calendar_grant(pool, account_id).await? {
        return Ok(0);
    }
    let Some(graph) = Graph::connect(pool, account_id)
        .await
        .map_err(|err| err.encoded())?
    else {
        return Ok(0);
    };
    sync_with(pool, account_id, &graph, chrono::Utc::now())
        .await
        .map_err(|err| err.encoded())
}

async fn sync_with(
    pool: &SqlitePool,
    account_id: &str,
    graph: &Graph,
    now: chrono::DateTime<chrono::Utc>,
) -> SyncResult<usize> {
    let repository = CalendarRepository::new(pool);
    let calendars = graph.calendars().await?;
    let listed = calendars
        .iter()
        .map(|calendar| calendar.id.as_str())
        .collect::<Vec<_>>();
    repository
        .retain_outlook_calendars(account_id, &listed)
        .await
        .map_err(persistence)?;
    let cursors = repository
        .outlook_cursors(account_id)
        .await
        .map_err(persistence)?;
    // One calendar failing must not keep the others stale; its error is
    // reported once the rest are done.
    let mut first_error = None;
    for calendar in &calendars {
        let cursor = cursors
            .iter()
            .find(|cursor| cursor.remote_id == calendar.id)
            .and_then(|cursor| cursor.sync_cursor.as_deref())
            .and_then(|raw| serde_json::from_str::<DeltaCursor>(raw).ok())
            .filter(|cursor| window_is_current(cursor, now));
        if let Err(error) = sync_calendar(pool, account_id, graph, calendar, cursor, now).await {
            tracing::warn!(account_id, %error, "outlook calendar sync failed");
            first_error.get_or_insert(error);
        }
    }
    if let Some(error) = first_error {
        return Err(error);
    }
    let count = repository
        .count_events(account_id, "outlook")
        .await
        .map_err(persistence)?;
    Ok(usize::try_from(count).unwrap_or_default())
}

fn persistence(error: crate::db::calendar::CalendarError) -> SyncError {
    SyncError::Transient(error.to_string())
}

fn window_is_current(cursor: &DeltaCursor, now: chrono::DateTime<chrono::Utc>) -> bool {
    chrono::DateTime::parse_from_rfc3339(&cursor.window_start).is_ok_and(|start| {
        now.signed_duration_since(start)
            <= chrono::Duration::days(WINDOW_PAST_DAYS + WINDOW_DRIFT_DAYS)
    })
}

/// What one pass returned: changed events, removed ids and, for a delta, the
/// link the next pass resumes at.
struct Pass {
    changed: Vec<MirroredEvent>,
    removed: Vec<String>,
    delta_link: Option<String>,
}

enum PassOutcome {
    Done(Pass),
    /// Graph no longer knows the delta link; only a pass from scratch helps.
    Expired,
    /// Graph refused the first request of a fresh delta: no delta here.
    Unsupported,
    /// Graph will not show this calendar to the account (403/404).
    Inaccessible(u16),
}

/// How a pass reads its pages.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PassKind {
    /// Following a stored delta link.
    Resume,
    /// Opening a new delta enumeration of the default calendar.
    FreshDelta,
    /// A plain `calendarView` listing: everything, no delta link.
    Listing,
}

async fn sync_calendar(
    pool: &SqlitePool,
    account_id: &str,
    graph: &Graph,
    calendar: &GraphCalendar,
    cursor: Option<DeltaCursor>,
    now: chrono::DateTime<chrono::Utc>,
) -> SyncResult<()> {
    let mut incremental = None;
    if let Some(cursor) = &cursor {
        let link = graph.follow(&cursor.delta_link)?;
        match run_pass(account_id, graph, calendar, link, PassKind::Resume).await? {
            PassOutcome::Done(pass) => incremental = Some((pass, cursor.window_start.clone())),
            PassOutcome::Inaccessible(status) => return skip_inaccessible(account_id, status),
            PassOutcome::Expired | PassOutcome::Unsupported => {
                tracing::info!(
                    account_id,
                    "outlook calendar delta link expired; resyncing from scratch"
                );
            }
        }
    }
    let (pass, window_start, complete) = match incremental {
        Some((pass, window_start)) => (pass, window_start, false),
        None => match fresh_pass(account_id, graph, calendar, now).await? {
            Ok(pass) => {
                let start = now - chrono::Duration::days(WINDOW_PAST_DAYS);
                (pass, start.to_rfc3339(), true)
            }
            Err(status) => return skip_inaccessible(account_id, status),
        },
    };
    let cursor = pass
        .delta_link
        .as_ref()
        .map(|delta_link| {
            serde_json::to_string(&DeltaCursor {
                delta_link: delta_link.clone(),
                window_start,
            })
        })
        .transpose()
        .map_err(|err| SyncError::Permanent(err.to_string()))?;
    let rows = pass
        .changed
        .iter()
        .map(MirroredEvent::as_row)
        .collect::<Vec<_>>();
    let removed = pass.removed.iter().map(String::as_str).collect::<Vec<_>>();
    CalendarRepository::new(pool)
        .apply_outlook_delta(
            account_id,
            OutlookCalendarDelta {
                calendar_id: &calendar.id,
                calendar_name: calendar_name(calendar),
                changed: &rows,
                removed: &removed,
                complete,
                cursor: cursor.as_deref(),
            },
        )
        .await
        .map_err(persistence)
}

/// Birthdays, holiday subscriptions and withdrawn shares answer 403/404 on
/// every pass. Such a calendar keeps what it had; it does not fail the account.
fn skip_inaccessible(account_id: &str, status: u16) -> SyncResult<()> {
    tracing::info!(
        account_id,
        status,
        "outlook calendar is not readable for this account; skipped"
    );
    Ok(())
}

/// Enumerate a calendar from scratch: the default calendar through its
/// delta, every other one (and a default one whose delta Graph refuses) as a
/// listing. `Err` carries the status of a calendar Graph will not show.
async fn fresh_pass(
    account_id: &str,
    graph: &Graph,
    calendar: &GraphCalendar,
    now: chrono::DateTime<chrono::Utc>,
) -> SyncResult<Result<Pass, u16>> {
    let start = graph_time(now - chrono::Duration::days(WINDOW_PAST_DAYS));
    let end = graph_time(now + chrono::Duration::days(WINDOW_FUTURE_DAYS));
    let windowed = |segments: &[&str]| -> SyncResult<reqwest::Url> {
        let mut url = graph.url(segments)?;
        url.query_pairs_mut()
            .append_pair("startDateTime", &start)
            .append_pair("endDateTime", &end);
        Ok(url)
    };
    if calendar.is_default {
        let delta = windowed(&["me", "calendarView", "delta"])?;
        match run_pass(account_id, graph, calendar, delta, PassKind::FreshDelta).await? {
            PassOutcome::Done(pass) => return Ok(Ok(pass)),
            PassOutcome::Inaccessible(status) => return Ok(Err(status)),
            PassOutcome::Unsupported | PassOutcome::Expired => tracing::info!(
                account_id,
                "outlook calendar delta unavailable; listing the calendar"
            ),
        }
    }
    let mut listing = windowed(&["me", "calendars", &calendar.id, "calendarView"])?;
    listing
        .query_pairs_mut()
        .append_pair("$top", &PAGE_SIZE.to_string())
        .append_pair("$select", LISTING_SELECT);
    match run_pass(account_id, graph, calendar, listing, PassKind::Listing).await? {
        PassOutcome::Done(pass) => Ok(Ok(pass)),
        PassOutcome::Inaccessible(status) => Ok(Err(status)),
        PassOutcome::Unsupported | PassOutcome::Expired => Err(SyncError::Transient(
            "outlook calendar listing ended without an answer".into(),
        )),
    }
}

fn calendar_name(calendar: &GraphCalendar) -> &str {
    calendar
        .name
        .as_deref()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or(&calendar.id)
}

fn graph_time(time: chrono::DateTime<chrono::Utc>) -> String {
    time.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

async fn run_pass(
    account_id: &str,
    graph: &Graph,
    calendar: &GraphCalendar,
    first: reqwest::Url,
    kind: PassKind,
) -> SyncResult<PassOutcome> {
    let mut url = first;
    let mut first_page = true;
    // Keyed by Graph id: a later page may report the same event again, and
    // the last word counts.
    let mut changed: std::collections::HashMap<String, MirroredEvent> = Default::default();
    let mut removed: std::collections::HashSet<String> = Default::default();
    let page_size = format!("odata.maxpagesize={PAGE_SIZE}");
    loop {
        let response = send_waiting_out_throttling(
            || {
                let extra = (kind != PassKind::Listing).then_some(page_size.as_str());
                graph.request_preferring(reqwest::Method::GET, url.clone(), extra)
            },
            "calendar pass",
        )
        .await?;
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            let body = response.text().await.unwrap_or_default();
            if let Some(outcome) = refusal(kind, first_page, status, &body) {
                return Ok(outcome);
            }
            return Err(http_failure("outlook", "calendar pass", status, &body));
        }
        let page: Page<GraphEvent> = parse(response, "calendar pass").await?;
        for event in page.value {
            if event.removed.is_some() || event.is_cancelled {
                changed.remove(&event.id);
                removed.insert(event.id);
                continue;
            }
            if let Some(mapped) = map_event(account_id, calendar.can_edit(), event) {
                removed.remove(&mapped.remote_id);
                changed.insert(mapped.remote_id.clone(), mapped);
            }
        }
        first_page = false;
        if let Some(next) = page.next_link {
            url = graph.follow(&next)?;
            continue;
        }
        let delta_link = match kind {
            PassKind::Listing => None,
            PassKind::Resume | PassKind::FreshDelta => Some(page.delta_link.ok_or_else(|| {
                SyncError::Transient("outlook calendar delta ended without a delta link".into())
            })?),
        };
        return Ok(PassOutcome::Done(Pass {
            changed: changed.into_values().collect(),
            removed: removed.into_iter().collect(),
            delta_link,
        }));
    }
}

/// What an error status means for a pass, if it means more than a failure.
fn refusal(kind: PassKind, first_page: bool, status: u16, body: &str) -> Option<PassOutcome> {
    match kind {
        PassKind::Resume if delta_link_expired(status, body) => Some(PassOutcome::Expired),
        PassKind::Resume if status == 403 => Some(PassOutcome::Inaccessible(status)),
        // A resumed link Graph no longer finds is a link to start over.
        PassKind::Resume if status == 404 => Some(PassOutcome::Expired),
        PassKind::FreshDelta if first_page && status == 403 => {
            Some(PassOutcome::Inaccessible(status))
        }
        // Not documented for this calendar, or not implemented: list instead.
        // 401 and 429 are the token's and the throttle's to answer.
        PassKind::FreshDelta
            if first_page
                && ((400..500).contains(&status) && status != 401 && status != 429
                    || status == 501) =>
        {
            Some(PassOutcome::Unsupported)
        }
        PassKind::Listing if matches!(status, 403 | 404) => Some(PassOutcome::Inaccessible(status)),
        _ => None,
    }
}

/// Graph answers an expired or unknown delta link with 410 Gone, and with a
/// `SyncState…`/`resyncRequired` error code on some deployments.
fn delta_link_expired(status: u16, body: &str) -> bool {
    status == 410
        || (matches!(status, 400 | 404)
            && ["syncstate", "resyncrequired"]
                .iter()
                .any(|code| body.to_ascii_lowercase().contains(code)))
}

/// Row ids are unique per account, not per calendar: with immutable ids an
/// event moved between calendars keeps its id and must keep its row.
pub fn row_id(account_id: &str, remote_id: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(format!("{account_id}|{remote_id}").as_bytes());
    let suffix = digest[..12]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("outlook-event-{suffix}")
}

/// Whether a body holds formatting the plain-text editor would lose. Fails
/// closed: a body is plain only if every tag is one of the structural few
/// below and no `div`, `p` or `span` carries a `style` — everything else
/// (links, images, tables, lists, fonts, emphasis, headings, code, `<style>`
/// blocks) keeps the description as Outlook has it.
fn is_rich_html(content: &str) -> bool {
    const STRUCTURAL: [&str; 8] = ["html", "head", "body", "meta", "div", "p", "span", "br"];
    let lower = content.to_ascii_lowercase();
    let mut rest = lower.as_str();
    while let Some(open) = rest.find('<') {
        rest = &rest[open + 1..];
        if let Some(comment) = rest.strip_prefix("!--") {
            let Some(end) = comment.find("-->") else {
                return true;
            };
            rest = &comment[end + 3..];
            continue;
        }
        let Some(close) = rest.find('>') else {
            return true;
        };
        let tag = &rest[..close];
        rest = &rest[close + 1..];
        if tag.starts_with("!doctype") {
            continue;
        }
        let tag = tag.strip_prefix('/').unwrap_or(tag);
        let name_end = tag
            .find(|c: char| c.is_ascii_whitespace() || c == '/')
            .unwrap_or(tag.len());
        let (name, attributes) = tag.split_at(name_end);
        if !STRUCTURAL.contains(&name) {
            return true;
        }
        if matches!(name, "div" | "p" | "span") && has_style_attribute(attributes) {
            return true;
        }
    }
    false
}

/// `style=`, allowing whitespace around the `=`, as a whole attribute name.
fn has_style_attribute(attributes: &str) -> bool {
    let mut rest = attributes;
    while let Some(at) = rest.find("style") {
        let before = rest[..at].chars().next_back();
        let after = rest[at + "style".len()..].trim_start();
        if before.is_none_or(|c| c.is_ascii_whitespace()) && after.starts_with('=') {
            return true;
        }
        rest = &rest[at + "style".len()..];
    }
    false
}

fn map_event(
    account_id: &str,
    calendar_can_edit: bool,
    event: GraphEvent,
) -> Option<MirroredEvent> {
    let start = event.start.as_ref()?;
    let all_day = event.is_all_day;
    let starts_at = normalize_time(start, all_day)?;
    let ends_at = event
        .end
        .as_ref()
        .and_then(|end| normalize_time(end, all_day))
        .unwrap_or_else(|| starts_at.clone());
    let mut rich_description = false;
    let description = event.body.and_then(|body| {
        let content = body.content?;
        let text = if body
            .content_type
            .as_deref()
            .is_some_and(|kind| kind.eq_ignore_ascii_case("html"))
        {
            rich_description = is_rich_html(&content);
            super::graph_html_body_to_text(&content)
        } else {
            content.trim().to_string()
        };
        Some(text).filter(|text| !text.is_empty())
    });
    let flags = EventFlags {
        read_only: !calendar_can_edit || event.is_organizer != Some(true),
        recurrence_instance: matches!(
            event.event_type.as_deref(),
            Some("occurrence" | "exception")
        ),
        rich_description,
        // A meeting the account declined stays on its calendar but takes no time.
        transparent: show_as_free(event.show_as.as_deref())
            || event
                .response_status
                .as_ref()
                .and_then(|status| status.response.as_deref())
                .is_some_and(|response| response.eq_ignore_ascii_case("declined")),
    };
    let attendees = event
        .attendees
        .into_iter()
        .filter_map(|attendee| attendee.email_address?.address)
        .map(|address| address.trim().to_string())
        .filter(|address| !address.is_empty())
        .collect::<Vec<_>>();
    Some(MirroredEvent {
        id: row_id(account_id, &event.id),
        uid: event
            .ical_uid
            .filter(|uid| !uid.trim().is_empty())
            .unwrap_or_else(|| event.id.clone()),
        remote_id: event.id,
        change_key: event.change_key,
        flags,
        // An empty title is the frontend's to name in the viewer's language.
        title: event
            .subject
            .map(|value| value.trim().to_string())
            .unwrap_or_default(),
        description,
        location: event
            .location
            .and_then(|location| location.display_name)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
        starts_at,
        ends_at,
        all_day,
        attendees_json: serde_json::to_string(&attendees).unwrap_or_else(|_| "[]".to_string()),
    })
}

/// `getSchedule` answers for at most this many people per request.
const SCHEDULE_BATCH: usize = 20;

#[derive(Debug, Deserialize)]
struct ScheduleInformation {
    #[serde(rename = "scheduleId")]
    schedule_id: String,
    #[serde(rename = "scheduleItems", default)]
    schedule_items: Vec<ScheduleItem>,
    /// Set when Graph will not say: an outside address, no permission.
    error: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct ScheduleItem {
    status: Option<String>,
    start: GraphDateTime,
    end: GraphDateTime,
}

/// Busy time of `addresses` from `/me/calendar/getSchedule`. The calendar
/// grant (`Calendars.ReadWrite`) covers it; an account without the grant
/// answers for nobody. People Graph will not answer for are left out, which
/// makes them unknown.
pub async fn schedules(
    pool: &SqlitePool,
    account_id: &str,
    addresses: &[String],
    start: chrono::DateTime<chrono::Utc>,
    end: chrono::DateTime<chrono::Utc>,
) -> Result<crate::calendar::free_busy::ProviderAnswers, String> {
    let mut answers = crate::calendar::free_busy::ProviderAnswers::default();
    if !stored_calendar_grant(pool, account_id).await? {
        return Ok(answers);
    }
    let Some(graph) = Graph::connect(pool, account_id)
        .await
        .map_err(|err| err.encoded())?
    else {
        return Ok(answers);
    };
    let url = graph
        .url(&["me", "calendar", "getSchedule"])
        .map_err(|err| err.encoded())?;
    let at = |time: chrono::DateTime<chrono::Utc>| json!({ "dateTime": time.format("%Y-%m-%dT%H:%M:%S").to_string(), "timeZone": "UTC" });
    // One chunk failing keeps what the others answered.
    for chunk in addresses.chunks(SCHEDULE_BATCH) {
        let body = json!({
            "schedules": chunk,
            "startTime": at(start),
            "endTime": at(end),
            "availabilityViewInterval": 30,
        });
        // Asked while someone waits in the editor: a throttled answer is
        // "unknown" now and asked again on the next refresh, not slept through.
        let response = match graph
            .request(reqwest::Method::POST, url.clone())
            .json(&body)
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) => {
                tracing::warn!(account_id, %error, "outlook schedule request failed");
                answers.failed.extend_from_slice(chunk);
                continue;
            }
        };
        let status = response.status();
        if !status.is_success() {
            // Personal Microsoft accounts have no getSchedule; asking again
            // every minute would not change that.
            tracing::warn!(
                account_id,
                status = status.as_u16(),
                "outlook schedule refused"
            );
            // Graph throttles with 429 alone, so a 403 is a lasting refusal.
            if crate::calendar::free_busy::is_refusal(status.as_u16(), true) {
                answers.refused.extend_from_slice(chunk);
            } else {
                answers.failed.extend_from_slice(chunk);
            }
            continue;
        }
        match parse::<Page<ScheduleInformation>>(response, "schedule").await {
            Ok(page) => answers.known.extend(map_schedules(page.value, chunk)),
            Err(error) => {
                tracing::warn!(account_id, %error, "outlook schedule unreadable");
                answers.failed.extend_from_slice(chunk);
            }
        }
    }
    Ok(answers)
}

fn map_schedules(
    schedules: Vec<ScheduleInformation>,
    asked: &[String],
) -> std::collections::HashMap<String, Vec<crate::calendar::availability::Busy>> {
    let instant = |time: &GraphDateTime| {
        let text = normalize_time(time, false)?;
        chrono::DateTime::parse_from_rfc3339(&text)
            .ok()
            .map(|time| time.with_timezone(&chrono::Utc))
    };
    let mut known = std::collections::HashMap::new();
    for schedule in schedules {
        let Some(address) = asked
            .iter()
            .find(|address| address.eq_ignore_ascii_case(&schedule.schedule_id))
        else {
            continue;
        };
        if schedule.error.is_some() {
            continue;
        }
        let busy = schedule
            .schedule_items
            .iter()
            .filter(|item| {
                !show_as_free(item.status.as_deref())
                    && !item
                        .status
                        .as_deref()
                        .is_some_and(|status| status.eq_ignore_ascii_case("unknown"))
            })
            .filter_map(|item| {
                Some(crate::calendar::availability::Busy {
                    start: instant(&item.start)?,
                    end: instant(&item.end)?,
                    tentative: item
                        .status
                        .as_deref()
                        .is_some_and(|status| status.eq_ignore_ascii_case("tentative")),
                })
            })
            .collect();
        known.insert(address.clone(), busy);
    }
    known
}

/// Whether Graph's `showAs` leaves the time free. Working elsewhere is
/// working: Outlook's scheduling assistant does not block it either.
pub(crate) fn show_as_free(show_as: Option<&str>) -> bool {
    show_as.is_some_and(|value| {
        value.eq_ignore_ascii_case("free") || value.eq_ignore_ascii_case("workingElsewhere")
    })
}

/// Graph writes `2026-07-14T16:00:00.0000000` plus a zone name. Requests ask
/// for UTC; a named IANA zone is honoured in case Graph ignored that.
fn normalize_time(time: &GraphDateTime, all_day: bool) -> Option<String> {
    let naive = chrono::NaiveDateTime::parse_from_str(&time.date_time, "%Y-%m-%dT%H:%M:%S%.f")
        .or_else(|_| chrono::NaiveDateTime::parse_from_str(&time.date_time, "%Y-%m-%dT%H:%M:%S"))
        .ok()?;
    if all_day {
        // An all-day boundary is a date. Converted to UTC it can land on the
        // evening before (a Berlin midnight is 22:00 UTC); the nearest
        // midnight is the date that was meant.
        let date = (naive + chrono::Duration::hours(12)).date();
        return Some(date.format("%Y-%m-%d").to_string());
    }
    let utc = match time
        .time_zone
        .as_deref()
        .filter(|zone| !matches!(*zone, "UTC" | "Etc/UTC" | "tzone://Microsoft/Utc"))
        .and_then(|zone| zone.parse::<chrono_tz::Tz>().ok())
    {
        Some(zone) => naive
            .and_local_timezone(zone)
            .earliest()?
            .with_timezone(&chrono::Utc),
        None => naive.and_utc(),
    };
    Some(utc.format("%Y-%m-%dT%H:%M:%SZ").to_string())
}

/// The fields of an event as the editor sends them.
#[derive(Debug, Clone)]
pub struct EventFields<'a> {
    pub title: &'a str,
    pub description: Option<&'a str>,
    pub location: Option<&'a str>,
    pub starts_at: chrono::DateTime<chrono::Utc>,
    pub ends_at: chrono::DateTime<chrono::Utc>,
    pub all_day: bool,
}

/// An event Graph returned for a write, with the calendar it lives in.
#[derive(Debug, Clone)]
pub struct WrittenEvent {
    pub calendar_id: String,
    pub event: MirroredEvent,
}

/// Why a write did not reach Graph.
#[derive(Debug, thiserror::Error)]
pub enum WriteError {
    #[error("outlook calendar access was not granted")]
    NotGranted,
    /// The event's `changeKey` is no longer the one the mirror saw.
    #[error("the event changed on the server")]
    Changed,
    /// The event no longer exists on the server.
    #[error("the event is gone on the server")]
    Gone,
    /// Graph refused the write: a meeting the account only attends, a
    /// read-only calendar.
    #[error("the server does not let this account change the event")]
    Forbidden,
    #[error(transparent)]
    Remote(#[from] SyncError),
}

fn time_json(time: chrono::DateTime<chrono::Utc>, all_day: bool) -> Value {
    let date_time = if all_day {
        time.format("%Y-%m-%dT00:00:00").to_string()
    } else {
        time.format("%Y-%m-%dT%H:%M:%S").to_string()
    };
    json!({ "dateTime": date_time, "timeZone": "UTC" })
}

fn text_body(description: Option<&str>) -> Value {
    json!({ "contentType": "text", "content": description.unwrap_or_default() })
}

fn location_json(location: Option<&str>) -> Value {
    json!({ "displayName": location.unwrap_or_default() })
}

/// The POST body of a new event. Attendees are listed only when invitations
/// are wanted: Exchange mails an invitation to everyone on the list. The
/// `transactionId` makes Graph answer a repeated POST — a throttling retry, a
/// response lost on the way back — with the event it already created.
fn create_body(fields: &EventFields<'_>, invite: &[String], transaction_id: &str) -> Value {
    let mut body = json!({
        "transactionId": transaction_id,
        "subject": fields.title,
        "body": text_body(fields.description),
        "start": time_json(fields.starts_at, fields.all_day),
        "end": time_json(fields.ends_at, fields.all_day),
        "isAllDay": fields.all_day,
    });
    if fields
        .location
        .is_some_and(|value| !value.trim().is_empty())
    {
        body["location"] = location_json(fields.location);
    }
    if !invite.is_empty() {
        body["attendees"] = Value::Array(
            invite
                .iter()
                .map(
                    |address| json!({ "emailAddress": { "address": address }, "type": "required" }),
                )
                .collect(),
        );
    }
    body
}

/// Empty and whitespace-only text both mean "none".
fn blank(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

/// The PATCH body: only what the editor changed. The stored description is a
/// text rendering of the body; it is sent back only when the editor changed
/// it, and never for a formatted body, which text would flatten.
pub fn patch_body(previous: &MirroredEventView<'_>, fields: &EventFields<'_>) -> Value {
    let mut patch = Map::new();
    if fields.title != previous.title {
        patch.insert("subject".into(), json!(fields.title));
    }
    if !previous.rich_description && blank(fields.description) != blank(previous.description) {
        patch.insert("body".into(), text_body(fields.description));
    }
    if blank(fields.location) != blank(previous.location) {
        patch.insert("location".into(), location_json(fields.location));
    }
    let start = time_json(fields.starts_at, fields.all_day);
    let end = time_json(fields.ends_at, fields.all_day);
    if fields.all_day != previous.all_day
        || start_text(fields.starts_at, fields.all_day) != previous.starts_at
        || start_text(fields.ends_at, fields.all_day) != previous.ends_at
    {
        patch.insert("start".into(), start);
        patch.insert("end".into(), end);
        patch.insert("isAllDay".into(), json!(fields.all_day));
    }
    Value::Object(patch)
}

/// The stored row, as far as an edit compares against it.
#[derive(Debug, Clone)]
pub struct MirroredEventView<'a> {
    pub title: &'a str,
    pub description: Option<&'a str>,
    pub location: Option<&'a str>,
    pub starts_at: &'a str,
    pub ends_at: &'a str,
    pub all_day: bool,
    pub rich_description: bool,
}

/// Whether the editor changed a formatted description it cannot write back.
pub fn changes_rich_description(
    previous: &MirroredEventView<'_>,
    fields: &EventFields<'_>,
) -> bool {
    previous.rich_description && blank(fields.description) != blank(previous.description)
}

/// The stored representation of a time, as `write_event` keeps it.
fn start_text(time: chrono::DateTime<chrono::Utc>, all_day: bool) -> String {
    if all_day {
        time.format("%Y-%m-%d").to_string()
    } else {
        time.format("%Y-%m-%dT%H:%M:%SZ").to_string()
    }
}

async fn connect_for_write(pool: &SqlitePool, account_id: &str) -> Result<Graph, WriteError> {
    Graph::connect(pool, account_id)
        .await?
        .ok_or(WriteError::NotGranted)
}

/// Create an event in the account's default calendar.
pub async fn create_event(
    pool: &SqlitePool,
    account_id: &str,
    fields: &EventFields<'_>,
    invite: &[String],
) -> Result<WrittenEvent, WriteError> {
    let graph = connect_for_write(pool, account_id).await?;
    let calendar_id = graph.default_calendar_id().await?;
    let url = graph.url(&["me", "calendars", &calendar_id, "events"])?;
    let body = create_body(fields, invite, &uuid::Uuid::new_v4().to_string());
    let response = send_waiting_out_throttling(
        || {
            graph
                .request(reqwest::Method::POST, url.clone())
                .json(&body)
        },
        "calendar create",
    )
    .await?;
    let response = accepted(response, "calendar create").await?;
    written(account_id, calendar_id, response, "calendar create").await
}

/// The `changeKey` on the server, before a write. `Ok(false)`: the event is gone.
async fn require_change_key(
    graph: &Graph,
    remote_id: &str,
    expected: Option<&str>,
) -> Result<bool, WriteError> {
    #[derive(Deserialize)]
    struct Current {
        #[serde(rename = "changeKey")]
        change_key: Option<String>,
    }
    let mut url = graph.url(&["me", "events", remote_id])?;
    url.query_pairs_mut().append_pair("$select", "changeKey");
    let response = send_waiting_out_throttling(
        || graph.request(reqwest::Method::GET, url.clone()),
        "calendar change key",
    )
    .await?;
    if response.status().as_u16() == 404 {
        return Ok(false);
    }
    let response = accepted(response, "calendar change key").await?;
    let current: Current = parse(response, "calendar change key").await?;
    // A mirror row without a change key cannot prove it is current.
    match (expected, current.change_key.as_deref()) {
        (Some(expected), Some(current)) if expected == current => Ok(true),
        _ => Err(WriteError::Changed),
    }
}

/// A 2xx answer, or the reason for anything else.
async fn accepted(
    response: reqwest::Response,
    label: &str,
) -> Result<reqwest::Response, WriteError> {
    let status = response.status().as_u16();
    match status {
        200..=299 => Ok(response),
        403 => Err(WriteError::Forbidden),
        404 => Err(WriteError::Gone),
        _ => {
            let body = response.text().await.unwrap_or_default();
            Err(http_failure("outlook", label, status, &body).into())
        }
    }
}

/// Apply `patch` to one event, if it is still the version the mirror saw.
pub async fn update_event(
    pool: &SqlitePool,
    account_id: &str,
    calendar_id: &str,
    remote_id: &str,
    change_key: Option<&str>,
    patch: &Value,
) -> Result<WrittenEvent, WriteError> {
    let graph = connect_for_write(pool, account_id).await?;
    if !require_change_key(&graph, remote_id, change_key).await? {
        return Err(WriteError::Gone);
    }
    let url = graph.url(&["me", "events", remote_id])?;
    let response = send_waiting_out_throttling(
        || {
            graph
                .request(reqwest::Method::PATCH, url.clone())
                .json(patch)
        },
        "calendar update",
    )
    .await?;
    let response = accepted(response, "calendar update").await?;
    written(
        account_id,
        calendar_id.to_string(),
        response,
        "calendar update",
    )
    .await
}

/// Delete one event (one occurrence, for a recurring series), if it is still
/// the version the mirror saw. An event that is already gone counts as deleted.
pub async fn delete_event(
    pool: &SqlitePool,
    account_id: &str,
    remote_id: &str,
    change_key: Option<&str>,
) -> Result<(), WriteError> {
    let graph = connect_for_write(pool, account_id).await?;
    if !require_change_key(&graph, remote_id, change_key).await? {
        return Ok(());
    }
    let url = graph.url(&["me", "events", remote_id])?;
    let response = send_waiting_out_throttling(
        || graph.request(reqwest::Method::DELETE, url.clone()),
        "calendar delete",
    )
    .await?;
    match accepted(response, "calendar delete").await {
        Ok(_) | Err(WriteError::Gone) => Ok(()),
        Err(error) => Err(error),
    }
}

async fn written(
    account_id: &str,
    calendar_id: String,
    response: reqwest::Response,
    label: &str,
) -> Result<WrittenEvent, WriteError> {
    let event: GraphEvent = parse(response, label).await?;
    // Graph just accepted this account's write to the calendar.
    let event = map_event(account_id, true, event).ok_or_else(|| {
        SyncError::Transient(format!("outlook {label} returned an event without a start"))
    })?;
    Ok(WrittenEvent { calendar_id, event })
}

#[cfg(test)]
mod tests;
