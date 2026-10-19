//! Server-side search: Gmail's `q=` and Microsoft Graph's `$search=`.
//! For IMAP we already have the local FTS5 index, plus a basic IMAP SEARCH path
//! in providers/imap.rs. This command unifies the surface for the frontend.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use sqlx::{QueryBuilder, Sqlite, SqlitePool};

use crate::commands::error::{CommandError, CommandResult};
use crate::providers::gmail::GmailProvider;
use crate::providers::outlook::OutlookProvider;
use crate::providers::traits::MessageEnvelope;
use crate::sync::{
    gmail_reconcile::ensure_gmail_archive_folder, message_ingest::upsert_provider_message,
};
use crate::utils::oauth::{access_token_from_account_config, OAuthProvider};

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/server_search/", optional_fields = nullable))]
pub struct ServerSearchInput {
    pub account_id: String,
    pub query: String,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/server_search/")
)]
pub struct ServerSearchHit {
    pub remote_id: String,
    pub subject: String,
    pub snippet: String,
    pub from: String,
    pub date: Option<String>,
}

#[tauri::command]
pub async fn server_search(
    pool: tauri::State<'_, SqlitePool>,
    input: ServerSearchInput,
) -> CommandResult<Vec<ServerSearchHit>> {
    let row: (String, String) =
        sqlx::query_as("SELECT provider, config FROM accounts WHERE id = ? LIMIT 1")
            .bind(&input.account_id)
            .fetch_one(pool.inner())
            .await
            .map_err(|err| err.to_string())?;
    let config = crate::commands::accounts::decrypt_account_config(&input.account_id, &row.1)?;
    let hits = match row.0.as_str() {
        "gmail" => gmail_search(&config, &input.query).await?,
        "outlook" => outlook_search(&config, &input.query).await?,
        // For IMAP the local FTS index already covers everything the server
        // knows. Returning empty keeps the frontend on local results.
        _ => Vec::new(),
    };
    // Only server-only mail is interesting: everything already synced is found
    // (and ranked) by the local search.
    drop_locally_known_hits(pool.inner(), &input.account_id, hits)
        .await
        .map_err(CommandError::from)
}

/// Removes hits whose remote id already exists as a local message.
pub(crate) async fn drop_locally_known_hits(
    pool: &SqlitePool,
    account_id: &str,
    hits: Vec<ServerSearchHit>,
) -> Result<Vec<ServerSearchHit>, String> {
    if hits.is_empty() {
        return Ok(hits);
    }

    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT remote_id FROM messages WHERE remote_id IS NOT NULL AND account_id = ",
    );
    query.push_bind(account_id.to_string());
    query.push(" AND remote_id IN (");
    let mut ids = query.separated(", ");
    for hit in &hits {
        ids.push_bind(hit.remote_id.clone());
    }
    query.push(")");

    let known: HashSet<String> = query
        .build_query_scalar::<String>()
        .fetch_all(pool)
        .await
        .map_err(|err| err.to_string())?
        .into_iter()
        .collect();

    Ok(hits
        .into_iter()
        .filter(|hit| !known.contains(&hit.remote_id))
        .collect())
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/server_search/", optional_fields = nullable))]
pub struct FetchServerHitInput {
    pub account_id: String,
    pub remote_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/server_search/")
)]
pub struct FetchedServerHit {
    pub message_id: String,
    pub thread_id: String,
}

/// Downloads a single server-search hit through the regular ingest pipeline
/// (threading, sanitizing, FTS via triggers) so it becomes a normal local
/// message, and returns the ids needed to open it.
#[tauri::command]
pub async fn fetch_server_search_hit(
    pool: tauri::State<'_, SqlitePool>,
    input: FetchServerHitInput,
) -> CommandResult<FetchedServerHit> {
    fetch_server_search_hit_impl(pool.inner(), &input)
        .await
        .map_err(CommandError::from)
}

