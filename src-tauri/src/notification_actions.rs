//! Headless execution for Android notification quick actions.
//!
//! WorkManager loads this in its own process, separate from Tao's Activity
//! process, so both native runtimes have an independent Android context.

use std::{
    path::Path,
    sync::{Mutex, OnceLock},
    time::Duration,
};

use jni::{
    objects::{GlobalRef, JObject, JString},
    sys::jstring,
    JNIEnv,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::{sqlite::SqliteConnectOptions, SqlitePool};

use crate::{
    commands::threads::{set_threads_read_inner_with_queue_prefix, UpdateThreadsReadInput},
    commands::workflow::{transition_thread_for_notification, TransitionInput},
    sync::{
        queue::QueuedAction,
        remote_mutations::execute_remote_mutation,
        worker::{drain_actions, DrainOutcome},
        workflow_executor::execute_queued_remote_mutation,
    },
    workflow::states::WorkflowState,
    AppState,
};

static ANDROID_CONTEXT: OnceLock<GlobalRef> = OnceLock::new();
static ANDROID_TLS: OnceLock<Result<(), String>> = OnceLock::new();
static ANDROID_INIT: Mutex<()> = Mutex::new(());

#[allow(non_snake_case)]
#[no_mangle]
pub extern "system" fn Java_de_tbuck_satchel_background_NotificationActionNative_executeNotificationAction(
    mut env: JNIEnv<'_>,
    _native: JObject<'_>,
    context: JObject<'_>,
    database_path: JString<'_>,
    event_id: JString<'_>,
    action_id: JString<'_>,
    thread_id: JString<'_>,
) -> jstring {
    let result = (|| -> Result<NotificationActionOutcome, String> {
        initialize_android_process(&mut env, &context)?;
        let database_path = java_string(&mut env, &database_path)?;
        let event_id = java_string(&mut env, &event_id)?;
        let action_id = java_string(&mut env, &action_id)?;
        let thread_id = java_string(&mut env, &thread_id)?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|error| error.to_string())?;
        runtime.block_on(execute_notification_action_inner(
            Path::new(&database_path),
            &event_id,
            &action_id,
            &thread_id,
        ))
    })();

    let outcome = result.unwrap_or_else(NotificationActionOutcome::not_applied);
    serde_json::to_string(&outcome)
        .ok()
        .and_then(|json| env.new_string(json).ok())
        .map(JString::into_raw)
        .unwrap_or(std::ptr::null_mut())
}

#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum NotificationActionOutcome {
    Completed,
    Pending {
        error: String,
    },
    NotApplied {
        error: String,
    },
    /// The provider refused the action for good. The row is `failed` and waits
    /// in the app for retry or discard, so WorkManager must stop retrying.
    Failed {
        error: String,
    },
}

impl NotificationActionOutcome {
    fn not_applied(error: String) -> Self {
        Self::NotApplied { error }
    }

