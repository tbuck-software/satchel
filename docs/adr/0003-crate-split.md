# 0003 Crate-Split: `satchel-core` ohne Tauri

Status: angenommen 2026-08-27

## Kontext

`src-tauri` ist ein Crate mit 65k Zeilen. `commands/` hält 47 % davon
und 548 direkte `sqlx::query`-Aufrufe in Tauri-Handlern; `db/queries.rs`
hat 31 Zeilen. `sync/manager.rs` hat 6.000 Zeilen mit Funktionen von
600 Zeilen. Jede Rust-Änderung baut das ganze Crate neu. Der Hub (0008)
braucht dieselbe Logik ohne Tauri.

## Entscheidung

- `satchel-core`: Domäne, DB, Sync, Provider — **ohne** Tauri-Abhängigkeit.
- `satchel-app` (heute `src-tauri`): Tauri-Commands als dünne Wrapper,
  Plattform-Plugins, Fenster.
- Reihenfolge: **erst mechanisch** verschieben (ein Commit, keine
  Logikänderung, Bauzeit messen), **dann Schnitt** Modul für Modul:
  Repository-Schicht pro Aggregat, `thiserror`-Fehlertypen, Zerlegung
  von `sync/manager.rs` zuerst.
- Messlatte: Ein Einzeiler in `commands/threads.rs` darf inkrementell
  nicht mehr als ~15 s kosten.

## Konsequenzen

Der Diff darf nie beides zugleich sein (Verschieben + Umbauen), sonst ist
er nicht reviewbar. Duplikate über Provider (`html_to_text`,
`parse_address_list`, Cursor-Parsing, Ordner-Mapping) werden beim
Schnitt zusammengeführt; CalDAV/CardDAV zieht von `commands/groupware.rs`
in `providers/`.
