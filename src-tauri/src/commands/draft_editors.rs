//! Which window edits which draft.
//!
//! One editor per draft id. A second window asking for a draft that is already
//! open gets the first one focused instead, so two composers never race each
//! other's saves. `save_draft`'s revision check catches whatever slips past.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use tauri::{AppHandle, Manager, WebviewWindow};

use crate::commands::error::CommandResult;

#[derive(Debug, Default)]
pub(crate) struct DraftEditors {
    /// Window label to the draft its composer holds. A window holds at most one.
    by_window: HashMap<String, String>,
}

impl DraftEditors {
    /// The other live window that edits `draft_id`. Entries of closed windows
    /// are dropped on the way.
    pub(crate) fn owner_besides(
        &mut self,
        draft_id: &str,
        window: &str,
        is_live: impl Fn(&str) -> bool,
    ) -> Option<String> {
        self.by_window.retain(|label, _| is_live(label));
        self.by_window
            .iter()
            .find(|(label, draft)| draft.as_str() == draft_id && label.as_str() != window)
            .map(|(label, _)| label.clone())
    }

    pub(crate) fn holds(&self, window: &str, draft_id: &str) -> bool {
        self.by_window.get(window).map(String::as_str) == Some(draft_id)
    }

    /// Records that `window` now edits `draft_id`. Returns the window that
    /// already owns the draft when the claim is refused.
    pub(crate) fn claim(
        &mut self,
        window: &str,
        draft_id: &str,
        is_live: impl Fn(&str) -> bool,
    ) -> Option<String> {
        if let Some(owner) = self.owner_besides(draft_id, window, is_live) {
            return Some(owner);
        }
        self.by_window
            .insert(window.to_string(), draft_id.to_string());
        None
    }

    /// Drops `window`'s claim, unless it has moved on to another draft since:
    /// an outgoing composer can unmount after its successor claimed.
    pub(crate) fn release(&mut self, window: &str, draft_id: &str) {
        if self.holds(window, draft_id) {
            self.by_window.remove(window);
        }
    }

    /// Hands `draft_id` to a newly built window, taking it from `from` if that
    /// window held it (a composer popping itself out).
    pub(crate) fn hand_over(&mut self, from: &str, to: &str, draft_id: &str) {
        if self.holds(from, draft_id) {
            self.by_window.remove(from);
        }
        self.by_window.insert(to.to_string(), draft_id.to_string());
    }
}

pub(crate) fn editors() -> &'static Mutex<DraftEditors> {
    static EDITORS: OnceLock<Mutex<DraftEditors>> = OnceLock::new();
    EDITORS.get_or_init(Default::default)
}

pub(crate) fn window_is_live(app: &AppHandle) -> impl Fn(&str) -> bool + '_ {
    move |label| app.get_webview_window(label).is_some()
}

pub(crate) fn focus_window(app: &AppHandle, label: &str) {
    #[cfg(desktop)]
    if let Some(window) = app.get_webview_window(label) {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
    #[cfg(mobile)]
    let _ = (app, label);
}

/// Claims `draft_id` for the calling window's composer. `false` means another
/// window has the draft and was brought forward instead.
#[tauri::command]
pub async fn claim_draft_editor(
    app: AppHandle,
    window: WebviewWindow,
    draft_id: String,
) -> CommandResult<bool> {
    let refused = editors()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .claim(window.label(), &draft_id, window_is_live(&app));
    match refused {
        Some(owner) => {
            focus_window(&app, &owner);
            Ok(false)
        }
        None => Ok(true),
    }
}

/// Gives up the calling window's claim on `draft_id` when its composer closes.
#[tauri::command]
pub async fn release_draft_editor(window: WebviewWindow, draft_id: String) -> CommandResult<()> {
    editors()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .release(window.label(), &draft_id);
    Ok(())
}

/// Focuses the window that already edits `draft_id`. `false` when no other
/// window has it and the caller may open the draft itself.
#[tauri::command]
pub async fn focus_draft_editor(
    app: AppHandle,
    window: WebviewWindow,
    draft_id: String,
) -> CommandResult<bool> {
    let owner = editors()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .owner_besides(&draft_id, window.label(), window_is_live(&app));
    match owner {
        Some(owner) => {
            focus_window(&app, &owner);
            Ok(true)
        }
        None => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::DraftEditors;

    fn all_live(_: &str) -> bool {
        true
    }

    #[test]
    fn a_second_window_cannot_claim_an_open_draft() {
        let mut editors = DraftEditors::default();
        assert_eq!(editors.claim("compose-1", "d1", all_live), None);
        assert_eq!(
            editors.claim("main", "d1", all_live),
            Some("compose-1".to_string())
        );
        // The owner itself may claim again, e.g. after a reload.
        assert_eq!(editors.claim("compose-1", "d1", all_live), None);
    }

    #[test]
    fn a_closed_window_releases_its_draft() {
        let mut editors = DraftEditors::default();
        editors.claim("compose-1", "d1", all_live);
        let gone = |label: &str| label != "compose-1";
        assert_eq!(editors.claim("main", "d1", gone), None);
    }

    #[test]
    fn a_late_release_does_not_drop_the_successor_claim() {
        let mut editors = DraftEditors::default();
        editors.claim("main", "d1", all_live);
        // The next composer in the same window claims before the old one unmounts.
        editors.claim("main", "d2", all_live);
        editors.release("main", "d1");
        assert_eq!(
            editors.owner_besides("d2", "compose-1", all_live),
            Some("main".to_string())
        );
        assert_eq!(editors.claim("compose-1", "d1", all_live), None);
        editors.release("main", "d2");
        assert_eq!(editors.owner_besides("d2", "compose-1", all_live), None);
    }

    #[test]
    fn popping_out_hands_the_draft_to_the_new_window() {
        let mut editors = DraftEditors::default();
        editors.claim("main", "d1", all_live);
        editors.hand_over("main", "compose-1", "d1");
        assert_eq!(
            editors.owner_besides("d1", "main", all_live),
            Some("compose-1".to_string())
        );
        assert_eq!(
            editors.claim("main", "d1", all_live),
            Some("compose-1".to_string())
        );
    }
}
