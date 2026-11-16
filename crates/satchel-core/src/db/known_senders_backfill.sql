-- Same address extraction as the `known_senders_sent_*` triggers in 0127.
INSERT INTO known_senders (email, source, account_id, updated_at)
SELECT address, 'sent:' || account_id, account_id, strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
FROM (
    SELECT account_id, LOWER(TRIM(CASE
        WHEN INSTR(value, '<') > 0 AND INSTR(value, '>') > INSTR(value, '<')
        THEN SUBSTR(value, INSTR(value, '<') + 1, INSTR(value, '>') - INSTR(value, '<') - 1)
        ELSE value
    END)) AS address
    FROM (
        SELECT m.account_id, j.value FROM messages m, json_each(COALESCE(m.to_addresses, '[]')) j
        WHERE m.rowid > ?1 AND m.rowid <= ?2 AND m.is_sent = 1 AND m.is_draft = 0
        UNION ALL
        SELECT m.account_id, j.value FROM messages m, json_each(COALESCE(m.cc_addresses, '[]')) j
        WHERE m.rowid > ?1 AND m.rowid <= ?2 AND m.is_sent = 1 AND m.is_draft = 0
        UNION ALL
        SELECT m.account_id, j.value FROM messages m, json_each(COALESCE(m.bcc_addresses, '[]')) j
        WHERE m.rowid > ?1 AND m.rowid <= ?2 AND m.is_sent = 1 AND m.is_draft = 0
    )
)
WHERE INSTR(address, '@') > 1
-- An account deleted since: its messages are gone with it, but be sure.
  AND EXISTS (SELECT 1 FROM accounts WHERE accounts.id = account_id)
ON CONFLICT (email, source) DO NOTHING;
