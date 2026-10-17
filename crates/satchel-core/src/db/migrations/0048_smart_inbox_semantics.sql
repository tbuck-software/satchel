-- Make Smart Inbox classification deterministic and independent of the Gmail
-- folder copy that happened to arrive first.
ALTER TABLE threads ADD COLUMN category_source TEXT NOT NULL DEFAULT 'legacy';
ALTER TABLE threads ADD COLUMN priority_source TEXT NOT NULL DEFAULT 'legacy';
ALTER TABLE messages ADD COLUMN provider_labels TEXT NOT NULL DEFAULT '[]';

CREATE TABLE IF NOT EXISTS sender_preferences (
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    sender_email TEXT NOT NULL,
    category TEXT,
    is_priority INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (account_id, sender_email)
);

CREATE INDEX IF NOT EXISTS idx_sender_preferences_category
    ON sender_preferences(account_id, category)
    WHERE category IS NOT NULL;

-- Recover the provider labels represented by existing Gmail folder copies
-- before the redundant CATEGORY_* rows are collapsed.
UPDATE messages AS target
SET provider_labels = COALESCE((
    SELECT json_group_array(DISTINCT COALESCE(NULLIF(TRIM(label_folder.remote_id), ''), label_folder.name))
    FROM messages AS sibling
    JOIN folders AS label_folder ON label_folder.id = sibling.folder_id
    WHERE sibling.account_id = target.account_id
      AND sibling.remote_id = target.remote_id
), '[]')
WHERE EXISTS (
    SELECT 1 FROM accounts
    WHERE accounts.id = target.account_id AND accounts.provider = 'gmail'
);

-- Every Gmail account needs a real local home for messages that carry no
-- primary label (INBOX/SENT/DRAFT/TRASH/SPAM). New syncs already create this
-- folder; the migration supplies it for older databases as well.
INSERT INTO folders (
    id, account_id, name, full_path, folder_type, remote_id, uidvalidity,
    message_count, unread_count, sync_enabled, sort_order, created_at, updated_at
)
SELECT
    'fld-gmail-archive-migration-' || accounts.id,
    accounts.id,
    'Archiviert',
    'Archive',
    'archive',
    'ARCHIVE',
    NULL,
    0,
    0,
    1,
    0,
    strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
    strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
FROM accounts
WHERE accounts.provider = 'gmail'
  AND NOT EXISTS (
      SELECT 1 FROM folders
      WHERE folders.account_id = accounts.id AND folders.folder_type = 'archive'
  );

-- Categories and state labels are Gmail views, not physical mailboxes. Older
-- versions stored full message copies in them. Resolve every redundant copy to
-- one canonical message before any row moves.
CREATE TEMP TABLE satchel_gmail_virtual_view_folders AS
SELECT folders.id AS folder_id
FROM folders
JOIN accounts ON accounts.id = folders.account_id
WHERE accounts.provider = 'gmail'
  AND (
    UPPER(TRIM(COALESCE(folders.remote_id, ''))) LIKE 'CATEGORY_%'
    OR UPPER(TRIM(COALESCE(folders.remote_id, ''))) IN ('IMPORTANT', 'STARRED', 'UNREAD', 'CHAT')
  );

CREATE TEMP TABLE satchel_gmail_category_duplicates AS
SELECT category_message.id AS redundant_id,
       COALESCE(
           (
               SELECT kept.id
               FROM messages AS kept
               JOIN folders AS kept_folder ON kept_folder.id = kept.folder_id
               WHERE kept.account_id = category_message.account_id
                 AND kept.remote_id = category_message.remote_id
                 AND kept.id != category_message.id
                 AND kept_folder.id NOT IN (SELECT folder_id FROM satchel_gmail_virtual_view_folders)
               ORDER BY CASE kept_folder.folder_type
                   WHEN 'inbox' THEN 0
                   WHEN 'sent' THEN 1
                   WHEN 'drafts' THEN 2
                   WHEN 'archive' THEN 3
                   ELSE 4
               END, kept.id
               LIMIT 1
           ),
           (
               SELECT MIN(sibling.id)
               FROM messages AS sibling
               JOIN folders AS sibling_folder ON sibling_folder.id = sibling.folder_id
               WHERE sibling.account_id = category_message.account_id
                 AND sibling.remote_id = category_message.remote_id
                 AND sibling_folder.id IN (SELECT folder_id FROM satchel_gmail_virtual_view_folders)
           )
       ) AS kept_id
