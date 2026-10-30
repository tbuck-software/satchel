use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[cfg(test)]
use satchel_core::db::messages::MessageSearchError;

pub type CommandResult<T> = Result<T, CommandError>;

/// Error contract at a Tauri command boundary.
///
/// Rust chooses the message key and supplies interpolation values. The webview
/// owns the language and turns this object into the text shown to the user.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/error/"))]
pub struct CommandError {
    pub key: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, Value>,
}

impl CommandError {
    pub fn new(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            params: BTreeMap::new(),
        }
    }

    pub fn with_param(mut self, name: impl Into<String>, value: impl Into<Value>) -> Self {
        self.params.insert(name.into(), value.into());
        self
    }

    pub fn unexpected(error: impl ToString) -> Self {
        Self::new("backend.errors.unexpected").with_param("message", error.to_string())
    }

    /// Encode the same contract for persisted or nested string fields.
    ///
    /// Tauri serializes command errors itself, but queue and sync summaries
    /// still store errors as strings. The frontend decoder accepts this form as
    /// well, so those paths do not have to expose language-specific text.
    pub fn encoded(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| self.key.clone())
    }
}

impl fmt::Display for CommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.params.is_empty() {
            return formatter.write_str(&self.key);
        }
        write!(formatter, "{} {:?}", self.key, self.params)
    }
}

impl std::error::Error for CommandError {}

impl From<String> for CommandError {
    fn from(error: String) -> Self {
        if error.starts_with("backend.errors.") {
            Self::new(error)
        } else if let Ok(decoded) = serde_json::from_str::<Self>(&error) {
            if decoded.key.starts_with("backend.errors.") {
                return decoded;
            }
            Self::unexpected(error)
        } else {
            Self::unexpected(error)
        }
    }
}

impl From<&str> for CommandError {
    fn from(error: &str) -> Self {
        error.to_string().into()
    }
}

impl From<crate::sync::errors::SyncError> for CommandError {
    fn from(error: crate::sync::errors::SyncError) -> Self {
        error.to_string().into()
    }
}

impl From<satchel_core::drafts::DraftError> for CommandError {
    fn from(error: satchel_core::drafts::DraftError) -> Self {
        use satchel_core::drafts::DraftError;
        match error {
            DraftError::InvalidRecipient { value } => {
                Self::new("backend.errors.recipientAddressInvalid").with_param("value", value)
            }
            DraftError::Signature(error) => error.into(),
            DraftError::Message(error) => error.into(),
            DraftError::Thread(error) => error.into(),
            DraftError::Failure(error) => error.into(),
            other => Self::new(other.message_key()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CommandError, MessageSearchError};

    #[test]
    fn serializes_as_the_ipc_translation_contract() {
        let error = CommandError::new("backend.errors.folderExists")
            .with_param("path", "Archive/2026")
            .with_param("count", 4);

        assert_eq!(
            serde_json::to_value(error).unwrap(),
            serde_json::json!({
                "key": "backend.errors.folderExists",
                "params": { "count": 4, "path": "Archive/2026" }
            })
        );
    }

    #[test]
    fn folder_recovery_strings_preserve_keys_and_params_at_the_command_boundary() {
        for key in [
            "backend.errors.imapUidValidityMissing",
            "backend.errors.imapUidValidityChanged",
        ] {
            let encoded =
                serde_json::json!({"key": key, "params": {"folder": "Work/Drafts"}}).to_string();
            let error = CommandError::from(encoded);
            assert_eq!(error.key, key);
            assert_eq!(error.params["folder"], "Work/Drafts");
        }
        assert_eq!(
            CommandError::from("database is closed").key,
            "backend.errors.unexpected"
        );
    }

    #[test]
    fn omits_empty_params() {
        assert_eq!(
            serde_json::to_value(CommandError::new("backend.errors.inlineBackfillRunning"))
                .unwrap(),
            serde_json::json!({ "key": "backend.errors.inlineBackfillRunning" })
        );
    }

    #[test]
    fn preserves_a_domain_key_from_an_internal_string_result() {
        let error = CommandError::from("backend.errors.asideFolderMissing".to_string());

        assert_eq!(error.key, "backend.errors.asideFolderMissing");
        assert!(error.params.is_empty());
    }

    #[test]
    fn encodes_the_contract_for_string_fields() {
        let encoded = CommandError::new("backend.errors.asideFolderUnavailable")
            .with_param("account", "person@example.test")
            .encoded();

        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&encoded).unwrap(),
            serde_json::json!({
                "key": "backend.errors.asideFolderUnavailable",
                "params": { "account": "person@example.test" }
            })
        );
    }

    #[test]
    fn draft_conflicts_and_recipient_values_keep_the_command_contract() {
        use satchel_core::drafts::DraftError;
        assert_eq!(
            serde_json::to_value(CommandError::from(DraftError::ChangedElsewhere)).unwrap(),
            serde_json::json!({ "key": "backend.errors.draftChangedElsewhere" })
        );
        assert_eq!(
            serde_json::to_value(CommandError::from(DraftError::InvalidRecipient {
                value: "invalid chip".into(),
            }))
            .unwrap(),
            serde_json::json!({
                "key": "backend.errors.recipientAddressInvalid",
                "params": { "value": "invalid chip" }
            })
        );
    }

    #[test]
    fn draft_projection_errors_hide_sql_at_the_command_adapter() {
        use satchel_core::{db::threads::ThreadError, drafts::DraftError};
        let error = DraftError::Thread(ThreadError::Persistence(sqlx::Error::Protocol(
            "private draft SQL detail".into(),
        )));
        assert_eq!(
            serde_json::to_value(CommandError::from(error)).unwrap(),
            serde_json::json!({ "key": "backend.errors.threadPersistenceFailed" })
        );
    }

    #[test]
    fn hides_message_search_persistence_detail_at_ipc_boundary() {
        let detail = "near SELECT: syntax error";
        let error = CommandError::from(MessageSearchError::Persistence(sqlx::Error::Protocol(
            detail.into(),
        )));
        let serialized = serde_json::to_value(error).unwrap();

        assert_eq!(
            serialized,
            serde_json::json!({ "key": "backend.errors.messageSearchFailed" })
        );
        assert!(!serialized.to_string().contains(detail));
    }
}
