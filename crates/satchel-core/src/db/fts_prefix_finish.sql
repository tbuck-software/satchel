DROP TRIGGER messages_ai;
DROP TRIGGER messages_ad;
DROP TRIGGER messages_au;
DROP TRIGGER messages_prefix_ai;
DROP TRIGGER messages_prefix_ad;
DROP TRIGGER messages_prefix_au;
DROP TABLE messages_fts;
ALTER TABLE messages_prefix_fts RENAME TO messages_fts;
DROP TABLE message_prefix_indexed;
CREATE TRIGGER messages_ai AFTER INSERT ON messages BEGIN
    INSERT INTO messages_fts(rowid, subject, body_text, from_name, from_address, to_addresses) VALUES (new.rowid, new.subject, new.body_text, new.from_name, new.from_address, new.to_addresses);
END;
CREATE TRIGGER messages_ad AFTER DELETE ON messages BEGIN
    INSERT INTO messages_fts(messages_fts, rowid, subject, body_text, from_name, from_address, to_addresses) VALUES ('delete', old.rowid, old.subject, old.body_text, old.from_name, old.from_address, old.to_addresses);
END;
CREATE TRIGGER messages_au
AFTER UPDATE OF subject, body_text, from_name, from_address, to_addresses ON messages BEGIN
    INSERT INTO messages_fts(messages_fts, rowid, subject, body_text, from_name, from_address, to_addresses) VALUES ('delete', old.rowid, old.subject, old.body_text, old.from_name, old.from_address, old.to_addresses);
    INSERT INTO messages_fts(rowid, subject, body_text, from_name, from_address, to_addresses) VALUES (new.rowid, new.subject, new.body_text, new.from_name, new.from_address, new.to_addresses);
END;
