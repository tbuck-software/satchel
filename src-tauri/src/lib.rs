// Two clippy style lints are allowed crate-wide rather than sprinkled as local
// attributes, because both fire on patterns that are idiomatic here and not
// defects:
//
// - `type_complexity`: sqlx `query_as` returns anonymous tuples whose shape is
//   the query's column list. Naming each one adds indirection without adding
//   information.
// - `too_many_arguments`: the message-mapping helpers thread request-scoped
//   context (app handle, pool, plugin runtime, key availability) explicitly
//   instead of hiding it in shared mutable state. That is the safer trade.
//
// Everything else is enforced: CI runs `cargo clippy --all-targets -D warnings`.
#![allow(clippy::type_complexity, clippy::too_many_arguments)]

// The domain lives in `satchel-core`, which knows nothing about Tauri. It is
// re-exported here so `crate::db::…`, `crate::providers::…` and the rest keep
// resolving from the command handlers.
pub use satchel_core::{db, email, events, providers, runtime_capabilities, workflow};

pub mod commands;
mod connection_state;
#[cfg(target_os = "android")]
pub mod notification_actions;
pub mod plugins;
pub mod sync;
pub mod utils;

use std::sync::Arc;

use events::bus::EventBus;
use plugins::runtime::PluginRuntime;
use sqlx::SqlitePool;
use sync::delivery_reconciliation::DeliveryReconciler;
use sync::queue::SyncQueue;
use tauri::{Emitter, Manager};
use tokio::sync::Mutex;

#[derive(Clone, Debug)]
pub enum OAuthFlowResult {
    Pending,
    Completed {
        account_id: String,
        provider: String,
        connected: bool,
        has_refresh_token: bool,
        expires_at: Option<String>,
    },
    Failed(String),
}

#[derive(Clone, Debug)]
pub struct UnlockedPgpSession {
    pub key_id: String,
    pub passphrase: String,
    pub unlocked_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Clone, Debug)]
pub struct UnlockedSmimeSession {
    pub cert_id: String,
    pub passphrase: Option<String>,
    pub unlocked_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Clone)]
pub struct AppState {
    pub queue: Arc<SyncQueue>,
    pub delivery_reconciler: Arc<DeliveryReconciler>,
    pub events: Arc<EventBus>,
    pub(crate) sync_activity: Arc<connection_state::SyncActivity>,
    pub plugin_runtime: Arc<PluginRuntime>,
    pub idle_control: Arc<std::sync::OnceLock<sync::idle::IdleControl>>,
    pub oauth_results: Arc<Mutex<std::collections::HashMap<String, OAuthFlowResult>>>,
    pub pgp_unlock_sessions: Arc<Mutex<std::collections::HashMap<String, UnlockedPgpSession>>>,
    pub smime_unlock_sessions: Arc<Mutex<std::collections::HashMap<String, UnlockedSmimeSession>>>,
    pub pending_open_with: Arc<commands::open_with::PendingOpenWith>,
    pub write_grants: Arc<commands::write_grants::WriteGrants>,
}

impl Default for AppState {
    fn default() -> Self {
        Self::with_plugin_runtime(PluginRuntime::default())
    }
}

