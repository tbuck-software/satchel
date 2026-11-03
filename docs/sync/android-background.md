# Android background sync

## Scheduling and delivery

The foreground process keeps its provider connections alive while Android
allows it to run. `SatchelSyncService` supplies the persistent foreground
notification. It returns `START_NOT_STICKY`; periodic work is the recovery path
when that process stops.

`BackgroundMailSyncWorkScheduler` registers unique periodic work named
`satchel-background-mail-sync-v1`. It requests a 15-minute interval, a connected
network, and exponential retry backoff starting at 30 seconds. Re-registering
uses `ExistingPeriodicWorkPolicy.UPDATE`. Boot and package-replacement receivers
restore it when background sync is enabled.

Fifteen minutes is the minimum configurable interval, not a delivery deadline.
Constraints and system scheduling can delay or skip a run. See Android's
[periodic-work documentation](https://developer.android.com/develop/background-work/background-tasks/persistent/getting-started/define-work#schedule_periodic_work).
During Doze, background network access and WorkManager jobs are deferred until
maintenance windows. See the [Doze restrictions](https://developer.android.com/training/monitoring-device-state/doze-standby#restrictions).

`BackgroundMailSyncWorker` skips provider work if the main process is serving
mail. Otherwise its separate process opens the profile under the database lock,
refreshes active inboxes, applies a bounded mobile storage-budget pass, and
delivers pending native notifications. Notification acknowledgements record
whether Android actually displayed each message. Incomplete refreshes request
up to five retries; the next periodic run remains scheduled.

The storage target is 500 MB of active SQLite pages. Each pass releases at most
64 recoverable bodies. Reusable pages remain in the database file for later
writes; mobile does not run a full VACUUM. Headers, drafts, imports, POP3 mail,
and unverified or stale IMAP identities are retained even above the target.
Released body text no longer contributes to local full-text search until the
message content is fetched again.

## Device verification still required

The following physical-device measurements remain open after the November
2026 backlog audit. Emulator results cannot establish manufacturer-specific
process-killing behavior.

1. Record device model, OS/build, app commit, battery mode and whether battery
   optimization is exempt. Use the same test account and timestamped messages.
2. Send mail while the app is visible, backgrounded, and after its process has
   stopped. Record provider arrival, worker start/end, and notification time.
3. Repeat unplugged with the screen off through a Doze maintenance window and
   after reboot. Verify the next foreground sync recovers any delayed messages.
4. Repeat with the manufacturer's default and restricted battery policies.
   Record gaps and recovery behavior rather than treating 15 minutes as a limit.
5. Send a file attachment from the platform document picker, restart the app
   while offline, and verify draft recovery and eventual delivery.
6. Run the same compose, attachment, crypto, OAuth and notification scenarios on
   iOS. The generated Swift integration and simulator-build CI cover wiring,
   but do not prove full Android/iOS behavior parity.

Implementation references are `BackgroundSync.kt`, `NotificationActionWork.kt`,
`src-tauri/src/notification_actions.rs`, and `crates/satchel-core/src/db/storage.rs`.
