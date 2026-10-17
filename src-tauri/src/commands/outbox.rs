use serde::Serialize;
use sqlx::SqlitePool;
use tauri::State;

use crate::{
    commands::error::{CommandError, CommandResult},
    sync::queue::QueuedAction,
    AppState,
};

/// What a queued attachment costs, without its bytes. The list is a projection;
/// carrying every base64 payload through it would be megabytes per refresh.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/outbox/"))]
pub struct OutboxAttachmentDto {
    pub filename: String,
    pub mime_type: String,
    pub size: i64,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/outbox/"))]
pub struct OutboxItemDto {
    pub id: String,
    pub account_id: String,
    pub action_type: String,
    pub status: String,
    pub retry_count: u32,
    pub created_at: String,
    pub updated_at: String,
    pub last_error: Option<String>,
    pub next_retry_at: Option<String>,
    pub scheduled_for: Option<String>,
    pub subject: Option<String>,
    pub recipients: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    /// Enough of the payload to rebuild the message in the composer. Without it
    /// a mail that never went out leaves nothing but its subject.
    pub body: Option<String>,
    pub body_html: Option<String>,
    pub attachments: Vec<OutboxAttachmentDto>,
    /// Set once the provider has seen bytes of this attempt.
    pub submission_started_at: Option<String>,
}

fn payload_strings(input: Option<&serde_json::Value>, field: &str) -> Vec<String> {
    input
        .and_then(|input| input.get(field))
        .and_then(|value| value.as_array())
        .map(|values| {
            values
                .iter()
                .filter_map(|value| value.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn payload_text(input: Option<&serde_json::Value>, field: &str) -> Option<String> {
    input
        .and_then(|input| input.get(field))
        .and_then(|value| value.as_str())
        .map(str::to_string)
}

fn payload_attachments(input: Option<&serde_json::Value>) -> Vec<OutboxAttachmentDto> {
    input
        .and_then(|input| input.get("attachments"))
        .and_then(|value| value.as_array())
        .map(|values| {
            values
                .iter()
                .map(|attachment| OutboxAttachmentDto {
                    filename: payload_text(Some(attachment), "filename").unwrap_or_default(),
                    mime_type: payload_text(Some(attachment), "mime_type").unwrap_or_default(),
                    // base64 expands by 4/3; report what the file weighs.
                    size: payload_text(Some(attachment), "content_base64")
                        .map(|encoded| (encoded.len() / 4 * 3) as i64)
                        .unwrap_or(0),
                })
                .collect()
        })
        .unwrap_or_default()
}

impl From<QueuedAction> for OutboxItemDto {
    fn from(action: QueuedAction) -> Self {
        let input = action.payload.get("input");
        Self {
            id: action.id,
            account_id: action.account_id,
            action_type: action.action_type,
            status: action.status,
            retry_count: action.retry_count,
            created_at: action.created_at,
            updated_at: action.updated_at,
            last_error: action.last_error,
            next_retry_at: action.next_retry_at,
            scheduled_for: action.scheduled_for,
            subject: payload_text(input, "subject"),
            recipients: payload_strings(input, "to"),
            cc: payload_strings(input, "cc"),
            bcc: payload_strings(input, "bcc"),
            body: payload_text(input, "body"),
            body_html: payload_text(input, "body_html"),
            attachments: payload_attachments(input),
            submission_started_at: action.submission_started_at,
        }
    }
}

#[tauri::command]
pub async fn list_outbox(
    state: State<'_, AppState>,
    pool: State<'_, crate::db::ReadPool>,
) -> CommandResult<Vec<OutboxItemDto>> {
    let items = state.queue.list_mail_outbox(pool.inner().as_ref()).await?;
    Ok(items.into_iter().map(OutboxItemDto::from).collect())
}

#[tauri::command]
pub async fn list_failed_sync_actions(
    state: State<'_, AppState>,
    pool: State<'_, SqlitePool>,
) -> CommandResult<Vec<OutboxItemDto>> {
    let mut items = Vec::new();
    for action in state.queue.list_failed_moves(pool.inner()).await? {
        let thread_id = satchel_core::sync::remote_mutations::remote_move_thread_id_from_json(
            &action.payload.to_string(),
        );
        let mut item = OutboxItemDto::from(action);
        if let Some(thread_id) = thread_id {
            item.subject = sqlx::query_scalar("SELECT subject FROM threads WHERE id = ?")
                .bind(thread_id)
                .fetch_optional(pool.inner())
                .await
                .map_err(CommandError::unexpected)?
                .flatten();
        }
        items.push(item);
    }
    Ok(items)
}

#[tauri::command]
pub async fn discard_failed_sync_action(
    pool: State<'_, SqlitePool>,
    state: State<'_, AppState>,
    id: String,
) -> CommandResult<()> {
    if !satchel_core::mail_actions::discard_failed_move(pool.inner(), &id).await? {
        return Err(CommandError::new(
            "backend.errors.outboxActionCannotBeDiscarded",
        ));
    }
    state
        .events
        .publish(crate::events::bus::AppEvent::SyncCompleted);
    Ok(())
}

#[tauri::command]
pub async fn retry_outbox_action(
    state: State<'_, AppState>,
    pool: State<'_, SqlitePool>,
    id: String,
) -> CommandResult<()> {
    if state.queue.requeue_failed_move(pool.inner(), &id).await? {
        return Ok(());
    }
    Ok(state.queue.requeue_mail(pool.inner(), &id).await?)
}

#[tauri::command]
pub async fn retry_failed_sync_actions(
    state: State<'_, AppState>,
    pool: State<'_, SqlitePool>,
) -> CommandResult<u64> {
    Ok(state
        .queue
        .requeue_failed_remote_mutations(pool.inner())
        .await?)
}

#[tauri::command]
pub async fn undo_outbox_send(
    state: State<'_, AppState>,
    pool: State<'_, SqlitePool>,
    id: String,
) -> CommandResult<crate::commands::drafts::DraftDto> {
    satchel_core::drafts::restore_queued_draft(pool.inner(), &state.queue, &id)
        .await
        .map_err(CommandError::from)
}

#[tauri::command]
pub async fn cancel_outbox_action(
    state: State<'_, AppState>,
    pool: State<'_, SqlitePool>,
    id: String,
) -> CommandResult<()> {
    // No read-then-delete: the worker can claim the row in between, send the
    // mail, and then find nothing left to record it with.
    if state.queue.discard_mail_action(pool.inner(), &id).await? {
        return Ok(());
    }
    Err(CommandError::new(
        "backend.errors.outboxActionCannotBeDiscarded",
    ))
}

#[cfg(test)]
mod tests {
    use super::OutboxItemDto;
    use crate::sync::queue::{QueuedAction, DISCARDABLE_MAIL_STATUSES};

    #[test]
    fn carries_enough_of_the_payload_to_rebuild_the_message() {
        let action = QueuedAction::new(
            "acc-1".into(),
            "send_message",
            serde_json::json!({
                "input": {
                    "subject": "Angebot",
                    "to": ["a@x.de"],
                    "cc": ["c@x.de"],
                    "bcc": ["b@x.de"],
                    "body": "Guten Tag,",
                    "body_html": "<p>Guten Tag,</p>",
                    "attachments": [
                        { "filename": "offer.pdf", "mime_type": "application/pdf",
                          "content_base64": "AAAAAAAA" }
                    ]
                }
            }),
        );

        let dto = OutboxItemDto::from(action);

        assert_eq!(dto.body.as_deref(), Some("Guten Tag,"));
        assert_eq!(dto.body_html.as_deref(), Some("<p>Guten Tag,</p>"));
        assert_eq!(dto.bcc, vec!["b@x.de".to_string()]);
        assert_eq!(dto.cc, vec!["c@x.de".to_string()]);
        assert_eq!(dto.attachments.len(), 1);
        assert_eq!(dto.attachments[0].filename, "offer.pdf");
        assert_eq!(dto.attachments[0].size, 6);
    }

    #[test]
    fn stays_empty_when_the_payload_has_no_message() {
        let dto = OutboxItemDto::from(QueuedAction::new(
            "acc-1".into(),
            "remote_mutation",
            serde_json::json!({ "kind": "blocked_move" }),
        ));

        assert!(dto.body.is_none());
        assert!(dto.recipients.is_empty());
        assert!(dto.attachments.is_empty());
    }

    /// The list the DELETE in `Queue::discard_mail_action` carries in its
    /// WHERE clause. `sending` is the one that matters: the worker owns that
    /// row, and removing it would delete the only record of a mail on its way.
    #[test]
    fn only_unsent_mail_can_be_discarded() {
        for status in [
            "pending",
            "scheduled",
            "failed",
            "delivered",
            "delivery_unknown",
        ] {
            assert!(DISCARDABLE_MAIL_STATUSES.contains(&status), "{status}");
        }
        assert!(!DISCARDABLE_MAIL_STATUSES.contains(&"sending"));
    }
}
