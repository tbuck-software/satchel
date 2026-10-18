# Release & Distribution Guide

This document covers everything required to ship Satchel to end users on macOS
(primary target) and what to do on first-launch when you don't have an Apple
Developer account yet.

## TL;DR

```sh
# Dev channel (hot reload, isolated app data)
pnpm run tauri:dev

# Daily-driver channel (release build, isolated from dev and stable)
pnpm run install:daily:mac

# Stable local install
pnpm run install:mac

# Production build with signing + notarization (recommended for distribution)
APPLE_ID="you@example.com" \
APPLE_PASSWORD="app-specific-password" \
APPLE_TEAM_ID="YOURTEAMID" \
APPLE_SIGNING_IDENTITY="Developer ID Application: Your Name (YOURTEAMID)" \
pnpm run release:mac
```

The bundle ends up in `target/release/bundle/macos/Satchel.app` and as
a `.dmg` in `target/release/bundle/dmg/`.

## Channels

Use separate channels so development cannot silently mutate the data used by
your daily mail client.

| Channel | Command | App | Identifier / AppData |
|---|---|---|---|
| Dev | `pnpm run tauri:dev` | `Satchel Dev` | `de.tbuck.satchel.dev` |
| Daily | `pnpm run install:daily:mac` | `/Applications/Satchel Daily.app` | `de.tbuck.satchel.daily` |
| Stable | `pnpm run install:mac` / `pnpm run release:mac` | `/Applications/Satchel.app` | `de.tbuck.satchel` |

