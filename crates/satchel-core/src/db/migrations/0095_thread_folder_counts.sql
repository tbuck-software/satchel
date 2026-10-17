-- Sidebar counts follow effective folders and count distinct visible threads.
ALTER TABLE folders ADD COLUMN thread_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE folders ADD COLUMN thread_unread_count INTEGER NOT NULL DEFAULT 0;
CREATE TABLE thread_folders (
    folder_id TEXT NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
    thread_id TEXT NOT NULL,
    message_count INTEGER NOT NULL,
    unread_count INTEGER NOT NULL,
    visible INTEGER NOT NULL,
    PRIMARY KEY (folder_id, thread_id)
) WITHOUT ROWID;
CREATE INDEX idx_thread_folders_thread ON thread_folders(thread_id, folder_id);
CREATE INDEX idx_messages_effective_folder_thread_read
ON messages(COALESCE(pending_folder_id, folder_id), thread_id, is_read);
CREATE INDEX idx_messages_thread_effective_folder_read
ON messages(thread_id, COALESCE(pending_folder_id, folder_id), is_read);

CREATE TRIGGER thread_folders_ai AFTER INSERT ON thread_folders BEGIN
    UPDATE folders SET thread_count = thread_count + new.visible,
        thread_unread_count = thread_unread_count + (new.visible AND new.unread_count > 0)
    WHERE id = new.folder_id;
END;
CREATE TRIGGER thread_folders_ad AFTER DELETE ON thread_folders BEGIN
    UPDATE folders SET thread_count = thread_count - old.visible,
        thread_unread_count = thread_unread_count - (old.visible AND old.unread_count > 0)
    WHERE id = old.folder_id;
END;
CREATE TRIGGER thread_folders_au AFTER UPDATE ON thread_folders BEGIN
    UPDATE folders SET thread_count = thread_count + new.visible - old.visible,
        thread_unread_count = thread_unread_count
            + (new.visible AND new.unread_count > 0) - (old.visible AND old.unread_count > 0)
    WHERE id = new.folder_id;
END;

CREATE TRIGGER messages_thread_folders_ai AFTER INSERT ON messages BEGIN
    INSERT INTO thread_folders(folder_id, thread_id, message_count, unread_count, visible)
    SELECT COALESCE(new.pending_folder_id, new.folder_id), new.thread_id,
           COUNT(*), COALESCE(SUM(m.is_read = 0), 0),
           CASE WHEN LOWER(TRIM(f.folder_type)) = 'inbox'
                THEN COALESCE(t.workflow_state NOT IN ('aside', 'done', 'snoozed'), 0) ELSE 1 END
    FROM messages m
    JOIN folders f ON f.id = COALESCE(new.pending_folder_id, new.folder_id)
    LEFT JOIN threads t ON t.id = new.thread_id
    WHERE m.thread_id = new.thread_id
      AND COALESCE(m.pending_folder_id, m.folder_id) = f.id
    HAVING COUNT(*) > 0
    ON CONFLICT(folder_id, thread_id) DO UPDATE SET
        message_count = excluded.message_count, unread_count = excluded.unread_count,
        visible = excluded.visible;
    DELETE FROM thread_folders
    WHERE folder_id = COALESCE(new.pending_folder_id, new.folder_id)
      AND thread_id = new.thread_id
      AND NOT EXISTS (SELECT 1 FROM messages m WHERE m.thread_id = new.thread_id
          AND COALESCE(m.pending_folder_id, m.folder_id) = COALESCE(new.pending_folder_id, new.folder_id));
END;

CREATE TRIGGER messages_thread_folders_ad AFTER DELETE ON messages BEGIN
    INSERT INTO thread_folders(folder_id, thread_id, message_count, unread_count, visible)
    SELECT COALESCE(old.pending_folder_id, old.folder_id), old.thread_id,
           COUNT(*), COALESCE(SUM(m.is_read = 0), 0),
           CASE WHEN LOWER(TRIM(f.folder_type)) = 'inbox'
                THEN COALESCE(t.workflow_state NOT IN ('aside', 'done', 'snoozed'), 0) ELSE 1 END
    FROM messages m
    JOIN folders f ON f.id = COALESCE(old.pending_folder_id, old.folder_id)
    LEFT JOIN threads t ON t.id = old.thread_id
    WHERE m.thread_id = old.thread_id
      AND COALESCE(m.pending_folder_id, m.folder_id) = f.id
    HAVING COUNT(*) > 0
    ON CONFLICT(folder_id, thread_id) DO UPDATE SET
        message_count = excluded.message_count, unread_count = excluded.unread_count,
        visible = excluded.visible;
    DELETE FROM thread_folders
    WHERE folder_id = COALESCE(old.pending_folder_id, old.folder_id)
      AND thread_id = old.thread_id
      AND NOT EXISTS (SELECT 1 FROM messages m WHERE m.thread_id = old.thread_id
          AND COALESCE(m.pending_folder_id, m.folder_id) = COALESCE(old.pending_folder_id, old.folder_id));
