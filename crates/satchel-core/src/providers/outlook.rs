use base64::Engine as _;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

use crate::email::text::strip_html_tags;
use crate::{
    providers::{
        cursor,
        traits::{
            FetchedAttachment, FlagUpdate, MailProvider, MessageEnvelope, MessageRef,
            OutgoingMessage, RemoteAttachment, RemoteFolder, RemoteIdTranslation,
            RemoteMessageRemoval, SyncBatch, SyncCheckpoint,
        },
    },
    utils::oauth::{access_token_from_account_config, OAuthProvider},
};

mod batch;

const GRAPH_BASE_DEFAULT: &str = "https://graph.microsoft.com/v1.0";

/// Base URL of the Microsoft Graph API.
///
/// Read per call rather than cached so a test can point the provider at a local
/// fake; nothing in production sets the variable.
fn graph_api_base() -> String {
    crate::utils::environment::debug_env("SATCHEL_GRAPH_API_BASE")
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| GRAPH_BASE_DEFAULT.to_string())
}

/// Delta pages a single run may read before it stops and records where it got to.
///
/// A mailbox whose delta token expired re-enumerates from the start, and the
/// cursor used to be written only once the last page had arrived. Any
/// interruption — rate limit, token expiry, network change — threw the entire
/// run away, so a large mailbox began at page 1 again and never finished a pass.
const MAX_DELTA_PAGES_PER_RUN: usize = 25;