The daily channel is the intended dogfood build: it is compiled as a release
binary, installed into `/Applications`, and has its own AppData directory. The
installer copies the local `.env` into that channel's AppData directory, but
release binaries ignore that runtime file. Public OAuth client IDs and redirect
URIs are compiled from the build environment or the build-time `.env`. Google's
Desktop app registration also requires its installed-app `client_secret`, carried
as `SATCHEL_GMAIL_DESKTOP_CLIENT_SECRET`. This is public client metadata, as
[Google documents](https://developers.google.com/identity/protocols/oauth2/#installed).
Confidential Web application client secrets must never be shipped. Generic
`SATCHEL_*_CLIENT_SECRET` inputs remain development-only.
The installer makes a consistent SQLite `.backup` snapshot before replacing the
app. It also saves the credential master key in a separate Keychain item and
writes its account name to `master-key.keychain` in the backup directory.
Restore through Backup & Export uses that item. Automatic snapshots depend on
this Mac's Keychain; use a passphrase-protected app backup for transfer to
another machine. Pruning an automatic backup also deletes its Keychain snapshot.
For a portable backup, use Settings → Backup & Export: the database and its
master key must travel together, with the key wrapped in a user passphrase.
`VACUUM INTO` or `sqlite3 .backup` can produce a database snapshot, but that
file alone cannot recover encrypted credentials. Never copy the database and its
WAL/SHM files separately.

Stable installs require a clean `main` checkout. Use the daily channel for
feature branches. `node scripts/install-mac.mjs --channel stable
--allow-unstable-stable-install` explicitly overrides that guard and can expose
the stable database to experimental migrations.

A successful clean stable install records a local annotated `v<version>` tag.
Reinstalling that commit is allowed; another commit needs a version bump rather
than moving the tag. Override installs are unversioned. Before a release, update
`CHANGELOG.md` and keep the versions in `package.json`, `src-tauri/Cargo.toml`,
and `src-tauri/tauri.conf.json` aligned, then regenerate the lockfiles. Push the
release tag explicitly after reviewing the installed build.

`make import-daily-data` snapshots the existing Daily database and key first,
then copies the Stable master key into Daily's profile-specific Keychain entry
and verifies the read-back before replacing the database. It accepts legacy
`pgp-master-key-v1` files, retaining an owner-only file fallback if Keychain is
unavailable. A missing or unreadable keychain-only key stops the import. The
scripts pass keys to `security -i` over stdin, capture both output streams, and
never include key material in process arguments or diagnostic messages.

## Upgrade notes

macOS asks once for Keychain access when Satchel first reads the migrated key.
Click "Always Allow". Ad-hoc-signed Daily builds made with `--no-sign` may ask
again after each new build because the code signature changes. Click "Always
Allow" again for the new build.

The active desktop key lives under service `satchel`, account
`pgp-master-key-v1:<sha256 of the profile directory's native path bytes>`.
macOS hashes UTF-8 bytes without normalizing the path. Stable, Daily and Dev
therefore have separate entries. Satchel migrates the legacy plaintext
`pgp-master-key-v1` file and deletes it only after an identical Keychain
read-back. `db::MASTER_KEY_FILES` names portable backup files and legacy fallback
locations.

Dev is intentionally separate. Use it for hot reload and unsafe iteration; do
not rely on it to represent the exact release runtime.

## First-Launch on macOS without Notarization

If you skip Apple Developer signing, macOS Gatekeeper will block the app on
first launch with `"Satchel" cannot be opened because the developer cannot be
verified`.

There are three ways to bypass:

1. Right-click the .app → "Open" → confirm in the dialog (only required once).
2. From terminal: `xattr -dr com.apple.quarantine /Applications/Satchel.app`
3. System Settings → Privacy & Security → "Open anyway" after the first failed
   launch.

For a public release this is unacceptable — get an Apple Developer Program
membership ($99/year), generate a Developer ID Application certificate, and use
the signed-build invocation above.

## Signing & Notarization (one-time setup)

The macOS bundle enables Hardened Runtime and uses `src-tauri/Entitlements.plist`.
Wasmtime 36 maps compiled plugin code with `mprotect`, so it needs
`com.apple.security.cs.allow-unsigned-executable-memory`. `allow-jit` alone
does not cover that allocation path. CI signs the existing WASM integration
test with these entitlements and runs a real plugin under Hardened Runtime.
Release packaging also verifies that the signed app carries the entitlement.
See the [Wasmtime platform discussion](https://github.com/bytecodealliance/wasmtime/issues/11989).

The macOS build validates the stapled notarization ticket after packaging. If
Tauri reports success without attaching a valid ticket, the workflow repeats
the bundle command up to three times, reusing the compiled binary. Retries use
`tauri bundle`, so they do not rebuild the frontend or recompile Rust.
During packaging, a scoped `xcrun` wrapper retries stapling up to three times
before Tauri creates the containers. All other `xcrun` commands go directly to
the original tool. This compensates for the pinned CLI ignoring stapling failures.
This also recreates the DMG and updater archive from the stapled app. Missing
signing timestamps and `hdiutil` resource-busy errors receive up to three
packaging attempts as well. Compiler errors and other packaging errors stop
immediately. A missing ticket after the third bundle attempt blocks publication.
CI also extracts the updater archive and mounts the DMG read-only to verify
the shipped app copies' signatures and tickets. Each final ticket check permits
three validation attempts to tolerate lost connections to Apple's service;
publication still requires successful validation.

1. Join the Apple Developer Program.
2. In Xcode: Settings → Accounts → add your Apple ID → Manage Certificates →
   create a *Developer ID Application* certificate.
3. Generate an [app-specific password](https://appleid.apple.com/account/manage)
   for `notarytool`.
4. Look up your Team ID in the [Membership portal](https://developer.apple.com/account).
5. Export the four env vars shown in the TL;DR above.
6. Run `pnpm run release:check` before the first build; it fails fast if any
   signing/notarization variable is missing, versions are inconsistent, the
   stable identifier changed, the git worktree is dirty, or any release gate fails. The gate includes i18n,
   types, frontend tests, Rust formatting and strict Clippy, both Rust crates'
   tests with built plugin components, the production build and bundle budgets,
   Chromium/WebKit smoke tests, and production CSP tests. Every Tauri command
   uses the CLI installed from `pnpm-lock.yaml`.
7. Run `pnpm run release:mac`. The Tauri build invocation
   automatically signs and submits to `notarytool`; logs land in
   `target/release/bundle/macos/notarization.log`.

## Scheduled desktop releases

The `Desktop releases` workflow builds `main` of `tbuck-software/satchel`:

- Nightly at 03:14 UTC every day. An unchanged main is skipped.
- Stable at 06:14 UTC every Monday. An unchanged main is skipped.
- `pnpm run release:nightly` or `pnpm run release:stable` dispatches the same
  workflow manually. It requires a GitHub CLI login with workflow access.

The workflow runs the complete check suite before packaging macOS arm64/x64,
Linux x64 AppImage, and Windows x64 NSIS. It uploads a release only after
all four desktop targets succeed. An incomplete build publishes no update.

Bundled WASI plugins use a separate Ed25519 plugin signature. The release-only
secret `SATCHEL_PLUGIN_SIGNING_PRIVATE_KEY` must match the public key pinned in
`src-tauri/src/plugins/registry.rs`. After building the components,
`scripts/package-release-plugins.mjs` signs them into `output/bundled-plugins`;
the unsigned development manifests never enter the release bundle. Local
packaging can use `SATCHEL_PLUGIN_SIGNING_KEY` pointing to the private PEM file.
Each native release binary checks these packages with
`--verify-plugin-packages <directory>`, which rejects unsigned components even
in debug builds. macOS repeats that check on the app, the extracted updater
archive and the app inside the mounted DMG. Missing, unsigned or modified
components stop publication. This check does not start the UI or open a profile.

The next version is the next patch after the higher of the checked-in base
version and the latest stable release. Nightlies use that version with
`-nightly.<workflow run number>`. Source version files remain the development
base; build jobs update them and the app's Cargo lock entry in their disposable
checkout. Stable tags are immutable. The rolling `nightly` prerelease contains
only a channel manifest pointing to immutable versioned, signed assets.

Preparation reserves the source tag before the long build. Publication checks
that tag against the verified source and uses the existing tag without asking
GitHub to tag an older workflow commit. A failed build can leave a tag without
a release; the next stable build of another commit takes the next free patch
version. Reserved tags are never moved. Only a complete verified release can
change an update feed.

## Install a released build

On macOS:

```sh
pnpm run install:nightly   # Satchel Daily.app, separate daily profile
pnpm run install:stable    # Satchel.app, stable profile
# Also available as make nightly / make stable.
```

No checkout or GitHub login is needed for the public standalone installer,
only Node 22 or later:

```sh
curl -fLsS https://github.com/tbuck-software/satchel/releases/latest/download/install-satchel.mjs | node --input-type=module - --channel stable --open
```

For Satchel Daily, download the installer from the latest versioned nightly
prerelease on the [releases page](https://github.com/tbuck-software/satchel/releases)
and run it with `--channel nightly --open`. From a checkout, use
`pnpm run install:nightly`.

The GitHub CLI command remains available for authenticated downloads:

```sh
gh release download --repo tbuck-software/satchel --pattern install-satchel.mjs --output - | node --input-type=module - --channel stable --open
```

The bootstrapper is bundled from the same source and carries its MIT license and updater public
key. It has no npm dependency or access token of its own.

The installer downloads a prebuilt app, verifies its Tauri/minisign signature
against the public key in the checkout, checks the bundle identifier and code
signature, asks the running app to quit, creates a consistent SQLite/keychain
backup, and replaces the app atomically. An unsuccessful draft save prevents
quit and stops installation. The app's signing identity is preserved.

While the repository is private, `gh auth login` supplies access to release
downloads. No token is embedded in an app or committed to Git. Once the
repository is public, the same installer and update URLs work anonymously.
Linux and Windows installers are available on the releases page.

## In-app updates

Packaged desktop releases check their channel two minutes after startup and
every six hours. Settings also offers a manual check. The sidebar offers a
new version, shows download progress, and asks for a restart. Updates are
never installed without that action. The backend verifies the updater signature,
then asks every composer window to save before installation. A failed save or
an unresponsive window cancels the update. Source builds and mobile apps do
not use this updater. Mobile apps are distributed through their app stores.

On macOS, the verified replacement bundle has its quarantine attribute removed
before restart. Quarantine remains enabled for mail attachments. If an older
release already runs in App Translocation, reinstall with the standalone
installer from your Mac's Terminal once. The installer recognizes the
translocated process and closes it before replacing the app and reopening it
from Applications.

Nightly uses the existing Satchel Daily identifier and profile so the two
channels remain separate. A nightly never replaces stable or shares its data.
The existing local source-build commands remain available as `make daily` and
`make install`; they do not subscribe to a release feed.

Before public release, in-app checks cannot read the private GitHub feed. Use
one of the authenticated installation commands above. After the repository is
made public, already installed release builds can offer updates without a new
build, account, or token.

## Signing and release secrets

`TAURI_SIGNING_PRIVATE_KEY` is required for every release. Its matching public
key is committed in `tauri.conf.json`. Keep a separate owner-only backup of the
private key outside the repository; changing it breaks updates for installed
clients. An optional key password uses `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.

Private macOS builds may be ad-hoc signed; the authenticated installer verifies
the independent updater signature before removing quarantine. Public macOS
releases fail closed unless Developer ID signing and notarization are configured:

- `APPLE_CERTIFICATE`, a base64 PKCS#12 distribution certificate with private key.
- `APPLE_CERTIFICATE_PASSWORD` and `APPLE_SIGNING_IDENTITY`.
- For notarization, either the App Store Connect API secrets
  `APPLE_API_KEY_P8`, `APPLE_API_KEY_ID`, and `APPLE_API_ISSUER`, or
  `APPLE_ID`, `APPLE_PASSWORD` as an app-specific password, and `APPLE_TEAM_ID`.

App Store Connect API credentials take precedence when configured. The API key
uses the Developer role and is written only to an owner-only file in the Mac
runner's temporary directory. The signing identity and key stay outside the
source checkout. Neither your Apple account password nor an interactive login
is needed by a release runner.

Updater signatures are separate from OS publisher signing. Windows NSIS
installers currently have no Authenticode signature. SmartScreen can warn about
an unknown publisher, and Windows policies can block unsigned apps. See
[Microsoft's signing guidance](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/smartscreen-reputation)
before adding a Windows code-signing service or certificate.

Download the Google OAuth **Desktop app** client JSON and store it in the
repository secret `SATCHEL_GMAIL_DESKTOP_CLIENT_CONFIG`. The workflow validates
the `installed` client type and Google token endpoint before exporting the client
ID and public desktop credential to the Rust build. Web client JSON fails closed.
Desktop Google authorization uses an ephemeral loopback port and PKCE S256.
Initial Gmail sign-in requests only `gmail.modify`, with account identity read
from Gmail's `users.getProfile`. Contacts and Calendar connect separately in
settings. Native Google clients do not support incremental authorization, so
feature connections explicitly repeat mail access and the same client's
previous optional grants. Granted scopes and the durable flow request are
checked before replacing account credentials.
Use repository variables for the public Outlook client ID and optional redirect
URI listed in `.env.example`. Outlook client secrets are never release inputs.
Google app branding and scope verification are separate provider-side steps;
publishing the GitHub repository does not complete them.

Generated dependency notices, font notices, the project license, and bundled
WASM components are included as app resources. Each release includes its
updater manifest, signatures, standalone installers, and SHA-256 checksums.

## Vendored dependencies

`lettre` is a fork under `vendor/lettre`, held in place by `[patch.crates-io]`
in the root `Cargo.toml`. A patched path is invisible to `cargo update`, so
before cutting a release, check that the fork is still current:

```sh
SATCHEL_VENDOR_UPSTREAM_CHECK=1 cargo test -p satchel-core --test vendor_lettre
```

The same test runs every Monday in the `Security audit` workflow. Reviewed drift
through 0.11.23 is temporarily accepted until 2026-11-12 UTC; a later release or
an expired exception turns the job red. If it fails, `vendor/lettre/SATCHEL_PATCH.md` says what deviates from the registry
source, how it was verified, and under which upstream fix the fork can go away
entirely.

## Frontend bundle

`pnpm run tauri:build` runs `pnpm run build` automatically, which writes
`dist/`. There's nothing additional to do for the frontend.

## Known issues on first launch

- **macOS Notification permission**: on first launch, macOS prompts for permission
  to send notifications. If denied, `notification:enabled` setting in Satchel
  will appear to do nothing — re-enable in System Settings → Notifications.
- **Tray icon**: uses the bundled `icons/icon.png`. Replace with a 22×22 @2x
  monochrome icon for a cleaner menubar appearance.
- **Gmail/Outlook freshness**: OAuth accounts use a lightweight 15-second Inbox
  poll. Generic IMAP accounts use IDLE and automatically fall back to a
  30-second Inbox poll when the server rejects IDLE.
- **GroupWise**: works through the generic IMAP provider when IMAP/SMTP is
  enabled on the GroupWise server. Some GroupWise servers expose unusual folder
  flags; Satchel treats unknown folders as `custom`.

## Development environment variables

| Variable | Effect |
|---|---|
| `SATCHEL_SEED_DEMO_DATA=1` | Explicitly seed demo account/threads in the local database (never implicit) |
| `VITE_SATCHEL_MOCK=1` | Enable browser-only frontend mock data; ignored inside Tauri |
| `SATCHEL_PERF_LOG=1` | Print timing for `commands::messages::*` operations |
| `SATCHEL_IMAP_TIMEOUT_SECS=20` | Override per-IMAP-operation timeout (default 20s) |
| `SATCHEL_IMAP_SYNC_ALL_FOLDERS=0` | Restrict sync to inbox/drafts/sent only |
| `SATCHEL_THREADS_PAGE_SIZE=500` | Threads fetched per list call; values above 1000 are clamped |
