use crate::db::{messages::SentMessageError, signatures::SignatureError, threads::ThreadError};

pub type DraftResult<T> = Result<T, DraftError>;

/// Draft editing failures, independent of command transport and UI state.
#[derive(Debug, thiserror::Error)]
pub enum DraftError {
    #[error("backend.errors.signatureVariantInvalid")]
    SignatureVariantInvalid,
    #[error("backend.errors.signatureNotFound")]
    SignatureNotFound,
    #[error("backend.errors.draftGoneElsewhere")]
    GoneElsewhere,
    #[error("backend.errors.draftChangedElsewhere")]
    ChangedElsewhere,
    #[error("backend.errors.draftSendPending")]
    SendPending,
    #[error("backend.errors.outboxActionCannotBeDiscarded")]
    OutboxActionCannotBeDiscarded,
    #[error("backend.errors.queuedDraftNotRestorable")]
    QueuedDraftNotRestorable,
    #[error("backend.errors.recipientAddressInvalid")]
    InvalidRecipient { value: String },
    #[error(transparent)]
    Signature(#[from] SignatureError),
    #[error(transparent)]
    Message(#[from] SentMessageError),
    #[error(transparent)]
    Thread(#[from] ThreadError),
    #[error("{0}")]
    Failure(String),
}

impl DraftError {
    pub(super) fn unexpected(error: impl ToString) -> Self {
        Self::Failure(error.to_string())
    }

    pub fn message_key(&self) -> &str {
        match self {
            Self::SignatureVariantInvalid => "backend.errors.signatureVariantInvalid",
            Self::SignatureNotFound => "backend.errors.signatureNotFound",
            Self::GoneElsewhere => "backend.errors.draftGoneElsewhere",
            Self::ChangedElsewhere => "backend.errors.draftChangedElsewhere",
            Self::SendPending => "backend.errors.draftSendPending",
            Self::OutboxActionCannotBeDiscarded => "backend.errors.outboxActionCannotBeDiscarded",
            Self::QueuedDraftNotRestorable => "backend.errors.queuedDraftNotRestorable",
            Self::InvalidRecipient { .. } => "backend.errors.recipientAddressInvalid",
            Self::Signature(error) => match error {
                SignatureError::NameEmpty => "backend.errors.signatureNameRequired",
                SignatureError::NotFoundForAccount => "backend.errors.signatureNotFound",
                SignatureError::OwnedByAnotherAccount => "backend.errors.signatureAccountMismatch",
                SignatureError::InvalidFirstContactMode => {
                    "backend.errors.signatureFirstContactModeInvalid"
                }
                SignatureError::InvalidSuggestionStatus => {
                    "backend.errors.signatureSuggestionStatusInvalid"
                }
                SignatureError::Persistence(_) => "backend.errors.signaturePersistenceFailed",
            },
            Self::Message(error) => match error {
                SentMessageError::Persistence(_) => "backend.errors.sentMessagePersistenceFailed",
                SentMessageError::ReplyContextLoad(_) => "backend.errors.replyContextLoadFailed",
                SentMessageError::ReplyThreadNotFound { .. } => {
                    "backend.errors.replyThreadNotFound"
                }
            },
            Self::Thread(_) => "backend.errors.threadPersistenceFailed",
            Self::Failure(message) if message.starts_with("backend.errors.") => message,
            Self::Failure(_) => "backend.errors.unexpected",
        }
    }

    // Queue failures are persisted strings, preserving the existing message-key
    // contract for errors which carry interpolation values.
    pub(super) fn encoded(&self) -> String {
        match self {
            Self::InvalidRecipient { value } => serde_json::json!({
                "key": self.message_key(), "params": { "value": value }
            })
            .to_string(),
            Self::Failure(message) if !message.starts_with("backend.errors.") => {
                serde_json::json!({
                    "key": "backend.errors.unexpected", "params": { "message": message }
                })
                .to_string()
            }
            _ => serde_json::json!({ "key": self.message_key() }).to_string(),
        }
    }
}

impl From<String> for DraftError {
    fn from(error: String) -> Self {
        Self::Failure(error)
    }
}
