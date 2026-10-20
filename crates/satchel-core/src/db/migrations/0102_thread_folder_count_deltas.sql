-- The thread_folders triggers from 0095 recounted the whole thread for every
-- message row they saw, unchanged rows included: marking a 3,000-message
-- thread read rescanned the thread 3,000 times inside one write lock, and a
-- thread's inserts grew quadratically. A message row only ever moves one
-- (folder, thread) pair by one message and at most one unread message, so the
-- triggers now apply that delta. `visible` keeps its 0095 definition and stays
-- maintained by the threads/folders triggers; the upsert refreshes it as before.
DROP TRIGGER IF EXISTS messages_thread_folders_ai;
DROP TRIGGER IF EXISTS messages_thread_folders_ad;
DROP TRIGGER IF EXISTS messages_thread_folders_au;

CREATE TRIGGER messages_thread_folders_ai AFTER INSERT ON messages
WHEN new.thread_id IS NOT NULL
BEGIN
    INSERT INTO thread_folders(folder_id, thread_id, message_count, unread_count, visible)
    SELECT f.id, new.thread_id, 1, COALESCE(new.is_read = 0, 0),
           CASE WHEN LOWER(TRIM(f.folder_type)) = 'inbox'
                THEN COALESCE(t.workflow_state NOT IN ('aside', 'done', 'snoozed'), 0) ELSE 1 END
    FROM folders f
    LEFT JOIN threads t ON t.id = new.thread_id
    WHERE f.id = COALESCE(new.pending_folder_id, new.folder_id)
    ON CONFLICT(folder_id, thread_id) DO UPDATE SET
        message_count = message_count + 1,
        unread_count = unread_count + excluded.unread_count,
        visible = excluded.visible;
END;

CREATE TRIGGER messages_thread_folders_ad AFTER DELETE ON messages
WHEN old.thread_id IS NOT NULL
BEGIN
    UPDATE thread_folders SET message_count = message_count - 1,
        unread_count = MAX(unread_count - COALESCE(old.is_read = 0, 0), 0)
    WHERE folder_id = COALESCE(old.pending_folder_id, old.folder_id) AND thread_id = old.thread_id;
    DELETE FROM thread_folders
    WHERE folder_id = COALESCE(old.pending_folder_id, old.folder_id) AND thread_id = old.thread_id
      AND message_count <= 0;
END;

-- The new pair is counted before the old one is released: when only `is_read`
-- changed, both are the same row and it never passes through zero.
CREATE TRIGGER messages_thread_folders_au AFTER UPDATE OF folder_id, pending_folder_id, thread_id, is_read ON messages
WHEN COALESCE(old.pending_folder_id, old.folder_id) IS NOT COALESCE(new.pending_folder_id, new.folder_id)
  OR old.thread_id IS NOT new.thread_id
  OR COALESCE(old.is_read = 0, 0) <> COALESCE(new.is_read = 0, 0)
BEGIN
    INSERT INTO thread_folders(folder_id, thread_id, message_count, unread_count, visible)
    SELECT f.id, new.thread_id, 1, COALESCE(new.is_read = 0, 0),
           CASE WHEN LOWER(TRIM(f.folder_type)) = 'inbox'
                THEN COALESCE(t.workflow_state NOT IN ('aside', 'done', 'snoozed'), 0) ELSE 1 END
    FROM folders f
    LEFT JOIN threads t ON t.id = new.thread_id
    WHERE new.thread_id IS NOT NULL AND f.id = COALESCE(new.pending_folder_id, new.folder_id)
    ON CONFLICT(folder_id, thread_id) DO UPDATE SET
        message_count = message_count + 1,
        unread_count = unread_count + excluded.unread_count,
        visible = excluded.visible;
    UPDATE thread_folders SET message_count = message_count - 1,
        unread_count = MAX(unread_count - COALESCE(old.is_read = 0, 0), 0)
    WHERE old.thread_id IS NOT NULL
      AND folder_id = COALESCE(old.pending_folder_id, old.folder_id) AND thread_id = old.thread_id;
    DELETE FROM thread_folders
    WHERE old.thread_id IS NOT NULL
      AND folder_id = COALESCE(old.pending_folder_id, old.folder_id) AND thread_id = old.thread_id
      AND message_count <= 0;
END;
