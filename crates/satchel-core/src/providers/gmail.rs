use crate::{
    providers::{
        cursor,
        traits::{
            FetchedAttachment, FlagUpdate, MailProvider, MessageEnvelope, MessageRef,
            OutgoingMessage, RemoteAttachment, RemoteFolder, RemoteMessageRemoval,
            RemoteMessageState, SyncBatch, SyncCheckpoint,
        },
    },
    utils::{
        mime::{
            decode_bytes_with_declared_charset, decode_html_entities, decode_mime_encoded_words,
            extract_email_address, parse_address_list_tokens, AddressListDecodeStrategy,
        },
        oauth::{access_token_from_account_config, OAuthProvider},
    },
};
use base64::Engine;
use reqwest::Url;
use serde::{Deserialize, Serialize};

const GMAIL_API_BASE_DEFAULT: &str = "https://gmail.googleapis.com/gmail/v1/users/me";

/// Base URL of the Gmail API.
///
/// Debug builds read a per-call override for local provider fixtures.
fn gmail_api_base() -> String {
    crate::utils::environment::debug_env("SATCHEL_GMAIL_API_BASE")
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| GMAIL_API_BASE_DEFAULT.to_string())
}

/// Gmail's per-user budget in quota units per second.
///
/// The reference documents 6,000 units per minute per user. That is the figure
/// used here rather than the larger per-project ceiling, because the block this
/// guards against was a per-user one ("User-rate limit exceeded").
const QUOTA_UNITS_PER_SECOND: f64 = 100.0;

/// How far the pacer may run ahead of the steady rate.
///
/// Gmail measures on a moving average and tolerates short bursts, so a strict
/// one-request-at-a-time drip would be needlessly slow for the small, spread-out
/// calls an incremental sync makes.
const QUOTA_BURST_UNITS: f64 = 100.0;

/// Documented per-method costs. These are not uniform — `messages.get` is twenty
/// times a `labels.list` — so pacing by request count rather than by cost gets
/// the budget wrong by more than an order of magnitude depending on the mix.
mod cost {
    pub const LABELS_LIST: f64 = 1.0;
    pub const LABELS_CREATE: f64 = 5.0;
    pub const GET_PROFILE: f64 = 1.0;
    pub const HISTORY_LIST: f64 = 2.0;
    pub const MESSAGES_LIST: f64 = 5.0;
    pub const DRAFTS_LIST: f64 = 5.0;
    pub const MESSAGES_GET: f64 = 20.0;
    pub const MESSAGES_TRASH: f64 = 20.0;
    pub const ATTACHMENTS_GET: f64 = 20.0;
    pub const DRAFTS_CREATE: f64 = 10.0;
    pub const DRAFTS_DELETE: f64 = 10.0;
    pub const DRAFTS_UPDATE: f64 = 15.0;
    pub const BATCH_MODIFY: f64 = 50.0;
    pub const MESSAGES_SEND: f64 = 100.0;
}

/// What a request is for, which decides how much of the budget it may take.
///
/// Not every Gmail call is worth the same. Sending a mail is the user waiting;
/// hunting through old attachments for calendar invites is the app being
/// curious. Both used to draw on one queue in arrival order, so a backfill
/// could hold up a send — and worse, earn a block that then stopped sending
/// altogether.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Priority {
    /// Someone is waiting: sending, opening a mail, saving a draft, a sync the
    /// user asked for.
    Interactive,
    /// Bulk work with no one watching. Yields, and stands down entirely while
    /// the account is throttled.
    Background,
}

tokio::task_local! {
    static TASK_PRIORITY: Priority;
}

/// Run background work so its Gmail calls yield to interactive ones.
///
/// Scoped rather than passed down: the provider methods are shared between the
/// two — the same `fetch_attachment` serves a backfill and someone clicking an
/// attachment — so the intent belongs to the caller, not the call.
pub async fn as_background<F: std::future::Future>(work: F) -> F::Output {
    TASK_PRIORITY.scope(Priority::Background, work).await
}

fn current_priority() -> Priority {
    TASK_PRIORITY
        .try_with(|priority| *priority)
        .unwrap_or(Priority::Interactive)
}

/// Ceiling for background work, leaving the rest of the allowance free for
/// whatever the user is doing.
const BACKGROUND_UNITS_PER_SECOND: f64 = 40.0;

/// How long background work stays away after a block lapses.
///
/// The block itself is over, but Gmail's window is a moving average and the
/// interactive calls that were held back are about to run. Bulk work waiting a
/// few more minutes costs nothing and keeps that window clear.
const BACKGROUND_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(300);

/// Distance an interactive call keeps from a lapsing block before it may fire.
///
/// Gmail measures on a moving window, so the instant a block lapses is still
/// inside it — returning on the boundary renews the block, which is the whole
/// reason `consecutive_blocks` exists. The outbox holds its queued actions past
/// the boundary for exactly this reason (`RATE_LIMIT_RETRY_MARGIN` in
/// `src-tauri/src/sync/queue_worker.rs`); a call coming straight from the UI
/// kept no distance at all and so could re-earn a block for everyone, from a
/// single click.
const BLOCK_LAPSE_MARGIN: std::time::Duration = std::time::Duration::from_secs(30);

/// Spreads Gmail requests across the per-user budget.
///
/// Tracks the instant the budget next allows a request. Each call reserves its
/// own cost and moves that instant forward, so callers queue behind each other
/// without any shared counter drifting. Pacing a single loop is not enough: one
/// sync run touches every label, and the list, history and detail calls all draw
/// on the same budget.
struct QuotaPacer {
    next_free: std::time::Instant,
    /// Second, slower clock that only background work has to satisfy. It cannot
    /// spend more than its share, so there is always room left for a send.
    background_next_free: std::time::Instant,
    /// When Gmail has told us to stop entirely, until when.
    ///
    /// A per-user block is not a budgeting problem and pacing cannot work
    /// around it: even a 1-unit `labels.list` comes back 429 while it holds.
    /// Recording it in one place stops every other call path from spending its
    /// own retries discovering the same block — which is what turned one
    /// throttle into a steady stream of them.
    blocked_until: Option<std::time::Instant>,
    /// Blocks hit back-to-back, and when the last one lapsed.
    ///
    /// Honouring the stated time exactly is not enough. Gmail measures on a
    /// moving average, so a request in the second the block lapses is still too
    /// early and simply earns the next one: observed live as a 429 every 900
    /// seconds, on the second, indefinitely. Each consecutive block therefore
    /// waits longer than asked.
    consecutive_blocks: u32,
    last_block_lapsed: Option<std::time::Instant>,
}

impl QuotaPacer {
    /// Reserve `units` and return how long the caller must wait first.
    ///
    /// `None` means background work should not run at all right now — either a
    /// block is in force or one has just lapsed.
    fn reserve(&mut self, units: f64, priority: Priority) -> Option<std::time::Duration> {
        let now = std::time::Instant::now();

        // Interactive work waits a block out instead of failing, but it may not
        // return on the boundary, and the calls that waited together may not
        // return as one — so the block becomes a floor for the ordinary
        // reservation below rather than a wait of its own.
        let mut floor = now;
        if let Some(until) = self.blocked_until {
            if until > now {
                if priority == Priority::Background {
                    return None;
                }
                floor = until + BLOCK_LAPSE_MARGIN;
            } else {
                self.blocked_until = None;
            }
        }

        if priority == Priority::Background && self.in_background_cooldown(now) {
            return None;
        }

        // Idle time earns burst credit, but only up to the allowance — without
        // this clamp an app left running overnight would bank hours of budget
        // and fire the entire next sync at once.
        let earliest =
            now - std::time::Duration::from_secs_f64(QUOTA_BURST_UNITS / QUOTA_UNITS_PER_SECOND);
        if self.next_free < earliest {
            self.next_free = earliest;
        }
        if self.background_next_free < earliest {
            self.background_next_free = earliest;
        }

        let start = match priority {
            Priority::Interactive => self.next_free.max(floor),
            // Background satisfies both clocks, so it can never take more than
            // its share even when the shared budget is idle.
            Priority::Background => self.next_free.max(self.background_next_free).max(floor),
        };
        self.next_free = start + std::time::Duration::from_secs_f64(units / QUOTA_UNITS_PER_SECOND);
        if priority == Priority::Background {
            self.background_next_free =
                start + std::time::Duration::from_secs_f64(units / BACKGROUND_UNITS_PER_SECOND);
        }
        Some(start.saturating_duration_since(now))
    }

    /// Whether a block lapsed recently enough that bulk work should still wait.
    fn in_background_cooldown(&self, now: std::time::Instant) -> bool {
        self.last_block_lapsed
            .is_some_and(|lapsed| now < lapsed + BACKGROUND_COOLDOWN)
    }

    /// Record a server-imposed block so other callers do not walk into it.
    ///
    /// Returns the delay actually applied, which grows for repeat offences.
    fn block_for(&mut self, delay: std::time::Duration) -> std::time::Duration {
        let now = std::time::Instant::now();

        // A block arriving right after the previous one lapsed means the last
        // wait was too short, whatever the server said. Anything later is
        // treated as a fresh incident.
        let consecutive = self
            .last_block_lapsed
            .is_some_and(|lapsed| now.saturating_duration_since(lapsed) <= REPEAT_BLOCK_WINDOW);
        self.consecutive_blocks = if consecutive {
            self.consecutive_blocks.saturating_add(1)
        } else {
            1
        };

        let multiplier = 1u32 << (self.consecutive_blocks - 1).min(MAX_BLOCK_DOUBLINGS);
        let applied = (delay * multiplier).min(MAX_BLOCK_WAIT);

        let until = now + applied;
        if self.blocked_until.is_none_or(|current| until > current) {
            self.blocked_until = Some(until);
        }
        self.last_block_lapsed = Some(until);
        applied
    }

    /// How long the current block still has to run, if any.
    fn block_remaining(&self) -> Option<std::time::Duration> {
        self.blocked_until
            .map(|until| until.saturating_duration_since(std::time::Instant::now()))
            .filter(|remaining| !remaining.is_zero())
    }
}

/// Re-arm a block that was recorded before this process started.
///
/// The pacer holds the block in memory, which a restart used to wipe: the app
/// came back up, fired its startup work within seconds, and walked straight
/// into a block that still had half an hour to run — which Gmail measures on a
/// moving window, so knocking extended it. `sync::rate_limit_state` owns the
/// stored copy (the pool lives there, not here) and calls this at startup.
pub async fn restore_block(remaining: std::time::Duration, consecutive_blocks: u32) {
    if remaining.is_zero() {
        return;
    }
    let mut pacer = QUOTA_PACER.lock().await;
    let until = std::time::Instant::now() + remaining;
    if pacer.blocked_until.is_none_or(|current| until > current) {
        pacer.blocked_until = Some(until);
    }
    pacer.consecutive_blocks = consecutive_blocks;
    // Treated as one incident continuing, so the next block keeps doubling
    // instead of restarting at the server's first, shortest figure.
    pacer.last_block_lapsed = Some(until);
    tracing::info!(
        "gmail still rate limited from an earlier run: {}s remaining",
        remaining.as_secs()
    );
}

/// Drop the block, so a test can act as a freshly started process.
#[cfg(test)]
pub async fn clear_block_for_test() {
    let mut pacer = QUOTA_PACER.lock().await;
    pacer.blocked_until = None;
    pacer.consecutive_blocks = 0;
    pacer.last_block_lapsed = None;
}

/// The block still in force, as (remaining, consecutive blocks), for storing.
pub async fn block_snapshot() -> Option<(std::time::Duration, u32)> {
    let pacer = QUOTA_PACER.lock().await;
    pacer
        .block_remaining()
        .map(|remaining| (remaining, pacer.consecutive_blocks))
}

static QUOTA_PACER: std::sync::LazyLock<tokio::sync::Mutex<QuotaPacer>> =
    std::sync::LazyLock::new(|| {
        tokio::sync::Mutex::new(QuotaPacer {
            next_free: std::time::Instant::now(),
            background_next_free: std::time::Instant::now(),
            blocked_until: None,
            consecutive_blocks: 0,
            last_block_lapsed: None,
        })
    });

/// Wait until the budget allows a call costing `units`.
///
/// `Err` means the caller is background work that must not run now.
async fn await_quota(
    units: f64,
    priority: Priority,
    label: &str,
) -> crate::sync::errors::SyncResult<()> {
    // The lock is released before sleeping so concurrent callers can take their
    // own slot rather than serialising on the mutex itself.
    let (wait, retry_after) = {
        let mut pacer = QUOTA_PACER.lock().await;
        let wait = pacer.reserve(units, priority);
        let retry_after = wait.is_none().then(|| pacer.block_remaining()).flatten();
        (wait, retry_after)
    };
    let Some(wait) = wait else {
        return Err(crate::sync::errors::SyncError::rate_limited(
            format!(
                "gmail {label} skipped: background work stands down while the account is throttled"
            ),
            retry_after,
        ));
    };
    if !wait.is_zero() {
        tokio::time::sleep(wait).await;
    }
    Ok(())
}

/// Longest a single request will sit and wait for a block to lift.
///
/// Beyond this the run gives up and reports when Gmail said it would be safe,
/// rather than sleeping the maximum and knocking again — knocking early is what
/// keeps a block alive, and it is exactly what an earlier version of this code
/// did by capping the server-supplied time and retrying anyway.
pub(crate) const MAX_RATE_LIMIT_WAIT: std::time::Duration = std::time::Duration::from_secs(90);

pub(crate) const RATE_LIMIT_ATTEMPTS: usize = 3;

/// How soon after a block lapses a new one counts as the same incident.
///
/// Generous on purpose: the live pattern was a fresh block within a second of
/// the previous one expiring, and treating those as unrelated is what let the
/// cycle repeat forever.
const REPEAT_BLOCK_WINDOW: std::time::Duration = std::time::Duration::from_secs(120);

/// Ceiling for the doubling, so a bad hour cannot push the next attempt days out.
pub(crate) const MAX_BLOCK_DOUBLINGS: u32 = 4;

/// Absolute ceiling on a single block, regardless of doublings.
const MAX_BLOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(3600);

/// Whether a failed response is Gmail asking us to slow down.
///
/// 429 is the documented answer, but Gmail also returns 403 with a
/// `rateLimitExceeded` reason for the same condition. Treating that as a
/// permanent authorisation failure would park the account in an error state.
fn is_rate_limited(status: reqwest::StatusCode, body: &str) -> bool {
    status.as_u16() == 429
        || (status.as_u16() == 403
            && (body.contains("rateLimitExceeded") || body.contains("userRateLimitExceeded")))
}

/// How long to wait before retrying a rate-limited request.
///
/// Gmail states when the block lifts, either in a `Retry-After` header or as an
/// RFC 3339 timestamp inside the message ("Retry after 2026-07-27T17:20:03Z").
/// Both are honoured; without a hint the delay doubles per attempt.
pub(crate) fn rate_limit_delay(
    retry_after_header: Option<&str>,
    body: &str,
    attempt: u32,
) -> std::time::Duration {
    let from_header = retry_after_header
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(std::time::Duration::from_secs);

    // The timestamp sits inline in a JSON string and runs straight into
    // `","status":…` with no delimiter, so take only the characters RFC 3339
    // can contain rather than trying to trim the tail off.
    let from_body = body
        .split("Retry after ")
        .nth(1)
        .map(|rest| {
            rest.chars()
                .take_while(|c| {
                    c.is_ascii_digit() || matches!(c, '-' | ':' | 'T' | 'Z' | '.' | '+')
                })
                .collect::<String>()
        })
        .and_then(|stamp| chrono::DateTime::parse_from_rfc3339(&stamp).ok())
        .map(|when| when.signed_duration_since(chrono::Utc::now()))
        // A retry time already past yields a negative duration, which `to_std`
        // rejects — that correctly means there is nothing left to wait out.
        .and_then(|delta| delta.to_std().ok());

    // Returned uncapped on purpose: the caller needs the real figure to decide
    // between waiting and giving up, and to record the block for other callers.
    from_header
        .or(from_body)
        .unwrap_or_else(|| std::time::Duration::from_secs(1u64 << attempt))
}

