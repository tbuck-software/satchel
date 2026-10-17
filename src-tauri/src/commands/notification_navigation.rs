//! Retain a notification click until the main frontend has installed its listener.

use std::sync::Mutex;
use tauri::Emitter;

#[derive(Default)]
struct PendingNavigation {
    ready: bool,
    thread_id: Option<String>,
}

impl PendingNavigation {
    fn publish(&mut self, thread_id: String) -> Option<String> {
        if self.ready {
            Some(thread_id)
        } else {
            self.thread_id = Some(thread_id);
            None
        }
    }

    fn take(&mut self) -> Option<String> {
        self.ready = true;
        self.thread_id.take()
    }
}

static NAVIGATION: Mutex<PendingNavigation> = Mutex::new(PendingNavigation {
    ready: false,
    thread_id: None,
});

pub fn focus_thread(app: &tauri::AppHandle, thread_id: String) {
    let live = NAVIGATION
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .publish(thread_id);
    if let Some(thread_id) = live {
        let _ = app.emit(
            "mail:focus-thread",
            serde_json::json!({ "thread_id": thread_id }),
        );
    }
}

/// Consume a cold-start click and switch subsequent clicks to the live event feed.
#[tauri::command]
pub fn take_pending_notification_thread() -> Option<String> {
    NAVIGATION
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .take()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cold_start_click_is_taken_once_then_clicks_go_live() {
        let mut navigation = PendingNavigation::default();
        assert_eq!(navigation.publish("launch".into()), None);
        assert_eq!(navigation.take().as_deref(), Some("launch"));
        assert_eq!(navigation.take(), None);
        assert_eq!(navigation.publish("live".into()).as_deref(), Some("live"));
        assert_eq!(navigation.take(), None);
    }

    #[test]
    fn latest_click_before_readiness_wins() {
        let mut navigation = PendingNavigation::default();
        navigation.publish("first".into());
        navigation.publish("second".into());
        assert_eq!(navigation.take().as_deref(), Some("second"));
    }
}
