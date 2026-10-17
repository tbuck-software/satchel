use std::collections::HashSet;

use chrono::{DateTime, Duration, Utc};

#[derive(Debug, Clone)]
pub struct ThreadCandidate {
    pub id: String,
    pub normalized_subject: String,
    pub message_ids: HashSet<String>,
    pub last_message_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct IncomingMessage {
    pub message_id: String,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub subject: String,
    pub received_at: DateTime<Utc>,
}

pub use crate::email::subject::subject_key as normalize_subject;

pub fn assign_thread(message: &IncomingMessage, existing: &[ThreadCandidate]) -> Option<String> {
    if let Some(in_reply_to) = &message.in_reply_to {
        if let Some(found) = existing
            .iter()
            .find(|thread| thread.message_ids.contains(in_reply_to))
        {
            return Some(found.id.clone());
        }
    }

    for reference in message.references.iter().rev() {
        if let Some(found) = existing
            .iter()
            .find(|thread| thread.message_ids.contains(reference))
        {
            return Some(found.id.clone());
        }
    }

    let normalized = normalize_subject(&message.subject);
    existing
        .iter()
        .find(|thread| {
            thread.normalized_subject == normalized
                && (message.received_at - thread.last_message_at)
                    .num_days()
                    .abs()
                    <= Duration::days(30).num_days()
        })
        .map(|thread| thread.id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_common_reply_prefix() {
        assert_eq!(normalize_subject("Re: AW: WG: Test"), "test");
    }
}