/// Send a Gmail request, waiting out rate limits instead of failing the run.
///
/// Retrying only on the next sync tick — the previous behaviour — keeps landing
/// inside the penalty window and extends it. `build` is a closure because
/// `reqwest::RequestBuilder` cannot be reused across attempts.
///
/// Non-rate-limit responses are returned untouched so callers keep their own
/// status handling, including the 404 skip in the message loop.
async fn send_with_rate_limit_retry<F>(
    build: F,
    label: &str,
    units: f64,
) -> crate::sync::errors::SyncResult<reqwest::Response>
where
    F: Fn() -> reqwest::RequestBuilder,
{
    send_with_rate_limit_retry_mapped(build, label, units, |err| {
        format!("gmail {label} failed: {err}").into()
    })
    .await
}

/// [`send_with_rate_limit_retry`] with the caller deciding what a transport
/// error means. A submission needs that: whether its body got out decides
/// between a retry and `delivery_unknown`.
async fn send_with_rate_limit_retry_mapped<F, E>(
    build: F,
    label: &str,
    units: f64,
    transport_error: E,
) -> crate::sync::errors::SyncResult<reqwest::Response>
where
    F: Fn() -> reqwest::RequestBuilder,
    E: Fn(reqwest::Error) -> crate::sync::errors::SyncError,
{
    let priority = current_priority();
    let mut attempt = 1u32;
    loop {
        // A block recorded by any other call path is honoured here, so one 429
        // does not have to be rediscovered by every endpoint in turn.
        if let Some(remaining) = QUOTA_PACER.lock().await.block_remaining() {
            // The margin is part of the wait, so it counts against the ceiling —
            // otherwise "longest a single request will wait" quietly becomes
            // MAX_RATE_LIMIT_WAIT + BLOCK_LAPSE_MARGIN.
            if priority == Priority::Background
                || remaining + BLOCK_LAPSE_MARGIN > MAX_RATE_LIMIT_WAIT
            {
                return Err(crate::sync::errors::SyncError::rate_limited(
                    format!(
                        "gmail {label} skipped: rate limited for another {}s",
                        remaining.as_secs()
                    ),
                    Some(remaining),
                ));
            }
        }

        // Every attempt costs the same units, retries included — Gmail bills the
        // rejected call too, so charging only the first would understate usage
        // exactly when we are already over budget.
        await_quota(units, priority, label).await?;

        let response = build().send().await.map_err(&transport_error)?;

        let status = response.status();
        if status.is_success() || !(status.as_u16() == 429 || status.as_u16() == 403) {
            return Ok(response);
        }

        // Reading the body consumes the response, so capture the header first.
        let retry_after = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let body = response.text().await.unwrap_or_default();

        if !is_rate_limited(status, &body) {
            return Err(crate::sync::errors::http_failure(
                "gmail",
                label,
                status.as_u16(),
                &body,
            ));
        }

        let delay = rate_limit_delay(retry_after.as_deref(), &body, attempt);
        // The applied wait can exceed what Gmail asked for; report that figure
        // rather than the server's, so the log and the account error say how
        // long calls are actually paused.
        let delay = QUOTA_PACER.lock().await.block_for(delay);

        if attempt >= RATE_LIMIT_ATTEMPTS as u32 {
            return Err(
                crate::sync::errors::http_failure("gmail", label, status.as_u16(), &body)
                    .with_retry_after(delay),
            );
        }

        if delay > MAX_RATE_LIMIT_WAIT {
            // Logged as well as returned: this is the one event worth finding in
            // a log afterwards, and returning it only to the caller hid the
            // block entirely — the account showed an error while the log looked
            // quiet, which made it impossible to tell a fresh 429 from a stale
            // one still being waited out.
            tracing::warn!(
                "gmail {label} rate limited ({status}); Gmail asks for {}s, \
                 longer than this run will wait — pausing all Gmail calls until then",
                delay.as_secs()
            );
            return Err(crate::sync::errors::SyncError::rate_limited(
                format!(
                    "gmail {label} rate limited; Gmail asks for {}s, longer than this run will wait. \
                     Retrying sooner is what keeps the block alive, so the sync stops here.",
                    delay.as_secs()
                ),
                Some(delay),
            ));
        }

        tracing::warn!(
            "gmail {label} rate limited ({status}); waiting {:?} before attempt {}",
            delay,
            attempt + 1
        );
        tokio::time::sleep(delay).await;
        attempt += 1;
    }
}

#[derive(Debug, Clone)]
pub struct GmailProvider {
    config_json: crate::accounts::AccountConfig,
}

#[derive(Debug, Deserialize)]
struct GmailLabelsResponse {
    labels: Option<Vec<GmailLabel>>,
}

