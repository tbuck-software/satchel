# 0017 Der Katalog des Hauptfensters wird nach Ladezeitpunkt geteilt

Status: angenommen 2026-11-06, ergänzt 0004

## Kontext

0004 hat einen Katalog pro Sprache festgelegt. Das Hauptfenster lud ihn
vollständig vor dem ersten Frame (~3.300 Schlüssel, 58 KiB gzip auf Deutsch),
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
  wer nirgends steht, könnte zur Laufzeit gebaut werden. `pnpm i18n:build`
  erzeugt die Datei, `pnpm i18n:check` prüft sie.
- Jedes dynamisch importierte App-Modul ist eine Grenze. Ihr Chunk hält die
  Schlüssel ihres statischen Abschlusses abzüglich dessen, was beim Import
  sicher schon geladen ist (Start plus die Chunks, in denen **jeder**
  Importeur liegt). Die Zuordnung leitet `scripts/i18n-split.mjs` aus der
  Nutzung ab, nicht aus den Part-Dateien.
- „Nennen" ist absichtlich weit: ein Schlüssel in einer Tabelle
  (`labelKey: 'folders.inbox'`) oder der feste Kopf eines gebauten Schlüssels
  (`` `thread.state.${s}` ``, `'outbox.status.' + s`) zählt in dem Modul, das
  das Literal hält. Ein solcher String erreicht `t()` erst, wenn dieses Modul
  geladen ist — und mit ihm sein Chunk. `t()`-Aufrufe ohne festen Kopf
  (`` t(`${x}.label`) ``) lehnt `i18n:check` ab; ihr Zuhause ist
  `UNANCHORED_KEYS`. Dateien mit schlüsselförmigen Literalen, die keine
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
- Vitest kennt das Plugin nicht und mischt in `src/test/setup.ts` den vollen
  Katalog beider Sprachen. `en.json`/`de.json` bleiben als volle Kataloge
  bestehen — für Tests und als Quelle der Chunks —, werden aber nicht mehr
  ausgeliefert.

## Konsequenzen

- Gemessen 2026-11-06: Start 247,2 → 208,2 KiB gzip; Start-Katalog
  964 von 3.340 Schlüsseln (Deutsch 20,1 statt 58 KiB gzip). Das
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
- Was die Analyse nicht sehen kann, sind Schlüssel aus Daten, die weder ein
  Modul noch Rust als Literal hält (z. B. ein von der App persistierter
  Schlüssel einer lazy Ansicht, der beim nächsten Start gerendert wird). Wer
  so etwas baut, trägt den Präfix in `STARTUP.prefixes` ein.
- `dev:mock` bekommt den Start-Katalog vom Plugin, ergänzt um die Schlüssel
  der Mock-Importe; der eingecheckte Katalog bleibt der des Produktionsgraphen.
