//! Mail notifications on macOS, delivered through `UNUserNotificationCenter`.
//!
//! `tauri-plugin-notification` drops the notification response on desktop — its
//! `show()` calls `notify-rust` and discards the returned `NotificationResponse`.
//! A click therefore never reaches any handler, which is why the app used to
//! merely come to the front (macOS activates it on its own) and stay wherever
//! the user last was.
//!
//! `UserNotifications` hands the delegate the clicked notification's `userInfo`
//! verbatim. That is the only way to learn *which* of several pending mail
//! notifications the user picked — a heuristic over "the most recent thread"
//! cannot, and a click that arrives minutes later out of the notification centre
//! is entirely ordinary.
//!
//! The framework requires a real application bundle: `currentNotificationCenter`
//! raises `NSInternalInconsistencyException` when the executable is not inside a
//! `.app`. `cargo tauri dev` runs the bare binary, so every entry point here is
//! gated on [`is_bundled`] and callers fall back to the plugin.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{define_class, msg_send, MainThreadMarker, MainThreadOnly};
use objc2_foundation::{
    NSArray, NSBundle, NSDictionary, NSObject, NSObjectProtocol, NSSet, NSString,
};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNMutableNotificationContent, UNNotification, UNNotificationAction,
    UNNotificationActionOptions, UNNotificationCategory, UNNotificationCategoryOptions,
    UNNotificationPresentationOptions, UNNotificationRequest, UNNotificationResponse,
    UNNotificationSound, UNUserNotificationCenter, UNUserNotificationCenterDelegate,
};
use tauri::{AppHandle, Emitter, Manager};

/// Category the mail notifications register under. The quick actions hang off
/// the category, not off the individual notification.
const CATEGORY_MAIL: &str = "satchel.mail";

const ACTION_ARCHIVE: &str = "satchel.archive";
const ACTION_TRASH: &str = "satchel.trash";
const ACTION_READ: &str = "satchel.read";

/// Key under which the thread id travels in the notification's `userInfo`.
const USER_INFO_THREAD_ID: &str = "thread_id";

/// Set once the delegate is installed. Delivery before that point would drop
/// clicks on the floor, so [`deliver`] refuses to run until it is true.
static READY: AtomicBool = AtomicBool::new(false);

/// Set when the authorization callback reported a refusal.
///
/// `addNotificationRequest` succeeds regardless — an unauthorized process hands
/// its banner to a system that silently discards it. Without this flag a denied
/// permission is indistinguishable from a delivered notification, which is the
/// one failure mode a user reads as "Satchel does not notify me".
static AUTHORIZATION_DENIED: AtomicBool = AtomicBool::new(false);

/// The app handle the delegate routes actions through.
static APP: OnceLock<AppHandle> = OnceLock::new();

/// Whether this process runs from a real `.app` bundle.
///
/// Deliberately checks the bundle *path* rather than the identifier:
/// `mac-notification-sys` (reached through `tauri-plugin-notification`)
/// swizzles `-[NSBundle bundleIdentifier]` to fake one for unbundled binaries,
/// so the identifier stops being evidence of anything once a plugin
/// notification has been sent.
pub fn is_bundled() -> bool {
    static BUNDLED: OnceLock<bool> = OnceLock::new();
    *BUNDLED.get_or_init(|| {
        let path = NSBundle::mainBundle().bundlePath();
        path.to_string().ends_with(".app")
    })
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements beyond the ivars
    // declared here, and the delegate methods are only ever invoked by
    // UserNotifications on the main thread.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "SatchelNotificationDelegate"]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl UNUserNotificationCenterDelegate for Delegate {
        /// Without this the system suppresses banners while Satchel is the
        /// frontmost app — which is exactly when a new mail is worth showing in
        /// a client the user keeps open all day.
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(
            &self,
            _center: &UNUserNotificationCenter,
            _notification: &UNNotification,
            completion: &block2::DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            completion.call((UNNotificationPresentationOptions::Banner
                | UNNotificationPresentationOptions::List
                | UNNotificationPresentationOptions::Sound,));
        }

        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn did_receive_response(
            &self,
            _center: &UNUserNotificationCenter,
            response: &UNNotificationResponse,
            completion: &block2::DynBlock<dyn Fn()>,
        ) {
            handle_response(response);
            completion.call(());
        }
    }
);

