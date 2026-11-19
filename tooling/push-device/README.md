# Android push devices

Satchel Desktop can pair directly under **Settings → Notifications → Make this computer visible**.
On Android choose **Find computer**, select the computer, compare the six digits
and approve on the computer. Both devices need Internet access. They share a
local network during discovery. The phone can receive subsequent signals on
mobile data. Additional computers pair independently.

For a headless NAS with the helper installed, run `device.py visible` with the
helper's Python interpreter. On first use it asks for the IMAP inboxes to watch.
Confirm the displayed code in the SSH session. The installer accepts `visible`
as its first argument for this flow. The NAS holds its IMAP cursor in memory
and writes its pairing credentials at enrollment. This mode uses the shared relay.

### Configuring a NAS from your computer

In Satchel settings open **Notifications → Set up a headless NAS** to copy the
installer command. Run it in an interactive SSH session to the NAS. The NAS
needs Python 3 with venv, curl, and an Internet connection. With no local Satchel
profile, the helper asks for an IMAP server, login and password source on that
machine. It can read an existing password file accessible to the service user
or store the password in a working OS keyring. Restrict a password file to the
service user. Existing accounts on your computer or phone are never exported
by the installer. Each Satchel client continues to access its own mail provider.

On Linux with a systemd user manager, the installer enables and starts
`satchel-push.service`. For operation after logout and at boot, that user's
manager needs lingering. On appliances with a startup task scheduler, use the
command printed by the installer as a boot task under the same NAS user.
You can also run that command manually over SSH in the foreground:

```sh
~/.satchel-push/venv/bin/python ~/.satchel-push/device.py run
```

Use `--pair-only` with `visible` when another process manager starts the
watcher. A NAS reboot only requires restarting the watcher. Pairing and mailbox
settings persist.

## Private Firebase setup

In Satchel on Android, open **Settings → Notifications → Own push service → Add device**. Run the
copied command on the computer that will watch your inboxes, locally or in an
interactive SSH session. Confirm the computer's name on the phone. The first
wake signal verifies delivery. Both devices need to be on the same reachable
network for pairing. After that, both use outbound internet connections.

The installer requires macOS or Linux, Python 3 with venv, and curl. It
installs under `~/.satchel-push`. On macOS the watcher runs as a login agent.
On Linux it installs a systemd user service. Enable lingering if the service
must survive logout. On a NAS without a user service manager the installer
prints the command for its startup task.

## First computer

Each installation uses its owner's Firebase project. FCM is available on the
free Spark plan. Satchel has no project-wide sending credential in the APK or
installer. On a computer with the Google Cloud and Firebase CLIs installed,
the first run can create and configure a private project. Google may ask you to
sign in. The Google account on the Android phone is independent of that login.

For an existing project, register the Android package `de.tbuck.satchel`,
enable Cloud Messaging, and save the Android configuration as
`~/.satchel-push/google-services.json`. The sender uses Google Application
Default Credentials. A service account for the watcher needs
`cloudmessaging.messages.create` in that project. Use workload identity where
available, or point `GOOGLE_APPLICATION_CREDENTIALS` to a protected credential
file on the watcher. Keep this file private. The Android configuration contains
the public client settings the phone needs.

## Mail sources and additional devices

With Satchel Desktop running, the watcher observes inbox inserts in its local
profile. This covers Gmail API and IMAP accounts already configured there.
Desktop notifications may be muted. Set `SATCHEL_PUSH_DB` for another profile.

Without a desktop profile, or with `SATCHEL_PUSH_IMAP=1`, the installer asks
for the inboxes to watch. It uses read-only IMAP over TLS. It keeps UID cursors
in memory and uses IDLE when supported. Passwords come from the OS keyring or a
password file you provide. Gmail also supports this mode for accounts eligible
for an app password. The watcher reads mailbox status without downloading
messages. Idle connections and retries do not write heartbeat files.

Use **Add device** again for a second computer. Its command carries the
existing Google project ID. Authorize that computer to send through the same
project. Each computer can independently wake the phone. Satchel fetches mail
and deduplicates notifications using its local notification outbox.

## Testing and removal

Run `~/.satchel-push/venv/bin/python ~/.satchel-push/device.py test` to send a
wake signal. A signal triggers a fetch. A mail notification appears when that
fetch finds eligible new mail. Android's notification permission, account
connectivity and notification settings still apply.

**Remove** in Satchel revokes that computer's ability to trigger a fetch on
the phone. To stop the watcher, run
`launchctl bootout gui/$(id -u)/de.tbuck.satchel.push-device` on macOS or
`systemctl --user disable --now satchel-push` on Linux. Remove its launch
agent or unit and private configuration when retiring the computer.
