use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{FromRow, SqlitePool};
use std::sync::OnceLock;

use crate::{
    commands::error::CommandResult,
    events::bus::{AppEvent, EventBus},
    AppState,
};

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/sources/"))]
pub struct SourceCapabilities {
    pub can_reply: bool,
    pub can_forward: bool,
    pub can_compose: bool,
    pub has_folders: bool,
    pub remote_flags: bool,
    pub remote_archive: bool,
    pub remote_delete: bool,
    pub has_recipients: bool,
    pub default_notify: bool,
}

/// Capability profile for a source kind — or, for `"pop3"`, a protocol.
///
/// POP3 accounts are `kind = "email"` but the protocol has no folders, no
/// server-side flags and no remote delete semantics worth exposing. Without a
/// separate profile the UI would offer archive and star buttons that quietly
/// do nothing on the server.
pub fn capabilities_for(kind: &str) -> SourceCapabilities {
    if kind == "pop3" {
        return SourceCapabilities {
            can_reply: true,
            can_forward: true,
            can_compose: true,
            // Everything below is what POP3 structurally cannot do.
            has_folders: false,
            remote_flags: false,
            remote_archive: false,
            remote_delete: false,
            has_recipients: true,
            default_notify: true,
        };
    }

    let email = kind == "email";
    SourceCapabilities {
        can_reply: email,
        can_forward: email,
        can_compose: email,
        has_folders: email,
        remote_flags: email,
        remote_archive: email,
        remote_delete: email,
        has_recipients: email,
        default_notify: matches!(kind, "email" | "webhook"),
    }
}

