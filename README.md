# Satchel

Satchel is a keyboard-first, local-first mail client. Mail lives in a local
SQLite database; the app talks to servers to sync, not to read.

## License

Satchel is licensed under the [MIT License](LICENSE). Third-party assets retain
their own licenses; see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

## Current Scope

**Frontend (Vue 3 + TypeScript)**

- Three-pane inbox with virtualised thread list, time grouping and smart buckets
- Full-text search over message bodies (SQLite FTS5) with operator syntax
- Keyboard-first triage (`j/k`, arrows, `r/a/f`, `e`, `#`, `u`, `:`, `/`, `?`)
  with undo on every destructive action
- Rich-text composer: aliases, signatures, templates, attachments, inline
  images, send-later, undo-send, S/MIME sign & encrypt
- Reader with isolated, CSP-enforced HTML rendering, remote-content blocking,
  external-link protection, printing and a raw-source viewer
- Calendar invitations, contacts, rules, smart folders, vacation responder

**Backend (Rust + Tauri 2)**

- IMAP (with IDLE push), POP3, Gmail API and Microsoft Graph providers
- Message parsing and IMAP cursor handling via the
  [postroom](https://github.com/tbuck-software/postroom) crates
- Offline-first sync: SQLite + FTS5, sync queue, outbox worker, snooze and
  follow-up workers, maintenance/vacuum
- OAuth2 with PKCE for Gmail and Outlook
- WASM plugin runtime (wasmtime) with capability grants, audit log and metrics
- RSS feed sources
- mbox import/export, full platform data backup and in-app database backup/restore

**Known limitations**

- Outlook/Graph sending was rewritten to submit MIME and is **not yet
  verified against a live account.**
- POP3 has not been verified against a live server.
- PGP signing and encryption work end to end (RFC 3156), but key management
  lives in the plugin manager rather than a dedicated settings page.
- Account credentials are encrypted at rest (AES-GCM) with a key from the OS
  keychain. Existing legacy key files remain readable, while creating a new
  plaintext fallback requires explicit opt-in on desktop. **Message content is not encrypted** — anyone with the database
  file can read the mail in it.

## Install and update

Download a macOS DMG, Windows installer, or Linux AppImage from
[Releases](https://github.com/tbuck-software/satchel/releases/latest).
macOS builds are signed and notarized. Windows installers currently have no
Authenticode signature and may show an unknown-publisher warning.

On macOS, the standalone installer also backs up your existing database and
credential key before updating. It needs Node 22 or later, with no checkout or
GitHub login:

```sh
curl -fLsS https://github.com/tbuck-software/satchel/releases/latest/download/install-satchel.mjs | node --input-type=module - --channel stable --open
```

Desktop releases are built from `main`: nightly every day and stable every
Monday. Satchel uses the stable channel; Satchel Daily has a separate profile.
Packaged apps check for signed updates and offer installation with a restart.
From a checkout, `pnpm run install:stable` and `pnpm run install:nightly` use
the same verified installer.

See [docs/RELEASE.md](docs/RELEASE.md) for installers, release channels, signing,
and distribution. [SECURITY.md](SECURITY.md) explains how to report vulnerabilities.
[PRIVACY.md](PRIVACY.md) describes account data, local storage and network access.

## Development

The repo is on pnpm; `pnpm-lock.yaml` is the lockfile CI installs from, and an
`npm install` alongside it produces a second, conflicting tree.

```bash
pnpm install
pnpm run dev          # frontend only, in a browser
pnpm run tauri:dev    # the actual desktop app
pnpm run build        # frontend production build
```

The Rust toolchain is pinned in `rust-toolchain.toml`, so `cargo` picks the
right version by itself inside this checkout.

Run `make check` before committing; `make help` lists the build and install
targets. `pnpm run lint` runs Oxlint correctness rules, including the Vue
plugin's script checks. Intentional exceptions carry reasons at the affected
lines. `pnpm run scripts:any:check` rejects growth or stale entries in
`scripts/implicit-any-allowlist.json`; the existing 250 diagnostics across 21
scripts remain until those scripts gain type annotations. Both checks run in
CI and `pnpm run check`. Parser fuzzing uses a separate nightly toolchain;
see [the fuzzing guide](docs/FUZZING.md).

## IMAP and POP3 accounts

- Add credentials in the account `config` JSON (`host`, `port`, `tls`,
  `username`, `password`), or set `SATCHEL_IMAP_PASSWORD`.
- POP3 uses the same shape under a `pop3` key and additionally understands
  `leave_on_server` (default `true`). POP3 has no folders and no server-side
  flags, so archive, move and read state stay local to this machine.
- SMTP send is supported for IMAP and POP3 accounts via `config.smtp` (or flat `smtp_*` keys). Defaults:
  - host defaults to IMAP host
  - port defaults to `587`
  - username defaults to IMAP username
  - password defaults to IMAP password (or `SATCHEL_SMTP_PASSWORD`)
  - from address defaults to account email
- Accounts are normally added through onboarding or `Settings` → `Accounts`;
  the raw `config` JSON below is what those screens write.
- In the running app, open the Command Palette (`:`) and run `Run Mail Sync`.
- Synced folders and messages are persisted into local SQLite.

### Where the password ends up

The password is part of the account `config`, and that whole column is
encrypted at rest (`enc:v2:` prefix, AES-GCM). The OS keychain holds one
master key per profile for that encryption, not one entry per mail account.
The credential is only as protected as the master key, and anyone who has both the database
and the key can read it back.

If the keychain is unreachable at startup, Satchel refuses to mint a new
plaintext master key next to the database (fail closed). A fallback key file
that already exists from an earlier run is migrated into the profile's keychain
entry and deleted after a verified read-back. If migration fails, it remains
available so existing installs keep working. Headless setups can opt in with
`SATCHEL_ALLOW_PLAINTEXT_KEY_FALLBACK=1`.

The example below is what you would type; it is not what sits on disk.

Example account `config`:

```json
{
  "imap": {
    "host": "imap.example.com",
    "port": 993,
    "tls": true,
    "username": "you@example.com",
    "password": "app-password"
  },
  "smtp": {
    "host": "smtp.example.com",
    "port": 587,
    "tls": true,
    "username": "you@example.com",
    "password": "app-password",
    "from_address": "you@example.com"
  }
}
```

- Demo seed data is opt-in with `SATCHEL_SEED_DEMO_DATA=1`.

## OAuth (Gmail / Outlook)

- Set environment variables:
  - easiest for local dev: create `.env` in repo root (see `.env.example`)
  - debug builds load `.env` at runtime, including the app-private file provisioned by the Android debug installer
  - release builds use the public client IDs and redirect URIs compiled from the build configuration; they ignore runtime `.env` files and development overrides
  - client secrets are never compiled into the binary; release builds require a public OAuth client registration
  - `SATCHEL_GMAIL_CLIENT_ID`, optional `SATCHEL_GMAIL_CLIENT_SECRET`, optional `SATCHEL_GMAIL_REDIRECT_URI`
  - `SATCHEL_OUTLOOK_CLIENT_ID`, optional `SATCHEL_OUTLOOK_CLIENT_SECRET`, optional `SATCHEL_OUTLOOK_REDIRECT_URI`
- Add the account through the setup wizard (`Settings` -> `Accounts` -> `Add account`)
  and pick Google or Microsoft. Satchel opens the browser and takes the callback
  on a loopback listener; there is no separate OAuth pane.
- If the browser cannot reach that listener (firewall, remote desktop, no default
  browser), the wizard waits two minutes and then offers `Finish the sign-in by
  hand`: copy `state` and `code` out of the browser's address bar and paste them
  there. The flow is spent on the first attempt, so a wrong pair means starting over.

## Contributing

`AGENTS.md` documents the conventions this repo enforces (UI language, design
tokens, IPC wiring, migration rules, the mail-renderer CSP). See
[CONTRIBUTING.md](CONTRIBUTING.md) for setup and checks. Run `make check`
before committing.

## Directory Highlights

- `src/`: frontend app
- `src-tauri/`: Rust backend + migrations
- `plugin-sdk/`: SDK crate for plugins
- `plugins/`: built-in plugin crates
- `docs/PLAN.md`, `docs/IMPLEMENTATION.md`: historical planning documents, kept for context
- `docs/archive/`: historical plans, kept for context only
