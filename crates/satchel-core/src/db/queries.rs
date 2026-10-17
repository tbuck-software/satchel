use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "core/db/queries/"))]
pub struct ThreadSummary {
    pub id: String,
    pub subject: String,
    pub workflow_state: String,
    pub unread_count: i64,
}

pub async fn list_threads(
    pool: &sqlx::SqlitePool,
    state_filter: Option<&str>,
) -> Result<Vec<ThreadSummary>, sqlx::Error> {
    let rows = if let Some(state) = state_filter {
        sqlx::query_as::<_, ThreadSummary>(
            "SELECT id, subject, workflow_state, unread_count FROM threads WHERE workflow_state = ? ORDER BY last_message_date DESC",
        )
        .bind(state)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as::<_, ThreadSummary>(
            "SELECT id, subject, workflow_state, unread_count FROM threads ORDER BY last_message_date DESC",
        )
        .fetch_all(pool)
        .await?
    };

    Ok(rows)
}
