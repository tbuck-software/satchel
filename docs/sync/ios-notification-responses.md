# iOS notification responses

The mail notification delegate installs synchronously after the notification
plugin. It decodes local `satchel.mail` responses itself, including persisted
`__EXTRA__` and legacy thread IDs. Mail never enters the plugin's response
handler, which relies on process-local notification records. Unrelated
notifications and foreground presentation still go through the previous delegate.

The Swift initializer belongs to the final Xcode executable. Rust resolves its
C entry point at plugin setup and fails setup if it is missing. A Rust ABI wrapper
passes the returned NSObject pointer to Tauri's registration API, with the same
ownership convention as the pinned notification plugin. There is no link-time
initializer reference from the Rust cdylib. The generated `main.mm` retains a
table of public Swift C entry points using Clang's
[`used` attribute](https://clang.llvm.org/docs/AttributeReference.html#used).
The table never calls the initializer. Simulator CI checks the final executable's
export trie, in addition to compiling the integration.

`mobile_notifications_frontend_ready` reserves a native listener lease when
`listenerId` is null. The frontend registers its route callback, then activates
that lease with `ready: true`. Disposal retires it with `ready: false`. Native
lease numbers only increase; delayed readiness or disposal from an older
bootstrap cannot revive it or stop a newer listener. Lease IPC is serialized
within the frontend realm, including reservations that resolve after disposal.
Android and desktop return no lease and keep their existing notification paths.

The gate retains response payloads and OS completion closures until ACK.
It emits unacknowledged responses in native arrival order once per active lease.
A new active lease replays even responses already emitted to an old listener.
An emission racing disposal may reach no callback; it remains recoverable.
Callbacks reject other leases and disposed owners. Within the same composer
store, replay joins in-flight work or re-ACKs completed work without repeating
navigation, composer replacement, or shared thread actions. Completed IDs remain
in memory for the store's lifetime to tolerate lost ACKs and queued duplicates.
ACKs are response-scoped, so an old handler can finish after takeover. Native
completion runs outside the lock and only once, including repeated ACKs.

Reply opens a visible composer and awaits its replacement guard. Refusal or a
superseding composer request cancels that reply without dismissing notifications.
An accepted reply carries the notification text into the composer before ACK.
A thrown action or draft-save error sends no ACK; native retains the response
and reply text for retry on the next listener lifetime. A dismissal error after
a successful open leaves the text in the composer and does not repeat the reply.

These are in-process guarantees. Orderly bootstrap disposal/recreation with the
same stores preserves in-flight work and deduplication. A fresh WebView can
replay native unacknowledged responses, but has lost the old store's deduplication
history and cannot guarantee exactly-once JS side effects. Process termination
loses this native queue and its OS closures too. Cold launch is supported for a
response actually handed to the new process by iOS, not as a durable action queue.
