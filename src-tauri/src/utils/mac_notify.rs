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
    NSArray, NSBundle, NSDictionary, NSError, NSObject, NSObjectProtocol, NSSet, NSString,
};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNAuthorizationStatus, UNErrorCode, UNErrorDomain,
    UNMutableNotificationContent, UNNotification, UNNotificationAction,
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
    /// The system accepted the request.
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
    if !READY.load(Ordering::Acquire) {
        return Some(NativeDelivery::Unavailable);
    }
    None
}

fn submission_outcome(error: Option<&NSError>) -> Result<NativeDelivery, String> {
    let Some(error) = error else {
        return Ok(NativeDelivery::Delivered);
    };
    // SAFETY: the framework exports this constant for the life of the process.
    let permission_denied = unsafe { UNErrorDomain }
        .is_some_and(|domain| *error.domain() == *domain)
        && error.code() == UNErrorCode::NotificationsNotAllowed.0;
    if permission_denied {
        Ok(NativeDelivery::PermissionDenied)
    } else {
        Err(format!(
            "macOS rejected the notification: {}",
            error.localizedDescription()
        ))
    }
}

#[cfg(test)]
#[test]
fn native_completion_distinguishes_acceptance_from_permission_and_dispatch_errors() {
    assert_eq!(submission_outcome(None), Ok(NativeDelivery::Delivered));
    // SAFETY: the framework owns the error domain, and no user info is passed.
    let denied = unsafe {
        NSError::errorWithDomain_code_userInfo(
            UNErrorDomain.unwrap(),
            UNErrorCode::NotificationsNotAllowed.0,
            None,
        )
    };
    assert_eq!(
        submission_outcome(Some(&denied)),
        Ok(NativeDelivery::PermissionDenied)
    );
    // Same code in another domain is an ordinary error.
    let failed = unsafe {
        NSError::errorWithDomain_code_userInfo(
            &NSString::from_str("test"),
            UNErrorCode::NotificationsNotAllowed.0,
            None,
        )
    };
    assert!(submission_outcome(Some(&failed)).is_err());
}

/// Query live authorization, then wait for the system to accept the request.
/// The callbacks retain the Objective-C objects. Only the result crosses await.
fn submit(
    request: Retained<UNNotificationRequest>,
) -> impl std::future::Future<Output = Result<NativeDelivery, String>> {
    use std::sync::{Arc, Mutex};
    let (sender, result) = tokio::sync::oneshot::channel();
    let sender = Arc::new(Mutex::new(Some(sender)));
    let finish = move |outcome| {
        if let Some(sender) = sender
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            let _ = sender.send(outcome);
        }
    };
    let center = UNUserNotificationCenter::currentNotificationCenter();
    let delivery_center = center.clone();
    let settings = RcBlock::new(
        move |settings: std::ptr::NonNull<objc2_user_notifications::UNNotificationSettings>| {
            // SAFETY: the framework keeps the settings alive during this callback.
            match unsafe { settings.as_ref() }.authorizationStatus() {
                // Without permission this delivery is suppressed. Retrying would show
                // stale mail after the user eventually answers the system prompt.
                UNAuthorizationStatus::Denied | UNAuthorizationStatus::NotDetermined => {
                    finish(Ok(NativeDelivery::PermissionDenied));
                }
                _ => {
                    let finish = finish.clone();
                    let completion = RcBlock::new(move |error: *mut NSError| {
                        // SAFETY: the framework owns a non-null error during the callback.
                        finish(submission_outcome(unsafe { error.as_ref() }));
                    });
                    delivery_center
                        .addNotificationRequest_withCompletionHandler(&request, Some(&completion));
                }
            }
        },
    );
    center.getNotificationSettingsWithCompletionHandler(&settings);
    async move {
        tokio::time::timeout(std::time::Duration::from_secs(10), result)
            .await
            .map_err(|_| "macOS notification request timed out".to_string())?
            .map_err(|_| "macOS notification callback was dropped".to_string())?
    }
}

/// Summary identifier: a new summary replaces the one on screen.
const SUMMARY_IDENTIFIER: &str = "satchel-mail-summary";

/// The banner that stands in for the mails of a burst.
pub async fn deliver_summary(title: &str, sound: bool) -> Result<NativeDelivery, String> {
    if let Some(unavailable) = native_unavailable() {
        return Ok(unavailable);
    }
    let completion = {
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
        submit(request)
    };
    completion.await
}

/// Deliver a mail notification carrying `thread_id`, stacked with the other
/// mail of `account_id`.
pub async fn deliver(
    event_id: &str,
    title: &str,
    body: &str,
    thread_id: &str,
    account_id: &str,
    account_label: &str,
    sound: bool,
) -> Result<NativeDelivery, String> {
    if let Some(unavailable) = native_unavailable() {
        return Ok(unavailable);
    }

    let completion = {
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
        let user_info: Retained<NSDictionary> = unsafe {
            Retained::cast_unchecked(NSDictionary::from_slices::<NSString>(&keys, &values))
        };
        // SAFETY: the dictionary holds only property-list types, as the framework
        // requires for `userInfo`.
        unsafe { content.setUserInfo(&user_info) };

        // Each mail has its own ID. Retrying the same outbox event replaces its banner.
        let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
            &NSString::from_str(&format!("satchel-mail:{event_id}")),
            &content,
            None,
        );
        submit(request)
    };
    completion.await
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