impl AppState {
    fn with_plugin_runtime(plugin_runtime: PluginRuntime) -> Self {
        let events = Arc::new(EventBus::new(256));
        Self {
            queue: Arc::new(SyncQueue::new()),
            delivery_reconciler: Arc::new(DeliveryReconciler::new()),
            sync_activity: Arc::new(connection_state::SyncActivity::new(events.clone())),
            events,
            plugin_runtime: Arc::new(plugin_runtime),
            idle_control: Arc::new(std::sync::OnceLock::new()),
            oauth_results: Arc::new(Mutex::new(std::collections::HashMap::new())),
            pgp_unlock_sessions: Arc::new(Mutex::new(std::collections::HashMap::new())),
            smime_unlock_sessions: Arc::new(Mutex::new(std::collections::HashMap::new())),
            pending_open_with: Arc::new(commands::open_with::PendingOpenWith::default()),
            write_grants: Arc::new(commands::write_grants::WriteGrants::default()),
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    commands::notifications::notification_session_started_at();
    #[cfg(debug_assertions)]
    let _ = dotenvy::dotenv();

    let builder = tauri::Builder::default()
        .plugin(utils::navigation::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_fs::init());

    // Mail notifications are registered on every platform; the native half only
    // exists on Android, and the commands answer honestly elsewhere.
    let builder = builder.plugin(commands::mobile_notifications::init());

    // Same arrangement for the foreground service that keeps IMAP IDLE alive on
    // Android; elsewhere the command is a no-op.
    let builder = builder.plugin(commands::background_sync::init());

    // Tray, autostart and single-instance are desktop concepts. A phone has no
    // second instance to focus and no launch agent to register.
    #[cfg(desktop)]
    let builder = builder
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(commands::app_update::UpdateState::default())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![commands::tray::AUTOSTART_ARG]),
        ))
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            // The file first, and the main window only if there was none.
            // On Windows and Linux a double-click in the file manager *is* the
            // second process start; raising the window there drags the client
            // the user had put away back on screen, next to the viewer they
            // actually asked for. AGENTS.md wants the message file in a window
            // of its own.
            let raise_main = !opens_a_file_of_its_own(&argv);
            handle_open_with_args(app, &argv);
            if raise_main {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                    let _ = window.unminimize();
                }
            }
        }));

    builder
        .setup(|app| {
            let main_config = &app.config().app.windows[0];
            utils::navigation::guard(
                tauri::WebviewWindowBuilder::from_config(app, main_config)?, app.handle(),
            ).build()?;
            #[cfg(debug_assertions)]
            load_app_support_env(app);

            // Initialise structured logging + crash handler before anything else
            // so we capture failures during sub-system bootstrap.
            if let Ok(log_dir) = app
                .path()
                .resolve("satchel/logs", tauri::path::BaseDirectory::AppLog)
            {
                utils::telemetry::init(log_dir);
            }

            // Recover any interrupted database replacement before opening a
            // pool. Prepared secrets already use the active profile key; its
            // fingerprint is verified before the snapshot is installed.
            let profile_dir = utils::profile_dir(app.handle()).map_err(std::io::Error::other)?;
            utils::crypto::start_master_key_load(profile_dir.clone());
            db::apply_pending_restore(&profile_dir).map_err(std::io::Error::other)?;

            // Must run on the main thread and before the first notification is
            // delivered — a notification shown without an installed delegate
            // drops its click silently.
            #[cfg(target_os = "macos")]
            utils::mac_notify::init(app.handle().clone());

            #[cfg(desktop)]
            let plugin_runtime = PluginRuntime::with_compilation_cache(
                app.path().resolve("satchel/wasmtime", tauri::path::BaseDirectory::AppCache).ok(),
            );
            #[cfg(mobile)]
            let plugin_runtime = PluginRuntime::default();
            let state = AppState::with_plugin_runtime(plugin_runtime);
            app.manage(state.clone());
            // Name the directories plugins may be loaded from before anything
            // asks the runtime for one. Bundled resources come first so a copy
            // dropped into the writable profile cannot take over their id, and
            // the working directory is not a candidate at all.
            let mut plugin_dirs = Vec::new();
            if let Ok(resource_dir) = app.path().resource_dir() {
                plugin_dirs.push(resource_dir.join("plugins"));
            }
            plugin_dirs.push(profile_dir.join("plugins"));
            let _ = state.plugin_runtime.set_plugin_dirs(plugin_dirs);
            commands::mobile_notifications::initialize_mobile_notification_bridge(app.handle());

            // Pool MUST be initialized before the queue worker spawns.
            let pool = tauri::async_runtime::block_on(db::connect_pools(profile_dir.join("satchel.db")))
                .map_err(std::io::Error::other)?;
            let read_pool = pool.read;
            let checkpoint_pool = pool.checkpoint;
            let pool = pool.write;
            if let Ok(grants) =
                tauri::async_runtime::block_on(commands::plugins::load_plugin_grants(&pool))
            {
                let _ = state.plugin_runtime.set_permission_grants(grants);
            }
            if let Ok(disabled) =
                tauri::async_runtime::block_on(commands::plugins::load_disabled_plugins(&pool))
            {
                let _ = state.plugin_runtime.set_disabled_plugins(disabled);
            }
            let plugin_runtime = state.plugin_runtime.clone();
            tauri::async_runtime::spawn(async move {
                let _ = plugin_runtime.prewarm_message_transform_plugins();
            });
            app.manage::<SqlitePool>(pool.clone());
            app.manage(read_pool);
            // Checked once the pool exists; the window may show for a frame.
            #[cfg(desktop)]
            if commands::tray::launched_by_autostart(std::env::args())
                && tauri::async_runtime::block_on(commands::tray::start_minimized_enabled(&pool))
            {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                }
            }
            // Before any worker can call a provider: a block recorded by an
            // earlier run is still running on the server, and knocking renews it.
            tauri::async_runtime::block_on(sync::rate_limit_state::restore(&pool));
            commands::sources::spawn_rss_worker(pool.clone(), state.events.clone());

            let app_handle = app.handle().clone();
            state.delivery_reconciler.start(
                state.queue.clone(),
                pool.clone(),
                app_handle.clone(),
            );
            // These checks scan large parts of the mailbox. Starting all of
            // them beside the first Inbox query made a cold Android launch
            // spend seconds fighting for SQLite and storage bandwidth. They
            // are repairs, not prerequisites, so let the first screen paint
            // and then run them sequentially to keep contention bounded.
            let startup_maintenance_pool = pool.clone();
            let startup_maintenance_app = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(15)).await;

                // On Android these migrations share the process and database
                // with every account poller. Keep them out of the same memory
                // window as provider snapshots; large mailboxes otherwise
                // make the OS terminate the whole app before sync commits.
                #[cfg(mobile)]
                let _mobile_work_guard = sync::leases::acquire_mobile_mailbox_work_permit().await;

                sync::maintenance::run_startup_data_repairs(&startup_maintenance_pool, &startup_maintenance_app).await;

                if let Ok(moved) =
                    sync::repairs::repair_imap_thread_id_collisions(&startup_maintenance_pool).await
                {
                    if moved > 0 {
                        tracing::info!(messages_moved = moved, "repaired imap thread ids");
                        let _ = startup_maintenance_app.emit(
                            "mail:incoming",
                            serde_json::json!({ "reason": "imap_thread_repair" }),
                        );
                    }
                }
                // After collisions are untangled, stitch reply chains that were
                // split into per-message threads back into one conversation.
                if let Ok(moved) =
                    sync::thread_resolution::merge_reply_split_threads(&startup_maintenance_pool).await
                {
                    if moved > 0 {
                        tracing::info!(messages_moved = moved, "merged reply-split threads");
                        let _ = startup_maintenance_app.emit(
                            "mail:incoming",
                            serde_json::json!({ "reason": "imap_thread_repair" }),
                        );
                    }
                }
                if let Ok(repaired) =
                    commands::threads::repair_unread_state_inner(&startup_maintenance_pool).await
                {
                    if repaired.messages_updated > 0 {
                        tracing::info!(
                            messages_updated = repaired.messages_updated,
                            threads_updated = repaired.threads_updated,
                            "repaired unread state"
                        );
                        let _ = startup_maintenance_app.emit(
                            "mail:incoming",
                            serde_json::json!({ "reason": "unread_repair" }),
                        );
                    }
                }

                if let Ok(imported) = sync::calendar_backfill::backfill_calendar_invites_from_attachments(
                    &startup_maintenance_pool,
                    0,
                )
                .await
                {
                    if imported > 0 {
                        tracing::info!(imported, "imported calendar invite attachments");
                    }
                }
                sync::maintenance::spawn_data_repair_worker(startup_maintenance_pool, startup_maintenance_app);
            });
            // Report account rows that are not encrypted. They are not adopted:
            // a plaintext row cannot be told from one another process planted.
            let credential_pool = pool.clone();
            tauri::async_runtime::spawn(async move {
                match commands::accounts::report_plaintext_account_configs(&credential_pool).await {
                    Ok(unusable) if unusable > 0 => {
                        tracing::error!(unusable, "accounts with unencrypted credentials are locked");
                    }
                    Err(err) => tracing::error!(%err, "credential encryption check failed"),
                    _ => {}
                }
            });

            sync::queue_worker::spawn_queue_worker(
                state.clone(),
                pool.clone(),
                app_handle.clone(),
            );
            let idle_control = sync::idle::spawn_idle_supervisors(
                pool.clone(),
                app_handle.clone(),
                state.events.clone(),
            );
            let _ = state.idle_control.set(idle_control.clone());
            app.manage(idle_control);
            sync::maintenance::spawn_checkpoint_worker(checkpoint_pool.clone());
            sync::maintenance::spawn_maintenance_worker(pool.clone(), checkpoint_pool, app_handle.clone());
            sync::maintenance::spawn_snooze_worker(pool.clone(), app_handle.clone());
            sync::maintenance::spawn_followup_worker(pool.clone(), app_handle.clone());
            sync::maintenance::spawn_webhook_worker(pool.clone());

            // Bridge AppEvent -> Tauri events for the frontend.
            let event_app = app_handle.clone();
            let event_pool = pool.clone();
            let event_plugin_runtime = state.plugin_runtime.clone();
            let event_sync_activity = state.sync_activity.clone();
            let mut subscriber = state.events.subscribe();
            tauri::async_runtime::spawn(async move {
                use events::bus::AppEvent;
                loop {
                    let event = match subscriber.recv().await {
                        Ok(event) => event,
                        // Lagged = we missed events during a burst (large sync).
                        // The old `while let Ok` exited here, permanently killing
                        // notifications/webhooks until restart. Resync the webview and continue.
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                            tracing::warn!(target: "events", "bridge lagged, {missed} events dropped");
                            let _ = event_app.emit("mail:resync", serde_json::json!({ "missed": missed }));
                            let _ = event_app.emit("connection:state", serde_json::json!({
                                "syncing": event_sync_activity.is_syncing(),
                            }));
                            continue;
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    };
                    if let Some((event_name, idempotency_key, payload)) =
                        plugin_event_payload(&event)
                    {
                        let webhook_pool = event_pool.clone();
                        let webhook_event = event_name.clone();
                        let webhook_key = idempotency_key.clone();
                        let webhook_payload = payload.clone();
                        tauri::async_runtime::spawn(async move {
                            if let Err(error) =
                                db::webhooks::WebhookRepository::new(&webhook_pool)
                                    .enqueue_deliveries(
                                        &webhook_event,
                                        &webhook_key,
                                        &webhook_payload,
                                    )
                                    .await
                            {
                                tracing::warn!(
                                    target: "events",
                                    error = ?error,
                                    "webhook delivery enqueue failed"
                                );
                            }
                        });
                        let runtime = event_plugin_runtime.clone();
                        tauri::async_runtime::spawn_blocking(move || {
                            let _ = runtime.dispatch_event(&event_name, &idempotency_key, payload);
                        });
                    }

                    match event {
                        AppEvent::SyncStarted | AppEvent::SyncCompleted => {
                            // A queued completion also announces changed mail. It
                            // cannot end other active operations, so read the tracker.
                            let _ = event_app.emit("connection:state", serde_json::json!({
                                "syncing": event_sync_activity.is_syncing(),
                            }));
                        }
                        AppEvent::ThreadsUpdated(ids) => {
                            let mut dismissed = Vec::new();
                            for batch in ids.chunks(500) {
                                let mut query = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
                                    "SELECT DISTINCT m.thread_id FROM messages m \
                                     JOIN folders f ON f.id = m.folder_id \
                                     WHERE m.is_read = 0 AND f.folder_type = 'inbox' \
                                     AND m.thread_id IN (",
                                );
                                let mut separated = query.separated(", ");
                                for id in batch {
                                    separated.push_bind(id);
                                }
                                separated.push_unseparated(")");
                                if let Ok(notifiable) = query.build_query_scalar::<String>().fetch_all(&event_pool).await {
                                    let notifiable: std::collections::HashSet<_> = notifiable.into_iter().collect();
                                    dismissed.extend(batch.iter().filter(|id| !notifiable.contains(*id)).cloned());
                                }
                            }
                            if !dismissed.is_empty() {
                                let _ = commands::mobile_notifications::dismiss_mail_notifications(&event_app, dismissed).await;
                            }
                            let _ = event_app.emit("thread:updated", serde_json::json!({ "thread_ids": ids }));
                        }
                        AppEvent::Notification { title, body } => {
                            let prefs =
                                commands::notifications::load_preferences(&event_pool).await;
                            if prefs.enabled && prefs.scope != "off" {
                                let strings = commands::ui_strings::current();
                                let title = match title {
                                    events::bus::NotificationTitle::SendFailed => {
                                        strings.notification_send_failed
                                    }
                                    events::bus::NotificationTitle::SyncFailed => {
                                        strings.notification_sync_failed
                                    }
                                };
                                commands::notifications::show_notification(
                                    &event_app, &title, &body,
                                );
                            }
                        }
                        AppEvent::NewMail {
                            event_id,
                            account_id,
                            from_name,
                            from_address,
                            subject,
                            preview,
                            thread_id,
                        } => {
                            commands::tray::mark_new_mail(&event_app, &event_pool, &thread_id)
                                .await;
                            let prefs =
                                commands::notifications::load_preferences(&event_pool).await;
                            let allowed = prefs.enabled
                                && match prefs.scope.as_str() {
                                    "off" => false,
                                    "all" | "inbox" => true,
                                    _ => true,
                                };
                            // Outcome of the delivery attempt, as a note for the
                            // outbox row: Ok(None) plain delivery, Ok(Some(note))
                            // delivered on a degraded path, Err(note) not shown.
                            // Android stacks its own group summary.
                            #[cfg(not(target_os = "android"))]
                            let coalesced = match allowed
                                .then(commands::notifications::decide_mail_burst)
                            {
                                Some(commands::notifications::BurstDecision::Coalesce {
                                    count,
                                }) => Some(count),
                                _ => None,
                            };
                            #[cfg(target_os = "android")]
                            let coalesced: Option<u32> = None;
                            let outcome: Result<Option<String>, String> = if let Some(count) =
                                coalesced
                            {
                                commands::notifications::show_mail_summary(
                                    &event_app,
                                    count,
                                    prefs.sound,
                                );
                                Err(commands::notifications::reason_note(
                                    commands::notifications::reason::FILTER_COALESCED,
                                    format!("burst; shown in a summary of {count}"),
                                ))
                            } else if allowed {
                                let title = from_name
                                    .clone()
                                    .filter(|s| !s.trim().is_empty())
                                    .unwrap_or_else(|| from_address.clone());
                                let account_label = sqlx::query_scalar::<_, String>(
                                    "SELECT email FROM accounts WHERE id = ? LIMIT 1",
                                )
                                .bind(&account_id)
                                .fetch_optional(&event_pool)
                                .await
                                .ok()
                                .flatten()
                                .unwrap_or_else(|| account_id.clone());
                                match commands::notifications::show_mail_notification(
                                    &event_app,
                                    &event_id,
                                    &account_id,
                                    &account_label,
                                    &title,
                                    &subject,
                                    &preview,
                                    &thread_id,
                                    prefs.sound,
                                )
                                .await
                                {
                                    Ok(commands::notifications::MailDeliveryOutcome::Native) => {
                                        Ok(None)
                                    }
                                    Ok(commands::notifications::MailDeliveryOutcome::Fallback(
                                        note,
                                    )) => Ok(Some(note)),
                                    Ok(
                                        commands::notifications::MailDeliveryOutcome::Suppressed(
                                            note,
                                        ),
                                    ) => Err(note),
                                    Err(error) => {
                                        tracing::warn!(%error, %event_id, "mail notification could not be rendered");
                                        Err(commands::notifications::reason_note(
                                            commands::notifications::reason::DISPATCH_ERROR,
                                            &error,
                                        ))
                                    }
                                }
                            } else {
                                // A preference change after the row was claimed is an
                                // intentional dismissal, not a delivery failure.
                                Err(commands::notifications::reason_note(
                                    commands::notifications::reason::SETTING_DISABLED,
                                    "preference changed after the row was claimed",
                                ))
                            };
                            let acknowledged = match &outcome {
                                Ok(note) => {
                                    commands::notifications::mark_mail_notification_delivered_with_note(
                                        &event_pool,
                                        std::slice::from_ref(&event_id),
                                        note.as_deref(),
                                    )
                                    .await
                                }
                                Err(note) if note.starts_with(
                                    commands::notifications::reason::DISPATCH_ERROR,
                                ) => {
                                    // Retryable: back to pending, reason kept.
                                    commands::notifications::mark_mail_notification_failed(
                                        &event_pool,
                                        &event_id,
                                        note,
                                    )
                                    .await
                                }
                                Err(note) => {
                                    commands::notifications::mark_mail_notification_suppressed(
                                        &event_pool,
                                        &event_id,
                                        note,
                                    )
                                    .await
                                }
                            };
                            if let Err(error) = acknowledged {
                                tracing::warn!(%error, %event_id, "mail notification outcome could not be recorded");
                            }
                            let _ = event_app.emit(
                                "mail:incoming",
                                serde_json::json!({
                                    "account_id": account_id,
                                    "thread_id": thread_id,
                                    "from": from_address,
                                    "subject": subject,
                                }),
                            );
                        }
                        AppEvent::ConnectionState {
                            online: _,
                            syncing: _,
                            last_sync_at,
                            pending_outbox,
                            pending_sends,
                            failed_outbox,
                            failed_sends,
                            failed_remote_mutations,
                        } => {
                            let _ = event_app.emit(
                                "connection:state",
                                serde_json::json!({
                                    "online": null,
                                    "syncing": event_sync_activity.is_syncing(),
                                    "last_sync_at": last_sync_at,
                                    "pending_outbox": pending_outbox,
                                    "pending_sends": pending_sends,
                                    "failed_outbox": failed_outbox,
                                    "failed_sends": failed_sends,
                                    "failed_remote_mutations": failed_remote_mutations,
                                }),
                            );
                        }
                    }
                }
            });

            // Tray + window-close-to-tray. Restore the attention cursor before
            // the icon is built so a minimized login launch does not briefly
            // claim there is nothing new.
            let pending_tray_threads =
                tauri::async_runtime::block_on(commands::tray::load_pending_thread_ids(&pool));
            commands::tray::install_tray(&app_handle, pending_tray_threads).ok();

            // AppKit owns the lower half of the Dock menu (window list,
            // Options, Hide and Quit). Install Satchel's app actions only
            // after their managed state and native windows are ready.
            #[cfg(target_os = "macos")]
            utils::mac_dock_menu::init(app.handle().clone());

            // Handle "Open with Satchel" launches: argv[1..] may carry an .ics path.
            let argv: Vec<String> = std::env::args().collect();
            handle_open_with_args(&app_handle, &argv);

            Ok(())
        })
        .on_window_event(|window, event| {
            #[cfg(desktop)]
            if window.label() == "main" && matches!(event, tauri::WindowEvent::Focused(true)) {
                commands::tray::clear_new_mail(window.app_handle());
            }
            #[cfg(desktop)]
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                // Hide to tray instead of quitting unless the user explicitly chose Quit.
                if let Some(state) = window
                    .app_handle()
                    .try_state::<commands::tray::TrayPreferences>()
                {
                    if state
                        .quit_on_close
                        .load(std::sync::atomic::Ordering::Relaxed)
                    {
                        return;
                    }
                }
                api.prevent_close();
                let _ = window.hide();
            }
            // Mobile has no window chrome to close and no tray to hide into.
            #[cfg(mobile)]
            let _ = (window, event);
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_update::check_app_update,
            commands::app_update::download_app_update,
            commands::app_update::app_update_progress,
            commands::app_update::install_app_update,
            commands::app_update::acknowledge_app_update_flush,
            commands::accounts::list_accounts,
            commands::accounts::add_account,
            commands::accounts::deactivate_account,
            commands::accounts::delete_account,
            commands::accounts::test_imap_account_connection,
            commands::accounts::update_account_password,
            commands::accounts::clear_account_oauth_error,
            commands::mailboxes::list_mailboxes,
            commands::folders::create_folder,
            commands::folders::rename_folder,
            commands::folders::delete_folder,
            commands::messages::list_messages_preview,
            commands::workflow::transition_threads,
            commands::open_with::take_pending_open_with,
            commands::draft_flush::acknowledge_draft_flush,
            commands::oauth::abort_oauth_flow,
            commands::oauth::list_open_oauth_flows,
            commands::messages::list_message_bodies,
            commands::messages::load_thread_messages,
            commands::messages::get_message_source,
            commands::messages::search_message_hits,
            commands::server_search::server_search,
            commands::server_search::fetch_server_search_hit,
            commands::messages::send_message,
            commands::contacts::list_contacts,
            commands::contacts::search_contacts,
            commands::contacts::sync_contact_avatars,
            commands::contacts::sync_local_contact_avatars,
            commands::contacts::upsert_contact,
            commands::contacts::set_contact_avatar,
            commands::contacts::delete_contact,
            commands::diagnostics::record_frontend_diagnostic,
            commands::external_links::open_external_url,
            commands::reveal::show_in_folder,
            commands::threads::list_cached_threads,
            commands::threads::list_threads,
            commands::threads::list_smart_inbox_summary,
            commands::threads::get_thread,
            commands::threads::get_threads,
            commands::threads::message_link_target,
            commands::threads::resolve_message_link,
            commands::threads::update_thread_state,
            commands::threads::update_thread_organization,
            commands::threads::set_thread_tags,
            commands::threads::trash_threads,
            commands::threads::set_threads_read,
            commands::workflow::list_recent_transitions,
            commands::sync::run_imap_sync,
            commands::sync::test_imap_connection,
            commands::onboarding::discover_account,
            commands::onboarding::detect_mail_provider,
            commands::onboarding::autoconfigure_groupware,
            commands::oauth::start_oauth_flow,
            commands::oauth::poll_oauth_flow,
            commands::oauth::complete_oauth_flow,
            commands::oauth::oauth_status,
            commands::pgp::list_pgp_keys,
            commands::pgp::pgp_engine_status,
            commands::pgp::import_pgp_keys,
            commands::pgp::validate_pgp_bundle_passphrase,
            commands::pgp::generate_pgp_key,
            commands::pgp::unlock_pgp_keys,
            commands::pgp::delete_pgp_key,
            commands::smime::list_smime_certificates,
            commands::smime::add_smime_certificate,
            commands::smime::import_smime_pkcs12,
            commands::smime::generate_smime_keypair,
            commands::smime::unlock_smime_certificates,
            commands::smime::delete_smime_certificate,
            commands::rules::list_rules,
            commands::rules::upsert_rule,
            commands::rules::delete_rule,
            commands::rules::run_rules_for_thread,
            commands::rules::run_rules_for_threads,
            commands::sources::get_source_capabilities,
            commands::sources::list_sources,
            commands::sources::create_source,
            commands::sources::delete_source,
            commands::sources::poll_source,
            commands::sources::import_opml,
            commands::plugins::list_plugins,
            commands::plugins::set_plugin_permission,
            commands::plugins::set_plugin_enabled,
            commands::plugins::execute_plugin_command,
            commands::plugins::list_plugin_audit_log,
            commands::plugins::list_plugin_metrics,
            commands::drafts::save_draft,
            commands::drafts::list_drafts,
            commands::drafts::load_draft,
            commands::drafts::delete_draft,
            commands::signatures::list_signatures,
            commands::signatures::upsert_signature,
            commands::signatures::delete_signature,
            commands::signatures::list_signature_assignments,
            commands::signatures::upsert_signature_assignment,
            commands::signatures::resolve_signature,
            commands::signatures::discover_signature_suggestions,
            commands::signatures::list_signature_suggestions,
            commands::signatures::set_signature_suggestion_status,
            commands::aliases::list_aliases,
            commands::aliases::upsert_alias,
            commands::aliases::delete_alias,
            commands::templates::list_templates,
            commands::templates::upsert_template,
            commands::templates::delete_template,
            commands::templates::render_template,
            commands::followups::list_follow_ups,
            commands::followups::upsert_follow_up,
            commands::followups::cancel_follow_up,
            commands::categories::set_thread_category,
            commands::categories::set_thread_priority,
            commands::categories::set_sender_preference,
            commands::categories::get_sender_preference,
            commands::calendar::respond_to_invite,
            commands::calendar::list_invites,
            commands::calendar::list_calendar_events,
            commands::calendar::list_calendar_sources,
            commands::calendar::create_calendar_event,
            commands::calendar::delete_calendar_event,
            commands::calendar::sync_google_calendar,
            commands::calendar::import_ics,
            commands::calendar::import_ics_path,
            commands::groupware::configure_groupware,
            commands::groupware::sync_groupware,
            commands::outbox::list_outbox,
            commands::outbox::retry_outbox_action,
            commands::outbox::retry_failed_sync_actions,
            commands::outbox::cancel_outbox_action,
            commands::outbox::undo_outbox_send,
            commands::outbox::list_failed_sync_actions,
            commands::outbox::discard_failed_sync_action,
            commands::tray::set_quit_on_close,
            commands::tray::set_autostart,
            commands::tray::set_start_minimized,
            commands::write_grants::choose_save_path,
            commands::write_grants::choose_directory,
            commands::tray::tray_preferences,
            commands::tray::tray_snapshot,
            commands::tray::open_tray_thread,
            commands::tray::open_tray_destination,
            commands::tray::act_on_tray_thread,
            commands::thread_actions::set_thread_muted,
            commands::thread_actions::mark_threads_spam,
            commands::star::set_threads_starred,
            commands::blocked_senders::list_blocked_senders,
            commands::blocked_senders::block_sender,
            commands::blocked_senders::unblock_sender,
            commands::smart_folders::list_smart_folders,
            commands::smart_folders::upsert_smart_folder,
            commands::smart_folders::delete_smart_folder,
            commands::topics::list_topics,
            commands::topics::upsert_topic,
            commands::topics::delete_topic,
            commands::topics::add_topic_items,
            commands::topics::remove_topic_item,
            commands::topics::list_topic_memberships,
            commands::topics::get_topic_timeline,
            commands::topics::upsert_topic_note,
            commands::topics::delete_topic_note,
            commands::topics::add_topic_file,
            commands::topics::list_topic_files,
            commands::topics::download_topic_file,
            commands::topics::delete_topic_file,
            commands::backup::export_mailbox,
            commands::backup::import_mbox,
            commands::backup::export_platform_data,
            commands::backup::import_platform_data,
            commands::backup::cancel_import,
            commands::backup::create_backup,
            commands::backup::list_backups,
            commands::backup::restore_backup,
            commands::backup::cancel_pending_restore,
            commands::backup::prune_backups,
            commands::backup::credentials_status,
            commands::provider_features::list_provider_labels,
            commands::provider_features::refresh_provider_labels,
            commands::provider_features::create_provider_label,
            commands::vacation::get_vacation,
            commands::vacation::set_vacation,
            commands::webhooks::list_webhooks,
            commands::webhooks::upsert_webhook,
            commands::webhooks::delete_webhook,
            commands::unsubscribe::unsubscribe,
            commands::attachments::list_attachments_for_messages,
            commands::attachments::download_attachment,
            commands::attachments::open_attachment_default,
            commands::attachments::toggle_attachment_quick_look,
            commands::inline_cache::inline_image_cache_status,
            commands::inline_cache::start_inline_image_backfill,
            commands::inline_cache::stop_inline_image_backfill,
            commands::move_threads::move_threads_to_folder,
            commands::notifications::set_notification_preferences,
            commands::notifications::get_notification_preferences,
            commands::notifications::open_compose_window,
            commands::draft_editors::claim_draft_editor,
            commands::draft_editors::release_draft_editor,
            commands::draft_editors::focus_draft_editor,
            commands::ui_strings::set_ui_strings,
            commands::connection::current_connection_state,
            commands::diagnostics::diagnostics_overview,
            commands::diagnostics_report::diagnostics_report,
            commands::diagnostics_report::app_build_info,
            commands::diagnostics_report::open_logs_folder,
            commands::crash::pending_crash_reports,
            commands::message_file::open_message_file,
            commands::message_file::read_message_file_attachment,
            commands::message_file::open_message_file_in_window,
            commands::runtime::runtime_capabilities,
            commands::background_sync::set_background_sync,
            commands::background_sync::background_sync_status,
            commands::background_sync::open_background_sync_settings,
            commands::message_body::load_message_body,
            commands::notification_navigation::take_pending_notification_thread,
            commands::mobile_notifications::consume_pending_mobile_notification,
            commands::mobile_notifications::dismiss_mobile_notifications,
            commands::mobile_notifications::mobile_notifications_frontend_ready,
        ])
        .build(tauri::generate_context!())
        .expect("failed to build satchel tauri app")
        .run(|app, event| {
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = event {
                commands::tray::show_main(app);
            }
            // macOS delivers "Open with" to a *running* app through this event,
            // not through argv. Without it the association only ever worked on
            // a cold start, which is the rarer half of the cases. The variant
            // exists on Apple platforms; this file-window handler is macOS-only.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Opened { urls } = &event {
                for url in urls {
                    if let Ok(path) = url.to_file_path() {
                        open_path_with_satchel(app, &path);
                    }
                }
            }
            // Coming back from the background is the moment a phone has to
            // catch up: nothing was fetched while the process was suspended,
            // and the frontend re-syncs when it hears this.
            #[cfg(mobile)]
            if let tauri::RunEvent::Resumed = event {
                let _ = app.emit("mobile:resumed", ());
            }
            // Quitting used to end the process outright, so up to ten seconds
            // of typing — the autosave interval — went with it, without a word.
            #[cfg(desktop)]
            if let tauri::RunEvent::ExitRequested { api, .. } = &event {
                if request_draft_flush(app) {
                    api.prevent_exit();
                }
            }
            if matches!(&event, tauri::RunEvent::Exit) {
                app.state::<AppState>().delivery_reconciler.shutdown();
            }
        });
}

