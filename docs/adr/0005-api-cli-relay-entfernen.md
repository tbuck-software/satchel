# 0005 Lokale HTTP-API, CLI und Relay werden entfernt

Status: angenommen 2026-08-27

## Kontext

- `local_api.rs` (1.100 Zeilen handgerollter HTTP-Server + MCP) und
  `satchel-cli` (830 Zeilen) haben keinen Nutzer: KI-Werkzeuge lesen
  direkt die SQLite.
- `relay/` (axum, Postgres, APNs) wurde nie betrieben; Android-Code
  (`RelayWork.kt`, `commands/mobile_relay.rs`, `MobileRelaySettings.vue`)
  hängt daran.

## Entscheidung

- Alle drei werden gelöscht, samt Tabellen-Nutzung (`api_tokens`,
  `api_audit_log`) — Migrationen bleiben additiv, Tabellen werden per
  Nachzügler-Migration gedroppt.
- `backup`/`restore`/`credentials-status` aus der CLI werden
  **App-Einstellungen** (VACUUM INTO + `master.key`).
- Lesezugriff für KI wird ein **dokumentierter Vertrag**: ein paar
  read-only SQL-Views (`v_threads`, `v_messages`, …), deren Spalten
  stabil bleiben.
- Die spätere Hub-API (0008) entsteht **neu auf axum**, nicht aus
  `local_api.rs`.

## Konsequenzen

`KNOWN_UNWIRED` in `backend.ipc-coverage.test.ts` schrumpft. `relay.yml`
und der Dependabot-Eintrag für `/relay` fallen weg.
