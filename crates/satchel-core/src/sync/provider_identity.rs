//! Stable local identities for provider data. Every input retains its byte
//! boundaries before hashing, including punctuation and empty components.

use sha2::{Digest, Sha256};

use crate::providers::traits::MessageEnvelope;

pub fn provider_local_id(kind: &str, provider: &str, fields: &[&str]) -> String {
    let provider = provider.to_ascii_lowercase();
    let mut hash = Sha256::new();
    for value in std::iter::once("satchel-provider-identity-v2")
        .chain(std::iter::once(kind))
        .chain(std::iter::once(provider.as_str()))
        .chain(fields.iter().copied())
    {
        hash.update((value.len() as u64).to_be_bytes());
        hash.update(value.as_bytes());
    }
    // The prefix is still read by legacy provider-specific projections. The
    // hash includes the unchanged provider value, so prefix escaping is cosmetic.
    let prefix = provider
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect::<String>();
    format!("{kind}-{prefix}-v2-{:x}", hash.finalize())
}

pub fn provider_message_db_id(
    provider: &str,
    account_id: &str,
    folder_remote_id: &str,
    message: &MessageEnvelope,
) -> String {
    let generation = message.remote_uidvalidity.unwrap_or_default().to_string();
    let generation = if provider.eq_ignore_ascii_case("imap") {
        generation.as_str()
    } else {
        ""
    };
    provider_local_id(
        "m",
        provider,
        &[account_id, folder_remote_id, generation, &message.remote_id],
    )
}

#[cfg(test)]
mod tests {
    use super::provider_local_id;

    #[test]
    fn punctuation_and_component_boundaries_are_part_of_the_identity() {
        assert_ne!(
            provider_local_id("m", "imap", &["a", "Projects/2026", "1"]),
            provider_local_id("m", "imap", &["a", "Projects-2026", "1"])
        );
        assert_ne!(
            provider_local_id("th", "imap", &["a", "<a.b@example.com>"]),
            provider_local_id("th", "imap", &["a", "<a-b@example.com>"])
        );
        assert_ne!(
            provider_local_id("m", "imap", &["a-b", "c"]),
            provider_local_id("m", "imap", &["a", "b-c"])
        );
        assert_ne!(
            provider_local_id("m", "imap", &["a", ""]),
            provider_local_id("m", "imap", &["a"])
        );
        assert_ne!(
            provider_local_id("m", "imap", &["a"]),
            provider_local_id("th", "imap", &["a"])
        );
        assert!(provider_local_id("m", "imap", &["a"]).starts_with("m-imap-"));
    }
}
