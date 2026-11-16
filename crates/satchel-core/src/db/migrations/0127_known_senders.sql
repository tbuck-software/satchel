-- Addresses that belong to people the user knows: everyone an account has
-- written to (`sent:<account>`) and everyone in a synced address book
-- (`macos`, `gmail:<account>`, `outlook:<account>`, `carddav:<account>`). The
-- smart Inbox and the new-sender review read it by exact address; a display
-- name never makes a sender known. Rows that came with an account go with it.
CREATE TABLE known_senders (
    email TEXT NOT NULL,
    source TEXT NOT NULL,
    account_id TEXT REFERENCES accounts(id) ON DELETE CASCADE,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (email, source)
) WITHOUT ROWID;

CREATE INDEX idx_known_senders_account ON known_senders(account_id);

-- Every recipient of a sent message. Rows stay when the message goes: having
-- written to someone once is the signal, not the message itself.
CREATE TRIGGER known_senders_sent_ai AFTER INSERT ON messages
WHEN new.is_sent = 1 AND new.is_draft = 0 BEGIN
    INSERT INTO known_senders (email, source, account_id, updated_at)
    SELECT address, 'sent:' || new.account_id, new.account_id, strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
    FROM (
        SELECT LOWER(TRIM(CASE
            WHEN INSTR(value, '<') > 0 AND INSTR(value, '>') > INSTR(value, '<')
            THEN SUBSTR(value, INSTR(value, '<') + 1, INSTR(value, '>') - INSTR(value, '<') - 1)
            ELSE value
        END)) AS address
        FROM (
            SELECT value FROM json_each(COALESCE(new.to_addresses, '[]'))
            UNION ALL SELECT value FROM json_each(COALESCE(new.cc_addresses, '[]'))
            UNION ALL SELECT value FROM json_each(COALESCE(new.bcc_addresses, '[]'))
        )
    )
    WHERE INSTR(address, '@') > 1
    ON CONFLICT (email, source) DO NOTHING;
END;

CREATE TRIGGER known_senders_sent_au
AFTER UPDATE OF is_sent, is_draft, to_addresses, cc_addresses, bcc_addresses ON messages
WHEN new.is_sent = 1 AND new.is_draft = 0 BEGIN
    INSERT INTO known_senders (email, source, account_id, updated_at)
    SELECT address, 'sent:' || new.account_id, new.account_id, strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
    FROM (
        SELECT LOWER(TRIM(CASE
            WHEN INSTR(value, '<') > 0 AND INSTR(value, '>') > INSTR(value, '<')
            THEN SUBSTR(value, INSTR(value, '<') + 1, INSTR(value, '>') - INSTR(value, '<') - 1)
            ELSE value
        END)) AS address
        FROM (
            SELECT value FROM json_each(COALESCE(new.to_addresses, '[]'))
            UNION ALL SELECT value FROM json_each(COALESCE(new.cc_addresses, '[]'))
            UNION ALL SELECT value FROM json_each(COALESCE(new.bcc_addresses, '[]'))
        )
    )
    WHERE INSTR(address, '@') > 1
    ON CONFLICT (email, source) DO NOTHING;
END;

-- Recipients of mail sent before this migration, in blocks after startup
-- (`db::known_senders::KNOWN_SENDERS_SENT`).
INSERT OR IGNORE INTO app_settings (key, value, updated_at)
VALUES ('data_repair.known_senders_sent', '', strftime('%Y-%m-%dT%H:%M:%SZ', 'now'));