/// How long a quit waits for open composers to put their drafts on disk.
///
/// One IPC round trip and one write, no more. A window that never answers — a
/// frozen webview, a page mid-navigation — must not turn Quit into a hang.
#[cfg(desktop)]
const DRAFT_FLUSH_GRACE: std::time::Duration = std::time::Duration::from_millis(1000);

/// Set once the flush round has been asked for, so the exit that follows it is
/// not held back a second time. Cleared again when a window reports a failed
/// save, so the next quit asks again.
#[cfg(desktop)]
static DRAFT_FLUSH_REQUESTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[cfg(desktop)]
#[derive(Debug, PartialEq, Eq)]
enum DraftFlushStep {
    Wait,
    Exit,
    /// A window could not save its draft: stay open and show it.
    Abort,
}

/// One quit attempt waiting for the composer windows to answer.
#[cfg(desktop)]
#[derive(Debug)]
struct DraftFlushRound {
    outstanding: usize,
    settled: bool,
}

#[cfg(desktop)]
impl DraftFlushRound {
    fn new(windows: usize) -> Self {
        Self {
            outstanding: windows,
            settled: false,
        }
    }

    fn acknowledge(&mut self, saved: bool) -> DraftFlushStep {
        if self.settled {
            return DraftFlushStep::Wait;
        }
        if !saved {
            self.settled = true;
            return DraftFlushStep::Abort;
        }
        self.outstanding = self.outstanding.saturating_sub(1);
        if self.outstanding == 0 {
            self.settled = true;
            DraftFlushStep::Exit
        } else {
            DraftFlushStep::Wait
        }
    }

