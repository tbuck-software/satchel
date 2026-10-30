# iOS notification responses

The mail notification delegate installs synchronously after the notification
plugin. It decodes local `satchel.mail` responses itself, including persisted
`__EXTRA__` and legacy thread IDs. Mail never enters the plugin's response
handler, which relies on process-local notification records. Unrelated
notifications and foreground presentation still go through the previous delegate.

The delegate registers as a Tauri `Plugin`, so its Swift imports the `Tauri`
module. That module exists only inside the Rust build: tauri's build script
links it into libapp.a. The Xcode target `tauri ios init` generates links
libapp.a and has no `Tauri` module, so Swift compiled there fails with
"no such module 'Tauri'". The sources are therefore a Swift package,
`src-tauri/ios/Package.swift`, which `src-tauri/build.rs` builds and links into
libapp.a the way plugin crates link theirs. It resolves `Tauri` from
`src-tauri/.tauri/tauri-api`, a copy of the package tauri publishes as
`DEP_TAURI_IOS_LIBRARY_PATH`. `scripts/apply-ios-integration.mjs` copies no
Swift into the Xcode target and fails if a Swift file there imports `Tauri`.

Rust resolves the initializer's C entry point at plugin setup and fails setup
if it is missing. A Rust ABI wrapper passes the returned NSObject pointer to
Tauri's registration API, with the same ownership convention as the pinned
notification plugin. There is no link-time initializer reference from Rust.
The linker loads an archive member only when something references it, so the
generated `main.mm` holds a table of the public Swift C entry points, retained
with Clang's [`used` attribute](https://clang.llvm.org/docs/AttributeReference.html#used).
That table is what carries the package into the app's code image; it never
calls the initializer. Simulator CI checks that image's export trie, in
addition to compiling the integration. Xcode 16 Debug builds link the code into
`Satchel.app/Satchel.debug.dylib` (`ENABLE_DEBUG_DYLIB`) and leave `Satchel` a
stub that loads it; `dlsym(RTLD_DEFAULT, …)` searches that dylib too.
`scripts/verify-ios-hook-exports.mjs` takes the image from
`xcodebuild -showBuildSettings`: the debug dylib when one was built, otherwise
the executable. That dylib exports every symbol of libapp.a, more than a 64 MiB
output buffer holds, so the script reads `dyld_info -exports` line by line and
keeps only the hooks. It reports every missing hook and fails on any, and on a
failed `dyld_info`.

`mobile_notifications_frontend_ready` reserves a native listener lease when
`listenerId` is null. The frontend registers its route callback, then activates
that lease with `ready: true`. Disposal retires it with `ready: false`. Native
lease numbers only increase; delayed readiness or disposal from an older
bootstrap cannot revive it or stop a newer listener. Lease IPC is serialized
within the frontend realm, including reservations that resolve after disposal.
Android and desktop return no lease and keep their existing notification paths.
A failed reservation, activation or retirement reports through the connection
`notifications` error source. The banner's Retry retires the current lease and
connects a new one, registering the route callback only if it is still missing;
a successful activation clears the error, unless a response is still held (see
below). Resume, Android navigation and the
focus/visibility handlers are registered separately and survive a lease failure.

The gate retains response payloads and OS completion closures until ACK. The
frontend ACKs through the `mobile_notification_response_handled` command
(`backendMobileNotificationResponseHandled`), which reaches native through the
same `dlsym` path. A failed ACK rejects in the frontend and reports through the
`notifications` source; completed work stays deduplicated, so the next delivery,
for example on the lease Retry connects, ACKs it again.
It emits unacknowledged responses in native arrival order once per active lease.
A new active lease replays even responses already emitted to an old listener.
An emission racing disposal may reach no callback; it remains recoverable.
Callbacks reject other leases and disposed owners. Within the same composer
store, replay joins in-flight work or re-ACKs completed work without repeating
navigation, composer replacement, or shared thread actions. Completed IDs remain
in memory for the store's lifetime to tolerate lost ACKs and queued duplicates.
A failed action is forgotten instead, so the next delivery runs it again.
ACKs are response-scoped, so an old handler can finish after takeover. Native
completion runs outside the lock and only once, including repeated ACKs.

