//! Delta Chat (chatmail) message headers.
//!
//! Delta Chat is chat over ordinary mail. What makes a message a chat message
//! is the `Chat-Version` header ("Incoming messages": it "MAY be used to detect
//! if a messages comes from a compatible messenger"; the Subject "MUST NOT be
//! used to detect compatible messengers, groups or whatever"). Groups carry a
//! `Chat-Group-ID` and `Chat-Group-Name`.
//!
//! Spec: <https://github.com/chatmail/core/blob/main/spec.md>, version 0.37.0.
use serde::{Deserialize, Serialize};

use crate::utils::mime::decode_mime_encoded_words;

/// The only version the spec defines ("Messengers MUST add a `Chat-Version:
/// 1.0` header to outgoing messages").
pub const CHAT_VERSION: &str = "1.0";

/// The chat headers of one message, as ingest stores them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatHeaders {
    pub version: String,
    /// Only a group id the spec allows; anything else is dropped.
    pub group_id: Option<String>,
    pub group_name: Option<String>,
}

/// Read the chat headers off a raw header block. `None` without `Chat-Version`.
pub fn parse_chat_headers(header_block: &str) -> Option<ChatHeaders> {
    chat_headers_from(|name| header_values(header_block, name).into_iter().next())
}

/// The same, for providers that hand headers over one by one (Gmail, Graph).
pub fn chat_headers_from(lookup: impl Fn(&str) -> Option<String>) -> Option<ChatHeaders> {
    let version = lookup("Chat-Version")?.trim().to_string();
    if version.is_empty() {
        return None;
    }
    let group_id = lookup("Chat-Group-ID")
        .map(|value| value.trim().to_string())
        .filter(|value| is_valid_group_id(value));
    let group_name = group_id
        .as_ref()
        .and_then(|_| lookup("Chat-Group-Name"))
        .map(|value| decode_mime_encoded_words(value.trim()))
        .filter(|value| !value.trim().is_empty());
    Some(ChatHeaders {
        version,
        group_id,
        group_name,
    })
}

/// "Groups": the group id "MUST be created only from the characters `0`-`9`,
/// `A`-`Z`, `a`-`z` `_` and `-`, MUST have a length of at least 11 characters
/// and no more than 32 characters."
pub fn is_valid_group_id(value: &str) -> bool {
    (11..=32).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

/// The Message-ID of an outgoing group message: "Outgoing group messages MUST
/// use `Gr.<group-id>.<unique data>`" ("Outgoing groups messages").
pub fn group_message_id(group_id: &str, domain: &str) -> String {
    let unique = uuid::Uuid::new_v4().simple().to_string();
    let domain = domain.trim();
    let domain = if domain.is_empty() {
        "satchel.local"
    } else {
        domain
    };
    format!("<Gr.{group_id}.{unique}@{domain}>")
}

/// The Subject of an outgoing chat message. One-to-one: "Message from <sender
/// name>"; a group uses its name, as the spec's own examples do
/// (`Chat: My Group: Hello group ...`).
pub fn chat_subject(sender: &str, group_name: Option<&str>, text: &str) -> String {
    match group_name.map(str::trim).filter(|name| !name.is_empty()) {
        Some(group) => {
            let excerpt = excerpt(text, 32);
            if excerpt.is_empty() {
                format!("Chat: {group}")
            } else {
                format!("Chat: {group}: {excerpt}")
            }
        }
        None => format!("Message from {}", sender.trim()),
    }
}

fn excerpt(text: &str, max_chars: usize) -> String {
    let first_line = text.lines().map(str::trim).find(|line| !line.is_empty());
    let Some(line) = first_line else {
        return String::new();
    };
    if line.chars().count() <= max_chars {
        return line.to_string();
    }
    let cut: String = line.chars().take(max_chars).collect();
    // End on a whole word when the line has one to end on.
    let cut = match cut.rfind(char::is_whitespace) {
        Some(space) if !line[cut.len()..].starts_with(char::is_whitespace) => &cut[..space],
        _ => cut.as_str(),
    };
    format!("{} ...", cut.trim_end())
}

/// What the outgoing message needs besides the usual headers.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/messages/", optional_fields = nullable)
)]
pub struct ChatSendOptions {
    #[serde(default)]
    pub group_id: Option<String>,
    #[serde(default)]
    pub group_name: Option<String>,
    /// The sender chose to send encrypted but unsigned because the own key
    /// stays locked. Without it a locked key holds the reply back.
    #[serde(default)]
    pub allow_unsigned: bool,
    /// The reply was end-to-end encrypted when the sender sent it. A retry
    /// whose plan no longer encrypts (a peer key or the own key gone) waits
    /// in the Outbox instead of going out in plain text.
    #[serde(default)]
    pub encryption_required: bool,
}