#[tauri::command]
pub fn get_source_capabilities() -> std::collections::HashMap<String, SourceCapabilities> {
    ["email", "pop3", "webhook", "script", "rss"]
        .into_iter()
        .map(|kind| (kind.to_string(), capabilities_for(kind)))
        .collect()
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/sources/"))]
pub struct SourceDto {
    pub id: String,
    pub display_name: String,
    pub identity: String,
    pub kind: String,
    pub provider: String,
    pub color: Option<String>,
    pub config: String,
    pub sync_state: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/sources/", optional_fields = nullable))]
pub struct CreateSourceInput {
    pub name: String,
    pub kind: String,
    pub identity: String,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub config: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/sources/", optional_fields = nullable))]
pub struct ImportOpmlInput {
    pub opml: String,
    #[serde(default = "default_poll_minutes")]
    pub poll_minutes: i64,
    #[serde(default)]
    pub notify: bool,
}

fn default_poll_minutes() -> i64 {
    30
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/sources/"))]
pub struct OpmlImportSummary {
    pub created: usize,
    pub skipped: usize,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/sources/"))]
pub struct IngestItem {
    #[serde(default)]
    pub external_id: Option<String>,
    #[serde(default)]
    pub thread_key: Option<String>,
    pub title: String,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub body_html: Option<String>,
    #[serde(default)]
    pub timestamp: Option<String>,
    #[serde(default)]
    pub level: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub data: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/sources/"))]
pub struct IngestResult {
    pub thread_id: String,
    pub message_id: String,
    pub created: bool,
}

static SOURCE_INGEST_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

#[tauri::command]
pub async fn list_sources(
    pool: tauri::State<'_, crate::db::ReadPool>,
) -> CommandResult<Vec<SourceDto>> {
    Ok(list_sources_inner(pool.inner().as_ref()).await?)
}

pub async fn list_sources_inner(pool: &SqlitePool) -> Result<Vec<SourceDto>, String> {
    sqlx::query_as("SELECT id, display_name, email AS identity, kind, provider, color, config, sync_state FROM accounts WHERE kind <> 'email' AND is_active=1 ORDER BY created_at")
        .fetch_all(pool).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn create_source(
    pool: tauri::State<'_, SqlitePool>,
    input: CreateSourceInput,
) -> CommandResult<SourceDto> {
    Ok(create_source_inner(pool.inner(), input).await?)
}

pub async fn create_source_inner(
    pool: &SqlitePool,
    input: CreateSourceInput,
) -> Result<SourceDto, String> {
    let name = clean(&input.name, 160, "name")?;
    let identity = clean(&input.identity, 2048, "identity")?;
    if !matches!(input.kind.as_str(), "webhook" | "script" | "rss") {
        return Err("unsupported source kind".into());
    }
    let id = format!("src-{}", uuid::Uuid::new_v4());
    let folder_id = format!("fld-{id}");
    let now = chrono::Utc::now().to_rfc3339();
    let mut config = input.config;
    if !config.is_object() {
        config = serde_json::json!({});
    }
    if input.kind == "rss" {
        let url = reqwest::Url::parse(&identity).map_err(|_| "invalid feed URL".to_string())?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err("feed URL must use http or https".into());
        }
    }
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    sqlx::query("INSERT INTO accounts (id,display_name,email,provider,kind,config,credentials_ref,sync_state,color,is_active,created_at,updated_at) VALUES (?,?,?,?,?,?,?,'{}',?,1,?,?)")
        .bind(&id).bind(&name).bind(&identity).bind(&input.kind).bind(&input.kind).bind(crate::commands::accounts::encrypt_account_config(&id, &config.to_string())?)
        .bind(format!("source://{id}")).bind(input.color.as_deref().unwrap_or("#4cd68a")).bind(&now).bind(&now)
        .execute(&mut *tx).await.map_err(|e| e.to_string())?;
    sqlx::query("INSERT INTO folders (id,account_id,name,full_path,folder_type,remote_id,message_count,unread_count,sync_enabled,sort_order,created_at,updated_at) VALUES (?,?,?,?, 'inbox',?,0,0,1,0,?,?)")
        .bind(&folder_id).bind(&id).bind(&name).bind(&name).bind(&id).bind(&now).bind(&now)
        .execute(&mut *tx).await.map_err(|e| e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())?;
    let dto = SourceDto {
        id: id.clone(),
        display_name: name,
        identity,
        kind: input.kind.clone(),
        provider: input.kind,
        color: input.color.or(Some("#4cd68a".into())),
        config: config.to_string(),
        sync_state: Some("{}".into()),
    };
    if dto.kind == "rss" {
        if let Err(error) = poll_rss_inner(pool, &id, true).await {
            let now = chrono::Utc::now().to_rfc3339();
            let _ = sqlx::query(
                "UPDATE accounts SET sync_status='error',last_sync_error=?,last_sync_error_at=?,updated_at=? WHERE id=?",
            )
            .bind(&error)
            .bind(&now)
            .bind(&now)
            .bind(&id)
            .execute(pool)
            .await;
        }
    }
    Ok(dto)
}

#[tauri::command]
pub async fn delete_source(
    pool: tauri::State<'_, SqlitePool>,
    source_id: String,
) -> CommandResult<()> {
    sqlx::query("UPDATE accounts SET is_active=0,updated_at=? WHERE id=? AND kind<>'email'")
        .bind(chrono::Utc::now().to_rfc3339())
        .bind(source_id)
        .execute(pool.inner())
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn import_opml(
    pool: tauri::State<'_, SqlitePool>,
    input: ImportOpmlInput,
) -> CommandResult<OpmlImportSummary> {
    if input.opml.len() > 2 * 1024 * 1024 {
        return Err("OPML exceeds 2 MiB".into());
    }
    let feeds = parse_opml(&input.opml);
    if feeds.is_empty() {
        return Err("OPML contains no feed outlines".into());
    }
    if feeds.len() > 500 {
        return Err("OPML contains more than 500 feeds".into());
    }
    let mut summary = OpmlImportSummary {
        created: 0,
        skipped: 0,
        errors: Vec::new(),
    };
    for (name, url) in feeds {
        let exists = sqlx::query_scalar::<_, i64>(
            "SELECT 1 FROM accounts WHERE kind='rss' AND is_active=1 AND LOWER(email)=LOWER(?)",
        )
        .bind(&url)
        .fetch_optional(pool.inner())
        .await
        .map_err(|error| error.to_string())?
        .is_some();
        if exists {
            summary.skipped += 1;
            continue;
        }
        let created = create_source_inner(
            pool.inner(),
            CreateSourceInput {
                name: name.clone(),
                kind: "rss".into(),
                identity: url.clone(),
                color: None,
                config: serde_json::json!({
                    "poll_minutes": input.poll_minutes.clamp(5, 24 * 60),
                    "notify": input.notify,
                }),
            },
        )
        .await;
        match created {
            Ok(_) => summary.created += 1,
            Err(error) => summary.errors.push(format!("{name}: {error}")),
        }
    }
    Ok(summary)
}

pub async fn publish_source_event(
    pool: &SqlitePool,
    events: &EventBus,
    source_id: &str,
    item: &IngestItem,
    result: &IngestResult,
) {
    if !result.created {
        return;
    }
    let source = sqlx::query_as::<_, (String, String, String)>(
        "SELECT kind,display_name,config FROM accounts WHERE id=? AND is_active=1",
    )
    .bind(source_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();
    let Some((kind, display_name, config)) = source else {
        return;
    };
    let Ok(config) = crate::commands::accounts::decrypt_account_config(source_id, &config) else {
        return;
    };
    if !source_should_notify(&kind, &config, item.level.as_deref()) {
        return;
    }
    events.publish(AppEvent::NewMail {
        event_id: result.message_id.clone(),
        account_id: source_id.to_string(),
        from_name: item.author.clone().or(Some(display_name)),
        from_address: format!("{kind}:{source_id}"),
        subject: item.title.clone(),
        preview: item.body.clone().unwrap_or_default(),
        thread_id: result.thread_id.clone(),
    });
}

fn source_should_notify(kind: &str, config: &str, level: Option<&str>) -> bool {
    let configured = serde_json::from_str::<serde_json::Value>(config)
        .ok()
        .and_then(|value| value.get("notify").and_then(serde_json::Value::as_bool));
    configured.unwrap_or(match kind {
        "webhook" => true,
        "script" => matches!(level, Some("warn" | "error")),
        _ => false,
    })
}

pub async fn ingest_source_inner(
    pool: &SqlitePool,
    source_id: &str,
    item: IngestItem,
) -> Result<IngestResult, String> {
    let title = clean(&item.title, 500, "title")?;
    let payload_size = serde_json::to_vec(&item).map_err(|e| e.to_string())?.len();
    if payload_size > 256 * 1024 {
        return Err("payload exceeds 256 KiB".into());
    }
    let (kind, name): (String, String) = sqlx::query_as(
        "SELECT kind,display_name FROM accounts WHERE id=? AND kind<>'email' AND is_active=1",
    )
    .bind(source_id)
    .fetch_one(pool)
    .await
    .map_err(|_| "source not found".to_string())?;
    let _ingest_guard = SOURCE_INGEST_LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await;
    let external = item
        .external_id
        .as_deref()
        .map(|value| clean(value, 500, "external_id"))
        .transpose()?
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    if let Some((mid, tid)) = sqlx::query_as::<_, (String, String)>(
        "SELECT id,thread_id FROM messages WHERE account_id=? AND remote_id=? LIMIT 1",
    )
    .bind(source_id)
    .bind(&external)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?
    {
        return Ok(IngestResult {
            thread_id: tid,
            message_id: mid,
            created: false,
        });
    }
    let thread_key = item
        .thread_key
        .as_deref()
        .map(|value| clean(value, 500, "thread_key"))
        .transpose()?
        .unwrap_or_else(|| external.clone());
    let thread_id = format!("th-{}-{}", slug(source_id), stable_key(&thread_key));
    let message_id = format!("m-{}-{}", slug(source_id), stable_key(&external));
    let now = match item.timestamp.as_deref() {
        Some(value) => chrono::DateTime::parse_from_rfc3339(value)
            .map_err(|_| "timestamp must be RFC3339".to_string())?
            .to_rfc3339(),
        None => chrono::Utc::now().to_rfc3339(),
    };
    let level = item.level.as_deref().unwrap_or("info");
    if !matches!(level, "info" | "warn" | "error") {
        return Err("level must be info, warn or error".into());
    }
    let priority = match level {
        "error" => 3,
        "warn" => 2,
        _ => 1,
    };
    let body = item.body.unwrap_or_default();
    let author = item
        .author
        .as_deref()
        .map(|value| clean(value, 320, "author"))
        .transpose()?
        .unwrap_or_else(|| name.clone());
    if let Some(url) = item.url.as_deref() {
        let parsed = reqwest::Url::parse(url).map_err(|_| "invalid item url".to_string())?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err("item url must use http or https".into());
        }
    }
    let sanitized_html =
        crate::email::sanitizer::sanitize_optional_email_html(item.body_html.clone())
            .await
            .map_err(|error| error.to_string())?;
    let payload = serde_json::json!({"kind":kind,"level":level,"url":item.url,"data":item.data});
    let folder_id: String = sqlx::query_scalar("SELECT id FROM folders WHERE account_id=? LIMIT 1")
        .bind(source_id)
        .fetch_one(pool)
        .await
        .map_err(|e| e.to_string())?;
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    sqlx::query("INSERT INTO threads (id,account_id,subject,last_message_date,message_count,unread_count,has_attachments,participants,snippet,workflow_state,workflow_changed_at,priority,board_column,category,created_at,updated_at) VALUES (?,?,?,?,1,1,0,?,?,'inbox',?,?,'inbox','updates',?,?) ON CONFLICT(id) DO UPDATE SET subject=excluded.subject,last_message_date=excluded.last_message_date,message_count=threads.message_count+1,unread_count=threads.unread_count+1,snippet=excluded.snippet,priority=MAX(threads.priority,excluded.priority),updated_at=excluded.updated_at")
        .bind(&thread_id).bind(source_id).bind(&title).bind(&now).bind(serde_json::json!([author]).to_string()).bind(&body).bind(&now).bind(priority).bind(&now).bind(&now)
        .execute(&mut *tx).await.map_err(|e|e.to_string())?;
    sqlx::query("INSERT INTO messages (id,account_id,folder_id,thread_id,message_id_header,from_address,from_name,to_addresses,cc_addresses,subject,date,body_text,body_html,original_body_html,snippet,is_read,is_sent,has_attachments,remote_id,source_payload,created_at,updated_at) VALUES (?,?,?,?,?,?,?,'[]','[]',?,?,?,?,?,?,0,0,0,?,?,?,?)")
        .bind(&message_id).bind(source_id).bind(folder_id).bind(&thread_id).bind(format!("<{message_id}@satchel.source>")).bind(format!("{kind}:{source_id}")).bind(&author).bind(&title).bind(&now).bind(&body).bind(sanitized_html).bind(item.body_html).bind(&body).bind(&external).bind(payload.to_string()).bind(&now).bind(&now)
        .execute(&mut *tx).await.map_err(|e|e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())?;
    let _ = crate::commands::rules::run_rules_for_thread_inner(pool, &thread_id).await;
    Ok(IngestResult {
        thread_id,
        message_id,
        created: true,
    })
}

#[tauri::command]
pub async fn poll_source(
    pool: tauri::State<'_, SqlitePool>,
    state: tauri::State<'_, AppState>,
    source_id: String,
) -> CommandResult<usize> {
    Ok(poll_rss_with_events(pool.inner(), &source_id, false, Some(state.events.as_ref())).await?)
}
pub fn spawn_rss_worker(pool: SqlitePool, events: std::sync::Arc<EventBus>) {
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            interval.tick().await;
            let sources = sqlx::query_as::<_, (String, String, Option<String>, Option<String>)>(
                "SELECT id,config,last_sync_at,last_sync_error_at FROM accounts WHERE kind='rss' AND is_active=1",
            )
            .fetch_all(&pool)
            .await
            .unwrap_or_default();
            let now = chrono::Utc::now();
            for (id, config, last_sync_at, last_error_at) in sources {
                if !feed_poll_due(
                    &config,
                    last_sync_at.as_deref(),
                    last_error_at.as_deref(),
                    now,
                ) {
                    continue;
                }
                if let Err(error) =
                    poll_rss_with_events(&pool, &id, false, Some(events.as_ref())).await
                {
                    let error_at = chrono::Utc::now().to_rfc3339();
                    let _ = sqlx::query("UPDATE accounts SET sync_status='error',last_sync_error=?,last_sync_error_at=?,updated_at=? WHERE id=?")
                        .bind(&error)
                        .bind(&error_at)
                        .bind(&error_at)
                        .bind(&id)
                        .execute(&pool)
                        .await;
                    tracing::warn!(source_id = id, "rss poll failed: {error}")
                }
            }
        }
    });
}

fn configured_poll_minutes(config: &str) -> i64 {
    serde_json::from_str::<serde_json::Value>(config)
        .ok()
        .and_then(|value| {
            value
                .get("poll_minutes")
                .and_then(serde_json::Value::as_i64)
        })
        .unwrap_or(30)
        .clamp(5, 24 * 60)
}

fn feed_poll_is_initial(state: &serde_json::Value, requested_initial: bool) -> bool {
    requested_initial || (state.get("rss_initialized").is_none() && state.get("rss_seen").is_none())
}

fn feed_poll_due(
    config: &str,
    last_sync_at: Option<&str>,
    last_error_at: Option<&str>,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    let last_attempt = [last_sync_at, last_error_at]
        .into_iter()
        .flatten()
        .filter_map(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&chrono::Utc))
        .max();
    last_attempt.is_none_or(|last| {
        now.signed_duration_since(last)
            >= chrono::Duration::minutes(configured_poll_minutes(config))
    })
}

pub async fn poll_rss_inner(
    pool: &SqlitePool,
    source_id: &str,
    initial: bool,
) -> Result<usize, String> {
    poll_rss_with_events(pool, source_id, initial, None).await
}

async fn poll_rss_with_events(
    pool: &SqlitePool,
    source_id: &str,
    initial: bool,
    events: Option<&EventBus>,
) -> Result<usize, String> {
    let (url, state): (String, Option<String>) = sqlx::query_as(
        "SELECT email,sync_state FROM accounts WHERE id=? AND kind='rss' AND is_active=1",
    )
    .bind(source_id)
    .fetch_one(pool)
    .await
    .map_err(|_| "rss source not found".to_string())?;
    let mut state_value = state
        .as_deref()
        .and_then(|value| serde_json::from_str::<serde_json::Value>(value).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    let initial = feed_poll_is_initial(&state_value, initial);
    let current_etag = state_value
        .get("rss_etag")
        .and_then(serde_json::Value::as_str);
    let current_modified = state_value
        .get("rss_last_modified")
        .and_then(serde_json::Value::as_str);
    let Some(mut document) = fetch_feed_document(&url, current_etag, current_modified).await?
    else {
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query("UPDATE accounts SET sync_status='idle',last_sync_error=NULL,last_sync_error_at=NULL,last_sync_at=?,updated_at=? WHERE id=?")
            .bind(&now)
            .bind(&now)
            .bind(source_id)
            .execute(pool)
            .await
            .map_err(|error| error.to_string())?;
        return Ok(0);
    };
    let mut effective_url = url.clone();
    let mut entries = parse_feed(&document.body);
    if entries.is_empty() {
        let discovered = discover_feed_url(&url, &document.body).ok_or_else(|| {
            "URL contains neither RSS/Atom nor a discoverable feed link".to_string()
        })?;
        document = fetch_feed_document(&discovered, None, None)
            .await?
            .ok_or_else(|| "discovered feed unexpectedly returned not-modified".to_string())?;
        entries = parse_feed(&document.body);
        if entries.is_empty() {
            return Err("discovered URL is not a supported RSS/Atom feed".into());
        }
        effective_url = discovered;
        sqlx::query("UPDATE accounts SET email=?,updated_at=? WHERE id=?")
            .bind(&effective_url)
            .bind(chrono::Utc::now().to_rfc3339())
            .bind(source_id)
            .execute(pool)
            .await
            .map_err(|error| error.to_string())?;
    }
    entries.sort_by_key(|entry| std::cmp::Reverse(entry_timestamp(entry)));
    let mut seen = state_value
        .get("rss_seen")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect::<Vec<_>>();
    let watermark = state_value
        .get("rss_watermark")
        .and_then(serde_json::Value::as_str)
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&chrono::Utc));
    let mut created = 0;
    let candidates = feed_candidates(&entries, &seen, initial, watermark);
    for item in candidates.into_iter().rev() {
        let result = ingest_source_inner(pool, source_id, item.clone()).await?;
        if result.created {
            created += 1
        }
        if let Some(events) = events {
            publish_source_event(pool, events, source_id, item, &result).await;
        }
    }
    for item in &entries {
        if let Some(id) = &item.external_id {
            if !seen.contains(id) {
                seen.push(id.clone())
            }
        }
    }
    if seen.len() > 500 {
        seen = seen.split_off(seen.len() - 500)
    }
    state_value["rss_seen"] = serde_json::json!(seen);
    state_value["rss_initialized"] = serde_json::json!(true);
    if let Some(latest) = entries.iter().filter_map(entry_timestamp).max() {
        let watermark = match watermark {
            Some(current) => current.max(latest),
            None => latest,
        };
        state_value["rss_watermark"] = serde_json::json!(watermark.to_rfc3339());
    }
    if let Some(etag) = document.etag {
        state_value["rss_etag"] = serde_json::json!(etag);
    }
    if let Some(last_modified) = document.last_modified {
        state_value["rss_last_modified"] = serde_json::json!(last_modified);
    }
    state_value["rss_url"] = serde_json::json!(effective_url);
    sqlx::query("UPDATE accounts SET sync_state=?,sync_status='idle',last_sync_error=NULL,last_sync_error_at=NULL,last_sync_at=?,updated_at=? WHERE id=?")
        .bind(state_value.to_string())
        .bind(chrono::Utc::now().to_rfc3339())
        .bind(chrono::Utc::now().to_rfc3339())
        .bind(source_id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(created)
}

struct FeedDocument {
    body: String,
    etag: Option<String>,
    last_modified: Option<String>,
}

async fn fetch_feed_document(
    url: &str,
    etag: Option<&str>,
    last_modified: Option<&str>,
) -> Result<Option<FeedDocument>, String> {
    let client = satchel_core::utils::http::client_builder()
        .timeout(std::time::Duration::from_secs(30))
        .user_agent("Satchel/0.1 feed reader")
        .build()
        .map_err(|error| error.to_string())?;
    let mut request = client.get(url);
    if let Some(etag) = etag {
        request = request.header(reqwest::header::IF_NONE_MATCH, etag);
    }
    if let Some(modified) = last_modified {
        request = request.header(reqwest::header::IF_MODIFIED_SINCE, modified);
    }
    let response = request.send().await.map_err(|error| error.to_string())?;
    if response.status() == reqwest::StatusCode::NOT_MODIFIED {
        return Ok(None);
    }
    let mut response = response
        .error_for_status()
        .map_err(|error| error.to_string())?;
    let etag = response
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|value| value.to_str().ok())
        .map(ToString::to_string);
    let last_modified = response
        .headers()
        .get(reqwest::header::LAST_MODIFIED)
        .and_then(|value| value.to_str().ok())
        .map(ToString::to_string);
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|error| error.to_string())? {
        if body.len() + chunk.len() > 2 * 1024 * 1024 {
            return Err("feed/page exceeds 2 MiB".into());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(Some(FeedDocument {
        body: String::from_utf8(body).map_err(|_| "feed/page is not valid UTF-8".to_string())?,
        etag,
        last_modified,
    }))
}

fn discover_feed_url(base_url: &str, html: &str) -> Option<String> {
    let base = reqwest::Url::parse(base_url).ok()?;
    let lower = html.to_ascii_lowercase();
    let mut offset = 0;
    while let Some(relative_start) = lower[offset..].find("<link") {
        let start = offset + relative_start;
        let end = html[start..].find('>')? + start + 1;
        let tag = &html[start..end];
        let rel = markup_attr(tag, "rel").unwrap_or_default();
        let content_type = markup_attr(tag, "type").unwrap_or_default();
        let href = markup_attr(tag, "href");
        let alternate = rel
            .split_whitespace()
            .any(|value| value.eq_ignore_ascii_case("alternate"));
        let feed_type = matches!(
            content_type.to_ascii_lowercase().as_str(),
            "application/rss+xml" | "application/atom+xml" | "application/feed+json"
        );
        if alternate && feed_type {
            let resolved = base.join(&href?).ok()?;
            if matches!(resolved.scheme(), "http" | "https") {
                return Some(resolved.to_string());
            }
        }
        offset = end;
    }
    None
}

fn markup_attr(tag: &str, name: &str) -> Option<String> {
    let bytes = tag.as_bytes();
    let lower = tag.to_ascii_lowercase();
    let name = name.to_ascii_lowercase();
    let mut offset = 0;
    while let Some(position) = lower[offset..].find(&name) {
        let start = offset + position;
        let before_ok = start == 0 || !bytes[start - 1].is_ascii_alphanumeric();
        let mut cursor = start + name.len();
        while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
            cursor += 1;
        }
        if !before_ok || bytes.get(cursor) != Some(&b'=') {
            offset = start + name.len();
            continue;
        }
        cursor += 1;
        while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
            cursor += 1;
        }
        let quote = *bytes.get(cursor)?;
        if quote != b'\'' && quote != b'"' {
            return None;
        }
        cursor += 1;
        let end = tag[cursor..].find(char::from(quote))? + cursor;
        return Some(tag[cursor..end].to_string());
    }
    None
}

fn parse_opml(opml: &str) -> Vec<(String, String)> {
    let lower = opml.to_ascii_lowercase();
    let mut offset = 0;
    let mut feeds = Vec::new();
    while let Some(relative_start) = lower[offset..].find("<outline") {
        let start = offset + relative_start;
        let Some(relative_end) = opml[start..].find('>') else {
            break;
        };
        let end = start + relative_end + 1;
        let tag = &opml[start..end];
        if let Some(url) = markup_attr(tag, "xmlurl") {
            if reqwest::Url::parse(&url)
                .ok()
                .is_some_and(|value| matches!(value.scheme(), "http" | "https"))
            {
                let name = markup_attr(tag, "title")
                    .or_else(|| markup_attr(tag, "text"))
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or_else(|| url.clone());
                if !feeds.iter().any(|(_, existing)| existing == &url) {
                    feeds.push((name, url));
                }
            }
        }
        offset = end;
    }
    feeds
}

fn feed_candidates<'a>(
    entries: &'a [IngestItem],
    seen: &[String],
    initial: bool,
    watermark: Option<chrono::DateTime<chrono::Utc>>,
) -> Vec<&'a IngestItem> {
    if initial {
        let newest_dated = entries
            .iter()
            .filter(|entry| entry_timestamp(entry).is_some())
            .max_by_key(|entry| entry_timestamp(entry))
            .or_else(|| entries.first());
        newest_dated.into_iter().collect()
    } else {
        entries
            .iter()
            .filter(|entry| !seen.contains(&entry.external_id.clone().unwrap_or_default()))
            .filter(|entry| {
                watermark.is_none_or(|watermark| {
                    entry_timestamp(entry).is_none_or(|timestamp| timestamp >= watermark)
                })
            })
            .collect()
    }
}

