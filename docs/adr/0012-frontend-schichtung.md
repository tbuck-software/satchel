# 0012 Frontend-Schichtung für Mail-Zustand

Status: angenommen 2026-08-27

## Kontext

60 Komponenten/Views rufen `services/backend` direkt, 27 Stores.
`CLAUDE.md` verbietet nur `invoke`. Mit dem Hub (0008) kommt ein zweiter
Datenpfad, der irgendwo mit Backend-Events zusammenlaufen muss.
`stores/threads.ts` (1.500 Zeilen) hat vier Testdateien — die Tests
kennen die vier Module schon.

## Entscheidung

- **Mail-Zustand** (Threads, Messages, Mailboxes, Topics — alles, was
  der Hub spiegelt) fließt ausschließlich über Stores/Composables.
  Komponenten rufen dafür kein Backend. Ein Lint erzwingt das.
- **Settings-Panes** dürfen Formulare weiter direkt an Kommandos senden;
  ein Store gewinnt dort nichts.
- `threads.ts` wird in Collection, Selection, Filter/Buckets und
  Persistenz geteilt. `App.vue` (900 Zeilen Script, 15 Zeilen Template)
  wird `bootstrap.ts`.

## Konsequenzen

Hygiene vorher: tote Komponenten (`ComposeEditor`, `AssistantBar`,
`common/`, …), Hex-Werte, `localStorage`-Streu. Tailwind ist entgegen der ersten Karte kein Randfall (999 Klassen in 23 Dateien) — ob es bleibt oder geht, ist eine eigene Entscheidung.
