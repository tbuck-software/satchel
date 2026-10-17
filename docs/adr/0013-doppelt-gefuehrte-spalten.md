# 0013 Doppelt geführte Spalten im Mail-Modell

Status: angenommen 2026-09-14

## Kontext

Drei Werte stehen an zwei Orten, ohne dass festgehalten war, welcher gilt:

- `threads.subject`, `threads.snippet`, `threads.last_message_date` und
  daneben die neueste Nachricht, die die Thread-Liste per Join liest.
- `category` auf `messages` und auf `threads` (`0008_categories.sql`).
- `threads.board_column` neben `threads.workflow_state`.

Migrationen sind additiv; Spalten zu entfernen ist keine Option ohne
Tabellen-Rebuild, und ein Rebuild ist genau das, was Trigger verliert.

## Entscheidung

- **Betreff, Vorschau, Datum:** Angezeigt wird die neueste Nachricht
  (`is_draft ASC, date DESC, id DESC`). Die Thread-Spalten sind die
  Projektion desselben Werts, geschrieben von `refresh_thread_projection_tx`.
  Sie bleiben, weil `last_message_date` die Sortierung und das Keyset-Paging
  der Liste trägt und weil sie greifen, wenn ein Thread gerade keine lesbare
  Nachricht hat. Nicht jeder Schreibpfad frischt die Projektion sofort auf
  (siehe die Startreparatur für `unread_count`); der Join zeigt deshalb auch
  dann das Richtige. Seit PERF-15 ist er ein einzelner Index-Lookup pro Zeile.
- **`category`:** gilt auf `threads`. `messages.category` wird nicht gelesen
  und nicht geschrieben (nur Demo-Daten füllen sie) und bleibt als tote
  Spalte stehen. `src/types/mail.ts` kennt deshalb nur `Thread.category`.
- **`board_column`:** nur noch Spiegel. Gelesen wird ausschließlich
  `workflow_state`; Schreibpfade dürfen `board_column` mitsetzen, müssen aber
  nicht. `blocked_projection_pending` prüft seit ARCH-19 nur den Zustand.

## Konsequenzen

Neuer Code liest `threads.category` und `workflow_state`, nie
`messages.category` oder `board_column`. Wer eine der Thread-Spalten für die
Anzeige lesen will, liest stattdessen die neueste Nachricht mit.