impl Delegate {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        // SAFETY: plain `-init` on a class with no ivars.
        unsafe { msg_send![super(mtm.alloc::<Self>().set_ivars(())), init] }
    }
}

/// Install the notification delegate and register the mail category.
///
/// Must run on the main thread and before the first notification is delivered;
/// `setup()` in `lib.rs` satisfies both.
pub fn init(app: AppHandle) {
    if !is_bundled() {
        return;
    }
    let Some(mtm) = MainThreadMarker::new() else {
        tracing::error!(target: "notify", "mac_notify::init called off the main thread");
        return;
    };
    let _ = APP.set(app);

    // `currentNotificationCenter` raises rather than returns nil when the
    // framework refuses this process — an unsigned bundle is the case a
    // `.app`-path check cannot see. An uncaught NSException here would take the
    // app down during startup, so a raise degrades to the plugin instead.
    let installed = objc2::exception::catch(|| {
        let center = UNUserNotificationCenter::currentNotificationCenter();

        // The delegate is a *weak* property on the center, so it has to outlive
        // this call — hence the leak. It lives as long as the process does.
        let delegate = Delegate::new(mtm);
        let protocol: &ProtocolObject<dyn UNUserNotificationCenterDelegate> =
            ProtocolObject::from_ref(&*delegate);
        center.setDelegate(Some(protocol));
        std::mem::forget(delegate);

        center.setNotificationCategories(&mail_category());

        let options = UNAuthorizationOptions::Alert
            | UNAuthorizationOptions::Sound
            | UNAuthorizationOptions::Badge;
        let completion = RcBlock::new(|granted: objc2::runtime::Bool, _error: *mut _| {
            AUTHORIZATION_DENIED.store(!granted.as_bool(), Ordering::Release);
            if !granted.as_bool() {
                tracing::warn!(target: "notify", "notification authorization denied");
            }
        });
        center.requestAuthorizationWithOptions_completionHandler(options, &completion);
    });

    match installed {
        Ok(()) => READY.store(true, Ordering::Release),
        Err(err) => tracing::warn!(target: "notify", "UserNotifications unavailable: {err:?}"),
    }
}

/// The quick actions shown on a mail notification.
///
/// `Destructive` paints the delete action red; none of the three carry
/// `Foreground`, so
/// acting on a mail never yanks the app in front of whatever the user is doing.
fn mail_category() -> Retained<NSSet<UNNotificationCategory>> {
    let strings = crate::commands::ui_strings::current();
    let action = |id: &str, title: &str, options: UNNotificationActionOptions| {
        UNNotificationAction::actionWithIdentifier_title_options(
            &NSString::from_str(id),
            &NSString::from_str(title),
            options,
        )
    };

    let actions = NSArray::from_retained_slice(&[
        action(
            ACTION_ARCHIVE,
            &strings.notification_archive,
            UNNotificationActionOptions::empty(),
        ),
        action(
            ACTION_READ,
            &strings.notification_read,
            UNNotificationActionOptions::empty(),
        ),
        action(
            ACTION_TRASH,
            &strings.notification_trash,
            UNNotificationActionOptions::Destructive,
        ),
    ]);

    let category = UNNotificationCategory::categoryWithIdentifier_actions_intentIdentifiers_options(
        &NSString::from_str(CATEGORY_MAIL),
        &actions,
        &NSArray::from_retained_slice(&[]),
        UNNotificationCategoryOptions::empty(),
    );
    NSSet::from_retained_slice(&[category])
}

/// Outcome of one native delivery attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeDelivery {
    /// Handed to `UNUserNotificationCenter`.
    Delivered,
    /// Not a bundled `.app`, so the framework is unavailable.
    NotBundled,
    /// Bundled, but the delegate could not be installed (unsigned bundle).
    Unavailable,
    /// The user refused notification permission. The plugin fallback would be
    /// refused by the same TCC entry, so the caller should not try it.
    PermissionDenied,
}

