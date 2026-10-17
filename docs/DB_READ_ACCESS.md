# Lesezugriff auf die Satchel-Datenbank

Status: gültig ab 2026-08-28
Grundlage: `docs/adr/0005-api-cli-relay-entfernen.md`
Schema: `src-tauri/src/db/migrations/0059_ai_read_views.sql`

Satchel hat keine API. KI-Werkzeuge und Skripte lesen die SQLite-Datei direkt.
Damit das nicht bei jeder Schema-Änderung bricht, gibt es einen schmalen
Vertrag: die `v_*`-Views. Wer Satchel von außen liest, liest Views — nie die
Tabellen.

## Die Regel

> Spalten in `v_*` werden nie umbenannt oder entfernt, nur ergänzt; wer eine
> Tabelle ändert, passt die View an.

Die Views sind die Abstraktionsschicht, die es sonst nicht gibt. `threads`,
`messages` und `accounts` sind seit `0001_init` stetig gewachsen und wachsen
weiter; die View entkoppelt den Leser davon. Umbenennen einer Tabellenspalte
heißt: View im selben Commit nachziehen, damit der Spaltenname draußen gleich
bleibt. `src-tauri/tests/db_views.rs` fängt ab, dass eine View still nicht mehr
auflöst.

## Wo die Datei liegt

Pro Kanal ein eigenes Verzeichnis unter Application Support, abgeleitet aus der
Bundle-ID (siehe `app_support_dir` in `src-tauri/src/bin/satchel-cli.rs`):

| Kanal  | Pfad                                                                  |
| ------ | --------------------------------------------------------------------- |
| stable | `~/Library/Application Support/de.tbuck.satchel/satchel/satchel.db`       |
| daily  | `~/Library/Application Support/de.tbuck.satchel.daily/satchel/satchel.db` |
| dev    | `~/Library/Application Support/de.tbuck.satchel.dev/satchel/satchel.db`   |

Daneben liegen `satchel.db-wal` und `satchel.db-shm`. Der Master-Key liegt im
OS-Schlüsselbund unter dem Service `satchel`; der Account heißt
`pgp-master-key-v1:<SHA-256 der nativen Profilpfad-Bytes>`. Eine alte Klartextdatei
`pgp-master-key-v1` wird nach verifiziertem Schlüsselbund-Import gelöscht.
Die Views enthalten nichts, was den Schlüssel bräuchte.

## Read-only öffnen

Die App hält einen Pool offener Verbindungen und die Datenbank läuft im
WAL-Modus. Ein zweiter Schreiber ist ein Problem, ein zweiter Leser nicht —
solange er sich als Leser zu erkennen gibt:

```bash
sqlite3 -readonly "$HOME/Library/Application Support/de.tbuck.satchel/satchel/satchel.db" \
  "SELECT id, subject FROM v_threads ORDER BY last_message_date DESC LIMIT 20;"
```

```python
import sqlite3
path = "/Users/…/de.tbuck.satchel/satchel/satchel.db"
con = sqlite3.connect(f"file:{path}?mode=ro", uri=True)
```

Nicht `db`, `wal` und `shm` kopieren, um „sicher" zu lesen — das fängt einen
halben Schreibvorgang ein. Wer einen Schnappschuss will, nimmt `VACUUM INTO`
(die App macht das im Backup genauso).

## Die Views

Jede View trägt ihren Vertrag als Kommentarblock in der Migration. Kurzfassung:

### `v_accounts`

Konten und sonstige Quellen. Identität, keine Zugangsdaten.

`id, kind, provider, email, display_name, color, is_active, last_sync_at,
created_at, updated_at`

`accounts.config` (dort liegt die verschlüsselte Zugangsinformation),
`credentials_ref` und die Sync-Fehlertexte fehlen absichtlich.

```sql
SELECT id, kind, email, display_name FROM v_accounts WHERE is_active = 1;
```

### `v_folders`

Ordner je Konto. `folder_type` ist die Rolle (`inbox`, `sent`, `trash`,
`archive`, `drafts`, `junk`, …), `name` das Blatt, `full_path` der volle Pfad.

`id, account_id, account_email, name, full_path, folder_type, message_count,
unread_count, sync_enabled, sort_order, created_at, updated_at`

`message_count`/`unread_count` werden in der View über `messages` gezählt und
stimmen mit `SELECT COUNT(*) FROM v_messages WHERE folder_id = …` überein. Die
Zahl im Mail-UI ist bewusst eine andere: dort werden Konversationen gezählt,
ohne erledigte, zurückgestellte und beiseitegelegte.

```sql
SELECT account_email, name, unread_count
FROM v_folders WHERE folder_type = 'inbox' ORDER BY account_email;
```

### `v_threads`

Konversationen — die Einheit, auf die Workflow, Board und Snooze wirken.

`id, account_id, account_email, subject, snippet, participants,
participants_json, message_count, unread_count, first_message_date,
last_message_date, workflow_state, workflow_changed_at, snoozed_until,
due_date, is_starred, is_muted, has_attachments, priority, category, assignee,
tags, topic_ids, created_at, updated_at`

- `participants` ist eine kommagetrennte Adressliste; `participants_json` das
  rohe Array (Einträge sind `{"name","email"}`-Objekte oder blanke Adressen, je
  nachdem, wann die Zeile geschrieben wurde).
- `tags` sind kommagetrennte Tag-**Namen**, `topic_ids` kommagetrennte
  Themen-IDs.
- `snoozed_until` ist `threads.snooze_until`: liegt der Zeitpunkt in der
  Zukunft, ist der Thread bis dahin ausgeblendet.