#[derive(Debug, Deserialize)]
struct GmailLabel {
    id: String,
    name: String,
    #[serde(rename = "type")]
    kind: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GmailMessageListResponse {
    messages: Option<Vec<GmailMessageListItem>>,
    #[serde(rename = "nextPageToken")]
    next_page_token: Option<String>,
    #[serde(rename = "resultSizeEstimate")]
    result_size_estimate: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct GmailDraftListResponse {
    drafts: Option<Vec<GmailDraftListItem>>,
    #[serde(rename = "nextPageToken")]
    next_page_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GmailDraftListItem {
    id: String,
    message: GmailMessageListItem,
}

#[derive(Debug, Clone, Deserialize)]
struct GmailMessageListItem {
    id: String,
}

#[derive(Debug, Deserialize)]
struct GmailMessageResponse {
    id: String,
    #[serde(rename = "threadId")]
    thread_id: Option<String>,
    #[serde(rename = "historyId")]
    history_id: Option<String>,
    #[serde(rename = "labelIds")]
    label_ids: Option<Vec<String>>,
    snippet: Option<String>,
    #[serde(rename = "internalDate")]
    internal_date: Option<String>,
    payload: Option<GmailPayload>,
    raw: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GmailPayload {
    filename: Option<String>,
    #[serde(rename = "mimeType")]
    mime_type: Option<String>,
    headers: Option<Vec<GmailHeader>>,
    body: Option<GmailBody>,
    parts: Option<Vec<GmailPayload>>,
}

#[derive(Debug, Clone, Deserialize)]
struct GmailHeader {
    name: String,
    value: String,
}

#[derive(Debug, Deserialize)]
struct GmailBody {
    data: Option<String>,
    #[serde(rename = "attachmentId")]
    attachment_id: Option<String>,
    size: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct GmailAttachmentResponse {
    data: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GmailHistoryResponse {
    history: Option<Vec<GmailHistoryRecord>>,
    #[serde(rename = "historyId")]
    history_id: Option<String>,
    #[serde(rename = "nextPageToken")]
    next_page_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GmailHistoryRecord {
    #[serde(rename = "messagesAdded")]
    messages_added: Option<Vec<GmailHistoryMessage>>,
    #[serde(rename = "messagesDeleted")]
    messages_deleted: Option<Vec<GmailHistoryMessage>>,
    // Label changes (read/unread, star, archive done in other clients) must be
    // re-fetched too, otherwise local flag state drifts from the server forever.
    #[serde(rename = "labelsAdded")]
    labels_added: Option<Vec<GmailHistoryMessage>>,
    #[serde(rename = "labelsRemoved")]
    labels_removed: Option<Vec<GmailHistoryMessage>>,
}

#[derive(Debug, Deserialize)]
struct GmailHistoryMessage {
    message: GmailMessageListItem,
}

#[derive(Debug, Default)]
struct ExtractedBody {
    text: Option<String>,
    html: Option<String>,
    calendar: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct GmailCursorState {
    #[serde(default)]
    latest_ts: u64,
    #[serde(default)]
    backfill_page_token: Option<String>,
    #[serde(default)]
    backfill_total_messages: usize,
    #[serde(default)]
    backfill_remaining_messages: usize,
    #[serde(default)]
    history_id: Option<String>,
    /// Set when Gmail dropped the stored `startHistoryId`.
    ///
    /// The timestamp catch-up that follows an expiry filters on `internalDate`,
    /// so mail that was labelled while the cursor was dead is invisible to it.
    /// Only a fresh backfill lists the label as it stands now.
    #[serde(default)]
    history_expired: bool,
    #[serde(default)]
    format_version: u8,
}

type ParsedGmailCursor = cursor::ParsedCursor<GmailCursorState>;

/// What a sync run should do about historical mail for one folder.
#[derive(Debug, Clone, PartialEq, Eq)]
enum BackfillStep {
    /// History is complete for this folder — incremental sync is all that's left.
    Skip,
    /// No cursor yet: start paging the label from the newest message backwards.
    Bootstrap,
    /// Resume paging from a stored page token.
    Continue(String),
}

/// The decision a sync run makes before touching the network.
///
/// Split out from `sync_messages` because it is where the interesting bug
/// lived and it is the cheapest part to test: no HTTP fake needed.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SyncPlan {
    /// `history.list` start id — the only source of *new* mail.
    incremental_from: Option<String>,
    /// Timestamp for the `after:` fallback used when history is unavailable.
    catch_up_from: Option<u64>,
    backfill: BackfillStep,
}

impl GmailProvider {
    // Version 3 invalidates bodies decoded from Gmail's full representation
    // using an original non-UTF-8 charset header.
    // A version upgrade restarts the server-backed folder backfill so cached
    // bodies are rebuilt from Gmail instead of repaired heuristically.
    const CURSOR_FORMAT_VERSION: u8 = 3;

    pub fn from_account_config(config_json: &crate::accounts::AccountConfig) -> Self {
        Self {
            config_json: config_json.clone(),
        }
    }

    async fn access_token(&self) -> Result<String, String> {
        access_token_from_account_config(OAuthProvider::Gmail, &self.config_json).await
    }

    /// Fetches a single message by its Gmail id and maps it to an envelope.
    /// Used by the targeted download behind server-search hits.
    pub async fn fetch_message_by_id(
        &self,
        remote_id: &str,
    ) -> crate::sync::errors::SyncResult<MessageEnvelope> {
        let token = self.access_token().await?;
        let client = crate::utils::http::client();
        let (mut messages, _, _) = Self::fetch_message_details(
            &client,
            &token,
            vec![GmailMessageListItem {
                id: remote_id.to_string(),
            }],
        )
        .await?;
        Ok(messages
            .pop()
            .ok_or_else(|| "message no longer exists on the server".to_string())?)
    }

    /// Fetch the provider's original RFC 822 message for explicit source
    /// inspection and cryptographic metadata repair.
    pub async fn fetch_raw_message(
        &self,
        remote_id: &str,
    ) -> crate::sync::errors::SyncResult<Vec<u8>> {
        let token = self.access_token().await?;
        let response = send_with_rate_limit_retry(
            || {
                crate::utils::http::client()
                    .get(format!("{}/messages/{remote_id}", gmail_api_base()))
                    .query(&[("format", "raw")])
                    .bearer_auth(&token)
            },
            "message source fetch",
            cost::MESSAGES_GET,
        )
        .await?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(crate::sync::errors::http_failure(
                "gmail",
                "message source fetch",
                status.as_u16(),
                &body,
            ));
        }
        let raw = response
            .json::<GmailMessageResponse>()
            .await
            .map_err(|err| format!("gmail message source parse failed: {err}"))?
            .raw
            .and_then(|value| decode_gmail_body_bytes(&value))
            .ok_or_else(|| "gmail response did not contain message source".to_string())?;
        Ok(raw)
    }

    fn parse_cursor_state(since: &SyncCheckpoint) -> ParsedGmailCursor {
        cursor::parse_state(since.cursor.as_deref(), |raw| {
            let mut state = GmailCursorState::default();
            if let Some(ts) = parse_gmail_cursor(Some(raw)) {
                state.latest_ts = ts;
            }
            state
        })
    }

    /// Decide what a run does, from the cursor alone.
    ///
    /// The incremental pass and the backfill are **independent**: they used to
    /// be the two arms of one `if`, so a folder with a pending backfill token
    /// never ran `history.list` and therefore never saw new mail. Backfill
    /// advances one 100-message page per run, so for a large label that state
    /// lasted for days.
    fn plan_sync(parsed: &ParsedGmailCursor) -> SyncPlan {
        let incremental_from = parsed
            .state
            .history_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);

        let pending = parsed
            .state
            .backfill_page_token
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);

        let needs_format_backfill = parsed.had_checkpoint
            && !parsed.was_legacy
            && parsed.state.format_version < Self::CURSOR_FORMAT_VERSION;

        let backfill = match pending {
            // Start at the newest page after a format change. Continuing an
            // older partial backfill would leave its already-visited pages in
            // the cache with the previous decoder's output.
            _ if needs_format_backfill => BackfillStep::Bootstrap,
            Some(token) => BackfillStep::Continue(token),
            // An expired history cursor leaves a hole no incremental pass can
            // see: the `after:` catch-up matches on receive time, and the
            // snapshot cannot discover IDs never cached locally. Re-list the label.
            None if parsed.state.history_expired => BackfillStep::Bootstrap,
            // A legacy `ts:` cursor never backfilled, so it starts one now.
            None if !parsed.had_checkpoint || parsed.was_legacy => BackfillStep::Bootstrap,
            None => BackfillStep::Skip,
        };

        // Only meaningful once a previous run recorded a timestamp. Without one
        // the `after:` query has no lower bound and the catch-up loop would page
        // through the entire label — which is what the backfill is for.
        let catch_up_from =
            (parsed.had_checkpoint && parsed.state.latest_ts > 0).then_some(parsed.state.latest_ts);

        SyncPlan {
            incremental_from,
            catch_up_from,
            backfill,
        }
    }

    /// Does this error mean the stored `startHistoryId` is gone?
    ///
    /// Gmail keeps roughly a week of history and answers 404 (sometimes 410)
    /// once the cursor falls out of that window. Recovering means re-listing
    /// the folder, so a throttle must never be mistaken for an expiry — that
    /// would trigger a full listing on every rate limit.
    fn history_cursor_expired(err: &str) -> bool {
        if err.contains("rate limited") {
            return false;
        }
        err.contains("status 404")
            || err.contains("status 410")
            || err.contains("failedPrecondition")
    }

    /// Fetch one page of historical mail and advance the backfill cursor.
    ///
    /// Returns the messages and the newest timestamp on the page. The page's
    /// `historyId`s are deliberately discarded: backfilled messages carry
    /// ancient ids by construction, and feeding them into the cursor could only
    /// ever move it backwards.
    async fn backfill_one_page(
        client: &reqwest::Client,
        token: &str,
        folder: &str,
        page_token: Option<&str>,
        cursor_state: &mut GmailCursorState,
    ) -> crate::sync::errors::SyncResult<(Vec<MessageEnvelope>, u64)> {
        let payload = Self::list_message_page(client, token, folder, None, page_token).await?;
        let items = payload.messages.unwrap_or_default();
        let fetched_count = items.len();
        let total_estimate = payload
            .result_size_estimate
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or(cursor_state.backfill_total_messages.max(fetched_count));
        let (page_messages, page_latest_ts, _page_history_id) =
            Self::fetch_message_details(client, token, items).await?;

        if let Some(next_page_token) = payload
            .next_page_token
            .filter(|value| !value.trim().is_empty())
        {
            let remaining = if cursor_state.backfill_remaining_messages > 0 {
                cursor_state
                    .backfill_remaining_messages
                    .saturating_sub(fetched_count)
                    .max(1)
            } else {
                total_estimate.saturating_sub(fetched_count).max(1)
            };
            cursor_state.backfill_page_token = Some(next_page_token);
            cursor_state.backfill_total_messages = total_estimate.max(fetched_count + remaining);
            cursor_state.backfill_remaining_messages = remaining;
        } else {
            cursor_state.backfill_page_token = None;
            cursor_state.backfill_total_messages = total_estimate.max(fetched_count);
            cursor_state.backfill_remaining_messages = 0;
        }

        Ok((page_messages, page_latest_ts))
    }

    async fn list_message_page(
        client: &reqwest::Client,
        token: &str,
        folder: &str,
        after_ts: Option<u64>,
        page_token: Option<&str>,
    ) -> crate::sync::errors::SyncResult<GmailMessageListResponse> {
        Self::list_message_page_with_label(client, token, Some(folder), after_ts, page_token).await
    }

    async fn list_message_page_with_label(
        client: &reqwest::Client,
        token: &str,
        folder: Option<&str>,
        after_ts: Option<u64>,
        page_token: Option<&str>,
    ) -> crate::sync::errors::SyncResult<GmailMessageListResponse> {
        let url = Self::gmail_message_list_url(folder, after_ts, page_token)?;

        let response = send_with_rate_limit_retry(
            || client.get(url.clone()).bearer_auth(token),
            "messages list",
            cost::MESSAGES_LIST,
        )
        .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(crate::sync::errors::http_failure(
                "gmail",
                "messages list",
                status.as_u16(),
                &body,
            ));
        }

        Ok(response
            .json()
            .await
            .map_err(|err| format!("gmail messages list parse failed: {err}"))?)
    }

    fn gmail_message_list_url(
        folder: Option<&str>,
        after_ts: Option<u64>,
        page_token: Option<&str>,
    ) -> crate::sync::errors::SyncResult<Url> {
        let mut url = Url::parse(&format!("{}/messages", gmail_api_base()))
            .map_err(|err| format!("invalid gmail messages URL: {err}"))?;
        {
            let mut query = url.query_pairs_mut();
            if let Some(folder) = folder {
                query.append_pair("labelIds", folder);
            }
            query.append_pair("maxResults", "100");
            if let Some(ts) = after_ts {
                query.append_pair("q", &format!("after:{}", ts.saturating_sub(120)));
            }
            if let Some(page_token) = page_token.filter(|value| !value.trim().is_empty()) {
                query.append_pair("pageToken", page_token);
            }
        }
        Ok(url)
    }

    /// How many detail fetches may be in flight at once.
    ///
    /// The pacer frees a `MESSAGES_GET` slot every 200 ms (20 units against a
    /// 100-unit budget), but a single request often takes longer than that
    /// round trip, so a strictly sequential loop leaves reserved slots unused.
    /// Overlapping a few requests spends the budget the pacer already grants —
    /// it cannot exceed it, because every request still reserves its own slot.
    const DETAIL_FETCHES_IN_FLIGHT: usize = 3;

    async fn fetch_message_details(
        client: &reqwest::Client,
        token: &str,
        item_ids: Vec<GmailMessageListItem>,
    ) -> crate::sync::errors::SyncResult<(Vec<MessageEnvelope>, u64, Option<String>)> {
        use futures::StreamExt;

        let mut messages = Vec::new();
        let mut latest_ts = 0u64;
        let mut latest_history_id = None::<String>;

        // `buffered` (not `buffer_unordered`) keeps the list order, so the
        // stored batch stays newest-first exactly as the sequential loop did.
        let mut details = futures::stream::iter(item_ids.into_iter().map(|item| async move {
            let detail = send_with_rate_limit_retry(
                || {
                    client
                        .get(format!("{}/messages/{}", gmail_api_base(), item.id))
                        .query(&[("format", "full")])
                        .bearer_auth(token)
                },
                "message fetch",
                cost::MESSAGES_GET,
            )
            .await?;

            if !detail.status().is_success() {
                let status = detail.status();
                // History records can reference messages deleted in the meantime
                // (and labelsRemoved records often do) — skip instead of failing
                // the whole sync run.
                if status.as_u16() == 404 {
                    return Ok(None);
                }
                let body = detail.text().await.unwrap_or_default();
                return Err(crate::sync::errors::http_failure(
                    "gmail",
                    "message fetch",
                    status.as_u16(),
                    &body,
                ));
            }

            let mut message = detail
                .json::<GmailMessageResponse>()
                .await
                .map_err(|err| format!("gmail message parse failed: {err}"))?;
            validate_gmail_full_message(&message)?;
            if message.label_ids.is_none() {
                message.label_ids =
                    Some(Self::confirm_omitted_labels(client, token, &item.id).await?);
            }
            if gmail_message_needs_raw_crypto(&message)
                || message
                    .payload
                    .as_ref()
                    .is_some_and(gmail_body_needs_raw_source)
            {
                let raw = send_with_rate_limit_retry(
                    || {
                        client
                            .get(format!("{}/messages/{}", gmail_api_base(), item.id))
                            .query(&[("format", "raw")])
                            .bearer_auth(token)
                    },
                    "message raw fetch",
                    cost::MESSAGES_GET,
                )
                .await?;
                if !raw.status().is_success() {
                    let status = raw.status();
                    let body = raw.text().await.unwrap_or_default();
                    return Err(crate::sync::errors::http_failure(
                        "gmail",
                        "message raw fetch",
                        status.as_u16(),
                        &body,
                    ));
                }
                let source = raw
                    .json::<GmailMessageResponse>()
                    .await
                    .map_err(|err| format!("gmail raw message parse failed: {err}"))?;
                if source.id != message.id {
                    return Err("gmail raw lookup returned a mismatched message".into());
                }
                message.raw =
                    Some(source.raw.ok_or_else(|| {
                        "gmail response did not contain message source".to_string()
                    })?);
            }
            if let Some(raw) = message.raw.as_deref() {
                validate_gmail_raw_source(raw)?;
            }
            Ok(Some(message))
        }))
        .buffered(Self::DETAIL_FETCHES_IN_FLIGHT);

        while let Some(result) = details.next().await {
            let Some(detail) = result? else {
                continue;
            };

            let internal_ts = detail
                .internal_date
                .as_deref()
                .and_then(|value| value.parse::<i64>().ok())
                .map(|millis| millis / 1000)
                .unwrap_or(0)
                .max(0) as u64;
            latest_ts = latest_ts.max(internal_ts);
            latest_history_id =
                Self::newest_history_id(latest_history_id, detail.history_id.clone());
            messages.push(map_gmail_message(detail));
        }

        Ok((messages, latest_ts, latest_history_id))
    }

    /// Gmail's JSON drops an empty `labelIds`, and the API reference does not
    /// say otherwise. A full response without it is therefore either a
    /// genuinely unlabeled message (archived and read, with no category) or a
    /// response cut short. Ask for the labels alone (`format=minimal`: "only
    /// email message ID and labels") and accept an empty set only from a
    /// complete answer that again carries none.
    async fn confirm_omitted_labels(
        client: &reqwest::Client,
        token: &str,
        id: &str,
    ) -> crate::sync::errors::SyncResult<Vec<String>> {
        let response = send_with_rate_limit_retry(
            || {
                client
                    .get(format!("{}/messages/{}", gmail_api_base(), id))
                    .query(&[("format", "minimal")])
                    .bearer_auth(token)
            },
            "message label lookup",
            cost::MESSAGES_GET,
        )
        .await?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(crate::sync::errors::http_failure(
                "gmail",
                "message label lookup",
                status.as_u16(),
                &body,
            ));
        }
        let minimal = response
            .json::<GmailMessageResponse>()
            .await
            .map_err(|err| format!("gmail label lookup parse failed: {err}"))?;
        if minimal.id != id || !gmail_identity_is_complete(&minimal) {
            return Err("gmail label lookup returned an incomplete message".into());
        }
        if minimal.label_ids.is_some() {
            return Err("gmail message lookup omitted its labels".into());
        }
        Ok(Vec::new())
    }

    async fn fetch_current_drafts(
        client: &reqwest::Client,
        token: &str,
    ) -> crate::sync::errors::SyncResult<Vec<MessageEnvelope>> {
        let mut page_token = None::<String>;
        let mut messages = Vec::new();
        loop {
            let mut url = Url::parse(&format!("{}/drafts", gmail_api_base()))
                .map_err(|err| format!("invalid gmail drafts URL: {err}"))?;
            {
                let mut query = url.query_pairs_mut();
                query.append_pair("maxResults", "100");
                if let Some(page_token) = page_token.as_deref() {
                    query.append_pair("pageToken", page_token);
                }
            }
            let response = send_with_rate_limit_retry(
                || client.get(url.clone()).bearer_auth(token),
                "drafts list",
                cost::DRAFTS_LIST,
            )
            .await?;
            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                return Err(crate::sync::errors::http_failure(
                    "gmail",
                    "drafts list",
                    status.as_u16(),
                    &body,
                ));
            }
            let payload: GmailDraftListResponse = response
                .json()
                .await
                .map_err(|err| format!("gmail drafts list parse failed: {err}"))?;
            for draft in payload.drafts.unwrap_or_default() {
                let (mut fetched, _, _) =
                    Self::fetch_message_details(client, token, vec![draft.message]).await?;
                if let Some(message) = fetched.first_mut() {
                    message.remote_draft_id = Some(draft.id);
                }
                messages.extend(fetched);
            }
            match payload
                .next_page_token
                .filter(|value| !value.trim().is_empty())
            {
                Some(next) => page_token = Some(next),
                None => break,
            }
        }
        Ok(messages)
    }

    /// Numeric max of two Gmail historyIds. The naive `.or()` chain used to keep
    /// whichever detail was processed *last* — with newest-first listing that is
    /// the OLDEST message, whose historyId expires within days and permanently
    /// kills incremental sync (history.list 404).
    fn newest_history_id(current: Option<String>, candidate: Option<String>) -> Option<String> {
        match (current, candidate) {
            (Some(current), Some(candidate)) => {
                let current_num = current.parse::<u64>().unwrap_or(0);
                let candidate_num = candidate.parse::<u64>().unwrap_or(0);
                Some(if candidate_num > current_num {
                    candidate
                } else {
                    current
                })
            }
            (current, candidate) => current.or(candidate),
        }
    }

    /// The mailbox's current historyId from the profile endpoint — the correct
    /// checkpoint to snapshot "now" before/while backfilling older mail.
    async fn fetch_profile_history_id(
        client: &reqwest::Client,
        token: &str,
    ) -> crate::sync::errors::SyncResult<Option<String>> {
        let response = send_with_rate_limit_retry(
            || {
                client
                    .get(format!("{}/profile", gmail_api_base()))
                    .bearer_auth(token)
            },
            "profile request",
            cost::GET_PROFILE,
        )
        .await?;
        if !response.status().is_success() {
            return Ok(None);
        }
        let value: serde_json::Value = response
            .json()
            .await
            .map_err(|err| format!("gmail profile parse failed: {err}"))?;
        Ok(value.get("historyId").and_then(|raw| {
            raw.as_str()
                .map(str::to_string)
                .or_else(|| raw.as_u64().map(|num| num.to_string()))
        }))
    }

    async fn list_history_page(
        client: &reqwest::Client,
        token: &str,
        folder: &str,
        start_history_id: &str,
        page_token: Option<&str>,
    ) -> crate::sync::errors::SyncResult<GmailHistoryResponse> {
        let mut url = Url::parse(&format!("{}/history", gmail_api_base()))
            .map_err(|err| format!("invalid gmail history URL: {err}"))?;
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("startHistoryId", start_history_id);
            query.append_pair("labelId", folder);
            // No historyTypes filter: we need labelsAdded/labelsRemoved too so
            // read/star/archive changes made in other clients reach us.
            if let Some(page_token) = page_token.filter(|value| !value.trim().is_empty()) {
                query.append_pair("pageToken", page_token);
            }
        }
        let response = send_with_rate_limit_retry(
            || client.get(url.clone()).bearer_auth(token),
            "history request",
            cost::HISTORY_LIST,
        )
        .await?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(crate::sync::errors::http_failure(
                "gmail",
                "history request",
                status.as_u16(),
                &body,
            ));
        }
        Ok(response
            .json()
            .await
            .map_err(|err| format!("gmail history parse failed: {err}"))?)
    }

    fn finalize_cursor(state: GmailCursorState) -> Option<String> {
        // A history id alone is worth persisting: a folder that is empty when it
        // is first synced has no timestamp and no backfill token, and dropping
        // the cursor made it re-bootstrap — another messages.list plus another
        // getProfile — on every single run, forever.
        let has_history_id = !state
            .history_id
            .as_deref()
            .map(str::trim)
            .unwrap_or_default()
            .is_empty();

        if state.latest_ts == 0
            && !has_history_id
            && !state.history_expired
            && state
                .backfill_page_token
                .as_deref()
                .map(str::trim)
                .unwrap_or_default()
                .is_empty()
        {
            None
        } else {
            Some(
                serde_json::to_string(&state).unwrap_or_else(|_| format!("ts:{}", state.latest_ts)),
            )
        }
    }
}

fn gmail_move_label_changes(messages: &[MessageRef], target: &str) -> (Vec<String>, Vec<String>) {
    let mut add_labels = Vec::new();
    let mut remove_labels = Vec::new();
    if target.eq_ignore_ascii_case("ARCHIVE") {
        remove_labels.push("INBOX".to_string());
        for source in messages
            .iter()
            .filter_map(|message| message.source_folder.as_deref())
        {
            let normalized = source.trim();
            if !normalized.is_empty()
                && !normalized.eq_ignore_ascii_case("INBOX")
                && !normalized.eq_ignore_ascii_case("ARCHIVE")
            {
                remove_labels.push(normalized.to_string());
            }
        }
    } else {
        add_labels.push(target.to_string());
        if !target.eq_ignore_ascii_case("INBOX") {
            remove_labels.push("INBOX".to_string());
        }
        if target.eq_ignore_ascii_case("TRASH") || target.eq_ignore_ascii_case("INBOX") {
            for source in messages
                .iter()
                .filter_map(|message| message.source_folder.as_deref())
            {
                let normalized = source.trim();
                // ARCHIVE is Satchel's synthetic representation of "not in
                // Inbox", never a real Gmail label accepted by batchModify.
                if normalized.is_empty()
                    || normalized.eq_ignore_ascii_case("INBOX")
                    || normalized.eq_ignore_ascii_case("ARCHIVE")
                    || normalized.eq_ignore_ascii_case("TRASH")
                    || normalized.eq_ignore_ascii_case(target)
                {
                    continue;
                }
                remove_labels.push(normalized.to_string());
            }
        }
    }

    add_labels.sort();
    add_labels.dedup();
    remove_labels.sort();
    remove_labels.dedup();
    (add_labels, remove_labels)
}

#[async_trait::async_trait]
impl MailProvider for GmailProvider {
    async fn sync_folders(&self) -> crate::sync::errors::SyncResult<Vec<RemoteFolder>> {
        let token = self.access_token().await?;
        let client = crate::utils::http::client();
        let response = send_with_rate_limit_retry(
            || {
                client
                    .get(format!("{}/labels", gmail_api_base()))
                    .bearer_auth(&token)
            },
            "labels request",
            cost::LABELS_LIST,
        )
        .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(crate::sync::errors::http_failure(
                "gmail",
                "labels request",
                status.as_u16(),
                &body,
            ));
        }

        let payload: GmailLabelsResponse = response
            .json()
            .await
            .map_err(|err| format!("gmail labels parse failed: {err}"))?;

