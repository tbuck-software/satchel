//! Trash eviction with bounded discovery and at most 500 message deletions per batch.

use super::threads::{
    clear_thread_list_snapshots_tx, refresh_thread_projection_tx, remote_move_queued_for_thread,
    ThreadError,
};
use super::topics::TopicRepository;
use sqlx::SqlitePool;

const THREAD_PAGE: i64 = 100;
const MESSAGE_BATCH: i64 = 500;

#[derive(Debug)]
pub struct TrashPurgeBatch {
    pub cursor: Option<String>,
    pub removed_threads: u64,
    pub removed_messages: u64,
}

pub async fn purge_batch(
    pool: &SqlitePool,
    cutoff: &str,
    after: &str,
) -> Result<TrashPurgeBatch, ThreadError> {
    // Discovery does not hold SQLite's writer lock.
    let candidates: Vec<(String, String)> =
        sqlx::query_as("SELECT id, account_id FROM threads WHERE id > ? ORDER BY id LIMIT ?")
            .bind(after)
            .bind(THREAD_PAGE)
            .fetch_all(pool)
            .await?;
    let mut result = TrashPurgeBatch {
        cursor: None,
        removed_threads: 0,
        removed_messages: 0,
    };
    if candidates.is_empty() {
        return Ok(result);
    }
    let mut cursor = after.to_owned();
    for (thread_id, account_id) in candidates {
        let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
        // Revalidate after obtaining the lock: a user may have moved the thread.
        let eligible: bool = sqlx::query_scalar(
            "SELECT EXISTS (
               SELECT 1 FROM threads t WHERE t.id = ? AND t.workflow_state = 'done'
                 AND COALESCE(t.workflow_changed_at, t.updated_at) < ?
                 AND EXISTS (SELECT 1 FROM messages m JOIN folders f ON f.id = m.folder_id
                   WHERE m.thread_id = t.id AND LOWER(TRIM(f.folder_type)) = 'trash')
                 AND NOT EXISTS (SELECT 1 FROM messages m LEFT JOIN folders f ON f.id = m.folder_id
                   WHERE m.thread_id = t.id AND LOWER(TRIM(COALESCE(f.folder_type, ''))) <> 'trash')
             )",
        )
        .bind(&thread_id)
        .bind(cutoff)
        .fetch_one(&mut *tx)
        .await?;
        if !eligible || remote_move_queued_for_thread(&mut tx, &account_id, &thread_id).await? {
            tx.rollback().await?;
            cursor = thread_id;
            continue;
        }
        let limit = MESSAGE_BATCH - result.removed_messages as i64;
        sqlx::query("DELETE FROM topic_items WHERE item_type = 'message' AND item_id IN (SELECT id FROM messages WHERE thread_id = ? ORDER BY id LIMIT ?)")
            .bind(&thread_id).bind(limit).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM attachments WHERE message_id IN (SELECT id FROM messages WHERE thread_id = ? ORDER BY id LIMIT ?)")
            .bind(&thread_id).bind(limit).execute(&mut *tx).await?;
        result.removed_messages += sqlx::query("DELETE FROM messages WHERE id IN (SELECT id FROM messages WHERE thread_id = ? ORDER BY id LIMIT ?)")
            .bind(&thread_id).bind(limit).execute(&mut *tx).await?.rows_affected();
        let remains: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM messages WHERE thread_id = ?)")
                .bind(&thread_id)
                .fetch_one(&mut *tx)
                .await?;
        if remains {
            let updated_at: String =
                sqlx::query_scalar("SELECT updated_at FROM threads WHERE id = ?")
                    .bind(&thread_id)
                    .fetch_one(&mut *tx)
                    .await?;
            refresh_thread_projection_tx(&mut tx, &thread_id, &updated_at).await?;
        } else {
            TopicRepository::remove_thread_and_message_links_tx(&mut tx, &thread_id).await?;
            sqlx::query("DELETE FROM thread_tags WHERE thread_id = ?")
                .bind(&thread_id)
                .execute(&mut *tx)
                .await?;
            sqlx::query("DELETE FROM workflow_events WHERE thread_id = ?")
                .bind(&thread_id)
                .execute(&mut *tx)
                .await?;
            result.removed_threads += sqlx::query("DELETE FROM threads WHERE id = ?")
                .bind(&thread_id)
                .execute(&mut *tx)
                .await?
                .rows_affected();
            cursor = thread_id;
        }
        clear_thread_list_snapshots_tx(&mut tx).await?;
        tx.commit().await?;
        if result.removed_messages >= MESSAGE_BATCH as u64 {
            break;
        }
    }
    result.cursor = Some(cursor);
    Ok(result)
}