/// `Chat-Version`, and the group headers when the chat is a group. Values are
/// header text; the builder folds and encodes them.
pub fn chat_extra_headers(options: &ChatSendOptions) -> Vec<(String, String)> {
    let mut headers = vec![("Chat-Version".to_string(), CHAT_VERSION.to_string())];
    if let Some(group_id) = options
        .group_id
        .as_deref()
        .map(str::trim)
        .filter(|id| is_valid_group_id(id))
    {
        headers.push(("Chat-Group-ID".to_string(), group_id.to_string()));
        if let Some(name) = options
            .group_name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
        {
            headers.push(("Chat-Group-Name".to_string(), name.to_string()));
        }
    }
    headers
}

/// Whether a message may update Autocrypt peer state ("Updating Autocrypt
/// Peer State": ignore `multipart/report` and more than one `From` address).
pub fn autocrypt_eligible(from: Option<&str>, content_type: Option<&str>) -> bool {
    let report = content_type.is_some_and(|value| {
        value
            .trim()
            .to_ascii_lowercase()
            .starts_with("multipart/report")
    });
    let senders = from.map_or(0, |value| {
        crate::utils::mime::extract_email_addresses(value).len()
    });
    !report && senders <= 1
}

/// The same, read off a raw header block.
pub fn autocrypt_eligible_block(header_block: &str) -> bool {
    autocrypt_eligible(
        header_values(header_block, "From")
            .first()
            .map(String::as_str),
        header_values(header_block, "Content-Type")
            .first()
            .map(String::as_str),
    )
}

/// Every value of one header in a raw header block, unfolded, in order.
/// Header names compare case-insensitively.
pub fn header_values(header_block: &str, name: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut current: Option<String> = None;
    for raw_line in header_block.lines() {
        let line = raw_line.trim_end_matches('\r');
        if line.is_empty() {
            break;
        }
        if line.starts_with(' ') || line.starts_with('\t') {
            if let Some(value) = current.as_mut() {
                if !value.is_empty() {
                    value.push(' ');
                }
                value.push_str(line.trim());
            }
            continue;
        }
        if let Some(value) = current.take() {
            values.push(value);
        }
        if let Some((key, value)) = line.split_once(':') {
            if key.trim().eq_ignore_ascii_case(name) {
                current = Some(value.trim().to_string());
            }
        }
    }
    if let Some(value) = current {
        values.push(value);
    }
    values
}

#[cfg(test)]
mod tests {
    use super::*;

    const GROUP_HEADERS: &str = "From: Lena <lena@example.test>\r\n\
To: me@example.test, jonas@example.test\r\n\
Subject: Chat: Family: Photos\r\n\
Chat-Version: 1.0\r\n\
Chat-Group-ID: abcDEF123_-x\r\n\
Chat-Group-Name: =?utf-8?q?Famili=C3=A9?=\r\n\
\r\n";

    #[test]
    fn a_message_without_chat_version_is_mail() {
        assert_eq!(
            parse_chat_headers("From: a@example.test\r\nSubject: Chat: hi\r\n\r\n"),
            None
        );
    }