        let mut folders = payload
            .labels
            .unwrap_or_default()
            .into_iter()
            .filter(|label| {
                label
                    .kind
                    .as_deref()
                    .map(|kind| {
                        kind.eq_ignore_ascii_case("system") || kind.eq_ignore_ascii_case("user")
                    })
                    .unwrap_or(true)
            })
            .filter(|label| !gmail_label_is_state_view(&label.id))
            .map(|label| RemoteFolder {
                kind: gmail_system_folder_kind(&label.id).map(str::to_string),
                id: label.id,
                name: label.name,
            })
            .collect::<Vec<_>>();
        // Keep the order stable for deterministic sync/checkpoint behavior.
        // CATEGORY_* labels remain discovery sources for archived/filtered mail;
        // classification itself reads the message's complete label set.
        folders.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(folders)
    }

    async fn sync_messages(
        &self,
        folder: &str,
        since: SyncCheckpoint,
    ) -> crate::sync::errors::SyncResult<SyncBatch> {
        let token = self.access_token().await?;
        let client = crate::utils::http::client();
        if folder.eq_ignore_ascii_case("DRAFT") || folder.eq_ignore_ascii_case("DRAFTS") {
            return Ok(SyncBatch {
                messages: Self::fetch_current_drafts(&client, &token).await?,
                removals: Vec::new(),
                remote_uidvalidity: None,
                reset_folder: false,
                snapshot_required: true,
                next: since,
            });
        }
        let parsed_cursor = Self::parse_cursor_state(&since);
        let plan = Self::plan_sync(&parsed_cursor);
        let mut cursor_state = parsed_cursor.state;

        let mut messages = Vec::new();
        let mut removals = Vec::new();
        let mut snapshot_required = false;
        let mut latest_ts = cursor_state.latest_ts;

        // ---- Phase 1: incremental. Runs on every sync, backfill or not. ----
        {
            let mut history_synced = false;
            if let Some(history_id) = plan.incremental_from.clone() {
                let mut page_token = None::<String>;
                history_synced = true;
                loop {
                    let payload = match Self::list_history_page(
                        &client,
                        &token,
                        folder,
                        &history_id,
                        page_token.as_deref(),
                    )
                    .await
                    {
                        Ok(payload) => payload,
                        Err(err) if Self::history_cursor_expired(&err.to_string()) => {
                            // startHistoryId expired (Gmail keeps roughly a week
                            // of history). Self-heal: drop the cursor, run the
                            // timestamp catch-up below, and mark the folder for
                            // a fresh backfill — the catch-up alone would miss
                            // every message labelled while the cursor was dead.
                            cursor_state.history_id = None;
                            cursor_state.backfill_page_token = None;
                            cursor_state.history_expired = true;
                            history_synced = false;
                            snapshot_required = true;
                            break;
                        }
                        Err(err) => return Err(err),
                    };
                    let mut seen = std::collections::HashSet::<String>::new();
                    let mut items = Vec::new();
                    for record in payload.history.unwrap_or_default() {
                        for deleted in record.messages_deleted.unwrap_or_default() {
                            removals.push(RemoteMessageRemoval {
                                remote_id: deleted.message.id,
                                // messagesDeleted is mailbox-wide. It is not a
                                // label removal, so every local label copy must go.
                                source_folder: None,
                            });
                        }
                        let mut touched = record.messages_added.unwrap_or_default();
                        touched.extend(record.labels_added.unwrap_or_default());
                        touched.extend(record.labels_removed.unwrap_or_default());
                        for changed in touched {
                            if seen.insert(changed.message.id.clone()) {
                                items.push(changed.message);
                            }
                        }
                    }
                    let (page_messages, page_latest_ts, _detail_history_id) =
                        Self::fetch_message_details(&client, &token, items).await?;
                    latest_ts = latest_ts.max(page_latest_ts);
                    // Details may already reflect changes beyond this history
                    // response. Only the completed feed authorizes progress.
                    if payload
                        .next_page_token
                        .as_deref()
                        .is_none_or(|token| token.trim().is_empty())
                    {
                        cursor_state.history_id = Self::newest_history_id(
                            cursor_state.history_id.take(),
                            payload.history_id.clone(),
                        );
                    }
                    messages.extend(page_messages);

                    match payload
                        .next_page_token
                        .filter(|value| !value.trim().is_empty())
                    {
                        Some(next) => page_token = Some(next),
                        None => break,
                    }
                }
            }
            if !history_synced {
                // Only with a lower bound. Without one this pages the entire
                // label, which on a first run would download the mailbox before
                // the backfill below ever got a turn.
                if let Some(since_ts) = plan.catch_up_from {
                    let mut page_token = None::<String>;

                    loop {
                        let payload = Self::list_message_page(
                            &client,
                            &token,
                            folder,
                            Some(since_ts),
                            page_token.as_deref(),
                        )
                        .await?;
                        let items = payload.messages.unwrap_or_default();
                        let next_page_token = payload.next_page_token;
                        let (page_messages, page_latest_ts, page_history_id) =
                            Self::fetch_message_details(&client, &token, items).await?;
                        latest_ts = latest_ts.max(page_latest_ts);
                        cursor_state.history_id = Self::newest_history_id(
                            cursor_state.history_id.take(),
                            page_history_id,
                        );
                        messages.extend(page_messages);

                        match next_page_token.filter(|value| !value.trim().is_empty()) {
                            Some(next) => page_token = Some(next),
                            None => break,
                        }
                    }
                }

                // Seed the history cursor from the mailbox's CURRENT historyId.
                // Doing it here — rather than inside the backfill, as before —
                // means every path leaves a usable cursor behind, so the next
                // run can go incremental even while backfill is still running.
                if cursor_state
                    .history_id
                    .as_deref()
                    .map(str::trim)
                    .unwrap_or_default()
                    .is_empty()
                {
                    if let Ok(fresh) = Self::fetch_profile_history_id(&client, &token).await {
                        cursor_state.history_id = fresh;
                    }
                }
            }
        }

        // ---- Phase 2: at most one page of history, and never fatal. ----
        //
        // A backfill page is a partial, historical view of the folder — it is
        // NEVER a folder snapshot, so it must not touch `snapshot_required`:
        // reconciliation needs complete folder presence, followed by global
        // resolution of absent local IDs. A time window cannot prove absence.
        //
        // Failures here are swallowed on purpose. These calls are rate-limited
        // far more often than the incremental pass, and propagating the error
        // would make the manager drop the whole batch — throwing away the new
        // mail phase 1 just fetched along with the advanced history cursor, on
        // exactly the accounts that need to catch up.
        let mut backfill_advanced = matches!(plan.backfill, BackfillStep::Skip);
        match &plan.backfill {
            BackfillStep::Skip => {}
            step => {
                let page_token = match step {
                    BackfillStep::Continue(token) => Some(token.as_str()),
                    _ => None,
                };
                match Self::backfill_one_page(
                    &client,
                    &token,
                    folder,
                    page_token,
                    &mut cursor_state,
                )
                .await
                {
                    Ok((page_messages, page_latest_ts)) => {
                        backfill_advanced = true;
                        latest_ts = latest_ts.max(page_latest_ts);
                        messages.extend(page_messages);
                    }
                    Err(err) => {
                        tracing::warn!(
                            %folder,
                            %err,
                            "gmail backfill page deferred; incremental results kept"
                        );
                    }
                }
            }
        }

        // The same message can show up in both phases; the upsert is idempotent
        // but the round trip is not free.
        let mut seen_remote_ids = std::collections::HashSet::<String>::new();
        messages.retain(|message| seen_remote_ids.insert(message.remote_id.clone()));

        cursor_state.latest_ts = latest_ts;
        // Do not acknowledge a cursor/body format migration when its first
        // backfill request failed. Keeping the old version makes the next sync
        // retry from the server instead of silently accepting stale bodies.
        if backfill_advanced {
            cursor_state.format_version = Self::CURSOR_FORMAT_VERSION;
        }
        // The re-list has reached the end of the label, so the gap the expiry
        // left is closed. Checking that a page actually ran keeps the run that
        // *discovers* the expiry from clearing the flag it just set.
        if !matches!(plan.backfill, BackfillStep::Skip)
            && backfill_advanced
            && cursor_state.backfill_page_token.is_none()
        {
            cursor_state.history_expired = false;
        }

        Ok(SyncBatch {
            messages,
            removals,
            remote_uidvalidity: None,
            reset_folder: false,
            snapshot_required,
            next: SyncCheckpoint {
                cursor: Self::finalize_cursor(cursor_state),
            },
        })
    }

    async fn resolve_snapshot_message(
        &self,
        remote_id: &str,
    ) -> crate::sync::errors::SyncResult<Option<MessageEnvelope>> {
        let token = self.access_token().await?;
        let (mut messages, _, _) = Self::fetch_message_details(
            &crate::utils::http::client(),
            &token,
            vec![GmailMessageListItem {
                id: remote_id.to_string(),
            }],
        )
        .await?;
        // fetch_message_details omits only explicit HTTP 404 responses.
        // Parsing, authentication, quota and transport errors remain errors.
        let message = messages.pop();
        if message
            .as_ref()
            .is_some_and(|message| message.remote_id != remote_id)
        {
            return Err("gmail snapshot lookup returned a mismatched message".into());
        }
        Ok(message)
    }

    async fn sync_message_state_snapshot(
        &self,
        folder: &str,
    ) -> crate::sync::errors::SyncResult<Option<crate::providers::traits::RemoteMessageStateSnapshot>>
    {
        // Folder snapshots repair moves/deletes even after a history cursor
        // has expired. Absent IDs still need account-wide resolution before
        // removal; flags remain sourced from full message payloads.

        let token = self.access_token().await?;
        let client = crate::utils::http::client();
        let mut page_token = None::<String>;
        let mut states = Vec::new();

        loop {
            let payload =
                Self::list_message_page(&client, &token, folder, None, page_token.as_deref())
                    .await?;
            states.extend(
                payload
                    .messages
                    .unwrap_or_default()
                    .into_iter()
                    .map(|message| RemoteMessageState {
                        remote_id: message.id,
                        is_read: None,
                        is_starred: None,
                    }),
            );

            match payload
                .next_page_token
                .filter(|value| !value.trim().is_empty())
            {
                Some(next) => page_token = Some(next),
                None => break,
            }
        }

        Ok(Some(crate::providers::traits::RemoteMessageStateSnapshot {
            states,
            // Gmail message ids are account-wide and never renumber.
            remote_uidvalidity: None,
        }))
    }

    async fn send(&self, message: OutgoingMessage) -> crate::sync::errors::SyncResult<()> {
        let token = self.access_token().await?;
        let raw = build_gmail_raw_message(&message)?;
        let payload = bytes::Bytes::from(
            serde_json::json!({
                "raw": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw.as_bytes())
            })
            .to_string(),
        );

        let client = crate::utils::http::client();
        // The client-wide deadline covers the upload too; sized to the message,
        // a few megabytes over a slow uplink can still finish.
        let deadline = crate::utils::http::upload_timeout(
            crate::utils::http::UPLOAD_BASE_TIMEOUT,
            payload.len(),
        );
        let upload = crate::utils::http::UploadProgress::default();
        let response = send_with_rate_limit_retry_mapped(
            || {
                client
                    .post(format!("{}/messages/send", gmail_api_base()))
                    .bearer_auth(&token)
                    .header(reqwest::header::CONTENT_TYPE, "application/json")
                    .timeout(deadline)
                    .body(upload.body(payload.clone()))
            },
            "send",
            cost::MESSAGES_SEND,
            |err| {
                crate::sync::errors::http_submission_transport_failure(
                    "gmail",
                    err,
                    upload.fully_handed_over(),
                )
            },
        )
        .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(crate::sync::errors::http_failure(
                "gmail",
                "send",
                status.as_u16(),
                &body,
            ));
        }

        Ok(())
    }

    async fn set_flags(
        &self,
        _messages: &[MessageRef],
        _flags: FlagUpdate,
    ) -> crate::sync::errors::SyncResult<()> {
        let token = self.access_token().await?;
        if _messages.is_empty() {
            return Ok(());
        }

        let mut add_labels = Vec::new();
        let mut remove_labels = Vec::new();
        if let Some(read) = _flags.read {
            if read {
                remove_labels.push("UNREAD".to_string());
            } else {
                add_labels.push("UNREAD".to_string());
            }
        }
        if let Some(starred) = _flags.starred {
            if starred {
                add_labels.push("STARRED".to_string());
            } else {
                remove_labels.push("STARRED".to_string());
            }
        }

        if add_labels.is_empty() && remove_labels.is_empty() {
            return Ok(());
        }

        let payload = serde_json::json!({
            "ids": _messages.iter().map(|message| message.remote_id.clone()).collect::<Vec<_>>(),
            "addLabelIds": add_labels,
            "removeLabelIds": remove_labels,
        });

        let client = crate::utils::http::client();
        let response = send_with_rate_limit_retry(
            || {
                client
                    .post(format!("{}/messages/batchModify", gmail_api_base()))
                    .bearer_auth(&token)
                    .json(&payload)
            },
            "set_flags",
            cost::BATCH_MODIFY,
        )
        .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(crate::sync::errors::http_failure(
                "gmail",
                "set_flags",
                status.as_u16(),
                &body,
            ));
        }

        Ok(())
    }

    async fn move_messages(
        &self,
        _messages: &[MessageRef],
        _target: &str,
    ) -> crate::sync::errors::SyncResult<()> {
        let token = self.access_token().await?;
        if _messages.is_empty() {
            return Ok(());
        }

        let target = _target.trim();
        if target.is_empty() {
            return Err("gmail move target is empty".into());
        }

        let (add_labels, remove_labels) = gmail_move_label_changes(_messages, target);

        let payload = serde_json::json!({
            "ids": _messages.iter().map(|message| message.remote_id.clone()).collect::<Vec<_>>(),
            "addLabelIds": add_labels,
            "removeLabelIds": remove_labels,
        });

        let client = crate::utils::http::client();
        let response = send_with_rate_limit_retry(
            || {
                client
                    .post(format!("{}/messages/batchModify", gmail_api_base()))
                    .bearer_auth(&token)
                    .json(&payload)
            },
            "move_messages",
            cost::BATCH_MODIFY,
        )
        .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(crate::sync::errors::http_failure(
                "gmail",
                "move_messages",
                status.as_u16(),
                &body,
            ));
        }

        Ok(())
    }

    async fn create_folder(&self, path: &str) -> crate::sync::errors::SyncResult<()> {
        let name = path.trim();
        if name.is_empty() {
            return Err("gmail label name is empty".into());
        }

        let token = self.access_token().await?;
        let client = crate::utils::http::client();
        let payload = serde_json::json!({
            "name": name,
            "labelListVisibility": "labelShow",
            "messageListVisibility": "show",
        });
        let response = send_with_rate_limit_retry(
            || {
                client
                    .post(format!("{}/labels", gmail_api_base()))
                    .bearer_auth(&token)
                    .json(&payload)
            },
            "label create",
            cost::LABELS_CREATE,
        )
        .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            let lower = body.to_ascii_lowercase();
            if status == reqwest::StatusCode::CONFLICT
                || lower.contains("label name exists")
                || lower.contains("already exists")
            {
                return Ok(());
            }
            return Err(crate::sync::errors::http_failure(
                "gmail",
                "label create",
                status.as_u16(),
                &body,
            ));
        }
        Ok(())
    }

    async fn save_draft(
        &self,
        raw_rfc822: &[u8],
        _folder_hint: Option<&str>,
    ) -> crate::sync::errors::SyncResult<Option<String>> {
        let token = self.access_token().await?;
        let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw_rfc822);
        let payload = serde_json::json!({"message": {"raw": raw}});

        let client = crate::utils::http::client();
        let response = send_with_rate_limit_retry(
            || {
                client
                    .post(format!("{}/drafts", gmail_api_base()))
                    .bearer_auth(&token)
                    .json(&payload)
            },
            "draft create",
            cost::DRAFTS_CREATE,
        )
        .await?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(crate::sync::errors::http_failure(
                "gmail",
                "draft create",
                status.as_u16(),
                &body,
            ));
        }
        let parsed: serde_json::Value = response
            .json()
            .await
            .map_err(|err| format!("gmail draft parse failed: {err}"))?;
        Ok(parsed
            .get("id")
            .and_then(|v| v.as_str())
            .map(str::to_string))
    }

    async fn upsert_draft(
        &self,
        message: OutgoingMessage,
        remote_draft_id: Option<&str>,
        _folder_hint: Option<&str>,
    ) -> crate::sync::errors::SyncResult<Option<String>> {
        let token = self.access_token().await?;
        let discovered_draft_id = if remote_draft_id.is_none() {
            if let Some(message_id) = message
                .message_id_header
                .as_deref()
                .filter(|value| !value.trim().is_empty())
            {
                Self::fetch_current_drafts(&crate::utils::http::client(), &token)
                    .await?
                    .into_iter()
                    .find(|draft| draft.message_id_header.as_deref() == Some(message_id))
                    .and_then(|draft| draft.remote_draft_id)
            } else {
                None
            }
        } else {
            None
        };
        let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(build_gmail_raw_message(&message)?.as_bytes());
        let payload = serde_json::json!({"message": {"raw": raw}});
        let client = crate::utils::http::client();
        let effective_draft_id = remote_draft_id
            .filter(|id| !id.trim().is_empty())
            .map(str::to_string)
            .or(discovered_draft_id);
        let response = send_with_rate_limit_retry(
            || {
                let request = if let Some(id) = effective_draft_id.as_deref() {
                    client
                        .put(format!("{}/drafts/{id}", gmail_api_base()))
                        .bearer_auth(&token)
                } else {
                    client
                        .post(format!("{}/drafts", gmail_api_base()))
                        .bearer_auth(&token)
                };
                request.json(&payload)
            },
            "draft upsert",
            cost::DRAFTS_UPDATE,
        )
        .await?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(crate::sync::errors::http_failure(
                "gmail",
                "draft upsert",
                status.as_u16(),
                &body,
            ));
        }
        let parsed: serde_json::Value = response
            .json()
            .await
            .map_err(|err| format!("gmail draft parse failed: {err}"))?;
        Ok(parsed
            .get("id")
            .and_then(|v| v.as_str())
            .map(str::to_string))
    }

    async fn resolve_draft_id(
        &self,
        message_remote_id: &str,
        _folder_hint: Option<&str>,
    ) -> crate::sync::errors::SyncResult<Option<String>> {
        let token = self.access_token().await?;
        let client = crate::utils::http::client();
        Ok(Self::fetch_current_drafts(&client, &token)
            .await?
            .into_iter()
            .find(|message| message.remote_id == message_remote_id)
            .and_then(|message| message.remote_draft_id))
    }

    async fn delete_draft(&self, remote_draft_id: &str) -> crate::sync::errors::SyncResult<()> {
        let token = self.access_token().await?;
        let client = crate::utils::http::client();
        let response = send_with_rate_limit_retry(
            || {
                client
                    .delete(format!("{}/drafts/{remote_draft_id}", gmail_api_base()))
                    .bearer_auth(&token)
            },
            "draft delete",
            cost::DRAFTS_DELETE,
        )
        .await?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(crate::sync::errors::http_failure(
                "gmail",
                "draft delete",
                status.as_u16(),
                &body,
            ));
        }
        Ok(())
    }

    async fn fetch_attachment(
        &self,
        message: &MessageRef,
        remote_part_id: &str,
    ) -> crate::sync::errors::SyncResult<FetchedAttachment> {
        let token = self.access_token().await?;
        let client = crate::utils::http::client();
        let response = send_with_rate_limit_retry(
            || {
                client
                    .get(format!(
                        "{}/messages/{}/attachments/{}",
                        gmail_api_base(),
                        message.remote_id,
                        remote_part_id
                    ))
                    .bearer_auth(&token)
            },
            "attachment fetch",
            cost::ATTACHMENTS_GET,
        )
        .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(crate::sync::errors::http_failure(
                "gmail",
                "attachment fetch",
                status.as_u16(),
                &body,
            ));
        }

        let payload: GmailAttachmentResponse = response
            .json()
            .await
            .map_err(|err| format!("gmail attachment parse failed: {err}"))?;
        let data = payload.data.unwrap_or_default();
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(data.as_bytes())
            .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(data.as_bytes()))
            .map_err(|err| format!("gmail attachment decode failed: {err}"))?;

        Ok(FetchedAttachment {
            filename: None,
            content_type: "application/octet-stream".to_string(),
            bytes,
        })
    }

    async fn delete_messages(
        &self,
        _messages: &[MessageRef],
    ) -> crate::sync::errors::SyncResult<()> {
        let token = self.access_token().await?;
        if _messages.is_empty() {
            return Ok(());
        }

        // messages.trash instead of batchDelete: batchDelete needs the full
        // https://mail.google.com/ scope (we only request gmail.modify → 403 on
        // every delete) and would bypass Trash irreversibly. Trash matches what
        // a mail client's "delete" means and is recoverable for 30 days.
        let client = crate::utils::http::client();
        for message in _messages {
            let response = send_with_rate_limit_retry(
                || {
                    client
                        .post(format!(
                            "{}/messages/{}/trash",
                            gmail_api_base(),
                            message.remote_id
                        ))
                        .bearer_auth(&token)
                },
                "delete_messages",
                cost::MESSAGES_TRASH,
            )
            .await?;

            let status = response.status();
            // Already gone (deleted elsewhere) is success for our purposes.
            if !status.is_success() && status.as_u16() != 404 {
                let body = response.text().await.unwrap_or_default();
                return Err(crate::sync::errors::http_failure(
                    "gmail",
                    "delete_messages",
                    status.as_u16(),
                    &body,
                ));
            }
        }

        Ok(())
    }
}

