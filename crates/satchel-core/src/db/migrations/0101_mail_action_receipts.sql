-- Undo follows the provider delivery through reconciliation, including IMAP UID changes.
CREATE TABLE mail_action_receipts (
    id TEXT PRIMARY KEY,
    state TEXT NOT NULL DEFAULT 'available',
    created_at TEXT NOT NULL
);
CREATE TABLE mail_action_threads (
    receipt_id TEXT NOT NULL REFERENCES mail_action_receipts(id) ON DELETE CASCADE,
    thread_id TEXT NOT NULL,
    account_id TEXT NOT NULL,
    workflow_state TEXT NOT NULL,
    board_column TEXT NOT NULL,
    snooze_until TEXT,
    action_at TEXT NOT NULL,
    PRIMARY KEY (receipt_id, thread_id)
);
CREATE TABLE mail_action_messages (
    receipt_id TEXT NOT NULL REFERENCES mail_action_receipts(id) ON DELETE CASCADE,
    message_id TEXT NOT NULL,
    original_folder_id TEXT NOT NULL,
    target_folder_id TEXT NOT NULL,
    PRIMARY KEY (receipt_id, message_id)
);
CREATE INDEX idx_mail_action_messages_current ON mail_action_messages(message_id);
CREATE TABLE mail_action_queue (
    receipt_id TEXT NOT NULL REFERENCES mail_action_receipts(id) ON DELETE CASCADE,
    action_id TEXT NOT NULL,
    PRIMARY KEY (receipt_id, action_id)
);
-- A dependent action waits without spending retry attempts while its delivery
-- is still pending, sending, reconciling, or waiting for a manual retry.
CREATE TABLE sync_queue_dependencies (
    action_id TEXT NOT NULL REFERENCES sync_queue(id) ON DELETE CASCADE,
    depends_on TEXT NOT NULL,
    PRIMARY KEY (action_id, depends_on)
);
