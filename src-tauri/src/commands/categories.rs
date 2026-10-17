use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::commands::error::{CommandError, CommandResult};
use satchel_core::db::sender_preferences::{
    SenderPreferenceCategoryUpdate, SenderPreferenceError, SenderPreferenceRecord,
    SenderPreferenceRepository, SenderPreferenceUpdate,
};
use satchel_core::db::threads::ThreadRepository;

pub use satchel_core::db::sender_preferences::{classify_message, provider_category};

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/categories/", optional_fields = nullable))]
pub struct SetSenderPreferenceInput {
    pub account_id: String,
    pub sender_email: String,
    /// Missing means unchanged; JSON null clears the account's category.
    #[serde(default, deserialize_with = "deserialize_category_update")]
    // The custom deserializer accepts a nullable string, not the domain enum.
    #[cfg_attr(feature = "ipc-types", ts(as = "Option<String>", optional = nullable))]
    pub category: SenderPreferenceCategoryUpdate,
    #[serde(default)]
    pub is_priority: Option<bool>,
}

fn deserialize_category_update<'de, D>(
    deserializer: D,
) -> Result<SenderPreferenceCategoryUpdate, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let category = Option::<String>::deserialize(deserializer)?;
    Ok(match category {
        Some(value) => SenderPreferenceCategoryUpdate::Set(value),
        None => SenderPreferenceCategoryUpdate::Clear,
    })
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/categories/")
)]
pub struct SenderPreference {
    pub sender_email: String,
    pub category: Option<String>,
    pub is_priority: bool,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/categories/", optional_fields = nullable))]
pub struct SetThreadPriorityInput {
    pub thread_id: String,
    pub priority: u8,
}

impl From<SenderPreferenceRecord> for SenderPreference {
    fn from(record: SenderPreferenceRecord) -> Self {
        Self {
            sender_email: record.sender_email,
            category: record.category,
            is_priority: record.is_priority,
        }
    }
}

impl From<SenderPreferenceError> for CommandError {
    fn from(error: SenderPreferenceError) -> Self {
        match error {
            SenderPreferenceError::AccountIdEmpty => {
                Self::new("backend.errors.senderPreferenceAccountInvalid")
            }
            SenderPreferenceError::SenderEmailInvalid => {
                Self::new("backend.errors.senderPreferenceEmailInvalid")
            }
            SenderPreferenceError::CategoryInvalid => {
                Self::new("backend.errors.senderPreferenceCategoryInvalid")
            }
            SenderPreferenceError::Persistence(error) => {
                tracing::error!(error = ?error, "sender preference persistence failed");
                Self::new("backend.errors.senderPreferencePersistenceFailed")
            }
            SenderPreferenceError::Projection(error) => {
                tracing::error!(error = ?error, "sender preference projection failed");
                Self::new("backend.errors.senderPreferencePersistenceFailed")
            }
        }
    }
}

/// Current filing of a sender, so menus can show the state instead of offering
/// both "add" and "remove".
#[tauri::command]
pub async fn get_sender_preference(
    pool: tauri::State<'_, crate::db::ReadPool>,
    sender_email: String,
) -> CommandResult<SenderPreference> {
    SenderPreferenceRepository::new(pool.inner().as_ref())
        .get(&sender_email)
        .await
        .map(Into::into)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn set_thread_category(
    pool: tauri::State<'_, SqlitePool>,
    thread_id: String,
    category: Option<String>,
) -> CommandResult<()> {
    ThreadRepository::new(pool.inner())
        .set_category(&thread_id, category.as_deref())
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn set_thread_priority(
    pool: tauri::State<'_, SqlitePool>,
    input: SetThreadPriorityInput,
) -> CommandResult<()> {
    ThreadRepository::new(pool.inner())
        .set_priority(&input.thread_id, input.priority)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn set_sender_preference(
    pool: tauri::State<'_, SqlitePool>,
    input: SetSenderPreferenceInput,
) -> CommandResult<()> {
    SenderPreferenceRepository::new(pool.inner())
        .set(SenderPreferenceUpdate {
            account_id: input.account_id,
            sender_email: input.sender_email,
            category: input.category,
            is_priority: input.is_priority,
        })
        .await
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sender_preference_errors_are_stable_and_redacted() {
        assert_eq!(
            CommandError::from(SenderPreferenceError::CategoryInvalid).key,
            "backend.errors.senderPreferenceCategoryInvalid"
        );
        assert_eq!(
            CommandError::from(SenderPreferenceError::Persistence(sqlx::Error::Protocol(
                "private SQL detail".into()
            )))
            .key,
            "backend.errors.senderPreferencePersistenceFailed"
        );
    }

    #[test]
    fn nested_category_input_keeps_missing_and_null_distinct() {
        let missing: SetSenderPreferenceInput = serde_json::from_value(serde_json::json!({
            "account_id": "account",
            "sender_email": "sender@example.test"
        }))
        .expect("missing category");
        assert_eq!(missing.category, SenderPreferenceCategoryUpdate::Unchanged);

        let clear: SetSenderPreferenceInput = serde_json::from_value(serde_json::json!({
            "account_id": "account",
            "sender_email": "sender@example.test",
            "category": null
        }))
        .expect("null category");
        assert_eq!(clear.category, SenderPreferenceCategoryUpdate::Clear);
    }
}
