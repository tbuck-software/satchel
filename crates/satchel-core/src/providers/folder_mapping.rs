use crate::workflow::states::is_aside_folder;

/// Map a provider folder to Satchel's semantic folder type.
pub fn provider_folder_type(
    provider: &str,
    name: &str,
    remote_id: &str,
    kind: Option<&str>,
) -> &'static str {
    if is_aside_folder(name) {
        return "aside";
    }
    if let Some(kind) = kind.map(str::trim).filter(|value| !value.is_empty()) {
        match kind.to_ascii_lowercase().as_str() {
            "inbox" => return "inbox",
            "sent" | "sentitems" => return "sent",
            "draft" | "drafts" => return "drafts",
            "trash" | "deleted" | "deleteditems" => return "trash",
            "archive" => return "archive",
            "spam" | "junk" | "junkemail" => return "spam",
            _ => {}
        }
    }

    let lower = name.to_ascii_lowercase();
    let remote = remote_id.trim().to_ascii_lowercase();
    let provider = provider.trim().to_ascii_lowercase();
    if provider == "gmail" {
        match remote.as_str() {
            "inbox" => return "inbox",
            "sent" => return "sent",
            "draft" | "drafts" => return "drafts",
            "trash" => return "trash",
            "spam" => return "spam",
            "archive" => return "archive",
            _ => {}
        }
    }
    if provider == "outlook" {
        match remote.as_str() {
            "inbox" => return "inbox",
            "sentitems" => return "sent",
            "drafts" => return "drafts",
            "deleteditems" => return "trash",
            "archive" => return "archive",
            "junkemail" => return "spam",
            _ => {}
        }
    }

    if lower.contains("archive") || lower.contains("archiv") {
        "archive"
    } else if lower.contains("sent") || lower.contains("gesendet") {
        "sent"
    } else if lower.contains("draft") || lower.contains("entwurf") || lower.contains("entwürf") {
        "drafts"
    } else if lower.contains("trash")
        || lower.contains("deleted")
        || lower.contains("papierkorb")
        || lower.contains("gelöscht")
        || lower.contains("geloescht")
    {
        "trash"
    } else if lower.contains("spam") || lower.contains("junk") {
        "spam"
    } else if lower == "inbox" || lower == "posteingang" {
        "inbox"
    } else {
        "custom"
    }
}

/// Return the provider-specific fallback identifier for a semantic folder type.
pub fn default_remote_folder(provider: &str, folder_type: &str) -> &'static str {
    match (provider, folder_type) {
        ("gmail", "archive") => "ARCHIVE",
        ("gmail", "trash") => "TRASH",
        ("gmail", "spam") => "SPAM",
        ("outlook", "trash") => "deleteditems",
        ("outlook", "spam") => "junkemail",
        (_, "trash") => "Trash",
        (_, "spam") => "Spam",
        (_, "archive") => "Archive",
        _ => "INBOX",
    }
}

/// Return the remote identifier used when provisioning a local move target.
///
/// Outlook's existing archive provisioning path uses the lowercase well-known
/// id, while remote mutation fallback historically uses the display-path form.
/// Keep that distinction explicit until those persisted target semantics can be
/// migrated independently.
pub fn provisioned_remote_folder(provider: &str, folder_type: &str) -> &'static str {
    match (provider, folder_type) {
        ("outlook", "archive") => "archive",
        _ => default_remote_folder(provider, folder_type),
    }
}

#[cfg(test)]
mod tests {
    use super::{default_remote_folder, provider_folder_type, provisioned_remote_folder};

    #[test]
    fn inbound_folder_roles_cover_kind_provider_id_and_name_fallbacks() {
        for (kind, expected) in [
            ("inbox", "inbox"),
            ("sentitems", "sent"),
            ("drafts", "drafts"),
            ("deleteditems", "trash"),
            ("archive", "archive"),
            ("junkemail", "spam"),
        ] {
            assert_eq!(
                provider_folder_type("imap", "arbitrary", "arbitrary", Some(kind)),
                expected
            );
        }

        for (provider, remote_id, expected) in [
            ("gmail", "INBOX", "inbox"),
            ("gmail", "SENT", "sent"),
            ("gmail", "DRAFT", "drafts"),
            ("gmail", "TRASH", "trash"),
            ("gmail", "SPAM", "spam"),
            ("gmail", "ARCHIVE", "archive"),
            ("outlook", "inbox", "inbox"),
            ("outlook", "sentitems", "sent"),
            ("outlook", "drafts", "drafts"),
            ("outlook", "deleteditems", "trash"),
            ("outlook", "archive", "archive"),
            ("outlook", "junkemail", "spam"),
        ] {
            assert_eq!(
                provider_folder_type(provider, "arbitrary", remote_id, None),
                expected
            );
        }

        assert_eq!(
            provider_folder_type("imap", "INBOX.Archive", "INBOX.Archive", None),
            "archive"
        );
        assert_eq!(
            provider_folder_type("gmail", "Aside", "Label_123", Some("user")),
            "aside"
        );
        assert_eq!(
            provider_folder_type("imap", "Projects", "Projects", None),
            "custom"
        );
    }

    #[test]
    fn outbound_folder_defaults_keep_provider_specific_ids() {
        for (provider, folder_type, expected) in [
            ("gmail", "archive", "ARCHIVE"),
            ("gmail", "trash", "TRASH"),
            ("gmail", "spam", "SPAM"),
            ("outlook", "trash", "deleteditems"),
            ("outlook", "spam", "junkemail"),
            ("imap", "trash", "Trash"),
            ("imap", "spam", "Spam"),
            ("imap", "archive", "Archive"),
            ("imap", "inbox", "INBOX"),
        ] {
            assert_eq!(default_remote_folder(provider, folder_type), expected);
        }

        assert_eq!(default_remote_folder("outlook", "archive"), "Archive");
        assert_eq!(provisioned_remote_folder("outlook", "archive"), "archive");
    }
}
