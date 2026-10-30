# S/MIME signed corpus

`tests/smime_signed_corpus.rs` sends these messages through
`parse_raw_message_body` and the S/MIME engine. A signature only counts if
what is displayed is the entity it verifies.

| File | Shape |
| --- | --- |
| `detached-signed.eml` | Detached `multipart/signed`, CRLF, preamble, signer certificate embedded |
| `forged-*.eml` | Structures in which the displayed part and the signed part could differ |

The signer is a throwaway self-signed certificate; only the signed message,
with the certificate inside, is checked in, never a private key. The tests
check the signature maths, not chain trust.

`python3 generate.py` rebuilds `detached-signed.eml` with a fresh certificate
(needs `openssl`). The forged messages reuse its entity and signature; they are
not generated and stay as checked in.
