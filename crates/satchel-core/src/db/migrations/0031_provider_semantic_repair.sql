UPDATE messages
SET is_draft = 1
WHERE is_draft <> 1
  AND EXISTS (
      SELECT 1 FROM folders
      WHERE folders.id = messages.folder_id
        AND LOWER(TRIM(folders.folder_type)) = 'drafts'
  );

UPDATE messages
SET is_sent = 1
WHERE is_sent <> 1
  AND EXISTS (
      SELECT 1 FROM folders
      WHERE folders.id = messages.folder_id
        AND LOWER(TRIM(folders.folder_type)) = 'sent'
  );

UPDATE threads
SET message_count = (
        SELECT COUNT(DISTINCT CASE
            WHEN messages.id LIKE 'm-imap-%' THEN COALESCE(
                NULLIF(TRIM(CASE
                    WHEN LOWER(COALESCE(messages.message_id_header, '')) LIKE '%@imap.flowmail.local>' THEN ''
                    ELSE COALESCE(messages.message_id_header, '')
                END), ''),
                COALESCE(NULLIF(TRIM(messages.folder_id), ''), '') || ':' ||
                    COALESCE(NULLIF(TRIM(messages.remote_id), ''), messages.id)
            )
            ELSE COALESCE(
                NULLIF(TRIM(messages.message_id_header), ''),
                NULLIF(TRIM(messages.remote_id), ''),
                messages.id
            )
        END)
        FROM messages
        WHERE messages.thread_id = threads.id
    ),
    unread_count = (
        SELECT COUNT(DISTINCT CASE
            WHEN messages.id LIKE 'm-imap-%' THEN COALESCE(
                NULLIF(TRIM(CASE
                    WHEN LOWER(COALESCE(messages.message_id_header, '')) LIKE '%@imap.flowmail.local>' THEN ''
                    ELSE COALESCE(messages.message_id_header, '')
                END), ''),
                COALESCE(NULLIF(TRIM(messages.folder_id), ''), '') || ':' ||
                    COALESCE(NULLIF(TRIM(messages.remote_id), ''), messages.id)
            )
            ELSE COALESCE(
                NULLIF(TRIM(messages.message_id_header), ''),
                NULLIF(TRIM(messages.remote_id), ''),
                messages.id
            )
        END)
        FROM messages
        WHERE messages.thread_id = threads.id AND messages.is_read = 0
    ),
    is_starred = COALESCE((
        SELECT MAX(messages.is_starred)
        FROM messages
        WHERE messages.thread_id = threads.id
    ), 0);