fn entry_timestamp(item: &IngestItem) -> Option<chrono::DateTime<chrono::Utc>> {
    item.timestamp
        .as_deref()
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&chrono::Utc))
}
fn parse_feed(xml: &str) -> Vec<IngestItem> {
    let tag = if xml.contains("<entry") {
        "entry"
    } else {
        "item"
    };
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find(&format!("<{tag}")) {
        rest = &rest[start..];
        let Some(open) = rest.find('>') else { break };
        let Some(end) = rest.find(&format!("</{tag}>")) else {
            break;
        };
        let block = &rest[open + 1..end];
        let title = xml_value(block, "title").unwrap_or_else(|| "Ohne Titel".into());
        let link = xml_value(block, "link").or_else(|| xml_attr(block, "link", "href"));
        let id = xml_value(block, "guid")
            .or_else(|| xml_value(block, "id"))
            .or_else(|| link.clone())
            .unwrap_or_else(|| format!("{:x}", Sha256::digest(block.as_bytes())));
        let body = xml_value(block, "description")
            .or_else(|| xml_value(block, "summary"))
            .or_else(|| xml_value(block, "content"));
        let timestamp = xml_value(block, "pubDate")
            .and_then(|d| {
                chrono::DateTime::parse_from_rfc2822(&d)
                    .ok()
                    .map(|v| v.to_rfc3339())
            })
            .or_else(|| {
                xml_value(block, "updated").and_then(|value| {
                    chrono::DateTime::parse_from_rfc3339(&value)
                        .ok()
                        .map(|timestamp| timestamp.to_rfc3339())
                })
            });
        out.push(IngestItem {
            external_id: Some(id),
            thread_key: None,
            title: crate::utils::mime::decode_html_entities(&strip_tags(&title)),
            author: xml_value(block, "author").or_else(|| xml_value(block, "dc:creator")),
            body: body
                .as_deref()
                .map(|v| crate::utils::mime::decode_html_entities(&strip_tags(v))),
            body_html: body,
            timestamp,
            level: Some("info".into()),
            url: link,
            data: None,
        });
        rest = &rest[end + tag.len() + 3..];
    }
    out
}
fn xml_value(block: &str, tag: &str) -> Option<String> {
    let start = block.find(&format!("<{tag}"))?;
    let tail = &block[start..];
    let open = tail.find('>')?;
    let end = tail.find(&format!("</{tag}>"))?;
    Some(
        tail[open + 1..end]
            .trim()
            .trim_start_matches("<![CDATA[")
            .trim_end_matches("]]>")
            .to_string(),
    )
}
fn xml_attr(block: &str, tag: &str, attr: &str) -> Option<String> {
    let start = block.find(&format!("<{tag}"))?;
    let tail = &block[start..block[start..].find('>')? + start];
    let key = format!("{attr}=\"");
    let p = tail.find(&key)? + key.len();
    Some(tail[p..].split('"').next()?.to_string())
}
fn strip_tags(v: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for c in v.chars() {
        if c == '<' {
            inside = true
        } else if c == '>' {
            inside = false
        } else if !inside {
            out.push(c)
        }
    }
    out
}

