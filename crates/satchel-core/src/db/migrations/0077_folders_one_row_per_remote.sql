-- One local row per remote folder. A move or workflow into a folder the sync
-- had not seen yet created a row with a random id; the next sync upserted by
-- its derived id and added a second row, and messages split between them.
--
-- Merges the duplicates that exist, then makes a new one impossible. Only
-- duplicate rows and their messages are touched. Survivor: the row with the
-- derived `fld-imap-` id, else the most recently updated one. Rows that keep a
-- random id get the derived one at their next folder sync.
CREATE TEMP TABLE satchel_folder_merge AS
SELECT f.id AS old_id,
       (SELECT s.id
        FROM folders s
        WHERE s.account_id = f.account_id
          AND COALESCE(s.remote_id, s.full_path) = COALESCE(f.remote_id, f.full_path)
        ORDER BY (s.id LIKE 'fld-imap-%') DESC, s.updated_at DESC, s.id ASC
        LIMIT 1) AS new_id
FROM folders f;

DELETE FROM satchel_folder_merge WHERE old_id = new_id;

UPDATE messages
SET folder_id = (SELECT new_id FROM satchel_folder_merge WHERE old_id = messages.folder_id)
WHERE folder_id IN (SELECT old_id FROM satchel_folder_merge);

UPDATE messages
SET pending_folder_id = (SELECT new_id FROM satchel_folder_merge WHERE old_id = messages.pending_folder_id)
WHERE pending_folder_id IN (SELECT old_id FROM satchel_folder_merge);

UPDATE OR IGNORE sync_quarantine
SET folder_id = (SELECT new_id FROM satchel_folder_merge WHERE old_id = sync_quarantine.folder_id)
WHERE folder_id IN (SELECT old_id FROM satchel_folder_merge);

DELETE FROM sync_quarantine WHERE folder_id IN (SELECT old_id FROM satchel_folder_merge);

DELETE FROM folders WHERE id IN (SELECT old_id FROM satchel_folder_merge);

DROP TABLE satchel_folder_merge;

CREATE UNIQUE INDEX idx_folders_account_remote
ON folders(account_id, COALESCE(remote_id, full_path));
