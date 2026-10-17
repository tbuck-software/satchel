CREATE TABLE message_addresses (
    message_id TEXT NOT NULL,
    thread_id TEXT,
    email TEXT NOT NULL,
    PRIMARY KEY (message_id, email)
) WITHOUT ROWID;
CREATE INDEX idx_message_addresses_email_thread ON message_addresses(email, thread_id, message_id);
CREATE TRIGGER message_addresses_ai AFTER INSERT ON messages BEGIN
    INSERT INTO message_addresses(message_id, thread_id, email)
SELECT m.id, m.thread_id, LOWER(TRIM(address))
FROM messages m JOIN (
    SELECT id, from_address AS address FROM messages WHERE messages.id = new.id
    UNION ALL SELECT messages.id, j.value FROM messages, json_each(COALESCE(to_addresses, '[]')) j WHERE messages.id = new.id
    UNION ALL SELECT messages.id, j.value FROM messages, json_each(COALESCE(cc_addresses, '[]')) j WHERE messages.id = new.id
    UNION ALL SELECT messages.id, j.value FROM messages, json_each(COALESCE(bcc_addresses, '[]')) j WHERE messages.id = new.id
) a ON a.id = m.id WHERE TRIM(address) <> ''
    ON CONFLICT(message_id, email) DO NOTHING;
END;
CREATE TRIGGER message_addresses_au AFTER UPDATE OF id, thread_id, from_address, to_addresses, cc_addresses, bcc_addresses ON messages BEGIN
    DELETE FROM message_addresses WHERE message_id = old.id;
    INSERT INTO message_addresses(message_id, thread_id, email)
SELECT m.id, m.thread_id, LOWER(TRIM(address))
FROM messages m JOIN (
    SELECT id, from_address AS address FROM messages WHERE messages.id = new.id
    UNION ALL SELECT messages.id, j.value FROM messages, json_each(COALESCE(to_addresses, '[]')) j WHERE messages.id = new.id
    UNION ALL SELECT messages.id, j.value FROM messages, json_each(COALESCE(cc_addresses, '[]')) j WHERE messages.id = new.id
    UNION ALL SELECT messages.id, j.value FROM messages, json_each(COALESCE(bcc_addresses, '[]')) j WHERE messages.id = new.id
) a ON a.id = m.id WHERE TRIM(address) <> ''
    ON CONFLICT(message_id, email) DO NOTHING;
END;
CREATE TRIGGER message_addresses_ad AFTER DELETE ON messages BEGIN
    DELETE FROM message_addresses WHERE message_id = old.id;
END;
INSERT OR IGNORE INTO app_settings(key, value, updated_at)
VALUES ('data_repair.message_addresses', '', strftime('%Y-%m-%dT%H:%M:%SZ', 'now'));