/// UserNotifications stores category labels at registration time. Registering
/// the same identifier again replaces them for subsequently displayed alerts.
pub fn refresh_category() {
    if !is_bundled() || !READY.load(Ordering::Acquire) {
        return;
    }
    UNUserNotificationCenter::currentNotificationCenter()
        .setNotificationCategories(&mail_category());
}

fn native_unavailable() -> Option<NativeDelivery> {
    if !is_bundled() {
        return Some(NativeDelivery::NotBundled);
    }
    if AUTHORIZATION_DENIED.load(Ordering::Acquire) {
        return Some(NativeDelivery::PermissionDenied);
    }
    if !READY.load(Ordering::Acquire) {
        return Some(NativeDelivery::Unavailable);
    }
    None
}

/// Summary identifier: a new summary replaces the one on screen.
const SUMMARY_IDENTIFIER: &str = "satchel-mail-summary";

/// The banner that stands in for the mails of a burst.
pub fn deliver_summary(title: &str, sound: bool) -> NativeDelivery {
    if let Some(unavailable) = native_unavailable() {
        return unavailable;
    }
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(title));
    if sound {
        content.setSound(Some(&UNNotificationSound::defaultSound()));
    }
    let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
        &NSString::from_str(SUMMARY_IDENTIFIER),
        &content,
        None,
    );
    UNUserNotificationCenter::currentNotificationCenter()
        .addNotificationRequest_withCompletionHandler(&request, None);
    NativeDelivery::Delivered
}

/// Deliver a mail notification carrying `thread_id`, stacked with the other
/// mail of `account_id`.
pub fn deliver(
    title: &str,
    body: &str,
    thread_id: &str,
    account_id: &str,
    account_label: &str,
    sound: bool,
) -> NativeDelivery {
    if let Some(unavailable) = native_unavailable() {
        return unavailable;
    }

    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(title));
    content.setBody(&NSString::from_str(body));
    content.setCategoryIdentifier(&NSString::from_str(CATEGORY_MAIL));
    // One stack per account, as on Android; per-thread stacks flood the notification centre.
    content.setThreadIdentifier(&NSString::from_str(&format!("account:{account_id}")));
    if !account_label.is_empty() {
        content.setSubtitle(&NSString::from_str(account_label));
    }
    if sound {
        content.setSound(Some(&UNNotificationSound::defaultSound()));
    }

    let keys = [&*NSString::from_str(USER_INFO_THREAD_ID)];
    let values = [&*NSString::from_str(thread_id)];
    let user_info: Retained<NSDictionary> =
        unsafe { Retained::cast_unchecked(NSDictionary::from_slices::<NSString>(&keys, &values)) };
    // SAFETY: the dictionary holds only property-list types, as the framework
    // requires for `userInfo`.
    unsafe { content.setUserInfo(&user_info) };

    // A fresh identifier per notification: reusing one replaces the notification
    // already on screen, which would silently swallow a second new mail.
    let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
        &NSString::from_str(&uuid::Uuid::new_v4().simple().to_string()),
        &content,
        None,
    );
    UNUserNotificationCenter::currentNotificationCenter()
        .addNotificationRequest_withCompletionHandler(&request, None);
    NativeDelivery::Delivered
}

/// Remove every delivered mail notification belonging to one of the threads.
pub fn dismiss_threads(thread_ids: Vec<String>) {
    if !is_bundled() || thread_ids.is_empty() {
        return;
    }
    let wanted = std::collections::HashSet::<String>::from_iter(thread_ids);
    let center = UNUserNotificationCenter::currentNotificationCenter();
    let completion = RcBlock::new(
        move |notifications: std::ptr::NonNull<NSArray<UNNotification>>| {
            // SAFETY: UserNotifications owns the array for the duration of this
            // completion callback.
            let notifications = unsafe { notifications.as_ref() };
            let mut identifiers = Vec::new();
            for index in 0..notifications.count() {
                let notification = notifications.objectAtIndex(index);
                let request = notification.request();
                let user_info = request.content().userInfo();
                let key = NSString::from_str(USER_INFO_THREAD_ID);
                let thread_id = user_info
                    .objectForKey(&key)
                    .and_then(|value| value.downcast::<NSString>().ok())
                    .map(|value| value.to_string());
                if thread_id.is_some_and(|thread_id| wanted.contains(&thread_id)) {
                    identifiers.push(request.identifier());
                }
            }
            if !identifiers.is_empty() {
                UNUserNotificationCenter::currentNotificationCenter()
                    .removeDeliveredNotificationsWithIdentifiers(&NSArray::from_retained_slice(
                        &identifiers,
                    ));
            }
        },
    );
    center.getDeliveredNotificationsWithCompletionHandler(&completion);
}

