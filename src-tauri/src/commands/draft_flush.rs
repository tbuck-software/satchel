//! Draft acknowledgements use the IPC caller, never a claimed event sender.

#[cfg(desktop)]
use std::{
    collections::BTreeSet,
    sync::{Arc, LazyLock, Mutex},
};

#[cfg(desktop)]
pub(crate) struct PendingFlush {
    pending: BTreeSet<String>,
    pub(crate) progress: crate::DraftFlushRound,
}

#[cfg(desktop)]
static PENDING: LazyLock<Mutex<Option<Arc<Mutex<PendingFlush>>>>> = LazyLock::new(Mutex::default);

#[cfg(desktop)]
pub(crate) fn begin(windows: Vec<String>) -> Arc<Mutex<PendingFlush>> {
    let round = Arc::new(Mutex::new(PendingFlush {
        progress: crate::DraftFlushRound::new(windows.len()),
        pending: windows.into_iter().collect(),
    }));
    *PENDING.lock().expect("draft flush lock") = Some(round.clone());
    round
}

#[cfg(desktop)]
impl PendingFlush {
    fn acknowledge(&mut self, caller: &str, saved: bool) -> crate::DraftFlushStep {
        if !self.pending.remove(caller) {
            return crate::DraftFlushStep::Wait;
        }
        self.progress.acknowledge(saved)
    }
}

#[tauri::command]
pub fn acknowledge_draft_flush(window: tauri::WebviewWindow, saved: bool) {
    #[cfg(desktop)]
    {
        let round = PENDING.lock().expect("draft flush lock").clone();
        if let Some(round) = round {
            let step = round
                .lock()
                .expect("draft flush round lock")
                .acknowledge(window.label(), saved);
            crate::settle_draft_flush(window.app_handle(), step, Some(window.label()));
        }
    }
    #[cfg(mobile)]
    let _ = (window, saved);
}

#[cfg(desktop)]
use tauri::Manager;

#[cfg(all(test, desktop))]
mod tests {
    use crate::DraftFlushStep::{Abort, Exit, Wait};

    fn round() -> super::PendingFlush {
        super::PendingFlush {
            pending: std::collections::BTreeSet::from(["main".into(), "compose-1".into()]),
            progress: crate::DraftFlushRound::new(2),
        }
    }

    #[test]
    fn only_each_expected_window_can_acknowledge_once() {
        let mut round = round();
        assert_eq!(round.acknowledge("message-file-1", false), Wait);
        assert_eq!(round.acknowledge("tray-popover", true), Wait);
        assert_eq!(round.acknowledge("main", true), Wait);
        assert_eq!(round.acknowledge("main", true), Wait);
        assert_eq!(round.acknowledge("compose-1", true), Exit);
        assert_eq!(round.acknowledge("compose-1", true), Wait);
    }

    #[test]
    fn a_failed_save_from_an_expected_window_cancels_the_deadline() {
        let mut round = round();
        assert_eq!(round.acknowledge("compose-1", false), Abort);
        assert_eq!(round.progress.deadline(), Wait);
        assert_eq!(round.acknowledge("main", true), Wait);
    }
}
