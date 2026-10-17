INSERT OR IGNORE INTO message_addresses(message_id, thread_id, email)
SELECT m.id, m.thread_id, LOWER(TRIM(address))
FROM messages m JOIN (
    SELECT id, from_address AS address FROM messages WHERE messages.rowid > ?1 AND messages.rowid <= ?2
    UNION ALL SELECT messages.id, j.value FROM messages, json_each(COALESCE(to_addresses, '[]')) j WHERE messages.rowid > ?1 AND messages.rowid <= ?2
    UNION ALL SELECT messages.id, j.value FROM messages, json_each(COALESCE(cc_addresses, '[]')) j WHERE messages.rowid > ?1 AND messages.rowid <= ?2
    UNION ALL SELECT messages.id, j.value FROM messages, json_each(COALESCE(bcc_addresses, '[]')) j WHERE messages.rowid > ?1 AND messages.rowid <= ?2
) a ON a.id = m.id WHERE TRIM(address) <> '' ;