    /// A window that never answers must not turn Quit into a hang.
    fn deadline(&mut self) -> DraftFlushStep {
        if self.settled {
            return DraftFlushStep::Wait;
        }
        self.settled = true;
        DraftFlushStep::Exit
    }
}

/// Ask every window that can hold an unsaved draft to write it, and answer
/// whether the exit has to wait.
///
/// Only `main` and the composer popouts are asked: the message viewer and the
/// tray popover hold nothing that could be lost. The exit follows the last
/// acknowledgement or the deadline; a window that reports a failed save stops
/// it and is brought to the front.
#[cfg(desktop)]
fn request_draft_flush(app: &tauri::AppHandle) -> bool {
    use std::sync::atomic::Ordering;

    if DRAFT_FLUSH_REQUESTED.swap(true, Ordering::SeqCst) {
        return false;
    }
    let windows: Vec<_> = app
        .webview_windows()
        .into_keys()
        .filter(|label| label == "main" || label.starts_with("compose-"))
        .collect();
    if windows.is_empty() {
        return false;
    }

    let round = commands::draft_flush::begin(windows);
    let _ = app.emit("app:flush-drafts", ());

    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(DRAFT_FLUSH_GRACE);
        let step = round
            .lock()
            .map(|mut round| round.progress.deadline())
            .unwrap_or(DraftFlushStep::Exit);
        settle_draft_flush(&app, step, None);
    });
    true
}

