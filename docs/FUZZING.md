# Parser fuzzing

The isolated `fuzz/` package exercises HTML sanitization, MIME body parsing,
ICS invitations and the streaming mbox import reader. It depends on Core,
without compiling Tauri. The mbox reader is also used by the app importer.

Install nightly separately from the pinned application toolchain:

```sh
rustup toolchain install nightly --profile minimal
cargo +nightly install cargo-fuzz --locked
node scripts/seed-fuzz-corpus.mjs
cargo +nightly fuzz run sanitizer -- -max_total_time=300 -max_len=262144
```

Repeat for `mime_body`, `ics` and `mbox_import`. The nightly Linux CI job runs
all four for five minutes each and uploads crash reproducers. Seeds come from
`src-tauri/tests/corpus`, sanitizer regression payloads and a minimal ICS invite.
CI retains evolved corpora between runs. It creates an empty optional `.env`
when absent so Core does not rebuild between targets; parser jobs need no OAuth
configuration. Corpus and artifact directories are
ignored locally. Replay a crash with `cargo +nightly fuzz run <target> <file>`.

The sanitizer target reparses output and rejects executable elements, event
attributes, JavaScript URL attributes and style text that can reopen markup.
The other parsers must handle arbitrary bytes without panicking. mbox also
checks that the reader consumes the entire input. Input length and per-input
execution time are bounded in CI.

`cargo test -p satchel-core mail_state_sequences_match_model` runs 64 generated
queue operation sequences on the pinned toolchain. It checks claims, retries,
failures, submission, restart recovery, delivered mail, discard and repeated
enqueue against an independent state model after every operation. Proptest
prints a reproducible seed and shrinks failing sequences.
