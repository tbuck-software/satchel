# 0015 Sync-Transport für den Hub

Status: angenommen 2026-11-05. Ergänzt 0008 und 0009. Ersetzt in 0010 den
Transport über HTTPS, das Pinning des Zertifikats und das Pairing „am Hub“
(bei einem blinden Hub über ein gepaartes Gerät). Lockert in 0008 die
Festlegung auf axum und die Rolle des Hubs als Autorität für
Einmal-Aktionen (ein blinder Hub vergibt nur Leases).

## Kontext

0008 legt einen Hub fest, 0010 das Pairing dazu, beides mit HTTPS als
Transport und Tailscale/WireGuard als bequemem Weg. Ohne VPN bleiben zwei
Fälle offen:

- Ein Hub auf einem öffentlichen Server (VPS) ist erreichbar, aber Port,
  Zertifikat und Firewall sind Handarbeit.
- Ein Hub zu Hause hinter NAT (Mac mini, NAS) ist von unterwegs ohne
  Portweiterleitung gar nicht erreichbar.

Außerdem will der Maintainer wissen, ob das Projekt später selbst etwas
betreiben könnte, das die Einrichtung einfacher macht. 0001 schließt einen
gehosteten Dienst heute aus. Diese Tür soll trotzdem billig offen bleiben.

## Entscheidung

### Transport: iroh

- Hub und Geräte verbinden sich über **iroh** (n0, Rust). iroh baut
  QUIC-Verbindungen zwischen Endpunkten, die über ihren öffentlichen
  Schlüssel (EndpointId) adressiert werden. Es versucht zuerst eine direkte
  Verbindung per NAT-Traversal und leitet über ein Relay, wenn die scheitert.
  Die Verbindung ist Ende-zu-Ende verschlüsselt. Das Relay sieht nur
  Chiffretext, dazu die EndpointIds der Gegenstellen und die Datenmenge.
- Ein Transport für beide Fälle. Ein Hub auf einem öffentlichen Server ist
  direkt erreichbar, einer hinter NAT per Hole-Punching oder über ein Relay.
  Ein zweiter Weg über eigenes HTTPS entfällt; wo UDP gesperrt ist, trägt das
  Relay die Verbindung.
- Das Pinning aus 0010 bleibt, nur am Schlüssel statt am Zertifikat: Das
  Gerät speichert beim Pairing die EndpointId des Hubs, iroh authentifiziert
  die Gegenstelle über genau diesen Schlüssel. Der Hub nimmt nur EndpointIds
  gepaarter Geräte an.
- Die Hub-API aus 0008 läuft als eigenes Protokoll (ALPN) über
  iroh-Verbindungen. Ob darin HTTP liegt (axum, wie 0008 vorsah) oder ein
  eigenes Nachrichtenformat, entscheidet die Umsetzung.
- Tailscale/WireGuard bleiben möglich. iroh findet dann den direkten Weg.

### Relays und Adresssuche sind Konfiguration

- Welche Relays ein Gerät benutzt, ist eine Liste in den Einstellungen
  (`RelayMode::Custom` mit `RelayMap`), kein Code. Einträge können sein:
  ein selbst betriebenes `iroh-relay`, die öffentlichen Relays von n0 und
  später ein Relay des Projekts (nur nach einem ADR, das 0001 ändert, siehe
  unten).
- Die Liste reist mit dem Pairing und danach als Einstellung im Ereignis-Log.
  Den Standard zu wechseln braucht deshalb keine neue Client-Version.
- `iroh-relay` ist eine einzelne Binary oder das Docker-Image
  `n0computer/iroh-relay`. Es braucht eine öffentliche IP und einen
  DNS-Namen; TLS per ACME ist eingebaut. Wer den Hub auf einem VPS betreibt,
  stellt das Relay daneben und trägt es als ersten Eintrag ein.
- Die öffentlichen Relays von n0 kosten nichts, sind ohne veröffentlichte
  Grenzen rate-limitiert und haben keine Verfügbarkeitszusage. n0 nennt sie
  „suitable for development and testing“ und rät für Produktion davon ab.
  Satchel trägt sie deshalb nur als Startwert ein, damit ein Pairing ohne
  eigenen Server funktioniert, und kennzeichnet sie in den Einstellungen als
  fremde Infrastruktur ohne Garantie. Sync-Verkehr ist klein, und nach dem
  ersten Kontakt läuft er meist direkt. Ein eigenes Relay bleibt die
  Empfehlung für den Alltag.
- Ein bloßer Schlüssel reicht iroh nicht: Die Adressen dazu sucht es über
  einen Lookup-Dienst, standardmäßig n0s DNS/Pkarr unter `dns.iroh.link`.
  Satchel hängt nicht davon ab. Das Pairing speichert die vollständige
  Adresse des Hubs (EndpointId und Relay-URL). Ein Lookup-Dienst, von n0
  oder selbst betrieben (`iroh-dns-server`), ist wie ein Relay ein optionaler
  Eintrag.