fn clean(value: &str, max: usize, field: &str) -> Result<String, String> {
    let v = value.trim();
    if v.is_empty() || v.chars().count() > max || v.chars().any(char::is_control) {
        return Err(format!("invalid {field}"));
    }
    Ok(v.to_string())
}
fn slug(v: &str) -> String {
    v.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .chars()
        .take(120)
        .collect()
}

fn stable_key(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
        .chars()
        .take(24)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn source_ingest_sanitizes_html_and_keeps_original() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::db::init(&pool).await.unwrap();
        sqlx::query("INSERT INTO accounts (id, display_name, email, provider, kind, config, credentials_ref, created_at, updated_at) VALUES ('source', 'Feed', 'https://example.test/feed', 'rss', 'rss', '{}', '', '', '')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO folders (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at) VALUES ('folder', 'source', 'Feed', 'Feed', 'inbox', 'source', '', '')").execute(&pool).await.unwrap();
        let item: super::IngestItem = serde_json::from_value(serde_json::json!({"title":"Item", "body_html":"<p onclick=bad()>Hi<script>bad()</script></p>"})).unwrap();
        let result = super::ingest_source_inner(&pool, "source", item)
            .await
            .unwrap();
        let (cleaned, original): (String, String) =
            sqlx::query_as("SELECT body_html, original_body_html FROM messages WHERE id = ?")
                .bind(&result.message_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(cleaned, "<p>Hi</p>");
        assert!(original.contains("<script>"));
    }

    #[test]
    fn capability_boundary() {
        assert!(capabilities_for("email").can_reply);
        assert!(!capabilities_for("rss").can_reply);
        assert!(!capabilities_for("rss").default_notify)
    }
    #[test]
    fn external_ids_that_slug_equally_stay_distinct() {
        assert_ne!(stable_key("build/a"), stable_key("build-a"));
    }
    #[test]
    fn notification_defaults_are_source_appropriate_and_overridable() {
        assert!(source_should_notify("webhook", "{}", Some("info")));
        assert!(!source_should_notify("rss", "{}", Some("error")));
        assert!(source_should_notify("script", "{}", Some("error")));
        assert!(!source_should_notify("script", "{}", Some("info")));
        assert!(!source_should_notify(
            "webhook",
            r#"{"notify":false}"#,
            Some("error")
        ));
        assert!(source_should_notify(
            "rss",
            r#"{"notify":true}"#,
            Some("info")
        ));
    }
    #[test]
    fn parses_rss_and_atom() {
        assert_eq!(parse_feed("<rss><item><title>Neu</title><guid>1</guid><description>Text</description></item></rss>").len(),1);
        assert_eq!(
            parse_feed(
                "<feed><entry><title>A</title><id>x</id><summary>B</summary></entry></feed>"
            )
            .len(),
            1
        )
    }

    #[test]
    fn initial_feed_import_only_takes_newest_then_all_unseen() {
        let entries = vec![
            IngestItem {
                external_id: Some("new".into()),
                thread_key: None,
                title: "New".into(),
                author: None,
                body: None,
                body_html: None,
                timestamp: Some("2026-07-16T12:00:00Z".into()),
                level: None,
                url: None,
                data: None,
            },
            IngestItem {
                external_id: Some("old".into()),
                thread_key: None,
                title: "Old".into(),
                author: None,
                body: None,
                body_html: None,
                timestamp: Some("2026-07-15T12:00:00Z".into()),
                level: None,
                url: None,
                data: None,
            },
        ];
        assert_eq!(feed_candidates(&entries, &[], true, None).len(), 1);
        assert_eq!(
            feed_candidates(&entries, &["old".into()], false, None)[0]
                .external_id
                .as_deref(),
            Some("new")
        );
    }

    #[test]
    fn feed_watermark_uses_actual_instants_and_ignores_late_backfill() {
        let entries = vec![
            IngestItem {
                external_id: Some("lexically-later".into()),
                thread_key: None,
                title: "Older instant".into(),
                author: None,
                body: None,
                body_html: None,
                timestamp: Some("2026-07-16T01:00:00+14:00".into()),
                level: None,
                url: None,
                data: None,
            },
            IngestItem {
                external_id: Some("actually-newer".into()),
                thread_key: None,
                title: "Newer instant".into(),
                author: None,
                body: None,
                body_html: None,
                timestamp: Some("2026-07-15T23:00:00Z".into()),
                level: None,
                url: None,
                data: None,
            },
        ];
        let initial = feed_candidates(&entries, &[], true, None);
        assert_eq!(initial[0].external_id.as_deref(), Some("actually-newer"));

        let watermark = chrono::DateTime::parse_from_rfc3339("2026-07-15T22:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let later = feed_candidates(&entries, &["actually-newer".into()], false, Some(watermark));
        assert!(later.is_empty());
    }
    #[test]
    fn feed_poll_interval_is_configurable_and_uses_last_attempt() {
        let now = chrono::Utc::now();
        let recent = (now - chrono::Duration::minutes(10)).to_rfc3339();
        let old = (now - chrono::Duration::minutes(31)).to_rfc3339();
        assert!(!feed_poll_due(
            r#"{"poll_minutes":30}"#,
            Some(&recent),
            None,
            now
        ));
        assert!(feed_poll_due(
            r#"{"poll_minutes":30}"#,
            Some(&old),
            None,
            now
        ));
        assert!(!feed_poll_due(
            r#"{"poll_minutes":30}"#,
            None,
            Some(&recent),
            now
        ));
        assert_eq!(configured_poll_minutes(r#"{"poll_minutes":1}"#), 5);
    }
    #[test]
    fn a_failed_initial_feed_poll_remains_initial_on_retry() {
        assert!(feed_poll_is_initial(&serde_json::json!({}), false));
        assert!(!feed_poll_is_initial(
            &serde_json::json!({"rss_initialized":true,"rss_seen":[]}),
            false
        ));
        assert!(!feed_poll_is_initial(
            &serde_json::json!({"rss_seen":["legacy-id"]}),
            false
        ));
    }
    #[test]
    fn discovers_relative_rss_and_atom_links_from_html() {
        assert_eq!(
            discover_feed_url(
                "https://example.com/blog/",
                r#"<html><head><link rel="alternate" type="application/rss+xml" href="../feed.xml"></head></html>"#,
            )
            .as_deref(),
            Some("https://example.com/feed.xml")
        );
        assert_eq!(
            discover_feed_url(
                "https://example.com/",
                r#"<LINK HREF='/atom' TYPE='application/atom+xml' REL='alternate'>"#,
            )
            .as_deref(),
            Some("https://example.com/atom")
        );
        assert!(discover_feed_url(
            "https://example.com/",
            r#"<link rel="stylesheet" href="style.css">"#
        )
        .is_none());
    }
    #[test]
    fn parses_and_deduplicates_opml_feed_outlines() {
        let feeds = parse_opml(
            r#"<opml><body>
              <outline text="Example" xmlUrl="https://example.com/feed.xml" />
              <outline title='Atom' xmlUrl='https://example.com/atom' />
              <outline text="Duplicate" xmlUrl="https://example.com/feed.xml" />
              <outline text="Unsafe" xmlUrl="file:///tmp/feed" />
            </body></opml>"#,
        );
        assert_eq!(feeds.len(), 2);
        assert_eq!(feeds[0].0, "Example");
        assert_eq!(feeds[1].1, "https://example.com/atom");
    }
}