    fn from_drain(result: Result<DrainOutcome, String>) -> Self {
        match result {
            Ok(DrainOutcome::Completed) => Self::Completed,
            Ok(DrainOutcome::Pending { error }) | Err(error) => Self::Pending { error },
            Ok(DrainOutcome::Failed { error }) => Self::Failed { error },
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BackgroundMailNotification {
    event_id: String,
    thread_id: Option<String>,
    account_id: String,
    account_label: String,
    sender: String,
    subject: String,
    preview: String,
    sound: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BackgroundMailSyncResult {
    notifications: Vec<BackgroundMailNotification>,
    retry: bool,
    error: Option<String>,
}

/// Run a bounded Inbox refresh from WorkManager after Android reclaimed the UI
/// process. The JSON return keeps Android notification rendering native: the
/// headless Rust runtime never needs a Tauri `AppHandle` or WebView.
#[allow(non_snake_case)]
#[no_mangle]
pub extern "system" fn Java_de_tbuck_satchel_background_BackgroundMailSyncNative_syncMail(
    mut env: JNIEnv<'_>,
    _native: JObject<'_>,
    context: JObject<'_>,
    database_path: JString<'_>,
) -> jstring {
    let result = (|| -> Result<BackgroundMailSyncResult, String> {
        initialize_android_process(&mut env, &context)?;
        let database_path = java_string(&mut env, &database_path)?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|error| error.to_string())?;
        runtime.block_on(execute_background_mail_sync_inner(Path::new(
            &database_path,
        )))
    })();

    let response = match result {
        Ok(result) => result,
        Err(error) => BackgroundMailSyncResult {
            notifications: Vec::new(),
            retry: true,
            error: Some(error),
        },
    };
    serde_json::to_string(&response)
        .ok()
        .and_then(|json| env.new_string(json).ok())
        .map(JString::into_raw)
        .unwrap_or(std::ptr::null_mut())
}

#[allow(non_snake_case)]
#[no_mangle]
pub extern "system" fn Java_de_tbuck_satchel_background_BackgroundMailSyncNative_acknowledgeNotifications(
    mut env: JNIEnv<'_>,
    _native: JObject<'_>,
    context: JObject<'_>,
    database_path: JString<'_>,
    event_ids_json: JString<'_>,
) -> jstring {
    let result = (|| -> Result<(), String> {
        initialize_android_process(&mut env, &context)?;
        let database_path = java_string(&mut env, &database_path)?;
        let event_ids_json = java_string(&mut env, &event_ids_json)?;
        let acknowledgements: Vec<serde_json::Value> =
            serde_json::from_str(&event_ids_json).map_err(|error| error.to_string())?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|error| error.to_string())?;
        runtime.block_on(acknowledge_background_mail_notifications(
            Path::new(&database_path),
            &acknowledgements,
        ))
    })();

    match result {
        Ok(()) => std::ptr::null_mut(),
        Err(error) => env
            .new_string(error)
            .map(JString::into_raw)
            .unwrap_or(std::ptr::null_mut()),
    }
}

fn initialize_android_process(env: &mut JNIEnv<'_>, context: &JObject<'_>) -> Result<(), String> {
    let _guard = ANDROID_INIT
        .lock()
        .map_err(|_| "Android runtime initialization lock is poisoned".to_string())?;
    if ANDROID_CONTEXT.get().is_none() {
        let global = env
            .new_global_ref(context)
            .map_err(|error| format!("Android context: {error}"))?;
        let vm = env
            .get_java_vm()
            .map_err(|error| format!("Android VM: {error}"))?;
        // SAFETY: the dedicated `:notification_actions` process owns this
        // native context. The retained global reference lives until it exits.
        unsafe {
            ndk_context::initialize_android_context(
                vm.get_java_vm_pointer().cast(),
                global.as_obj().as_raw().cast(),
            );
        }
        let _ = ANDROID_CONTEXT.set(global);
    }
    ANDROID_TLS
        .get_or_init(|| {
            let tls_context = env
                .new_local_ref(context)
                .map_err(|error| format!("Android TLS context: {error}"))?;
            rustls_platform_verifier::android::init_with_env(env, tls_context)
                .map_err(|error| format!("Android TLS: {error}"))
        })
        .clone()
}

fn java_string(env: &mut JNIEnv<'_>, value: &JString<'_>) -> Result<String, String> {
    env.get_string(value)
        .map(Into::into)
        .map_err(|error| error.to_string())
}

/// A pool plus the lock that keeps the app from swapping the file under it.
/// Field order matters: the pool is dropped before the lock is released.
struct BackgroundDatabase {
    pool: SqlitePool,
    _open: crate::db::profile_lock::DatabaseOpenLock,
}

/// Every pool this process opens goes through here, so a staged restore is
/// applied by whichever process starts first and never under an open one.
async fn open_background_database(database_path: &Path) -> Result<BackgroundDatabase, String> {
    let profile = database_path
        .parent()
        .ok_or_else(|| "Satchel database path has no profile directory".to_string())?;
    crate::db::apply_pending_restore(profile)?;
    let open = crate::db::profile_lock::hold_open(profile).await?;
    let options = SqliteConnectOptions::new()
        .filename(database_path)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(15));
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(2)
        .connect_with(options)
        .await
        .map_err(|error| format!("open Satchel database: {error}"))?;
    Ok(BackgroundDatabase { pool, _open: open })
}

async fn execute_background_mail_sync_inner(
    database_path: &Path,
) -> Result<BackgroundMailSyncResult, String> {
    let database = open_background_database(database_path).await?;
    let pool = database.pool.clone();
    crate::db::init(&pool)
        .await
        .map_err(|error| format!("prepare Satchel database: {error}"))?;

    let enabled = read_background_notification_setting(&pool, "notifications.enabled")
        .await?
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes"
            )
        })
        .unwrap_or(true);
    let scope = read_background_notification_setting(&pool, "notifications.scope")
        .await?
        .unwrap_or_else(|| "inbox".to_string());
    if !enabled || scope.eq_ignore_ascii_case("off") {
        return Ok(BackgroundMailSyncResult {
            notifications: Vec::new(),
            retry: false,
            error: None,
        });
    }

    let sync_result = crate::sync::mailbox_sync::sync_mailboxes(
        &pool,
        crate::sync::mailbox_sync::MailboxSyncRequest {
            events: None,
            app: None,
            accounts: crate::sync::mailbox_sync::AccountSelection::AllActive,
            folders: crate::sync::mailbox_sync::FolderSelection::Inbox,
        },
    )
    .await;
    let rows = crate::commands::notifications::pending_headless_mail_notifications(&pool).await?;

    let sound = read_background_notification_setting(&pool, "notifications.sound")
        .await?
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes"
            )
        })
        .unwrap_or(true);
    let notifications = rows
        .into_iter()
        .map(|notification| BackgroundMailNotification {
            event_id: notification.event_id,
            thread_id: Some(notification.thread_id),
            account_id: notification.account_id,
            account_label: notification.account_label,
            sender: notification
                .from_name
                .filter(|value| !value.trim().is_empty())
                .unwrap_or(notification.from_address),
            subject: notification.subject,
            preview: notification.preview,
            sound,
        })
        .collect();
    let (retry, error) = match sync_result {
        Ok(summary) if summary.accounts_failed == 0 => (false, None),
        Ok(summary) => (true, Some(summary.failures.join("; "))),
        Err(error) => (true, Some(error)),
    };
    Ok(BackgroundMailSyncResult {
        notifications,
        retry,
        error,
    })
}

