# Mail-Korpus (Regressionstests)

Jede `.eml` in diesem Verzeichnis wird von `src-tauri/tests/corpus.rs` durch
denselben Parser-Pfad geschickt, den eingehende IMAP-Nachrichten nehmen:
MIME-Zerlegung, Charset-Dekodierung, HTML-Sanitizing, Textextraktion.

## Zwei Quellen

| Ort | Eingecheckt | Zweck |
| --- | --- | --- |
| `src-tauri/tests/corpus/` | ja | synthetische, anonymisierte Fälle |
| `<repo>/.private/corpus/` | nein | echte Mails vom eigenen Rechner |

Das private Verzeichnis ist optional. Fehlt es, überspringt der Test es
stillschweigend — CI läuft nur gegen den eingecheckten Teil. Echte Mails
gehören **nie** ins Repo; wer einen Fehler aus einer echten Nachricht
reproduziert, baut daraus einen anonymisierten Fall hier.

## Einen Fall hinzufügen

1. `<name>.eml` ablegen. Die Bytes müssen echt sein: eine als
   ISO-8859-1 deklarierte Mail muss auch ISO-8859-1-Bytes enthalten, keine
   UTF-8-Bytes. `generate.py` in diesem Verzeichnis erzeugt die vorhandenen
   Fälle und ist die Vorlage für neue (`python3 generate.py`).
2. Optional `<name>.expect.json` daneben legen. Nur vorhandene Schlüssel
   werden geprüft:

```json
{
  "subject": "Angebot für die Büromöbel",
  "body_contains": ["Büromöbel", "Größe"],
  "charset": "iso-8859-1",
  "allow_replacement": false
}
```

- `subject` — exakter Betreff nach RFC-2047-Dekodierung.
- `body_contains` — Teilstrings, die im extrahierten Text vorkommen müssen.
- `body_excludes` — Teilstrings, die **nicht** vorkommen dürfen. Eine
  MIME-Grenze oder ein Part-Header hier heißt: der Splitter hat an der
  falschen Stelle geschnitten.
- `attachments` — alle Anhänge in Reihenfolge, je mit `filename` und `size`
  (dekodierte Bytes). Eine 0 ist genau das, was eine abgelehnte
  Transfer-Kodierung hinterlässt, deshalb ist die Zahl die Zusage.
- `charset` — erwartetes dekodiertes Charset (Groß-/Kleinschreibung egal).
- `allow_replacement` — nur auf `true` setzen, wenn U+FFFD im Text
  **korrekt** ist, weil die Quelldatei tatsächlich kaputte Bytes enthält.
  Für alles andere ist ein U+FFFD ein Bug im Decoder, kein Testproblem.

Ohne Sidecar gelten nur die Grundzusagen: Parsen gelingt, vorhandene
Header (Subject/From/Date) landen nicht leer im Ergebnis, ein vorhandener
Text- oder HTML-Teil ergibt nicht-leeren Text, und der Text enthält kein
U+FFFD.

## Ausführen

```bash
cargo test --manifest-path src-tauri/Cargo.toml --test corpus
cargo test --manifest-path src-tauri/Cargo.toml --test corpus -- --ignored
```

## Zwei Tests sind `#[ignore]` — und zwar zu Recht

`corpus_bodies_match_expected_content` und
`corpus_bodies_have_no_replacement_characters` schlagen fehl. Sie beschreiben
einen echten Fehler, keine zu strenge Erwartung:

`commands::message_file::parse_message_bytes` legt
`String::from_utf8_lossy` über die **gesamte** Nachricht, bevor irgendein
Charset gelesen wird; `providers::imap::parse_message_body` tut dasselbe mit den
Body-Bytes aus dem IMAP-Fetch. Jedes Byte über 0x7F wird damit zu U+FFFD,
lange bevor `decode_bytes_with_charset` es sehen könnte — und dekodiert danach
noch die UTF-8-Bytes des Ersatzzeichens als ISO-8859-1. Aus „für" wird „fï¿½r".

Betroffen ist ausschließlich `Content-Transfer-Encoding: 8bit` (sowie `binary`
und ein fehlender Header). Quoted-printable und Base64 überleben, weil sie auf
der Leitung ASCII sind — deshalb ist der Fehler nie aufgefallen.

Wer ihn behebt, nimmt die beiden `#[ignore]` wieder weg. Bis dahin bleiben die
Zusagen stehen, statt abgeschwächt zu werden.