#[cfg(desktop)]
fn settle_draft_flush(app: &tauri::AppHandle, step: DraftFlushStep, window: Option<&str>) {
    match step {
        DraftFlushStep::Wait => {}
        DraftFlushStep::Exit => exit_now(app),
        DraftFlushStep::Abort => {
            DRAFT_FLUSH_REQUESTED.store(false, std::sync::atomic::Ordering::SeqCst);
            commands::draft_editors::focus_window(app, window.unwrap_or("main"));
        }
    }
}

/// Leave, from whichever thread got there first. A second call after the app is
/// gone simply fails to reach the main thread.
#[cfg(desktop)]
pub(crate) fn exit_now(app: &tauri::AppHandle) {
    let app = app.clone();
    let _ = app.clone().run_on_main_thread(move || app.exit(0));
}

#[cfg(debug_assertions)]
fn load_app_support_env(app: &tauri::App) {
    if let Ok(path) = app
        .path()
        .resolve("satchel/.env", tauri::path::BaseDirectory::AppData)
    {
        let _ = dotenvy::from_path(path);
    }
}

fn handle_open_with_args(app: &tauri::AppHandle, argv: &[String]) {
    for arg in argv.iter().skip(1) {
        open_path_with_satchel(app, std::path::Path::new(arg));
    }
}

