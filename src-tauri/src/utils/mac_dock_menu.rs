//! Native actions in Satchel's macOS Dock menu.
//!
//! AppKit asks the current `NSApplicationDelegate` for
//! `applicationDockMenu:` whenever the Dock menu opens. Tao owns that delegate
//! and Tauri does not expose a Dock-menu API, so this adapter adds the one
//! optional delegate method to Tao's existing class. Replacing the delegate
//! would break Tauri's reopen and open-file handling.

use std::sync::{
    atomic::{AtomicBool, AtomicPtr, Ordering},
    OnceLock,
};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Imp, ProtocolObject, Sel};
use objc2::{define_class, sel, ClassType, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSApplication, NSApplicationDelegate, NSMenu, NSMenuItem};
use objc2_foundation::{NSObject, NSObjectProtocol, NSString};
use tauri::{AppHandle, Emitter, Manager};

const MAIN_WINDOW: &str = "main";
const TRAY_POPOVER: &str = "tray-popover";

static APP: OnceLock<AppHandle> = OnceLock::new();
static TARGET: AtomicPtr<DockMenuTarget> = AtomicPtr::new(std::ptr::null_mut());
static SYNC_RUNNING: AtomicBool = AtomicBool::new(false);

define_class!(
    // SAFETY: NSObject has no subclassing requirements. AppKit invokes menu
    // actions on the main thread, matching this class's thread kind.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "SatchelDockMenuTarget"]
    struct DockMenuTarget;

    impl DockMenuTarget {
        #[unsafe(method(satchelDockCompose:))]
        fn compose(&self, _sender: Option<&AnyObject>) {
            open_compose();
        }

        #[unsafe(method(satchelDockCalendar:))]
        fn calendar(&self, _sender: Option<&AnyObject>) {
            navigate_main("calendar");
        }

        #[unsafe(method(satchelDockSync:))]
        fn sync(&self, _sender: Option<&AnyObject>) {
            start_sync();
        }

        #[unsafe(method(satchelDockSettings:))]
        fn settings(&self, _sender: Option<&AnyObject>) {
            navigate_main("settings");
        }

        #[unsafe(method(satchelDockCloseOtherWindows:))]
        fn close_other_windows(&self, _sender: Option<&AnyObject>) {
            close_other_windows();
        }
    }

    unsafe impl NSObjectProtocol for DockMenuTarget {}
);

impl DockMenuTarget {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        // SAFETY: plain `-init` on a class without ivars.
        unsafe { objc2::msg_send![super(mtm.alloc::<Self>().set_ivars(())), init] }
    }
}

/// Install Satchel's optional Dock-menu callback on Tao's existing app
/// delegate. `setup()` calls this on AppKit's main thread.
pub fn init(app: AppHandle) {
    let Some(mtm) = MainThreadMarker::new() else {
        tracing::warn!("Dock menu initialization must run on the main thread");
        return;
    };

    if APP.set(app).is_err() {
        return;
    }

    // NSMenuItem's target is weak. Keep one target alive for the process by
    // deliberately transferring its retain count into a stable raw pointer.
    let target = DockMenuTarget::new(mtm);
    TARGET.store(Retained::into_raw(target), Ordering::Release);

    let application = NSApplication::sharedApplication(mtm);
    let Some(delegate) = application.delegate() else {
        tracing::warn!("macOS application delegate unavailable; Dock actions disabled");
        return;
    };
    let delegate_protocol: &ProtocolObject<dyn NSApplicationDelegate> = &delegate;
    let delegate_object: &AnyObject = delegate_protocol.as_ref();
    let delegate_class = delegate_object.class();

    type DockMenuMethod =
        unsafe extern "C-unwind" fn(&AnyObject, Sel, &NSApplication) -> *mut NSMenu;
    let implementation: Imp = unsafe {
        std::mem::transmute::<DockMenuMethod, Imp>(application_dock_menu as DockMenuMethod)
    };
    // `@@:@` = object return, object self, selector, object argument.
    let added = unsafe {
        objc2::ffi::class_addMethod(
            delegate_class as *const _ as *mut _,
            sel!(applicationDockMenu:),
            implementation,
            c"@@:@".as_ptr(),
        )
    };
    if !added.as_bool() {
        tracing::warn!("macOS application delegate already supplies a Dock menu");
    }
}

