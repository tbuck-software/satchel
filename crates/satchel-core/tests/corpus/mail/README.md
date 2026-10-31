# Synced-mail corpus

Ordinary, unencrypted messages for the parsers that read synced mail:
`parse_raw_message_body` (Gmail `raw`, Outlook `raw_mime`) and
`parse_message_body` (IMAP). The tests live in `src/providers/imap.rs`.

| File | Holds |
| --- | --- |
| `root-text-attachment.eml` | A root `text/plain` part labelled `attachment; filename="note.txt"`. Synced mail keeps showing it as the body; only decrypted payloads list such a part as a file |
