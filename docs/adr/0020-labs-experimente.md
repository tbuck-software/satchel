# 0020 Labs: Experimente hinter einem Schalter pro Gerät

Status: vorgeschlagen 2026-11-07

## Kontext

Delta-Chat-Kompatibilität soll ausprobierbar sein, bevor sie fertig ist, ohne
dass sie für alle mitläuft. Satchel hatte keine Feature-Flags. Der Start ist
eng budgetiert (`scripts/check-entry-bundles.mjs`, 0017): ein ausgeschaltetes
Experiment darf dort weder Code noch Texte kosten.

## Entscheidung

- Ein Flag steht in einer festen Liste, zweimal: `src/labs/flags.ts` (Id,
  Vorgabe aus, „wirkt nach Neustart“) und `db::labs::LABS_FLAGS`. Das Backend
  lehnt unbekannte Ids ab. Titel und Beschreibung stehen in
  `src/labs/flagCopy.ts`, das nur der Abschnitt Einstellungen → Labs lädt;
  `satisfies` hält beide Hälften gleich.
- Der Zustand liegt pro Gerät in `labs_flags` (Migration 0165); keine Zeile
  heißt aus. Gelesen wird über **einen** Store (`useLabsStore`, fragt
  `backendCommandsAvailable()`).
- Der Start hält von Labs nur einen leeren Platz für Seitenleisten-Zeilen
  (`src/labs/state.ts`) und einen dynamischen Import von `src/labs/runtime.ts`.
  Der lädt die Flags und startet für jedes eingeschaltete Flag dessen
  Aktivierung (`src/labs/activation.ts`). Eine Aktivierung bringt alles mit,
  was das Experiment der App hinzufügt (Route per `router.addRoute`,
  Seitenleisten-Zeile, `g`-Tastenfolge über `registerViewShortcut`) und gibt
  eine Funktion zurück, die es wieder entfernt.
- Ein Backend-Pfad, der nur für ein Experiment läuft, fragt das Flag selbst
  (`db::labs::is_enabled_in`), wie die Autocrypt-Verarbeitung beim Ingest.
  Daten, die ohnehin anfallen (der `Chat-Version`-Header), werden unabhängig
  vom Schalter gespeichert, damit Einschalten vorhandene Mail zeigt.

- Das eigene `Autocrypt:`-Header geht **nur mit Chat-Nachrichten** hinaus,
  nicht mit jeder Mail, und Peer-Schlüssel sammelt Satchel nur bei
  eingeschalteten Chats (Spam, Papierkorb, `multipart/report`, mehrere
  Absender, eigene Aliase und DMARC-`fail` zählen nicht). Level 1 empfiehlt das
  Header in jeder Mail („Header injection in outbound mail“). Folge: Schreibt
  jemand einem Delta-Chat-Kontakt länger als 35 Tage nur normale Mails, sinkt
  dessen Empfehlung für uns auf „discourage“ („Recommendation“, Schritt 4),
  bis die nächste Chat-Nachricht kommt. Bewusst so: ein Experiment soll den
  normalen Mailverkehr nicht verändern.
- Peer-Schlüssel kommen nur aus Mail, die Satchel **nach** dem Einschalten
  zum ersten Mal sieht: die Ingest-Pipeline liest das `Autocrypt:`-Header
  beim ersten Speichern einer Nachricht und behält den Header-Block nicht.
  Wer Chats einschaltet, hat für einen Kontakt also erst nach dessen nächster
  Nachricht einen Schlüssel.

## Folgen

- Ein neues Experiment: Eintrag in beiden Listen, Texte in einem eigenen
  Part, eine Aktivierung. Startkosten: keine, außer `backend.errors.*`.
- Eine Tastenfolge aus einer Aktivierung ist nicht umbelegbar und steht nicht
  im Kürzel-Editor; die Hinweiszeile der Folge nennt sie.
- Ein Experiment, das fertig wird, verlässt Labs: Flag aus beiden Listen
  nehmen, die Aktivierung in den normalen Code ziehen. Die Zeile in
  `labs_flags` bleibt liegen und wird nicht mehr gelesen.
