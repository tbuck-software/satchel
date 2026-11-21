-- Use the same explicit Resume flow as for an interrupted run. The outbox
-- holds any copies that had already been handed over separately.
UPDATE mail_merge_jobs
SET status = 'paused', pause_reason = 'restart'
WHERE status = 'running';
