use base64::Engine;
use serde::{Deserialize, Serialize};

use crate::commands::error::{CommandError, CommandResult};
use satchel_core::db::topics::{
    AddTopicItemsRequest, RemoveTopicItemRequest, TopicAddedBy, TopicError, TopicFileContentRecord,
    TopicFileRecord, TopicFileWriteRequest, TopicMembershipReceipt, TopicMembershipRecord,
    TopicMembershipsQuery, TopicNoteRecord, TopicNoteWriteRequest, TopicRecord, TopicRepository,
    TopicStatus, TopicTimelineItemRecord, TopicUpsertRequest,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/topics/"))]
pub struct TopicMembershipReceiptDto {
    pub item_id: String,
    pub revision: String,
}

impl From<TopicMembershipReceipt> for TopicMembershipReceiptDto {
    fn from(value: TopicMembershipReceipt) -> Self {
        Self {
            item_id: value.item_id,
            revision: value.revision,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/topics/"))]
pub struct UndoTopicMembershipsInput {
    pub topic_id: String,
    pub item_type: String,
    pub receipts: Vec<TopicMembershipReceiptDto>,
    pub restore: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/topics/", optional_fields = nullable))]
pub struct TopicInput {
    pub id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub due_date: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/topics/"))]
pub struct TopicDto {
    pub id: String,
    pub name: String,
    pub description: String,
    pub color: Option<String>,
    pub icon: Option<String>,
    pub status: String,
    pub due_date: Option<String>,
    pub item_count: i64,
    pub note_count: i64,
    pub unread_count: i64,
    pub last_sender_name: Option<String>,
    pub last_sender_email: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl From<TopicRecord> for TopicDto {
    fn from(record: TopicRecord) -> Self {
        Self {
            id: record.id,
            name: record.name,
            description: record.description,
            color: record.color,
            icon: record.icon,
            status: record.status.as_str().to_owned(),
            due_date: record.due_date,
            item_count: record.item_count,
            note_count: record.note_count,
            unread_count: record.unread_count,
            last_sender_name: record.last_sender_name,
            last_sender_email: record.last_sender_email,
            created_at: record.created_at,
            updated_at: record.updated_at,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/topics/", optional_fields = nullable))]
pub struct AddTopicItemsInput {
    pub topic_id: String,
    pub item_type: String,
    pub item_ids: Vec<String>,
    #[serde(default)]
    pub added_by: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/topics/", optional_fields = nullable))]
pub struct RemoveTopicItemInput {
    pub topic_id: String,
    pub item_type: String,
    pub item_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/topics/", optional_fields = nullable))]
pub struct TopicMembershipsInput {
    pub item_type: String,
    pub item_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/topics/"))]
pub struct TopicMembershipDto {
    pub topic_id: String,
    pub item_id: String,
}

impl From<TopicMembershipRecord> for TopicMembershipDto {
    fn from(record: TopicMembershipRecord) -> Self {
        Self {
            topic_id: record.topic_id,
            item_id: record.item_id,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/topics/", optional_fields = nullable))]
pub struct TopicNoteInput {
    pub id: Option<String>,
    pub topic_id: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/topics/"))]
pub struct TopicNoteDto {
    pub id: String,
    pub topic_id: String,
    pub content: String,
    pub created_at: String,
    pub updated_at: String,
}

impl From<TopicNoteRecord> for TopicNoteDto {
    fn from(record: TopicNoteRecord) -> Self {
        Self {
            id: record.id,
            topic_id: record.topic_id,
            content: record.content,
            created_at: record.created_at,
            updated_at: record.updated_at,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/topics/"))]
pub struct TopicTimelineItemDto {
    pub item_type: String,
    pub item_id: String,
    pub thread_id: Option<String>,
    pub title: Option<String>,
    pub snippet: Option<String>,
    pub content: Option<String>,
    pub from_address: Option<String>,
    pub sort_date: String,
    pub added_at: String,
    pub added_by: String,
    pub message_count: i64,
    pub unread_count: i64,
}

impl From<TopicTimelineItemRecord> for TopicTimelineItemDto {
    fn from(record: TopicTimelineItemRecord) -> Self {
        Self {
            item_type: record.item_type.as_str().to_owned(),
            item_id: record.item_id,
            thread_id: record.thread_id,
            title: record.title,
            snippet: record.snippet,
            content: record.content,
            from_address: record.from_address,
            sort_date: record.sort_date,
            added_at: record.added_at,
            added_by: record.added_by.as_str().to_owned(),
            message_count: record.message_count,
            unread_count: record.unread_count,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/topics/", optional_fields = nullable))]
pub struct TopicFileInput {
    pub topic_id: String,
    pub filename: String,
    #[serde(default)]
    pub mime_type: Option<String>,
    pub content_base64: String,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/topics/"))]
pub struct TopicFileDto {
    pub id: String,
    pub topic_id: String,
    pub filename: String,
    pub mime_type: String,
    pub size: i64,
    pub created_at: String,
}

impl From<TopicFileRecord> for TopicFileDto {
    fn from(record: TopicFileRecord) -> Self {
        Self {
            id: record.id,
            topic_id: record.topic_id,
            filename: record.filename,
            mime_type: record.mime_type,
            size: record.size,
            created_at: record.created_at,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/topics/"))]
pub struct TopicFileDownload {
    pub filename: String,
    pub mime_type: String,
    pub data_base64: String,
}

impl From<TopicFileContentRecord> for TopicFileDownload {
    fn from(record: TopicFileContentRecord) -> Self {
        Self {
            filename: record.filename,
            mime_type: record.mime_type,
            data_base64: base64::engine::general_purpose::STANDARD.encode(record.content),
        }
    }
}

impl From<TopicError> for CommandError {
    fn from(error: TopicError) -> Self {
        match error {
            TopicError::NameEmpty => Self::new("backend.errors.topicNameEmpty"),
            TopicError::InvalidStatus { .. } => Self::new("backend.errors.topicStatusInvalid"),
            TopicError::InvalidItemType { .. } => Self::new("backend.errors.topicItemTypeInvalid"),
            TopicError::InvalidAddedBy { .. } => Self::new("backend.errors.topicAddedByInvalid"),
            TopicError::NoteContentEmpty => Self::new("backend.errors.topicNoteContentEmpty"),
            TopicError::FilenameEmpty => Self::new("backend.errors.topicFilenameEmpty"),
            TopicError::FileNotFound => Self::new("backend.errors.topicFileNotFound"),
            TopicError::Persistence(error) => {
                tracing::error!(error = ?error, "topic persistence failed");
                Self::new("backend.errors.topicPersistenceFailed")
            }
        }
    }
}

fn map_topic_input(input: TopicInput) -> Result<TopicUpsertRequest, TopicError> {
    Ok(TopicUpsertRequest {
        id: input.id,
        name: input.name,
        description: input.description,
        color: input.color,
        icon: input.icon,
        status: input
            .status
            .as_deref()
            .map(TopicStatus::try_from)
            .transpose()?,
        due_date: input.due_date,
    })
}

fn map_add_items(input: AddTopicItemsInput) -> Result<AddTopicItemsRequest, TopicError> {
    Ok(AddTopicItemsRequest {
        topic_id: input.topic_id,
        item_type: input.item_type.as_str().try_into()?,
        item_ids: input.item_ids,
        added_by: input
            .added_by
            .as_deref()
            .map(TopicAddedBy::try_from)
            .transpose()?,
    })
}

fn map_remove_item(input: RemoveTopicItemInput) -> Result<RemoveTopicItemRequest, TopicError> {
    Ok(RemoveTopicItemRequest {
        topic_id: input.topic_id,
        item_type: input.item_type.as_str().try_into()?,
        item_id: input.item_id,
    })
}

fn map_membership_query(input: TopicMembershipsInput) -> Result<TopicMembershipsQuery, TopicError> {
    Ok(TopicMembershipsQuery {
        item_type: input.item_type.as_str().try_into()?,
        item_ids: input.item_ids,
    })
}

fn map_note_input(input: TopicNoteInput) -> TopicNoteWriteRequest {
    TopicNoteWriteRequest {
        id: input.id,
        topic_id: input.topic_id,
        content: input.content,
    }
}

fn decode_topic_file_content(content_base64: &str) -> Result<Vec<u8>, CommandError> {
    base64::engine::general_purpose::STANDARD
        .decode(content_base64.as_bytes())
        .map_err(|_| CommandError::new("backend.errors.topicFileContentInvalid"))
}

#[tauri::command]
pub async fn list_topics(
    pool: tauri::State<'_, crate::db::ReadPool>,
) -> CommandResult<Vec<TopicDto>> {
    TopicRepository::new(pool.inner().as_ref())
        .list()
        .await
        .map(|records| records.into_iter().map(Into::into).collect())
        .map_err(Into::into)
}

#[tauri::command]
pub async fn upsert_topic(
    pool: tauri::State<'_, sqlx::SqlitePool>,
    input: TopicInput,
) -> CommandResult<TopicDto> {
    let request = map_topic_input(input).map_err(CommandError::from)?;
    TopicRepository::new(pool.inner())
        .upsert(request)
        .await
        .map(Into::into)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn delete_topic(
    pool: tauri::State<'_, sqlx::SqlitePool>,
    id: String,
) -> CommandResult<()> {
    TopicRepository::new(pool.inner())
        .delete(&id)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn add_topic_items(
    pool: tauri::State<'_, sqlx::SqlitePool>,
    input: AddTopicItemsInput,
) -> CommandResult<Vec<TopicMembershipReceiptDto>> {
    add_topic_items_inner(pool.inner(), input).await
}

async fn add_topic_items_inner(
    pool: &sqlx::SqlitePool,
    input: AddTopicItemsInput,
) -> CommandResult<Vec<TopicMembershipReceiptDto>> {
    let request = map_add_items(input).map_err(CommandError::from)?;
    TopicRepository::new(pool)
        .add_items(request)
        .await
        .map(|rows| rows.into_iter().map(Into::into).collect())
        .map_err(Into::into)
}

#[tauri::command]
pub async fn remove_topic_item(
    pool: tauri::State<'_, sqlx::SqlitePool>,
    input: RemoveTopicItemInput,
) -> CommandResult<Option<TopicMembershipReceiptDto>> {
    remove_topic_item_inner(pool.inner(), input).await
}

async fn remove_topic_item_inner(
    pool: &sqlx::SqlitePool,
    input: RemoveTopicItemInput,
) -> CommandResult<Option<TopicMembershipReceiptDto>> {
    let request = map_remove_item(input).map_err(CommandError::from)?;
    TopicRepository::new(pool)
        .remove_item(request)
        .await
        .map(|row| row.map(Into::into))
        .map_err(Into::into)
}

#[tauri::command]
pub async fn undo_topic_memberships(
    pool: tauri::State<'_, sqlx::SqlitePool>,
    input: UndoTopicMembershipsInput,
) -> CommandResult<Vec<String>> {
    undo_topic_memberships_inner(pool.inner(), input).await
}

async fn undo_topic_memberships_inner(
    pool: &sqlx::SqlitePool,
    input: UndoTopicMembershipsInput,
) -> CommandResult<Vec<String>> {
    let item_type = input
        .item_type
        .as_str()
        .try_into()
        .map_err(CommandError::from)?;
    let receipts: Vec<_> = input
        .receipts
        .into_iter()
        .map(|row| TopicMembershipReceipt {
            item_id: row.item_id,
            revision: row.revision,
        })
        .collect();
    TopicRepository::new(pool)
        .undo_memberships(&input.topic_id, item_type, &receipts, input.restore)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn list_topic_memberships(
    pool: tauri::State<'_, crate::db::ReadPool>,
    input: TopicMembershipsInput,
) -> CommandResult<Vec<TopicMembershipDto>> {
    let query = map_membership_query(input).map_err(CommandError::from)?;
    TopicRepository::new(pool.inner().as_ref())
        .list_memberships(query)
        .await
        .map(|records| records.into_iter().map(Into::into).collect())
        .map_err(Into::into)
}

#[tauri::command]
pub async fn get_topic_timeline(
    pool: tauri::State<'_, crate::db::ReadPool>,
    topic_id: String,
) -> CommandResult<Vec<TopicTimelineItemDto>> {
    TopicRepository::new(pool.inner().as_ref())
        .timeline(&topic_id)
        .await
        .map(|records| records.into_iter().map(Into::into).collect())
        .map_err(Into::into)
}

#[tauri::command]
pub async fn upsert_topic_note(
    pool: tauri::State<'_, sqlx::SqlitePool>,
    input: TopicNoteInput,
) -> CommandResult<TopicNoteDto> {
    TopicRepository::new(pool.inner())
        .upsert_note(map_note_input(input))
        .await
        .map(Into::into)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn delete_topic_note(
    pool: tauri::State<'_, sqlx::SqlitePool>,
    id: String,
) -> CommandResult<()> {
    TopicRepository::new(pool.inner())
        .delete_note(&id)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn add_topic_file(
    pool: tauri::State<'_, sqlx::SqlitePool>,
    input: TopicFileInput,
) -> CommandResult<TopicFileDto> {
    // Keep the command's historical validation order: an empty filename wins
    // over malformed content, even though decoding happens in this adapter.
    if input.filename.trim().is_empty() {
        return Err(CommandError::from(TopicError::FilenameEmpty));
    }
    let content = decode_topic_file_content(&input.content_base64)?;
    TopicRepository::new(pool.inner())
        .add_file(TopicFileWriteRequest {
            topic_id: input.topic_id,
            filename: input.filename,
            mime_type: input.mime_type,
            content,
        })
        .await
        .map(Into::into)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn list_topic_files(
    pool: tauri::State<'_, crate::db::ReadPool>,
    topic_id: String,
) -> CommandResult<Vec<TopicFileDto>> {
    TopicRepository::new(pool.inner().as_ref())
        .list_files(&topic_id)
        .await
        .map(|records| records.into_iter().map(Into::into).collect())
        .map_err(Into::into)
}

#[tauri::command]
pub async fn download_topic_file(
    pool: tauri::State<'_, crate::db::ReadPool>,
    id: String,
) -> CommandResult<TopicFileDownload> {
    TopicRepository::new(pool.inner().as_ref())
        .download_file(&id)
        .await
        .map(Into::into)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn delete_topic_file(
    pool: tauri::State<'_, sqlx::SqlitePool>,
    id: String,
) -> CommandResult<()> {
    TopicRepository::new(pool.inner())
        .delete_file(&id)
        .await
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use base64::Engine;

    use super::{
        decode_topic_file_content, map_add_items, map_topic_input, TopicFileDownload, TopicInput,
    };
    use crate::commands::error::CommandError;
    use satchel_core::db::topics::TopicError;

    fn encode_topic_file_content(content: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(content)
    }

    #[test]
    fn file_base64_adapter_round_trips_and_rejects_invalid_input() {
        let content = decode_topic_file_content("aGFsbG8=").expect("valid base64");
        assert_eq!(content, b"hallo");
        assert_eq!(encode_topic_file_content(&content), "aGFsbG8=");
        let error = decode_topic_file_content("not base64").unwrap_err();
        assert_eq!(error.key, "backend.errors.topicFileContentInvalid");
    }

    #[test]
    fn file_download_keeps_the_existing_json_shape() {
        let download = TopicFileDownload {
            filename: "liste.csv".into(),
            mime_type: "text/csv".into(),
            data_base64: encode_topic_file_content(b"hallo"),
        };
        assert_eq!(
            serde_json::to_value(download).unwrap(),
            serde_json::json!({
                "filename": "liste.csv",
                "mime_type": "text/csv",
                "data_base64": "aGFsbG8="
            })
        );
    }

    #[test]
    fn topic_adapter_errors_are_stable_and_redact_sql_details() {
        let detail = "near SELECT: secret topic SQL";
        let error = CommandError::from(TopicError::Persistence(sqlx::Error::Protocol(
            detail.into(),
        )));
        let serialized = serde_json::to_value(error).unwrap();
        assert_eq!(
            serialized,
            serde_json::json!({ "key": "backend.errors.topicPersistenceFailed" })
        );
        assert!(!serialized.to_string().contains(detail));
    }

    #[test]
    fn adapters_validate_wire_strings_before_core_calls() {
        let status_error = CommandError::from(
            map_topic_input(TopicInput {
                id: None,
                name: "Topic".into(),
                description: None,
                color: None,
                icon: None,
                status: Some("closed".into()),
                due_date: None,
            })
            .unwrap_err(),
        );
        assert_eq!(status_error.key, "backend.errors.topicStatusInvalid");

        let item_error = CommandError::from(
            map_add_items(super::AddTopicItemsInput {
                topic_id: "topic-1".into(),
                item_type: "folder".into(),
                item_ids: vec!["folder-1".into()],
                added_by: None,
            })
            .unwrap_err(),
        );
        assert_eq!(item_error.key, "backend.errors.topicItemTypeInvalid");
    }
}

#[cfg(test)]
mod membership_command_tests {
    use super::*;

    #[tokio::test]
    async fn wire_receipts_round_trip_and_reject_a_delayed_undo() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("../crates/satchel-core/src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
        let topic = TopicRepository::new(&pool)
            .upsert(TopicUpsertRequest {
                id: None,
                name: "Guarded".into(),
                description: None,
                color: None,
                icon: None,
                status: None,
                due_date: None,
            })
            .await
            .unwrap();
        let add = || AddTopicItemsInput {
            topic_id: topic.id.clone(),
            item_type: "event".into(),
            item_ids: vec!["event".into()],
            added_by: None,
        };
        let first = add_topic_items_inner(&pool, add()).await.unwrap();
        let removed = remove_topic_item_inner(
            &pool,
            RemoveTopicItemInput {
                topic_id: topic.id.clone(),
                item_type: "event".into(),
                item_id: "event".into(),
            },
        )
        .await
        .unwrap()
        .unwrap();
        add_topic_items_inner(&pool, add()).await.unwrap();
        let wire = serde_json::json!({ "topic_id": topic.id, "item_type": "event", "receipts": first, "restore": false });
        let undo: UndoTopicMembershipsInput = serde_json::from_value(wire).unwrap();
        assert!(undo_topic_memberships_inner(&pool, undo)
            .await
            .unwrap()
            .is_empty());
        let stale_restore = UndoTopicMembershipsInput {
            topic_id: topic.id.clone(),
            item_type: "event".into(),
            receipts: vec![removed],
            restore: true,
        };
        assert!(undo_topic_memberships_inner(&pool, stale_restore)
            .await
            .unwrap()
            .is_empty());
        assert!(add_topic_items_inner(&pool, add())
            .await
            .unwrap()
            .is_empty());
    }
}
