ALTER TABLE contacts ADD COLUMN avatar_source TEXT;
ALTER TABLE contacts ADD COLUMN avatar_locked INTEGER NOT NULL DEFAULT 0;
ALTER TABLE contacts ADD COLUMN avatar_updated_at TEXT;
