//! Tasks: local lists that need no account, and remote lists synced through
//! a [`sync::TaskRemote`] (CalDAV VTODO today).
//!
//! Validation lives here so every caller (commands, the hub later) applies
//! the same rules; persistence is `crate::db::tasks`, the two-way sync is
//! [`sync`], and the iCalendar mapping is [`vtodo`].
use crate::db::tasks::TaskWrite;

pub mod recurrence;
pub mod sync;
pub mod vtodo;

pub use crate::db::tasks::{TaskError, TaskListRecord, TaskRecord, TaskRepository};

/// What the editor sends.
#[derive(Debug, Clone, Default)]
pub struct TaskInput {
    pub title: String,
    pub notes: Option<String>,
    pub due_at: Option<String>,
    pub reminder_at: Option<String>,
    pub priority: u8,
    pub completed_at: Option<String>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TaskInputError {
    #[error("task title is required")]
    TitleRequired,
    #[error("task due date is invalid")]
    DueInvalid,
    #[error("task reminder is invalid")]
    ReminderInvalid,
    #[error("task priority is out of range")]
    PriorityInvalid,
    #[error("task completion time is invalid")]
    CompletedInvalid,
}

impl TaskInputError {
    pub fn key(&self) -> &'static str {
        match self {
            Self::TitleRequired => "backend.errors.taskTitleRequired",
            Self::DueInvalid => "backend.errors.taskDueInvalid",
            Self::ReminderInvalid => "backend.errors.taskReminderInvalid",
            Self::PriorityInvalid => "backend.errors.taskPriorityInvalid",
            Self::CompletedInvalid => "backend.errors.taskCompletedInvalid",
        }
    }
}

fn utc_instant(value: &str) -> Option<String> {
    chrono::DateTime::parse_from_rfc3339(value.trim())
        .ok()
        .map(|instant| {
            instant
                .with_timezone(&chrono::Utc)
                .format("%Y-%m-%dT%H:%M:%SZ")
                .to_string()
        })
}

fn non_empty(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Normalize editor input: a trimmed title, a due date as `YYYY-MM-DD` or a
/// UTC instant, reminder and completion as UTC instants.
pub fn validate(input: TaskInput) -> Result<TaskWrite, TaskInputError> {
    let title = input.title.trim().to_string();
    if title.is_empty() {
        return Err(TaskInputError::TitleRequired);
    }
    let due_at = match non_empty(input.due_at) {
        None => None,
        Some(value) if value.len() == 10 => Some(
            chrono::NaiveDate::parse_from_str(&value, "%Y-%m-%d")
                .map_err(|_| TaskInputError::DueInvalid)?
                .format("%Y-%m-%d")
                .to_string(),
        ),
        Some(value) => Some(utc_instant(&value).ok_or(TaskInputError::DueInvalid)?),
    };
    let reminder_at = match non_empty(input.reminder_at) {
        None => None,
        Some(value) => Some(utc_instant(&value).ok_or(TaskInputError::ReminderInvalid)?),
    };
    let completed_at = match non_empty(input.completed_at) {
        None => None,
        Some(value) => Some(utc_instant(&value).ok_or(TaskInputError::CompletedInvalid)?),
    };
    if input.priority > 9 {
        return Err(TaskInputError::PriorityInvalid);
    }
    Ok(TaskWrite {
        title,
        notes: non_empty(input.notes),
        due_at,
        reminder_at,
        priority: input.priority,
        completed_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_editor_input() {
        let write = validate(TaskInput {
            title: "  Call Anna  ".into(),
            notes: Some("   ".into()),
            due_at: Some("2026-11-10T17:00:00+01:00".into()),
            reminder_at: Some("2026-11-10T08:00:00.000Z".into()),
            priority: 1,
            completed_at: None,
        })
        .unwrap();
        assert_eq!(write.title, "Call Anna");
        assert_eq!(write.notes, None);
        assert_eq!(write.due_at.as_deref(), Some("2026-11-10T16:00:00Z"));
        assert_eq!(write.reminder_at.as_deref(), Some("2026-11-10T08:00:00Z"));

        let date = validate(TaskInput {
            title: "x".into(),
            due_at: Some("2026-02-28".into()),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(date.due_at.as_deref(), Some("2026-02-28"));
    }

    #[test]
    fn rejects_what_cannot_be_stored() {
        let base = TaskInput {
            title: "x".into(),
            ..Default::default()
        };
        assert_eq!(
            validate(TaskInput {
                title: " ".into(),
                ..Default::default()
            })
            .unwrap_err(),
            TaskInputError::TitleRequired
        );
        assert_eq!(
            validate(TaskInput {
                due_at: Some("2026-02-30".into()),
                ..base.clone()
            })
            .unwrap_err(),
            TaskInputError::DueInvalid
        );
        assert_eq!(
            validate(TaskInput {
                reminder_at: Some("tomorrow".into()),
                ..base.clone()
            })
            .unwrap_err(),
            TaskInputError::ReminderInvalid
        );
        assert_eq!(
            validate(TaskInput {
                completed_at: Some("yesterday".into()),
                ..base.clone()
            })
            .unwrap_err(),
            TaskInputError::CompletedInvalid
        );
        assert_eq!(
            validate(TaskInput {
                priority: 10,
                ..base
            })
            .unwrap_err(),
            TaskInputError::PriorityInvalid
        );
    }
}