fn parse_gmail_cursor(cursor: Option<&str>) -> Option<u64> {
    cursor
        .and_then(|value| value.strip_prefix("ts:"))
        .and_then(|value| value.parse::<u64>().ok())
}

fn map_gmail_message(message: GmailMessageResponse) -> MessageEnvelope {
    let raw_body = message
        .raw
        .as_deref()
        .and_then(decode_gmail_body_bytes)
        .map(|raw| crate::providers::imap::parse_raw_message_body(&raw));
    let payload = message.payload.unwrap_or(GmailPayload {
        filename: None,
        mime_type: None,
        headers: None,
        body: None,
        parts: None,
    });
    let headers = payload.headers.clone().unwrap_or_default();
    let subject = header_value(&headers, "Subject").unwrap_or_else(|| "(no subject)".to_string());
    let from_raw =
        header_value(&headers, "From").unwrap_or_else(|| "unknown@gmail.local".to_string());
    let from_address = extract_email_address(&from_raw).unwrap_or_else(|| from_raw.clone());
    let from_name = display_name_from_mailbox(&from_raw, &from_address);
    let to_addresses = parse_gmail_address_list(header_value(&headers, "To").as_deref());
    let cc_addresses = parse_gmail_address_list(header_value(&headers, "Cc").as_deref());
    let bcc_addresses = parse_gmail_address_list(header_value(&headers, "Bcc").as_deref());
    let reply_to = header_value(&headers, "Reply-To").and_then(|raw| extract_email_address(&raw));
    let message_id_header = header_value(&headers, "Message-ID");
    let in_reply_to = header_value(&headers, "In-Reply-To");
    let references = header_value(&headers, "References")
        .map(|raw| split_references(&raw))
        .unwrap_or_default();
    let date_rfc3339 = message
        .internal_date
        .as_deref()
        .and_then(|value| value.parse::<i64>().ok())
        .and_then(|millis| {
            chrono::DateTime::<chrono::Utc>::from_timestamp_millis(millis)
                .map(|value| value.to_rfc3339())
        })
        .or_else(|| parse_date_header(header_value(&headers, "Date").as_deref()))
        .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());
    let mut body = extract_bodies(&payload);
    if let Some(parsed) = raw_body.as_ref() {
        body.text = if parsed.trust.root_crypto {
            parsed.text.clone()
        } else {
            parsed.text.clone().or(body.text)
        };
        body.html = if parsed.trust.root_crypto {
            parsed.html.clone()
        } else {
            parsed.html.clone().or(body.html)
        };
        body.calendar = parsed.calendar.clone().or(body.calendar);
    }
    let attachments = extract_attachments(&payload);
    let snippet = decode_html_entities(&decode_mime_encoded_words(
        message
            .snippet
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(&subject),
    ));

    let list_unsubscribe = header_value(&headers, "List-Unsubscribe");
    let list_unsubscribe_post = header_value(&headers, "List-Unsubscribe-Post");
    let (auth_spf, auth_dkim, auth_dmarc) = header_value(&headers, "Authentication-Results")
        .map(|value| crate::providers::traits::trusted_authentication_results(&value, "gmail"))
        .unwrap_or((None, None, None));

    let mailbox_ids = message.label_ids.unwrap_or_default();
    let is_read = !mailbox_ids
        .iter()
        .any(|label| label.eq_ignore_ascii_case("UNREAD"));
    let is_starred = mailbox_ids
        .iter()
        .any(|label| label.eq_ignore_ascii_case("STARRED"));

    let smime_encrypted = raw_body
        .as_ref()
        .and_then(|parsed| parsed.smime_encrypted.clone());
    let smime_signed = raw_body
        .as_ref()
        .and_then(|parsed| parsed.smime_signed.clone());
    let mut trust = raw_body
        .as_ref()
        .map(|parsed| parsed.trust.clone())
        .unwrap_or_else(|| {
            super::imap::root_crypto_trust(
                payload.mime_type.as_deref().unwrap_or_default(),
                &super::imap::ParsedBody {
                    text: body.text.clone(),
                    html: body.html.clone(),
                    ..Default::default()
                },
            )
        });
    trust.auth_verified = auth_spf.is_some() || auth_dkim.is_some() || auth_dmarc.is_some();
    let pgp_signed = raw_body.and_then(|parsed| parsed.pgp_signed);

    MessageEnvelope {
        remote_id: message.id,
        remote_uidvalidity: None,
        remote_thread_id: message.thread_id,
        original_source: None,
        mailbox_ids,
        message_id_header,
        in_reply_to,
        references,
        subject: decode_mime_encoded_words(&subject),
        snippet,
        from_address,
        from_name,
        to_addresses,
        cc_addresses,
        bcc_addresses,
        reply_to,
        remote_draft_id: None,
        received_at_rfc3339: message
            .internal_date
            .as_deref()
            .and_then(|value| value.parse::<i64>().ok())
            .and_then(chrono::DateTime::<chrono::Utc>::from_timestamp_millis)
            .map(|value| value.to_rfc3339()),
        date_rfc3339,
        body_text: body.text,
        body_html: body.html,
        is_read,
        is_starred,
        body_calendar: body.calendar,
        smime_encrypted,
        smime_signed,
        pgp_signed,
        trust,
        list_unsubscribe,
        list_unsubscribe_post,
        auth_spf,
        auth_dkim,
        auth_dmarc,
        attachments,
    }
}

/// Full responses must contain actual MIME structure before defaults in the
/// mapper are safe. Empty text is valid; an absent body is not evidence of it.
fn validate_gmail_full_message(message: &GmailMessageResponse) -> Result<(), String> {
    let invalid = || "gmail message lookup returned an incomplete message".to_string();
    let payload = message.payload.as_ref().ok_or_else(invalid)?;
    let headers = payload
        .headers
        .as_deref()
        .filter(|h| !h.is_empty())
        .ok_or_else(invalid)?;
    // An omitted label list is not proof of an empty one: the caller confirms
    // it with a label-only lookup.
    if !gmail_identity_is_complete(message)
        || message
            .label_ids
            .as_ref()
            .is_some_and(|labels| labels.iter().any(|label| label.trim().is_empty()))
        || !gmail_payload_is_usable(payload)
    {
        return Err(invalid());
    }
    let date_is_valid = match message.internal_date.as_deref() {
        Some(date) => date
            .parse::<i64>()
            .ok()
            .and_then(chrono::DateTime::<chrono::Utc>::from_timestamp_millis)
            .is_some(),
        None => parse_date_header(header_value(headers, "Date").as_deref()).is_some(),
    };
    if !date_is_valid {
        return Err("gmail message lookup returned an invalid or missing date".into());
    }
    Ok(())
}

/// Every message resource names its thread and its last history record.
/// Without them a response is a fragment, whatever else it carries.
fn gmail_identity_is_complete(message: &GmailMessageResponse) -> bool {
    !message.id.trim().is_empty()
        && message
            .thread_id
            .as_deref()
            .is_some_and(|thread| !thread.trim().is_empty())
        && message
            .history_id
            .as_deref()
            .is_some_and(|history| history.parse::<u64>().is_ok())
}

fn gmail_header_name_is_valid(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| (33..=126).contains(&byte) && byte != b':')
}

fn gmail_payload_is_usable(payload: &GmailPayload) -> bool {
    let Some(mime) = payload.mime_type.as_deref() else {
        return false;
    };
    if !mime.split_once('/').is_some_and(|(kind, subtype)| {
        !kind.is_empty() && !subtype.is_empty() && !mime.bytes().any(|b| b.is_ascii_whitespace())
    }) || payload.headers.as_ref().is_some_and(|headers| {
        headers
            .iter()
            .any(|header| !gmail_header_name_is_valid(&header.name) || header.value.contains('\0'))
    }) {
        return false;
    }
    let parts = payload.parts.as_deref().unwrap_or_default();
    if mime.to_ascii_lowercase().starts_with("multipart/") || !parts.is_empty() {
        return !parts.is_empty() && parts.iter().all(gmail_payload_is_usable);
    }
    let Some(body) = payload.body.as_ref() else {
        return false;
    };
    if body.size.is_some_and(|size| size < 0) {
        return false;
    }
    match body.data.as_deref() {
        Some(data) => decode_gmail_body_bytes(data).is_some(),
        None => {
            body.attachment_id
                .as_ref()
                .is_some_and(|id| !id.trim().is_empty())
                || body.size == Some(0)
        }
    }
}

/// Base64 alone is not a message. Require RFC headers and a complete container
/// before allowing the permissive MIME body parser to replace cached content.
fn validate_gmail_raw_source(source: &str) -> Result<(), String> {
    let invalid = || "gmail response contained invalid message source".to_string();
    let bytes = decode_gmail_body_bytes(source).ok_or_else(invalid)?;
    let parsed = super::imap::parse_raw_rfc822(&bytes).map_err(|_| invalid())?;
    let mut fields = 0;
    for line in parsed.raw_headers.lines() {
        if line.starts_with([' ', '\t']) && fields > 0 {
            continue;
        }
        let Some((name, _)) = line.split_once(':') else {
            return Err(invalid());
        };
        if !gmail_header_name_is_valid(name) || line.contains('\0') {
            return Err(invalid());
        }
        fields += 1;
    }
    if fields == 0 {
        return Err(invalid());
    }
    if let Some(content_type) = super::imap::parse_header_value(&parsed.raw_headers, "Content-Type")
    {
        if content_type.to_ascii_lowercase().starts_with("multipart/") {
            let boundary = content_type
                .split(';')
                .skip(1)
                .find_map(|param| {
                    let (name, value) = param.trim().split_once('=')?;
                    name.eq_ignore_ascii_case("boundary")
                        .then(|| value.trim().trim_matches('"'))
                })
                .filter(|value| !value.is_empty())
                .ok_or_else(invalid)?;
            let text = String::from_utf8_lossy(&bytes);
            if !text
                .lines()
                .any(|line| line.trim_end() == format!("--{boundary}"))
                || !text
                    .lines()
                    .any(|line| line.trim_end() == format!("--{boundary}--"))
            {
                return Err(invalid());
            }
        }
    }
    Ok(())
}

