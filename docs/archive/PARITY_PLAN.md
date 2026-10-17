> **Archiviert.** Historischer Stand, nicht mehr gepflegt und in Teilen überholt.
> Aktueller Backlog: `.private/todo/`.

# Satchel Design- und Feature-Parity Plan (Referenz: Screenshot)

Stand: 2026-02-20

## Ziel
Satchel soll in Layout, Informationsdichte und Kerninteraktionen parity-nah zur Referenz werden, ohne die vorhandenen Workflow-/Kanban-Funktionen zu verlieren.

## Gap-Analyse

| Bereich | Referenz | Ist-Stand Satchel | Gap | Priorität |
|---|---|---|---|---|
| App-Shell | 3-Spalten-Layout mit dichter Top-Toolbar und klaren Segmenten | 2-Spalten-Hauptlayout + einfache Sidebar | Mittlere Layout-Parity fehlt | P0 |
| Linke Navigation | Account-Liste + hierarchische Ordner (Inbox, Drafts, Sent, Labels) mit Counts und Icons | Workflow-Filter + einfache Routenlinks | Keine echte Mailbox-Tree-Navigation | P0 |
| Thread-Liste | Gruppierung nach Priorität/Heute/Gestern/Woche + dichte Zeilen + Badges | Flache Liste ohne Zeitgruppierung | Gruppierung, Dichte und Metadaten fehlen | P0 |
| Thread-Toolbar | Dichte Action-Leiste (archive/delete/snooze/more) | Nur wenige Buttons im Detail | Aktionstiefe und Icon-Toolbar fehlen | P0 |
| Nachrichtenansicht | Konversationsblöcke mit klarer Trennung, Header-Metadaten, Inline-Aktionen | Basis-Bubbles vorhanden | Feinschliff für echte Reader-Parity fehlt | P1 |
| Such-/Filter-UX | Prominente globale Suche + schnelle Filter in der Liste | Overlay-Suche vorhanden, aber nicht in der Shell verankert | Discoverability und schnelle Filter fehlen | P1 |
| Bulk-/Triaging-Flow | Checkboxen, Multi-Select, Schnellaktionen | Kein Multi-Select | High-throughput-Flow fehlt | P1 |
| visuelle Dichte | Kompakter, produktionsnaher Mail-Client-Look | Solider MVP-Look, aber luftiger | Typografie/Spacing/Iconografie noch nicht parity-nah | P1 |
| Collaboration-Elemente | Zuweisen/Teilen im Header sichtbar | Nicht vorhanden | Optional, aber für volle Parity relevant | P2 |
| Assist-Bar | Prompt-/Assistant-Leiste unten | Nicht vorhanden | Optional/Plugin-Feature | P2 |

## Parity-Milestones

## M1 — Shell + Navigation Parity (P0)
- [x] Neue App-Shell: echte 3-Spalten-Struktur (Nav | Thread-Liste | Reader)
- [x] Kompakte globale Toolbar oben im Content-Bereich
- [x] Linke Mailbox-Tree-Navigation mit Icons, Countern, Collapse/Expand
- [x] Account-Switcher mit Farb- und Provider-Indikator
- [x] Persistente Panel-Breiten (resizable split panes)

Abnahme:
- Layout entspricht strukturell der Referenz auf Desktop.
- Nutzer kann zwischen Accounts/Folders ohne Route-Bruch wechseln.

## M2 — Thread-Liste Parity (P0)
- [x] Gruppierungslogik: `Priority`, `Today`, `Yesterday`, `This Week`, `Older`
- [x] Kompakte Thread-Row mit Avatar, Betreff, Vorschau, Badges, Zeit, Zähler
- [x] Inline-Row-Actions auf Hover/Focus
- [x] Schnellsortierung (`by importance`, `by date`) + Filterchips
- [x] Virtuelle Liste für große Mengen

Abnahme:
- Liste ist gruppiert und auf hohe Dichte optimiert.
- Kernmetadaten sind ohne Öffnen des Threads erfassbar.

## M3 — Reader + Actions Parity (P0)
- [x] Dichte Action-Toolbar im Reader-Header (Archive, Delete, Snooze, Reply, More)
- [x] Konversationsblöcke mit sauberem Header (von/an/zeit)
- [x] Kompakte Inline-Reply-Bar am unteren Rand
- [x] Sticky Reader-Header mit Thread-Kontext
- [x] Attachments als Chips + Quick-Preview-Trigger

Abnahme:
- Thread komplett per Toolbar und Keyboard bearbeitbar.
- Reader visuell und funktional parity-nah.

## M4 — Triage-Speed Features (P1)
- [x] Multi-Select in Thread-Liste (Shift/Cmd/Ctrl)
- [x] Bulk-Actions (archive, done, waiting, move, tag)
- [x] Quick-Triage-Keymap (single-key, optional vim-normal mode)
- [x] Undo-Toast für destructive Actions

Abnahme:
- 20 Threads in Serie ohne Maus möglich.

## M5 — Visual Polish Parity (P1)
- [x] Neue Typografie-Hierarchie für Dense UI
- [x] Spacing/Radius/Border-Rules an Mail-Client-Pattern angleichen
- [x] Konsistente Icon-Sprache (Toolbar/Rows/Tree)
- [x] Hover/Focus/Selected/Unread States präzisieren

Abnahme:
- Visuelle Dichte und Scannability parity-nah.

## M6 — Optional Parity+ (P2)
- [x] Assign/Share-Actions im Header
- [x] Assistant-Bar unten (Plugin-gestützt)
- [x] Team-/Delegation-Metadaten pro Thread

Abnahme:
- Erweiterte Referenzfeatures optional nutzbar.

## Technische Umsetzung

## Frontend
- Neue Komponenten:
  - `src/components/layout/AppShell.vue`
  - `src/components/layout/MailboxTree.vue`
  - `src/components/layout/TopToolbar.vue`
  - `src/components/mail/ThreadGroupList.vue`
  - `src/components/mail/ThreadActionsBar.vue`
- Store-Erweiterungen:
  - `stores/threads.ts`: Gruppierung, Multi-Select, Bulk-Operations
  - `stores/ui.ts`: Pane-Width, density mode, toolbar state
  - `stores/messages.ts`: reader action state

## Backend
- Neue Commands (M2/M4):
  - `bulk_update_threads` (state/tag/archive)
  - `list_threads_grouped` (oder clientseitig gruppieren bei ausreichenden Daten)
- Optional (M6):
  - `assign_thread`, `share_thread`

## Datenmodell
- Bereits vorhanden und nutzbar: `folders`, `threads`, `messages`, `tags`.
- Ergänzungen für Collaboration (optional): `thread_assignees`, `thread_shares`.

## Delivery-Plan (empfohlen)

1. Sprint A (3-4 Tage): M1
2. Sprint B (3-4 Tage): M2
3. Sprint C (3-4 Tage): M3
4. Sprint D (2-3 Tage): M4
5. Sprint E (2 Tage): M5
6. Sprint F (optional, 2-3 Tage): M6

## Messbare Parity-Kriterien
- Time-to-triage (10 Threads) < 60 Sekunden mit Keyboard.
- Sichtbare Kerninfos pro Thread-Row ohne Öffnen: Betreff, Vorschau, Zeit, Badges.
- Keine Layout-Sprünge bei Fensterbreite 1280-1920.
- Reader-Aktionen in <= 1 Klick oder 1 Shortcut.

## Nächster konkreter Start
- Start mit M6 Assistant-Bar unten (Plugin-gestützt).
