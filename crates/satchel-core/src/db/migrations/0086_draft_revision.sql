-- Optimistic concurrency for draft saves. Only save_draft bumps it; thread-wide
-- updates (read flags, moves) touch updated_at and must not look like an edit.
ALTER TABLE messages ADD COLUMN draft_revision INTEGER NOT NULL DEFAULT 0;
