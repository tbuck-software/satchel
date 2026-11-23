//! Quit waits for acknowledgements from the IPC callers in the current round.

#[tauri::command]
pub fn acknowledge_draft_flush(window: tauri::WebviewWindow, round_id: String, saved: bool) {
    #[cfg(desktop)]
    desktop::acknowledge(window, round_id, saved);
    #[cfg(mobile)]
    let _ = (window, round_id, saved);
}

#[cfg(desktop)]
pub(crate) use desktop::request_exit;

#[cfg(desktop)]
mod desktop {
    use std::{collections::BTreeSet, sync::Mutex, time::Duration};

    use tauri::{AppHandle, Emitter, Manager, WebviewWindow};
    use tauri_plugin_dialog::{
        DialogExt, MessageDialogButtons, MessageDialogKind, MessageDialogResult,
    };

    use crate::commands::{app_update, draft_editors, ui_strings};

    /// After one second, ask whether to give the composer more time.
    const DRAFT_FLUSH_GRACE: Duration = Duration::from_secs(1);

    #[derive(Debug, PartialEq, Eq)]
    enum Step {
        Wait,
        FlushAgain,
        Confirm(String),
        StayOpen(Option<String>),
        Exit,
        QuitAnyway,
    }

    #[derive(PartialEq, Eq)]
    enum Phase {
        Saving,
        Confirming,
        Waiting,
    }

    struct Round {
        id: String,
        pending: BTreeSet<String>,
        failed: Option<String>,
        phase: Phase,
    }

    impl Round {
        fn new(windows: BTreeSet<String>) -> Self {
            Self {
                id: uuid::Uuid::new_v4().to_string(),
                pending: windows,
                failed: None,
                phase: Phase::Saving,
            }
        }

        fn acknowledge(&mut self, caller: &str, saved: bool) -> Step {
            if !self.pending.remove(caller) {
                return Step::Wait;
            }
            if !saved && self.failed.is_none() {
                self.failed = Some(caller.into());
            }
            self.progress()
        }

        fn progress(&self) -> Step {
            if self.phase == Phase::Confirming || !self.pending.is_empty() {
                return Step::Wait;
            }
            match &self.failed {
                Some(window) => Step::StayOpen(Some(window.clone())),
                // Other windows could have been edited while we waited.
                None if self.phase == Phase::Waiting => Step::FlushAgain,
                None => Step::Exit,
            }
        }

        fn deadline(&mut self) -> Step {
            match self.pending.first() {
                Some(window) if self.phase != Phase::Confirming => {
                    self.phase = Phase::Confirming;
                    Step::Confirm(window.clone())
                }
                _ => Step::Wait,
            }
        }

        fn decide(&mut self, decision: Decision) -> Step {
            if self.phase != Phase::Confirming {
                return Step::Wait;
            }
            match decision {
                Decision::KeepWaiting => {
                    self.phase = Phase::Waiting;
                    self.progress()
                }
                Decision::Cancel => Step::StayOpen(self.failed.clone()),
                Decision::QuitAnyway => Step::QuitAnyway,
            }
        }
    }

    enum Decision {
        KeepWaiting,
        Cancel,
        QuitAnyway,
    }

    enum QuitState {
        Idle,
        Flushing(Round),
        ExitAuthorized,
    }

    impl QuitState {
        fn advance(&mut self, id: &str, advance: impl FnOnce(&mut Round) -> Step) -> Step {
            let Self::Flushing(round) = self else {
                return Step::Wait;
            };
            if round.id != id {
                return Step::Wait;
            }
            let step = advance(round);
            match step {
                Step::Exit | Step::QuitAnyway => *self = Self::ExitAuthorized,
                Step::FlushAgain | Step::StayOpen(_) => *self = Self::Idle,
                _ => {}
            }
            step
        }
    }

    static QUIT: Mutex<QuitState> = Mutex::new(QuitState::Idle);

