-- Keep provider edits separate from the local revision that owns pending uploads.
ALTER TABLE messages ADD COLUMN provider_draft_revision INTEGER NOT NULL DEFAULT 0;
