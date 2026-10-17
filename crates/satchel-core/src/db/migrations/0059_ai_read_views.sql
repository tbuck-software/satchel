-- Read-only views for external tools (ADR 0005).
--
-- There is no API: AI tools open satchel.db directly. These views are the
-- contract that makes that safe to depend on. Their column names never change
-- — columns may be added, never renamed or removed. Whoever changes an
-- underlying table adapts the view in the same commit; that is the whole
-- point of the indirection.
--
-- Secrets stay out by construction. No view reaches `accounts.config` (which
-- holds encrypted credentials), `credentials_ref`, `oauth_flow_sessions`,
-- `pgp_keys`, `smime_certificates`, the `smime_*` message blobs, `api_tokens`,
-- `api_audit_log` or `plugin_state`.
--
-- These are ad-hoc read views, not the app's hot paths: several columns are
-- correlated subqueries. Filter on an id and use LIMIT. Read the file
-- read-only (`sqlite3 -readonly`, or `?mode=ro`) — the app keeps a WAL open
-- and a second writer only fights it. See docs/DB_READ_ACCESS.md.

-- v_accounts — one row per configured source (mail account or other kind).
--
-- Identity and freshness only. Connection settings, credential references and
-- sync error texts are deliberately absent: `accounts.config` is where the
-- encrypted credential lives, so exposing it would defeat the keychain.
--
-- kind: 'email' plus non-mail source kinds. provider: 'imap', 'gmail',
-- 'outlook', … is_active: 0 disabled, 1 syncing.
CREATE VIEW IF NOT EXISTS v_accounts AS
SELECT
    a.id                      AS id,
    a.kind                    AS kind,
    a.provider                AS provider,
    a.email                   AS email,
    a.display_name            AS display_name,
    a.color                   AS color,
    COALESCE(a.is_active, 0)  AS is_active,
    a.last_sync_at            AS last_sync_at,
    a.created_at              AS created_at,
    a.updated_at              AS updated_at
FROM accounts a;

-- v_folders — mailbox folders per account.
--
-- folder_type is the semantic role ('inbox', 'sent', 'trash', 'archive',
-- 'drafts', 'junk', 'custom'); `name` is the leaf label and `full_path` the
-- full remote path. Remote sync bookkeeping (remote_id, uidvalidity) is out:
-- it is meaningless without the provider session.
--
-- message_count/unread_count are the provider's counters, refreshed on sync —
-- for exact numbers count over v_messages.
CREATE VIEW IF NOT EXISTS v_folders AS
SELECT
    f.id                        AS id,
    f.account_id                AS account_id,
    a.email                     AS account_email,
    f.name                      AS name,
    f.full_path                 AS full_path,
    f.folder_type               AS folder_type,
    COALESCE(f.message_count, 0) AS message_count,
    COALESCE(f.unread_count, 0)  AS unread_count,
    COALESCE(f.sync_enabled, 1)  AS sync_enabled,
    COALESCE(f.sort_order, 0)    AS sort_order,
    f.created_at                AS created_at,
    f.updated_at                AS updated_at
FROM folders f
LEFT JOIN accounts a ON a.id = f.account_id;

-- v_threads — conversations, the unit the UI and the workflow act on.
--
-- workflow_state: 'inbox' | 'todo' | 'waiting' | 'done' | 'archived' | … —
-- the state a thread was moved to, not a folder. snoozed_until is the stored
-- `threads.snooze_until`: a timestamp in the future means the thread is
-- hidden until then.
--
-- participants is a comma-joined address list derived from the stored JSON;
-- participants_json keeps the raw array (entries are `{"name","email"}`
-- objects or bare address strings, depending on when the row was written).
-- tags is comma-joined tag *names*, topic_ids comma-joined topic ids — join
-- v_tags / v_topics for the rest.
--
-- first_message_date is computed from the messages of the thread and is NULL
-- while a thread has no message rows yet; last_message_date is the stored,
-- indexed sort key.
CREATE VIEW IF NOT EXISTS v_threads AS
SELECT
    t.id                        AS id,
    t.account_id                AS account_id,
    a.email                     AS account_email,
    t.subject                   AS subject,
    t.snippet                   AS snippet,
    (
        SELECT group_concat(email, ', ')
        FROM (
            SELECT DISTINCT
                CASE WHEN participant.type = 'object'
                     THEN json_extract(participant.value, '$.email')
                     ELSE participant.value
                END AS email
            FROM json_each(
                CASE WHEN json_valid(t.participants) THEN t.participants ELSE '[]' END
            ) AS participant
        )
        WHERE email IS NOT NULL AND TRIM(email) <> ''
    )                           AS participants,
    t.participants              AS participants_json,
    COALESCE(t.message_count, 0) AS message_count,
    COALESCE(t.unread_count, 0)  AS unread_count,
    (
        SELECT MIN(m.date) FROM messages m WHERE m.thread_id = t.id
    )                           AS first_message_date,
    t.last_message_date         AS last_message_date,
    t.workflow_state            AS workflow_state,
    t.workflow_changed_at       AS workflow_changed_at,
    t.snooze_until              AS snoozed_until,
    t.due_date                  AS due_date,
    COALESCE(t.is_starred, 0)   AS is_starred,
    COALESCE(t.is_muted, 0)     AS is_muted,
    COALESCE(t.has_attachments, 0) AS has_attachments,
    COALESCE(t.priority, 0)     AS priority,
    t.category                  AS category,
    t.assignee                  AS assignee,
    (
        SELECT group_concat(name, ', ')
        FROM (
            SELECT tags.name AS name
            FROM thread_tags
            JOIN tags ON tags.id = thread_tags.tag_id
            WHERE thread_tags.thread_id = t.id
            ORDER BY tags.name COLLATE NOCASE
        )
    )                           AS tags,
    (
        SELECT group_concat(topic_id, ',')
        FROM (
            SELECT topic_items.topic_id AS topic_id
            FROM topic_items
            WHERE topic_items.item_type = 'thread'
              AND topic_items.item_id = t.id
            ORDER BY topic_items.topic_id
        )
    )                           AS topic_ids,
    t.created_at                AS created_at,
    t.updated_at                AS updated_at
