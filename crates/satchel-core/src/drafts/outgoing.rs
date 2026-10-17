//! Shared composer snapshots and recipient normalization.
use crate::utils::mime::{decode_mime_encoded_words, extract_email_addresses, extract_mailboxes};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/messages/")
)]
pub struct OutgoingAttachmentInput {
    #[serde(
        deserialize_with = "crate::utils::filename::deserialize_filename",
        serialize_with = "crate::utils::filename::serialize_filename"
    )]
    pub filename: String,
    pub mime_type: String,
    /// Standard base64 (not URL-safe).
    pub content_base64: String,
    #[serde(default)]
    pub content_id: Option<String>,
    #[serde(default)]
    pub inline: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/messages/")
)]
pub struct OutgoingMessageInput {
    pub to: Vec<String>,
    #[serde(default)]
    pub cc: Vec<String>,
    #[serde(default)]
    pub bcc: Vec<String>,
    pub subject: String,
    pub body: String,
    #[serde(default)]
    pub body_html: Option<String>,
    #[serde(default)]
    pub thread_id: Option<String>,
    /// Local id of the message this one answers. The thread alone does not say
    /// which branch the user replied to, and the newest message in it is often
    /// not the one they had open.
    #[serde(default)]
    pub reply_to_message_id: Option<String>,
    #[serde(default)]
    pub attachments: Vec<OutgoingAttachmentInput>,
    /// Override the From-header (for aliases / send-as). `send_message` rejects
    /// anything that is neither the account's primary address nor one of its
    /// aliases. None means "use account primary address".
    #[serde(default)]
    pub from_address: Option<String>,
    #[serde(default)]
    pub from_name: Option<String>,
    #[serde(default)]
    pub reply_to: Option<String>,
    /// Local draft id; if set the draft row will be deleted on successful send.
    #[serde(default)]
    pub draft_id: Option<String>,
    /// RFC3339 timestamp at which to send (Send Later). None = send now.
    #[serde(default)]
    pub scheduled_for: Option<String>,
    /// RFC3339 timestamp for "remind me if no reply".
    #[serde(default)]
    pub follow_up_at: Option<String>,
    #[serde(default)]
    pub smime_sign_with_cert_id: Option<String>,
    #[serde(default)]
    pub smime_encrypt_to_recipients: bool,
    #[serde(default)]
    pub pgp_sign_with_key_id: Option<String>,
    #[serde(default)]
    pub pgp_encrypt_to_recipients: bool,
    /// "high" | "normal" | "low". Normal/None sends no header at all.
    #[serde(default)]
    pub priority: Option<String>,
    #[serde(default)]
    pub signature_id: Option<String>,
    #[serde(default)]
    pub signature_variant: Option<String>,
    /// Per-message edit of the resolved signature; replaces the stored body
    /// while keeping the signature's identity and inline assets.
    #[serde(default)]
    pub signature_override_text: Option<String>,
    #[serde(default)]
    pub signature_override_html: Option<String>,
}

/// Reduce recipient entries to the addresses they contain, in order.
///
/// An entry that holds no address is rejected instead of written into `To:`.
/// The old fallback wrote it through unchanged, so a chip cut at the comma of a
/// display name became a header line that reached nobody, without a warning.
pub fn normalize_recipients(values: &[String]) -> super::error::DraftResult<Vec<String>> {
    let mut recipients: Vec<String> = Vec::new();
    for value in values {
        let decoded = decode_mime_encoded_words(value.trim());
        if decoded.trim().is_empty() {
            continue;
        }
        let addresses = extract_email_addresses(&decoded);
        if addresses.is_empty() {
            return Err(super::DraftError::InvalidRecipient {
                value: value.trim().to_string(),
            });
        }
        for address in addresses {
            if !recipients.iter().any(|existing| existing == &address) {
                recipients.push(address);
            }
        }
    }
    Ok(recipients)
}

/// Display names from composer entries, keyed like
/// `OutgoingMessage::recipient_names`. The first name given for an address wins,
/// matching the first-wins dedupe in [`normalize_recipients`].
pub fn recipient_display_names(lists: &[&[String]]) -> std::collections::BTreeMap<String, String> {
    let mut names = std::collections::BTreeMap::new();
    for value in lists.iter().flat_map(|list| list.iter()) {
        for (name, address) in extract_mailboxes(&decode_mime_encoded_words(value.trim())) {
            if let Some(name) = name {
                names.entry(address.to_ascii_lowercase()).or_insert(name);
            }
        }
    }
    names
}