/// AppKit calls this immediately before displaying the Dock menu, which lets
/// the enabled state reflect the windows and sync work that exist right now.
unsafe extern "C-unwind" fn application_dock_menu(
    _delegate: &AnyObject,
    _selector: Sel,
    _application: &NSApplication,
) -> *mut NSMenu {
    let Some(mtm) = MainThreadMarker::new() else {
        return std::ptr::null_mut();
    };
    let Some(target) = TARGET.load(Ordering::Acquire).as_ref() else {
        return std::ptr::null_mut();
    };

    let menu = NSMenu::new(mtm);
    // We compute enabled states from Satchel's live window/sync state. AppKit's
    // responder-chain auto-validation would otherwise turn those rows back on
    // merely because the target implements their selectors.
    menu.setAutoenablesItems(false);
    let strings = crate::commands::ui_strings::current();
    add_action(
        &menu,
        mtm,
        target,
        &strings.dock_compose,
        sel!(satchelDockCompose:),
        true,
    );
    add_action(
        &menu,
        mtm,
        target,
        &strings.dock_calendar,
        sel!(satchelDockCalendar:),
        true,
    );
    add_action(
        &menu,
        mtm,
        target,
        &strings.dock_sync,
        sel!(satchelDockSync:),
        !SYNC_RUNNING.load(Ordering::Acquire),
    );
    add_action(
        &menu,
        mtm,
        target,
        &strings.dock_settings,
        sel!(satchelDockSettings:),
        true,
    );
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    add_action(
        &menu,
        mtm,
        target,
        &strings.dock_close_other_windows,
        sel!(satchelDockCloseOtherWindows:),
        APP.get().is_some_and(has_closable_windows),
    );

    Retained::autorelease_return(menu)
}

fn add_action(
    menu: &NSMenu,
    mtm: MainThreadMarker,
    target: &DockMenuTarget,
    title: &str,
    action: Sel,
    enabled: bool,
) {
    let item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            mtm.alloc(),
            &NSString::from_str(title),
            Some(action),
            &NSString::new(),
        )
    };
    item.setEnabled(enabled);
    // SAFETY: the selector is implemented by `DockMenuTarget`, and `TARGET`
    // keeps the target alive for the duration of the process.
    unsafe { item.setTarget(Some(target.as_super().as_super())) };
    menu.addItem(&item);
}

fn open_compose() {
    let Some(app) = APP.get().cloned() else {
        return;
    };
    tauri::async_runtime::spawn(async move {
        if let Err(error) = crate::commands::notifications::build_compose_window(&app, None, None) {
            tracing::warn!(%error, "failed to open composer from Dock menu");
        }
    });
}

fn navigate_main(destination: &'static str) {
    let Some(app) = APP.get() else {
        return;
    };
    crate::commands::tray::show_main(app);
    if let Err(error) = app.emit(
        "tray:navigate",
        serde_json::json!({ "destination": destination }),
    ) {
        tracing::warn!(%error, destination, "failed to navigate from Dock menu");
    }
}

fn start_sync() {
    if SYNC_RUNNING
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    let Some(app) = APP.get().cloned() else {
        SYNC_RUNNING.store(false, Ordering::Release);
        return;
    };
    let pool = app.state::<sqlx::SqlitePool>().inner().clone();
    let events = app.state::<crate::AppState>().events.clone();
    tauri::async_runtime::spawn(async move {
        let result = crate::sync::mailbox_sync::sync_mailboxes(
            &pool,
            crate::sync::mailbox_sync::MailboxSyncRequest {
                events: Some(&events),
                app: Some(&app),
                accounts: crate::sync::mailbox_sync::AccountSelection::AllActive,
                folders: crate::sync::mailbox_sync::FolderSelection::All,
            },
        )
        .await;
        SYNC_RUNNING.store(false, Ordering::Release);
        match result {
            Ok(summary) => tracing::info!(
                accounts_synced = summary.accounts_synced,
                accounts_failed = summary.accounts_failed,
                messages_synced = summary.messages_synced,
                "Dock menu synchronization finished"
            ),
            Err(error) => tracing::warn!(%error, "Dock menu synchronization failed"),
        }
    });
}

fn close_other_windows() {
    let Some(app) = APP.get() else {
        return;
    };
    if let Some(popover) = app.get_webview_window(TRAY_POPOVER) {
        let _ = popover.hide();
    }
    for (label, window) in app.webview_windows() {
        if is_closable_window(&label) {
            // `close`, unlike `destroy`, lets the standalone composer save a
            // dirty draft in its existing close-request handler.
            if let Err(error) = window.close() {
                tracing::warn!(%error, %label, "failed to close secondary window");
            }
        }
    }
}

fn has_closable_windows(app: &AppHandle) -> bool {
    app.webview_windows()
        .keys()
        .any(|label| is_closable_window(label))
}

fn is_closable_window(label: &str) -> bool {
    !matches!(label, MAIN_WINDOW | TRAY_POPOVER)
}

#[cfg(test)]
mod tests {
    use super::is_closable_window;

    #[test]
    fn main_and_tray_windows_are_not_closed() {
        assert!(!is_closable_window("main"));
        assert!(!is_closable_window("tray-popover"));
    }

    #[test]
    fn compose_and_message_windows_are_closed() {
        assert!(is_closable_window("compose-deadbeef"));
        assert!(is_closable_window("message-file-deadbeef"));
    }
}
