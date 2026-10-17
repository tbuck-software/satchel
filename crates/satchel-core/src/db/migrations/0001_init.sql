CREATE TABLE IF NOT EXISTS accounts (
    id TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    email TEXT NOT NULL,
    provider TEXT NOT NULL,
    config TEXT NOT NULL,
    credentials_ref TEXT NOT NULL,
    sync_state TEXT,
    color TEXT,
    is_active INTEGER DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS folders (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id),
    name TEXT NOT NULL,
    full_path TEXT NOT NULL,
    folder_type TEXT NOT NULL,
    remote_id TEXT,
    uidvalidity INTEGER,
    message_count INTEGER DEFAULT 0,
    unread_count INTEGER DEFAULT 0,
    sync_enabled INTEGER DEFAULT 1,
    sort_order INTEGER DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS messages (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id),
    folder_id TEXT NOT NULL REFERENCES folders(id),
    thread_id TEXT,
    message_id_header TEXT,
    in_reply_to TEXT,
    references_header TEXT,
    from_address TEXT NOT NULL,
    from_name TEXT,
    to_addresses TEXT NOT NULL,
    cc_addresses TEXT,
    bcc_addresses TEXT,
    reply_to TEXT,
    subject TEXT,
    date TEXT NOT NULL,
    body_text TEXT,
    body_html TEXT,
    snippet TEXT,
    is_read INTEGER DEFAULT 0,
    is_starred INTEGER DEFAULT 0,
    is_draft INTEGER DEFAULT 0,
    is_sent INTEGER DEFAULT 0,
    has_attachments INTEGER DEFAULT 0,
    remote_uid INTEGER,
    remote_id TEXT,
    raw_size INTEGER,
    raw_headers TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
    subject,
    body_text,
    from_name,
    from_address,
    content='messages',
    content_rowid='rowid',
    tokenize='unicode61 remove_diacritics 2'
);

CREATE TABLE IF NOT EXISTS threads (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id),
    subject TEXT,
    last_message_date TEXT NOT NULL,
    message_count INTEGER DEFAULT 1,
    unread_count INTEGER DEFAULT 0,
    has_attachments INTEGER DEFAULT 0,
    participants TEXT,
    snippet TEXT,
    workflow_state TEXT DEFAULT 'inbox',
    workflow_changed_at TEXT,
    snooze_until TEXT,
    due_date TEXT,
    priority INTEGER DEFAULT 0,
    board_column TEXT,
    board_position REAL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS attachments (
    id TEXT PRIMARY KEY,
    message_id TEXT NOT NULL REFERENCES messages(id),
    filename TEXT,
    content_type TEXT NOT NULL,
    size INTEGER,
    content_id TEXT,
    storage_path TEXT,
    is_inline INTEGER DEFAULT 0,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS tags (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    color TEXT,
    icon TEXT,
    is_system INTEGER DEFAULT 0,
    sort_order INTEGER DEFAULT 0
);

CREATE TABLE IF NOT EXISTS thread_tags (
    thread_id TEXT NOT NULL REFERENCES threads(id),
    tag_id TEXT NOT NULL REFERENCES tags(id),
    PRIMARY KEY (thread_id, tag_id)
);

CREATE TABLE IF NOT EXISTS rules (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    description TEXT,
    is_active INTEGER DEFAULT 1,
    priority INTEGER DEFAULT 0,
    trigger_type TEXT NOT NULL,
    trigger_config TEXT NOT NULL,
    actions TEXT NOT NULL,
    last_triggered TEXT,
    trigger_count INTEGER DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS plugin_state (
    plugin_id TEXT NOT NULL,
    key TEXT NOT NULL,
    value TEXT,
    PRIMARY KEY (plugin_id, key)
);

CREATE TABLE IF NOT EXISTS contacts (
    id TEXT PRIMARY KEY,
    email TEXT NOT NULL UNIQUE,
    display_name TEXT,
    avatar_url TEXT,
    frequency INTEGER DEFAULT 0,
    last_contacted TEXT,
    notes TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS sync_queue (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL,
    action_type TEXT NOT NULL,
    payload TEXT NOT NULL,
    status TEXT DEFAULT 'pending',
    retry_count INTEGER DEFAULT 0,
    last_error TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_messages_thread ON messages(thread_id);
CREATE INDEX IF NOT EXISTS idx_messages_folder ON messages(folder_id);
CREATE INDEX IF NOT EXISTS idx_messages_date ON messages(date DESC);
CREATE INDEX IF NOT EXISTS idx_messages_account ON messages(account_id);
CREATE INDEX IF NOT EXISTS idx_threads_workflow ON threads(workflow_state);
CREATE INDEX IF NOT EXISTS idx_threads_board ON threads(board_column, board_position);
CREATE INDEX IF NOT EXISTS idx_threads_snooze ON threads(snooze_until) WHERE snooze_until IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_threads_date ON threads(last_message_date DESC);
CREATE INDEX IF NOT EXISTS idx_folders_account ON folders(account_id);
CREATE INDEX IF NOT EXISTS idx_attachments_message ON attachments(message_id);