/// Whether a launch carries a file Satchel opens on its own, without the
/// mailbox behind it.
///
/// Kept separate from `handle_open_with_args` so the "raise the main window?"
/// decision is answerable without a running app.
#[cfg(desktop)]
fn opens_a_file_of_its_own(argv: &[String]) -> bool {
    argv.iter()
        .skip(1)
        .any(|arg| classify_open_with(std::path::Path::new(arg)) != OpenWithKind::Unhandled)
}

/// Route one "Open with Satchel" path to whoever handles that file type.
///
/// Shared by the cold-start argv scan and the macOS `Opened` event so the two
/// entry points cannot drift apart.
fn open_path_with_satchel(app: &tauri::AppHandle, path: &std::path::Path) {
    match classify_open_with(path) {
        OpenWithKind::Invitation => {
            // Buffered as well as emitted: on a cold start this runs before any
            // webview exists, and the event would go nowhere.
            if let Some(state) = app.try_state::<AppState>() {
                state
                    .pending_open_with
                    .push(path.to_string_lossy().into_owned());
            }
            let _ = app.emit("ics:open-with", ());
        }
        #[cfg(desktop)]
        OpenWithKind::MessageFile => {
            // Straight to its own window: a message file is read and closed, and
            // routing it through the main window would mean booting the whole
            // client before the mail the user asked for shows up.
            let _ = commands::message_file::open_message_file_window(app, &path.to_string_lossy());
        }
        OpenWithKind::Unhandled => {}
    }
}

