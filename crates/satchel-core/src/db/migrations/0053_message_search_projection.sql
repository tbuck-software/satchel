-- Keep the columns needed to rank free-text search results away from the
-- multi-gigabyte messages table. FTS returns message rowids; resolving every
-- match against this compact projection avoids random reads through stored
-- bodies before the final result page is known.
CREATE TABLE message_search_projection (
    message_rowid INTEGER PRIMARY KEY,
    message_id TEXT NOT NULL,
    -- Draft imports and maintenance fixtures may be unthreaded temporarily.
    thread_id TEXT,
    account_id TEXT NOT NULL,
    date TEXT NOT NULL
);

INSERT INTO message_search_projection (message_rowid, message_id, thread_id, account_id, date)
SELECT rowid, id, thread_id, account_id, COALESCE(date, '')
FROM messages;

CREATE TRIGGER message_search_projection_ai AFTER INSERT ON messages BEGIN
    INSERT INTO message_search_projection (message_rowid, message_id, thread_id, account_id, date)
    VALUES (new.rowid, new.id, new.thread_id, new.account_id, COALESCE(new.date, ''));
END;

CREATE TRIGGER message_search_projection_ad AFTER DELETE ON messages BEGIN
    DELETE FROM message_search_projection WHERE message_rowid = old.rowid;
END;

CREATE TRIGGER message_search_projection_au
AFTER UPDATE OF id, thread_id, account_id, date ON messages BEGIN
    INSERT INTO message_search_projection (message_rowid, message_id, thread_id, account_id, date)
    VALUES (new.rowid, new.id, new.thread_id, new.account_id, COALESCE(new.date, ''))
    ON CONFLICT(message_rowid) DO UPDATE SET
        message_id = excluded.message_id,
        thread_id = excluded.thread_id,
        account_id = excluded.account_id,
        date = excluded.date;
END;