FROM threads t
LEFT JOIN accounts a ON a.id = t.account_id;

-- v_messages — individual mails.
--
-- body_text is the plain-text body as received or derived. The sanitized HTML
-- body is *not* exposed: it is the largest column in the database and unusable
-- as text; has_body_html only tells you it exists. Read raw headers, MIME
-- structure or S/MIME blobs from the tables if you truly need them.
--
-- to_addresses/cc_addresses are JSON arrays of address strings — use
-- `json_each(to_addresses)` to unnest. Bcc is not exposed.
--
-- thread_id may be NULL: drafts and imports are threaded asynchronously.
CREATE VIEW IF NOT EXISTS v_messages AS
SELECT
    m.id                        AS id,
    m.thread_id                 AS thread_id,
    m.account_id                AS account_id,
    a.email                     AS account_email,
    m.folder_id                 AS folder_id,
    f.name                      AS folder_name,
    f.folder_type               AS folder_type,
    m.from_address              AS from_address,
    m.from_name                 AS from_name,
    m.to_addresses              AS to_addresses,
    m.cc_addresses              AS cc_addresses,
    m.subject                   AS subject,
    m.date                      AS date,
    COALESCE(m.is_read, 0)      AS is_read,
    COALESCE(m.is_draft, 0)     AS is_draft,
    COALESCE(m.is_sent, 0)      AS is_sent,
    COALESCE(m.is_starred, 0)   AS is_starred,
    COALESCE(m.has_attachments, 0) AS has_attachments,
    m.snippet                   AS snippet,
    m.body_text                 AS body_text,
    (m.body_html IS NOT NULL AND m.body_html <> '') AS has_body_html,
    m.raw_size                  AS size_bytes,
    m.category                  AS category,
    m.list_unsubscribe          AS list_unsubscribe,
    m.created_at                AS created_at,
    m.updated_at                AS updated_at
FROM messages m
LEFT JOIN accounts a ON a.id = m.account_id
LEFT JOIN folders f ON f.id = m.folder_id;

-- v_attachments — attachment metadata, never content.
--
-- The bytes live on disk, not in this table. local_path points at the
-- downloaded copy and is NULL until the attachment has been fetched;
-- is_inline = 1 marks parts referenced by the HTML body (signature images and
-- the like) rather than real attachments.
CREATE VIEW IF NOT EXISTS v_attachments AS
SELECT
    att.id                      AS id,
    att.message_id              AS message_id,
    m.thread_id                 AS thread_id,
    m.account_id                AS account_id,
    att.filename                AS filename,
    att.content_type            AS content_type,
    COALESCE(att.size, 0)       AS size_bytes,
    COALESCE(att.is_inline, 0)  AS is_inline,
    att.content_id              AS content_id,
    (COALESCE(att.cached_path, att.storage_path) IS NOT NULL) AS is_downloaded,
    COALESCE(att.cached_path, att.storage_path) AS local_path,
    att.created_at              AS created_at
FROM attachments att
LEFT JOIN messages m ON m.id = att.message_id;

-- v_topics — cross-source collections ("Themen"), one per ongoing matter.
--
-- status: 'active' | 'done' | 'archived'. due_date is the follow-up date
-- ("Wiedervorlage"). The counts come from the polymorphic topic_items link
-- table; note that linked items may already be deleted — the counts include
-- links, and v_threads/v_messages are the truth about what still exists.
CREATE VIEW IF NOT EXISTS v_topics AS
SELECT
    tp.id                       AS id,
    tp.name                     AS name,
    tp.description              AS description,
    tp.status                   AS status,
    tp.color                    AS color,
    tp.icon                     AS icon,
    tp.due_date                 AS due_date,
    (
        SELECT COUNT(1) FROM topic_items
        WHERE topic_items.topic_id = tp.id AND topic_items.item_type = 'thread'
    )                           AS thread_count,
    (
        SELECT COUNT(1) FROM topic_items
        WHERE topic_items.topic_id = tp.id AND topic_items.item_type = 'message'
    )                           AS message_count,
    (
        SELECT COUNT(1) FROM topic_notes WHERE topic_notes.topic_id = tp.id
    )                           AS note_count,
    (
        SELECT COUNT(1) FROM topic_files WHERE topic_files.topic_id = tp.id
    )                           AS file_count,
    tp.created_at               AS created_at,
    tp.updated_at               AS updated_at
FROM topics tp;

-- v_tags — thread labels, with how many threads carry each.
--
-- is_system = 1 marks tags the app creates itself; those are not user data
-- and may reappear after deletion.
CREATE VIEW IF NOT EXISTS v_tags AS
SELECT
    tg.id                       AS id,
    tg.name                     AS name,
    tg.color                    AS color,
    tg.icon                     AS icon,
    COALESCE(tg.is_system, 0)   AS is_system,
    COALESCE(tg.sort_order, 0)  AS sort_order,
    (
        SELECT COUNT(1) FROM thread_tags WHERE thread_tags.tag_id = tg.id
    )                           AS thread_count
FROM tags tg;
