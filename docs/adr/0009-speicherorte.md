# 0009 Speicherorte: Mail-Server, Hub, Gerät

Status: angenommen 2026-08-27

## Entscheidung

| Ort | Was |
|---|---|
| Mail-Server (IMAP/Gmail/Graph) | Mail, Ordner, Read/Flag, **Done und Snooze als IMAP-Keyword bzw. Gmail-Label** — damit sehen es auch fremde Clients |
| Hub (Ereignis-Log) | Topics, Regeln, Smart-Folder, Signaturen, geplante Sends, Follow-ups, Snooze-Wecker, Sender-Präferenzen |
| Gerät | Body-Cache, UI-Zustand, Zugangsdaten, private Schlüssel |

Zugangsdaten und Schlüssel werden **nicht** synchronisiert, nur beim
Pairing übertragen (0010). OAuth-Tokens werden nie übertragen; jedes
Gerät autorisiert selbst (Deep-Link-Flow existiert), damit ein verlorenes
Gerät einzeln beim Anbieter abgemeldet werden kann.

## Konsequenzen

Done/Snooze wandern vom lokalen `workflow_events` auf den Server —
Provider brauchen `set_keywords`. Was heute in 24 Dateien direkt in
`localStorage` schreibt, wird auf „Gerät" oder „Hub" sortiert.