fn gmail_message_needs_raw_crypto(message: &GmailMessageResponse) -> bool {
    let Some(payload) = message.payload.as_ref() else {
        return false;
    };
    let mime = payload.mime_type.as_deref().unwrap_or_default();
    let content_type = payload
        .headers
        .as_deref()
        .and_then(|headers| header_value(headers, "Content-Type"))
        .unwrap_or_default()
        .to_ascii_lowercase();
    (mime.eq_ignore_ascii_case("multipart/signed")
        && (content_type.contains("application/pgp-signature")
            || content_type.contains("application/pkcs7-signature")
            || content_type.contains("application/x-pkcs7-signature")))
        || (mime.eq_ignore_ascii_case("multipart/encrypted")
            && content_type.contains("application/pgp-encrypted"))
        || mime.eq_ignore_ascii_case("application/pkcs7-mime")
        || mime.eq_ignore_ascii_case("application/x-pkcs7-mime")
}

/// Gmail can transcode text in `format=full` while retaining the original
/// Content-Type charset. Only the original MIME source binds those headers to
/// the bytes they describe. Reuse the raw parser instead of guessing whether
/// apparently valid UTF-8 is original legacy text or Gmail's transcoded text.
fn gmail_body_needs_raw_source(payload: &GmailPayload) -> bool {
    let mime = payload.mime_type.as_deref().unwrap_or_default();
    if (mime.eq_ignore_ascii_case("text/plain")
        || mime.eq_ignore_ascii_case("text/html")
        || mime.eq_ignore_ascii_case("text/calendar"))
        && payload
            .body
            .as_ref()
            .is_some_and(|body| body.data.is_some())
    {
        let charset = payload
            .headers
            .as_deref()
            .and_then(|headers| header_value(headers, "Content-Type"))
            .and_then(|content_type| extract_charset_param(&content_type));
        let needs_original_charset = match charset {
            Some(charset) => {
                !charset.eq_ignore_ascii_case("utf-8") && !charset.eq_ignore_ascii_case("utf8")
            }
            None => payload
                .body
                .as_ref()
                .and_then(|body| body.data.as_deref())
                .and_then(decode_gmail_body_bytes)
                .is_some_and(|bytes| !bytes.is_ascii()),
        };
        if needs_original_charset {
            return true;
        }
    }
    payload
        .parts
        .as_deref()
        .unwrap_or_default()
        .iter()
        .any(gmail_body_needs_raw_source)
}

/// Gmail labels that only restate a message's own state.
///
/// Read, starred and important are columns on the message, not places mail
/// lives. Satchel gives messages without a primary mailbox a synthetic archive
/// home. Syncing these views stored a second full copy of the message per label,
/// because `provider_message_db_id` scopes the row id by folder, and cost a
/// backfill page per label on every run.
///
/// Deliberately does NOT include `CATEGORY_*`: archived mail loses `INBOX` but
/// keeps its category, so for that mail the category label is the only place it
/// exists. The classifier also reads it (`synthetic_classification_headers`).
pub fn gmail_label_is_state_view(id: &str) -> bool {
    matches!(
        id.trim().to_ascii_uppercase().as_str(),
        "UNREAD" | "STARRED" | "IMPORTANT" | "CHAT"
    )
}

fn gmail_system_folder_kind(id: &str) -> Option<&'static str> {
    match id.to_ascii_uppercase().as_str() {
        "INBOX" => Some("inbox"),
        "SENT" => Some("sent"),
        "DRAFT" | "DRAFTS" => Some("drafts"),
        "TRASH" => Some("trash"),
        "SPAM" => Some("spam"),
        _ => None,
    }
}