fn max_delta_pages_per_run() -> usize {
    std::env::var("SATCHEL_OUTLOOK_DELTA_PAGES")
        .ok()
        .and_then(|raw| raw.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(MAX_DELTA_PAGES_PER_RUN)
}

#[derive(Debug, Clone)]
pub struct OutlookProvider {
    config_json: crate::accounts::AccountConfig,
    /// One pool per provider: a client per call was a TLS handshake per call.
    client: reqwest::Client,
}

#[derive(Debug, Deserialize)]
struct GraphListResponse<T> {
    value: Vec<T>,
    #[serde(rename = "@odata.nextLink")]
    next_link: Option<String>,
    #[serde(rename = "@odata.deltaLink")]
    delta_link: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GraphMailFolder {
    id: String,
    #[serde(rename = "displayName")]
    display_name: String,
    #[serde(rename = "childFolderCount")]
    child_folder_count: Option<i64>,
    #[serde(rename = "totalItemCount")]
    total_item_count: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct GraphMessage {
    id: String,
    #[serde(rename = "parentFolderId")]
    parent_folder_id: Option<String>,
    #[serde(rename = "conversationId")]
    conversation_id: Option<String>,
    subject: Option<String>,
    from: Option<GraphRecipientWrapper>,
    #[serde(rename = "toRecipients")]
    to_recipients: Option<Vec<GraphRecipientWrapper>>,
    #[serde(rename = "ccRecipients")]
    cc_recipients: Option<Vec<GraphRecipientWrapper>>,
    #[serde(rename = "bccRecipients")]
    bcc_recipients: Option<Vec<GraphRecipientWrapper>>,
    #[serde(rename = "replyTo")]
    reply_to: Option<Vec<GraphRecipientWrapper>>,
    #[serde(rename = "receivedDateTime")]
    received_date_time: Option<String>,
    #[serde(rename = "sentDateTime")]
    sent_date_time: Option<String>,
    #[serde(rename = "internetMessageId")]
    internet_message_id: Option<String>,
    #[serde(rename = "internetMessageHeaders")]
    internet_message_headers: Option<Vec<GraphInternetHeader>>,
    #[serde(rename = "isDraft")]
    is_draft: Option<bool>,
    #[serde(rename = "bodyPreview")]
    body_preview: Option<String>,
    #[serde(rename = "isRead")]
    is_read: Option<bool>,
    body: Option<GraphItemBody>,
    attachments: Option<Vec<GraphAttachment>>,
    flag: Option<GraphFollowupFlag>,
    /// Delta tombstone marker: deleted/moved messages arrive as
    /// `{"id": ..., "@removed": {...}}` with every other field absent. They
    /// must be skipped, not ingested as phantom empty messages.
    #[serde(rename = "@removed")]
    removed: Option<serde_json::Value>,
    #[serde(skip)]
    raw_mime: Option<Vec<u8>>,
}

#[derive(Debug, Deserialize)]
struct GraphFollowupFlag {
    #[serde(rename = "flagStatus")]
    flag_status: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GraphRecipientWrapper {
    #[serde(rename = "emailAddress")]
    email_address: GraphEmailAddress,
}

#[derive(Debug, Deserialize)]
struct GraphEmailAddress {
    name: Option<String>,
    address: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GraphInternetHeader {
    name: String,
    value: String,
}

#[derive(Debug, Deserialize)]
struct GraphItemBody {
    #[serde(rename = "contentType")]
    content_type: Option<String>,
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GraphAttachment {
    id: String,
    name: Option<String>,
    #[serde(rename = "contentType")]
    content_type: Option<String>,
    size: Option<i64>,
    #[serde(rename = "isInline")]
    is_inline: Option<bool>,
    #[serde(rename = "contentId")]
    content_id: Option<String>,
    #[serde(rename = "contentBytes")]
    content_bytes: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GraphIdTranslation {
    #[serde(rename = "sourceId")]
    source_id: String,
    #[serde(rename = "targetId")]
    target_id: Option<String>,
    #[serde(rename = "errorDetails")]
    error_details: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct MimeQuarantine {
    attempts: i64,
    last_error: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct OutlookCursorState {
    #[serde(default)]
    latest_received: String,
    #[serde(default)]
    mime_quarantine: BTreeMap<String, MimeQuarantine>,
    #[serde(default)]
    backfill_next_link: Option<String>,
    #[serde(default)]
    backfill_bootstrap_pending: bool,
    #[serde(default)]
    backfill_total_messages: usize,
    #[serde(default)]
    backfill_remaining_messages: usize,
    #[serde(default)]
    delta_link: Option<String>,
    /// Where a bounded or interrupted delta pass stopped.
    ///
    /// Set while an enumeration is still running, so the next run resumes at
    /// that page instead of starting the whole folder again. Cleared as soon as
    /// the pass reaches its `deltaLink`.
    #[serde(default)]
    delta_next_link: Option<String>,
    #[serde(default)]
    format_version: u8,
}

type ParsedOutlookCursor = cursor::ParsedCursor<OutlookCursorState>;

impl OutlookProvider {
    const CURSOR_FORMAT_VERSION: u8 = 1;

    pub fn from_account_config(config_json: &crate::accounts::AccountConfig) -> Self {
        Self {
            config_json: config_json.clone(),
            client: crate::utils::http::client(),
        }
    }

    async fn access_token(&self) -> crate::sync::errors::SyncResult<String> {
        access_token_from_account_config(OAuthProvider::Outlook, &self.config_json).await
    }

    /// Fetches a single message by its Graph id and maps it to an envelope,
    /// together with its parent folder's remote id (Graph messages do not carry
    /// mailbox ids in their envelope). Used by the targeted download behind
    /// server-search hits.
    pub async fn fetch_message_by_id(
        &self,
        remote_id: &str,
    ) -> crate::sync::errors::SyncResult<(MessageEnvelope, Option<String>)> {
        let token = self.access_token().await?;
        let mut message = Self::load_graph_message(&self.client, &token, remote_id).await?;
        let parent_folder_id = message.parent_folder_id.clone();
        Self::load_cryptographic_mime(&self.client, &token, &mut message).await?;
        Ok((map_graph_message(message), parent_folder_id))
    }

    async fn load_graph_message(
        client: &reqwest::Client,
        token: &str,
        remote_id: &str,
    ) -> crate::sync::errors::SyncResult<GraphMessage> {
        let mut url = Url::parse(&format!("{}/me/messages/{remote_id}", graph_api_base()))
            .map_err(|err| format!("invalid outlook message URL: {err}"))?;
        {
            let mut query = url.query_pairs_mut();
            query.append_pair(
                "$select",
                "id,conversationId,parentFolderId,subject,from,toRecipients,ccRecipients,bccRecipients,replyTo,receivedDateTime,sentDateTime,internetMessageId,internetMessageHeaders,isDraft,bodyPreview,isRead,flag,body",
            );
            query.append_pair(
                "$expand",
                "attachments($select=id,name,contentType,size,isInline,contentId)",
            );
        }

        let response = batch::send_throttled(
            || {
                client.get(url.clone()).bearer_auth(token).header(
                    "Prefer",
                    "IdType=\"ImmutableId\", outlook.body-content-type=\"html\"",
                )
            },
            "message fetch",
        )
        .await?;

        let value: serde_json::Value = response
            .json()
            .await
            .map_err(|err| format!("outlook message parse failed: {err}"))?;
        serde_json::from_value(value)
            .map_err(|err| format!("outlook message parse failed: {err}").into())
    }

    /// Fetch the original MIME message for explicit source inspection and
    /// cryptographic metadata repair.
    pub async fn fetch_raw_message(
        &self,
        remote_id: &str,
    ) -> crate::sync::errors::SyncResult<Vec<u8>> {
        let token = self.access_token().await?;
        let response = batch::send_throttled(
            || {
                self.client
                    .get(format!(
                        "{}/me/messages/{remote_id}/$value",
                        graph_api_base()
                    ))
                    .bearer_auth(&token)
                    .header("Prefer", "IdType=\"ImmutableId\"")
            },
            "message source fetch",
        )
        .await?;
        Ok(response
            .bytes()
            .await
            .map_err(|err| format!("outlook message source read failed: {err}"))?
            .to_vec())
    }

    async fn load_cryptographic_mime(
        client: &reqwest::Client,
        token: &str,
        message: &mut GraphMessage,
    ) -> crate::sync::errors::SyncResult<()> {
        if !outlook_message_needs_raw_crypto(message) {
            return Ok(());
        }

        let response = batch::send_throttled(
            || {
                client
                    .get(format!(
                        "{}/me/messages/{}/$value",
                        graph_api_base(),
                        message.id
                    ))
                    .bearer_auth(token)
                    .header("Prefer", "IdType=\"ImmutableId\"")
            },
            "signed message MIME fetch",
        )
        .await?;
        message.raw_mime = Some(
            response
                .bytes()
                .await
                .map_err(|err| format!("outlook cryptographic message MIME read failed: {err}"))?
                .to_vec(),
        );
        Ok(())
    }

    fn parse_cursor_state(since: &SyncCheckpoint) -> ParsedOutlookCursor {
        cursor::parse_state(since.cursor.as_deref(), |raw| {
            let mut state = OutlookCursorState::default();
            if let Some(value) = parse_outlook_cursor(Some(raw)) {
                state.latest_received = value;
            }
            state
        })
    }

    async fn load_message_page(
        client: &reqwest::Client,
        token: &str,
        folder: &str,
        since_date: Option<&str>,
        next_link: Option<&str>,
    ) -> crate::sync::errors::SyncResult<GraphListResponse<GraphMessage>> {
        let request = if let Some(next_link) = next_link.filter(|value| !value.trim().is_empty()) {
            client.get(next_link.to_string())
        } else {
            let mut url = Url::parse(&format!(
                "{}/me/mailFolders/{folder}/messages",
                graph_api_base()
            ))
            .map_err(|err| format!("invalid outlook messages URL: {err}"))?;
            {
                let mut query = url.query_pairs_mut();
                query.append_pair("$top", "100");
                query.append_pair(
                    "$select",
                    "id,conversationId,subject,from,toRecipients,ccRecipients,bccRecipients,replyTo,receivedDateTime,sentDateTime,internetMessageId,internetMessageHeaders,isDraft,bodyPreview,isRead,flag,body",
                );
                query.append_pair(
                    "$expand",
                    "attachments($select=id,name,contentType,size,isInline,contentId)",
                );
                query.append_pair("$orderby", "receivedDateTime DESC");
                if let Some(since_date) = since_date.filter(|value| !value.trim().is_empty()) {
                    query.append_pair("$filter", &format!("receivedDateTime ge {since_date}"));
                }
            }
            client.get(url)
        };

        let response = batch::send_throttled(
            || {
                request
                    .try_clone()
                    .expect("replayable Graph GET")
                    .bearer_auth(token)
                    .header(
                        "Prefer",
                        "IdType=\"ImmutableId\", outlook.body-content-type=\"html\"",
                    )
            },
            "messages list",
        )
        .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(crate::sync::errors::http_failure(
                "outlook",
                "messages list",
                status.as_u16(),
                &body,
            ));
        }

        Ok(response
            .json()
            .await
            .map_err(|err| format!("outlook messages parse failed: {err}"))?)
    }

    async fn folder_total_messages(
        client: &reqwest::Client,
        token: &str,
        folder: &str,
    ) -> crate::sync::errors::SyncResult<Option<usize>> {
        let response = batch::send_throttled(
            || {
                client
                    .get(format!("{}/me/mailFolders/{folder}", graph_api_base()))
                    .bearer_auth(token)
                    .query(&[("$select", "totalItemCount")])
            },
            "folder metadata",
        )
        .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(crate::sync::errors::http_failure(
                "outlook",
                "folder metadata",
                status.as_u16(),
                &body,
            ));
        }

        let payload: GraphMailFolder = response
            .json()
            .await
            .map_err(|err| format!("outlook folder metadata parse failed: {err}"))?;

        Ok(payload
            .total_item_count
            .filter(|value| *value >= 0)
            .and_then(|value| usize::try_from(value).ok()))
    }

    async fn replace_draft_attachments(
        client: &reqwest::Client,
        token: &str,
        draft_id: &str,
        attachments: &[crate::providers::traits::OutgoingAttachment],
    ) -> crate::sync::errors::SyncResult<()> {
        let mut next = Some(format!(
            "{}/me/messages/{draft_id}/attachments?$select=id,name,contentType,size,isInline,contentId,contentBytes&$top=100",
            graph_api_base()
        ));
        let mut existing = Vec::new();
        while let Some(url) = next.take() {
            let response = client
                .get(url)
                .bearer_auth(token)
                .header("Prefer", "IdType=\"ImmutableId\"")
                .send()
                .await
                .map_err(|err| format!("outlook draft attachments list failed: {err}"))?;
            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                return Err(crate::sync::errors::http_failure(
                    "outlook",
                    "draft attachments list",
                    status.as_u16(),
                    &body,
                ));
            }
            let page: GraphListResponse<GraphAttachment> = response
                .json()
                .await
                .map_err(|err| format!("outlook draft attachments parse failed: {err}"))?;
            existing.extend(page.value);
            next = page.next_link.filter(|value| !value.trim().is_empty());
        }

        let mut retained_ids = HashSet::new();
        let mut missing = Vec::new();
        for attachment in attachments {
            if let Some(matched) = existing.iter().find(|candidate| {
                !retained_ids.contains(&candidate.id)
                    && graph_attachment_matches(candidate, attachment)
            }) {
                retained_ids.insert(matched.id.clone());
            } else {
                missing.push(attachment);
            }
        }

        // Copy-on-write: upload every missing desired attachment before
        // removing obsolete ones. A failed upload leaves the old set intact.
        let mut created_ids = Vec::new();
        for attachment in missing {
            let response = match client
                .post(format!(
                    "{}/me/messages/{draft_id}/attachments",
                    graph_api_base()
                ))
                .bearer_auth(token)
                .header("Prefer", "IdType=\"ImmutableId\"")
                .json(&graph_attachment_json(attachment))
                .send()
                .await
            {
                Ok(response) => response,
                Err(err) => {
                    Self::rollback_created_attachments(client, token, draft_id, &created_ids).await;
                    return Err(format!("outlook draft attachment create failed: {err}").into());
                }
            };
            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                Self::rollback_created_attachments(client, token, draft_id, &created_ids).await;
                return Err(crate::sync::errors::http_failure(
                    "outlook",
                    "draft attachment create",
                    status.as_u16(),
                    &body,
                ));
            }
            let created: GraphAttachment = match response.json().await {
                Ok(created) => created,
                Err(err) => {
                    Self::rollback_created_attachments(client, token, draft_id, &created_ids).await;
                    return Err(format!("outlook draft attachment parse failed: {err}").into());
                }
            };
            created_ids.push(created.id);
        }

        for attachment in existing {
            if retained_ids.contains(&attachment.id) {
                continue;
            }
            Self::delete_draft_attachment(client, token, draft_id, &attachment.id).await?;
        }
        Ok(())
    }

    async fn find_draft_by_client_id(
        client: &reqwest::Client,
        token: &str,
        client_id: &str,
    ) -> crate::sync::errors::SyncResult<Option<String>> {
        let mut next = Some({
            let mut url = Url::parse(&format!(
                "{}/me/mailFolders/drafts/messages",
                graph_api_base()
            ))
            .map_err(|err| format!("invalid outlook draft lookup URL: {err}"))?;
            url.query_pairs_mut()
                .append_pair("$top", "100")
                .append_pair("$select", "id,internetMessageHeaders");
            url.to_string()
        });
        while let Some(url) = next.take() {
            let response = client
                .get(url)
                .bearer_auth(token)
                .header("Prefer", "IdType=\"ImmutableId\"")
                .send()
                .await
                .map_err(|err| format!("outlook draft lookup failed: {err}"))?;
            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                return Err(crate::sync::errors::http_failure(
                    "outlook",
                    "draft lookup",
                    status.as_u16(),
                    &body,
                ));
            }
            let page: GraphListResponse<GraphMessage> = response
                .json()
                .await
                .map_err(|err| format!("outlook draft lookup parse failed: {err}"))?;
            if let Some(message) = page.value.into_iter().find(|message| {
                message
                    .internet_message_headers
                    .as_deref()
                    .unwrap_or_default()
                    .iter()
                    .any(|header| {
                        header.name.eq_ignore_ascii_case("X-Satchel-Draft-ID")
                            && header.value == client_id
                    })
            }) {
                return Ok(Some(message.id));
            }
            next = page.next_link.filter(|value| !value.trim().is_empty());
        }
        Ok(None)
    }

    async fn delete_draft_attachment(
        client: &reqwest::Client,
        token: &str,
        draft_id: &str,
        attachment_id: &str,
    ) -> crate::sync::errors::SyncResult<()> {
        let response = client
            .delete(format!(
                "{}/me/messages/{draft_id}/attachments/{attachment_id}",
                graph_api_base()
            ))
            .bearer_auth(token)
            .header("Prefer", "IdType=\"ImmutableId\"")
            .send()
            .await
            .map_err(|err| format!("outlook draft attachment delete failed: {err}"))?;
        if !response.status().is_success() && response.status().as_u16() != 404 {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(crate::sync::errors::http_failure(
                "outlook",
                "draft attachment delete",
                status.as_u16(),
                &body,
            ));
        }
        Ok(())
    }

    async fn rollback_created_attachments(
        client: &reqwest::Client,
        token: &str,
        draft_id: &str,
        created_ids: &[String],
    ) {
        for attachment_id in created_ids {
            let _ = Self::delete_draft_attachment(client, token, draft_id, attachment_id).await;
        }
    }

    async fn load_delta_page(
        client: &reqwest::Client,
        token: &str,
        folder: &str,
        delta_link: Option<&str>,
    ) -> crate::sync::errors::SyncResult<GraphListResponse<GraphMessage>> {
        let request = if let Some(delta_link) = delta_link.filter(|value| !value.trim().is_empty())
        {
            client.get(delta_link.to_string())
        } else {
            let mut url = Url::parse(&format!(
                "{}/me/mailFolders/{folder}/messages/delta",
                graph_api_base()
            ))
            .map_err(|err| format!("invalid outlook delta URL: {err}"))?;
            {
                let mut query = url.query_pairs_mut();
                query.append_pair("$top", "100");
                query.append_pair(
                    "$select",
                    "id,conversationId,subject,from,toRecipients,ccRecipients,bccRecipients,replyTo,receivedDateTime,sentDateTime,internetMessageId,internetMessageHeaders,isDraft,bodyPreview,isRead,flag,body",
                );
                query.append_pair(
                    "$expand",
                    "attachments($select=id,name,contentType,size,isInline,contentId)",
                );
            }
            client.get(url)
        };

        let response = batch::send_throttled(
            || {
                request
                    .try_clone()
                    .expect("replayable Graph GET")
                    .bearer_auth(token)
                    .header(
                        "Prefer",
                        "IdType=\"ImmutableId\", outlook.body-content-type=\"html\"",
                    )
            },
            "delta",
        )
        .await?;
        Ok(response
            .json()
            .await
            .map_err(|err| format!("outlook delta parse failed: {err}"))?)
    }

    async fn hydrate_sync_message(
        client: &reqwest::Client,
        token: &str,
        mut message: GraphMessage,
        state: &mut OutlookCursorState,
        attempted: &mut HashSet<String>,
    ) -> Option<MessageEnvelope> {
        let id = message.id.clone();
        if !attempted.insert(id.clone())
            || state
                .mime_quarantine
                .get(&id)
                .is_some_and(|row| row.attempts >= crate::db::sync_quarantine::MAX_INGEST_ATTEMPTS)
        {
            return None;
        }
        match Self::load_cryptographic_mime(client, token, &mut message).await {
            Ok(()) => {
                state.mime_quarantine.remove(&id);
                Some(map_graph_message(message))
            }
            Err(error) => {
                let row = state.mime_quarantine.entry(id.clone()).or_default();
                row.attempts += 1;
                row.last_error = error.to_string();
                tracing::warn!(remote_id = %id, attempts = row.attempts, %error, "outlook MIME fetch quarantined");
                None
            }
        }
    }

    async fn backfill_one_page(
        client: &reqwest::Client,
        token: &str,
        folder: &str,
        state: &mut OutlookCursorState,
        attempted: &mut HashSet<String>,
    ) -> crate::sync::errors::SyncResult<(Vec<MessageEnvelope>, String)> {
        let payload = Self::load_message_page(
            client,
            token,
            folder,
            None,
            state.backfill_next_link.as_deref(),
        )
        .await?;
        let fetched_count = payload.value.len();
        let total = if state.backfill_total_messages > 0 {
            state.backfill_total_messages
        } else {
            Self::folder_total_messages(client, token, folder)
                .await
                .ok()
                .flatten()
                .unwrap_or(fetched_count)
        };
        let mut latest = state.latest_received.clone();
        let mut messages = Vec::new();
        for message in payload.value {
            if let Some(received) = message.received_date_time.as_deref() {
                if received > latest.as_str() {
                    latest = received.to_string();
                }
            }
            if let Some(message) =
                Self::hydrate_sync_message(client, token, message, state, attempted).await
            {
                messages.push(message);
            }
        }
        state.backfill_next_link = payload.next_link.filter(|value| !value.trim().is_empty());
        if state.backfill_next_link.is_some() {
            let remaining = if state.backfill_remaining_messages > 0 {
                state
                    .backfill_remaining_messages
                    .saturating_sub(fetched_count)
                    .max(1)
            } else {
                total.saturating_sub(fetched_count).max(1)
            };
            state.backfill_total_messages = total.max(fetched_count + remaining);
            state.backfill_remaining_messages = remaining;
        } else {
            state.backfill_total_messages = total.max(fetched_count);
            state.backfill_remaining_messages = 0;
        }
        Ok((messages, latest))
    }

    /// Whether this state is worth writing back.
    ///
    /// Any one of the four is: a timestamp, a delta token, a delta pass in
    /// progress, or a backfill in progress. A folder that is empty when it is
    /// first synced has no timestamp and no backfill token, and dropping its
    /// delta token made it re-bootstrap — another enumeration plus another
    /// `$count` — on every single run, forever.
    fn cursor_is_worth_keeping(state: &OutlookCursorState) -> bool {
        fn is_set(link: Option<&String>) -> bool {
            !link
                .map(|value| value.trim())
                .unwrap_or_default()
                .is_empty()
        }

        !state.latest_received.trim().is_empty()
            || !state.mime_quarantine.is_empty()
            || state.backfill_bootstrap_pending
            || is_set(state.delta_link.as_ref())
            || is_set(state.delta_next_link.as_ref())
            || is_set(state.backfill_next_link.as_ref())
    }

    fn finalize_cursor(state: OutlookCursorState) -> Option<String> {
        if !Self::cursor_is_worth_keeping(&state) {
            return None;
        }
        Some(
            serde_json::to_string(&state)
                .unwrap_or_else(|_| format!("ts:{}", state.latest_received)),
        )
    }
}

#[async_trait::async_trait]
impl MailProvider for OutlookProvider {
    async fn sync_folders(&self) -> crate::sync::errors::SyncResult<Vec<RemoteFolder>> {
        let token = self.access_token().await?;
        let client = self.client.clone();
        let mut folders = load_all_folders(&client, &token).await?;
        for (well_known, kind) in [
            ("inbox", "inbox"),
            ("sentitems", "sent"),
            ("drafts", "drafts"),
            ("deleteditems", "trash"),
            ("archive", "archive"),
            ("junkemail", "spam"),
        ] {
            let response = batch::send_throttled(
                || {
                    client
                        .get(format!("{}/me/mailFolders/{well_known}", graph_api_base()))
                        .query(&[("$select", "id")])
                        .bearer_auth(&token)
                },
                "well-known folder",
            )
            .await;
            let Ok(response) = response else { continue };
            if !response.status().is_success() {
                continue;
            }
            let Ok(value) = response.json::<serde_json::Value>().await else {
                continue;
            };
            let Some(id) = value.get("id").and_then(|id| id.as_str()) else {
                continue;
            };
            if let Some(folder) = folders.iter_mut().find(|folder| folder.id == id) {
                folder.kind = Some(kind.to_string());
            }
        }
        folders.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(folders)
    }

    async fn sync_messages(
        &self,
        folder: &str,
        since: SyncCheckpoint,
    ) -> crate::sync::errors::SyncResult<SyncBatch> {
        let token = self.access_token().await?;
        let client = self.client.clone();
        let parsed_cursor = Self::parse_cursor_state(&since);
        let mut cursor_state = parsed_cursor.state;
        let has_backfill_pending = cursor_state
            .backfill_next_link
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| !value.is_empty());
        let should_bootstrap_backfill = !has_backfill_pending
            && (!parsed_cursor.had_checkpoint
                || parsed_cursor.was_legacy
                || cursor_state.backfill_bootstrap_pending);
        cursor_state.backfill_bootstrap_pending = should_bootstrap_backfill;
        let mut latest_received = cursor_state.latest_received.clone();
        let mut messages = Vec::new();
        let mut removals = Vec::new();
        let mut snapshot_required = false;

        let mut attempted = HashSet::new();
        let retries = cursor_state
            .mime_quarantine
            .iter()
            .filter(|(_, row)| row.attempts < crate::db::sync_quarantine::MAX_INGEST_ATTEMPTS)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        // Establish a delta continuation even on the first run. Older mail is
        // then bounded by delta paging and the independent backfill page.
        {
            // A partial enumeration outranks the stored delta token: it is the
            // continuation of the pass that token belongs to.
            let mut delta_link = cursor_state
                .delta_next_link
                .clone()
                .filter(|value| !value.trim().is_empty())
                .or_else(|| cursor_state.delta_link.clone());
            let mut resynced = false;
            let mut pages_read = 0usize;
            // Re-established below by whichever exit the loop takes; a pass that
            // runs to its `deltaLink` must not leave a stale continuation behind.
            cursor_state.delta_next_link = None;

            loop {
                let payload =
                    match Self::load_delta_page(&client, &token, folder, delta_link.as_deref())
                        .await
                    {
                        Ok(payload) => payload,
                        // 410 Gone / resyncRequired: the delta token expired. Reset
                        // to a fresh full delta enumeration once instead of failing
                        // this folder on every future run with the same dead token.
                        Err(err)
                            if !resynced
                                && delta_link.is_some()
                                && (err.to_string().contains("status 410")
                                    || err.to_string().contains("resyncRequired")) =>
                        {
                            resynced = true;
                            snapshot_required = true;
                            delta_link = None;
                            cursor_state.delta_link = None;
                            continue;
                        }
                        // Keep what this run already read. Failing the whole call
                        // discards those pages *and* the cursor that points past
                        // them, which is what made a long enumeration restart from
                        // page 1 forever. Nothing read yet is still an error.
                        Err(err) if pages_read > 0 => {
                            tracing::warn!(
                                %folder,
                                %err,
                                pages_read,
                                "outlook delta page failed; earlier pages kept"
                            );
                            cursor_state.delta_next_link = delta_link;
                            break;
                        }
                        Err(err) => return Err(err),
                    };
                pages_read += 1;
                for message in payload.value {
                    // Skip delta tombstones — ingesting them would resurrect
                    // deleted messages as empty unread phantoms and clobber the
                    // stored real data.
                    if message.removed.is_some() {
                        cursor_state.mime_quarantine.remove(&message.id);
                        removals.push(RemoteMessageRemoval {
                            remote_id: message.id,
                            source_folder: Some(folder.to_string()),
                        });
                        continue;
                    }
                    if let Some(received) = message.received_date_time.as_deref() {
                        if received > latest_received.as_str() {
                            latest_received = received.to_string();
                        }
                    }
                    if let Some(message) = Self::hydrate_sync_message(
                        &client,
                        &token,
                        message,
                        &mut cursor_state,
                        &mut attempted,
                    )
                    .await
                    {
                        messages.push(message);
                    }
                }

                match payload.next_link.filter(|value| !value.trim().is_empty()) {
                    Some(next) => {
                        if pages_read >= max_delta_pages_per_run() {
                            cursor_state.delta_next_link = Some(next);
                            break;
                        }
                        delta_link = Some(next);
                    }
                    None => {
                        cursor_state.delta_link = payload
                            .delta_link
                            .filter(|value| !value.trim().is_empty())
                            .or(delta_link);
                        break;
                    }
                }
            }
            // The backfill state is deliberately left alone here. Clearing it —
            // as this branch used to, harmlessly, while the two passes were
            // mutually exclusive — would abandon an in-progress backfill after
            // a single page now that both run in the same call.
        }

        for id in retries {
            // A delta tombstone drops its id from the quarantine after the
            // retry list was taken; there is nothing left to retry.
            if attempted.contains(&id) || !cursor_state.mime_quarantine.contains_key(&id) {
                continue;
            }
            match Self::load_graph_message(&client, &token, &id).await {
                Ok(message) => {
                    if let Some(message) = Self::hydrate_sync_message(
                        &client,
                        &token,
                        message,
                        &mut cursor_state,
                        &mut attempted,
                    )
                    .await
                    {
                        messages.push(message);
                    }
                }
                Err(error) => {
                    if let Some(row) = cursor_state.mime_quarantine.get_mut(&id) {
                        row.attempts += 1;
                        row.last_error = error.to_string();
                    }
                }
            }
        }
        let mut backfill_advanced = !(has_backfill_pending || should_bootstrap_backfill);
        if !backfill_advanced {
            match Self::backfill_one_page(
                &client,
                &token,
                folder,
                &mut cursor_state,
                &mut attempted,
            )
            .await
            {
                Ok((page_messages, page_latest)) => {
                    messages.extend(page_messages);
                    latest_received = latest_received.max(page_latest);
                    backfill_advanced = true;
                    cursor_state.backfill_bootstrap_pending = false;
                }
                Err(error) => {
                    tracing::warn!(%folder, %error, "outlook backfill deferred; delta results kept")
                }
            }
        }

        cursor_state.latest_received = latest_received;
        if backfill_advanced {
            cursor_state.format_version = Self::CURSOR_FORMAT_VERSION;
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

    async fn sync_message_state_snapshot(
        &self,
        folder: &str,
    ) -> crate::sync::errors::SyncResult<Option<crate::providers::traits::RemoteMessageStateSnapshot>>
    {
        let token = self.access_token().await?;
        let client = self.client.clone();
        let mut next = Some({
            let mut url = Url::parse(&format!(
                "{}/me/mailFolders/{folder}/messages",
                graph_api_base()
            ))
            .map_err(|err| format!("invalid outlook state snapshot URL: {err}"))?;
            url.query_pairs_mut()
                .append_pair("$top", "100")
                .append_pair("$select", "id,isRead,flag");
            url.to_string()
        });
        let mut states = Vec::new();

        while let Some(url) = next.take() {
            let response = batch::send_throttled(
                || {
                    client
                        .get(url.clone())
                        .bearer_auth(&token)
                        .header("Prefer", "IdType=\"ImmutableId\"")
                },
                "state snapshot",
            )
            .await?;
            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                return Err(crate::sync::errors::http_failure(
                    "outlook",
                    "state snapshot",
                    status.as_u16(),
                    &body,
                ));
            }
            let page: GraphListResponse<GraphMessage> = response
                .json()
                .await
                .map_err(|err| format!("outlook state snapshot parse failed: {err}"))?;
            states.extend(
                page.value
                    .into_iter()
                    .filter(|message| message.removed.is_none())
                    .map(|message| crate::providers::traits::RemoteMessageState {
                        remote_id: message.id,
                        is_read: message.is_read,
                        is_starred: message.flag.and_then(|flag| {
                            flag.flag_status
                                .map(|status| status.eq_ignore_ascii_case("flagged"))
                        }),
                    }),
            );
            next = page.next_link.filter(|value| !value.trim().is_empty());
        }
        Ok(Some(crate::providers::traits::RemoteMessageStateSnapshot {
            states,
            // Graph immutable ids are account-wide and never renumber.
            remote_uidvalidity: None,
        }))
    }

    async fn translate_message_ids(
        &self,
        source_ids: &[String],
    ) -> crate::sync::errors::SyncResult<Vec<RemoteIdTranslation>> {
        let token = self.access_token().await?;
        let client = self.client.clone();
        let mut translations = Vec::with_capacity(source_ids.len());

        // Graph accepts at most 1000 ids per translateExchangeIds request.
        for chunk in source_ids.chunks(1000) {
            let response = batch::send_throttled(
                || {
                    client
                        .post(format!("{}/me/translateExchangeIds", graph_api_base()))
                        .bearer_auth(&token)
                        .json(&serde_json::json!({
                            "inputIds": chunk,
                            "sourceIdType": "restId",
                            "targetIdType": "restImmutableEntryId"
                        }))
                },
                "id translation",
            )
            .await?;
            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                return Err(crate::sync::errors::http_failure(
                    "outlook",
                    "id translation",
                    status.as_u16(),
                    &body,
                ));
            }
            let payload: GraphListResponse<GraphIdTranslation> = response
                .json()
                .await
                .map_err(|err| format!("outlook id translation parse failed: {err}"))?;
            for item in payload.value {
                translations.push(map_graph_id_translation(item)?);
            }
        }
        Ok(translations)
    }

    async fn send(&self, message: OutgoingMessage) -> crate::sync::errors::SyncResult<()> {
        // Always submit as MIME.
        //
        // The JSON message resource cannot carry In-Reply-To or References:
        // Graph only accepts x-prefixed names in internetMessageHeaders and
        // rejects the whole request with 400 otherwise, so every reply failed.
        // Bcc and Disposition-Notification-To had the same problem. Handing
        // Graph a complete RFC822 message sidesteps all three, and reuses the
        // builder that already sanitises headers against injection.
        let token = self.access_token().await?;
        let client = self.client.clone();

        let raw = match message
            .raw_rfc822_override
            .as_deref()
            .filter(|s| !s.trim().is_empty())
        {
            // Pre-built payload (S/MIME and similar) — pass through untouched.
            Some(raw) => raw.to_string(),
            // Bcc belongs in the headers here: /sendMail derives recipients
            // from them and strips Bcc before delivery, exactly like Gmail's
            // messages.send. This is NOT the raw-SMTP path.
            // from_address/from_name are resolved by the outbox before dispatch.
            None => crate::email::build::build_rfc822_with_bcc(&message, None, None)?,
        };

        let payload = base64::engine::general_purpose::STANDARD.encode(raw.as_bytes());
        // The client-wide deadline covers the upload too; sized to the message,
        // a few megabytes over a slow uplink can still finish.
        let deadline = crate::utils::http::upload_timeout(
            crate::utils::http::UPLOAD_BASE_TIMEOUT,
            payload.len(),
        );
        let upload = crate::utils::http::UploadProgress::default();
        let response = client
            .post(format!("{}/me/sendMail", graph_api_base()))
            .bearer_auth(token)
            .header("Content-Type", "text/plain")
            .timeout(deadline)
            .body(upload.body(payload))
            .send()
            .await
            .map_err(|err| {
                crate::sync::errors::http_submission_transport_failure(
                    "outlook",
                    err,
                    upload.fully_handed_over(),
                )
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(crate::sync::errors::http_failure(
                "outlook",
                "send",
                status.as_u16(),
                &body,
            ));
        }

        Ok(())
    }

    async fn set_flags(
        &self,
        messages: &[MessageRef],
        flags: FlagUpdate,
    ) -> crate::sync::errors::SyncResult<()> {
        let mut payload = serde_json::Map::new();
        if let Some(read) = flags.read {
            payload.insert("isRead".to_string(), serde_json::Value::Bool(read));
        }
        if let Some(starred) = flags.starred {
            payload.insert(
                "flag".to_string(),
                serde_json::json!({
                    "flagStatus": if starred { "flagged" } else { "notFlagged" }
                }),
            );
        }
        if payload.is_empty() || messages.is_empty() {
            return Ok(());
        }
        let payload = serde_json::Value::Object(payload);
        let operations: Vec<_> = messages
            .iter()
            .map(|message| batch::GraphOperation {
                remote_id: message.remote_id.clone(),
                method: "PATCH",
                url: format!("/me/messages/{}", message.remote_id),
                body: Some(payload.clone()),
                immutable_ids: false,
                missing_is_done: false,
            })
            .collect();
        let token = self.access_token().await?;
        batch::run(&self.client, &token, "set_flags", &operations).await
    }

    async fn move_messages(
        &self,
        messages: &[MessageRef],
        target: &str,
    ) -> crate::sync::errors::SyncResult<()> {
        let target = target.trim();
        if target.is_empty() {
            return Err("outlook move target is empty".into());
        }
        if messages.is_empty() {
            return Ok(());
        }
        let operations: Vec<_> = messages
            .iter()
            .map(|message| batch::GraphOperation {
                remote_id: message.remote_id.clone(),
                method: "POST",
                url: format!("/me/messages/{}/move", message.remote_id),
                body: Some(serde_json::json!({ "destinationId": target })),
                immutable_ids: true,
                missing_is_done: false,
            })
            .collect();
        let token = self.access_token().await?;
        batch::run(&self.client, &token, "move_messages", &operations).await
    }

    async fn delete_messages(
        &self,
        messages: &[MessageRef],
    ) -> crate::sync::errors::SyncResult<()> {
        if messages.is_empty() {
            return Ok(());
        }
        let operations: Vec<_> = messages
            .iter()
            .map(|message| batch::GraphOperation {
                remote_id: message.remote_id.clone(),
                method: "DELETE",
                url: format!("/me/messages/{}", message.remote_id),
                body: None,
                immutable_ids: false,
                missing_is_done: true,
            })
            .collect();
        let token = self.access_token().await?;
        batch::run(&self.client, &token, "delete_messages", &operations).await
    }

    async fn create_folder(&self, path: &str) -> crate::sync::errors::SyncResult<()> {
        let name = path.trim();
        if name.is_empty() {
            return Err("outlook folder name is empty".into());
        }

        let token = self.access_token().await?;
        let response = self
            .client
            .post(format!("{}/me/mailFolders", graph_api_base()))
            .bearer_auth(token)
            .json(&serde_json::json!({ "displayName": name }))
            .send()
            .await
            .map_err(|err| format!("outlook folder create failed: {err}"))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            let lower = body.to_ascii_lowercase();
            if status == reqwest::StatusCode::CONFLICT
                || lower.contains("already exists")
                || lower.contains("errorfolderexists")
            {
                return Ok(());
            }
            return Err(crate::sync::errors::http_failure(
                "outlook",
                "folder create",
                status.as_u16(),
                &body,
            ));
        }
        Ok(())
    }

    async fn upsert_draft(
        &self,
        message: OutgoingMessage,
        remote_draft_id: Option<&str>,
        _folder_hint: Option<&str>,
    ) -> crate::sync::errors::SyncResult<Option<String>> {
        let token = self.access_token().await?;
        let client = self.client.clone();
        let create_payload = build_graph_message_object(&message);
        let discovered_draft_id = if remote_draft_id.is_none() {
            if let Some(client_id) = message
                .message_id_header
                .as_deref()
                .filter(|value| !value.trim().is_empty())
            {
                Self::find_draft_by_client_id(&client, &token, client_id).await?
            } else {
                None
            }
        } else {
            None
        };
        let effective_draft_id = remote_draft_id
            .filter(|id| !id.trim().is_empty())
            .map(str::to_string)
            .or(discovered_draft_id);

        if let Some(existing) = effective_draft_id.as_deref() {
            let update_payload = build_graph_draft_update_object(&message);
            let response = client
                .patch(format!("{}/me/messages/{existing}", graph_api_base()))
                .bearer_auth(&token)
                .header("Prefer", "IdType=\"ImmutableId\"")
                .json(&update_payload)
                .send()
                .await
                .map_err(|err| format!("outlook draft update failed: {err}"))?;
            if response.status().is_success() {
                Self::replace_draft_attachments(&client, &token, existing, &message.attachments)
                    .await?;
                return Ok(Some(existing.to_string()));
            }
            if response.status().as_u16() != 404 {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                return Err(crate::sync::errors::http_failure(
                    "outlook",
                    "draft update",
                    status.as_u16(),
                    &body,
                ));
            }
        }

        let response = client
            .post(format!("{}/me/messages", graph_api_base()))
            .bearer_auth(&token)
            .header("Prefer", "IdType=\"ImmutableId\"")
            .json(&create_payload)
            .send()
            .await
            .map_err(|err| format!("outlook draft create failed: {err}"))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(crate::sync::errors::http_failure(
                "outlook",
                "draft create",
                status.as_u16(),
                &body,
            ));
        }
        let parsed: serde_json::Value = response
            .json()
            .await
            .map_err(|err| format!("outlook draft parse failed: {err}"))?;
        Ok(parsed
            .get("id")
            .and_then(|value| value.as_str())
            .map(str::to_string))
    }

    async fn delete_draft(&self, remote_draft_id: &str) -> crate::sync::errors::SyncResult<()> {
        let token = self.access_token().await?;
        let client = self.client.clone();
        let response = client
            .delete(format!(
                "{}/me/messages/{remote_draft_id}",
                graph_api_base()
            ))
            .bearer_auth(token)
            .send()
            .await
            .map_err(|err| format!("outlook draft delete failed: {err}"))?;
        if !response.status().is_success() && response.status().as_u16() != 404 {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(crate::sync::errors::http_failure(
                "outlook",
                "draft delete",
                status.as_u16(),
                &body,
            ));
        }
        Ok(())
    }

    async fn resolve_draft_id(
        &self,
        message_remote_id: &str,
        _folder_hint: Option<&str>,
    ) -> crate::sync::errors::SyncResult<Option<String>> {
        Ok(Some(message_remote_id.to_string()))
    }

    async fn fetch_attachment(
        &self,
        message: &MessageRef,
        remote_part_id: &str,
    ) -> crate::sync::errors::SyncResult<FetchedAttachment> {
        let token = self.access_token().await?;
        let client = self.client.clone();
        let response = batch::send_throttled(
            || {
                client
                    .get(format!(
                        "{}/me/messages/{}/attachments/{}",
                        graph_api_base(),
                        message.remote_id,
                        remote_part_id
                    ))
                    .bearer_auth(&token)
            },
            "attachment fetch",
        )
        .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(crate::sync::errors::http_failure(
                "outlook",
                "attachment fetch",
                status.as_u16(),
                &body,
            ));
        }

        let attachment: GraphAttachment = response
            .json()
            .await
            .map_err(|err| format!("outlook attachment parse failed: {err}"))?;
        let content = attachment
            .content_bytes
            .ok_or_else(|| "outlook attachment response had no contentBytes".to_string())?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(content.as_bytes())
            .map_err(|err| format!("outlook attachment decode failed: {err}"))?;
        Ok(FetchedAttachment {
            filename: attachment.name,
            content_type: crate::email::mime_type::normalize_mime_type(
                attachment.content_type.as_deref().unwrap_or_default(),
            ),
            bytes,
        })
    }
}

async fn load_all_folders(
    client: &reqwest::Client,
    token: &str,
) -> crate::sync::errors::SyncResult<Vec<RemoteFolder>> {
    let mut acc = Vec::new();
    let mut queue = vec![None::<String>];

    while let Some(parent_id) = queue.pop() {
        let mut next_link = None::<String>;

        loop {
            let request = if let Some(next_link) = next_link.as_deref() {
                client.get(next_link.to_string())
            } else {
                let endpoint = parent_id
                    .as_deref()
                    .map(|parent| {
                        format!("{}/me/mailFolders/{parent}/childFolders", graph_api_base())
                    })
                    .unwrap_or_else(|| format!("{}/me/mailFolders", graph_api_base()));
                client.get(endpoint).query(&[("$top", "100")])
            };

            let response = batch::send_throttled(
                || {
                    request
                        .try_clone()
                        .expect("replayable Graph GET")
                        .bearer_auth(token)
                },
                "folders request",
            )
            .await?;

            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                return Err(crate::sync::errors::http_failure(
                    "outlook",
                    "folders request",
                    status.as_u16(),
                    &body,
                ));
            }

            let payload: GraphListResponse<GraphMailFolder> = response
                .json()
                .await
                .map_err(|err| format!("outlook folders parse failed: {err}"))?;

            for folder in payload.value {
                acc.push(RemoteFolder {
                    id: folder.id.clone(),
                    name: folder.display_name.clone(),
                    kind: None,
                });

                if folder.child_folder_count.unwrap_or(0) > 0 {
                    queue.push(Some(folder.id));
                }
            }

            match payload.next_link.filter(|value| !value.trim().is_empty()) {
                Some(next) => next_link = Some(next),
                None => break,
            }
        }
    }

    Ok(acc)
}