FROM messages AS category_message
JOIN folders AS category_folder ON category_folder.id = category_message.folder_id
JOIN accounts ON accounts.id = category_message.account_id
WHERE accounts.provider = 'gmail'
  AND category_message.remote_id IS NOT NULL
  AND category_message.remote_id != ''
  AND category_folder.id IN (SELECT folder_id FROM satchel_gmail_virtual_view_folders)
  AND category_message.id != COALESCE(
      (
          SELECT kept.id
          FROM messages AS kept
          JOIN folders AS kept_folder ON kept_folder.id = kept.folder_id
          WHERE kept.account_id = category_message.account_id
            AND kept.remote_id = category_message.remote_id
            AND kept.id != category_message.id
            AND kept_folder.id NOT IN (SELECT folder_id FROM satchel_gmail_virtual_view_folders)
          ORDER BY CASE kept_folder.folder_type
              WHEN 'inbox' THEN 0
              WHEN 'sent' THEN 1
              WHEN 'drafts' THEN 2
              WHEN 'archive' THEN 3
              ELSE 4
          END, kept.id
          LIMIT 1
      ),
      (
          SELECT MIN(sibling.id)
          FROM messages AS sibling
          JOIN folders AS sibling_folder ON sibling_folder.id = sibling.folder_id
          WHERE sibling.account_id = category_message.account_id
            AND sibling.remote_id = category_message.remote_id
            AND sibling_folder.id IN (SELECT folder_id FROM satchel_gmail_virtual_view_folders)
      )
  );

-- A virtual-view-only message is archived, not deleted. Moving its canonical row
-- preserves attachments and local ids.
UPDATE messages AS category_message
SET folder_id = (
        SELECT archive_folder.id
        FROM folders AS archive_folder
        WHERE archive_folder.account_id = category_message.account_id
          AND archive_folder.folder_type = 'archive'
        ORDER BY archive_folder.updated_at DESC
        LIMIT 1
    ),
    updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
WHERE folder_id IN (
        SELECT category_folder.id
        FROM folders AS category_folder
        JOIN accounts ON accounts.id = category_folder.account_id
        WHERE accounts.provider = 'gmail'
          AND category_folder.id IN (SELECT folder_id FROM satchel_gmail_virtual_view_folders)
    )
  AND category_message.id NOT IN (
      SELECT redundant_id FROM satchel_gmail_category_duplicates
  )
  AND NOT EXISTS (
      SELECT 1
      FROM messages AS kept
      JOIN folders AS kept_folder ON kept_folder.id = kept.folder_id
      WHERE kept.account_id = category_message.account_id
        AND kept.remote_id = category_message.remote_id
        AND kept.id != category_message.id
        AND kept_folder.id NOT IN (SELECT folder_id FROM satchel_gmail_virtual_view_folders)
  );

-- Preserve objects that point at a virtual-view copy while collapsing it.
UPDATE attachments
SET message_id = (
    SELECT kept_id FROM satchel_gmail_category_duplicates
    WHERE redundant_id = attachments.message_id
)
WHERE message_id IN (SELECT redundant_id FROM satchel_gmail_category_duplicates);

UPDATE calendar_invites
SET message_id = (
    SELECT kept_id FROM satchel_gmail_category_duplicates
    WHERE redundant_id = calendar_invites.message_id
)
WHERE message_id IN (SELECT redundant_id FROM satchel_gmail_category_duplicates);

UPDATE follow_up_reminders
SET message_id = (
    SELECT kept_id FROM satchel_gmail_category_duplicates
    WHERE redundant_id = follow_up_reminders.message_id
)
WHERE message_id IN (SELECT redundant_id FROM satchel_gmail_category_duplicates);

DELETE FROM messages
WHERE id IN (SELECT redundant_id FROM satchel_gmail_category_duplicates);

DROP TABLE satchel_gmail_category_duplicates;
DROP TABLE satchel_gmail_virtual_view_folders;

-- Explicit Gmail categories outrank the old address/header heuristic. The
-- provider label is now attached to every surviving copy of the message.
UPDATE messages
SET category = CASE
        WHEN EXISTS (SELECT 1 FROM json_each(provider_labels) WHERE UPPER(TRIM(value)) = 'CATEGORY_UPDATES') THEN 'updates'
        WHEN EXISTS (SELECT 1 FROM json_each(provider_labels) WHERE UPPER(TRIM(value)) = 'CATEGORY_FORUMS') THEN 'updates'
        WHEN EXISTS (SELECT 1 FROM json_each(provider_labels) WHERE UPPER(TRIM(value)) = 'CATEGORY_PROMOTIONS') THEN 'promotions'
        WHEN EXISTS (SELECT 1 FROM json_each(provider_labels) WHERE UPPER(TRIM(value)) = 'CATEGORY_SOCIAL') THEN 'social'
        WHEN EXISTS (SELECT 1 FROM json_each(provider_labels) WHERE UPPER(TRIM(value)) = 'CATEGORY_PERSONAL') THEN 'personal'
        ELSE category
    END
