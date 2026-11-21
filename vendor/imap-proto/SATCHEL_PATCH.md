# Satchel imap-proto patch

This is the crates.io release of `imap-proto` 0.16.6 (archive SHA-256
`ba1f9b30846c3d04371159ef3a0413ce7c1ae0a8c619cd255c60b3d902553f22`).
The licenses come from the upstream release commit
`55e6c27189c7086a5726fe8bf224ba5c24d19d0c`.

The CAPABILITY parser accepts an exact, case-insensitive `IMAP4rev2` atom
as an alternative to `IMAP4rev1`. This lets async-imap decode capability
lists from rev2-only servers. Satchel still requires a successful CAPABILITY
command and an explicit `UIDPLUS` or `IMAP4rev2` advertisement before marking
messages deleted for UID EXPUNGE.

Remove the patch when the upstream parser accepts CAPABILITY replies from
rev2-only servers. The `copy_move_uses_uid_expunge_on_a_rev2_only_server`
test checks this through the IMAP session.