    /// Only main and the composer popouts can hold unsaved drafts.
    pub(crate) fn request_exit(app: &AppHandle) -> bool {
        let retry = match &*QUIT.lock().expect("draft flush lock") {
            QuitState::ExitAuthorized => return false,
            QuitState::Flushing(round) if round.phase == Phase::Waiting => Some(round.id.clone()),
            QuitState::Flushing(_) => return true,
            QuitState::Idle => None,
        };
        if let Some(id) = retry {
            advance(app, &id, Round::deadline);
            return true;
        }
        if app_update::installation_in_progress(app) {
            return true;
        }

        let windows: BTreeSet<_> = app
            .webview_windows()
            .into_keys()
            .filter(|label| label == "main" || label.starts_with("compose-"))
            .collect();
        if windows.is_empty() {
            return app_update::install_on_exit(app);
        }
        let round = Round::new(windows);
        let id = round.id.clone();
        *QUIT.lock().expect("draft flush lock") = QuitState::Flushing(round);
        let _ = app.emit("app:flush-drafts", &id);

        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(DRAFT_FLUSH_GRACE).await;
            let _ = app
                .clone()
                .run_on_main_thread(move || advance(&app, &id, Round::deadline));
        });
        true
    }

    pub(super) fn acknowledge(window: WebviewWindow, id: String, saved: bool) {
        let app = window.app_handle().clone();
        let _ = app.clone().run_on_main_thread(move || {
            advance(&app, &id, |round| round.acknowledge(window.label(), saved));
        });
    }

    fn advance(app: &AppHandle, id: &str, update: impl FnOnce(&mut Round) -> Step) {
        let step = QUIT.lock().expect("draft flush lock").advance(id, update);
        match step {
            Step::Wait | Step::StayOpen(None) => {}
            Step::FlushAgain => {
                if !request_exit(app) {
                    *QUIT.lock().expect("draft flush lock") = QuitState::ExitAuthorized;
                    crate::exit_now(app);
                }
            }
            Step::Confirm(window) => confirm_quit(app, id, &window),
            Step::Exit if app_update::install_on_exit(app) => {
                // The updater owns this exit. A failed install leaves Quit available.
                *QUIT.lock().expect("draft flush lock") = QuitState::Idle;
            }
            Step::Exit | Step::QuitAnyway => crate::exit_now(app),
            Step::StayOpen(Some(window)) => draft_editors::focus_window(app, &window),
        }
    }

    fn confirm_quit(app: &AppHandle, id: &str, window: &str) {
        draft_editors::focus_window(app, window);
        let strings = ui_strings::current();
        // Use native UI so a stalled webview cannot make the decision unreachable.
        let dialog = app
            .dialog()
            .message(strings.draft_quit_message)
            .title(strings.tray_quit)
            .kind(MessageDialogKind::Warning)
            .buttons(MessageDialogButtons::YesNoCancelCustom(
                strings.draft_quit_wait.clone(),
                strings.draft_quit_anyway.clone(),
                strings.draft_quit_cancel,
            ));
        let dialog = match app.get_webview_window(window) {
            Some(parent) => dialog.parent(&parent),
            None => dialog,
        };
        let app = app.clone();
        let id = id.to_owned();
        dialog.show_with_result(move |result| {
            let decision = match result {
                MessageDialogResult::Custom(label) if label == strings.draft_quit_wait => {
                    Decision::KeepWaiting
                }
                MessageDialogResult::Custom(label) if label == strings.draft_quit_anyway => {
                    Decision::QuitAnyway
                }
                _ => Decision::Cancel,
            };
            let _ = app.clone().run_on_main_thread(move || {
                advance(&app, &id, |round| round.decide(decision));
            });
        });
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn round() -> Round {
            Round::new(BTreeSet::from(["main".into(), "compose-1".into()]))
        }

        #[test]
        fn only_each_expected_window_can_acknowledge_once() {
            let mut round = round();
            assert_eq!(round.acknowledge("message-file-1", false), Step::Wait);
            assert_eq!(round.acknowledge("tray-popover", true), Step::Wait);
            assert_eq!(round.acknowledge("main", true), Step::Wait);
            assert_eq!(round.acknowledge("main", true), Step::Wait);
            assert_eq!(round.acknowledge("compose-1", true), Step::Exit);
        }

        #[test]
        fn a_failed_save_leaves_the_app_open_and_focuses_the_composer() {
            let mut round = round();
            assert_eq!(round.acknowledge("compose-1", false), Step::Wait);
            assert_eq!(
                round.acknowledge("main", true),
                Step::StayOpen(Some("compose-1".into()))
            );
            assert_eq!(round.deadline(), Step::Wait);
        }

        #[tokio::test]
        async fn a_save_taking_longer_than_one_second_waits_for_the_user() {
            let mut round = round();
            assert_eq!(round.acknowledge("main", true), Step::Wait);
            tokio::time::sleep(DRAFT_FLUSH_GRACE).await;
            assert_eq!(round.deadline(), Step::Confirm("compose-1".into()));
            tokio::time::sleep(Duration::from_millis(100)).await;
            assert_eq!(round.acknowledge("compose-1", true), Step::Wait);
            assert_eq!(round.decide(Decision::KeepWaiting), Step::FlushAgain);
        }

        #[test]
        fn a_frozen_composer_can_be_left_open_or_explicitly_quit() {
            let round = round();
            let cancelled = round.id.clone();
            let mut state = QuitState::Flushing(round);
            assert_eq!(
                state.advance(&cancelled, Round::deadline),
                Step::Confirm("compose-1".into())
            );
            assert_eq!(
                state.advance(&cancelled, |round| round.decide(Decision::Cancel)),
                Step::StayOpen(None)
            );
            assert!(matches!(state, QuitState::Idle));

            let next = self::round();
            let id = next.id.clone();
            state = QuitState::Flushing(next);
            assert_eq!(
                state.advance(&cancelled, |round| round.acknowledge("compose-1", true)),
                Step::Wait
            );
            assert_eq!(
                state.advance(&id, Round::deadline),
                Step::Confirm("compose-1".into())
            );
            assert_eq!(
                state.advance(&id, |round| round.decide(Decision::KeepWaiting)),
                Step::Wait
            );
            assert_eq!(
                state.advance(&id, Round::deadline),
                Step::Confirm("compose-1".into())
            );
            assert_eq!(
                state.advance(&id, |round| round.decide(Decision::QuitAnyway)),
                Step::QuitAnyway
            );
            assert!(matches!(state, QuitState::ExitAuthorized));
        }
    }
}