/// Settle the outbox rows the headless worker just handed to Android.
///
/// The worker reports per mail whether Android actually posted it, so a
/// refused `POST_NOTIFICATIONS` ends the row as `permission_denied` here, the
/// same way the foreground path settles it.
async fn acknowledge_background_mail_notifications(
    database_path: &Path,
    acknowledgements: &[serde_json::Value],
) -> Result<(), String> {
    let database = open_background_database(database_path).await?;
    let pool = database.pool.clone();
    crate::commands::notifications::record_android_worker_outcomes(&pool, acknowledgements).await
}

async fn read_background_notification_setting(
    pool: &SqlitePool,
    key: &str,
) -> Result<Option<String>, String> {
    sqlx::query_scalar("SELECT value FROM app_settings WHERE key = ?")
        .bind(key)
        .fetch_optional(pool)
        .await
        .map_err(|error| format!("load background notification setting: {error}"))
}

pub(crate) async fn execute_notification_action_inner(
    database_path: &Path,
    event_id: &str,
    action_id: &str,
    thread_id: &str,
) -> Result<NotificationActionOutcome, String> {
    if event_id.trim().is_empty() || thread_id.trim().is_empty() {
        return Err("invalid notification identity".to_string());
    }
    let database = open_background_database(database_path).await?;
    let pool = database.pool.clone();
    let state = AppState::default();

    let identity = format!("{event_id}\u{0}{action_id}\u{0}{thread_id}");
    let digest = format!("{:x}", Sha256::digest(identity.as_bytes()));
    let queue_prefix = format!("notification:{digest}");
    let action_key = format!("notification_action:{digest}");
    let queued_ids = if let Some(stored) = load_action_record(&pool, &action_key).await? {
        stored
    } else {
        let mut queued = match action_id {
            "satchel.archive" => {
                let current: String =
                    sqlx::query_scalar("SELECT workflow_state FROM threads WHERE id = ? LIMIT 1")
                        .bind(thread_id)
                        .fetch_one(&pool)
                        .await
                        .map_err(|error| format!("load notification thread: {error}"))?;
                let from = WorkflowState::from_db_value(&current)
                    .ok_or_else(|| format!("unsupported workflow state `{current}`"))?;
                if from == WorkflowState::Done {
                    // Recover an older worker interrupted after queuing the
                    // move but before Read. Resolve flags only after the move
                    // has reconciled the destination's folder-scoped UIDs.
                    let ids = notification_queue_ids(&pool, &queue_prefix).await?;
                    match NotificationActionOutcome::from_drain(
                        drain_notification_action_mutations(&pool, &state, &ids).await,
                    ) {
                        NotificationActionOutcome::Completed => {}
                        unfinished => return Ok(unfinished),
                    }
                    set_threads_read_inner_with_queue_prefix(
                        &pool,
                        &state,
                        UpdateThreadsReadInput {
                            thread_ids: vec![thread_id.to_string()],
                            is_read: true,
                        },
                        Some(&queue_prefix),
                    )
                    .await?
                } else {
                    transition_thread_for_notification(
                        &pool,
                        &state,
                        TransitionInput {
                            thread_id: thread_id.to_string(),
                            from,
                            to: WorkflowState::Done,
                            snooze_until: None,
                        },
                        &queue_prefix,
                    )
                    .await?
                }
            }
            "satchel.read" => {
                set_threads_read_inner_with_queue_prefix(
                    &pool,
                    &state,
                    UpdateThreadsReadInput {
                        thread_ids: vec![thread_id.to_string()],
                        is_read: true,
                    },
                    Some(&queue_prefix),
                )
                .await?
            }
            _ => return Err("unsupported notification action".to_string()),
        };
        let mut seen = std::collections::HashSet::new();
        queued.retain(|id| seen.insert(id.clone()));
        if let Err(error) = store_action_record(&pool, &action_key, &queued).await {
            return Ok(NotificationActionOutcome::Pending { error });
        }
        queued
    };

    // The action record is durable even when the provider is unreachable.
    // Retrying that sync is normal local-first behavior, not a failed action.
    Ok(NotificationActionOutcome::from_drain(
        drain_notification_action_mutations(&pool, &state, &queued_ids).await,
    ))
}

