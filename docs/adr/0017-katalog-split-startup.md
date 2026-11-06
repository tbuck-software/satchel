# 0017 Der Katalog des Hauptfensters wird nach Ladezeitpunkt geteilt

Status: angenommen 2026-11-06, ergänzt 0004

## Kontext

0004 hat einen Katalog pro Sprache festgelegt. Das Hauptfenster lud ihn
vollständig vor dem ersten Frame (~3.300 Schlüssel, 60 KiB gzip auf Deutsch),
obwohl der Code von Einstellungen, Regeln, Kalender, Aufräumen, Spamfilter
oder Server-Regeln längst erst bei Bedarf lädt. Die letzte Feature-Welle ließ
den Start von 239,7 auf 247,2 KiB gzip wachsen, fast nur durch Texte. Viewer
und Tray hatten das Problem nie: ihre Teilkataloge werden schon aus dem
Modulgraphen ihres Einstiegs abgeleitet.

## Entscheidung

- Der Start lädt `src/i18n/startup.<locale>.json`: die Schlüssel, die Module
  im **statischen** Importgraphen von `src/main.ts` nennen, dazu `backend.*`
  und jedes Schlüssel-Literal aus den Rust-Quellen (Rust schickt Schlüssel,
  Toasts rendern sie beim Start) und alle Schlüssel, die kein Modul nennt —
  wer nirgends steht, könnte zur Laufzeit gebaut werden. Dazu kommen die
  Schlüssel der lazy Komponenten, die bei jedem Start montiert werden
  (`FIRST_PAINT`: Kontenzeilen und Agenda der Seitenleiste, Verbindungsbanner,
  auf dem Telefon Sync-Zeile und Schreiben-Knopf); sonst kostete der erste
  Frame eine Chunk-Runde. `pnpm i18n:build` erzeugt die Datei,
  `pnpm i18n:check` prüft sie.
- Jedes dynamisch importierte App-Modul ist eine Grenze. Ihr Chunk hält die
  Schlüssel ihres statischen Abschlusses, die nicht im Start-Katalog stehen.
  Abzuziehen, was ein Eltern-Chunk schon geladen hat, sparte über alle
  Chunks 2,8 KiB und war der fehleranfälligste Teil; es entfällt. Die
  Zuordnung leitet `scripts/i18n-split.mjs` aus der Nutzung ab, nicht aus den
  Part-Dateien.
- „Nennen" ist absichtlich weit: ein Schlüssel in einer Tabelle
  (`labelKey: 'folders.inbox'`) oder der feste Kopf eines gebauten Schlüssels
  (`` `thread.state.${s}` ``) zählt in dem Modul, das das Literal hält. Ein
  solcher String erreicht `t()` erst, wenn dieses Modul geladen ist — und mit
  ihm sein Chunk. Ein Schlüssel, der am Aufruf aus Teilen ohne festen Kopf
  entsteht (`` t(`${x}.label`) ``, `t(p + '.label')`), lässt `i18n:check`
  scheitern; sein Zuhause wäre `UNANCHORED_KEYS`. Dateien mit schlüsselförmigen Literalen, die keine
  Schlüssel sind, stehen begründet in `NOT_KEYS`.
- Ein Vite-Plugin (`scripts/vite-i18n-chunks.mjs`) schreibt jeden solchen
  Import im Code des Hauptfensters um zu `withMessages(id, () => import(…))`
  (`src/i18n/messageChunks.ts`). Der Chunk der
  aktiven Sprache wird **vor** dem Import gemischt, also bevor das Modul
  ausgewertet oder gerendert wird. Ein Sprachwechsel lädt alle bisher
  geladenen Chunks in der neuen Sprache, bevor er umschaltet; ein Chunk, der
  während eines Wechsels ankommt, lädt in beiden Sprachen. Die Chunks liegen
  nicht auf der Platte: das Plugin beantwortet `src/i18n/chunks/<id>.<locale>.json`
  aus dem vollen Katalog, der Build kompiliert sie vor wie jeden Katalog.
  Welche Datei zu welcher ID gehört, steht in einer Tabelle, die beim ersten
  lazy Import lädt; sonst trüge das Einstiegs-Chunk zwei gehashte
  Dateinamen pro Importstelle.
