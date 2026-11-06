# 0018 Bereinigtes HTML an Übersetzungs-Engines ohne Sprachmodell

Status: angenommen 2026-11-06

## Kontext

0016 legt für KI-Funktionen fest: Mails gehen als reiner Text an das Modell,
Antworten werden als Text gezeigt, nie als gerendertes HTML. Der Grund ist,
dass ein Sprachmodell Anweisungen befolgt, auch solche, die in einer Mail
stehen, und dass seine Antwort deshalb nicht vertrauenswürdig ist.

Die Übersetzung (0016, Funktion 7) läuft zuerst über DeepL. DeepL ist kein
Modell, dem man Anweisungen gibt. Es übersetzt Text und lässt mit
`tag_handling=html` das Markup stehen. Ein Newsletter mit Tabellen,
Spalten und Bildern bleibt so ein Newsletter. Als Text übersetzt verliert er
sein Layout, und der Leser sieht danach etwas anderes als vorher.

HTML zu senden heißt aber, mehr zu senden als Text. Mail-HTML enthält URLs:
Links, Bildquellen, `srcset`, Tabellenhintergründe. In Werbe- und
Benachrichtigungsmails tragen viele davon pro Empfänger erzeugte Tokens.
Ein Klick-Link wie `https://click.example/r?u=4711&l=12` oder ein
Tracking-Pixel `https://t.example/open.gif?u=4711` sagt dem Absender, wer
die Mail geöffnet hat. Wer diese URLs an Dritte weitergibt, gibt ihnen
Kennungen weiter, die der Absender dem Empfänger zugeordnet hat.

Bei DeepL kommt der Tarif hinzu. Die Bedingungen
(<https://www.deepl.com/en/pro-license>, 3.3.2) behalten sich vor, alles,
was über die kostenlose API gesendet wird, dauerhaft zu speichern, und
verlangen in 8.3.11, über sie keine personenbezogenen Daten zu
verarbeiten. Mit einem Pro-Schlüssel bewahrt DeepL Inhalte nur auf,
solange es technisch nötig ist (3.1.2), bei Fehlern bis zu 72 Stunden
(3.1.4). Laut Datenschutzerklärung (<https://www.deepl.com/en/privacy>,
Abschnitt 4) nutzt es sie nicht, um den Dienst zu verbessern. Abgerufen
am 2026-11-06.

## Entscheidung

- Eine Engine sagt über `TranslationEngine::accepts_html()`, ob sie Markup
  annimmt. DeepL tut das und bekommt das gespeicherte, bereits bereinigte
  HTML. Eine KI-Engine nach 0016 sagt `false` und bekommt den Text-Body.
  Ihre Antwort wird als Text gezeigt. Für Sprachmodelle ändert dieses ADR
  nichts.
- **URLs gehen nicht mit.** Vor dem Senden ersetzt Satchel jede URL in
  `src`, `srcset`, `href`, `background`, `poster` und `cite` durch einen
  Platzhalter (`#satchel-a3-end`), ebenso jedes `url(…)` in Styles
  (`#satchel-c1-end`). Seiteninterne Anker (`#…`) bleiben stehen. Nach der
  Übersetzung setzt Satchel die URLs auf dem Gerät wieder ein. Das gilt
  auch für Links, nicht nur für Bilder: Ein Klick-Link trägt dasselbe Token
  wie ein Pixel, und eine Übersetzung braucht keine einzige URL. Entfernt
  die Engine einen Platzhalter, bleibt die Stelle ohne URL. Erfindet sie
  einen, läuft er durch den Sanitizer wie jede andere Adresse.
- Was die Engine zurückgibt, läuft erneut durch den E-Mail-Sanitizer
  (`sanitize_stored_email_html`), wird gespeichert und beim Lesen noch
  einmal bereinigt. Es rendert im selben `iframe` wie das Original: gleiche
  CSP (`src/utils/emailCsp.ts`), gleiche Sandbox ohne `allow-scripts`,
  gleiche Bildregel (`useImagePolicy`). Eine Übersetzung kann damit nichts,
  was die Mail selbst nicht schon konnte. Externe Bilder bleiben gesperrt,
  bis der Nutzer sie für diese Mail oder diesen Absender erlaubt.
- Passt das HTML nicht in eine Anfrage (bei DeepL 128 KiB), geht der
  Text-Body, und die Übersetzung wird als Text gezeigt. Passt auch der
  nicht, verlässt nichts das Gerät, und das Anfrage-Protokoll bekommt
  keinen Eintrag.
- Betreff, Absender, Empfänger und Anhänge gehen nicht mit. Verschlüsselte
  Mails (S/MIME, PGP inline und PGP/MIME) gehen nie, auch nicht entschlüsselt.
- **Gmail und kostenlose Tarife:** Mails aus Google-Konten (über die Gmail-API
  gelesen) gehen nicht an eine Engine, deren Bedingungen erlauben, den Text
  zu behalten oder damit zu lernen (`may_retain_text()`). Bei DeepL ist das
  die kostenlose API. Googles Limited-Use-Regeln erlauben die Weitergabe von
  Gmail-Daten nicht an einen Dritten, der sie behalten darf. Mails anderer
  Konten bleiben mit einem kostenlosen Schlüssel übersetzbar, nach einer
  Zustimmung, die den Tarif nennt. Das ist eine Entscheidung des Maintainers
  und darf gelockert werden, wenn sich die Bedingungen ändern.

## Konsequenzen

Der Platzhalter-Weg hängt davon ab, dass die Engine Attributwerte nicht
anfasst. Für DeepL ist das die Grundlage von `tag_handling=html`. Ob es im
Betrieb hält, prüft der erste Test mit einem echten Schlüssel. Fehlt danach
etwas, fehlt eine URL, kein Text.

Die Platzhalter machen das gesendete HTML kleiner. Mehr Newsletter passen so
in eine Anfrage.

Eine weitere Engine, die HTML annimmt, übernimmt diese Regeln, ohne sie neu
zu bauen. Sie liegen im gemeinsamen Dienst (`translation/service.rs`), nicht
in der Engine.

Wer HTML an ein Sprachmodell senden will, schreibt ein neues ADR.