- Die Voreinstellung von iroh veröffentlicht EndpointId und Adressen jedes
  Geräts bei n0. Satchel nutzt sie nicht (kein `presets::N0`): Solange kein
  Hub gepaart ist, startet kein Endpoint, und veröffentlicht wird nur, wenn
  der Nutzer einen Lookup-Dienst einträgt. Eine App, die beim Start einen fremden Dienst
  kontaktiert, widerspräche dem Local-first-Versprechen.
- Maßgeblich ist die Relay-Liste im Ereignis-Log; ein Gerät darf lokal
  Einträge ergänzen, aber keine entfernen. Wechselt das Relay des Hubs,
  schreibt ein gepaartes Gerät die neue Liste zuerst ins Log, und der Hub
  bleibt eine Übergangszeit über das alte Relay erreichbar. Jedes Gerät
  behält die zuletzt bekannte Liste als Rückfall und versucht beim
  Verbindungsaufbau alle Einträge. Wer länger offline war als die
  Übergangszeit, findet den Hub über einen eingetragenen Lookup-Dienst oder
  paart sich neu.

### Ereignis-Log Ende-zu-Ende verschlüsselt

- Das Ereignis-Log aus 0009 (Topics, Regeln, Smart-Folder, Signaturen,
  geplante Sends, Follow-ups, Snooze-Wecker, Sender-Präferenzen) wird auf
  dem Gerät verschlüsselt, bevor es das Gerät verlässt. Der **Log-Schlüssel**
  ist symmetrisch und allen gepaarten Geräten gemeinsam. Das erste Gerät, das
  das Log anlegt, erzeugt ihn.
- Das Pairing überträgt den Log-Schlüssel. Anders als Kontodaten und private
  Schlüssel in 0010 ist er nicht abwählbar, denn ohne ihn liest ein Gerät das
  Log nicht. Der QR-Code trägt die Adresse der Gegenstelle (EndpointId und
  Relay-URL) und den Einmal-Code, nicht den Schlüssel; der geht danach über
  die iroh-Verbindung.
- Beim Pairing mit einem Hub legt der Nutzer dessen Rolle fest. Nur bei
  „vertrauter Hub“ sendet das Gerät den Log-Schlüssel an den Hub. Ein blinder
  Hub bekommt ihn nie; dort koppelt ein neues Gerät über ein schon gepaartes
  (siehe unten).
- Wird ein Gerät entfernt, sperrt der Hub dessen EndpointId. Auslösen darf
  das jedes gepaarte Gerät; der Hub prüft dafür dessen Signatur. Danach
  rotiert ein Gerät mit Schlüssel den Log-Schlüssel: Der neue Schlüssel geht
  als Eintrag ins Log, einzeln für die EndpointId jedes verbleibenden Geräts
  verpackt, sodass auch Geräte, die gerade offline sind, ihn später abholen.
  Frühere Schlüssel bleiben als Kette erhalten, damit neue Geräte alte
  Ereignisse lesen. Neue Ereignisse sind für das entfernte Gerät unlesbar.
  Was es schon hatte, behält es.
- Das Rotieren widerruft keine Passwörter, die das entfernte Gerät beim
  Pairing bekommen hat (0010). Wer ein Gerät verliert, ändert sie beim
  Anbieter. OAuth-Konten trifft das nicht, weil jedes Gerät eigene Tokens hat
  (0009).

### Zwei Hub-Rollen

- Ein **vertrauter Hub** gehört dem Nutzer und hält den Log-Schlüssel. Er
  darf Einmal-Aktionen selbst ausführen. Für geplantes Senden braucht er die
  Zugangsdaten des Kontos: Passwörter per Pairing (0010), für OAuth-Konten
  eine eigene Autorisierung, weil Tokens nie übertragen werden (0009). Ein
  Hub ohne Bildschirm startet den Flow selbst und behält den PKCE-Verifier
  (in `oauth_flow_sessions`, übersteht also einen Neustart). Der Nutzer
  meldet sich im Browser eines gepaarten Geräts an, und nur der einmalige
  Autorisierungscode reist zum Hub. Das Token entsteht auf dem Hub, verlässt
  ihn nie und lässt sich beim Anbieter einzeln widerrufen.
  Beispiele: `satchel --headless` auf eigenem VPS, NAS oder Mac mini, oder
  der Desktop-Client mit Lease (0008).
- Ein **blinder Hub** hat den Schlüssel nicht. Er speichert Chiffretext,
  bringt Ereignisse in eine Reihenfolge und vergibt Leases für
  Einmal-Aktionen. Ausführen tut das nächste Gerät, das online ist und die
  Lease bekommt. Die Aktion kennt er nur als ID; Fälligkeit und Inhalt liegen
  im Chiffretext. Er sieht, welche Geräte wann wie viel schreiben.
