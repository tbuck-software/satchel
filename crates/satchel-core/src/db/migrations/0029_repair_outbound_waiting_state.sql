-- This repair follows the signature profile schema introduced in migration 0028.
-- Replies sent outside FlowMail arrive through provider sync rather than the
-- local send pipeline. Older Gmail syncs therefore left conversations whose
-- newest message is the user's own reply in Inbox instead of Waiting.
UPDATE threads
SET workflow_state = 'waiting',
    board_column = 'waiting',
    workflow_changed_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
    updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
WHERE workflow_state IN ('inbox', 'todo')
  AND unread_count = 0
  AND EXISTS (
    SELECT 1
    FROM messages latest
    JOIN accounts account ON account.id = latest.account_id
    JOIN folders latest_folder ON latest_folder.id = latest.folder_id
    WHERE latest.thread_id = threads.id
      AND latest.date = (
        SELECT MAX(candidate.date)
        FROM messages candidate
        WHERE candidate.thread_id = threads.id
      )
      AND (
        LOWER(TRIM(latest.from_address)) = LOWER(TRIM(account.email))
        OR EXISTS (
          SELECT 1
          FROM account_aliases alias
          WHERE alias.account_id = account.id
            AND LOWER(TRIM(alias.email)) = LOWER(TRIM(latest.from_address))
        )
      )
      AND (
        latest.is_sent = 1
        OR LOWER(latest_folder.folder_type) = 'sent'
        OR EXISTS (
          SELECT 1
          FROM messages sent_copy
          JOIN folders sent_folder ON sent_folder.id = sent_copy.folder_id
          WHERE sent_copy.account_id = latest.account_id
            AND sent_copy.remote_id = latest.remote_id
            AND LOWER(sent_folder.folder_type) = 'sent'
        )
      )
  );
