-- Narrow the FTS update trigger to the indexed columns.
--
-- Flag flips (is_read, is_starred, folder moves) are by far the most common
-- message updates, and the old AFTER UPDATE trigger re-tokenized the full
-- body for every one of them — pure write amplification on each sync run.
-- External-content FTS5 reads row content on demand, so index entries stay
-- correct for updates that leave the indexed columns untouched.

DROP TRIGGER IF EXISTS messages_au;

CREATE TRIGGER messages_au
AFTER UPDATE OF subject, body_text, from_name, from_address, to_addresses ON messages
BEGIN
    INSERT INTO messages_fts(messages_fts, rowid, subject, body_text, from_name, from_address, to_addresses)
    VALUES ('delete', old.rowid, old.subject, old.body_text, old.from_name, old.from_address, old.to_addresses);
    INSERT INTO messages_fts(rowid, subject, body_text, from_name, from_address, to_addresses)
    VALUES (new.rowid, new.subject, new.body_text, new.from_name, new.from_address, new.to_addresses);
END;
