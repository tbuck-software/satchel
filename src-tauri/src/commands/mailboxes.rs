use serde::Serialize;

use crate::workflow::states::is_aside_folder;

use super::error::{CommandError, CommandResult};

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/mailboxes/")
)]
pub struct MailboxDto {
    pub id: String,
    pub account_id: String,
    pub account_email: String,
    pub name: String,
    pub mailbox_type: String,
    pub thread_count: u32,
    pub unread_count: u32,
}

#[tauri::command]
pub async fn list_mailboxes(
    pool: tauri::State<'_, crate::db::ReadPool>,
) -> CommandResult<Vec<MailboxDto>> {
    let rows = crate::db::mailboxes::list_mailboxes(pool.inner().as_ref())
        .await
        .map_err(CommandError::unexpected)?;
    Ok(rows
        .into_iter()
        .map(|row| MailboxDto {
            id: row.id,
            account_id: row.account_id,
            account_email: row.account_email,
            name: row.name.clone(),
            mailbox_type: classify_mailbox_type(&row.name, &row.folder_type).to_string(),
            thread_count: row.thread_count.max(0) as u32,
            unread_count: row.unread_count.max(0) as u32,
        })
        .collect())
}

fn classify_mailbox_type(name: &str, folder_type: &str) -> &'static str {
    let lower = name.to_ascii_lowercase();
    let stored = folder_type.to_ascii_lowercase();
    let inbox_descendant = lower.starts_with("inbox.") || lower.starts_with("inbox/");

    if stored == "aside" || is_aside_folder(name) {
        return "aside";
    }

    if lower.starts_with("category_") {
        return "custom";
    }

    if stored == "archive" || lower.contains("archive") || lower.contains("archiv") {
        "archive"
    } else if stored == "sent" || lower.contains("sent") {
        "sent"
    } else if stored == "drafts" || lower.contains("draft") {
        "drafts"
    } else if stored == "trash" || lower.contains("trash") || lower.contains("deleted") {
        "trash"
    } else if lower.contains("spam") || lower.contains("junk") {
        "spam"
    } else if lower.contains("notif") || lower.contains("updates") || lower.contains("forum") {
        "notifications"
    } else if lower.contains("blocked") {
        "blocked"
    } else if lower == "inbox" || (stored == "inbox" && !inbox_descendant) {
        "inbox"
    } else {
        "custom"
    }
}

#[cfg(test)]
mod tests {
    use super::classify_mailbox_type;

    #[test]
    fn legacy_inbox_descendants_are_not_primary_inbox() {
        assert_eq!(classify_mailbox_type("INBOX", "inbox"), "inbox");
        assert_eq!(classify_mailbox_type("Posteingang", "inbox"), "inbox");
        assert_eq!(classify_mailbox_type("INBOX.Clients", "inbox"), "custom");
        assert_eq!(classify_mailbox_type("INBOX.Blocked", "inbox"), "blocked");
        assert_eq!(classify_mailbox_type("Aside", "aside"), "aside");
    }
}
