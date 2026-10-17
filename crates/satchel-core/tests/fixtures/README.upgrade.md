# Older database upgrade fixture

`upgrade-v22.db` is a frozen SQLite database generated from migrations 0001–0022
at commit `eed95b44`. The generator verified those SQL bytes against the current
copies and stored their SHA-384 checksums in the SQLx ledger. It contains one
synthetic account, inbox, thread and message, with no credentials or real mail.
SQLite integrity and search-index checks pass before the upgrade.

The test copies the file to a temporary directory. It adds representative
retired migration ledger entries 0023–0026, upgrades through the production
initializer, compares the complete schema to `schema.snapshot.sql`, and checks
mail content and search integrity. Another test corrupts an applied checksum
and verifies that startup still fails.

Keep this fixture frozen when adding migrations. Update the head schema
snapshot for intentional schema changes; the older fixture must still upgrade.