/// Route a click or quick action to the app.
fn handle_response(response: &UNNotificationResponse) {
    let Some(app) = APP.get() else {
        return;
    };

    let user_info = response.notification().request().content().userInfo();
    let key = NSString::from_str(USER_INFO_THREAD_ID);
    let thread_id = user_info
        .objectForKey(&key)
        .and_then(|value| value.downcast::<NSString>().ok())
        .map(|value| value.to_string());
    let Some(thread_id) = thread_id.filter(|id| !id.is_empty()) else {
        return;
    };

    let action = response.actionIdentifier().to_string();
    // SAFETY: reading framework string constants.
    let default_action =
        unsafe { objc2_user_notifications::UNNotificationDefaultActionIdentifier.to_string() };

    match action.as_str() {
        ACTION_ARCHIVE => run_thread_action(app.clone(), thread_id, ThreadAction::Archive),
        ACTION_TRASH => run_thread_action(app.clone(), thread_id, ThreadAction::Trash),
        ACTION_READ => run_thread_action(app.clone(), thread_id, ThreadAction::MarkRead),
        other if other == default_action => {
            crate::commands::tray::show_main(app);
            crate::commands::notification_navigation::focus_thread(app, thread_id);
        }
        // Dismissal and unknown identifiers are not actions.
        _ => {}
    }
}

#[derive(Clone, Copy)]
enum ThreadAction {
    Archive,
    Trash,
    MarkRead,
}

/// Apply a quick action without bringing the window forward.
///
/// The frontend learns about it through `thread:updated`, which the stores
/// already listen for — the notification path must not become a second way of
/// mutating threads that the UI does not hear about.
fn run_thread_action(app: AppHandle, thread_id: String, action: ThreadAction) {
    tauri::async_runtime::spawn(async move {
        use crate::commands::threads::{
            archive_threads_inner, set_threads_read, trash_threads, ThreadIdsInput,
            UpdateThreadsReadInput,
        };

        let pool = app.state::<sqlx::SqlitePool>();
        let state = app.state::<crate::AppState>();
        let ids = vec![thread_id.clone()];

        let result = match action {
            ThreadAction::Archive => {
                archive_threads_inner(&pool, &state, ThreadIdsInput { thread_ids: ids })
                    .await
                    .map(|_| ())
            }
            ThreadAction::Trash => trash_threads(pool, state, ThreadIdsInput { thread_ids: ids })
                .await
                .map(|_| ()),
            ThreadAction::MarkRead => {
                set_threads_read(
                    pool,
                    state,
                    UpdateThreadsReadInput {
                        thread_ids: ids,
                        is_read: true,
                    },
                )
                .await
            }
        };

        match result {
            Ok(()) => {
                dismiss_threads(vec![thread_id.clone()]);
                let _ = app.emit(
                    "thread:updated",
                    serde_json::json!({ "thread_id": thread_id }),
                );
            }
            Err(err) => {
                tracing::error!(target: "notify", "quick action failed: {err}");
                // macOS removed the banner on the click; without this the
                // failure looks exactly like success.
                let strings = crate::commands::ui_strings::current();
                let label = match action {
                    ThreadAction::Archive => strings.notification_archive,
                    ThreadAction::Trash => strings.notification_trash,
                    ThreadAction::MarkRead => strings.notification_read,
                };
                crate::commands::notifications::show_notification(
                    &app,
                    &strings.notification_action_failed_title,
                    &strings
                        .notification_action_not_applied_message
                        .replace("{action}", &label),
                );
            }
        }
    });
}
