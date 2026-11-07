# 0019 Regeln verschieben und leiten weiter, lokal und auf dem Server

Status: vorgeschlagen 2026-11-07

## Kontext

Regeln konnten bisher Status, Priorität, Kategorie und Tags setzen und als
gelesen markieren. Seit ManageSieve (`server_rules`) läuft eine Regel
zusätzlich als Sieve-Skript auf dem IMAP-Server, damit sie auch bei
geschlossenem Satchel greift. Satchel führt jede Regel trotzdem lokal aus;
das war nur sicher, weil die einzige Server-Aktion (`\Seen`) idempotent ist.

Verschieben, Anheften (`\Flagged`) und Weiterleiten sollen dazukommen, lokal
und auf dem Server. Verschieben und Flags darf man zweimal ausführen.
Weiterleiten nicht: Laufen Server und Satchel dieselbe Regel, ginge jede Mail
zweimal raus. Die Zeit allein trennt die beiden nicht sauber. Der Server
filtert bei der Zustellung, Satchel beim nächsten Sync. Ein Push, der
abbricht, lässt auf dem Server das alte oder schon das neue Skript laufen,
und jemand kann Satchels Skript im Webmail abschalten.

## Entscheidung

- Eine Regel hält eine geordnete Liste von Aktionen (`move_to_folder:<id>`,
  `star`, `forward:<adresse>`, `forward_no_copy:<adresse>` neben den
  bisherigen), höchstens eine Verschiebung und eine Weiterleitung je Regel.
  Eine Regel, die verschiebt, gehört zu einem Konto (`rules.account_id`).
  Ist das Konto weg, läuft die Regel nicht mehr und zeigt sich als defekt.
- **Verschieben:** Pro Lauf wird eine Nachricht höchstens einmal verschoben,
  die erste passende Regel (nach Priorität) bestimmt den Ordner. Vorher sind
  die Flags aller passenden Regeln gesetzt. Im Sieve-Skript stehen dafür
  zuerst alle Flags und Weiterleitungen, dann eine `if … elsif …`-Kette der
  `fileinto`. Nie in die Inbox, nach „Gesendet“ oder in die Entwürfe: Regeln
  laufen auf Inbox-Mail und würden sich selbst wieder auslösen. Der
  Server-Name des Zielordners gehört zur Version der Regel: Ein umbenannter
  oder gelöschter Ordner macht eine neue Version, die neu gepusht wird.
- **Vom Server einsortierte Mail:** `fileinto` setzt das Keyword
  `$SatchelFiled`. Satchel lässt beim Sync seine Regeln auch auf neue Mail
  mit diesem Keyword laufen, die nicht in der Inbox liegt (außer Spam und
  Papierkorb). Sonst sähe eine Regel, die nur lokal läuft, die Mail nie. Die
  Regeln, die der Server schon ausgeführt hat, finden die Mail dort, wo sie
  sie hinlegen würden; ihre Verschiebung tut nichts.
- **Weiterleiten, was auf dem Server laufen kann:** Satchel führt Buch,
  welche Versionen einer Regel auf dem Server laufen können, und seit wann:
  die zuletzt bestätigte (`pushed_rules`, `pushed_rules_since`) und jede,
  die ein unbestätigter Push geschrieben haben kann (`possibly_live_rules`,
  mit dem Hash ihres Skripts). Ein Push trägt seine Versionen vor dem
  Schreiben ein; scheitert er oder bricht er ab, bleiben alte und neue
  stehen. Gelöscht wird nur, was Satchel sicher weiß: ein bestätigter Push
  (dann gilt nur er), ein bestätigtes Abschalten, oder der Rücklesecheck.
  Eine unveränderte Regel behält ihren Startzeitpunkt über jeden Push.
- **Rücklesecheck:** Höchstens alle 10 Minuten liest ein Sync vor dem
  Verarbeiten neuer Mail das aktive Skript zurück (LISTSCRIPTS, GETSCRIPT,
  Hash). Ist es Satchels bestätigtes, bleibt alles; ist es das eines
  unbestätigten Pushs, gilt dieses; läuft keins von Satchels, übernimmt
  Satchel alle Weiterleitungen wieder. Entspricht der aktuelle Regelstand
  nicht dem bestätigten Skript (Regel oder Ordner geändert), wird neu
  gepusht.
- **Wer leitet weiter:** Maßgeblich ist die Weiterleitung, nicht die ganze
  Regel: Auslöser und Adresse (`pushed_forwards`, und je Version in
  `possibly_live_rules`), mit eigenem Startzeitpunkt. Kam die Mail an,
  während eine Weiterleitung der Regel an dieselbe Adresse auf dem Server
  laufen konnte, deren Auslöser auf die Mail passt (Empfangszeit des Servers,
  zwei Minuten Spielraum für Uhrabweichung), leitet Satchel sie nicht weiter.
  Ein später hinzugefügtes Anheften ändert daran nichts; passt die Mail auf
  einen älteren, dort noch laufenden Auslöser, ist sie die des Servers. Der Server markiert, was er weitergeleitet hat, mit einem
  Keyword für alle Regeln, `$SatchelFwd`: Server begrenzen eigene Keywords
  pro Postfach (Dovecots Maildir auf 26, Cyrus auf 128). Welche Regel lief,
  folgt aus dem Buch oben; eine markierte Mail einer Regel, die dort laufen
  kann, ist auch dann die des Servers, wenn sie vor deren Startzeitpunkt
  ankam. Eine neue Adresse ist eine andere Weiterleitung: Sie übernimmt
  Satchel, bis der Server sie hat.
