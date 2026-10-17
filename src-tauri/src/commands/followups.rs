use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::commands::error::{CommandError, CommandResult};
use satchel_core::db::followups::{
    FollowUpError, FollowUpRecord, FollowUpRepository, FollowUpUpsertRequest,
};

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/followups/")
)]
pub struct FollowUpDto {
    pub id: String,
    pub account_id: String,
    pub thread_id: String,
    pub message_id: Option<String>,
    pub due_at: String,
    pub status: String,
    pub note: Option<String>,
    pub subject: Option<String>,
    pub snippet: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub resolved_at: Option<String>,
}

impl From<FollowUpRecord> for FollowUpDto {
    fn from(record: FollowUpRecord) -> Self {
        Self {
            id: record.id,
            account_id: record.account_id,
            thread_id: record.thread_id,
            message_id: record.message_id,
            due_at: record.due_at,
            status: record.status,
            note: record.note,
            subject: record.subject,
            snippet: record.snippet,
            created_at: record.created_at,
            updated_at: record.updated_at,
            resolved_at: record.resolved_at,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/followups/", optional_fields = nullable))]
pub struct FollowUpInput {
    #[serde(default)]
    pub id: Option<String>,
    pub thread_id: String,
    pub due_at: String,
    #[serde(default)]
    pub note: Option<String>,
}

impl From<FollowUpInput> for FollowUpUpsertRequest {
    fn from(input: FollowUpInput) -> Self {
        Self {
            id: input.id,
            thread_id: input.thread_id,
            due_at: input.due_at,
            note: input.note,
        }
    }
}

pub(crate) fn follow_up_error_key(error: FollowUpError) -> String {
    match error {
        FollowUpError::InvalidDueAt => "backend.errors.followUpTimestampInvalid",
        FollowUpError::ThreadNotFound => "backend.errors.threadNotFound",
        FollowUpError::Persistence(error) => {
            tracing::error!(error = ?error, "follow-up persistence failed");
            "backend.errors.followUpPersistenceFailed"
        }
    }
    .to_owned()
}

impl From<FollowUpError> for CommandError {
    fn from(error: FollowUpError) -> Self {
        Self::new(follow_up_error_key(error))
    }
}

#[tauri::command]
pub async fn list_follow_ups(
    pool: tauri::State<'_, crate::db::ReadPool>,
    status: Option<String>,
) -> CommandResult<Vec<FollowUpDto>> {
    FollowUpRepository::new(pool.inner().as_ref())
        .list(status.as_deref())
        .await
        .map(|rows| rows.into_iter().map(Into::into).collect())
        .map_err(Into::into)
}

#[tauri::command]
pub async fn upsert_follow_up(
    pool: tauri::State<'_, SqlitePool>,
    input: FollowUpInput,
) -> CommandResult<FollowUpDto> {
    FollowUpRepository::new(pool.inner())
        .upsert(input.into())
        .await
        .map(Into::into)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn cancel_follow_up(pool: tauri::State<'_, SqlitePool>, id: String) -> CommandResult<()> {
    FollowUpRepository::new(pool.inner())
        .cancel(&id)
        .await
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_the_complete_core_projection_to_the_ipc_dto() {
        let record = FollowUpRecord {
            id: "follow-1".into(),
            account_id: "account-1".into(),
            thread_id: "thread-1".into(),
            message_id: Some("message-1".into()),
            due_at: "2026-08-31T12:00:00+00:00".into(),
            status: "pending".into(),
            note: Some("check back".into()),
            subject: Some("Subject".into()),
            snippet: Some("Snippet".into()),
            created_at: "2026-08-30T12:00:00+00:00".into(),
            updated_at: "2026-08-30T12:00:00+00:00".into(),
            resolved_at: None,
        };

        let dto = FollowUpDto::from(record.clone());
        assert_eq!(dto.id, record.id);
        assert_eq!(dto.account_id, record.account_id);
        assert_eq!(dto.thread_id, record.thread_id);
        assert_eq!(dto.message_id, record.message_id);
        assert_eq!(dto.due_at, record.due_at);
        assert_eq!(dto.status, record.status);
        assert_eq!(dto.note, record.note);
        assert_eq!(dto.subject, record.subject);
        assert_eq!(dto.snippet, record.snippet);
        assert_eq!(dto.created_at, record.created_at);
        assert_eq!(dto.updated_at, record.updated_at);
        assert_eq!(dto.resolved_at, record.resolved_at);
    }

    #[test]
    fn redacts_follow_up_persistence_details_at_the_ipc_boundary() {
        let detail = "near UPDATE follow_up_reminders: database is locked";
        let error = CommandError::from(FollowUpError::Persistence(sqlx::Error::Protocol(
            detail.into(),
        )));
        let serialized = serde_json::to_value(error).expect("command error JSON");

        assert_eq!(
            serialized,
            serde_json::json!({ "key": "backend.errors.followUpPersistenceFailed" })
        );
        assert!(!serialized.to_string().contains(detail));
    }

    #[test]
    fn maps_invalid_timestamp_and_missing_thread_to_stable_keys() {
        assert_eq!(
            CommandError::from(FollowUpError::InvalidDueAt).key,
            "backend.errors.followUpTimestampInvalid"
        );
        assert_eq!(
            CommandError::from(FollowUpError::ThreadNotFound).key,
            "backend.errors.threadNotFound"
        );
    }
}