fn split_references(raw: &str) -> Vec<String> {
    raw.split_whitespace()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn header_value(headers: &[GmailHeader], key: &str) -> Option<String> {
    headers
        .iter()
        .find(|header| header.name.eq_ignore_ascii_case(key))
        .map(|header| decode_mime_encoded_words(header.value.trim()))
}

fn parse_gmail_address_list(raw: Option<&str>) -> Vec<String> {
    parse_address_list_tokens(
        raw.unwrap_or_default(),
        AddressListDecodeStrategy::MimeEncodedWords,
    )
}

fn display_name_from_mailbox(raw: &str, address: &str) -> Option<String> {
    let cleaned = raw
        .replace(address, "")
        .replace(['<', '>', '"'], "")
        .trim()
        .to_string();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

fn parse_date_header(raw: Option<&str>) -> Option<String> {
    raw.and_then(|value| chrono::DateTime::parse_from_rfc2822(value).ok())
        .map(|value| value.with_timezone(&chrono::Utc).to_rfc3339())
}

fn extract_bodies(payload: &GmailPayload) -> ExtractedBody {
    let mut result = ExtractedBody::default();
    extract_bodies_recursive(payload, &mut result);
    result
}

fn extract_bodies_recursive(payload: &GmailPayload, result: &mut ExtractedBody) {
    let mime_type = payload
        .mime_type
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if let Some(data) = payload.body.as_ref().and_then(|body| body.data.as_deref()) {
        let decoded = decode_gmail_part(payload, data);
        if !decoded.trim().is_empty() {
            if mime_type == "text/plain" && result.text.is_none() {
                result.text = Some(decoded);
            } else if mime_type == "text/html" && result.html.is_none() {
                result.html = Some(decoded);
            } else if mime_type.starts_with("text/calendar") && result.calendar.is_none() {
                result.calendar = Some(decoded);
            }
        }
    }

    for part in payload.parts.as_deref().unwrap_or(&[]) {
        extract_bodies_recursive(part, result);
    }
}

fn extract_attachments(payload: &GmailPayload) -> Vec<RemoteAttachment> {
    let mut result = Vec::new();
    extract_attachments_recursive(payload, &mut result);
    result
}

fn extract_attachments_recursive(payload: &GmailPayload, result: &mut Vec<RemoteAttachment>) {
    let filename = payload
        .filename
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(decode_mime_encoded_words);
    let attachment_id = payload
        .body
        .as_ref()
        .and_then(|body| body.attachment_id.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty());

    if let Some(remote_part_id) = attachment_id {
        let content_type = crate::email::mime_type::normalize_mime_type(
            payload.mime_type.as_deref().unwrap_or_default(),
        );
        let headers = payload.headers.clone().unwrap_or_default();
        let disposition = header_value(&headers, "Content-Disposition")
            .unwrap_or_default()
            .to_ascii_lowercase();
        let content_id = header_value(&headers, "Content-ID")
            .map(|value| value.trim_matches(['<', '>']).to_string())
            .filter(|value| !value.is_empty());
        let is_inline = disposition.contains("inline") || content_id.is_some();
        result.push(RemoteAttachment {
            remote_part_id: remote_part_id.to_string(),
            filename,
            content_type,
            size: payload.body.as_ref().and_then(|body| body.size),
            content_id,
            is_inline,
            content_base64: normalized_attachment_base64(payload),
        });
    }

    for part in payload.parts.as_deref().unwrap_or(&[]) {
        extract_attachments_recursive(part, result);
    }
}

/// `RemoteAttachment.content_base64` must hold final attachment bytes in
/// standard base64 — the IMAP path stores bytes the same way. Gmail has
/// already applied Content-Transfer-Encoding to `body.data`; the remaining
/// base64url wrapper is the API transport encoding, not the MIME encoding.
fn normalized_attachment_base64(payload: &GmailPayload) -> Option<String> {
    let data = payload.body.as_ref()?.data.as_deref()?;
    let Some(bytes) = decode_gmail_body_bytes(data) else {
        return Some(data.to_string());
    };
    Some(base64::engine::general_purpose::STANDARD.encode(bytes))
}

/// Gmail's `body.data` is base64url-wrapped for the JSON API, but Gmail has
/// already applied the MIME Content-Transfer-Encoding. The original CTE header
/// is still present as metadata and must not be applied a second time: literal
/// text such as a URL parameter `=DE` would otherwise become byte 0xDE and
/// invalidate an otherwise correct UTF-8 body.
fn decode_gmail_part(payload: &GmailPayload, data: &str) -> String {
    let bytes = decode_gmail_body_bytes(data);
    let Some(bytes) = bytes else {
        return String::new();
    };

    let charset = payload
        .headers
        .as_deref()
        .and_then(|headers| header_value(headers, "Content-Type"))
        .and_then(|content_type| extract_charset_param(&content_type));
    let Some(charset) = charset else {
        if bytes.is_ascii() {
            return String::from_utf8(bytes).expect("ASCII is valid UTF-8");
        }
        tracing::warn!("gmail body contains non-ASCII bytes but declares no charset");
        return String::new();
    };
    match decode_bytes_with_declared_charset(&bytes, &charset) {
        Ok((decoded, false)) => decoded,
        Ok((decoded, true)) => {
            tracing::warn!(
                charset,
                "gmail body contains bytes invalid for its declared charset"
            );
            decoded
        }
        Err(err) => {
            tracing::warn!(charset, %err, "gmail body declares an unsupported charset");
            String::new()
        }
    }
}

/// Minimal `charset=` parameter lookup for Content-Type headers.
fn extract_charset_param(content_type: &str) -> Option<String> {
    for raw_part in content_type.split(';').skip(1) {
        let Some((name, value)) = raw_part.split_once('=') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case("charset") {
            let trimmed = value.trim().trim_matches('"').trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn decode_gmail_body_bytes(data: &str) -> Option<Vec<u8>> {
    // Gmail sometimes omits base64 padding; fall back to NO_PAD.
    base64::engine::general_purpose::URL_SAFE
        .decode(data)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(data))
        .ok()
}

fn build_gmail_raw_message(
    message: &OutgoingMessage,
) -> Result<String, crate::email::build::BuildError> {
    if let Some(raw) = message
        .raw_rfc822_override
        .as_deref()
        .filter(|s| !s.trim().is_empty())
    {
        return Ok(raw.to_string());
    }
    // Gmail's messages.send derives recipients from the headers and strips Bcc
    // on delivery — without the Bcc header, Bcc recipients silently get nothing.
    crate::email::build::build_rfc822_with_bcc(message, None, None)
}

#[cfg(test)]
mod tests {
    use super::{
        build_gmail_raw_message, cost, decode_gmail_part, gmail_message_needs_raw_crypto,
        gmail_move_label_changes, is_rate_limited, map_gmail_message, rate_limit_delay,
        BackfillStep, GmailCursorState, GmailHeader, GmailHistoryResponse, GmailMessageResponse,
        GmailPayload, GmailProvider, Priority, QuotaPacer, BACKGROUND_COOLDOWN,
        BACKGROUND_UNITS_PER_SECOND, BLOCK_LAPSE_MARGIN, MAX_BLOCK_WAIT, MAX_RATE_LIMIT_WAIT,
        QUOTA_BURST_UNITS, QUOTA_UNITS_PER_SECOND, REPEAT_BLOCK_WINDOW,
    };
    use crate::providers::traits::{MessageRef, OutgoingMessage, SyncCheckpoint};
    use base64::Engine;

    #[test]
    fn rejects_markup_in_attachment_mime_types() {
        let mut payload = payload_with_headers(
            r#"image/png"><link rel=preconnect href=https://u1.t.example>"#,
            &[],
        );
        payload.body = Some(super::GmailBody {
            attachment_id: Some("x".into()),
            size: Some(4),
            data: None,
        });
        assert_eq!(
            super::extract_attachments(&payload)[0].content_type,
            "application/octet-stream"
        );
    }

    #[test]
    fn a_prebuilt_payload_keeps_the_bcc_header_the_outbox_put_on_it() {
        // messages.send derives its recipients from the headers, so the crypto
        // payload arrives with Bcc already in its delivery headers. Passing it
        // through untouched is what makes those recipients reachable.
        let raw = "From: me@example.test\r\nTo: her@example.test\r\n\
                   Bcc: chef@example.test\r\n\r\nencrypted";
        let message = OutgoingMessage {
            to: vec!["her@example.test".to_owned()],
            bcc: vec!["chef@example.test".to_owned()],
            body: "the plaintext that must not be rebuilt".to_owned(),
            raw_rfc822_override: Some(raw.to_owned()),
            ..Default::default()
        };

        assert_eq!(build_gmail_raw_message(&message).unwrap(), raw);
    }

    #[test]
    fn a_send_without_a_prebuilt_payload_gets_its_bcc_from_the_builder() {
        let message = OutgoingMessage {
            to: vec!["her@example.test".to_owned()],
            bcc: vec!["chef@example.test".to_owned()],
            from_address: Some("me@example.test".to_owned()),
            body: "plain text".to_owned(),
            ..Default::default()
        };

        assert!(build_gmail_raw_message(&message)
            .unwrap()
            .contains("Bcc: chef@example.test"));
    }

    fn payload_with_headers(mime_type: &str, headers: &[(&str, &str)]) -> GmailPayload {
        GmailPayload {
            filename: None,
            mime_type: Some(mime_type.to_string()),
            headers: Some(
                headers
                    .iter()
                    .map(|(name, value)| GmailHeader {
                        name: (*name).to_string(),
                        value: (*value).to_string(),
                    })
                    .collect(),
            ),
            body: None,
            parts: None,
        }
    }

    #[test]
    fn raw_mime_is_requested_for_signed_and_encrypted_messages() {
        for (mime_type, protocol) in [
            ("multipart/signed", "application/pgp-signature"),
            ("multipart/encrypted", "application/pgp-encrypted"),
            ("multipart/signed", "application/pkcs7-signature"),
            ("multipart/signed", "application/x-pkcs7-signature"),
        ] {
            let message = GmailMessageResponse {
                id: "message".to_string(),
                thread_id: None,
                history_id: None,
                label_ids: None,
                snippet: None,
                internal_date: None,
                payload: Some(payload_with_headers(
                    mime_type,
                    &[(
                        "Content-Type",
                        &format!("{mime_type}; protocol=\"{protocol}\"; boundary=x"),
                    )],
                )),
                raw: None,
            };
            assert!(gmail_message_needs_raw_crypto(&message));
        }
    }

    fn base64url(data: &[u8]) -> String {
        base64::engine::general_purpose::URL_SAFE.encode(data)
    }

    #[test]
    fn raw_legacy_smime_is_mapped_to_the_verification_payload() {
        let raw = b"Content-Type: multipart/signed; protocol=\"application/x-pkcs7-signature\"; boundary=smime\r\n\r\n\
--smime\r\nContent-Type: text/plain\r\n\r\nSigned body\r\n\
--smime\r\nContent-Type: application/x-pkcs7-signature\r\n\r\nAQID\r\n--smime--\r\n";
        let message = GmailMessageResponse {
            id: "signed-message".to_string(),
            thread_id: None,
            history_id: None,
            label_ids: None,
            snippet: Some("Signed body".to_string()),
            internal_date: None,
            payload: Some(payload_with_headers(
                "multipart/signed",
                &[
                    ("From", "sender@example.com"),
                    ("Content-Type", "multipart/signed; protocol=\"application/x-pkcs7-signature\"; boundary=smime"),
                ],
            )),
            raw: Some(base64url(raw)),
        };

        let mapped = map_gmail_message(message);

        assert!(mapped.smime_signed.is_some());
        assert!(mapped.pgp_signed.is_none());
    }

    #[test]
    fn does_not_apply_gmails_quoted_printable_header_twice() {
        let raw = "<p>Apple\u{00a0}Account: Verlängerung 9,99\u{00a0}€</p>\
                   <a href=\"https://example.test/account?country=DE&lang=de\">Link</a>";
        let payload = payload_with_headers(
            "text/html",
            &[
                ("Content-Type", "text/html; charset=\"utf-8\""),
                ("Content-Transfer-Encoding", "quoted-printable"),
            ],
        );

        assert_eq!(decode_gmail_part(&payload, &base64url(raw.as_bytes())), raw);
    }

    #[test]
    fn decodes_transfer_decoded_part_with_latin1_charset() {
        let raw = b"Gr\xFC\xDFe aus K\xF6ln";
        let payload = payload_with_headers(
            "text/plain",
            &[
                ("Content-Type", "text/plain; charset=iso-8859-1"),
                ("Content-Transfer-Encoding", "quoted-printable"),
            ],
        );

        assert_eq!(
            decode_gmail_part(&payload, &base64url(raw)),
            "Grüße aus Köln"
        );
    }

    #[test]
    fn does_not_guess_windows_1252_when_gmail_declares_utf8() {
        let raw = b"Freundliche Gr\xFC\xDFe";
        let payload = payload_with_headers(
            "text/plain",
            &[("Content-Type", "text/plain; charset=utf-8")],
        );

        assert_eq!(
            decode_gmail_part(&payload, &base64url(raw)),
            "Freundliche Gr��e"
        );
    }

    #[test]
    fn rejects_unknown_declared_gmail_charset_instead_of_guessing_utf8() {
        let payload = payload_with_headers(
            "text/plain",
            &[("Content-Type", "text/plain; charset=x-made-up-charset")],
        );

        assert_eq!(decode_gmail_part(&payload, &base64url(b"hello")), "");
    }

    #[test]
    fn accepts_unambiguous_ascii_without_a_declared_charset() {
        let payload = payload_with_headers("text/plain", &[("Content-Type", "text/plain")]);

        assert_eq!(decode_gmail_part(&payload, &base64url(b"hello")), "hello");
    }

    #[test]
    fn rejects_non_ascii_without_a_declared_charset_instead_of_guessing_utf8() {
        let payload = payload_with_headers("text/plain", &[("Content-Type", "text/plain")]);

        assert_eq!(
            decode_gmail_part(&payload, &base64url("Grüße".as_bytes())),
            ""
        );
    }

    #[test]
    fn leaves_unencoded_part_untouched() {
        let raw = b"<p>plain html with = and == signs stays</p>";
        let payload =
            payload_with_headers("text/html", &[("Content-Type", "text/html; charset=utf-8")]);

        assert_eq!(
            decode_gmail_part(&payload, &base64url(raw)),
            "<p>plain html with = and == signs stays</p>"
        );
    }

    #[test]
    fn does_not_apply_gmails_quoted_printable_attachment_header_twice() {
        use base64::engine::general_purpose::STANDARD;

        let raw = b"country=DE&lang=de";
        let mut payload = payload_with_headers(
            "application/octet-stream",
            &[("Content-Transfer-Encoding", "quoted-printable")],
        );
        payload.body = Some(super::GmailBody {
            attachment_id: Some("att-1".to_string()),
            size: Some(raw.len() as i64),
            data: Some(base64url(raw)),
        });

        let attachments = super::extract_attachments(&payload);
        assert_eq!(attachments.len(), 1);
        let decoded = STANDARD
            .decode(attachments[0].content_base64.as_deref().expect("content"))
            .expect("valid standard base64");
        assert_eq!(decoded, raw);
    }

    #[test]
    fn keeps_attachment_bytes_untouched_without_qp_encoding() {
        use base64::engine::general_purpose::STANDARD;

        let raw = b"\x00\x01binary\xff";
        let mut payload = payload_with_headers("application/octet-stream", &[]);
        payload.body = Some(super::GmailBody {
            attachment_id: Some("att-2".to_string()),
            size: Some(raw.len() as i64),
            data: Some(base64url(raw)),
        });

        let attachments = super::extract_attachments(&payload);
        assert_eq!(attachments.len(), 1);
        let decoded = STANDARD
            .decode(attachments[0].content_base64.as_deref().unwrap())
            .unwrap();
        assert_eq!(decoded, raw);
    }

    #[test]
    fn synthetic_archive_is_never_sent_as_a_gmail_label_id() {
        let refs = vec![MessageRef {
            remote_id: "message".to_string(),
            source_folder: Some("ARCHIVE".to_string()),
            source_uidvalidity: None,
        }];
        let (add, remove) = gmail_move_label_changes(&refs, "INBOX");
        assert_eq!(add, vec!["INBOX"]);
        assert!(remove.is_empty());
    }

    /// Build the checkpoint a previous run would have persisted.
    fn checkpoint(state: &GmailCursorState) -> SyncCheckpoint {
        SyncCheckpoint {
            cursor: GmailProvider::finalize_cursor(state.clone()),
        }
    }

    fn plan_for(state: &GmailCursorState) -> super::SyncPlan {
        GmailProvider::plan_sync(&GmailProvider::parse_cursor_state(&checkpoint(state)))
    }

    /// The regression this whole refactor exists for.
    ///
    /// Backfill and incremental sync used to be the two arms of one `if`, so a
    /// folder with a pending backfill token never called `history.list` — the
    /// only path that fetches *new* mail. Backfill advances one 100-message page
    /// per run, so a large label stalled new mail for days.
    #[test]
    fn a_pending_backfill_still_runs_the_incremental_pass() {
        let plan = plan_for(&GmailCursorState {
            latest_ts: 1_785_000_000,
            history_id: Some("500".into()),
            backfill_page_token: Some("tok".into()),
            backfill_remaining_messages: 900,
            format_version: GmailProvider::CURSOR_FORMAT_VERSION,
            ..Default::default()
        });

        assert_eq!(plan.incremental_from.as_deref(), Some("500"));
        assert_eq!(plan.backfill, BackfillStep::Continue("tok".into()));
    }

    /// A cursorless folder must not run the `after:` catch-up: with no lower
    /// bound it pages the whole label, downloading the mailbox in one run
    /// before the backfill gets a turn.
    #[test]
    fn a_first_run_backfills_without_a_catch_up_list() {
        let plan = GmailProvider::plan_sync(&GmailProvider::parse_cursor_state(&SyncCheckpoint {
            cursor: None,
        }));

        assert_eq!(plan.incremental_from, None);
        assert_eq!(plan.catch_up_from, None);
        assert_eq!(plan.backfill, BackfillStep::Bootstrap);
    }

    #[test]
    fn a_legacy_timestamp_cursor_bootstraps_backfill_and_catches_up() {
        let plan = GmailProvider::plan_sync(&GmailProvider::parse_cursor_state(&SyncCheckpoint {
            cursor: Some("ts:1700000000".into()),
        }));

        assert_eq!(plan.catch_up_from, Some(1_700_000_000));
        assert_eq!(plan.backfill, BackfillStep::Bootstrap);
    }

    /// An expired history cursor leaves a gap the incremental pass cannot see:
    /// `after:` filters on `internalDate`, so mail labelled while the cursor was
    /// dead never matches. Only a fresh listing of the label finds it.
    #[test]
    fn an_expired_history_forces_a_backfill_bootstrap() {
        let plan = plan_for(&GmailCursorState {
            latest_ts: 1_785_000_000,
            history_id: Some("900".into()),
            history_expired: true,
            format_version: GmailProvider::CURSOR_FORMAT_VERSION,
            ..Default::default()
        });

        assert_eq!(plan.backfill, BackfillStep::Bootstrap);
    }

    /// The flag must not restart the backfill it started. A run that already
    /// holds a page token continues from it, or the label is listed from the
    /// newest page again on every run for as long as the backfill lasts.
    #[test]
    fn an_expired_history_resumes_a_running_backfill_instead_of_restarting_it() {
        let plan = plan_for(&GmailCursorState {
            latest_ts: 1_785_000_000,
            history_id: Some("900".into()),
            history_expired: true,
            backfill_page_token: Some("page-2".into()),
            format_version: GmailProvider::CURSOR_FORMAT_VERSION,
            ..Default::default()
        });

        assert_eq!(plan.backfill, BackfillStep::Continue("page-2".into()));
    }

    #[test]
    fn the_expiry_flag_survives_a_cursor_round_trip_and_defaults_to_false() {
        let state = GmailCursorState {
            latest_ts: 1_785_000_000,
            history_expired: true,
            ..Default::default()
        };
        let persisted = GmailProvider::finalize_cursor(state).expect("cursor persists");
        let parsed = GmailProvider::parse_cursor_state(&SyncCheckpoint {
            cursor: Some(persisted),
        });
        assert!(parsed.state.history_expired);

        // A cursor written before this field existed still deserializes.
        let legacy = GmailProvider::parse_cursor_state(&SyncCheckpoint {
            cursor: Some(r#"{"latest_ts":1785000000,"format_version":2}"#.into()),
        });
        assert!(!legacy.state.history_expired);
        assert_eq!(legacy.state.format_version, 2);
    }

    #[test]
    fn a_finished_backfill_never_restarts() {
        let plan = plan_for(&GmailCursorState {
            latest_ts: 1_785_000_000,
            history_id: Some("900".into()),
            format_version: GmailProvider::CURSOR_FORMAT_VERSION,
            ..Default::default()
        });

        assert_eq!(plan.backfill, BackfillStep::Skip);
        assert_eq!(plan.incremental_from.as_deref(), Some("900"));
    }

    #[test]
    fn an_older_format_restarts_backfill_from_the_newest_page() {
        let plan = plan_for(&GmailCursorState {
            latest_ts: 1_785_000_000,
            history_id: Some("900".into()),
            backfill_page_token: Some("old-page".into()),
            format_version: GmailProvider::CURSOR_FORMAT_VERSION - 1,
            ..Default::default()
        });

        assert_eq!(plan.incremental_from.as_deref(), Some("900"));
        assert_eq!(plan.catch_up_from, Some(1_785_000_000));
        assert_eq!(plan.backfill, BackfillStep::Bootstrap);
    }

    /// Pins the three lines that used to clear the backfill state at the end of
    /// the incremental branch. They were dead code while the branches were
    /// exclusive; interleaved, they would run every time and abandon the
    /// backfill after a single page.
    #[test]
    fn an_incremental_pass_never_clears_a_pending_backfill_token() {
        let state = GmailCursorState {
            latest_ts: 1_785_000_000,
            history_id: Some("500".into()),
            backfill_page_token: Some("tok".into()),
            backfill_remaining_messages: 900,
            backfill_total_messages: 1000,
            ..Default::default()
        };
        let round_tripped = GmailProvider::parse_cursor_state(&checkpoint(&state)).state;

        assert_eq!(round_tripped.backfill_page_token.as_deref(), Some("tok"));
        assert_eq!(round_tripped.backfill_remaining_messages, 900);
    }

    /// `newest_history_id` picks the numeric maximum, so a backfilled page —
    /// whose per-message ids are ancient by construction — cannot move the
    /// cursor backwards regardless of argument order.
    #[test]
    fn a_backfilled_page_cannot_rewind_the_history_cursor() {
        assert_eq!(
            GmailProvider::newest_history_id(Some("948213".into()), Some("120".into())),
            Some("948213".into())
        );
        assert_eq!(
            GmailProvider::newest_history_id(Some("120".into()), Some("948213".into())),
            Some("948213".into())
        );
    }

    /// An empty folder has no timestamp and no backfill token. Dropping its
    /// cursor made it re-bootstrap on every run — messages.list plus getProfile,
    /// forever.
    #[test]
    fn a_cursor_with_only_a_history_id_survives_a_round_trip() {
        let state = GmailCursorState {
            history_id: Some("777".into()),
            ..Default::default()
        };
        let persisted = GmailProvider::finalize_cursor(state);
        assert!(persisted.is_some(), "history-id-only cursor must persist");

        let parsed = GmailProvider::parse_cursor_state(&SyncCheckpoint { cursor: persisted });
        assert!(parsed.had_checkpoint);
        assert_eq!(parsed.state.history_id.as_deref(), Some("777"));
    }

    /// State views are dropped; CATEGORY_* is not. Archived mail loses INBOX
    /// but keeps its category, so the category label is the only place that
    /// mail lives — and the classifier reads it.
    #[test]
    fn state_labels_are_dropped_but_category_labels_are_kept() {
        for dropped in ["UNREAD", "STARRED", "IMPORTANT", "CHAT", "unread"] {
            assert!(
                super::gmail_label_is_state_view(dropped),
                "{dropped} should not be synced as a folder"
            );
        }
        for kept in [
            "INBOX",
            "SENT",
            "TRASH",
            "SPAM",
            "CATEGORY_PROMOTIONS",
            "CATEGORY_PERSONAL",
            "Label_42",
        ] {
            assert!(
                !super::gmail_label_is_state_view(kept),
                "{kept} must stay a real folder"
            );
        }
    }

    /// Misreading a throttle as an expiry would re-list the entire folder on
    /// every 429 — the opposite of backing off.
    #[test]
    fn an_expired_history_cursor_is_recognised_but_a_throttle_is_not() {
        assert!(GmailProvider::history_cursor_expired(
            "gmail history request failed with status 404: {\"error\":{\"code\":404}}"
        ));
        assert!(GmailProvider::history_cursor_expired(
            "gmail history request failed with status 410: gone"
        ));
        assert!(GmailProvider::history_cursor_expired(
            "gmail history failed: failedPrecondition"
        ));
        assert!(!GmailProvider::history_cursor_expired(
            "gmail history request skipped: rate limited for another 900s"
        ));
    }

    #[test]
    fn gmail_mapping_preserves_reply_to_bcc_and_deletion_history() {
        let message = map_gmail_message(GmailMessageResponse {
            id: "message".to_string(),
            thread_id: Some("thread".to_string()),
            history_id: Some("2".to_string()),
            label_ids: Some(vec!["DRAFT".to_string()]),
            snippet: Some("Body".to_string()),
            internal_date: Some("1784196000000".to_string()),
            raw: None,
            payload: Some(GmailPayload {
                filename: None,
                mime_type: Some("text/plain".to_string()),
                headers: Some(vec![
                    super::GmailHeader {
                        name: "From".to_string(),
                        value: "Sender <sender@example.com>".to_string(),
                    },
                    super::GmailHeader {
                        name: "Reply-To".to_string(),
                        value: "Replies <replies@example.net>".to_string(),
                    },
                    super::GmailHeader {
                        name: "Bcc".to_string(),
                        value: "hidden@example.com".to_string(),
                    },
                ]),
                body: None,
                parts: None,
            }),
        });
        assert_eq!(message.reply_to.as_deref(), Some("replies@example.net"));
        assert_eq!(message.bcc_addresses, vec!["hidden@example.com"]);

        let history: GmailHistoryResponse = serde_json::from_value(serde_json::json!({
            "history": [{
                "messagesDeleted": [{"message": {"id": "deleted"}}]
            }]
        }))
        .expect("history");
        assert_eq!(
            history.history.unwrap()[0]
                .messages_deleted
                .as_ref()
                .unwrap()[0]
                .message
                .id,
            "deleted"
        );
    }

    #[test]
    fn gmail_mapping_extracts_authentication_results() {
        let message = map_gmail_message(GmailMessageResponse {
            id: "message-auth".to_string(),
            thread_id: Some("thread".to_string()),
            history_id: None,
            label_ids: None,
            snippet: None,
            internal_date: None,
            raw: None,
            payload: Some(GmailPayload {
                filename: None,
                mime_type: None,
                headers: Some(vec![super::GmailHeader {
                    name: "Authentication-Results".to_string(),
                    value: "mx.google.com; spf=fail (domain does not designate 209.85.220.41 as permitted sender); dkim=pass header.d=example.com; dmarc=fail (p=none)".to_string(),
                }]),
                body: None,
                parts: None,
            }),
        });
        assert_eq!(message.auth_spf.as_deref(), Some("fail"));
        assert_eq!(message.auth_dkim.as_deref(), Some("pass"));
        assert_eq!(message.auth_dmarc.as_deref(), Some("fail"));
    }

    /// The real 429 body Gmail returned during a full sync. The retry time only
    /// appears inside the message text — there is no Retry-After header — so a
    /// parser that only reads headers would silently fall back to a guess.
    const RATE_LIMIT_BODY: &str = r#"{"error":{"code":429,"message":"User-rate limit exceeded.  Retry after 2026-07-27T17:20:03.851Z","status":"RESOURCE_EXHAUSTED"}}"#;

    #[test]
    fn recognises_both_shapes_of_rate_limiting() {
        assert!(is_rate_limited(
            reqwest::StatusCode::TOO_MANY_REQUESTS,
            RATE_LIMIT_BODY
        ));
        // Gmail dresses the same condition up as 403 often enough that treating
        // it as an auth failure would park the account in an error state.
        assert!(is_rate_limited(
            reqwest::StatusCode::FORBIDDEN,
            r#"{"error":{"errors":[{"reason":"rateLimitExceeded"}]}}"#
        ));
        // A genuine authorisation failure must stay a failure.
        assert!(!is_rate_limited(
            reqwest::StatusCode::FORBIDDEN,
            r#"{"error":{"errors":[{"reason":"insufficientPermissions"}]}}"#
        ));
    }

    #[test]
    fn a_single_cheap_call_is_not_delayed() {
        let mut pacer = QuotaPacer {
            next_free: std::time::Instant::now(),
            background_next_free: std::time::Instant::now(),
            blocked_until: None,
            consecutive_blocks: 0,
            last_block_lapsed: None,
        };
        assert_eq!(
            pacer.reserve(cost::LABELS_LIST, Priority::Interactive),
            Some(std::time::Duration::ZERO)
        );
    }

    #[test]
    fn expensive_calls_queue_behind_each_other() {
        let mut pacer = QuotaPacer {
            next_free: std::time::Instant::now(),
            background_next_free: std::time::Instant::now(),
            blocked_until: None,
            consecutive_blocks: 0,
            last_block_lapsed: None,
        };
        // A page of 100 message fetches costs 2000 units. At 100 units/second
        // the last one must be held back by close to the full 20 seconds; the
        // old count-based pacing let the same page through in 2.5 s.
        let mut last = std::time::Duration::ZERO;
        for _ in 0..100 {
            last = pacer
                .reserve(cost::MESSAGES_GET, Priority::Interactive)
                .expect("budget");
        }
        let expected =
            std::time::Duration::from_secs_f64(99.0 * cost::MESSAGES_GET / QUOTA_UNITS_PER_SECOND);
        let slack = std::time::Duration::from_millis(50);
        assert!(
            last >= expected - slack,
            "last call waited {last:?}, expected ~{expected:?}"
        );
    }

    #[test]
    fn background_work_cannot_take_the_whole_budget() {
        let mut pacer = QuotaPacer {
            next_free: std::time::Instant::now(),
            background_next_free: std::time::Instant::now(),
            blocked_until: None,
            consecutive_blocks: 0,
            last_block_lapsed: None,
        };

        // A backfill spending everything it can get.
        for _ in 0..20 {
            pacer
                .reserve(cost::MESSAGES_GET, Priority::Background)
                .expect("background may run");
        }

        // The send that follows waits for the shared budget only, never for the
        // slower background clock — which by now reaches much further ahead.
        let interactive = pacer
            .reserve(cost::MESSAGES_SEND, Priority::Interactive)
            .expect("interactive always gets a slot");
        let background = pacer
            .reserve(cost::MESSAGES_GET, Priority::Background)
            .expect("background may run");
        assert!(
            interactive < background,
            "interactive waited {interactive:?}, background {background:?}"
        );
        const { assert!(BACKGROUND_UNITS_PER_SECOND < QUOTA_UNITS_PER_SECOND) };
    }

    #[test]
    fn background_stands_down_while_blocked_and_just_after() {
        let mut pacer = QuotaPacer {
            next_free: std::time::Instant::now(),
            background_next_free: std::time::Instant::now(),
            blocked_until: None,
            consecutive_blocks: 0,
            last_block_lapsed: None,
        };
        pacer.block_for(std::time::Duration::from_secs(600));

        assert!(
            pacer
                .reserve(cost::MESSAGES_GET, Priority::Background)
                .is_none(),
            "bulk work does not queue up behind a block"
        );
        // The user's own calls still get an answer — how long the block runs —
        // so the caller can decide between waiting and reporting it.
        assert!(pacer
            .reserve(cost::MESSAGES_SEND, Priority::Interactive)
            .is_some());

        // Block over, but the window Gmail measures on has not moved on yet.
        pacer.blocked_until = None;
        pacer.last_block_lapsed = Some(std::time::Instant::now());
        assert!(
            pacer
                .reserve(cost::MESSAGES_GET, Priority::Background)
                .is_none(),
            "bulk work waits out the cooldown as well"
        );
        assert!(pacer
            .reserve(cost::MESSAGES_SEND, Priority::Interactive)
            .is_some());

        pacer.last_block_lapsed = Some(
            std::time::Instant::now() - BACKGROUND_COOLDOWN - std::time::Duration::from_secs(1),
        );
        assert!(
            pacer
                .reserve(cost::MESSAGES_GET, Priority::Background)
                .is_some(),
            "and resumes once it has passed"
        );
    }

    #[test]
    fn interactive_work_returns_after_a_block_rather_than_on_it() {
        let mut pacer = QuotaPacer {
            next_free: std::time::Instant::now(),
            background_next_free: std::time::Instant::now(),
            blocked_until: None,
            consecutive_blocks: 0,
            last_block_lapsed: None,
        };
        let block = std::time::Duration::from_secs(60);
        pacer.block_for(block);

        let wait = pacer
            .reserve(cost::MESSAGES_SEND, Priority::Interactive)
            .expect("interactive waits the block out");
        // Sleeping to the boundary and firing there is what renewed the block:
        // Gmail's window has not moved on in the second it lapses.
        let slack = std::time::Duration::from_millis(50);
        assert!(
            wait >= block + BLOCK_LAPSE_MARGIN - slack,
            "returned {wait:?}, block was {block:?} plus a {BLOCK_LAPSE_MARGIN:?} margin"
        );
    }

    #[test]
    fn calls_held_by_a_block_do_not_all_fire_at_once() {
        let mut pacer = QuotaPacer {
            next_free: std::time::Instant::now(),
            background_next_free: std::time::Instant::now(),
            blocked_until: None,
            consecutive_blocks: 0,
            last_block_lapsed: None,
        };
        pacer.block_for(std::time::Duration::from_secs(60));

        // Everything the user did during the block used to be handed the same
        // instant to wake on, unpaced, because the block returned before the
        // reservation ran — a burst landing the moment Gmail was watching.
        let mut waits = Vec::new();
        for _ in 0..5 {
            waits.push(
                pacer
                    .reserve(cost::MESSAGES_SEND, Priority::Interactive)
                    .expect("interactive waits the block out"),
            );
        }
        let step = std::time::Duration::from_secs_f64(cost::MESSAGES_SEND / QUOTA_UNITS_PER_SECOND);
        // Each wait is measured against a slightly later `now`, so the gaps come
        // out a hair under the reservation itself.
        let slack = std::time::Duration::from_millis(50);
        for pair in waits.windows(2) {
            assert!(
                pair[1] + slack >= pair[0] + step,
                "calls waited {:?} then {:?}, expected at least {step:?} between them",
                pair[0],
                pair[1]
            );
        }
    }

    #[test]
    fn cost_differences_are_respected() {
        let mut cheap = QuotaPacer {
            next_free: std::time::Instant::now(),
            background_next_free: std::time::Instant::now(),
            blocked_until: None,
            consecutive_blocks: 0,
            last_block_lapsed: None,
        };
        let mut pricey = QuotaPacer {
            next_free: std::time::Instant::now(),
            background_next_free: std::time::Instant::now(),
            blocked_until: None,
            consecutive_blocks: 0,
            last_block_lapsed: None,
        };
        for _ in 0..10 {
            cheap
                .reserve(cost::LABELS_LIST, Priority::Interactive)
                .expect("budget");
            pricey
                .reserve(cost::MESSAGES_SEND, Priority::Interactive)
                .expect("budget");
        }
        // labels.list is 1 unit, messages.send is 100 — pacing by request count
        // would treat these identically and blow the budget on the send path.
        assert!(pricey.next_free > cheap.next_free);
    }

    #[test]
    fn idle_time_does_not_bank_unlimited_burst() {
        // An app parked overnight must not wake up entitled to hours of budget.
        let mut pacer = QuotaPacer {
            next_free: std::time::Instant::now() - std::time::Duration::from_secs(3600),
            background_next_free: std::time::Instant::now(),
            blocked_until: None,
            consecutive_blocks: 0,
            last_block_lapsed: None,
        };
        pacer
            .reserve(cost::MESSAGES_GET, Priority::Interactive)
            .expect("budget");
        let banked = std::time::Instant::now().saturating_duration_since(pacer.next_free);
        let allowance =
            std::time::Duration::from_secs_f64(QUOTA_BURST_UNITS / QUOTA_UNITS_PER_SECOND);
        assert!(
            banked <= allowance,
            "banked {banked:?} of burst, allowance is {allowance:?}"
        );
    }

    /// The regression the first version of this parser had: the timestamp is
    /// followed immediately by `","status":…`, so trimming quotes and commas
    /// left the JSON tail attached and every real 429 fell back to a guess.
    ///
    /// The timestamp is generated relative to now — a fixed one drifts into the
    /// past and then a broken parser and a correct one both produce the
    /// fallback, which is how the first version of this test passed by accident.
    #[test]
    fn reads_the_retry_time_out_of_a_real_error_body() {
        let when = chrono::Utc::now() + chrono::Duration::seconds(30);
        let body = format!(
            r#"{{"error":{{"code":429,"message":"User-rate limit exceeded.  Retry after {}","errors":[{{"reason":"rateLimitExceeded"}}]}},"status":"RESOURCE_EXHAUSTED"}}"#,
            when.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
        );

        let delay = rate_limit_delay(None, &body, 1);
        assert!(
            delay >= std::time::Duration::from_secs(28)
                && delay <= std::time::Duration::from_secs(31),
            "expected ~30 s from the embedded timestamp, got {delay:?}"
        );
    }

    #[test]
    fn a_retry_time_already_past_falls_back_rather_than_underflowing() {
        let body = r#"{"message":"User-rate limit exceeded.  Retry after 2020-01-01T00:00:00.000Z","status":"X"}"#;
        // Nothing left to wait out, so the doubling fallback applies instead of
        // a negative duration wrapping into an enormous sleep.
        assert_eq!(
            rate_limit_delay(None, body, 1),
            std::time::Duration::from_secs(2)
        );
    }

    #[test]
    fn header_wins_over_body_hint() {
        let delay = rate_limit_delay(Some("30"), RATE_LIMIT_BODY, 1);
        assert_eq!(delay, std::time::Duration::from_secs(30));
    }

    #[test]
    fn falls_back_to_doubling_without_a_hint() {
        assert_eq!(
            rate_limit_delay(None, "{}", 1),
            std::time::Duration::from_secs(2)
        );
        assert_eq!(
            rate_limit_delay(None, "{}", 3),
            std::time::Duration::from_secs(8)
        );
    }

    #[test]
    fn reports_long_blocks_truthfully_rather_than_capping() {
        // The cap belongs to the caller, which needs the real figure to choose
        // between waiting and giving up. Capping here made a long block look
        // like 90 seconds, so the code slept 90 s and knocked again — which is
        // what kept the block alive instead of letting it expire.
        let long = rate_limit_delay(Some("100000"), "{}", 1);
        assert_eq!(long, std::time::Duration::from_secs(100_000));
        assert!(long > MAX_RATE_LIMIT_WAIT);
    }

    #[test]
    fn a_block_is_shared_across_call_paths() {
        let mut pacer = QuotaPacer {
            next_free: std::time::Instant::now(),
            background_next_free: std::time::Instant::now(),
            blocked_until: None,
            consecutive_blocks: 0,
            last_block_lapsed: None,
        };
        assert!(pacer.block_remaining().is_none());

        pacer.block_for(std::time::Duration::from_secs(300));
        assert!(pacer.block_remaining().unwrap() > std::time::Duration::from_secs(290));

        // A blocked request waits out the block rather than its own quota slot.
        // Without this, every endpoint burns its retries rediscovering the same
        // 429 — which is what the logs showed.
        assert!(
            pacer
                .reserve(cost::LABELS_LIST, Priority::Interactive)
                .expect("budget")
                > std::time::Duration::from_secs(290)
        );
    }

    /// The live failure this backoff exists for: Gmail asked for 900 s, the
    /// code waited exactly 900 s, asked again on the second and was blocked
    /// again — at 18:51:06, 19:06:06, 19:21:06 and 19:36:06, indefinitely.
    #[test]
    fn repeat_blocks_wait_longer_than_the_server_asked() {
        let mut pacer = QuotaPacer {
            next_free: std::time::Instant::now(),
            background_next_free: std::time::Instant::now(),
            blocked_until: None,
            consecutive_blocks: 0,
            last_block_lapsed: None,
        };
        let asked = std::time::Duration::from_secs(900);

        // First offence: take the server at its word.
        assert_eq!(pacer.block_for(asked), asked);

        // Pretend that block has just lapsed, which is when the next 429 landed.
        pacer.blocked_until = None;
        pacer.last_block_lapsed = Some(std::time::Instant::now());

        let second = pacer.block_for(asked);
        assert!(
            second > asked,
            "a repeat block must wait longer than the {asked:?} that just proved too short, got {second:?}"
        );
        assert_eq!(second, asked * 2);
    }

    #[test]
    fn an_unrelated_later_block_starts_over() {
        let mut pacer = QuotaPacer {
            next_free: std::time::Instant::now(),
            background_next_free: std::time::Instant::now(),
            blocked_until: None,
            consecutive_blocks: 3,
            // Long enough ago that this is a new incident, not a continuation.
            last_block_lapsed: Some(
                std::time::Instant::now()
                    - (REPEAT_BLOCK_WINDOW + std::time::Duration::from_secs(1)),
            ),
        };
        let asked = std::time::Duration::from_secs(60);
        assert_eq!(
            pacer.block_for(asked),
            asked,
            "escalation must not persist across incidents"
        );
    }

    #[test]
    fn escalation_stops_at_the_ceiling() {
        let mut pacer = QuotaPacer {
            next_free: std::time::Instant::now(),
            background_next_free: std::time::Instant::now(),
            blocked_until: None,
            consecutive_blocks: 0,
            last_block_lapsed: None,
        };
        let asked = std::time::Duration::from_secs(900);
        let mut applied = std::time::Duration::ZERO;
        for _ in 0..12 {
            applied = pacer.block_for(asked);
            pacer.blocked_until = None;
            pacer.last_block_lapsed = Some(std::time::Instant::now());
        }
        // A bad hour must not push the next attempt days out.
        assert_eq!(applied, MAX_BLOCK_WAIT);
    }

    #[test]
    fn a_shorter_block_does_not_shorten_a_longer_one() {
        let mut pacer = QuotaPacer {
            next_free: std::time::Instant::now(),
            background_next_free: std::time::Instant::now(),
            blocked_until: None,
            consecutive_blocks: 0,
            last_block_lapsed: None,
        };
        pacer.block_for(std::time::Duration::from_secs(600));
        pacer.block_for(std::time::Duration::from_secs(5));
        assert!(pacer.block_remaining().unwrap() > std::time::Duration::from_secs(590));
    }
}
