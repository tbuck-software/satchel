//! Tauri-free persistence for topics and their linked items.
use sqlx::{FromRow, QueryBuilder, Sqlite, SqlitePool};
use thiserror::Error;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TopicStatus {
    #[default]
    Active,
    Done,
    Archived,
}

impl TopicStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Done => "done",
            Self::Archived => "archived",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TopicItemType {
    Thread,
    Message,
    Event,
}

impl TopicItemType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Thread => "thread",
            Self::Message => "message",
            Self::Event => "event",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TopicAddedBy {
    User,
    Rule,
    Llm,
}

impl TopicAddedBy {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Rule => "rule",
            Self::Llm => "llm",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TopicTimelineItemKind {
    Thread,
    Message,
    Note,
}

impl TopicTimelineItemKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Thread => "thread",
            Self::Message => "message",
            Self::Note => "note",
        }
    }
}

#[derive(Debug, Error)]
pub enum TopicError {
    #[error("topic name must not be empty")]
    NameEmpty,
    #[error("invalid topic status: {value}")]
    InvalidStatus { value: String },
    #[error("invalid topic item type: {value}")]
    InvalidItemType { value: String },
    #[error("invalid topic added_by value: {value}")]
    InvalidAddedBy { value: String },
    #[error("note content must not be empty")]
    NoteContentEmpty,
    #[error("filename must not be empty")]
    FilenameEmpty,
    #[error("topic file not found")]
    FileNotFound,
    #[error("topic persistence failed")]
    Persistence(#[source] sqlx::Error),
}

impl From<sqlx::Error> for TopicError {
    fn from(error: sqlx::Error) -> Self {
        Self::Persistence(error)
    }
}

impl TryFrom<&str> for TopicStatus {
    type Error = TopicError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "active" => Ok(Self::Active),
            "done" => Ok(Self::Done),
            "archived" => Ok(Self::Archived),
            value => Err(TopicError::InvalidStatus {
                value: value.to_owned(),
            }),
        }
    }
}

impl TryFrom<&str> for TopicItemType {
    type Error = TopicError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "thread" => Ok(Self::Thread),
            "message" => Ok(Self::Message),
            "event" => Ok(Self::Event),
            value => Err(TopicError::InvalidItemType {
                value: value.to_owned(),
            }),
        }
    }
}

impl TryFrom<&str> for TopicAddedBy {
    type Error = TopicError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "user" => Ok(Self::User),
            "rule" => Ok(Self::Rule),
            "llm" => Ok(Self::Llm),
            value => Err(TopicError::InvalidAddedBy {
                value: value.to_owned(),
            }),
        }
    }
}

