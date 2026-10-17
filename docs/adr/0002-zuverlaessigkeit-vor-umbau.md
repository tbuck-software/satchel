# 0002 Zuverlässigkeit vor Umbau: Mail-Korpus und Diagnose-Seite

Status: angenommen 2026-08-27

## Kontext

Die störenden Fehler sind keine Abstürze, sondern stille Abweichungen:
falsch erkannte Encodings, Benachrichtigungen, die ein anderer Client
zeigt und Satchel nicht. Beide Klassen fangen Fixture-Tests nicht, und
Fehler verschwinden in `Result<_, String>` plus `eprintln!`.

## Entscheidung

1. **Korpus echter Problem-Mails.** Jeder Parser-Bug wird als `.eml`
   eingecheckt und zum Regressionstest. Roh unter `.private/corpus/`
   (lokal), ein anonymisierter Kern unter `src-tauri/tests/corpus/`
   (CI). Für Encoding-Fälle reicht Header + Body-Bytes mit ersetztem
   Text.
2. **Diagnose-Seite in der App** statt Log-Datei: letzte Sync-Fehler,
   `notification_outbox` (Zustand und Ablehnungsgrund je Nachricht),
   `sync_queue`, `account_sync_leases`, Rate-Limit-Blocks (`app_settings`).
   Die Daten liegen bereits in der DB; jede Entscheidung „nicht anzeigen"
   schreibt ihren Grund in die Outbox.
3. **Typisierte Fehler** (`thiserror`) ersetzen die String-Tags
   `[rate-limited]`/`[permanent]` aus `sync/errors.rs` — im Zuge von
   0003, Modul für Modul.
4. Logging ausschließlich über `tracing`; `eprintln!`/`println!` sind
   Bugs.

## Konsequenzen

Phase 2 des Backlogs läuft **vor** dem Crate-Split, damit der Korpus
jeden späteren Umbau absichert.