- Ereignisse sind authentifiziert: Jedes Gerät signiert seine Ereignisse,
  Reihenfolge und Zähler stehen im Chiffretext. Ein blinder Hub kann
  verzögern oder auslassen, aber nicht fälschen oder unbemerkt wiederholen.
  Einmal-Aktionen sind idempotent (beim Senden über die Message-ID), damit
  eine doppelt vergebene Lease nicht doppelt sendet.
- Ein Gerät paart sich bei einem blinden Hub über ein schon gepaartes Gerät:
  Das zeigt den QR-Code, übergibt den Log-Schlüssel und meldet dem Hub die
  neue EndpointId. Bei einem vertrauten Hub geht beides.
- **Jeder Hub und jedes Relay, das nicht dem Nutzer gehört, ist blind.** Das
  gilt auch für alles, was das Projekt später betreibt. Vertraut wird ein Hub
  nicht durch eine Einstellung, sondern dadurch, dass der Nutzer ihm den
  Log-Schlüssel gibt.
- Der Preis: Mit einem blinden Hub geht ein für 3 Uhr geplanter Send erst
  raus, wenn morgens das erste Gerät aufwacht. Wer pünktlich senden will,
  während alle Geräte aus sind, braucht einen vertrauten Hub, also einen
  eigenen Rechner, der läuft und die Zugangsdaten hat.

### Tür für einen Dienst des Projekts

Ein Relay oder blinder Hub, den das Projekt betreibt, müsste weder Mail noch
Zugangsdaten lesen: Das Relay sieht iroh-Chiffretext, der blinde Hub
Log-Chiffretext. Er wäre ein weiterer Eintrag in der Relay-Liste oder eine
Hub-Adresse beim Pairing, ohne Änderung am Client. Ihn tatsächlich
anzubieten, mit Betrieb, Missbrauchsschutz, Kosten und Verfügbarkeit, ist
ein gehosteter Dienst im Sinne von 0001 und braucht ein eigenes ADR, das
0001 ändert.

## Konsequenzen

- In 0009 heißt „Hub (Ereignis-Log)“ jetzt: Chiffretext auf dem Hub,
  Klartext nur auf Geräten und vertrauten Hubs.
- Neue Abhängigkeit iroh (1.x, `MIT OR Apache-2.0`, verträglich mit der
  MIT-Lizenz von Satchel). `iroh-relay` enthält zusätzlich eine
  `LICENSE-BSD3`; was sie abdeckt, prüfen wir, bevor wir das Relay selbst
  ausliefern. n0 empfiehlt eigene Relays für alle, die eine bestimmte
  Relay-Version brauchen. Solange der Startwert n0s Relays enthält, sollte
  iroh im Client nicht weit hinter der aktuellen Version liegen.
- Geräteschlüssel (EndpointId) und Log-Schlüssel werden wie Zugangsdaten
  behandelt: verschlüsselt mit dem Master-Key und umschlossen im Backup
  (siehe AGENTS.md, „Datenbank-Betrieb“). Sind alle Geräte und alle Backups
  verloren, ist das Log unlesbar. Mail, Done und Snooze liegen auf dem
  Mail-Server (0009) und bleiben; Topics, Regeln und geplante Sends sind weg.
- Das Bedrohungsmodell aus 0010 gilt weiter: Wer den QR-Code fotografiert,
  bekommt zusätzlich den Log-Schlüssel.
- Vor der Umsetzung klärt ein Probe-Build, ob iroh für Android und iOS
  baut (0006). Scheitert er, ist das ein eigenes ADR wert.
- Nicht entschieden sind hier: Wire-Format, Verschlüsselungsverfahren und
  Schlüsselrotation im Detail, Kompaktierung des Logs, Push aufs Handy.

## Quellen

- iroh, README und Lizenz: https://github.com/n0-computer/iroh
- Relays, Verschlüsselung, öffentliche Relays: https://docs.iroh.computer/concepts/relays
- Rate-Limits der öffentlichen Relays: https://docs.iroh.computer/relays/rate-limiting
- Eigenes Relay betreiben, `RelayMode::Custom`: https://docs.iroh.computer/add-a-relay
- `iroh-relay` (Binary, Zugriffsregeln): https://github.com/n0-computer/iroh/tree/main/iroh-relay
- Docker-Image: https://hub.docker.com/r/n0computer/iroh-relay
- Preise und Garantien von n0: https://www.iroh.computer/pricing
- `iroh-dns-server` (selbst betriebener Lookup-Dienst): https://github.com/n0-computer/iroh/tree/main/iroh-dns-server
