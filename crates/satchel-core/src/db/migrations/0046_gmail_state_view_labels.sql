-- Drop the Gmail labels that only restate a message's own state.
--
-- UNREAD, STARRED, IMPORTANT and CHAT are not places mail lives; read and
-- starred are already columns on the message. Because `provider_message_db_id`
-- scopes a row id by its folder, syncing them stored a second full copy of the
-- message body per label — on a real mailbox that is tens of thousands of rows
-- and gigabytes of duplicated HTML. `gmail_label_is_state_view` stops the sync
-- from recreating them; this removes what is already stored.
--
-- Deliberately conservative: a row is only deleted when the SAME message is
-- provably present in another folder of the same account. Should a message ever
-- exist solely under a state label, it stays rather than silently disappearing
-- from the mailbox.
--
-- CATEGORY_* is intentionally NOT touched: archived mail loses INBOX but keeps
-- its category, so for that mail the category label is the only place it lives.

CREATE TEMP TABLE satchel_gmail_state_view_folders AS
SELECT folders.id AS folder_id, folders.account_id AS account_id
FROM folders
JOIN accounts ON accounts.id = folders.account_id
WHERE accounts.provider = 'gmail'
  AND UPPER(TRIM(COALESCE(folders.remote_id, ''))) IN ('UNREAD', 'STARRED', 'IMPORTANT', 'CHAT');

-- Message rows that are safe to drop: duplicated somewhere outside these folders.
--
-- The `kept` lookup compares `remote_id` raw rather than TRIM()ed on purpose.
-- `idx_messages_account_remote` covers (account_id, remote_id), and wrapping the
-- column in a function makes it unusable — turning this into a full scan of the
-- messages table per candidate row, which on a real mailbox runs for minutes at
-- startup. Gmail ids never carry whitespace, and both sides are written by the
-- same code path.
CREATE TEMP TABLE satchel_gmail_redundant_messages AS
SELECT messages.id AS message_id
FROM messages
JOIN satchel_gmail_state_view_folders ON satchel_gmail_state_view_folders.folder_id = messages.folder_id
WHERE messages.remote_id IS NOT NULL
  AND messages.remote_id != ''
  AND EXISTS (
      SELECT 1
      FROM messages AS kept
      WHERE kept.account_id = messages.account_id
        AND kept.remote_id = messages.remote_id
        AND kept.folder_id != messages.folder_id
        AND kept.folder_id NOT IN (SELECT folder_id FROM satchel_gmail_state_view_folders)
  );

DELETE FROM attachments
WHERE message_id IN (SELECT message_id FROM satchel_gmail_redundant_messages);

DELETE FROM messages
WHERE id IN (SELECT message_id FROM satchel_gmail_redundant_messages);

-- Only remove folders that are now empty; anything left held a message that
-- existed nowhere else and must stay reachable.
DELETE FROM folders
WHERE id IN (
    SELECT folder_id FROM satchel_gmail_state_view_folders
    WHERE folder_id NOT IN (SELECT folder_id FROM messages WHERE folder_id IS NOT NULL)
);

DROP TABLE satchel_gmail_redundant_messages;
DROP TABLE satchel_gmail_state_view_folders;

-- Thread counters counted the duplicates. Recompute them the way 0031 does:
-- distinct per message identity, not per stored row.
UPDATE threads
SET message_count = (
        SELECT COUNT(DISTINCT COALESCE(
            NULLIF(TRIM(messages.message_id_header), ''),
            NULLIF(TRIM(messages.remote_id), ''),
            messages.id
        ))
        FROM messages
        WHERE messages.thread_id = threads.id
    ),
    unread_count = (
        SELECT COUNT(DISTINCT COALESCE(
            NULLIF(TRIM(messages.message_id_header), ''),
            NULLIF(TRIM(messages.remote_id), ''),
            messages.id
        ))
        FROM messages
        WHERE messages.thread_id = threads.id AND messages.is_read = 0
    ),
    is_starred = COALESCE((
        SELECT MAX(messages.is_starred)
        FROM messages
        WHERE messages.thread_id = threads.id
    ), 0)
WHERE EXISTS (
    SELECT 1 FROM accounts
    WHERE accounts.id = threads.account_id AND accounts.provider = 'gmail'
);