impl TryFrom<&str> for TopicTimelineItemKind {
    type Error = TopicError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "thread" => Ok(Self::Thread),
            "message" => Ok(Self::Message),
            "note" => Ok(Self::Note),
            value => Err(TopicError::InvalidItemType {
                value: value.to_owned(),
            }),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TopicUpsertRequest {
    pub id: Option<String>,
    pub name: String,
    pub description: Option<String>,
    pub color: Option<String>,
    pub icon: Option<String>,
    pub status: Option<TopicStatus>,
    pub due_date: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AddTopicItemsRequest {
    pub topic_id: String,
    pub item_type: TopicItemType,
    pub item_ids: Vec<String>,
    pub added_by: Option<TopicAddedBy>,
}

#[derive(Debug, Clone)]
pub struct RemoveTopicItemRequest {
    pub topic_id: String,
    pub item_type: TopicItemType,
    pub item_id: String,
}

#[derive(Debug, Clone)]
pub struct TopicMembershipsQuery {
    pub item_type: TopicItemType,
    pub item_ids: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct TopicNoteWriteRequest {
    pub id: Option<String>,
    pub topic_id: String,
    pub content: String,
}

/// File content is decoded by the app adapter before it crosses into Core.
#[derive(Debug, Clone)]
pub struct TopicFileWriteRequest {
    pub topic_id: String,
    pub filename: String,
    pub mime_type: Option<String>,
    pub content: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct TopicRecord {
    pub id: String,
    pub name: String,
    pub description: String,
    pub color: Option<String>,
    pub icon: Option<String>,
    pub status: TopicStatus,
    pub due_date: Option<String>,
    pub item_count: i64,
    pub note_count: i64,
    /// Unread messages across linked threads; drives the folder badge.
    pub unread_count: i64,
    /// Sender of the newest non-draft message across linked threads and
    /// messages. The topic avatar falls back to it when no icon is set.
    pub last_sender_name: Option<String>,
    pub last_sender_email: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct TopicMembershipRecord {
    pub topic_id: String,
    pub item_id: String,
}

#[derive(Debug, Clone)]
pub struct TopicNoteRecord {
    pub id: String,
    pub topic_id: String,
    pub content: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct TopicTimelineItemRecord {
    pub item_type: TopicTimelineItemKind,
    pub item_id: String,
    /// Navigation target; equals `item_id` for threads, `None` for notes.
    pub thread_id: Option<String>,
    pub title: Option<String>,
    pub snippet: Option<String>,
    /// Full markdown, notes only (for inline editing).
    pub content: Option<String>,
    pub from_address: Option<String>,
    pub sort_date: String,
    pub added_at: String,
    pub added_by: TopicAddedBy,
    pub message_count: i64,
    pub unread_count: i64,
}

/// Metadata only — content stays in the DB until explicitly downloaded.
#[derive(Debug, Clone)]
pub struct TopicFileRecord {
    pub id: String,
    pub topic_id: String,
    pub filename: String,
    pub mime_type: String,
    pub size: i64,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct TopicFileContentRecord {
    pub filename: String,
    pub mime_type: String,
    pub content: Vec<u8>,
}

// item_count only counts links whose target row still exists — the timeline's
// INNER JOINs hide orphans, so counting them would let the sidebar badge drift
// from what the timeline shows.
const TOPIC_SELECT: &str = "SELECT
        topics.id, topics.name, topics.description, topics.color, topics.icon,
        topics.status, topics.due_date,
        (
          SELECT COUNT(*) FROM topic_items ti
          WHERE ti.topic_id = topics.id AND (
            (ti.item_type = 'thread'
              AND EXISTS (SELECT 1 FROM threads t WHERE t.id = ti.item_id AND {visible_t}))
            OR (ti.item_type = 'message'
              AND EXISTS (SELECT 1 FROM messages m WHERE m.id = ti.item_id AND {visible_m}))
            OR ti.item_type = 'event'
          )
        ) AS item_count,
        (SELECT COUNT(*) FROM topic_notes n WHERE n.topic_id = topics.id) AS note_count,
        (
          SELECT COALESCE(SUM(t.unread_count), 0) FROM topic_items ti
          JOIN threads t ON t.id = ti.item_id
          WHERE ti.topic_id = topics.id AND ti.item_type = 'thread' AND {visible_t}
        ) AS unread_count,
        (
          SELECT linked.from_name FROM (
            SELECT m.from_name, m.date, m.id
            FROM topic_items ti
            JOIN messages m ON m.thread_id = ti.item_id
            WHERE ti.topic_id = topics.id AND ti.item_type = 'thread' AND m.is_draft = 0 AND {visible_m}
            UNION ALL
            SELECT m.from_name, m.date, m.id
            FROM topic_items ti
            JOIN messages m ON m.id = ti.item_id
            WHERE ti.topic_id = topics.id AND ti.item_type = 'message' AND m.is_draft = 0 AND {visible_m}
          ) linked
          ORDER BY linked.date DESC, linked.id DESC LIMIT 1
        ) AS last_sender_name,
        (
          SELECT linked.from_address FROM (
            SELECT m.from_address, m.date, m.id
            FROM topic_items ti
            JOIN messages m ON m.thread_id = ti.item_id
            WHERE ti.topic_id = topics.id AND ti.item_type = 'thread' AND m.is_draft = 0 AND {visible_m}
            UNION ALL
            SELECT m.from_address, m.date, m.id
            FROM topic_items ti
            JOIN messages m ON m.id = ti.item_id
            WHERE ti.topic_id = topics.id AND ti.item_type = 'message' AND m.is_draft = 0 AND {visible_m}
          ) linked
          ORDER BY linked.date DESC, linked.id DESC LIMIT 1
        ) AS last_sender_email,
        topics.created_at, topics.updated_at
     FROM topics";

fn topic_select() -> String {
    TOPIC_SELECT
        .replace(
            "{visible_t}",
            &crate::db::account_visibility::visible_account_sql("t.account_id"),
        )
        .replace(
            "{visible_m}",
            &crate::db::account_visibility::visible_account_sql("m.account_id"),
        )
}

const DELETE_THREAD_LINKS_SQL: &str =
    "DELETE FROM topic_items WHERE item_type = 'thread' AND item_id = ?";

#[derive(Debug, FromRow)]
struct TopicRow {
    id: String,
    name: String,
    description: String,
    color: Option<String>,
    icon: Option<String>,
    status: String,
    due_date: Option<String>,
    item_count: i64,
    note_count: i64,
    unread_count: i64,
    last_sender_name: Option<String>,
    last_sender_email: Option<String>,
    created_at: String,
    updated_at: String,
}

impl TopicRow {
    fn into_record(self) -> Result<TopicRecord, TopicError> {
        Ok(TopicRecord {
            id: self.id,
            name: self.name,
            description: self.description,
            color: self.color,
            icon: self.icon,
            status: self.status.as_str().try_into()?,
            due_date: self.due_date,
            item_count: self.item_count,
            note_count: self.note_count,
            unread_count: self.unread_count,
            last_sender_name: self.last_sender_name,
            last_sender_email: self.last_sender_email,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

#[derive(Debug, FromRow)]
struct TopicMembershipRow {
    topic_id: String,
    item_id: String,
}

impl From<TopicMembershipRow> for TopicMembershipRecord {
    fn from(row: TopicMembershipRow) -> Self {
        Self {
            topic_id: row.topic_id,
            item_id: row.item_id,
        }
    }
}

#[derive(Debug, FromRow)]
struct TopicNoteRow {
    id: String,
    topic_id: String,
    content: String,
    created_at: String,
    updated_at: String,
}

impl From<TopicNoteRow> for TopicNoteRecord {
    fn from(row: TopicNoteRow) -> Self {
        Self {
            id: row.id,
            topic_id: row.topic_id,
            content: row.content,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

#[derive(Debug, FromRow)]
struct TopicTimelineItemRow {
    item_type: String,
    item_id: String,
    thread_id: Option<String>,
    title: Option<String>,
    snippet: Option<String>,
    content: Option<String>,
    from_address: Option<String>,
    sort_date: String,
    added_at: String,
    added_by: String,
    message_count: i64,
    unread_count: i64,
}

impl TopicTimelineItemRow {
    fn into_record(self) -> Result<TopicTimelineItemRecord, TopicError> {
        Ok(TopicTimelineItemRecord {
            item_type: self.item_type.as_str().try_into()?,
            item_id: self.item_id,
            thread_id: self.thread_id,
            title: self.title,
            snippet: self.snippet,
            content: self.content,
            from_address: self.from_address,
            sort_date: self.sort_date,
            added_at: self.added_at,
            added_by: self.added_by.as_str().try_into()?,
            message_count: self.message_count,
            unread_count: self.unread_count,
        })
    }
}

#[derive(Debug, FromRow)]
struct TopicFileRow {
    id: String,
    topic_id: String,
    filename: String,
    mime_type: String,
    size: i64,
    created_at: String,
}

impl From<TopicFileRow> for TopicFileRecord {
    fn from(row: TopicFileRow) -> Self {
        Self {
            id: row.id,
            topic_id: row.topic_id,
            filename: row.filename,
            mime_type: row.mime_type,
            size: row.size,
            created_at: row.created_at,
        }
    }
}

/// Persistence boundary for topics.
pub struct TopicRepository<'a> {
    pool: &'a SqlitePool,
}

impl<'a> TopicRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn list(&self) -> Result<Vec<TopicRecord>, TopicError> {
        let rows = sqlx::query_as::<_, TopicRow>(&format!(
            "{} ORDER BY topics.name COLLATE NOCASE ASC",
            topic_select()
        ))
        .fetch_all(self.pool)
        .await?;
        rows.into_iter().map(TopicRow::into_record).collect()
    }

    pub async fn upsert(&self, request: TopicUpsertRequest) -> Result<TopicRecord, TopicError> {
        let name = request.name.trim();
        if name.is_empty() {
            return Err(TopicError::NameEmpty);
        }
        let status = request.status.unwrap_or_default();
        let id = request
            .id
            .unwrap_or_else(|| format!("topic-{}", uuid::Uuid::new_v4()));
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO topics (id, name, description, color, icon, status, due_date, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                description = excluded.description,
                color = excluded.color,
                icon = excluded.icon,
                status = excluded.status,
                due_date = excluded.due_date,
                updated_at = excluded.updated_at",
        )
        .bind(&id)
        .bind(name)
        .bind(request.description.as_deref().unwrap_or(""))
        .bind(request.color.as_deref())
        .bind(request.icon.as_deref())
        .bind(status.as_str())
        .bind(request.due_date.as_deref())
        .bind(&now)
        .bind(&now)
        .execute(self.pool)
        .await?;
        let row = sqlx::query_as::<_, TopicRow>(&format!("{} WHERE topics.id = ?", topic_select()))
            .bind(&id)
            .fetch_one(self.pool)
            .await?;
        row.into_record()
    }

    pub async fn delete(&self, id: &str) -> Result<(), TopicError> {
        sqlx::query("DELETE FROM topics WHERE id = ?")
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    /// Returns only links inserted by this transaction, with each item listed once.
    /// Callers can undo this receipt without guessing the previous membership.
    pub async fn add_items(
        &self,
        request: AddTopicItemsRequest,
    ) -> Result<Vec<String>, TopicError> {
        let added_by = request.added_by.unwrap_or(TopicAddedBy::User);
        let now = chrono::Utc::now().to_rfc3339();
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let mut added = Vec::new();
        for item_id in &request.item_ids {
            let result = sqlx::query(
                "INSERT OR IGNORE INTO topic_items (topic_id, item_type, item_id, added_at, added_by)
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(&request.topic_id)
            .bind(request.item_type.as_str())
            .bind(item_id)
            .bind(&now)
            .bind(added_by.as_str())
            .execute(&mut *tx)
            .await?;
            if result.rows_affected() > 0 {
                added.push(item_id.clone());
            }
        }
        tx.commit().await?;
        Ok(added)
    }

    /// Returns whether a link was actually deleted.
    pub async fn remove_item(&self, request: RemoveTopicItemRequest) -> Result<bool, TopicError> {
        let result = sqlx::query(
            "DELETE FROM topic_items WHERE topic_id = ? AND item_type = ? AND item_id = ?",
        )
        .bind(&request.topic_id)
        .bind(request.item_type.as_str())
        .bind(&request.item_id)
        .execute(self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Remove every thread link for a thread that is being deleted.
    ///
    /// Thread links are polymorphic and therefore cannot be cleaned up by a
    /// foreign key on `item_id`. Deleting an absent thread is intentionally
    /// idempotent.
    pub async fn remove_thread_links(&self, thread_id: &str) -> Result<(), TopicError> {
        sqlx::query(DELETE_THREAD_LINKS_SQL)
            .bind(thread_id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    /// Remove every thread link while the caller's transaction is open.
    ///
    /// Thread projection cleanup has to delete the polymorphic topic links in
    /// the same transaction as the dependent rows and the thread itself.
    pub(crate) async fn remove_thread_links_tx(
        tx: &mut sqlx::Transaction<'_, Sqlite>,
        thread_id: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(DELETE_THREAD_LINKS_SQL)
            .bind(thread_id)
            .execute(&mut **tx)
            .await?;
        Ok(())
    }

    /// Remove thread and message links while the caller's transaction is open.
    ///
    /// Polymorphic links have no foreign key on `item_id`, so deleting a
    /// message cannot clean its topic memberships automatically. The message
    /// ids are selected before the caller removes the rows.
    pub(crate) async fn remove_thread_and_message_links_tx(
        tx: &mut sqlx::Transaction<'_, Sqlite>,
        thread_id: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "DELETE FROM topic_items
             WHERE (item_type = 'thread' AND item_id = ?)
                OR (item_type = 'message' AND item_id IN (
                    SELECT id FROM messages WHERE thread_id = ?
                ))",
        )
        .bind(thread_id)
        .bind(thread_id)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    /// Move every topic link from one thread to another atomically.
    ///
    /// Existing destination links win, including their original `added_at`
    /// and `added_by` metadata. A missing source is a no-op. Equal ids are
    /// also a no-op so callers cannot insert and then delete the same link.
    pub async fn transfer_thread_links(
        &self,
        from_thread_id: &str,
        to_thread_id: &str,
    ) -> Result<(), TopicError> {
        if from_thread_id == to_thread_id {
            return Ok(());
        }

        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        Self::transfer_thread_links_tx(&mut tx, from_thread_id, to_thread_id).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Move every topic link while the caller's transaction is open.
    ///
    /// Existing destination links win, including their original `added_at`
    /// and `added_by` metadata. A missing source is a no-op. Equal ids are
    /// also a no-op so callers cannot insert and then delete the same link.
    pub(crate) async fn transfer_thread_links_tx(
        tx: &mut sqlx::Transaction<'_, Sqlite>,
        from_thread_id: &str,
        to_thread_id: &str,
    ) -> Result<(), sqlx::Error> {
        if from_thread_id == to_thread_id {
            return Ok(());
        }

        sqlx::query(
            "INSERT OR IGNORE INTO topic_items (topic_id, item_type, item_id, added_at, added_by)
             SELECT topic_id, 'thread', ?, added_at, added_by
             FROM topic_items
             WHERE item_type = 'thread' AND item_id = ?",
        )
        .bind(to_thread_id)
        .bind(from_thread_id)
        .execute(&mut **tx)
        .await?;
        sqlx::query("DELETE FROM topic_items WHERE item_type = 'thread' AND item_id = ?")
            .bind(from_thread_id)
            .execute(&mut **tx)
            .await?;
        Ok(())
    }

    pub async fn list_memberships(
        &self,
        query: TopicMembershipsQuery,
    ) -> Result<Vec<TopicMembershipRecord>, TopicError> {
        if query.item_ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut builder = QueryBuilder::<Sqlite>::new(
            "SELECT topic_id, item_id FROM topic_items WHERE item_type = ",
        );
        builder.push_bind(query.item_type.as_str());
        builder.push(" AND item_id IN (");
        let mut separated = builder.separated(", ");
        for item_id in &query.item_ids {
            separated.push_bind(item_id);
        }
        builder.push(")");
        let rows = builder
            .build_query_as::<TopicMembershipRow>()
            .fetch_all(self.pool)
            .await?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    pub async fn timeline(
        &self,
        topic_id: &str,
    ) -> Result<Vec<TopicTimelineItemRecord>, TopicError> {
        let rows = sqlx::query_as::<_, TopicTimelineItemRow>(&format!(
            "SELECT * FROM (
               SELECT 'thread' AS item_type,
                      threads.id AS item_id,
                      threads.id AS thread_id,
                      COALESCE((
                        SELECT messages.subject FROM messages
                        WHERE messages.thread_id = threads.id
                        ORDER BY messages.is_draft ASC, messages.date DESC, messages.id DESC
                        LIMIT 1
                      ), threads.subject) AS title,
                      COALESCE((
                        SELECT messages.snippet FROM messages
                        WHERE messages.thread_id = threads.id
                        ORDER BY messages.is_draft ASC, messages.date DESC, messages.id DESC
                        LIMIT 1
                      ), threads.snippet) AS snippet,
                      NULL AS content,
                      (
                        SELECT messages.from_address FROM messages
                        WHERE messages.thread_id = threads.id
                        ORDER BY messages.date DESC LIMIT 1
                      ) AS from_address,
                      threads.last_message_date AS sort_date,
                      pi.added_at AS added_at,
                      pi.added_by AS added_by,
                      threads.message_count AS message_count,
                      threads.unread_count AS unread_count
               FROM topic_items pi
               JOIN threads ON threads.id = pi.item_id
               WHERE pi.topic_id = ? AND pi.item_type = 'thread' AND {}
               UNION ALL
               SELECT 'message', messages.id, messages.thread_id, messages.subject,
                      messages.snippet, NULL, messages.from_address, messages.date,
                      pi.added_at, pi.added_by, 1,
                      CASE WHEN messages.is_read = 0 THEN 1 ELSE 0 END
               FROM topic_items pi
               JOIN messages ON messages.id = pi.item_id
               WHERE pi.topic_id = ? AND pi.item_type = 'message' AND {}
               UNION ALL
               SELECT 'note', n.id, NULL, NULL, substr(n.content, 1, 200), n.content,
                      NULL, n.created_at, n.created_at, 'user', 0, 0
               FROM topic_notes n
               WHERE n.topic_id = ?
             ) ORDER BY sort_date DESC, item_id DESC",
            crate::db::account_visibility::visible_account_sql("threads.account_id"),
            crate::db::account_visibility::visible_account_sql("messages.account_id")
        ))
        .bind(topic_id)
        .bind(topic_id)
        .bind(topic_id)
        .fetch_all(self.pool)
        .await?;
        rows.into_iter()
            .map(TopicTimelineItemRow::into_record)
            .collect()
    }

    pub async fn upsert_note(
        &self,
        request: TopicNoteWriteRequest,
    ) -> Result<TopicNoteRecord, TopicError> {
        if request.content.trim().is_empty() {
            return Err(TopicError::NoteContentEmpty);
        }
        let id = request
            .id
            .unwrap_or_else(|| format!("note-{}", uuid::Uuid::new_v4()));
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO topic_notes (id, topic_id, content, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
                content = excluded.content,
                updated_at = excluded.updated_at",
        )
        .bind(&id)
        .bind(&request.topic_id)
        .bind(&request.content)
        .bind(&now)
        .bind(&now)
        .execute(self.pool)
        .await?;
        let row = sqlx::query_as::<_, TopicNoteRow>(
            "SELECT id, topic_id, content, created_at, updated_at FROM topic_notes WHERE id = ?",
        )
        .bind(&id)
        .fetch_one(self.pool)
        .await?;
        Ok(row.into())
    }

    pub async fn delete_note(&self, id: &str) -> Result<(), TopicError> {
        sqlx::query("DELETE FROM topic_notes WHERE id = ?")
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    pub async fn add_file(
        &self,
        request: TopicFileWriteRequest,
    ) -> Result<TopicFileRecord, TopicError> {
        let filename = request.filename.trim();
        if filename.is_empty() {
            return Err(TopicError::FilenameEmpty);
        }
        let id = format!("tfile-{}", uuid::Uuid::new_v4());
        let now = chrono::Utc::now().to_rfc3339();
        let mime_type = request
            .mime_type
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("application/octet-stream");
        sqlx::query(
            "INSERT INTO topic_files (id, topic_id, filename, mime_type, size, content, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&request.topic_id)
        .bind(filename)
        .bind(mime_type)
        .bind(request.content.len() as i64)
        .bind(&request.content)
        .bind(&now)
        .execute(self.pool)
        .await?;
        let row = sqlx::query_as::<_, TopicFileRow>(
            "SELECT id, topic_id, filename, mime_type, size, created_at FROM topic_files WHERE id = ?",
        )
        .bind(&id)
        .fetch_one(self.pool)
        .await?;
        Ok(row.into())
    }

    pub async fn list_files(&self, topic_id: &str) -> Result<Vec<TopicFileRecord>, TopicError> {
        let rows = sqlx::query_as::<_, TopicFileRow>(
            "SELECT id, topic_id, filename, mime_type, size, created_at
             FROM topic_files WHERE topic_id = ? ORDER BY created_at ASC",
        )
        .bind(topic_id)
        .fetch_all(self.pool)
        .await?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    pub async fn download_file(&self, id: &str) -> Result<TopicFileContentRecord, TopicError> {
        let row = sqlx::query_as::<_, (String, String, Vec<u8>)>(
            "SELECT filename, mime_type, content FROM topic_files WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(self.pool)
        .await?
        .ok_or(TopicError::FileNotFound)?;
        Ok(TopicFileContentRecord {
            filename: row.0,
            mime_type: row.1,
            content: row.2,
        })
    }

    pub async fn delete_file(&self, id: &str) -> Result<(), TopicError> {
        sqlx::query("DELETE FROM topic_files WHERE id = ?")
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    /// Remove links whose target row no longer exists. Called from the hourly
    /// maintenance sweep; `item_type = 'event'` is left alone until event
    /// linking ships.
    pub async fn cleanup_orphaned_items(&self) -> Result<u64, TopicError> {
        let result = sqlx::query(
            "DELETE FROM topic_items
             WHERE (item_type = 'thread'
                    AND NOT EXISTS (SELECT 1 FROM threads WHERE threads.id = topic_items.item_id))
                OR (item_type = 'message'
                    AND NOT EXISTS (SELECT 1 FROM messages WHERE messages.id = topic_items.item_id))",
        )
        .execute(self.pool)
        .await?;
        Ok(result.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use sqlx::sqlite::SqlitePoolOptions;

    use super::*;

    #[tokio::test]
    async fn sender_lookup_starts_from_topic_links() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory db");
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .expect("migrations");

        let plan = sqlx::query_as::<_, (i64, i64, i64, String)>(&format!(
            "EXPLAIN QUERY PLAN {} WHERE topics.id = ?",
            topic_select()
        ))
        .bind("topic-1")
        .fetch_all(&pool)
        .await
        .unwrap();
        let details = plan.iter().map(|row| row.3.as_str()).collect::<Vec<_>>();

        assert!(
            !details.iter().any(|detail| detail.starts_with("SCAN m")),
            "sender lookup must start from the topic's links, not scan every message: {details:?}"
        );
    }
}