WHERE EXISTS (
    SELECT 1 FROM accounts
    WHERE accounts.id = messages.account_id AND accounts.provider = 'gmail'
);

-- Recover Gmail's IMPORTANT signal where it is still present. Existing high
-- priorities without that provider label are conservatively treated as an old
-- manual/rule choice so the migration never demotes a user decision.
UPDATE threads
SET priority = CASE
        WHEN priority >= 2 THEN priority
        WHEN EXISTS (
            SELECT 1
            FROM messages AS latest, json_each(latest.provider_labels)
            WHERE latest.thread_id = threads.id
              AND latest.id = (
                  SELECT newest.id FROM messages AS newest
                  WHERE newest.thread_id = threads.id
                  ORDER BY newest.is_draft ASC, newest.date DESC, newest.id DESC
                  LIMIT 1
              )
              AND UPPER(TRIM(json_each.value)) = 'IMPORTANT'
        ) THEN 2
        ELSE priority
    END,
    priority_source = CASE
        WHEN EXISTS (
            SELECT 1
            FROM messages AS latest, json_each(latest.provider_labels)
            WHERE latest.thread_id = threads.id
              AND latest.id = (
                  SELECT newest.id FROM messages AS newest
                  WHERE newest.thread_id = threads.id
                  ORDER BY newest.is_draft ASC, newest.date DESC, newest.id DESC
                  LIMIT 1
              )
              AND UPPER(TRIM(json_each.value)) = 'IMPORTANT'
        ) THEN 'provider'
        WHEN priority >= 2 THEN 'manual_thread'
        ELSE 'heuristic'
    END
WHERE EXISTS (
    SELECT 1 FROM accounts
    WHERE accounts.id = threads.account_id AND accounts.provider = 'gmail'
);

-- Project the latest non-draft message classification to the thread. Existing
-- values are legacy automatic values; future manual choices carry an explicit
-- source and are protected by the ingest path.
UPDATE threads
SET category = COALESCE((
        SELECT latest.category
        FROM messages AS latest
        WHERE latest.thread_id = threads.id
        ORDER BY latest.is_draft ASC, latest.date DESC, latest.id DESC
        LIMIT 1
    ), category, 'personal'),
    category_source = CASE
        WHEN EXISTS (
            SELECT 1
            FROM messages AS latest, json_each(latest.provider_labels)
            WHERE latest.thread_id = threads.id
              AND latest.id = (
                  SELECT newest.id FROM messages AS newest
                  WHERE newest.thread_id = threads.id
                  ORDER BY newest.is_draft ASC, newest.date DESC, newest.id DESC
                  LIMIT 1
              )
              AND UPPER(TRIM(json_each.value)) LIKE 'CATEGORY_%'
        ) THEN 'provider'
        ELSE 'heuristic'
    END;

-- Category copies used to inflate projections. Rebuild each thread from the
-- surviving canonical rows using the same distinct identity as migration 0046.
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

-- Satchel's "done" transition archives remotely. If Gmail still says INBOX
-- and no mutation is queued, the old local state is stale and was the reason
-- Spark showed conversations that Satchel hid.
UPDATE threads
SET workflow_state = 'inbox',
    board_column = 'inbox',
    snooze_until = NULL,
    workflow_changed_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
    updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
WHERE workflow_state = 'done'
  AND EXISTS (
      SELECT 1 FROM accounts
      WHERE accounts.id = threads.account_id AND accounts.provider = 'gmail'
  )
  AND EXISTS (
      SELECT 1
      FROM messages
      JOIN folders ON folders.id = messages.folder_id
      WHERE messages.thread_id = threads.id
        AND LOWER(TRIM(COALESCE(folders.remote_id, folders.name))) = 'inbox'
  )
  AND NOT EXISTS (
      SELECT 1 FROM sync_queue
      WHERE sync_queue.account_id = threads.account_id
        AND sync_queue.action_type = 'remote_mutation'
        AND sync_queue.status IN ('pending', 'sending', 'scheduled')
  );
