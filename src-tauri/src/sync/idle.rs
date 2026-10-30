//! IMAP IDLE supervisor — keeps a long-lived connection open per account/inbox folder
//! and triggers a sync when the server signals new messages.
//!
//! Design notes:
//! - One supervisor task per account, joined via `JoinHandle` in a shared map.
//! - The top-level supervisor listens on an mpsc channel for `AddAccount(id)` /
//!   `RemoveAccount(id)` so account add/remove takes effect without an app restart.
//! - On any IDLE event we run an incremental sync, which now publishes
//!   `AppEvent::NewMail` for genuinely new inbox mail (notifications pipeline).
//! - On error: backoff (10s → 30s → 60s → 120s, capped) and reconnect.

use std::collections::HashMap;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures::FutureExt;
use sqlx::SqlitePool;
use tauri::async_runtime::JoinHandle;
use tauri::{AppHandle, Emitter};
use tokio::sync::{mpsc, Mutex, Notify};

use crate::events::bus::EventBus;
use crate::providers::imap::ImapProvider;
use crate::providers::traits::{IdleEvent, MailProvider};
use crate::sync::mailbox_sync::{AccountSelection, FolderSelection, MailboxSyncRequest};

const API_INBOX_POLL_INTERVAL: Duration = Duration::from_secs(15);
const POP3_POLL_INTERVAL: Duration = Duration::from_secs(60);
const IMAP_FALLBACK_POLL_INTERVAL: Duration = Duration::from_secs(30);
const IMAP_IDLE_RETRY_INTERVAL: Duration = Duration::from_secs(5 * 60);
const IMAP_IDLE_UNAVAILABLE: &str = "imap IDLE unavailable:";

/// How long a raised sync signal waits for company.
///
/// A server sends one IDLE event per delivery and a mailing list delivers in
/// bursts, so a run per event meant dozens of full account syncs queueing
/// behind one account lock. Two seconds collects a burst without making new
/// mail feel late.
const IDLE_SYNC_DEBOUNCE: Duration = Duration::from_secs(2);

/// A pending sync, however many events asked for it.
///
/// The flag is what makes this a coalescer rather than a queue: events that
/// arrive while a run is in flight collapse into exactly one follow-up run.
#[derive(Default)]
struct SyncSignal {
    pending: AtomicBool,
    wake: Notify,
}

impl SyncSignal {
    fn raise(&self) {
        self.pending.store(true, Ordering::SeqCst);
        self.wake.notify_one();
    }

    fn take(&self) -> bool {
        self.pending.swap(false, Ordering::SeqCst)
    }
}

/// Turns a stream of IDLE signals into non-overlapping sync runs.
///
/// Runs until the task is dropped, which is what ties it to the watcher that
/// spawned it.
async fn coalesce_sync_runs<F, Fut>(signal: Arc<SyncSignal>, debounce: Duration, sync: F)
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    loop {
        signal.wake.notified().await;
        tokio::time::sleep(debounce).await;
        // Cleared before the run, so anything arriving during it earns the next
        // one instead of being swallowed by this one.
        if !signal.take() {
            continue;
        }
        // Nobody joins this task, and the supervisor's own `catch_unwind`
        // covers only the IDLE loop. A panicking run used to end the consumer
        // silently: IDLE kept raising the signal and no sync ever followed.
        if AssertUnwindSafe(sync()).catch_unwind().await.is_err() {
            tracing::error!("idle: triggered sync panicked; waiting for the next signal");
        }
    }
}

/// Stops the coalescing consumer when the watcher it belongs to ends.
///
/// The consumer deliberately outlives a single IDLE round, so nothing else
/// would end it on logout, account removal or a reconnect loop.
struct ConsumerTask(JoinHandle<()>);

impl Drop for ConsumerTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// What a push- or poll-triggered run asks the mailbox sync for.
///
/// Inbox, not every folder: the watcher only ever sits on INBOX, and a full
/// account run pays for a `UID FETCH 1:* (UID FLAGS)` in each of Archive,
/// Drafts, Aside and the snooze folders to learn something only the inbox
/// could have told it.
fn idle_sync_request<'a>(
    events: Option<&'a Arc<EventBus>>,
    app: Option<&'a AppHandle>,
    account_id: &'a str,
) -> MailboxSyncRequest<'a> {
    MailboxSyncRequest {
        events,
        app,
        accounts: AccountSelection::One(account_id),
        folders: FolderSelection::Inbox,
    }
}