pub(crate) async fn fetch_server_search_hit_impl(
    pool: &SqlitePool,
    input: &FetchServerHitInput,
) -> Result<FetchedServerHit, String> {
    // A sync run may have caught up in the meantime — then there is nothing to do.
    if let Some(existing) = find_local_message(pool, &input.account_id, &input.remote_id).await? {
        return Ok(existing);
    }

    let row: (String, String, String, String) = sqlx::query_as(
        "SELECT provider, email, display_name, config FROM accounts WHERE id = ? LIMIT 1",
    )
    .bind(&input.account_id)
    .fetch_one(pool)
    .await
    .map_err(|err| err.to_string())?;
    let (provider, email, display_name, config) = row;
    let config = crate::commands::accounts::decrypt_account_config(&input.account_id, &config)?;

    let (envelope, folder_hint) = match provider.as_str() {
        "gmail" => (
            GmailProvider::from_account_config(&config)
                .fetch_message_by_id(&input.remote_id)
                .await
                .map_err(|error| error.to_string())?,
            None,
        ),
        "outlook" => OutlookProvider::from_account_config(&config)
            .fetch_message_by_id(&input.remote_id)
            .await
            .map_err(|error| error.to_string())?,
        other => {
            return Err(format!(
                "targeted download is not supported for provider {other}"
            ))
        }
    };

    let (folder_id, folder_remote_id, folder_type) =
        resolve_target_folder(pool, &input.account_id, &provider, &envelope, folder_hint).await?;

    upsert_provider_message(
        None,
        pool,
        &provider,
        &input.account_id,
        &email,
        &display_name,
        &config,
        &folder_id,
        &folder_remote_id,
        &folder_type,
        &envelope,
        None,
    )
    .await?;

    find_local_message(pool, &input.account_id, &input.remote_id)
        .await?
        .ok_or_else(|| "fetched message was not persisted".to_string())
}

async fn find_local_message(
    pool: &SqlitePool,
    account_id: &str,
    remote_id: &str,
) -> Result<Option<FetchedServerHit>, String> {
    let row: Option<(String, String)> = sqlx::query_as(
        "SELECT id, thread_id FROM messages WHERE account_id = ? AND remote_id = ? LIMIT 1",
    )
    .bind(account_id)
    .bind(remote_id)
    .fetch_optional(pool)
    .await
    .map_err(|err| err.to_string())?;
    Ok(row.map(|(message_id, thread_id)| FetchedServerHit {
        message_id,
        thread_id,
    }))
}

/// Picks the local folder a fetched message belongs into: the first local
/// folder matching the provider's mailbox ids (Gmail labels resp. the Graph
/// parent folder), preferring primary mailboxes. Falls back to Gmail's
/// synthetic archive resp. the account's inbox.
pub(crate) async fn resolve_target_folder(
    pool: &SqlitePool,
    account_id: &str,
    provider: &str,
    envelope: &MessageEnvelope,
    folder_hint: Option<String>,
) -> Result<(String, String, String), String> {
    let candidates: Vec<String> = folder_hint
        .into_iter()
        .chain(envelope.mailbox_ids.iter().cloned())
        .collect();

    if !candidates.is_empty() {
        let mut query = QueryBuilder::<Sqlite>::new(
            "SELECT id, remote_id, folder_type FROM folders WHERE account_id = ",
        );
        query.push_bind(account_id.to_string());
        query.push(" AND remote_id IN (");
        let mut ids = query.separated(", ");
        for candidate in &candidates {
            ids.push_bind(candidate.clone());
        }
        query.push(")");

        let rows: Vec<(String, String, String)> = query
            .build_query_as()
            .fetch_all(pool)
            .await
            .map_err(|err| err.to_string())?;
        let rank = |folder_type: &str| match folder_type {
            "inbox" => 0,
            "sent" => 1,
            "archive" => 2,
            "drafts" => 3,
            _ => 4,
        };
        if let Some(row) = rows
            .into_iter()
            .min_by_key(|(_, _, folder_type)| rank(folder_type))
        {
            return Ok(row);
        }
    }

    if provider == "gmail" {
        let archive_id = ensure_gmail_archive_folder(pool, account_id).await?;
        return Ok((archive_id, "ARCHIVE".to_string(), "archive".to_string()));
    }

    let inbox: Option<(String, String, String)> = sqlx::query_as(
        "SELECT id, remote_id, folder_type FROM folders
         WHERE account_id = ? AND folder_type = 'inbox' LIMIT 1",
    )
    .bind(account_id)
    .fetch_optional(pool)
    .await
    .map_err(|err| err.to_string())?;
    inbox.ok_or_else(|| "no local folder available for the fetched message".to_string())
}

