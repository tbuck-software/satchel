# S/MIME corpus

Each file is the plaintext a decrypted S/MIME envelope carries: one MIME
entity, CRLF line endings, synthetic content. The tests in
`src/utils/smime_engine.rs` send every file through the outbox's
sign-then-encrypt transform and back through the read path
(`parse_raw_message_body` → `decrypt` → `parse_decrypted_body`), so a case
here exercises the production round trip, not just the parser.

No key material is checked in. The tests issue their certificates at run time.

## Must decode

| File | Holds |
| --- | --- |
| `root-attachment.eml` | A binary attachment as the whole payload; bytes, disposition and Content-ID survive |
| `empty-text.eml` | An empty `text/plain` body |
| `image-only-html.eml` | HTML that is a single `cid:` image |
| `attached-envelope.eml` | A cover note and an encrypted mail attached as a file |
| `related-inline-image.eml` | HTML with an inline image that has a Content-ID but no filename |
| `alternative-unknown-part.eml` | `multipart/alternative` with one alternative Satchel cannot render |

## Must fail with a typed error

| File | Error |
| --- | --- |
| `malformed-signed-three-parts.eml` | `MalformedSigned`: `multipart/signed` with a third part |
| `nested-malformed-signed.eml` | `MalformedSigned`, below a cover note |
| `nested-inline-envelope.eml` | `RemainingEncryption`: an envelope shown inline, below a cover note |
| `missing-multipart-boundary.eml` | `MalformedMime`: `multipart/mixed` without a boundary |
| `unlabelled-binary.eml` | `UndecodedPart`: binary payload with no name and no disposition |
| `nested-unlabelled-binary.eml` | `UndecodedPart`, below a cover note |

Before these cases existed, every "must fail" file decoded successfully and
returned the cover note alone, or nothing at all.

The envelope in `nested-inline-envelope.eml` and `attached-envelope.eml` is a
truncated CMS header, not a decryptable message. The decoder must classify it
from the MIME structure alone, before any key is tried.