/// Which entry point a launch argument belongs to.
#[derive(Debug, PartialEq, Eq)]
enum OpenWithKind {
    /// Goes to the running client, which imports it as an invitation.
    Invitation,
    /// Gets its own window; never reaches the main webview.
    #[cfg(desktop)]
    MessageFile,
    Unhandled,
}

/// Decide what one "Open with Satchel" path is, without opening anything.
fn classify_open_with(path: &std::path::Path) -> OpenWithKind {
    if !path.is_file() {
        return OpenWithKind::Unhandled;
    }
    if path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("ics"))
    {
        return OpenWithKind::Invitation;
    }
    #[cfg(desktop)]
    if commands::message_file::is_message_file(path) {
        return OpenWithKind::MessageFile;
    }
    OpenWithKind::Unhandled
}

fn plugin_event_payload(
    event: &events::bus::AppEvent,
) -> Option<(String, String, serde_json::Value)> {
    match event {
        events::bus::AppEvent::ThreadsUpdated(thread_ids) => Some((
            "thread.updated".to_string(),
            format!("thread.updated:{}", thread_ids.join(",")),
            serde_json::json!({ "thread_ids": thread_ids, "thread_id": if thread_ids.len() == 1 { thread_ids.first() } else { None } }),
        )),
        events::bus::AppEvent::NewMail {
            event_id,
            account_id,
            from_name,
            from_address,
            subject,
            preview,
            thread_id,
        } => Some((
            "message.received".to_string(),
            format!("message.received:{account_id}:{event_id}"),
            serde_json::json!({
                "event_id": event_id,
                "account_id": account_id,
                "thread_id": thread_id,
                "from_name": from_name,
                "from_address": from_address,
                "subject": subject,
                "preview": preview,
            }),
        )),
        _ => None,
    }
}

