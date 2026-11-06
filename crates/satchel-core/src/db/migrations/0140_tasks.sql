-- Tasks: local lists that need no account, and remote lists mirrored from a
-- provider (CalDAV VTODO collections today; Google Tasks and Microsoft To Do
-- fit the same columns later).
--
-- The remote columns are deliberately provider-neutral: for CalDAV
-- `remote_id` is the resource href, `remote_version` its ETag and
-- `remote_payload` the last iCalendar text the server returned, which edits
-- patch so properties Satchel does not understand survive a round trip.
--
-- `pending_op` is the outbox: a local change to a remote task is written
-- here in the same statement as the change and cleared once the server has
-- it. A deletion stays as a hidden row until the server confirmed it.
CREATE TABLE task_lists (
    id TEXT PRIMARY KEY NOT NULL,
    -- NULL for local lists.
    account_id TEXT REFERENCES accounts(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    remote_id TEXT,
    -- Empty means "the default local list"; the frontend names it.
    name TEXT NOT NULL DEFAULT '',
    color TEXT,
    read_only INTEGER NOT NULL DEFAULT 0 CHECK (read_only IN (0, 1)),
    sync_cursor TEXT,
    last_synced_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    CHECK ((account_id IS NULL) = (remote_id IS NULL))
);

CREATE UNIQUE INDEX idx_task_lists_remote
    ON task_lists(account_id, remote_id)
    WHERE remote_id IS NOT NULL;

CREATE TABLE tasks (
    id TEXT PRIMARY KEY NOT NULL,
    list_id TEXT NOT NULL REFERENCES task_lists(id) ON DELETE CASCADE,
    uid TEXT NOT NULL,
    title TEXT NOT NULL,
    notes TEXT,
    -- 'YYYY-MM-DD' for a due date, RFC 3339 UTC for a due date-time.
    due_at TEXT,
    -- RFC 3339 UTC.
    reminder_at TEXT,
    -- When the reminder was shown; cleared whenever `reminder_at` changes.
    reminded_at TEXT,
    -- iCalendar scale: 0 none, 1 highest … 9 lowest.
    priority INTEGER NOT NULL DEFAULT 0 CHECK (priority BETWEEN 0 AND 9),
    completed_at TEXT,
    -- The mail the task was made from. No foreign key: threads merge and
    -- are rebuilt by provider reconciles; the Message-ID finds it again.
    thread_id TEXT,
    message_id TEXT,
    remote_id TEXT,
    remote_version TEXT,
    remote_payload TEXT,
    pending_op TEXT CHECK (pending_op IS NULL OR pending_op IN ('create', 'update', 'delete')),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX idx_tasks_list ON tasks(list_id, completed_at);
CREATE UNIQUE INDEX idx_tasks_remote ON tasks(list_id, remote_id) WHERE remote_id IS NOT NULL;
CREATE INDEX idx_tasks_open_due ON tasks(due_at) WHERE completed_at IS NULL;
CREATE INDEX idx_tasks_reminder ON tasks(reminder_at)
    WHERE reminder_at IS NOT NULL AND reminded_at IS NULL AND completed_at IS NULL;
CREATE INDEX idx_tasks_thread ON tasks(thread_id) WHERE thread_id IS NOT NULL;
CREATE INDEX idx_tasks_pending ON tasks(list_id) WHERE pending_op IS NOT NULL;