async fn gmail_search(
    config_json: &crate::commands::accounts::AccountConfig,
    query: &str,
) -> Result<Vec<ServerSearchHit>, String> {
    let token = access_token_from_account_config(OAuthProvider::Gmail, config_json).await?;
    let client = satchel_core::utils::http::client();
    let list = client
        .get("https://gmail.googleapis.com/gmail/v1/users/me/messages")
        .bearer_auth(&token)
        .query(&[("q", query), ("maxResults", "30")])
        .send()
        .await
        .map_err(|err| format!("gmail search list failed: {err}"))?;
    if !list.status().is_success() {
        let status = list.status();
        let body = list.text().await.unwrap_or_default();
        return Err(format!("gmail search failed with status {status}: {body}"));
    }
    let payload: serde_json::Value = list
        .json()
        .await
        .map_err(|err| format!("gmail search parse failed: {err}"))?;

    let ids: Vec<String> = payload
        .get("messages")
        .and_then(|m| m.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| m.get("id").and_then(|v| v.as_str().map(str::to_string)))
                .collect()
        })
        .unwrap_or_default();

    let mut hits = Vec::with_capacity(ids.len());
    for id in ids {
        let detail = client
            .get(format!(
                "https://gmail.googleapis.com/gmail/v1/users/me/messages/{id}"
            ))
            .bearer_auth(&token)
            .query(&[
                ("format", "metadata"),
                ("metadataHeaders", "From"),
                ("metadataHeaders", "Subject"),
                ("metadataHeaders", "Date"),
            ])
            .send()
            .await
            .map_err(|err| format!("gmail message get failed: {err}"))?;
        if !detail.status().is_success() {
            continue;
        }
        let v: serde_json::Value = detail.json().await.unwrap_or(serde_json::Value::Null);
        let snippet = v
            .get("snippet")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string();
        let mut subject = String::new();
        let mut from = String::new();
        let mut date = None;
        if let Some(headers) = v.pointer("/payload/headers").and_then(|h| h.as_array()) {
            for header in headers {
                let name = header.get("name").and_then(|n| n.as_str()).unwrap_or("");
                let value = header.get("value").and_then(|v| v.as_str()).unwrap_or("");
                match name.to_ascii_lowercase().as_str() {
                    "subject" => subject = value.to_string(),
                    "from" => from = value.to_string(),
                    "date" => date = Some(value.to_string()),
                    _ => {}
                }
            }
        }
        hits.push(ServerSearchHit {
            remote_id: id,
            subject,
            snippet,
            from,
            date,
        });
    }
    Ok(hits)
}

async fn outlook_search(
    config_json: &crate::commands::accounts::AccountConfig,
    query: &str,
) -> Result<Vec<ServerSearchHit>, String> {
    let token = access_token_from_account_config(OAuthProvider::Outlook, config_json).await?;
    let client = satchel_core::utils::http::client();
    // Microsoft Graph requires `ConsistencyLevel: eventual` for $search.
    let escaped = query.replace('"', "\\\"");
    let url = format!(
        "https://graph.microsoft.com/v1.0/me/messages?$search=\"{escaped}\"&$top=30&$select=id,subject,bodyPreview,from,receivedDateTime"
    );
    let response = client
        .get(&url)
        .bearer_auth(&token)
        .header("ConsistencyLevel", "eventual")
        .send()
        .await
        .map_err(|err| format!("outlook search failed: {err}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!(
            "outlook search failed with status {status}: {body}"
        ));
    }
    let payload: serde_json::Value = response
        .json()
        .await
        .map_err(|err| format!("outlook search parse failed: {err}"))?;

    let value = payload
        .get("value")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut hits = Vec::with_capacity(value.len());
    for v in value {
        let from = v
            .pointer("/from/emailAddress/address")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string();
        hits.push(ServerSearchHit {
            remote_id: v
                .get("id")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string(),
            subject: v
                .get("subject")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string(),
            snippet: v
                .get("bodyPreview")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string(),
            from,
            date: v
                .get("receivedDateTime")
                .and_then(|s| s.as_str())
                .map(str::to_string),
        });
    }
    Ok(hits)
}