async fn notification_queue_ids(pool: &SqlitePool, prefix: &str) -> Result<Vec<String>, String> {
    sqlx::query_scalar("SELECT id FROM sync_queue WHERE id LIKE ? ORDER BY id")
        .bind(format!("{prefix}:%"))
        .fetch_all(pool)
        .await
        .map_err(|error| error.to_string())
}

async fn store_action_record(
    pool: &SqlitePool,
    key: &str,
    queued_ids: &[String],
) -> Result<(), String> {
    let value = serde_json::to_string(queued_ids).map_err(|error| error.to_string())?;
    sqlx::query(
        "INSERT INTO app_settings (key, value, updated_at) VALUES (?, ?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
    )
    .bind(key)
    .bind(value)
    .bind(chrono::Utc::now().to_rfc3339())
    .execute(pool)
    .await
    .map_err(|error| error.to_string())?;
    Ok(())
}

async fn load_action_record(pool: &SqlitePool, key: &str) -> Result<Option<Vec<String>>, String> {
    let value: Option<String> = sqlx::query_scalar("SELECT value FROM app_settings WHERE key = ?")
        .bind(key)
        .fetch_optional(pool)
        .await
        .map_err(|error| error.to_string())?;
    value
        .map(|value| serde_json::from_str(&value).map_err(|error| error.to_string()))
        .transpose()
}

/// Run this action's queue rows here, settled the way the main worker settles
/// them: bounded retries, and a permanent error ends the row as `failed`.
pub(crate) async fn drain_notification_action_mutations(
    pool: &SqlitePool,
    state: &AppState,
    queued_ids: &[String],
) -> Result<DrainOutcome, String> {
    drain_actions(
        pool,
        &state.queue,
        queued_ids,
        async |action: &QueuedAction| {
            if matches!(
                serde_json::from_value(action.payload.clone()),
                Ok(
                    crate::sync::remote_mutations::RemoteMutationPayload::WorkflowMove { .. }
                        | crate::sync::remote_mutations::RemoteMutationPayload::BlockedMove { .. }
                )
            ) {
                execute_queued_remote_mutation(pool, None, state, action).await
            } else {
                execute_remote_mutation(pool, &action.account_id, action.payload.clone()).await
            }
        },
    )
    .await
}
