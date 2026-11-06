use serde::Deserialize;
use sqlx::SqlitePool;

use crate::commands::error::CommandResult;
use crate::AppState;

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/move_threads/", optional_fields = nullable))]
pub struct MoveThreadsInput {
    pub thread_ids: Vec<String>,
    pub target_folder_id: String,
}

/// Queue a durable move without changing confirmed provider identities.
///
/// Moving mail out of Spam into a folder that is not Trash tells the spam
/// filter that mail was wanted.
#[tauri::command]
pub async fn move_threads_to_folder(
    pool: tauri::State<'_, SqlitePool>,
    state: tauri::State<'_, AppState>,
    input: MoveThreadsInput,
) -> CommandResult<()> {
    move_threads_to_folder_inner(pool.inner(), state.inner(), input).await
}

pub(crate) async fn move_threads_to_folder_inner(
    pool: &SqlitePool,
    state: &AppState,
    input: MoveThreadsInput,
) -> CommandResult<()> {
    let rescued = rescued_from_spam(pool, &input).await;
    satchel_core::mail_actions::move_threads(
        pool,
        &state.queue,
        &input.thread_ids,
        satchel_core::mail_actions::MoveTarget::Folder(&input.target_folder_id),
    )
    .await?;
    if !rescued.is_empty() {
        // One example per sender and thread, like every other training path.
        if let Err(error) = satchel_core::spam::train_threads(
            pool,
            &rescued,
            satchel_core::spam::Label::Ham,
            satchel_core::spam::Signal::Explicit,
            None,
        )
        .await
        {
            tracing::warn!(%error, "spam filter training failed after a move out of Spam");
        }
    }
    Ok(())
}

/// The threads this move takes mail out of Spam for, if its target is a
/// folder where wanted mail lives.
async fn rescued_from_spam(pool: &SqlitePool, input: &MoveThreadsInput) -> Vec<String> {
    let target: Option<String> =
        sqlx::query_scalar("SELECT LOWER(COALESCE(folder_type, '')) FROM folders WHERE id = ?")
            .bind(&input.target_folder_id)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
    if target
        .as_deref()
        .is_none_or(|kind| matches!(kind, "spam" | "trash"))
    {
        return Vec::new();
    }
    let mut rescued = Vec::new();
    for thread_id in &input.thread_ids {
        let in_spam =
            satchel_core::spam::spam_folder_messages(pool, std::slice::from_ref(thread_id))
                .await
                .map(|messages| !messages.is_empty())
                .unwrap_or(false);
        if in_spam && !rescued.contains(thread_id) {
            rescued.push(thread_id.clone());
        }
    }
    rescued
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    /// A thread of three messages from one sender and one from another,
    /// moved out of Spam: two examples, not four.
    #[tokio::test]
    async fn moving_out_of_spam_trains_one_example_per_sender() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("pool");
        sqlx::migrate!("../crates/satchel-core/src/db/migrations")
            .run(&pool)
            .await
            .expect("migrations");
        sqlx::query(
            "INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
             VALUES ('acc', 'Me', 'me@example.test', 'pop3', '{}', '', '', '')",
        )
        .execute(&pool)
        .await
        .unwrap();
        for (id, kind) in [("spam", "spam"), ("work", "custom")] {
            sqlx::query(
                "INSERT INTO folders (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at)
                 VALUES (?, 'acc', ?, ?, ?, ?, '', '')",
            )
            .bind(id)
            .bind(id)
            .bind(id)
            .bind(kind)
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
        }
        sqlx::query(
            "INSERT INTO threads (id, account_id, subject, last_message_date, created_at, updated_at)
             VALUES ('thread', 'acc', 'Order', '', '', '')",
        )
        .execute(&pool)
        .await
        .unwrap();
        for (id, from) in [
            ("m1", "shop@store.example"),
            ("m2", "shop@store.example"),
            ("m3", "shop@store.example"),
            ("m4", "help@store.example"),
        ] {
            sqlx::query(
                "INSERT INTO messages (id, account_id, folder_id, thread_id, from_address, to_addresses, subject, body_text, date, created_at, updated_at)
                 VALUES (?, 'acc', 'spam', 'thread', ?, '[]', 'Your order', 'Your order has shipped', ?, '', '')",
            )
            .bind(id)
            .bind(from)
            .bind(format!("2026-01-0{}", &id[1..]))
            .execute(&pool)
            .await
            .unwrap();
        }

        move_threads_to_folder_inner(
            &pool,
            &AppState::default(),
            MoveThreadsInput {
                thread_ids: vec!["thread".into()],
                target_folder_id: "work".into(),
            },
        )
        .await
        .expect("move");

        let status = satchel_core::spam::status(&pool).await.unwrap();
        assert_eq!((status.spam_messages, status.ham_messages), (0, 2));
    }
}
