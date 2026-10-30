// Compile with the production decoder/gate. No notification is scheduled.
import Foundation

func decoded(_ info: [AnyHashable: Any], action: String = "tap", input: String? = nil) -> [String: Any] {
    guard case .route(let route) = SatchelNotificationResponse.decode(
        category: "satchel.mail", userInfo: info, actionId: action, inputValue: input
    ) else { fatalError("Expected a mail route for \(info)") }
    return route
}

// The exact persisted format written by notification 2.3.3 makeNotificationContent.
let pluginInfo: [AnyHashable: Any] = ["__EXTRA__": ["threadId": "thread-42"]]
assert(decoded(pluginInfo)["threadId"] as? String == "thread-42")
for legacy: [AnyHashable: Any] in [
    ["threadId": "thread-42"], ["thread_id": "thread-42"],
    ["payload": ["threadId": "thread-42"]], ["payload": ["thread_id": "thread-42"]],
    ["__EXTRA__": ["thread_id": "thread-42"]],
] {
    assert(decoded(legacy)["threadId"] as? String == "thread-42")
}
for action in ["tap", "satchel.read", "satchel.archive", "satchel.reply"] {
    let route = decoded(pluginInfo, action: action, input: "  an inline reply\n")
    assert(route["actionId"] as? String == action)
    assert(route["inputValue"] as? String == "  an inline reply\n")
    assert(route["type"] as? String == "notification-tap")
}
for malformed: [AnyHashable: Any] in [
    [:], ["__EXTRA__": "not a dictionary"], ["__EXTRA__": ["threadId": 42]],
    ["threadId": NSNull()], ["payload": ["threadId": " \n\t"]],
] {
    guard case .ignored = SatchelNotificationResponse.decode(
        category: "satchel.mail", userInfo: malformed, actionId: "tap"
    ) else { fatalError("Malformed payload was accepted") }
}
for category in ["", "other.app", "other.category"] {
    guard case .unrelated = SatchelNotificationResponse.decode(
        category: category, userInfo: pluginInfo, actionId: "satchel.read"
    ) else { fatalError("Unrelated notification was claimed") }
}
for action in ["dismiss", "com.apple.UNNotificationDismissActionIdentifier", "other.action"] {
    guard case .ignored = SatchelNotificationResponse.decode(
        category: "satchel.mail", userInfo: pluginInfo, actionId: action
    ) else { fatalError("Dismiss or unknown action navigated") }
}

var delivered: [String] = []
var completed: [String] = []
var responseIds: [String] = []
let gate = SatchelRouteGate { route in
    delivered.append(route["actionId"] as! String)
    responseIds.append(route["responseId"] as! String)
}
let actions = ["tap", "satchel.read", "satchel.archive", "satchel.reply"]
// Cold launch: no frontend or plugin action listener exists yet. Multiple
// responses must survive, rather than overwriting a single pending route.
for action in actions {
    gate.queue(decoded(pluginInfo, action: action)) {
        assert(delivered.contains(action), "Completion must follow dispatch")
        completed.append(action)
    }
}
assert(delivered.isEmpty && completed.isEmpty)
gate.markFrontendReady()
assert(delivered == actions && completed.isEmpty, "Dispatch must not complete background actions")
for id in responseIds { gate.acknowledge(id) }
assert(completed == actions)
for id in responseIds { gate.acknowledge(id) }
gate.acknowledge("unknown-response")
gate.markFrontendReady()
assert(delivered == actions && completed == actions, "Repeated ACK/readiness must not redeliver or complete twice")
// Warm launch: deliver immediately through the same gate, once per response.
for action in actions {
    gate.queue(decoded(pluginInfo, action: action)) {
        assert(delivered.contains(action), "Completion must follow dispatch")
        completed.append(action)
    }
}
assert(delivered == actions + actions && completed == actions)
for id in responseIds.suffix(actions.count) { gate.acknowledge(id) }
assert(completed == actions + actions)

// Readiness before any response, and reentrant delivery, preserve ordering.
var reentrant: [String] = []
var reentrantGate: SatchelRouteGate!
reentrantGate = SatchelRouteGate { route in
    let action = route["actionId"] as! String
    reentrant.append(action)
    if action == "satchel.read" { reentrantGate.queue(decoded(pluginInfo, action: "satchel.reply")) }
}
reentrantGate.markFrontendReady()
reentrantGate.queue(decoded(pluginInfo, action: "satchel.read"))
assert(reentrant == ["satchel.read", "satchel.reply"])

// Racing native response callbacks must not duplicate or lose events.
let deliveredLock = NSLock()
var ids = Set<Int>()
var count = 0
var completions = 0
var concurrentGate: SatchelRouteGate!
concurrentGate = SatchelRouteGate { route in
    deliveredLock.lock()
    ids.insert(route["id"] as! Int)
    count += 1
    deliveredLock.unlock()
    concurrentGate.acknowledge(route["responseId"] as! String)
}
DispatchQueue.concurrentPerform(iterations: 1000) { id in
    concurrentGate.queue(["id": id]) {
        deliveredLock.lock()
        assert(ids.contains(id))
        completions += 1
        deliveredLock.unlock()
    }
    if id % 10 == 0 { concurrentGate.markFrontendReady() }
}
concurrentGate.markFrontendReady()
assert(count == 1000 && ids.count == 1000 && completions == 1000)
print("iOS notification payload and cold/warm delivery regressions passed")
