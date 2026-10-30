import Foundation

/// The notification plugin's iOS content stores `extra` under `__EXTRA__`,
/// but its actionPerformed event omits it. Decode the persisted content instead.
enum SatchelNotificationResponse {
    case unrelated
    case ignored
    case route([String: Any])

    static func decode(
        category: String,
        userInfo: [AnyHashable: Any],
        actionId: String,
        inputValue: String? = nil
    ) -> SatchelNotificationResponse {
        guard category == "satchel.mail" else { return .unrelated }
        let supported = ["tap", "satchel.read", "satchel.archive", "satchel.reply"]
        guard supported.contains(actionId) else { return .ignored }

        // Also accept the direct and payload dictionaries used by earlier builds.
        let extra = userInfo["__EXTRA__"] as? [String: Any]
        let payload = userInfo["payload"] as? [String: Any]
        let extraId = extra?["threadId"] ?? extra?["thread_id"]
        let directId = userInfo["threadId"] ?? userInfo["thread_id"]
        let payloadId = payload?["threadId"] ?? payload?["thread_id"]
        let threadId = extraId ?? directId ?? payloadId
        guard let threadId = threadId as? String,
              !threadId.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            return .ignored
        }
        var route: [String: Any] = [
            "type": "notification-tap", "threadId": threadId, "actionId": actionId,
        ]
        if let satchel = userInfo["satchel"] as? [String: Any],
           let accountId = satchel["accountId"] as? String {
            route["accountId"] = accountId
        }
        if let inputValue { route["inputValue"] = inputValue }
        return .route(route)
    }
}

/// Delivers every response once, in order, after the frontend has a listener.
/// Keep the OS response alive until the frontend finishes the action. Emission
/// and completion happen outside the lock so callbacks can enqueue a response.
final class SatchelRouteGate {
    private let lock = NSLock()
    private var frontendReady = false
    private var pendingRoutes: [[String: Any]] = []
    private var completions: [String: () -> Void] = [:]
    private var draining = false
    private let emit: ([String: Any]) -> Void

    init(emit: @escaping ([String: Any]) -> Void) {
        self.emit = emit
    }

    func queue(_ route: [String: Any], completion: @escaping () -> Void = {}) {
        lock.lock()
        let responseId = UUID().uuidString
        var event = route
        event["responseId"] = responseId
        completions[responseId] = completion
        pendingRoutes.append(event)
        drain()
    }

    func markFrontendReady() {
        lock.lock()
        frontendReady = true
        drain()
    }

    func acknowledge(_ responseId: String) {
        lock.lock()
        let completion = completions.removeValue(forKey: responseId)
        lock.unlock()
        completion?()
    }

    /// Called with the lock held; always releases it before returning.
    private func drain() {
        guard frontendReady, !draining else {
            lock.unlock()
            return
        }
        draining = true
        while !pendingRoutes.isEmpty {
            let route = pendingRoutes.removeFirst()
            lock.unlock()
            emit(route)
            lock.lock()
        }
        draining = false
        lock.unlock()
    }
}
