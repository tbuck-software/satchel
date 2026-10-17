ALTER TABLE threads ADD COLUMN is_muted INTEGER NOT NULL DEFAULT 0;

CREATE INDEX IF NOT EXISTS idx_threads_muted ON threads(is_muted) WHERE is_muted = 1;
