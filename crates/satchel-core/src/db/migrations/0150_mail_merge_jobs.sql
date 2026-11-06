-- Mail merge: one personalised copy of a message per recipient.
--
-- A job keeps the message once (`template`, the composer's send input with
-- its attachments) and one row per recipient with that row's field values.
-- The runner renders a row and hands it to the outbox as an ordinary
-- `send_message` action, one at a time and `delay_seconds` apart, so retries,
-- the Sent folder and the send identity are the outbox's. `queue_id` links a
-- row to its outbox action; `message_id_header` is fixed per row, so handing
-- the same row over twice can never deliver it twice.
--
-- The pace is per account, not per job: a copy goes `delay_seconds` after the
-- account's latest hand-over (`handed_over_at`) in any of its jobs.
-- `request_id` makes a retried create return the job it already made.
-- `released_at` marks a finished job whose attachments and unused field
-- values were dropped; finished jobs go after 30 days.
CREATE TABLE mail_merge_jobs (
    id TEXT PRIMARY KEY NOT NULL,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    status TEXT NOT NULL CHECK (status IN ('running', 'paused', 'cancelled', 'completed')),
    -- Why a job paused on its own (`failures`, `restart`); NULL when the user paused it.
    pause_reason TEXT,
    subject TEXT NOT NULL,
    source_label TEXT NOT NULL DEFAULT '',
    template TEXT NOT NULL,
    delay_seconds INTEGER NOT NULL CHECK (delay_seconds BETWEEN 1 AND 3600),
    next_send_at TEXT,
    consecutive_failures INTEGER NOT NULL DEFAULT 0,
    request_id TEXT,
    released_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    finished_at TEXT
);

CREATE INDEX idx_mail_merge_jobs_status ON mail_merge_jobs(status, next_send_at);
CREATE INDEX idx_mail_merge_jobs_account ON mail_merge_jobs(account_id, status);
CREATE UNIQUE INDEX idx_mail_merge_jobs_request ON mail_merge_jobs(request_id) WHERE request_id IS NOT NULL;

CREATE TABLE mail_merge_recipients (
    id TEXT PRIMARY KEY NOT NULL,
    job_id TEXT NOT NULL REFERENCES mail_merge_jobs(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    email TEXT NOT NULL,
    fields TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('pending', 'queued', 'sent', 'failed', 'unknown', 'cancelled')),
    queue_id TEXT,
    message_id_header TEXT NOT NULL,
    error TEXT,
    attempts INTEGER NOT NULL DEFAULT 0,
    handed_over_at TEXT,
    sent_at TEXT,
    updated_at TEXT NOT NULL,
    UNIQUE (job_id, position)
);

CREATE INDEX idx_mail_merge_recipients_job_status ON mail_merge_recipients(job_id, status, position);