Reply opens a visible composer and awaits its replacement guard. The composer
store decides open requests one at a time, in arrival order: a second reply
waits for the answer to the first instead of taking its question away, then
meets the composer that answer left open. `App.vue` loads `ComposerHost` as an
async chunk once the first request is open, and the host registers its guard
when it mounts. A request that would replace an `unsavedBody` composer before
that waits for the guard, up to `REPLACE_GUARD_WAIT_MS` (10 s), instead of
replacing it unasked; a cold start's first lease can replay two replies in one
burst. If no guard comes, the request ends `interrupted`. A refusal (`kept`) cancels that reply
without dismissing notifications. An accepted reply carries the notification
text into the composer before ACK. The request is marked `unsavedBody`: the form
measures edits from what it opened with, so it calls the reply untouched, but
until its first save the text exists nowhere else. The host therefore asks and
saves it before any replacement, as it does for an edited draft.
Two outcomes are failures, not refusals: an accepted replacement whose open
draft cannot be saved (`unsaved`), and a question nobody answered
(`interrupted`: another dialog took it away, or no host came up to ask). Neither
sends an ACK, and neither does a thrown action; native retains the response and
reply text. A dismissal error after a successful open leaves the text in the
composer and does not repeat the reply.

Native delivers a retained response again only to a new active lease, so a
response the frontend could not ACK is held: the bootstrap re-issues the lease
once whatever stopped it is out of the way, in the running app.

- `interrupted`: when the dialog that took the question closes, or the composer
  closes. A toast says the reply is kept.
- `unsaved`: when the composer closes, for example after the save-failed
  toast's Retry saved the draft. Another dialog closing does not count; the
  draft still could not be saved.
- A failed read or archive: after `NOTIFICATION_RETRY_DELAY_MS` (15 s) times
  the deliveries it has failed so far, so the third and last delivery comes
  about 45 s after the first.
- A reply whose action threw: only through the banner's Retry.

The composer counts as closed only if it is still closed after the current
task. A replacement closes it on the way to the request the user asked for, and
a held reply must not take that one over.

These are transitions, not states: each one re-issues the lease at most once,
so a response that keeps failing waits for the next thing the user does, or
for the next pause. A re-issue never starts while a confirm dialog is open,
because a replayed reply would ask its question and take that dialog away; it
waits for the dialog to close. While a response is held, the `notifications`
source reports `backend.errors.notificationResponseWaiting`, so the banner and
its Retry, which re-issues the lease, are there as a fallback. The report clears
when the last held response is ACKed.

Read and archive ACK only on the outcome `useThreadActions()` reports: `setRead`
resolving `true`, and `archive` resolving `true` once the thread is Done, also
when it already was. A rejected write or a rollback resolves `false`; the
response then stays unacknowledged for the next delivery.

`ensureThreadLoaded` tells a missing thread from a failed load. When the backend
answers `backend.errors.threadNotFound`, nothing a retry does can change that:
the response is ACKed, the notification dismissed, and a toast says the mail is
gone. Any other load failure counts as transient. A read or archive gets
`NOTIFICATION_ACTION_ATTEMPTS` (3) deliveries per response; the third failure
is ACKed with a toast saying the action could not be completed, instead of
replaying on every lease for the process lifetime. The paused re-issues above
reach that cap in a running app; Retry, a restart or any other re-issue counts
toward it as well. The count lives with the
composer store, like deduplication. A reply is never given up: its text exists
nowhere else, and only process termination ends its retention.

These are in-process guarantees. Orderly bootstrap disposal/recreation with the
same stores preserves in-flight work and deduplication. A fresh WebView can
replay native unacknowledged responses, but has lost the old store's deduplication
history and cannot guarantee exactly-once JS side effects. Process termination
loses this native queue and its OS closures too. Cold launch is supported for a
response actually handed to the new process by iOS, not as a durable action queue.
