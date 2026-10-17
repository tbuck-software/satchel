-- Sender preferences were written per account and only ever applied to threads
-- of that account. Prioritising someone in one mailbox therefore did nothing
-- when the same person wrote to another. Reads and back-applies now span all
-- accounts; this catches up the threads that were missed meanwhile.

CREATE INDEX IF NOT EXISTS idx_sender_preferences_email
    ON sender_preferences(sender_email);

UPDATE threads
SET priority = 2,
    priority_source = 'manual_sender',
    updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
WHERE priority_source NOT IN ('manual_thread', 'manual_sender')
  AND EXISTS (
    SELECT 1 FROM sender_preferences
    WHERE sender_preferences.is_priority > 0
      AND sender_preferences.sender_email = LOWER(TRIM(COALESCE((
        SELECT messages.from_address
        FROM messages
        WHERE messages.thread_id = threads.id
        ORDER BY messages.is_draft ASC, messages.date DESC, messages.id DESC
        LIMIT 1
      ), '')))
  );

UPDATE threads
SET category = (
      SELECT sender_preferences.category
      FROM sender_preferences
      WHERE sender_preferences.category IS NOT NULL
        AND sender_preferences.sender_email = LOWER(TRIM(COALESCE((
          SELECT messages.from_address
          FROM messages
          WHERE messages.thread_id = threads.id
          ORDER BY messages.is_draft ASC, messages.date DESC, messages.id DESC
          LIMIT 1
        ), '')))
      ORDER BY sender_preferences.updated_at DESC
      LIMIT 1
    ),
    category_source = 'manual_sender',
    updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
WHERE category_source NOT IN ('manual_thread', 'manual_sender')
  AND EXISTS (
    SELECT 1 FROM sender_preferences
    WHERE sender_preferences.category IS NOT NULL
      AND sender_preferences.sender_email = LOWER(TRIM(COALESCE((
        SELECT messages.from_address
        FROM messages
        WHERE messages.thread_id = threads.id
        ORDER BY messages.is_draft ASC, messages.date DESC, messages.id DESC
        LIMIT 1
      ), '')))
  );