- Der Build bricht ab, wenn der Start-Katalog veraltet ist, und wenn Rollup
  ein Modul mit Schlüsseln außerhalb des Start-Katalogs in den Startgraphen
  legt, das die Analyse für lazy hält.
- Die unabhängige Prüfung teilt die Analyse nicht: CI baut die App mit
  Mock-Transport für die Produktion (`pnpm run test:i18n`) und durchläuft
  Routen, alle Einstellungsbereiche, Leser, Tastenkürzel-Hilfe und Palette auf
  Englisch und Deutsch, in Chromium und WebKit. Sie sucht Katalog-Schlüssel im
  Text und die Warnung `[i18n] missing key`, die die App für jeden fehlenden
  Schlüssel schreibt. Ein Kaltstart darf außer dem Start-Katalog nur die
  Chunks des Leerlauf-Vorladens (`IDLE_PREFETCH`) und die vom Mock-Zustand
  ausgelösten laden.
- Vitest kennt das Plugin nicht und mischt in `src/test/setup.ts` den vollen
  Katalog beider Sprachen. `en.json`/`de.json` bleiben als volle Kataloge
  bestehen — für Tests und als Quelle der Chunks —, werden aber nicht mehr
  ausgeliefert.

## Konsequenzen

- Gemessen 2026-11-06: erster Frame 247,2 → 209,6 KiB gzip; Start-Katalog
  1.027 von 3.340 Schlüsseln (Deutsch 21,5 statt 60,0 KiB gzip), kein Chunk
  vor dem ersten Frame. Sobald die Inbox benutzbar ist, wärmt `bootstrap.ts`
  Leser und Palette vor; mit deren Chunks sind es 31,8 KiB Katalog. Das
  Einstiegs-Chunk wächst um 0,7 KiB (Lader und umgeschriebene Importe in
  Startmodulen), Viewer und Tray um 0,1 bzw. 0,2 KiB (der Lade-Hook in
  `i18n/runtime`).
- Chunks werden pro Grenze geschnitten, nicht dedupliziert: Thread-Ansicht,
  Suche, Entwürfe und Thread-Detail tragen je rund 400 überlappende
  Schlüssel (≈ 7 KiB gzip). Wer zwei davon öffnet, lädt die Überlappung
  zweimal — lokal, einmal pro Sitzung.
- Der erste lazy Import einer Sitzung lädt zusätzlich die Chunk-Tabelle;
  jeder Import wartet auf seinen Chunk, bevor sein Modul lädt.
- Ein Feature, das neue Texte in einer lazy Ansicht bringt, kostet den Start
  nichts mehr; ein Text in Startcode landet sichtbar im Diff von
  `startup.*.json`.
- `startup.*.json` ist eine weitere generierte Datei neben `en.json`,
  `de.json` und den Viewer-/Tray-Teilkatalogen. Zwei für sich grüne PRs
  können sie auf `main` gemeinsam veralten lassen; nach einem Rebase gehört
  `pnpm i18n:build` dazu, und Vitest wie Build nennen die fehlenden
  Schlüssel.
- Was die Analyse nicht sehen kann, sind Schlüssel aus Daten, die weder ein
  Modul noch Rust als Literal hält (z. B. ein von der App persistierter
  Schlüssel einer lazy Ansicht, der beim nächsten Start gerendert wird). Wer
  so etwas baut, trägt den Präfix in `STARTUP.prefixes` ein.
- `dev:mock` bekommt den Start-Katalog vom Plugin, ergänzt um die Schlüssel
  der Mock-Importe; der eingecheckte Katalog bleibt der des Produktionsgraphen.
