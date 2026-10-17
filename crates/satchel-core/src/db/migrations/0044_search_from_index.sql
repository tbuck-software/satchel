-- The sidebar's contact search issues `from:<full address>` queries. Those
-- carry no FTS term, so the operator branch scanned the whole messages table
-- with LIKE '%…%' — reading the wide body columns row by row. Full addresses
-- now match exactly (case-insensitive) against this index instead.
CREATE INDEX IF NOT EXISTS idx_messages_from_address_nocase
    ON messages(from_address COLLATE NOCASE);
