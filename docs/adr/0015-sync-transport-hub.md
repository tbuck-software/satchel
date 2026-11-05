# 0015 Sync-Transport für den Hub

Status: angenommen 2026-11-05. Ergänzt 0008 und 0009; ersetzt in 0010 den
Punkt „Transport über HTTPS des Hubs“.

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
  Die Verbindung ist Ende-zu-Ende verschlüsselt, das Relay sieht nur
  Chiffretext.
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
  später ein Relay des Projekts.
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
- Wechselt der Hub sein Relay, schreibt er die neue Relay-Liste zuerst ins
  Ereignis-Log und bleibt eine Übergangszeit über das alte Relay erreichbar.
  Jedes Gerät behält die zuletzt bekannte Liste als Rückfall und versucht
  beim Verbindungsaufbau alle Einträge.

### Ereignis-Log Ende-zu-Ende verschlüsselt

- Das Ereignis-Log aus 0009 (Topics, Regeln, Smart-Folder, Signaturen,
  geplante Sends, Follow-ups, Snooze-Wecker, Sender-Präferenzen) wird auf
  dem Gerät verschlüsselt, bevor es das Gerät verlässt. Der **Log-Schlüssel**
  ist symmetrisch und allen gepaarten Geräten gemeinsam. Das erste Gerät, das
  das Log anlegt, erzeugt ihn.
- Das Pairing überträgt den Log-Schlüssel. Anders als Kontodaten und private
  Schlüssel in 0010 ist er nicht abwählbar, denn ohne ihn liest ein Gerät das
  Log nicht. Der QR-Code trägt Adresse und Einmal-Code, nicht den Schlüssel;
  der geht danach über die iroh-Verbindung.
- Wird ein Gerät entfernt, sperrt der Hub dessen EndpointId, und ein Gerät
  mit Schlüssel rotiert den Log-Schlüssel. Neue Ereignisse sind für das
  entfernte Gerät unlesbar. Was es schon hatte, behält es.

### Zwei Hub-Rollen

- Ein **vertrauter Hub** gehört dem Nutzer und hält den Log-Schlüssel. Er
  darf Einmal-Aktionen selbst ausführen. Für geplantes Senden braucht er die
  Zugangsdaten des Kontos: Passwörter per Pairing (0010), für OAuth-Konten
  eine eigene Autorisierung, weil Tokens nie übertragen werden (0009). Ein
  Hub ohne Bildschirm holt sie über den Browser eines gepaarten Geräts.
  Beispiele: `satchel --headless` auf eigenem VPS, NAS oder Mac mini, oder
  der Desktop-Client mit Lease (0008).
- Ein **blinder Hub** hat den Schlüssel nicht. Er speichert Chiffretext,
  bringt Ereignisse in eine Reihenfolge und vergibt Leases für
  Einmal-Aktionen. Ausführen tut das nächste Gerät, das online ist und die
  Lease bekommt. Die Aktion kennt er nur als ID; Fälligkeit und Inhalt liegen
  im Chiffretext. Er sieht, welche Geräte wann wie viel schreiben.
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
  MIT-Lizenz von Satchel). n0 empfiehlt eigene Relays für alle, die eine
  bestimmte Relay-Version brauchen. Solange der Startwert n0s Relays enthält,
  darf iroh im Client nicht lange hinter der aktuellen Version liegen.
- Das Bedrohungsmodell aus 0010 gilt weiter: Wer den QR-Code und den
  Einmal-Code abgreift, bekommt den Log-Schlüssel.
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
