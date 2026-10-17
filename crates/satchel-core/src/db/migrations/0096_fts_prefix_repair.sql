-- Build beside the live index; the repair switches tables after the last block.
CREATE VIRTUAL TABLE messages_prefix_fts USING fts5(
    subject, body_text, from_name, from_address, to_addresses,
    content='messages', content_rowid='rowid',
    tokenize='unicode61 remove_diacritics 2', prefix='2 3'
);
CREATE TABLE message_prefix_indexed (message_rowid INTEGER PRIMARY KEY);
-- Explicit conflict clauses also work inside an outer message UPSERT.
CREATE TRIGGER messages_prefix_ai AFTER INSERT ON messages BEGIN
    INSERT INTO messages_prefix_fts(rowid, subject, body_text, from_name, from_address, to_addresses) VALUES (new.rowid, new.subject, new.body_text, new.from_name, new.from_address, new.to_addresses);
    INSERT INTO message_prefix_indexed VALUES (new.rowid) ON CONFLICT(message_rowid) DO NOTHING;
END;
CREATE TRIGGER messages_prefix_ad AFTER DELETE ON messages BEGIN
    INSERT INTO messages_prefix_fts(messages_prefix_fts, rowid, subject, body_text, from_name, from_address, to_addresses)
    SELECT 'delete', old.rowid, old.subject, old.body_text, old.from_name, old.from_address, old.to_addresses WHERE EXISTS (
        SELECT 1 FROM message_prefix_indexed WHERE message_rowid = old.rowid);
    DELETE FROM message_prefix_indexed WHERE message_rowid = old.rowid;
END;
CREATE TRIGGER messages_prefix_au
AFTER UPDATE OF subject, body_text, from_name, from_address, to_addresses ON messages BEGIN
    INSERT INTO messages_prefix_fts(messages_prefix_fts, rowid, subject, body_text, from_name, from_address, to_addresses)
    SELECT 'delete', old.rowid, old.subject, old.body_text, old.from_name, old.from_address, old.to_addresses WHERE EXISTS (
        SELECT 1 FROM message_prefix_indexed WHERE message_rowid = old.rowid);
    INSERT INTO messages_prefix_fts(rowid, subject, body_text, from_name, from_address, to_addresses) VALUES (new.rowid, new.subject, new.body_text, new.from_name, new.from_address, new.to_addresses);
    INSERT INTO message_prefix_indexed VALUES (new.rowid) ON CONFLICT(message_rowid) DO NOTHING;
END;
INSERT OR IGNORE INTO app_settings(key, value, updated_at)
VALUES ('data_repair.fts_prefix', '', strftime('%Y-%m-%dT%H:%M:%SZ', 'now'));
