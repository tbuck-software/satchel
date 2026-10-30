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

/// In-process response retention. A listener lease is reserved before JS registers
/// and activated afterwards. Replacing or disposing it stops delivery; every
/// unacknowledged response is replayed in native order to the next active lease.
/// Neither this queue nor the OS completion closures survive process termination.
final class SatchelRouteGate {
    private struct Response {
        let id: String
        let route: [String: Any]
        let completion: () -> Void
        var deliveredTo: UInt32 = 0
    }

    private let lock = NSLock()
    private var generation: UInt32 = 0
    private var active = false
    private var responses: [Response] = []
    private var draining = false
    private let emit: ([String: Any]) -> Void

    init(emit: @escaping ([String: Any]) -> Void) {
        self.emit = emit
    }

    func beginListener() -> UInt32 {
        lock.lock()
        defer { lock.unlock() }
        generation += 1
        active = false
        return generation
    }

    func setListener(_ id: UInt32, ready: Bool) {
        lock.lock()
        guard id == generation else {
            lock.unlock()
            return
        }
        // Retire disposed leases so a late ready cannot resurrect them.
        if !ready {
            generation += 1
        }
        active = ready
        drain()
    }

    func queue(_ route: [String: Any], completion: @escaping () -> Void = {}) {
        lock.lock()
        responses.append(Response(id: UUID().uuidString, route: route, completion: completion))
        drain()
    }

    func acknowledge(_ responseId: String) {
        lock.lock()
        let index = responses.firstIndex { $0.id == responseId }
        let completion = index.map { responses.remove(at: $0).completion }
        lock.unlock()
        // An older listener may finish after a new lease took ownership.
        // ACK is response-scoped and can complete each OS callback only once.
        completion?()
    }

    /// Called with the lock held; always releases it before returning. Recheck
    /// the lease after each emission, including reentrant disposal/registration.
    private func drain() {
        guard active, !draining else {
            lock.unlock()
            return
        }
        draining = true
        while active, let index = responses.firstIndex(where: { $0.deliveredTo != generation }) {
            responses[index].deliveredTo = generation
            var route = responses[index].route
            route["responseId"] = responses[index].id
            route["listenerId"] = generation
            lock.unlock()
            emit(route)
            lock.lock()
        }
        draining = false
        lock.unlock()
    }
}