#[cfg(all(test, desktop))]
mod tests {
    use super::{
        classify_open_with, opens_a_file_of_its_own, DraftFlushRound, DraftFlushStep, OpenWithKind,
    };

    #[test]
    fn a_failed_draft_save_stops_the_quit() {
        let mut round = DraftFlushRound::new(2);
        assert_eq!(round.acknowledge(true), DraftFlushStep::Wait);
        assert_eq!(round.acknowledge(false), DraftFlushStep::Abort);
        // The deadline must not exit behind the window that asked to stay.
        assert_eq!(round.deadline(), DraftFlushStep::Wait);
    }

    #[test]
    fn the_quit_follows_the_last_successful_save_or_the_deadline() {
        let mut round = DraftFlushRound::new(2);
        assert_eq!(round.acknowledge(true), DraftFlushStep::Wait);
        assert_eq!(round.acknowledge(true), DraftFlushStep::Exit);
        assert_eq!(round.deadline(), DraftFlushStep::Wait);

        let mut silent = DraftFlushRound::new(1);
        assert_eq!(silent.deadline(), DraftFlushStep::Exit);
        assert_eq!(silent.acknowledge(false), DraftFlushStep::Wait);
    }

    #[test]
    fn thread_change_payload_carries_the_whole_batch() {
        let event = super::events::bus::AppEvent::ThreadsUpdated(vec!["a".into(), "b".into()]);
        let (name, _, payload) = super::plugin_event_payload(&event).unwrap();
        assert_eq!(name, "thread.updated");
        assert_eq!(payload["thread_ids"], serde_json::json!(["a", "b"]));
        let event = super::events::bus::AppEvent::ThreadsUpdated(vec!["a".into()]);
        let (_, _, payload) = super::plugin_event_payload(&event).unwrap();
        assert_eq!(payload["thread_id"], "a");
    }

    fn scratch(name: &str, contents: &[u8]) -> std::path::PathBuf {
        let path =
            std::env::temp_dir().join(format!("satchel-open-with-{}-{name}", std::process::id()));
        std::fs::write(&path, contents).expect("write fixture");
        path
    }

    /// A second process start with a mail file is a double-click in the file
    /// manager, not a request to see the client again.
    #[test]
    fn a_launch_carrying_a_handled_file_does_not_raise_the_main_window() {
        let invitation = scratch("invite.ics", b"BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n");
        let message = scratch("mail.eml", b"Subject: hi\r\n\r\nbody\r\n");
        let other = scratch("notes.txt", b"nothing to open");

        assert_eq!(classify_open_with(&invitation), OpenWithKind::Invitation);
        assert_eq!(classify_open_with(&message), OpenWithKind::MessageFile);
        assert_eq!(classify_open_with(&other), OpenWithKind::Unhandled);

        let argv = |path: &std::path::Path| {
            vec!["satchel".to_string(), path.to_string_lossy().into_owned()]
        };
        assert!(opens_a_file_of_its_own(&argv(&invitation)));
        assert!(opens_a_file_of_its_own(&argv(&message)));
        assert!(!opens_a_file_of_its_own(&argv(&other)));

        // A plain second launch has nothing to open and means "come back".
        assert!(!opens_a_file_of_its_own(&["satchel".to_string()]));
        // A path that is not there at all is not a reason to stay hidden.
        assert!(!opens_a_file_of_its_own(&[
            "satchel".to_string(),
            "/nowhere/gone.eml".to_string(),
        ]));

        for path in [invitation, message, other] {
            let _ = std::fs::remove_file(path);
        }
    }
}
