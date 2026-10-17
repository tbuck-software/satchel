//! Tauri-free persistence for saved searches (smart folders).
use sqlx::{FromRow, SqlitePool};
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct SmartFolderUpsertRequest {
    pub id: Option<String>,
    pub name: String,
    pub icon: Option<String>,
    pub query: String,
    pub sort_order: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmartFolderRecord {
    pub id: String,
    pub name: String,
    pub icon: Option<String>,
    pub query: String,
    pub sort_order: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Error)]
pub enum SmartFolderError {
    #[error("smart folder persistence failed: {0}")]
    Persistence(#[source] sqlx::Error),
}

impl From<sqlx::Error> for SmartFolderError {
    fn from(error: sqlx::Error) -> Self {
        Self::Persistence(error)
    }
}

#[derive(Debug, FromRow)]
struct StoredSmartFolder {
    id: String,
    name: String,
    icon: Option<String>,
    query: String,
    sort_order: i64,
    created_at: String,
    updated_at: String,
}

impl From<StoredSmartFolder> for SmartFolderRecord {
    fn from(row: StoredSmartFolder) -> Self {
        Self {
            id: row.id,
            name: row.name,
            icon: row.icon,
            query: row.query,
            sort_order: row.sort_order,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

const SMART_FOLDER_SELECT: &str =
    "SELECT id, name, icon, query, sort_order, created_at, updated_at FROM smart_folders";

/// Owns persistence for saved searches while leaving serialization and Tauri
/// command concerns to the app crate.
pub struct SmartFolderRepository<'a> {
    pool: &'a SqlitePool,
}

impl<'a> SmartFolderRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn list(&self) -> Result<Vec<SmartFolderRecord>, SmartFolderError> {
        sqlx::query_as::<_, StoredSmartFolder>(&format!(
            "{SMART_FOLDER_SELECT} ORDER BY sort_order ASC, name ASC"
        ))
        .fetch_all(self.pool)
        .await
        .map(|rows| rows.into_iter().map(Into::into).collect())
        .map_err(Into::into)
    }

    pub async fn upsert(
        &self,
        input: SmartFolderUpsertRequest,
    ) -> Result<SmartFolderRecord, SmartFolderError> {
        let id = input
            .id
            .unwrap_or_else(|| format!("sf-{}", uuid::Uuid::new_v4()));
        let now = chrono::Utc::now().to_rfc3339();
        let sort_order = input.sort_order.unwrap_or(0);

        sqlx::query(
            "INSERT INTO smart_folders (id, name, icon, query, sort_order, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                icon = excluded.icon,
                query = excluded.query,
                sort_order = excluded.sort_order,
                updated_at = excluded.updated_at",
        )
        .bind(&id)
        .bind(&input.name)
        .bind(input.icon.as_deref())
        .bind(&input.query)
        .bind(sort_order)
        .bind(&now)
        .bind(&now)
        .execute(self.pool)
        .await?;

        sqlx::query_as::<_, StoredSmartFolder>(&format!("{SMART_FOLDER_SELECT} WHERE id = ?"))
            .bind(id)
            .fetch_one(self.pool)
            .await
            .map(Into::into)
            .map_err(Into::into)
    }

    pub async fn delete(&self, id: &str) -> Result<(), SmartFolderError> {
        sqlx::query("DELETE FROM smart_folders WHERE id = ?")
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }
}
