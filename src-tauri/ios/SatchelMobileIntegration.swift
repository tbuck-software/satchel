import Foundation
import UIKit
import UserNotifications

@_silgen_name("satchel_mobile_event")
private func satchel_mobile_event(_ json: UnsafePointer<CChar>)

/// Holds a notification tap until the frontend is listening.
///
/// A tap can arrive while the WebView is still cold; emitting it then would
/// drop it on the floor. The route waits here and `satchel_mobile_frontend_ready`
/// releases it.
private final class SatchelRouteGate {
    static let shared = SatchelRouteGate()
    private let lock = NSLock()
    private var frontendReady = false
    private var pendingRoute: [String: Any]?

    func queue(_ route: [String: Any]) {
        lock.lock()
        pendingRoute = route
        let ready = frontendReady
        lock.unlock()
        if ready { flush() }
    }

    func markFrontendReady() {
        lock.lock()
        frontendReady = true
        lock.unlock()
        flush()
    }

    private func flush() {
        lock.lock()
        guard frontendReady, let route = pendingRoute else {
            lock.unlock()
            return
        }
        pendingRoute = nil
        lock.unlock()
        SatchelMobileIntegration.emit(route)
    }
}

private final class SatchelMobileIntegration: NSObject, UNUserNotificationCenterDelegate {
    static let shared = SatchelMobileIntegration()
    private var observers: [NSObjectProtocol] = []
    private var installed = false

    func install() {
        guard !installed else { return }
        installed = true
        UNUserNotificationCenter.current().delegate = self
    }

    func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        willPresent notification: UNNotification,
        withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void
    ) {
        completionHandler([.banner, .list, .sound, .badge])
    }

    func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        didReceive response: UNNotificationResponse,
        withCompletionHandler completionHandler: @escaping () -> Void
    ) {
        routeNotification(response.notification.request.content.userInfo)
        completionHandler()
    }

    private func routeNotification(_ userInfo: [AnyHashable: Any]) {
        let payload = userInfo["payload"] as? [String: Any]
        let threadId = (userInfo["threadId"] as? String)
            ?? (userInfo["thread_id"] as? String)
            ?? (payload?["threadId"] as? String)
            ?? (payload?["thread_id"] as? String)
        guard let threadId, !threadId.isEmpty else { return }
        let satchel = userInfo["satchel"] as? [String: Any]
        var route: [String: Any] = ["type": "notification-tap", "threadId": threadId]
        if let accountId = satchel?["accountId"] as? String {
            route["accountId"] = accountId
        }
        SatchelRouteGate.shared.queue(route)
    }

    static func emit(_ value: [String: Any]) {
        guard JSONSerialization.isValidJSONObject(value),
              let data = try? JSONSerialization.data(withJSONObject: value, options: [.sortedKeys]),
              let json = String(data: data, encoding: .utf8) else { return }
        json.withCString { satchel_mobile_event($0) }
    }
}

@_cdecl("satchel_mobile_install")
public func satchel_mobile_install() {
    SatchelMobileIntegration.shared.install()
}

@_cdecl("satchel_mobile_frontend_ready")
public func satchel_mobile_frontend_ready() {
    SatchelRouteGate.shared.markFrontendReady()
}
