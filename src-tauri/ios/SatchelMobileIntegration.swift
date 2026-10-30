import Foundation
import Tauri
import UIKit
import UserNotifications

@_silgen_name("satchel_mobile_event")
private func satchel_mobile_event(_ json: UnsafePointer<CChar>)

private let routeGate = SatchelRouteGate(emit: SatchelMobileIntegration.emit)

private final class SatchelMobileIntegration: NSObject, UNUserNotificationCenterDelegate {
    static let shared = SatchelMobileIntegration()
    private var previous: UNUserNotificationCenterDelegate?

    func install() {
        let center = UNUserNotificationCenter.current()
        guard center.delegate !== self else { return }
        previous = center.delegate
        center.delegate = self
    }

    func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        willPresent notification: UNNotification,
        withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void
    ) {
        // Preserve the plugin's foreground event, silent flag and presentation options.
        if let previous, previous.responds(to: #selector(userNotificationCenter(_:willPresent:withCompletionHandler:))) {
            previous.userNotificationCenter?(center, willPresent: notification, withCompletionHandler: completionHandler)
        } else {
            completionHandler([])
        }
    }

    func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        didReceive response: UNNotificationResponse,
        withCompletionHandler completionHandler: @escaping () -> Void
    ) {
        let request = response.notification.request
        let actionId = response.actionIdentifier == UNNotificationDefaultActionIdentifier
            ? "tap" : response.actionIdentifier
        let result = request.trigger is UNPushNotificationTrigger
            ? SatchelNotificationResponse.unrelated
            : SatchelNotificationResponse.decode(
                category: request.content.categoryIdentifier,
                userInfo: request.content.userInfo,
                actionId: actionId,
                inputValue: (response as? UNTextInputNotificationResponse)?.userText
            )
        switch result {
        case .route(let route):
            // Do not forward mail responses to plugin 2.3.3: it omits extra and
            // force-unwraps a process-local record that is absent on cold launch.
            // Background actions finish only after the frontend acknowledges
            // handling them, including when a cold WebView first has to load.
            routeGate.queue(route, completion: completionHandler)
        case .ignored:
            completionHandler()
        case .unrelated:
            if let previous, previous.responds(to: #selector(userNotificationCenter(_:didReceive:withCompletionHandler:))) {
                previous.userNotificationCenter?(center, didReceive: response, withCompletionHandler: completionHandler)
            } else {
                completionHandler()
            }
        }
    }

    func userNotificationCenter(_ center: UNUserNotificationCenter, openSettingsFor notification: UNNotification?) {
        previous?.userNotificationCenter?(center, openSettingsFor: notification)
    }

    static func emit(_ value: [String: Any]) {
        guard JSONSerialization.isValidJSONObject(value),
              let data = try? JSONSerialization.data(withJSONObject: value, options: [.sortedKeys]),
              let json = String(data: data, encoding: .utf8) else { return }
        json.withCString { satchel_mobile_event($0) }
    }
}

/// Registered synchronously after tauri-plugin-notification, before the event
/// loop can deliver a cold-launch response. Installing at main() is too early:
/// NotificationManager.init replaces the center's delegate during plugin setup.
private final class SatchelMailNotificationsPlugin: Plugin {
    override init() {
        super.init()
        SatchelMobileIntegration.shared.install()
    }
}

@_cdecl("init_plugin_satchel_mail_notifications")
func initSatchelMailNotificationsPlugin() -> Plugin {
    SatchelMailNotificationsPlugin()
}

@_cdecl("satchel_mobile_frontend_ready")
public func satchel_mobile_frontend_ready() {
    routeGate.markFrontendReady()
}

@_cdecl("satchel_mobile_response_handled")
public func satchel_mobile_response_handled(_ responseId: UnsafePointer<CChar>) {
    routeGate.acknowledge(String(cString: responseId))
}