fn parse_outlook_cursor(cursor: Option<&str>) -> Option<String> {
    cursor
        .and_then(|value| value.strip_prefix("ts:"))
        .map(str::to_string)
        .filter(|value| !value.trim().is_empty())
}

fn map_graph_id_translation(item: GraphIdTranslation) -> Result<RemoteIdTranslation, String> {
    let target_id = item
        .target_id
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            format!(
                "outlook id translation failed for {}: {}",
                item.source_id,
                item.error_details
                    .as_ref()
                    .map(serde_json::Value::to_string)
                    .unwrap_or_else(|| "missing target id".to_string())
            )
        })?;
    Ok(RemoteIdTranslation {
        source_id: item.source_id,
        target_id,
    })
}

fn outlook_message_needs_raw_crypto(message: &GraphMessage) -> bool {
    message
        .internet_message_headers
        .as_deref()
        .unwrap_or_default()
        .iter()
        .find(|header| header.name.eq_ignore_ascii_case("Content-Type"))
        .is_some_and(|header| {
            let value = header.value.to_ascii_lowercase();
            (value.contains("multipart/signed")
                && (value.contains("application/pgp-signature")
                    || value.contains("application/pkcs7-signature")
                    || value.contains("application/x-pkcs7-signature")))
                || (value.contains("multipart/encrypted")
                    && value.contains("application/pgp-encrypted"))
                || value.starts_with("application/pkcs7-mime")
                || value.starts_with("application/x-pkcs7-mime")
        })
}

