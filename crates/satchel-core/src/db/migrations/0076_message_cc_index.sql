-- `cc:` searched with LIKE over the JSON text, so `cc:ann` also hit
-- `joann@…`. `to:` uses the `to_addresses` column of messages_fts; Cc gets
-- its own token index. Contentless with delete support: a DELETE of a rowid
-- the backfill has not reached yet is a no-op instead of corrupting the index.
CREATE VIRTUAL TABLE message_cc_fts USING fts5(
    cc_addresses,
    content = '',
    contentless_delete = 1,
    tokenize = 'unicode61 remove_diacritics 2'
);

CREATE TRIGGER message_cc_fts_ai AFTER INSERT ON messages BEGIN
    INSERT INTO message_cc_fts (rowid, cc_addresses) VALUES (new.rowid, new.cc_addresses);
END;

CREATE TRIGGER message_cc_fts_ad AFTER DELETE ON messages BEGIN
    DELETE FROM message_cc_fts WHERE rowid = old.rowid;
END;

CREATE TRIGGER message_cc_fts_au AFTER UPDATE OF cc_addresses ON messages BEGIN
    DELETE FROM message_cc_fts WHERE rowid = old.rowid;
    INSERT INTO message_cc_fts (rowid, cc_addresses) VALUES (new.rowid, new.cc_addresses);
END;

-- Existing rows are indexed after startup by the `cc_index` data repair.
INSERT OR IGNORE INTO app_settings (key, value, updated_at)
VALUES ('data_repair.cc_index', '', strftime('%Y-%m-%dT%H:%M:%SZ', 'now'));
