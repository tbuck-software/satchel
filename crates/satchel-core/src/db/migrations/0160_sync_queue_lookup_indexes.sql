-- Bulk actions check every thread against the queue: is a provider move
-- pending for it, which queued flags does a new choice supersede, which
-- marker row lists it. Each lookup parsed every queued payload once per
-- thread, quadratic in the size of a bulk action. The expressions match QUEUED_MOVE_THREAD_ID and
-- QUEUED_KIND in sync/remote_mutations.rs; SQLite only uses an expression
-- index for the same expression.
CREATE INDEX IF NOT EXISTS idx_sync_queue_move_thread
    ON sync_queue((CASE WHEN json_valid(payload) THEN
        CASE json_extract(payload, '$.kind')
            WHEN 'workflow_move' THEN json_extract(payload, '$.workflow_thread_id')
            WHEN 'blocked_move' THEN json_extract(payload, '$.blocked_thread_id')
        END END))
    WHERE action_type = 'remote_mutation';
CREATE INDEX IF NOT EXISTS idx_sync_queue_kind_account
    ON sync_queue((CASE WHEN json_valid(payload) THEN json_extract(payload, '$.kind') END), account_id)
    WHERE action_type = 'remote_mutation';
-- Undo receipts are looked up by thread before every move.
CREATE INDEX IF NOT EXISTS idx_mail_action_threads_thread ON mail_action_threads(thread_id);
