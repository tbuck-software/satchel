CREATE INDEX idx_mail_merge_recipients_queue ON mail_merge_recipients(queue_id)
    WHERE queue_id IS NOT NULL;

CREATE VIEW paused_merge_actions AS
SELECT r.queue_id FROM mail_merge_recipients r
JOIN mail_merge_jobs j ON j.id = r.job_id
WHERE r.queue_id IS NOT NULL AND j.status = 'paused';