    #[test]
    fn reads_version_group_id_and_decoded_group_name() {
        assert_eq!(
            parse_chat_headers(GROUP_HEADERS),
            Some(ChatHeaders {
                version: "1.0".into(),
                group_id: Some("abcDEF123_-x".into()),
                group_name: Some("Familié".into()),
            })
        );
    }

    #[test]
    fn a_group_id_outside_the_spec_is_dropped_with_its_name() {
        let headers = "Chat-Version: 1.0\r\nChat-Group-ID: short\r\nChat-Group-Name: X\r\n\r\n";
        assert_eq!(
            parse_chat_headers(headers),
            Some(ChatHeaders {
                version: "1.0".into(),
                group_id: None,
                group_name: None,
            })
        );
        assert!(!is_valid_group_id("has space in it"));
        assert!(!is_valid_group_id(&"a".repeat(33)));
        assert!(is_valid_group_id(&"a".repeat(11)));
    }

    #[test]
    fn group_message_ids_carry_the_group_id() {
        let id = group_message_id("abcDEF123_-x", "example.test");
        assert!(id.starts_with("<Gr.abcDEF123_-x."));
        assert!(id.ends_with("@example.test>"));
    }

    #[test]
    fn subjects_follow_the_spec() {
        assert_eq!(chat_subject("Tim", None, "hi"), "Message from Tim");
        assert_eq!(
            chat_subject("Tim", Some("Family"), "Hello group"),
            "Chat: Family: Hello group"
        );
        assert_eq!(
            chat_subject("Tim", Some("Family"), &"word ".repeat(20)),
            "Chat: Family: word word word word word word ..."
        );
    }

    #[test]
    fn extra_headers_name_the_group_only_with_a_valid_id() {
        assert_eq!(
            chat_extra_headers(&ChatSendOptions::default()),
            vec![("Chat-Version".into(), "1.0".into())]
        );
        let group = chat_extra_headers(&ChatSendOptions {
            group_id: Some("abcDEF123_-x".into()),
            group_name: Some("Family".into()),
            ..Default::default()
        });
        assert_eq!(group.len(), 3);
        assert_eq!(group[1], ("Chat-Group-ID".into(), "abcDEF123_-x".into()));
        assert_eq!(group[2], ("Chat-Group-Name".into(), "Family".into()));
        let invalid = chat_extra_headers(&ChatSendOptions {
            group_id: Some("bad id".into()),
            group_name: Some("Family".into()),
            ..Default::default()
        });
        assert_eq!(invalid.len(), 1);
    }

    #[test]
    fn reports_and_mail_from_several_people_do_not_count_for_autocrypt() {
        assert!(autocrypt_eligible(
            Some("Lena <lena@example.test>"),
            Some("text/plain")
        ));
        assert!(!autocrypt_eligible(
            Some("lena@example.test, mia@example.test"),
            None
        ));
        assert!(!autocrypt_eligible(
            Some("mailer@example.test"),
            Some("multipart/report; report-type=delivery-status; boundary=x")
        ));
        assert!(autocrypt_eligible_block(
            "From: lena@example.test\r\nContent-Type: multipart/mixed; boundary=x\r\n\r\n"
        ));
        assert!(!autocrypt_eligible_block(
            "From: a@example.test, b@example.test\r\n\r\n"
        ));
    }

    #[test]
    fn header_values_collects_repeated_and_folded_headers() {
        let block = "Autocrypt: addr=a@example.test; keydata=AAAA\r\n BBBB\r\nX: y\r\nautocrypt: addr=b@example.test; keydata=CC\r\n\r\nAutocrypt: body\r\n";
        assert_eq!(
            header_values(block, "Autocrypt"),
            vec![
                "addr=a@example.test; keydata=AAAA BBBB".to_string(),
                "addr=b@example.test; keydata=CC".to_string()
            ]
        );
    }
}
