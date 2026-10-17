SELECT
            folders.id,
            folders.account_id,
            accounts.email AS account_email,
            folders.name,
            folders.folder_type,
            COUNT(DISTINCT CASE
              WHEN NOT (
                LOWER(TRIM(folders.folder_type)) = 'inbox'
                AND threads.workflow_state IN ('aside', 'done', 'snoozed')
              )
              THEN messages.thread_id
            END) AS thread_count,
            COUNT(DISTINCT CASE
              WHEN messages.is_read = 0
               AND NOT (
                 LOWER(TRIM(folders.folder_type)) = 'inbox'
                 AND threads.workflow_state IN ('aside', 'done', 'snoozed')
               )
              THEN messages.thread_id
            END) AS unread_count
         FROM folders
         JOIN accounts ON accounts.id = folders.account_id
         LEFT JOIN messages ON
           (messages.pending_folder_id IS NULL AND messages.folder_id = folders.id)
           OR messages.pending_folder_id = folders.id
         LEFT JOIN threads ON threads.id = messages.thread_id
         WHERE accounts.is_active = 1 AND accounts.kind = 'email'
           AND UPPER(TRIM(COALESCE(folders.remote_id, ''))) NOT IN (
             'CATEGORY_PERSONAL', 'CATEGORY_PROMOTIONS', 'CATEGORY_SOCIAL',
             'CATEGORY_UPDATES', 'CATEGORY_FORUMS',
             'IMPORTANT', 'STARRED', 'UNREAD', 'CHAT'
           )
         GROUP BY folders.id, folders.account_id, accounts.email, folders.name, folders.folder_type
         ORDER BY accounts.email ASC, folders.name ASC