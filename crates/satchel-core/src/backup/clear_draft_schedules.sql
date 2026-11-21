UPDATE messages SET scheduled_for = NULL WHERE is_draft = 1 AND scheduled_for IS NOT NULL;