END;

CREATE TRIGGER messages_thread_folders_au AFTER UPDATE OF folder_id, pending_folder_id, thread_id, is_read ON messages BEGIN
    INSERT INTO thread_folders(folder_id, thread_id, message_count, unread_count, visible)
    SELECT COALESCE(old.pending_folder_id, old.folder_id), old.thread_id,
           COUNT(*), COALESCE(SUM(m.is_read = 0), 0),
           CASE WHEN LOWER(TRIM(f.folder_type)) = 'inbox'
                THEN COALESCE(t.workflow_state NOT IN ('aside', 'done', 'snoozed'), 0) ELSE 1 END
    FROM messages m
    JOIN folders f ON f.id = COALESCE(old.pending_folder_id, old.folder_id)
    LEFT JOIN threads t ON t.id = old.thread_id
    WHERE m.thread_id = old.thread_id
      AND COALESCE(m.pending_folder_id, m.folder_id) = f.id
    HAVING COUNT(*) > 0
    ON CONFLICT(folder_id, thread_id) DO UPDATE SET
        message_count = excluded.message_count, unread_count = excluded.unread_count,
        visible = excluded.visible;
    DELETE FROM thread_folders
    WHERE folder_id = COALESCE(old.pending_folder_id, old.folder_id)
      AND thread_id = old.thread_id
      AND NOT EXISTS (SELECT 1 FROM messages m WHERE m.thread_id = old.thread_id
          AND COALESCE(m.pending_folder_id, m.folder_id) = COALESCE(old.pending_folder_id, old.folder_id));
    INSERT INTO thread_folders(folder_id, thread_id, message_count, unread_count, visible)
    SELECT COALESCE(new.pending_folder_id, new.folder_id), new.thread_id,
           COUNT(*), COALESCE(SUM(m.is_read = 0), 0),
           CASE WHEN LOWER(TRIM(f.folder_type)) = 'inbox'
                THEN COALESCE(t.workflow_state NOT IN ('aside', 'done', 'snoozed'), 0) ELSE 1 END
    FROM messages m
    JOIN folders f ON f.id = COALESCE(new.pending_folder_id, new.folder_id)
    LEFT JOIN threads t ON t.id = new.thread_id
    WHERE m.thread_id = new.thread_id
      AND COALESCE(m.pending_folder_id, m.folder_id) = f.id
    HAVING COUNT(*) > 0
    ON CONFLICT(folder_id, thread_id) DO UPDATE SET
        message_count = excluded.message_count, unread_count = excluded.unread_count,
        visible = excluded.visible;
    DELETE FROM thread_folders
    WHERE folder_id = COALESCE(new.pending_folder_id, new.folder_id)
      AND thread_id = new.thread_id
      AND NOT EXISTS (SELECT 1 FROM messages m WHERE m.thread_id = new.thread_id
          AND COALESCE(m.pending_folder_id, m.folder_id) = COALESCE(new.pending_folder_id, new.folder_id));
END;

CREATE TRIGGER threads_folder_visibility_ai AFTER INSERT ON threads BEGIN
    UPDATE thread_folders SET visible = CASE
        WHEN (SELECT LOWER(TRIM(folder_type)) FROM folders WHERE id = folder_id) = 'inbox'
        THEN COALESCE(new.workflow_state NOT IN ('aside', 'done', 'snoozed'), 0) ELSE 1 END WHERE thread_id = new.id;
END;

CREATE TRIGGER threads_folder_visibility_au AFTER UPDATE OF workflow_state ON threads BEGIN
    UPDATE thread_folders SET visible = CASE
        WHEN (SELECT LOWER(TRIM(folder_type)) FROM folders WHERE id = folder_id) = 'inbox'
        THEN COALESCE(new.workflow_state NOT IN ('aside', 'done', 'snoozed'), 0) ELSE 1 END WHERE thread_id = new.id;
END;

CREATE TRIGGER threads_folder_visibility_ad AFTER DELETE ON threads BEGIN
    UPDATE thread_folders SET visible = CASE
        WHEN (SELECT LOWER(TRIM(folder_type)) FROM folders WHERE id = folder_id) = 'inbox'
        THEN 0 ELSE 1 END WHERE thread_id = old.id;
END;

CREATE TRIGGER folders_thread_visibility_au AFTER UPDATE OF folder_type ON folders BEGIN
    UPDATE thread_folders SET visible = CASE WHEN LOWER(TRIM(new.folder_type)) = 'inbox'
        THEN COALESCE((SELECT workflow_state NOT IN ('aside', 'done', 'snoozed')
             FROM threads WHERE id = thread_id), 0) ELSE 1 END WHERE folder_id = new.id;
END;
INSERT OR IGNORE INTO app_settings(key, value, updated_at)
VALUES ('data_repair.thread_folders', '', strftime('%Y-%m-%dT%H:%M:%SZ', 'now'));
