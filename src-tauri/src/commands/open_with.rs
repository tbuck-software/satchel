//! The backend-owned queue for "Open with Satchel" invitations.

use crate::AppState;

/// OS-provided invitation paths. Events only wake the main window to drain this queue.
#[derive(Default)]
pub struct PendingOpenWith {
    paths: std::sync::Mutex<Vec<String>>,
}

impl PendingOpenWith {
    /// A launch command line carries a handful of paths at most. The cap is
    /// what keeps a buffer nobody ever drains from growing.
    const CAPACITY: usize = 8;

    pub fn push(&self, path: String) {
        let Ok(mut paths) = self.paths.lock() else {
            return;
        };
        if paths.len() >= Self::CAPACITY {
            return;
        }
        paths.push(path);
    }

    /// Hand over what is buffered, exactly once per path.
    pub fn take(&self) -> Vec<String> {
        self.paths
            .lock()
            .map(|mut paths| std::mem::take(&mut *paths))
            .unwrap_or_default()
    }
}

/// Take OS-provided paths from the main window, exactly once.
#[tauri::command]
pub fn take_pending_open_with(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, AppState>,
) -> Vec<String> {
    if window.label() != "main" {
        return Vec::new();
    }
    state.pending_open_with.take()
}

#[cfg(test)]
mod tests {
    use super::PendingOpenWith;

    #[test]
    fn a_launch_hands_its_paths_over_once_and_in_order() {
        let buffer = PendingOpenWith::default();
        buffer.push("first.ics".to_string());
        buffer.push("second.ics".to_string());

        assert_eq!(buffer.take(), vec!["first.ics", "second.ics"]);
        assert!(buffer.take().is_empty());
    }

    #[test]
    fn events_after_startup_are_buffered_until_the_main_window_takes_them() {
        let buffer = PendingOpenWith::default();
        assert!(buffer.take().is_empty());

        // A later signal also drains this backend-owned queue.
        buffer.push("later.ics".to_string());
        assert_eq!(buffer.take(), vec!["later.ics"]);
    }

    #[test]
    fn an_undrained_buffer_stops_growing() {
        let buffer = PendingOpenWith::default();
        for index in 0..(PendingOpenWith::CAPACITY + 5) {
            buffer.push(format!("{index}.ics"));
        }

        assert_eq!(buffer.take().len(), PendingOpenWith::CAPACITY);
    }
}
