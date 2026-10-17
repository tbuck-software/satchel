-- v_folders: make the counters count (ADR 0005).
--
-- Supersedes the definition in 0059, which handed both columns straight out of
-- `folders`. Those columns are written once, with 0, when the folder row is
-- created; no writer ever updates them. Every external reader following the
-- example query in docs/DB_READ_ACCESS.md got a constant 0 — a wrong number,
-- not an error.
--
-- The view counts instead of the sync maintaining a counter: nothing in the app
-- reads the two columns (`list_mailboxes` counts over `messages` itself), so a
-- maintained counter would exist for external readers alone and would have to
-- be kept correct in every path that writes, moves or deletes a message, plus a
-- reconcile run for the rows already at 0.
--
-- The numbers are message rows in the folder and agree with
-- `SELECT COUNT(*) FROM v_messages WHERE folder_id = …`. The mail UI shows
-- something else on purpose: it counts threads, follows pending moves and hides
-- threads that are done, snoozed or set aside.
--
-- Both subqueries run as covering index searches (idx_messages_folder and
-- idx_messages_folder_thread_read). Still an ad-hoc read view, not a hot path:
-- filter on an id and use LIMIT.
DROP VIEW IF EXISTS v_folders;
CREATE VIEW v_folders AS
SELECT
    f.id                        AS id,
    f.account_id                AS account_id,
    a.email                     AS account_email,
    f.name                      AS name,
    f.full_path                 AS full_path,
    f.folder_type               AS folder_type,
    (
        SELECT COUNT(1) FROM messages m WHERE m.folder_id = f.id
    )                           AS message_count,
    (
        SELECT COUNT(1) FROM messages m
        WHERE m.folder_id = f.id AND COALESCE(m.is_read, 0) = 0
    )                           AS unread_count,
    COALESCE(f.sync_enabled, 1)  AS sync_enabled,
    COALESCE(f.sort_order, 0)    AS sort_order,
    f.created_at                AS created_at,
    f.updated_at                AS updated_at
FROM folders f
LEFT JOIN accounts a ON a.id = f.account_id;
