-- 1 when the provider declared the folder's type itself (IMAP SPECIAL-USE,
-- Gmail system label, Outlook well-known name) rather than Satchel guessing
-- it from the name. Picks the Sent folder when an account has several
-- (`db::sent_copies::preferred_sent_folder`). Rows read 0 until the next
-- folder listing rewrites them.
ALTER TABLE folders ADD COLUMN special_use INTEGER NOT NULL DEFAULT 0;

-- Local copies of sent mail that landed in a different Sent folder than the
-- server copy, and so were never replaced by it.
INSERT OR IGNORE INTO app_settings (key, value, updated_at)
VALUES ('data_repair.sent_local_copies', '', strftime('%Y-%m-%dT%H:%M:%SZ', 'now'));
