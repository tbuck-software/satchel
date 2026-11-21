-- Keep copied failure details separate from the queue's retry classification.
CREATE TABLE IF NOT EXISTS restored_queue_failures (
    queue_id TEXT PRIMARY KEY REFERENCES sync_queue(id) ON DELETE CASCADE,
    error TEXT NOT NULL
);

CREATE TRIGGER IF NOT EXISTS restored_queue_failures_clear_on_retry
AFTER UPDATE OF status ON sync_queue
WHEN OLD.status IN ('failed', 'delivery_unknown') AND NEW.status IN ('pending', 'scheduled')
BEGIN
    DELETE FROM restored_queue_failures WHERE queue_id = NEW.id;
END;
