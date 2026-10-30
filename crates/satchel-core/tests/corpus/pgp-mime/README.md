# PGP/MIME signed corpus

Signed messages shaped after real clients' output and signed with the
test-only GnuPG fixture key in `../pgp` (its passphrase is in that README).
`tests/pgp_mime_corpus.rs` sends each one through `parse_raw_message_body`,
the path incoming mail takes, and verifies the kept bundle with the fixture
key. A signature only counts if what is displayed is the entity it verifies.

| File | Shape |
| --- | --- |
| `thunderbird-mixed-attachment.eml` | Thunderbird: preamble, protected headers, nested `multipart/mixed` with an attachment |
| `apple-mail-gpgtools.eml` | GPGTools in Apple Mail: folded parameters, empty preamble, signature type after other headers |
| `mutt-lf.eml`, `mutt-crlf.eml` | Mutt: text-mode signature, stored with LF and as sent with CRLF |
| `preamble-and-epilogue.eml` | Both optional areas filled, SHA-512, no empty line before the close delimiter |
| `nested-prefix-boundary.eml` | The inner boundary extends the outer one (`=_b_1`, `=_b_12`) |
| `forged-*.eml` | Structures in which the displayed part and the signed part could differ |
| `forged-unrecognised-protocol.eml`, `forged-missing-protocol.eml` | A root without a known protocol around a genuinely signed part |
| `forged-signature-part-attachment.eml` | A genuine signature whose part names itself an attachment |
| `forged-duplicate-signature-type.eml` | A signature part that names its type twice |

The forged messages reuse a genuine entity and signature from the fixtures
above. They must never show a valid signature over what they display.

`python3 generate.py` rebuilds the legitimate fixtures; it needs `gpg` and,
with libgcrypt 1.12.0, the scoped newer library described in `../pgp`.
Signatures carry their creation time, so a rerun changes the armor only. The
forged fixtures are not generated and stay as checked in.
