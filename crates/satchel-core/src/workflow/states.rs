use serde::{Deserialize, Serialize};

/// Provider-visible marker for Satchel's set-aside workflow. Gmail exposes it
/// as a label; IMAP and Outlook expose it as a regular folder.
pub const ASIDE_FOLDER_NAME: &str = "Aside";

pub fn is_aside_folder(value: &str) -> bool {
    let value = value.trim().to_ascii_lowercase();
    let name = ASIDE_FOLDER_NAME.to_ascii_lowercase();
    if value == name {
        return true;
    }

    // Some IMAP servers expose every user-created mailbox below the INBOX
    // namespace, e.g. `INBOX.Aside`. Keep the provider's complete path while
    // recognising the workflow by its leaf name. The single character after
    // INBOX is the hierarchy delimiter reported by LIST.
    value.strip_suffix(&name).is_some_and(|prefix| {
        prefix
            .strip_prefix("inbox")
            .is_some_and(|delimiter| delimiter.chars().count() == 1)
    })
}

#[cfg(test)]
mod tests {
    use super::{is_aside_folder, ASIDE_FOLDER_NAME};

    #[test]
    fn recognises_plain_and_inbox_scoped_names() {
        assert!(is_aside_folder(ASIDE_FOLDER_NAME));
        assert!(is_aside_folder("INBOX.Aside"));
        assert!(is_aside_folder("INBOX/Aside"));
        assert!(!is_aside_folder("Projects.Aside"));
        assert!(!is_aside_folder("Aside Later"));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "core/workflow/states/"))]
pub enum WorkflowState {
    Inbox,
    Todo,
    Waiting,
    Aside,
    Done,
    Snoozed,
}

impl WorkflowState {
    pub fn as_db_value(self) -> &'static str {
        match self {
            WorkflowState::Inbox => "inbox",
            WorkflowState::Todo => "todo",
            WorkflowState::Waiting => "waiting",
            WorkflowState::Aside => "aside",
            WorkflowState::Done => "done",
            WorkflowState::Snoozed => "snoozed",
        }
    }

    pub fn from_db_value(value: &str) -> Option<Self> {
        match value {
            "inbox" => Some(WorkflowState::Inbox),
            "todo" => Some(WorkflowState::Todo),
            "waiting" => Some(WorkflowState::Waiting),
            "aside" => Some(WorkflowState::Aside),
            "done" => Some(WorkflowState::Done),
            "snoozed" => Some(WorkflowState::Snoozed),
            _ => None,
        }
    }

    pub fn can_transition_to(self, next: WorkflowState) -> bool {
        if self == next {
            return true;
        }

        if next == WorkflowState::Inbox
            || next == WorkflowState::Aside
            || self == WorkflowState::Aside
        {
            return true;
        }

        matches!(
            (self, next),
            (WorkflowState::Inbox, WorkflowState::Todo)
                | (WorkflowState::Inbox, WorkflowState::Done)
                | (WorkflowState::Inbox, WorkflowState::Waiting)
                | (WorkflowState::Inbox, WorkflowState::Snoozed)
                | (WorkflowState::Todo, WorkflowState::Waiting)
                | (WorkflowState::Todo, WorkflowState::Done)
                | (WorkflowState::Todo, WorkflowState::Snoozed)
                | (WorkflowState::Waiting, WorkflowState::Todo)
                | (WorkflowState::Waiting, WorkflowState::Done)
                | (WorkflowState::Waiting, WorkflowState::Snoozed)
                | (WorkflowState::Snoozed, WorkflowState::Todo)
                | (WorkflowState::Snoozed, WorkflowState::Done)
                | (WorkflowState::Done, WorkflowState::Todo)
                | (WorkflowState::Done, WorkflowState::Snoozed)
        )
    }
}
