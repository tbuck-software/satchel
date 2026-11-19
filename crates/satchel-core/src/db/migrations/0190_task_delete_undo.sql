-- Keep the original task available for Undo after the server acknowledges DELETE.
ALTER TABLE tasks ADD COLUMN delete_confirmed_at TEXT;
CREATE INDEX idx_tasks_delete_confirmed ON tasks(delete_confirmed_at)
    WHERE delete_confirmed_at IS NOT NULL;
