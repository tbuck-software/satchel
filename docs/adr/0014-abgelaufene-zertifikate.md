# 0014 Ausnahmen für abgelaufene Server-Zertifikate

Status: angenommen 2026-11-04

## Kontext

Am 04.11.2026 lief bei einem Konto des Maintainers das Zertifikat eines
einzelnen IMAP-Knotens hinter einem Lastverteiler ab; die anderen Knoten
zeigten gültige Zertifikate, je nach Client traf es also nur manche. Satchel lehnte
die Verbindung zu Recht ab, zeigte aber eine abgeschnittene rustls-Fehlerkette
rot über der ganzen App. Ausblenden ließ sich die Meldung nicht, und ohne
Mail-Zugang gab es keinen Weg, bis der Betreiber das Zertifikat erneuert.

Andere Clients bieten an dieser Stelle „trotzdem vertrauen“ an, meist ohne
Ende und ohne Unterschied zwischen einem vergessenen Zertifikat und einem
fremden Server.

## Entscheidung

- Ein eigener Verifier (`providers::tls_trust::MailCertVerifier`) umhüllt den
  Plattform-Verifier (Desktop: gebündelte webpki-Roots, Mobile: OS-Store) für
  IMAP und POP3. Er lockert nichts von sich aus. Er hält fest, welches
  Zertifikat ein Host vorgelegt hat und warum es abgelehnt wurde, und gibt
  das als Key mit Parametern zurück
  (`backend.errors.tlsCertificateRejected`), nicht als Satz.
- Eine Ausnahme gibt es **nur** für ein abgelaufenes Zertifikat, dessen Kette
  und Name zu einem Zeitpunkt innerhalb seiner Gültigkeit verifizieren. Dafür
  prüft der Verifier ein zweites Mal mit `notAfter − 1 s` als Prüfzeit. Ein
  falscher Name, ein unbekannter Aussteller oder ein noch nicht gültiges
  Zertifikat bekommen keine Ausnahme.
- Die Ausnahme gilt für ein Konto, einen Host und genau einen
  SHA-256-Fingerabdruck (`tls_certificate_exceptions`), höchstens 7 Tage. Sie
  endet früher, sobald der Host wieder ein gültiges Zertifikat vorlegt
  (`tls_trust::settle` nach jedem Sync-Lauf), oder per Klick. Der Fingerabdruck
  kommt aus der aufgezeichneten Ablehnung, nicht vom Frontend: das Frontend
  bestätigt nur, was das Backend gesehen hat.
- Der Verifier läuft synchron im Handshake. Die Ausnahmen liegen deshalb
  zusätzlich im Prozessspeicher und werden beim Start sowie zu Beginn jedes
  Sync-Laufs aus der Datenbank geladen. Das deckt auch den mobilen
  Hintergrund-Worker in seinem eigenen Prozess ab.
- SMTP bleibt vorerst außen vor: lettre baut seine rustls-Konfiguration
  selbst, und ein abgelaufenes Zertifikat beim Senden landet ohnehin in der
  Outbox mit eigener Fehleranzeige.

## Konsequenzen

Ein Konto mit abgelehntem Zertifikat zeigt einen orangen Banner, der sich pro
Zertifikat dauerhaft ausblenden lässt, und ein Symbol an der Kontozeile. Wer
die Ausnahme nutzt, sieht sie dort als graues Schild, solange sie gilt.

Wer den Zeitraum verlängern, die Ausnahme auf andere Fehler ausweiten oder
SMTP einbeziehen will, schreibt ein neues ADR. Jede Lockerung des Verifiers
gehört in `tls_trust` und nirgendwo sonst.