- **Was das kostet:** Im Zweifel entscheidet Satchel für den Server. Wurde
  Satchels Skript auf dem Server abgeschaltet, ohne dass Satchel es weiß,
  fehlen Weiterleitungen bis zum nächsten Rücklesecheck (höchstens 10
  Minuten nach dem nächsten Sync, und nie, solange Satchel geschlossen ist).
  Nach einem erzwungenen Abschalten (Server weg oder verweigert die
  Anmeldung) bleiben die Versionen als „kann laufen“ stehen; Satchel kann
  nicht sehen, ob das Skript noch läuft. Wie viele Weiterleitungen dem Server
  überlassen wurden, steht an der Regel.
- **Einmal pro Message-ID:** Jede Entscheidung steht in `rule_forwards`
  (Konto, Regel, Message-ID) und wird in derselben Transaktion geschrieben.
  Über Konten hinweg leitet Satchel eine Message-ID je Regel einmal weiter.
  Laufen zwei Server dieselbe unscopete Regel, leitet jeder weiter; Satchel
  kann das nicht verhindern. Zeilen gehen mit ihrem Konto und nach 180 Tagen.
- **Schleifen und Fluten:** Keine Seite leitet weiter, was `Auto-Submitted`
  außer `no` trägt, ein Bounce ist (leerer Envelope-Absender, mit `envelope`
  geprüft, sonst über `Return-Path: <>`; Absender `postmaster` oder
  `mailer-daemon`), ein Bericht (`multipart/report`: DSN, MDN),
  Listen- oder Massenmail (`List-Id`, `List-Unsubscribe`, `Precedence: bulk,
  list, junk`), Spam (`X-Spam-Flag: YES`, `X-Spam-Status: Yes`, oder im
  Spam-Ordner), oder was vom Ziel selbst kommt oder an es antwortet (`From`,
  `Reply-To`, auch mit `+tag`). Eine eigene Adresse, auch mit `+tag`, ist als Ziel abgelehnt.
  Mehr als 20 Weiterleitungen einer Regel in einer Stunde pausieren ihre
  Weiterleitung, bis die Person sie ausdrücklich fortsetzt (Speichern der
  Regel reicht nicht). Gezählt wird, was am Schleifenschutz vorbei an die
  Outbox ging, ab dem letzten Fortsetzen; geprüft beim Regellauf und noch
  einmal, bevor eine Weiterleitung an die Outbox geht. Sieve kennt keinen
  Zähler, dort gelten die Grenzen des Servers.
- Weitergeleitet wird nur ankommende Mail. Regeln von Hand auf vorhandene
  Mail anzuwenden (offene Mail, sichtbare Liste) leitet nichts weiter.
- Satchel leitet als Anhang weiter (`message/rfc822`, Original byte-genau,
  höchstens 20 MiB), über die Outbox mit stabiler Message-ID, mit
  `Auto-Submitted: auto-generated` und nie mit Lesebestätigung. Der Server
  dagegen leitet per `redirect` um: Die Mail geht mit ihrem eigenen Absender
  weiter, was SPF/DMARC beim Empfänger scheitern lassen kann. Je nachdem,
  wer weiterleitet, kommt beim Ziel also eine andere Mail an.
- „Kopie behalten“ betrifft nur den Server (`redirect :copy` statt
  `redirect`). Ohne Kopie behält der Server die Mail nicht; scheitert die
  Weiterleitung, ist sie weg. Der Editor fragt dafür nach. Leitet Satchel
  weiter, ist die Mail schon da und bleibt. Das ist die eine gewollte
  Abweichung zwischen beiden Seiten.
- Fehlt dem Server eine Erweiterung (`fileinto`, `imap4flags`, `copy`), bleibt
  die Regel lokal, und das Badge nennt die Erweiterung, je Aktion. „Server“
  heißt: Diese Version läuft bestätigt auf jedem Konto, für das die Regel
  gilt. `mailbox` wird genutzt, wenn vorhanden (`mailboxexists`: einen auf
  dem Server gelöschten Ordner legt die Regel nicht neu an).

## Folgen

- Andere IMAP-Clients sehen zwei Keywords. Der Name sagt, woher sie kommen;
  Satchel liest nur die eigenen.
- Kann der Server ein Keyword nicht anlegen (volle Keyword-Tabelle), fehlt
  nur die Markierung; die Weiterleitung bleibt nach dem Buch die des
  Servers. Ungeprüft ist, ob ein Server dann die ganze Zustellung abweist.
- Zwei Satchel-Installationen am selben Konto teilen `rule_forwards` nicht.
  Läuft eine Regel mit Weiterleitung nicht auf dem Server, leiten beide
  Geräte weiter. Erst der Hub (ADR 0008) kann das lösen.
- Mail, die ohne Kopie weitergeleitet wurde, erreicht Satchel nicht mehr;
  Regeln, die nur lokal laufen, sehen sie nicht.
