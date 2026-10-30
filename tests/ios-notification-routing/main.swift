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
let owner = gate.beginListener()
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
gate.setListener(owner, ready: true)
assert(delivered == actions && completed.isEmpty, "Dispatch must not complete background actions")
for id in responseIds { gate.acknowledge(id) }
assert(completed == actions)
for id in responseIds { gate.acknowledge(id) }
gate.acknowledge("unknown-response")
gate.setListener(owner, ready: true)
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
reentrantGate.setListener(reentrantGate.beginListener(), ready: true)
reentrantGate.queue(decoded(pluginInfo, action: "satchel.read"))
assert(reentrant == ["satchel.read", "satchel.reply"])

// An emitted but unacknowledged response and a response received while
// disposed both replay to the next listener, retaining IDs and FIFO order.
var leases: [UInt32] = []
var replayed: [String] = []
var replayCompletions = 0
let replayGate = SatchelRouteGate { route in
    leases.append(route["listenerId"] as! UInt32)
    replayed.append(route["responseId"] as! String)
}
let old = replayGate.beginListener()
replayGate.queue(["id": 1]) { replayCompletions += 1 }
replayGate.setListener(old, ready: true)
let firstId = replayed[0]
replayGate.setListener(old, ready: false)
replayGate.queue(["id": 2]) { replayCompletions += 1 }
replayGate.setListener(old, ready: true)
assert(replayed == [firstId], "Retired readiness must not resurrect a listener")
let next = replayGate.beginListener()
assert(replayed == [firstId], "Reservation alone must not emit")
replayGate.setListener(next, ready: true)
assert(replayed.count == 3 && replayed[1] == firstId)
assert(leases == [old, next, next])
replayGate.setListener(old, ready: false)
replayGate.setListener(old, ready: true)
replayGate.setListener(next, ready: true)
assert(replayed.count == 3, "Stale lifecycle messages must not affect a new owner")
// An old in-flight handler can finish after takeover; repeated ACKs are safe.
replayGate.acknowledge(firstId)
replayGate.acknowledge(firstId)
replayGate.acknowledge(replayed[2])
assert(replayCompletions == 2)
let finalOwner = replayGate.beginListener()
replayGate.setListener(finalOwner, ready: true)
assert(replayed.count == 3, "Acknowledged responses must not replay")

// Dispose/recreate reentrantly during a drain. The new lease gets the retained
// head before subsequent responses, and an old dispose cannot stop that drain.
var switched = false
var switchingDeliveries: [Int] = []
var switchingGate: SatchelRouteGate!
var switchingOwner: UInt32 = 0
switchingGate = SatchelRouteGate { route in
    switchingDeliveries.append(route["id"] as! Int)
    if !switched {
        switched = true
        switchingGate.setListener(switchingOwner, ready: false)
        let replacement = switchingGate.beginListener()
        switchingGate.setListener(replacement, ready: true)
        switchingGate.setListener(switchingOwner, ready: false)
    } else {
        switchingGate.acknowledge(route["responseId"] as! String)
    }
}
switchingOwner = switchingGate.beginListener()
switchingGate.queue(["id": 1])
switchingGate.queue(["id": 2])
switchingGate.setListener(switchingOwner, ready: true)
assert(switchingDeliveries == [1, 1, 2])

// Completing a response can queue and ACK another one without holding the lock.
var completionReentrancy = 0
var completionGate: SatchelRouteGate!
completionGate = SatchelRouteGate { route in
    completionGate.acknowledge(route["responseId"] as! String)
}
completionGate.setListener(completionGate.beginListener(), ready: true)
completionGate.queue(["id": 1]) {
    completionReentrancy += 1
    completionGate.queue(["id": 2]) { completionReentrancy += 1 }
}
assert(completionReentrancy == 2)

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
let concurrentOwner = concurrentGate.beginListener()
DispatchQueue.concurrentPerform(iterations: 1000) { id in
    concurrentGate.queue(["id": id]) {
        deliveredLock.lock()
        assert(ids.contains(id))
        completions += 1
        deliveredLock.unlock()
    }
    if id % 10 == 0 { concurrentGate.setListener(concurrentOwner, ready: true) }
}
concurrentGate.setListener(concurrentOwner, ready: true)
assert(count == 1000 && ids.count == 1000 && completions == 1000)
// Concurrent reservation, stale ready/dispose and ACK calls leave every route
// recoverable. The final owner drains anything emitted without an ACK earlier.
let racingLock = NSLock()
var racingCompletions = 0
var racingGate: SatchelRouteGate!
racingGate = SatchelRouteGate { route in
    racingGate.acknowledge(route["responseId"] as! String)
    racingGate.acknowledge(route["responseId"] as! String)
}
DispatchQueue.concurrentPerform(iterations: 1000) { id in
    let lease = racingGate.beginListener()
    racingGate.queue(["id": id]) {
        racingLock.lock()
        racingCompletions += 1
        racingLock.unlock()
    }
    racingGate.setListener(lease, ready: true)
    racingGate.setListener(lease, ready: false)
    racingGate.setListener(lease, ready: true)
}
racingGate.setListener(racingGate.beginListener(), ready: true)
assert(racingCompletions == 1000)
print("iOS notification payload, listener lifetime, replay and concurrency regressions passed")
