# 0021 Icons aus Lucide statt selbst gezeichnet

Status: angenommen 2026-11-23

## Kontext

Satchel zeichnete jedes Icon selbst: rund 450 SVG-Pfade, verteilt über gut
hundert Komponenten. `AppIcon` (UI-12) hatte Raster, Strichstärke und Größen
vereinheitlicht, aber nicht die Zeichnungen. Dieselbe Bedeutung sah an zwei
Stellen verschieden aus, und manche Zeichnung sagte etwas anderes als ihre
Beschriftung: „Blockieren“ war ein durchgestrichener Lautsprecher. Jedes neue
Icon war eine neue Zeichnung, die niemand gegen ein Vokabular prüfte.

Der Styleguide schloss Pakete aus, weil nichts von außerhalb des Bundles
geladen werden soll (Local-first, CSP, offline).

## Entscheidung

- Icons kommen aus **Lucide** (`lucide`, ISC), exakt gepinnt. Das Paket liefert
  nur Pfaddaten (`IconNode`), keine Komponenten und keine Schrift; Vite bündelt
  die importierten Icons, der Rest fällt per Tree-Shaking weg. Zur Laufzeit wird
  nichts nachgeladen.
- `AppIcon` bleibt der einzige Rahmen: `<AppIcon :icon="Reply" />` zeichnet das
  Lucide-Icon auf dessen 24er-Raster, mit unserer Strichstärke 1.5 und den drei
  Größen 14/16/20. Lucides eigene Strichstärke (2) gilt nicht.
- Eine Bedeutung, ein Icon. Das Vokabular steht in `STYLEGUIDE.md` unter
  „Icons“. Wer eine neue Bedeutung braucht, trägt sie dort ein.
- Der Slot von `AppIcon` bleibt für Zeichen, für die Lucide kein Wort hat.
  `src/architecture/designSystem.test.ts` zählt solche handgezeichneten Icons
  in einer Liste, die nur schrumpfen darf. Illustrationen, Fortschrittsringe
  und Markenzeichen sind keine Icons und bleiben eigene Zeichnungen.
- Beim Build schreibt `scripts/vite-lucide-inline.mjs` jedes importierte Icon
  als Konstante in das importierende Modul. Als eigene Module teilten
  gemeinsam genutzte Icons den Start in winzige Chunks; das Hauptfenster lud
  dadurch 4,5 KiB gzip mehr JavaScript, mehr als die Icons selbst wiegen. Mit
  dem Plugin bleibt der Start bei 219,3 KiB wie zuvor. Dev-Server und Vitest
  importieren `lucide` direkt.
- Lucide-Updates kommen über Dependabot wie jede andere Abhängigkeit. Ein
  Update kann Zeichnungen ändern; das ist der Preis eines gepflegten Sets.

## Folgen

- Neue Icons kosten eine Zeile Import statt einer Zeichnung, und sie passen zu
  den vorhandenen.
- Lucide zeichnet mit 2er-Strich; bei 16 px und unserem 1.5er-Strich wirken
  Icons mit vielen Details etwas dichter als in Lucides eigener Vorschau.
- Ersetzt den Satz „no icon font and no package“ in `STYLEGUIDE.md`. Die
  Regel dahinter gilt weiter: Keine Ressource wird zur Laufzeit von außen
  geladen.