fn map_graph_message(message: GraphMessage) -> MessageEnvelope {
    let raw_body = message
        .raw_mime
        .as_deref()
        .map(crate::providers::imap::parse_raw_message_body);
    let subject = message
        .subject
        .unwrap_or_else(|| "(no subject)".to_string());
    let from = message.from.map(|value| value.email_address);
    let from_address = from
        .as_ref()
        .and_then(|value| value.address.as_ref())
        .cloned()
        .unwrap_or_else(|| "unknown@outlook.local".to_string());
    let from_name = from
        .and_then(|value| value.name)
        .filter(|value| !value.trim().is_empty());

    let to_addresses = message
        .to_recipients
        .unwrap_or_default()
        .into_iter()
        .filter_map(|recipient| recipient.email_address.address)
        .collect::<Vec<_>>();
    let cc_addresses = message
        .cc_recipients
        .unwrap_or_default()
        .into_iter()
        .filter_map(|recipient| recipient.email_address.address)
        .collect::<Vec<_>>();
    let bcc_addresses = message
        .bcc_recipients
        .unwrap_or_default()
        .into_iter()
        .filter_map(|recipient| recipient.email_address.address)
        .collect::<Vec<_>>();
    let reply_to = message
        .reply_to
        .unwrap_or_default()
        .into_iter()
        .find_map(|recipient| recipient.email_address.address);
    let header_value = |name: &str| {
        message
            .internet_message_headers
            .as_deref()
            .unwrap_or_default()
            .iter()
            .find(|header| header.name.eq_ignore_ascii_case(name))
            .map(|header| header.value.clone())
    };
    let in_reply_to = header_value("In-Reply-To");
    let references = header_value("References")
        .map(|value| value.split_whitespace().map(str::to_string).collect())
        .unwrap_or_default();
    let list_unsubscribe = header_value("List-Unsubscribe");
    let list_unsubscribe_post = header_value("List-Unsubscribe-Post");
    let (auth_spf, auth_dkim, auth_dmarc) = header_value("Authentication-Results")
        .map(|value| crate::providers::traits::trusted_authentication_results(&value, "outlook"))
        .unwrap_or((None, None, None));
    let is_draft = message.is_draft.unwrap_or(false);
    let remote_id = message.id;

    let mut body_html = message
        .body
        .as_ref()
        .and_then(|body| body.content_type.as_deref())
        .filter(|kind| kind.eq_ignore_ascii_case("html"))
        .and_then(|_| message.body.as_ref().and_then(|body| body.content.clone()));
    let mut body_text = match message.body.as_ref().and_then(|body| body.content.clone()) {
        Some(content)
            if message
                .body
                .as_ref()
                .and_then(|body| body.content_type.as_deref())
                .map(|kind| kind.eq_ignore_ascii_case("text"))
                .unwrap_or(false) =>
        {
            Some(content)
        }
        Some(content) => Some(graph_html_body_to_text(&content)),
        None => message.body_preview.clone(),
    };
    if let Some(parsed) = raw_body.as_ref() {
        body_text = if parsed.trust.root_crypto {
            parsed.text.clone()
        } else {
            parsed.text.clone().or(body_text)
        };
        body_html = if parsed.trust.root_crypto {
            parsed.html.clone()
        } else {
            parsed.html.clone().or(body_html)
        };
    }

    let attachments = message
        .attachments
        .unwrap_or_default()
        .into_iter()
        .map(|att| RemoteAttachment {
            remote_part_id: att.id,
            filename: att.name,
            content_type: crate::email::mime_type::normalize_mime_type(
                att.content_type.as_deref().unwrap_or_default(),
            ),
            size: att.size,
            content_id: att.content_id,
            is_inline: att.is_inline.unwrap_or(false),
            content_base64: None,
        })
        .collect::<Vec<_>>();

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
                &header_value("Content-Type").unwrap_or_default(),
                &super::imap::ParsedBody {
                    text: body_text.clone(),
                    html: body_html.clone(),
                    ..Default::default()
                },
            )
        });
    trust.auth_verified = auth_spf.is_some() || auth_dkim.is_some() || auth_dmarc.is_some();
    let pgp_signed = raw_body.and_then(|parsed| parsed.pgp_signed);

    MessageEnvelope {
        remote_id: remote_id.clone(),
        remote_uidvalidity: None,
        remote_thread_id: message.conversation_id,
        original_source: None,
        mailbox_ids: Vec::new(),
        message_id_header: message.internet_message_id,
        in_reply_to,
        references,
        subject: subject.clone(),
        snippet: message.body_preview.unwrap_or_else(|| subject.clone()),
        from_address,
        from_name,
        to_addresses,
        cc_addresses,
        bcc_addresses,
        reply_to,
        remote_draft_id: is_draft.then_some(remote_id),
        received_at_rfc3339: message.received_date_time.clone(),
        date_rfc3339: message
            .received_date_time
            .or(message.sent_date_time)
            .unwrap_or_else(|| chrono::Utc::now().to_rfc3339()),
        body_text,
        body_html,
        is_read: message.is_read.unwrap_or(false),
        is_starred: message
            .flag
            .as_ref()
            .and_then(|flag| flag.flag_status.as_deref())
            .map(|status| status.eq_ignore_ascii_case("flagged"))
            .unwrap_or(false),
        body_calendar: None,
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

fn graph_html_body_to_text(input: &str) -> String {
    let without_tags = input
        .replace("<br>", "\n")
        .replace("<br/>", "\n")
        .replace("<br />", "\n")
        .replace("</p>", "\n")
        .replace("</div>", "\n");

    strip_html_tags(&without_tags)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn build_graph_message_object(message: &OutgoingMessage) -> serde_json::Value {
    let mut internet_headers = Vec::new();
    if let Some(client_id) = message
        .message_id_header
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        internet_headers.push(serde_json::json!({
            "name": "X-Satchel-Draft-ID",
            "value": client_id,
        }));
    }
    if let Some(in_reply_to) = &message.in_reply_to {
        internet_headers.push(serde_json::json!({
            "name": "In-Reply-To",
            "value": in_reply_to,
        }));
    }
    if !message.references.is_empty() {
        internet_headers.push(serde_json::json!({
            "name": "References",
            "value": message.references.join(" "),
        }));
    }
    if let Some(rr) = message
        .request_read_receipt_to
        .as_deref()
        .filter(|s| !s.trim().is_empty())
    {
        internet_headers.push(serde_json::json!({
            "name": "Disposition-Notification-To",
            "value": format!("<{rr}>"),
        }));
    }

    let (content_type, content) = match message
        .body_html
        .as_deref()
        .filter(|s| !s.trim().is_empty())
    {
        Some(html) => ("HTML", html.to_string()),
        None => ("Text", message.body.clone()),
    };

    let attachments_json = message
        .attachments
        .iter()
        .map(graph_attachment_json)
        .collect::<Vec<_>>();

    let mut message_obj = serde_json::json!({
        "subject": message.subject,
        "body": {
            "contentType": content_type,
            "content": content,
        },
        "toRecipients": named_recipients_json(message, &message.to),
        "ccRecipients": named_recipients_json(message, &message.cc),
        "bccRecipients": named_recipients_json(message, &message.bcc),
        "internetMessageHeaders": internet_headers,
    });
    if !attachments_json.is_empty() {
        message_obj["attachments"] = serde_json::Value::Array(attachments_json);
    }
    if let Some(reply_to) = message.reply_to.as_deref().filter(|s| !s.trim().is_empty()) {
        message_obj["replyTo"] = serde_json::Value::Array(recipients_json(&[reply_to.to_string()]));
    }
    if let Some(from) = message
        .from_address
        .as_deref()
        .filter(|s| !s.trim().is_empty())
    {
        message_obj["from"] = serde_json::json!({
            "emailAddress": {
                "address": from,
                "name": message.from_name.as_deref().unwrap_or(""),
            }
        });
    }
    message_obj
}

fn build_graph_draft_update_object(message: &OutgoingMessage) -> serde_json::Value {
    let mut payload = build_graph_message_object(message);
    if let Some(object) = payload.as_object_mut() {
        // Graph only accepts internetMessageHeaders while creating a message;
        // attachments have their own CRUD endpoint and `from` is read-only.
        object.remove("internetMessageHeaders");
        object.remove("attachments");
        object.remove("from");
    }
    payload
}

fn graph_attachment_json(
    attachment: &crate::providers::traits::OutgoingAttachment,
) -> serde_json::Value {
    serde_json::json!({
        "@odata.type": "#microsoft.graph.fileAttachment",
        "name": attachment.filename,
        "contentType": if attachment.mime_type.is_empty() { "application/octet-stream" } else { &attachment.mime_type },
        "contentBytes": attachment.content_base64,
        "isInline": attachment.inline,
        "contentId": attachment.content_id,
    })
}

fn graph_attachment_matches(
    existing: &GraphAttachment,
    desired: &crate::providers::traits::OutgoingAttachment,
) -> bool {
    let existing_type = existing.content_type.as_deref().unwrap_or_default();
    let desired_type = if desired.mime_type.is_empty() {
        "application/octet-stream"
    } else {
        desired.mime_type.as_str()
    };
    existing.name.as_deref() == Some(desired.filename.as_str())
        && existing_type.eq_ignore_ascii_case(desired_type)
        && existing.is_inline.unwrap_or(false) == desired.inline
        && existing.content_id.as_deref().unwrap_or_default()
            == desired.content_id.as_deref().unwrap_or_default()
        && existing
            .content_bytes
            .as_deref()
            .is_some_and(|bytes| decoded_base64_eq(bytes, &desired.content_base64))
}

fn decoded_base64_eq(left: &str, right: &str) -> bool {
    let decode = |value: &str| {
        base64::engine::general_purpose::STANDARD
            .decode(value.trim().as_bytes())
            .or_else(|_| {
                base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(value.trim().as_bytes())
            })
    };
    match (decode(left), decode(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => left.trim() == right.trim(),
    }
}

fn named_recipients_json(message: &OutgoingMessage, values: &[String]) -> Vec<serde_json::Value> {
    let mut recipients = recipients_json(values);
    for recipient in &mut recipients {
        let address = recipient["emailAddress"]["address"]
            .as_str()
            .unwrap_or_default()
            .to_ascii_lowercase();
        if let Some(name) = message.recipient_names.get(&address) {
            recipient["emailAddress"]["name"] = serde_json::Value::from(name.as_str());
        }
    }
    recipients
}

fn recipients_json(values: &[String]) -> Vec<serde_json::Value> {
    values
        .iter()
        .filter(|value| !value.trim().is_empty())
        .map(|value| {
            serde_json::json!({
                "emailAddress": {
                    "address": value,
                }
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        build_graph_draft_update_object, build_graph_message_object, graph_attachment_matches,
        map_graph_id_translation, map_graph_message, outlook_message_needs_raw_crypto,
        GraphAttachment, GraphIdTranslation, GraphMessage,
    };
    use crate::providers::traits::{OutgoingAttachment, OutgoingMessage};

    #[test]
    fn rejects_markup_in_attachment_mime_types() {
        let message: GraphMessage = serde_json::from_value(serde_json::json!({
            "id": "message", "attachments": [{ "id": "x", "name": "logo.png", "contentType": "image/png\"><link rel=preconnect href=https://u1.t.example>", "isInline": true }]
        })).unwrap();
        assert_eq!(
            super::map_graph_message(message).attachments[0].content_type,
            "application/octet-stream"
        );
    }

    #[test]
    fn a_delta_link_alone_is_worth_persisting() {
        let state = super::OutlookCursorState {
            delta_link: Some("https://graph.example/delta?token=abc".into()),
            ..Default::default()
        };

        let cursor = super::OutlookProvider::finalize_cursor(state).expect("cursor");
        let restored = serde_json::from_str::<super::OutlookCursorState>(&cursor).expect("state");

        assert_eq!(
            restored.delta_link.as_deref(),
            Some("https://graph.example/delta?token=abc")
        );
    }

    #[test]
    fn an_empty_state_is_still_dropped() {
        assert!(
            super::OutlookProvider::finalize_cursor(super::OutlookCursorState::default()).is_none()
        );
        assert!(
            super::OutlookProvider::finalize_cursor(super::OutlookCursorState {
                delta_link: Some("   ".into()),
                backfill_next_link: Some(String::new()),
                ..Default::default()
            })
            .is_none()
        );
    }

    #[test]
    fn any_single_piece_of_progress_keeps_the_cursor() {
        let states = [
            super::OutlookCursorState {
                latest_received: "2026-08-11T12:00:00Z".into(),
                ..Default::default()
            },
            super::OutlookCursorState {
                delta_link: Some("delta".into()),
                ..Default::default()
            },
            super::OutlookCursorState {
                delta_next_link: Some("next".into()),
                ..Default::default()
            },
            super::OutlookCursorState {
                backfill_next_link: Some("backfill".into()),
                ..Default::default()
            },
        ];

        for state in states {
            assert!(
                super::OutlookProvider::cursor_is_worth_keeping(&state),
                "{state:?}"
            );
        }
    }

    #[test]
    fn raw_mime_is_requested_for_legacy_smime_signatures() {
        let graph: GraphMessage = serde_json::from_value(serde_json::json!({
            "id": "signed-message",
            "internetMessageHeaders": [{
                "name": "Content-Type",
                "value": "multipart/signed; protocol=\"application/x-pkcs7-signature\"; boundary=smime"
            }]
        }))
        .unwrap();

        assert!(outlook_message_needs_raw_crypto(&graph));
    }

    #[test]
    fn raw_legacy_smime_is_mapped_to_the_verification_payload() {
        let mut graph: GraphMessage = serde_json::from_value(serde_json::json!({
            "id": "signed-message",
            "from": { "emailAddress": { "address": "sender@example.com" } },
            "internetMessageHeaders": [{
                "name": "Content-Type",
                "value": "multipart/signed; protocol=\"application/x-pkcs7-signature\"; boundary=smime"
            }]
        }))
        .unwrap();
        graph.raw_mime = Some(
            b"Content-Type: multipart/signed; protocol=\"application/x-pkcs7-signature\"; boundary=smime\r\n\r\n\
--smime\r\nContent-Type: text/plain\r\n\r\nSigned body\r\n\
--smime\r\nContent-Type: application/x-pkcs7-signature\r\n\r\nAQID\r\n--smime--\r\n"
                .to_vec(),
        );

        let mapped = map_graph_message(graph);

        assert!(mapped.smime_signed.is_some());
        assert!(mapped.pgp_signed.is_none());
    }

    #[test]
    fn outlook_mapping_preserves_stable_thread_and_reply_metadata() {
        let graph: GraphMessage = serde_json::from_value(serde_json::json!({
            "id": "immutable-message",
            "conversationId": "conversation",
            "subject": "Subject",
            "from": {"emailAddress": {"name": "Sender", "address": "sender@example.com"}},
            "toRecipients": [{"emailAddress": {"address": "me@example.com"}}],
            "bccRecipients": [{"emailAddress": {"address": "hidden@example.com"}}],
            "replyTo": [{"emailAddress": {"address": "replies@example.net"}}],
            "sentDateTime": "2026-07-16T10:00:00Z",
            "internetMessageId": "<message@example.com>",
            "internetMessageHeaders": [
                {"name": "In-Reply-To", "value": "<parent@example.com>"},
                {"name": "References", "value": "<root@example.com> <parent@example.com>"}
            ],
            "isDraft": true,
            "isRead": true,
            "bodyPreview": "Body"
        }))
        .expect("graph message");
        let mapped = map_graph_message(graph);
        assert_eq!(
            mapped.message_id_header.as_deref(),
            Some("<message@example.com>")
        );
        assert_eq!(mapped.in_reply_to.as_deref(), Some("<parent@example.com>"));
        assert_eq!(mapped.references.len(), 2);
        assert_eq!(mapped.reply_to.as_deref(), Some("replies@example.net"));
        assert_eq!(mapped.bcc_addresses, vec!["hidden@example.com"]);
        assert_eq!(mapped.remote_draft_id.as_deref(), Some("immutable-message"));
    }

    #[test]
    fn outlook_draft_patch_uses_only_updateable_fields() {
        let message = OutgoingMessage {
            from_address: Some("me@example.com".into()),
            subject: "Updated".into(),
            in_reply_to: Some("<parent@example.com>".into()),
            message_id_header: Some("<draft-local@satchel.draft>".into()),
            attachments: vec![OutgoingAttachment {
                filename: "note.txt".into(),
                mime_type: "text/plain".into(),
                content_base64: "bm90ZQ==".into(),
                content_id: None,
                inline: false,
            }],
            ..OutgoingMessage::default()
        };

        let create = build_graph_message_object(&message);
        assert!(create.get("internetMessageHeaders").is_some());
        assert!(create["internetMessageHeaders"]
            .as_array()
            .expect("headers")
            .iter()
            .any(|header| header["name"] == "X-Satchel-Draft-ID"
                && header["value"] == "<draft-local@satchel.draft>"));
        assert!(create.get("attachments").is_some());
        let update = build_graph_draft_update_object(&message);
        assert!(update.get("internetMessageHeaders").is_none());
        assert!(update.get("attachments").is_none());
        assert!(update.get("from").is_none());
        assert_eq!(update["subject"], "Updated");

        let existing: GraphAttachment = serde_json::from_value(serde_json::json!({
            "id": "attachment",
            "name": "note.txt",
            "contentType": "text/plain",
            "isInline": false,
            "contentBytes": "bm90ZQ=="
        }))
        .expect("attachment");
        assert!(graph_attachment_matches(&existing, &message.attachments[0]));
    }

    #[test]
    fn outlook_translation_error_is_not_treated_as_remote_deletion() {
        let item: GraphIdTranslation = serde_json::from_value(serde_json::json!({
            "sourceId": "mutable-id",
            "errorDetails": {"errorCode": "ErrorInvalidIdMalformed"}
        }))
        .expect("translation result");
        let error = map_graph_id_translation(item).expect_err("conversion must fail");
        assert!(error.contains("mutable-id"));
        assert!(error.contains("ErrorInvalidIdMalformed"));
    }

    fn reply_message() -> OutgoingMessage {
        OutgoingMessage {
            from_address: Some("me@example.com".into()),
            from_name: Some("Me".into()),
            to: vec!["her@example.com".into()],
            cc: vec!["cc@example.com".into()],
            bcc: vec!["bcc@example.com".into()],
            subject: "Re: Angebot".into(),
            body: "Passt so.".into(),
            in_reply_to: Some("<original@example.com>".into()),
            references: vec!["<root@example.com>".into(), "<original@example.com>".into()],
            request_read_receipt_to: Some("me@example.com".into()),
            ..Default::default()
        }
    }

    /// Graph only accepts x-prefixed names in internetMessageHeaders, so the
    /// JSON path rejected every reply with 400 and dropped Bcc silently.
    /// Submitting MIME is what makes threading, Bcc and read receipts work.
    #[test]
    fn mime_submission_carries_the_headers_graph_rejects_as_json() {
        let raw = crate::email::build::build_rfc822_with_bcc(&reply_message(), None, None).unwrap();

        assert!(raw.contains("In-Reply-To: <original@example.com>"));
        assert!(raw.contains("References: <root@example.com> <original@example.com>"));
        assert!(raw.contains("Disposition-Notification-To: <me@example.com>"));
        assert!(raw.contains("Bcc: bcc@example.com"));
        assert!(raw.contains(r#"From: "Me" <me@example.com>"#));
    }

    #[test]
    fn mime_submission_is_base64_encoded_for_graph() {
        use base64::Engine as _;
        let raw = crate::email::build::build_rfc822_with_bcc(&reply_message(), None, None).unwrap();
        let encoded = base64::engine::general_purpose::STANDARD.encode(raw.as_bytes());

        let decoded = base64::engine::general_purpose::STANDARD
            .decode(encoded.as_bytes())
            .expect("round trips");
        assert_eq!(String::from_utf8(decoded).unwrap(), raw);
    }

    #[test]
    fn a_prebuilt_payload_is_passed_through_untouched() {
        // S/MIME hands us a finished message; rebuilding it would break the
        // signature.
        let mut message = reply_message();
        message.raw_rfc822_override = Some("From: me@example.com\r\n\r\nsigned".into());

        let raw = match message
            .raw_rfc822_override
            .as_deref()
            .filter(|s| !s.trim().is_empty())
        {
            Some(raw) => raw.to_string(),
            None => crate::email::build::build_rfc822_with_bcc(&message, None, None).unwrap(),
        };
        assert_eq!(raw, "From: me@example.com\r\n\r\nsigned");
    }
}
