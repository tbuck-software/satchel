# Decrypted-payload corpus

Each file is the plaintext a decryption yields: one MIME entity, CRLF line
endings, synthetic content. The tests in `src/utils/smime_engine.rs` send every
file through the outbox's sign-then-encrypt transform and back through the read
path (`parse_raw_message_body` → `decrypt` → `parse_decrypted_body`), so a case
here exercises the production round trip, not just the parser. The OpenPGP
tests in `src-tauri/src/commands/messages.rs` read the same files as PGP/MIME
plaintext.

No key material is checked in. The S/MIME tests issue their certificates at
run time.

## Must decode, with every part accounted for

| File | Holds |
| --- | --- |
| `root-attachment.eml` | A binary attachment as the whole payload; bytes, disposition and Content-ID survive |
| `empty-text.eml` | An empty `text/plain` body |
| `image-only-html.eml` | HTML that is a single `cid:` image |
| `attached-envelope.eml` | A cover note and an encrypted mail attached as a file |
| `related-inline-image.eml` | HTML with an inline image that has a Content-ID but no filename |
| `alternative-unknown-part.eml` | `multipart/alternative` with one alternative Satchel cannot render; it is listed as a file |
| `mixed-text-attachment-text.eml` | Apple Mail's layout for a file placed mid-body: text, PDF, text. Both texts are shown, in order |
| `mixed-html-attachment-html.eml` | The same with HTML parts |
| `root-calendar.eml` | A `text/calendar` invitation as the whole payload, listed as `invite.ics` |
| `calendar-attachment.eml` | A cover note and an `invite.ics` attachment |
| `alternative-calendar.eml` | Text, HTML and calendar alternatives; the calendar is listed as `invite.ics` |
| `vcard-part.eml` | A cover note and a nameless `text/x-vcard` part, listed as `contact.vcf` |
| `nested-signed-behind-cover.eml` | An unsigned cover and a nested `multipart/signed`; both texts are shown and the signature has partial scope |
| `alternative-file-in-hidden-rendering.eml` | Two plain alternatives, the second with a PDF. The first is shown; the PDF is still listed |
| `nameless-pdf.eml` | A PDF without name, disposition or Content-ID, listed as an unnamed file |
| `related-nameless-image.eml` | HTML and an image without name or Content-ID, listed as an unnamed file |
| `unlabelled-binary.eml` | Binary payload with no name and no disposition, listed as an unnamed file |
| `nested-unlabelled-binary.eml` | The same below a cover note |

## Must fail with a typed error

| File | Error |
| --- | --- |
| `malformed-signed-three-parts.eml` | `MalformedSigned`: `multipart/signed` with a third part |
| `nested-malformed-signed.eml` | `MalformedSigned`, below a cover note |
| `nested-inline-envelope.eml` | `RemainingEncryption`: an envelope shown inline, below a cover note |
| `missing-multipart-boundary.eml` | `MalformedMime`: `multipart/mixed` without a boundary |

Opaque signed-data shown inline below a cover note fails with `UndecodedPart`;
the test builds that payload at run time, because it needs a real signature.

Before these cases existed, every "must fail" file decoded successfully and
returned the cover note alone, or nothing at all. Before the "must decode"
cases from `mixed-text-attachment-text.eml` on, each returned only its first
text part, lost its calendar or contact data, or was refused.

The envelope in `nested-inline-envelope.eml` and `attached-envelope.eml` is a
truncated CMS header, not a decryptable message. The decoder must classify it
from the MIME structure alone, before any key is tried.
