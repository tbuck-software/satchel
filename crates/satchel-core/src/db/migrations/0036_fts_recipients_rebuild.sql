-- Rebuild the message search index to also cover recipient addresses.
--
-- External-content FTS5 tables cannot ALTER their column set, so the table
-- and its sync triggers are dropped and recreated with `to_addresses` added.
-- The final 'rebuild' command streams every stored message through the
-- tokenizer once — a one-time cost on first startup after this migration
-- (seconds to a few minutes on large mailboxes; the WAL temporarily grows by
-- roughly the index size and is truncated by the hourly maintenance
-- checkpoint).
--
-- `to_addresses` is stored as JSON array text; unicode61 treats the JSON
-- punctuation ('[', '"', ',', '@', '.') as separators, so addresses tokenize
-- into usable terms without preprocessing.

DROP TRIGGER IF EXISTS messages_ai;
DROP TRIGGER IF EXISTS messages_ad;
DROP TRIGGER IF EXISTS messages_au;
DROP TABLE IF EXISTS messages_fts;

CREATE VIRTUAL TABLE messages_fts USING fts5(
    subject,
    body_text,
    from_name,
    from_address,
    to_addresses,
    content='messages',
    content_rowid='rowid',
    tokenize='unicode61 remove_diacritics 2'
);

CREATE TRIGGER messages_ai AFTER INSERT ON messages BEGIN
    INSERT INTO messages_fts(rowid, subject, body_text, from_name, from_address, to_addresses)
    VALUES (new.rowid, new.subject, new.body_text, new.from_name, new.from_address, new.to_addresses);
END;

CREATE TRIGGER messages_ad AFTER DELETE ON messages BEGIN
    INSERT INTO messages_fts(messages_fts, rowid, subject, body_text, from_name, from_address, to_addresses)
    VALUES ('delete', old.rowid, old.subject, old.body_text, old.from_name, old.from_address, old.to_addresses);
END;

CREATE TRIGGER messages_au AFTER UPDATE ON messages BEGIN
    INSERT INTO messages_fts(messages_fts, rowid, subject, body_text, from_name, from_address, to_addresses)
    VALUES ('delete', old.rowid, old.subject, old.body_text, old.from_name, old.from_address, old.to_addresses);
    INSERT INTO messages_fts(rowid, subject, body_text, from_name, from_address, to_addresses)
    VALUES (new.rowid, new.subject, new.body_text, new.from_name, new.from_address, new.to_addresses);
END;

INSERT INTO messages_fts(messages_fts) VALUES('rebuild');
