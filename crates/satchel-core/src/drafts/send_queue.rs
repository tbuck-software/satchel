//! Persisted send snapshots used by draft restoration and the app outbox.
use super::outgoing::OutgoingMessageInput;
use crate::sync::queue::{QueuedAction, ACTION_SEND_MESSAGE};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendActionPayload {
    pub account_id: String,
    pub input: OutgoingMessageInput,
    /// RFC3339 timestamp at which to actually send. None means "immediately".
    #[serde(default)]
    pub scheduled_for: Option<String>,
    /// Stable Message-ID header — same value used as `messages.message_id_header`
    /// after persist. Generated once at action build time so retries reuse it.
    #[serde(default)]
    pub message_id_header: Option<String>,
}

pub fn build_send_action(
    account_id: String,
    input: OutgoingMessageInput,
    scheduled_for: Option<DateTime<Utc>>,
) -> Result<QueuedAction, String> {
    let message_id_header = format!("<{}@satchel.local>", uuid::Uuid::new_v4());
    let payload = SendActionPayload {
        account_id: account_id.clone(),
        input,
        scheduled_for: scheduled_for.map(|t| t.to_rfc3339()),
        message_id_header: Some(message_id_header),
    };
    let payload_value =
        serde_json::to_value(&payload).map_err(|err| format!("encode send payload: {err}"))?;

    let mut action = QueuedAction::new(account_id, ACTION_SEND_MESSAGE, payload_value);
    if let Some(when) = scheduled_for {
        action = action.scheduled(when);
    } else {
        action.status = "sending".into();
    }
    Ok(action)
}