fn provider_poll_interval(provider: &str) -> Option<Duration> {
    match provider {
        "gmail" | "outlook" => Some(API_INBOX_POLL_INTERVAL),
        "pop3" => Some(POP3_POLL_INTERVAL),
        _ => None,
    }
}

fn should_retry_imap_idle(error: &str) -> bool {
    !error.starts_with(IMAP_IDLE_UNAVAILABLE)
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct AccountRow {
    id: String,
    email: String,
    provider: String,
    config: String,
}

#[derive(Debug, Clone)]
pub enum IdleCommand {
    AddAccount(String),
    RemoveAccount(String),
}

#[derive(Clone)]
pub struct IdleControl {
    sender: mpsc::UnboundedSender<IdleCommand>,
}

impl IdleControl {
    #[cfg(test)]
    pub(crate) fn test_channel() -> (Self, mpsc::UnboundedReceiver<IdleCommand>) {
        let (sender, receiver) = mpsc::unbounded_channel();
        (Self { sender }, receiver)
    }

    pub fn add_account(&self, id: impl Into<String>) {
        let _ = self.sender.send(IdleCommand::AddAccount(id.into()));
    }

    pub fn remove_account(&self, id: impl Into<String>) {
        let _ = self.sender.send(IdleCommand::RemoveAccount(id.into()));
    }
}

pub fn spawn_idle_supervisors(
    pool: SqlitePool,
    app: AppHandle,
    events: Arc<EventBus>,
) -> IdleControl {
    let (tx, mut rx) = mpsc::unbounded_channel::<IdleCommand>();
    let control = IdleControl { sender: tx.clone() };
    let active: Arc<Mutex<HashMap<String, JoinHandle<()>>>> = Arc::new(Mutex::new(HashMap::new()));

    let pool_root = pool.clone();
    let app_root = app.clone();
    let events_root = events.clone();
    let active_root = active.clone();

    tauri::async_runtime::spawn(async move {
        // A transient database/key-store failure during bootstrap must not
        // leave the process alive but permanently without any pollers.
        loop {
            match load_accounts(&pool_root).await {
                Ok(accounts) => {
                    for account in accounts {
                        spawn_account_task(
                            &active_root,
                            pool_root.clone(),
                            app_root.clone(),
                            events_root.clone(),
                            account,
                        )
                        .await;
                    }
                    break;
                }
                Err(err) => {
                    tracing::error!(%err, "idle: bootstrap account load failed; retrying");
                    tokio::time::sleep(Duration::from_secs(10)).await;
                }
            }
        }

        // Listen for hot-reload commands.
        while let Some(cmd) = rx.recv().await {
            match cmd {
                IdleCommand::AddAccount(account_id) => {
                    match load_account(&pool_root, &account_id).await {
                        Ok(Some(account)) => {
                            // If a task is already running for this account, restart it.
                            let mut map = active_root.lock().await;
                            if let Some(handle) = map.remove(&account.id) {
                                handle.abort();
                            }
                            drop(map);
                            spawn_account_task(
                                &active_root,
                                pool_root.clone(),
                                app_root.clone(),
                                events_root.clone(),
                                account,
                            )
                            .await;
                        }
                        Ok(None) => {
                            tracing::warn!(%account_id, "idle: add command for unknown account");
                        }
                        Err(err) => {
                            tracing::error!(%account_id, %err, "idle: failed to load added account");
                        }
                    }
                }
                IdleCommand::RemoveAccount(account_id) => {
                    let mut map = active_root.lock().await;
                    if let Some(handle) = map.remove(&account_id) {
                        handle.abort();
                    }
                }
            }
        }
    });

    control
}

async fn load_accounts(pool: &SqlitePool) -> Result<Vec<AccountRow>, String> {
    let rows = sqlx::query_as::<_, AccountRow>(
        "SELECT id, email, provider, config FROM accounts WHERE is_active = 1 AND provider IN ('imap', 'pop3', 'gmail', 'outlook')",
    )
    .fetch_all(pool)
    .await
    .map_err(|err| err.to_string())?;
    // Keep encrypted configs here. Each account supervisor decrypts on every
    // retry, so a temporarily unavailable Keystore cannot make that account
    // disappear permanently during bootstrap.
    Ok(rows
        .into_iter()
        .filter(|account| !crate::sync::is_seeded_demo_account(&account.id, &account.email))
        .collect())
}

async fn load_account(pool: &SqlitePool, account_id: &str) -> Result<Option<AccountRow>, String> {
    sqlx::query_as::<_, AccountRow>(
        "SELECT id, email, provider, config FROM accounts WHERE id = ? AND is_active = 1 AND provider IN ('imap', 'pop3', 'gmail', 'outlook') LIMIT 1",
    )
    .bind(account_id)
    .fetch_optional(pool)
    .await
    .map_err(|err| err.to_string())
}

async fn spawn_account_task(
    active: &Arc<Mutex<HashMap<String, JoinHandle<()>>>>,
    pool: SqlitePool,
    app: AppHandle,
    events: Arc<EventBus>,
    account: AccountRow,
) {
    let id = account.id.clone();
    let id_for_handle = id.clone();
    let map_handle = active.clone();
    let handle = tauri::async_runtime::spawn(async move {
        supervise_account(pool, app, events, account).await;
        let mut map = map_handle.lock().await;
        map.remove(&id_for_handle);
    });
    let mut map = active.lock().await;
    map.insert(id, handle);
}

async fn reload_supervisor_account(
    pool: &SqlitePool,
    previous: &AccountRow,
) -> Result<Option<AccountRow>, String> {
    load_account(pool, &previous.id).await
}

async fn supervise_account(
    pool: SqlitePool,
    app: AppHandle,
    events: Arc<EventBus>,
    account: AccountRow,
) {
    let mut backoff_secs: u64 = 10;
    loop {
        let account = match reload_supervisor_account(&pool, &account).await {
            Ok(Some(account)) => account,
            Ok(None) => return,
            Err(error) => {
                tracing::warn!(%error, "idle: account reload failed");
                tokio::time::sleep(Duration::from_secs(backoff_secs)).await;
                continue;
            }
        };
        let result = AssertUnwindSafe(run_account_loop(&pool, &app, &events, &account))
            .catch_unwind()
            .await;
        match result {
            Ok(Ok(())) => {
                tracing::warn!(account = %account.email, "idle: account loop ended; restarting");
                backoff_secs = 10;
                tokio::time::sleep(Duration::from_secs(backoff_secs)).await;
            }
            Ok(Err(err)) => {
                tracing::warn!(
                    account = %account.email,
                    %err,
                    backoff_secs,
                    "idle: account loop error, backing off"
                );
                tokio::time::sleep(Duration::from_secs(backoff_secs)).await;
                backoff_secs = (backoff_secs * 2).min(120);
            }
            Err(_) => {
                tracing::error!(
                    account = %account.email,
                    backoff_secs,
                    "idle: account loop panicked; restarting"
                );
                tokio::time::sleep(Duration::from_secs(backoff_secs)).await;
                backoff_secs = (backoff_secs * 2).min(120);
            }
        }
    }
}

async fn run_account_loop(
    pool: &SqlitePool,
    app: &AppHandle,
    events: &Arc<EventBus>,
    account: &AccountRow,
) -> Result<(), String> {
    let outcome = crate::utils::oauth::refresh_account_config_reporting(
        pool,
        &account.id,
        &account.provider,
        &account.config,
    )
    .await;
    if let Some(payload) = auth_required_event(&account.id, &outcome) {
        let _ = app.emit("account:auth-required", payload);
    }
    let config = outcome.config.map_err(|err| err.encoded())?;

    match account.provider.as_str() {
        "imap" => {
            let provider = ImapProvider::from_account_config(&config, &account.email)?;
            let signal = Arc::new(SyncSignal::default());

            // One consumer per watcher, not one task per event. The account
            // lock used to serialize the per-event runs, which turned a burst
            // of deliveries into a queue of full account syncs that grew faster
            // than it drained.
            let _consumer = ConsumerTask(tauri::async_runtime::spawn({
                let signal = signal.clone();
                let pool = pool.clone();
                let app = app.clone();
                let events = events.clone();
                let account = account.clone();
                async move {
                    coalesce_sync_runs(signal, IDLE_SYNC_DEBOUNCE, move || {
                        let pool = pool.clone();
                        let app = app.clone();
                        let events = events.clone();
                        let account = account.clone();
                        async move {
                            sync_polled_inbox(&pool, &app, &events, &account).await;
                        }
                    })
                    .await;
                }
            }));

            let signal_for_cb = signal.clone();

            let on_event: Arc<dyn Fn(IdleEvent) + Send + Sync> = Arc::new(move |evt| {
                if !matches!(evt, IdleEvent::NewMessages | IdleEvent::FlagsChanged) {
                    return;
                }
                // Only the signal: the UI learns about new mail from
                // `mail:incoming` once the sync that follows has run. The
                // `sync:incoming` this used to emit had no listener.
                signal_for_cb.raise();
            });

            loop {
                match provider.idle("INBOX", on_event.clone()).await {
                    Ok(()) => return Ok(()),
                    Err(err) => {
                        // IDLE is optional in IMAP. Some otherwise healthy
                        // servers reject the command; retrying only IDLE left
                        // those accounts dependent on the much slower global
                        // frontend refresh. Poll Inbox and periodically probe
                        // IDLE again after transient errors. A server that does
                        // not advertise IDLE stays on polling until restart.
                        let retry_idle = should_retry_imap_idle(&err.to_string());
                        tracing::warn!(
                            account = %account.email,
                            %err,
                            retry_idle,
                            "idle: unavailable, polling inbox"
                        );
                        let retry_at = retry_idle
                            .then(|| tokio::time::Instant::now() + IMAP_IDLE_RETRY_INTERVAL);
                        loop {
                            sync_polled_inbox(pool, app, events, account).await;
                            if let Some(retry_at) = retry_at {
                                let now = tokio::time::Instant::now();
                                if now >= retry_at {
                                    break;
                                }
                                tokio::time::sleep(IMAP_FALLBACK_POLL_INTERVAL.min(retry_at - now))
                                    .await;
                            } else {
                                tokio::time::sleep(IMAP_FALLBACK_POLL_INTERVAL).await;
                            }
                        }
                    }
                }
            }
        }
        // No local push channel is available for the HTTP providers. Poll only
        // Inbox frequently; complete folder syncs remain on the foreground
        // maintenance path. Sleeping after each run (instead of `interval`)
        // prevents a slow request from accumulating ticks and syncing forever.
        "gmail" | "outlook" | "pop3" => loop {
            tokio::time::sleep(
                provider_poll_interval(&account.provider)
                    .expect("polling provider must have an interval"),
            )
            .await;
            sync_polled_inbox(pool, app, events, account).await;
        },
        other => Err(format!("unsupported provider for idle: {other}")),
    }
}

async fn sync_polled_inbox(
    pool: &SqlitePool,
    app: &AppHandle,
    events: &Arc<EventBus>,
    account: &AccountRow,
) {
    match crate::sync::mailbox_sync::sync_mailboxes(
        pool,
        idle_sync_request(Some(events), Some(app), &account.id),
    )
    .await
    {
        Ok(_) => {}
        Err(err) => {
            tracing::warn!(
                account = %account.email,
                %err,
                "idle: inbox poll sync failed"
            );
        }
    }
}

/// The frontend reloads the account banner on this event. It fires only when
/// the refresh stored a new rejection, never for a network flap or a repeat.
fn auth_required_event(
    account_id: &str,
    outcome: &crate::utils::oauth::RefreshOutcome,
) -> Option<serde_json::Value> {
    let recorded = outcome.recorded_error.as_ref()?;
    Some(serde_json::json!({
        "account_id": account_id,
        "error": recorded.error,
        "at": recorded.at,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn auth_required_fires_once_per_stored_rejection_with_its_details() {
        use crate::sync::errors::SyncError;
        use crate::utils::oauth::{RecordedOAuthError, RefreshOutcome};
        // A network flap after resume, and a rejection already on record,
        // change nothing the banner shows.
        for config in [
            Err(SyncError::Transient(
                "oauth refresh request failed: offline".into(),
            )),
            Err(SyncError::Permanent("invalid_grant".into())),
        ] {
            let outcome = RefreshOutcome {
                config,
                recorded_error: None,
            };
            assert_eq!(auth_required_event("account", &outcome), None);
        }
        let recorded = RefreshOutcome {
            config: Err(SyncError::Permanent("invalid_grant".into())),
            recorded_error: Some(RecordedOAuthError {
                error: "invalid_grant".into(),
                at: "2026-10-30T12:00:00+00:00".into(),
            }),
        };
        assert_eq!(
            auth_required_event("account", &recorded),
            Some(serde_json::json!({
                "account_id": "account",
                "error": "invalid_grant",
                "at": "2026-10-30T12:00:00+00:00",
            }))
        );
    }

    /// Drives the coalescer with an injected run, and reports how often it ran.
    struct Coalescer {
        signal: Arc<SyncSignal>,
        runs: Arc<AtomicUsize>,
        release: Arc<Notify>,
        handle: tokio::task::JoinHandle<()>,
    }

    impl Coalescer {
        /// `block_first` holds the first run open until `release` fires, which
        /// is how the "events during a run" case is staged.
        fn start(debounce: Duration, block_first: bool) -> Self {
            let signal = Arc::new(SyncSignal::default());
            let runs = Arc::new(AtomicUsize::new(0));
            let release = Arc::new(Notify::new());
            let handle = tokio::spawn({
                let signal = signal.clone();
                let runs = runs.clone();
                let release = release.clone();
                async move {
                    coalesce_sync_runs(signal, debounce, || {
                        let runs = runs.clone();
                        let release = release.clone();
                        async move {
                            let index = runs.fetch_add(1, Ordering::SeqCst);
                            if block_first && index == 0 {
                                release.notified().await;
                            }
                        }
                    })
                    .await;
                }
            });
            Self {
                signal,
                runs,
                release,
                handle,
            }
        }

        fn runs(&self) -> usize {
            self.runs.load(Ordering::SeqCst)
        }
    }

    async fn settle(millis: u64) {
        tokio::time::sleep(Duration::from_millis(millis)).await;
    }

    #[tokio::test]
    async fn supervisor_retry_reads_credentials_saved_after_it_started() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("../crates/satchel-core/src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
            VALUES ('oauth', 'OAuth', 'me@example.test', 'gmail', '{}', '', '', '')")
            .execute(&pool).await.unwrap();
        let initial = super::load_account(&pool, "oauth").await.unwrap().unwrap();
        sqlx::query("UPDATE accounts SET config = 'fresh credentials' WHERE id = 'oauth'")
            .execute(&pool)
            .await
            .unwrap();
        let reloaded = super::reload_supervisor_account(&pool, &initial)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(reloaded.config, "fresh credentials");
        sqlx::query("UPDATE accounts SET is_active = 0 WHERE id = 'oauth'")
            .execute(&pool)
            .await
            .unwrap();
        assert!(super::reload_supervisor_account(&pool, &initial)
            .await
            .unwrap()
            .is_none());
    }

    /// A burst during one sync may start only one following run.
    #[tokio::test]
    async fn eighty_events_during_one_run_start_at_most_one_more() {
        let coalescer = Coalescer::start(Duration::from_millis(10), true);
        coalescer.signal.raise();
        settle(150).await;
        assert_eq!(coalescer.runs(), 1, "the first run should be under way");

        for _ in 0..80 {
            coalescer.signal.raise();
        }
        settle(100).await;
        assert_eq!(coalescer.runs(), 1, "no run may overlap the one in flight");

        coalescer.release.notify_one();
        settle(300).await;
        assert_eq!(coalescer.runs(), 2, "the whole burst earns one follow-up");

        coalescer.handle.abort();
    }

    /// One panicking run must not take push sync down for the account.
    #[tokio::test]
    async fn a_panicking_run_leaves_the_consumer_running() {
        let signal = Arc::new(SyncSignal::default());
        let runs = Arc::new(AtomicUsize::new(0));
        let handle = tokio::spawn({
            let signal = signal.clone();
            let runs = runs.clone();
            async move {
                coalesce_sync_runs(signal, Duration::from_millis(10), || {
                    let runs = runs.clone();
                    async move {
                        if runs.fetch_add(1, Ordering::SeqCst) == 0 {
                            panic!("first run fails");
                        }
                    }
                })
                .await;
            }
        });

        signal.raise();
        settle(150).await;
        assert_eq!(runs.load(Ordering::SeqCst), 1);
        signal.raise();
        settle(150).await;
        assert_eq!(
            runs.load(Ordering::SeqCst),
            2,
            "the signal after a panic still starts a run"
        );
        assert!(!handle.is_finished());

        handle.abort();
    }

    #[tokio::test]
    async fn events_within_the_debounce_window_collapse() {
        let coalescer = Coalescer::start(Duration::from_millis(250), false);
        for _ in 0..3 {
            coalescer.signal.raise();
            settle(30).await;
        }
        settle(600).await;
        assert_eq!(coalescer.runs(), 1);

        coalescer.handle.abort();
    }

    #[tokio::test]
    async fn an_event_after_a_run_starts_a_new_run() {
        let coalescer = Coalescer::start(Duration::from_millis(20), false);
        coalescer.signal.raise();
        settle(200).await;
        coalescer.signal.raise();
        settle(200).await;
        assert_eq!(coalescer.runs(), 2);

        coalescer.handle.abort();
    }

    #[tokio::test]
    async fn the_consumer_stops_with_the_watcher() {
        let coalescer = Coalescer::start(Duration::from_millis(20), false);
        coalescer.signal.raise();
        settle(200).await;
        assert_eq!(coalescer.runs(), 1);

        coalescer.handle.abort();
        settle(50).await;
        coalescer.signal.raise();
        settle(200).await;
        assert_eq!(coalescer.runs(), 1, "a stopped watcher runs nothing");
    }

    /// An IDLE watcher only ever sits on INBOX, so the run it triggers has no
    /// business listing every folder of the account.
    #[test]
    fn an_idle_triggered_run_only_syncs_the_inbox() {
        let request = idle_sync_request(None, None, "account-1");
        assert_eq!(request.folders, FolderSelection::Inbox);
        assert_eq!(request.accounts, AccountSelection::One("account-1"));
    }

    #[test]
    fn http_mail_providers_poll_quickly_without_accelerating_pop3() {
        assert_eq!(
            provider_poll_interval("gmail"),
            Some(Duration::from_secs(15))
        );
        assert_eq!(
            provider_poll_interval("outlook"),
            Some(Duration::from_secs(15))
        );
        assert_eq!(
            provider_poll_interval("pop3"),
            Some(Duration::from_secs(60))
        );
        assert_eq!(provider_poll_interval("imap"), None);
        assert_eq!(IMAP_FALLBACK_POLL_INTERVAL, Duration::from_secs(30));
        assert!(!should_retry_imap_idle(
            "imap IDLE unavailable: server does not advertise IDLE"
        ));
        assert!(should_retry_imap_idle(
            "imap IDLE init failed: connection reset"
        ));
    }

    #[tokio::test]
    async fn bootstrap_keeps_accounts_whose_keystore_is_temporarily_unavailable() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("open in-memory sqlite");
        sqlx::query(
            "CREATE TABLE accounts (
                id TEXT PRIMARY KEY,
                email TEXT NOT NULL,
                provider TEXT NOT NULL,
                config TEXT NOT NULL,
                is_active INTEGER NOT NULL
            )",
        )
        .execute(&pool)
        .await
        .expect("create accounts table");
        sqlx::query(
            "INSERT INTO accounts (id, email, provider, config, is_active)
             VALUES ('account-1', 'mail@example.test', 'imap', 'enc:v1:unreadable', 1)",
        )
        .execute(&pool)
        .await
        .expect("insert account");

        let accounts = load_accounts(&pool).await.expect("load accounts");
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].config, "enc:v1:unreadable");
    }
}
