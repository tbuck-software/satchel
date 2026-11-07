//! Delta Chat–compatible chats (Labs → Chats).
//!
//! A chat is a mail thread whose messages carry `Chat-Version`. Reading it
//! uses the ordinary message commands; this module adds what the chat view
//! needs on top (group, members, whether the next message is encrypted) and
//! the quick reply, which goes through the normal outbox.
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tauri::AppHandle;

use crate::commands::error::{CommandError, CommandResult};
use crate::commands::messages::{OutgoingMessageInput, SendMessageResult};
use crate::AppState;
use satchel_core::autocrypt::{self, PreferEncrypt, Recommendation};
use satchel_core::chat::ChatSendOptions;
use satchel_core::db::autocrypt_peers::AutocryptPeerRepository;
use satchel_core::db::chats::{ChatError, ChatRepository, ChatThread};

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/chats/"))]
pub struct ChatMemberDto {
    pub address: String,
    pub name: Option<String>,
    /// Autocrypt recommendation for this member: `disable`, `discourage`,
    /// `available` or `encrypt`.
    pub encryption: String,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/chats/"))]
pub struct ChatMetaDto {
    pub thread_id: String,
    pub account_id: String,
    pub own_address: String,
    pub group_id: Option<String>,
    pub group_name: Option<String>,
    /// The other people in the chat, in header order.
    pub members: Vec<ChatMemberDto>,
    /// Whether this account has an OpenPGP key for `own_address`, which
    /// Autocrypt needs to announce and encrypt.
    pub own_key: bool,
    /// Whether the next message will be end-to-end encrypted.
    pub encrypts: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/chats/"))]
pub struct SendChatMessageInput {
    pub thread_id: String,
    pub text: String,
}

impl From<ChatError> for CommandError {
    fn from(error: ChatError) -> Self {
        match error {
            ChatError::NotFound => Self::new("backend.errors.threadNotFound"),
            ChatError::NotAChat => Self::new("backend.errors.chatNotAChat"),
            ChatError::Persistence(error) => {
                tracing::error!(error = ?error, "chat persistence failed");
                Self::new("backend.errors.threadPersistenceFailed")
            }
        }
    }
}

/// What Autocrypt allows for one outgoing chat message.
pub(crate) struct ChatEncryptionPlan {
    /// Our private key for the From address: id and armored key.
    pub own_key: Option<(String, String)>,
    pub recommendations: Vec<(String, Recommendation)>,
    /// The peers' keys, one per recipient, when every recipient has one.
    pub peer_keys: Vec<String>,
}

impl ChatEncryptionPlan {
    /// "encrypt" for every recipient, and a key of our own to encrypt to too.
    pub fn encrypts(&self) -> bool {
        self.own_key.is_some()
            && autocrypt::aggregate(
                &self
                    .recommendations
                    .iter()
                    .map(|(_, r)| *r)
                    .collect::<Vec<_>>(),
            ) == Recommendation::Encrypt
    }
}

/// Satchel announces `prefer-encrypt=mutual`, as Delta Chat does by default
/// (spec.md "Encryption": "`prefer-encrypt=mutual` MAY be set by default").
pub(crate) const OWN_PREFER_ENCRYPT: PreferEncrypt = PreferEncrypt::Mutual;

pub(crate) async fn chat_encryption_plan(
    pool: &SqlitePool,
    app: &AppHandle,
    account_id: &str,
    from_address: &str,
    recipients: &[String],
) -> Result<ChatEncryptionPlan, String> {
    let own_key = crate::commands::pgp::own_private_key_for(pool, app, from_address).await?;
    let peers = AutocryptPeerRepository::new(pool);
    let mut recommendations = Vec::with_capacity(recipients.len());
    let mut peer_keys = Vec::with_capacity(recipients.len());
    for recipient in recipients {
        let peer = peers
            .get(account_id, recipient)
            .await
            .map_err(|error| error.to_string())?;
        recommendations.push((
            recipient.clone(),
            autocrypt::recommend(peer.as_ref(), OWN_PREFER_ENCRYPT, false),
        ));
        if let Some(key) = peer.and_then(|peer| peer.public_key) {
            peer_keys.push(key);
        }
    }
    Ok(ChatEncryptionPlan {
        own_key,
        recommendations,
        peer_keys,
    })
}

fn meta_dto(chat: ChatThread, plan: &ChatEncryptionPlan) -> ChatMetaDto {
    let members = chat
        .recipients
        .iter()
        .map(|address| ChatMemberDto {
            address: address.clone(),
            name: chat.names.get(address).cloned(),
            encryption: plan
                .recommendations
                .iter()
                .find(|(addr, _)| addr == address)
                .map(|(_, r)| r.as_str())
                .unwrap_or(Recommendation::Disable.as_str())
                .to_string(),
        })
        .collect();
    ChatMetaDto {
        thread_id: chat.thread_id,
        account_id: chat.account_id,
        own_address: chat.own_address,
        group_id: chat.group_id,
        group_name: chat.group_name,
        members,
        own_key: plan.own_key.is_some(),
        encrypts: plan.encrypts(),
    }
}

#[tauri::command]
pub async fn get_chat_meta(
    app: AppHandle,
    pool: tauri::State<'_, SqlitePool>,
    thread_id: String,
) -> CommandResult<ChatMetaDto> {
    let chat = ChatRepository::new(pool.inner()).thread(&thread_id).await?;
    let plan = chat_encryption_plan(
        pool.inner(),
        &app,
        &chat.account_id,
        &chat.own_address,
        &chat.recipients,
    )
    .await
    .map_err(CommandError::unexpected)?;
    Ok(meta_dto(chat, &plan))
}

/// The outgoing message for a quick reply in `chat`.
fn chat_reply_input(chat: &ChatThread, text: &str) -> OutgoingMessageInput {
    let sender = if chat.account_name.trim().is_empty() {
        chat.own_address.as_str()
    } else {
        chat.account_name.as_str()
    };
    OutgoingMessageInput {
        to: chat.recipients.clone(),
        subject: satchel_core::chat::chat_subject(sender, chat.group_name.as_deref(), text),
        body: text.to_string(),
        thread_id: Some(chat.thread_id.clone()),
        reply_to_message_id: chat.reply_to_message_id.clone(),
        from_address: Some(chat.own_address.clone()),
        chat: Some(ChatSendOptions {
            group_id: chat.group_id.clone(),
            group_name: chat.group_name.clone(),
        }),
        ..Default::default()
    }
}

/// Send a chat line now: no Undo window, no signature, plain text. The
/// outbox adds the chat headers and Autocrypt (`sync::outbox`).
#[tauri::command]
pub async fn send_chat_message(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    pool: tauri::State<'_, SqlitePool>,
    input: SendChatMessageInput,
) -> CommandResult<SendMessageResult> {
    let text = input.text.trim_end().to_string();
    if text.trim().is_empty() {
        // The reply field sends nothing empty; reaching this is a bug.
        return Err(CommandError::new("backend.errors.unexpected"));
    }
    let chat = ChatRepository::new(pool.inner())
        .thread(&input.thread_id)
        .await?;
    if chat.recipients.is_empty() {
        return Err(CommandError::new("backend.errors.chatNoRecipient"));
    }
    let outgoing = chat_reply_input(&chat, &text);
    crate::commands::messages::send_message_inner(
        app,
        state.inner(),
        pool.inner(),
        chat.account_id.clone(),
        outgoing,
        crate::commands::messages::SendMode::Immediate,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn chat(group: Option<(&str, &str)>) -> ChatThread {
        ChatThread {
            thread_id: "th".into(),
            account_id: "acc".into(),
            account_name: "Tom".into(),
            own_address: "tom@example.test".into(),
            group_id: group.map(|g| g.0.to_string()),
            group_name: group.map(|g| g.1.to_string()),
            recipients: vec!["lena@example.test".into(), "jonas@example.test".into()],
            names: BTreeMap::from([("lena@example.test".to_string(), "Lena".to_string())]),
            reply_to_message_id: Some("m-last".into()),
        }
    }

    #[test]
    fn a_reply_answers_the_latest_message_with_chat_headers() {
        let input = chat_reply_input(&chat(Some(("abcDEF123_-x", "Family"))), "Hello group");
        assert_eq!(input.to, vec!["lena@example.test", "jonas@example.test"]);
        assert_eq!(input.subject, "Chat: Family: Hello group");
        assert_eq!(input.reply_to_message_id.as_deref(), Some("m-last"));
        assert_eq!(input.thread_id.as_deref(), Some("th"));
        assert_eq!(input.from_address.as_deref(), Some("tom@example.test"));
        assert_eq!(
            input.chat.as_ref().and_then(|c| c.group_id.as_deref()),
            Some("abcDEF123_-x")
        );
        assert!(input.body_html.is_none());
        assert!(input.signature_id.is_none());

        let one = chat_reply_input(&chat(None), "hi");
        assert_eq!(one.subject, "Message from Tom");
        assert!(one.chat.as_ref().is_some_and(|c| c.group_id.is_none()));
    }

    #[test]
    fn meta_reports_each_member_and_encrypts_only_with_every_key() {
        let plan = ChatEncryptionPlan {
            own_key: Some(("k".into(), "armored".into())),
            recommendations: vec![
                ("lena@example.test".into(), Recommendation::Encrypt),
                ("jonas@example.test".into(), Recommendation::Disable),
            ],
            peer_keys: vec!["lena-key".into()],
        };
        let dto = meta_dto(chat(None), &plan);
        assert_eq!(dto.members[0].name.as_deref(), Some("Lena"));
        assert_eq!(dto.members[0].encryption, "encrypt");
        assert_eq!(dto.members[1].encryption, "disable");
        assert!(dto.own_key);
        assert!(!dto.encrypts);

        let all = ChatEncryptionPlan {
            recommendations: vec![
                ("lena@example.test".into(), Recommendation::Encrypt),
                ("jonas@example.test".into(), Recommendation::Encrypt),
            ],
            ..plan
        };
        assert!(all.encrypts());
        let no_own_key = ChatEncryptionPlan {
            own_key: None,
            ..all
        };
        assert!(!no_own_key.encrypts());
    }
}
