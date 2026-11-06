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
- **URLs gehen nicht mit.** Jeder Link und jede Adresse, die mit
  `http(s)://` (oder einem anderen `schema://`), `//`, `www.` oder `mailto:`
  beginnt, wird zurückgehalten, so wie Satchel sie erkennt: in Links, Bildern,
  Styles, anderen Attributen und im Text, in HTML- wie in reinen Text-Mails. Hostnamen ohne diese
  Anfänge und E-Mail-Adressen im Text gehen als Text mit; sie sind Inhalt der
  Mail, und Satchel rät keine nackten Domains aus Fließtext. Im Einzelnen
  ersetzt Satchel durch Platzhalter der Form `#satchel-<Nonce>-a3-end`:
  - den ganzen Wert von `src`, `srcset`, `href`, `background`, `poster`,
    `cite`, jedes `data-*`, `id`, `name` und `summary`, was immer er enthält
    (auch `cid:`, `data:`, relative Pfade und JSON mit `https:\/\/`);
    seiteninterne Anker (`#…`) in `href` bleiben stehen;
  - den Inhalt jedes `url(…)` in Style-Attributen und `<style>`-Blöcken;
  - jede Adresse an jeder anderen Stelle: in weiteren Attributen (`alt`,
    `title`), in CSS-Strings wie `image-set('…')` und im Text selbst,
    Linktext eingeschlossen, auch JSON-escaped (`https:\/\/`, `\/\/host`).
    Als Schema gilt nur die Folge von Buchstaben direkt vor `://`; jedes
    andere Zeichen davor (`_`, Ziffer, `@`, `*`) ist eine Grenze. `www.` und
    `mailto:` setzen ein Wort nur nach einem Buchstaben oder Punkt fort.
    In HTML endet eine Adresse an Leerraum, `>`, `&nbsp;`, `&quot;`, einem
    `"` innerhalb eines Tags (im Text gehört `"` zum Link) oder einem `<`,
    das kein Tag ist, durch das der Link läuft: `<wbr>`, Elemente ohne
    sichtbaren Inhalt (Leerraum, `&nbsp;`, Nullbreite) und Inline-Elemente
    (`span`, `font`, `b`, `i`, `u`, `em`, `strong`, `a`, `sup`, `sub`), wenn
    der Link direkt dahinter weitergeht. In reinem Text endet sie nur an
    Leerraum, `<` und `>`; über einen Zeilenumbruch geht sie weiter, wenn
    direkt davor `://` oder `www.` steht oder die Zeile auf `=`, `&`, `?`
    oder `/` endet. Klammern, Anführungszeichen und IPv6-Hosts
    (`[2001:db8::1]`) im Link bleiben Teil der Adresse. Satzzeichen am Ende
    bleiben im Text, außer einer schließenden Klammer, die eine im Link
    geöffnete schließt.

  **Text-Weg:** Der Text-Body (reine Text-Mails und der Ausweichweg für zu
  großes HTML) läuft durch denselben Scanner wie der Text einer HTML-Mail.
  Auch der aufbewahrte gesendete Text im Anfrage-Protokoll enthält deshalb
  nur Platzhalter.

  Nach der Übersetzung setzt Satchel alles auf dem Gerät byte-genau wieder
  ein, in einem Durchgang. Das gilt für Links wie für Bilder: Ein Klick-Link
  trägt dasselbe Token wie ein Pixel, und eine Übersetzung braucht keine
  einzige URL. Die Nonce wird je Anfrage neu gezogen. Ein Platzhalter, den die
  Mail selbst enthält, den die Engine erfindet, umnummeriert oder in
  Großbuchstaben setzt, passt auf keine gespeicherte Adresse und bleibt
  wirkungslos. Entfernt die Engine einen Platzhalter, bleibt die Stelle ohne
  Adresse.
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
- **Kostenlose Tarife:** Keine Mail geht an eine Engine, deren Bedingungen
  personenbezogene Daten ausschließen (`accepts_personal_data()`), für kein
  Konto. Jede Mail enthält personenbezogene Daten, mindestens die des
  Absenders. Bei DeepL betrifft das die kostenlose API (8.3.11, dazu das
  vorbehaltene dauerhafte Speichern in 3.3.2). Ein solcher Schlüssel lässt
  sich speichern und testen, damit der Nutzer Tarif und Kontingent sieht.
  Übersetzt wird erst, wenn ein Pro-Schlüssel ihn ersetzt. Damit fällt auch
  der Konflikt mit Googles Limited-Use-Regeln für Gmail-Daten weg, ohne
  Sonderfall. Das ist eine Entscheidung des Maintainers und darf gelockert
  werden, wenn sich die Bedingungen ändern.

## Konsequenzen

Der Platzhalter-Weg hängt davon ab, dass die Engine Attributwerte nicht
anfasst und Platzhalter im Text als nicht übersetzbares Wort stehen lässt.
Für Attribute ist das die Grundlage von `tag_handling=html`. Ob es auch im
Text hält, prüft der erste Test mit einem echten Schlüssel. Fehlt danach
etwas, fehlt eine URL, kein Text.

Ein Link, der in einer reinen Text-Mail anders als oben beschrieben über
mehrere Zeilen umbrochen ist, kann teilweise mitgehen; die Texte in den
Einstellungen und in der Zustimmung sagen das.

Bewusst hingenommen: Ein Absender kann eine Adresse so schreiben, dass der
Scanner sie nicht erkennt, etwa mit CSS-Escapes (`\68ttps`) oder
Prozent-Kodierung des Schemas. Damit gibt er nur sein eigenes Token an
DeepL weiter, das er selbst vergeben hat. Satchel verfolgt solche Umgehungen
nicht weiter.

Die Platzhalter machen das gesendete HTML kleiner. Mehr Newsletter passen so
in eine Anfrage.

Eine weitere Engine, die HTML annimmt, übernimmt diese Regeln, ohne sie neu
zu bauen. Sie liegen im gemeinsamen Dienst (`translation/service.rs`), nicht
in der Engine.

Wer HTML an ein Sprachmodell senden will, schreibt ein neues ADR.
