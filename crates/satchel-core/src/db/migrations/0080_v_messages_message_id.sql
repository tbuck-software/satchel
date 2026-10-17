-- v_messages: expose the Message-ID (ADR 0005).
--
-- `satchel://message/<id>` links name a mail by its Message-ID, so a reader
-- holding such a link has to be able to find it. Supersedes the definition in
-- 0059; the new column is appended, every existing one stays as it was.
--
-- The value is the raw header, angle brackets included (`<abc@host>`); the link
-- carries it without them.
DROP VIEW IF EXISTS v_messages;
CREATE VIEW v_messages AS
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
    m.updated_at                AS updated_at,
    m.message_id_header         AS message_id_header
FROM messages m
LEFT JOIN accounts a ON a.id = m.account_id
LEFT JOIN folders f ON f.id = m.folder_id;