- `first_message_date` wird aus den Nachrichten berechnet und ist `NULL`,
  solange der Thread noch keine hat.

```sql
SELECT subject, workflow_state, unread_count, tags
FROM v_threads
WHERE workflow_state = 'todo'
ORDER BY last_message_date DESC
LIMIT 20;
```

### `v_messages`

Einzelne Nachrichten.

`id, thread_id, account_id, account_email, folder_id, folder_name, folder_type,
from_address, from_name, to_addresses, cc_addresses, subject, date, is_read,
is_draft, is_sent, is_starred, has_attachments, snippet, body_text,
has_body_html, size_bytes, category, list_unsubscribe, created_at, updated_at,
message_id_header`

- `body_text` ist der Klartext. Der sanitisierte HTML-Body fehlt: er ist die
  größte Spalte der Datenbank und als Text unbrauchbar. `has_body_html` sagt
  nur, dass es ihn gibt.
- `to_addresses` und `cc_addresses` sind JSON-Arrays — mit `json_each`
  entfalten. Bcc wird nicht ausgegeben.
- `thread_id` darf `NULL` sein: Entwürfe und Importe werden nachträglich
  einsortiert.
- `message_id_header` ist der rohe `Message-ID`-Header samt spitzen Klammern
  (`<abc@host>`). Seit `0080`.

```sql
SELECT date, from_address, subject
FROM v_messages
WHERE thread_id = 'th-1'
ORDER BY date;

SELECT m.subject, adr.value AS empfaenger
FROM v_messages m, json_each(m.to_addresses) adr
WHERE m.id = 'msg-1';
```

### `v_attachments`

Nur Metadaten, nie Inhalte. Die Bytes liegen als Datei auf der Platte;
`local_path` zeigt darauf und ist `NULL`, bis der Anhang geladen wurde.
`is_inline = 1` markiert Teile, die der HTML-Body referenziert
(Signaturbilder), keine echten Anhänge.

`id, message_id, thread_id, account_id, filename, content_type, size_bytes,
is_inline, content_id, is_downloaded, local_path, created_at`

```sql
SELECT filename, content_type, size_bytes
FROM v_attachments
WHERE is_inline = 0 AND size_bytes > 1000000
ORDER BY size_bytes DESC;
```

### `v_topics`

Themen — quellenübergreifende Sammlungen zu einem laufenden Vorgang.
`status` ist `active | done | archived`, `due_date` die Wiedervorlage.

`id, name, description, status, color, icon, due_date, thread_count,
message_count, note_count, file_count, created_at, updated_at`

Die Zähler zählen Verknüpfungen; verwaiste Links auf gelöschte Threads sind
möglich. Was wirklich existiert, sagen `v_threads` und `v_messages`.

```sql
SELECT name, status, due_date, thread_count
FROM v_topics WHERE status = 'active' ORDER BY due_date;
```

### `v_tags`

Tags samt Nutzung. `is_system = 1` sind vom Programm angelegte Tags.

`id, name, color, icon, is_system, sort_order, thread_count`

```sql
SELECT name, thread_count FROM v_tags ORDER BY thread_count DESC;
```

## Links: `satchel://`

„Link kopieren“ im Thread-Menü (`y`) legt einen Markdown-Link in die
Zwischenablage, etwa für einen T3-Thread oder eine Notiz:

```text
[Rechnung März](satchel://message/rechnung-1@app.io)
```

- `satchel://message/<id>` trägt die Message-ID **ohne** spitze Klammern,
  URL-kodiert (`@` bleibt lesbar). Sie ist auf jedem Gerät und in jedem
  Mail-Programm dieselbe. Der Link zeigt auf die erste echte Nachricht des
  Threads.
- `satchel://thread/<id>` ist der Rückfall für Mail ohne echte Message-ID und
  gilt nur in der Datenbank, aus der er stammt.

Ein Klick öffnet den Thread in Satchel, auf dem Desktop und unter Android
(nur in der gebauten App; macOS kennt das Schema erst über das `.app`-Bundle).
Wer den Link liest, statt ihn zu öffnen, löst ihn so auf:

```sql
SELECT thread_id FROM v_messages
WHERE message_id_header = '<rechnung-1@app.io>';

SELECT date, from_address, subject, body_text
FROM v_messages WHERE thread_id = :thread_id ORDER BY date;
```

Dieselbe Mail kann in mehreren Ordnern oder Konten liegen; die App nimmt die
neueste Kopie außerhalb von Papierkorb und Spam.

## Was draußen bleibt

Keine View greift auf `accounts.config`, `credentials_ref`,
`oauth_flow_sessions`, `pgp_keys`, `smime_certificates`, die
`smime_*`-Blobs der Nachrichten, `api_tokens`, `api_audit_log` oder
`plugin_state` zu. Der Test prüft, dass kein Spaltenname in einer `v_*`-View
`config`, `token`, `secret`, `password` oder `credential` enthält.

Das ist keine Zugriffskontrolle — wer die Datei lesen kann, kann auch die
Tabellen lesen. Es ist eine Vorgabe für den Vertrag: was durch die Views
erreichbar ist, darf ein Werkzeug ohne Nachdenken verwenden.

## Kosten

Die Views sind für Ad-hoc-Abfragen gedacht, nicht für die heißen Pfade der App.
`v_threads` berechnet Teilnehmer, Tags, Themen und `first_message_date` über
korrelierte Unterabfragen — auf einer Mailbox mit 100.000 Threads ist ein
`SELECT * FROM v_threads` ohne `WHERE` teuer. Auf IDs filtern und `LIMIT`
setzen.
