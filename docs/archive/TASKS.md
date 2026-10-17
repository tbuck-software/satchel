> **Archiviert.** Historischer Stand, nicht mehr gepflegt und in Teilen überholt.
> Aktueller Backlog: `.private/todo/`.

# Satchel Task Tracker

Stand: 2026-03-08
Referenzen: `docs/PLAN.md`, `docs/IMPLEMENTATION.md`

## Offen

- [ ] OAuth-End-to-End fuer Gmail/Outlook gegen reale Testkonten verifizieren

## Zuletzt erledigt

- [x] Compose-View an echtes Backend-Senden angebunden
- [x] Replies senden nun mit echten Empfaengern und bleiben lokal im bestehenden Thread
- [x] SMTP-Reply-Header (`In-Reply-To` / `References`) fuer IMAP-Sends verdrahtet
- [x] Gmail-Provider mit OAuth-basiertem Sync-/Send-Pfad statt Stub
- [x] Outlook-Provider mit OAuth-basiertem Sync-/Send-Pfad statt Stub
- [x] Mail-Sync verarbeitet jetzt IMAP, Gmail und Outlook ueber dieselbe Provider-Schicht
- [x] Provider-Aktionen fuer Gmail/Outlook implementiert: Flags, Move, Delete
- [x] OAuth-Refresh-Tokens werden bei echter Nutzung in `accounts.config` persistiert
- [x] OAuth-Flow oeffnet Browser direkt, nimmt lokalen Callback an und hat Copy-Link-/Manual-Fallback
- [x] Gmail-/Outlook-Sync paginiert jetzt ueber mehrere API-Seiten und fuehrt aeltere Historie als Backfill fort
- [x] Toolbar-/Bulk-Aktionen fuer Archivieren, Papierkorb und Read-Sync an echte Remote-Mailbox-Mutationen angebunden
- [x] Plugin-Build erzeugt jetzt pro Workspace-Plugin echte `component.wasm`-Artefakte
- [x] Wasmtime-Komponentenruntime fuer Plugins statt reinem In-Process-Mock integriert
- [x] HTML-Sanitization auf Backend-Seite mit `ammonia` gehaertet
- [x] Frontend-Renderer vor `v-html` zusaetzlich abgesichert

## Bereits umgesetzt

- [x] Reale SQLite-Queries fuer Accounts, Threads, Messages und Workflow-Transitions
- [x] IMAP-Sync inkl. SMTP-Send fuer IMAP-Accounts
- [x] Workflow-UI, Board, Keyboard-Navigation und Search-Overlay
- [x] PGP-Key-Management und entschluesselter Nachrichten-Renderpfad
