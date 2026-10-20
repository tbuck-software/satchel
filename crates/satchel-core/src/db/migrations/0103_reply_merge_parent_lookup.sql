-- The reply-merge pass only checks messages added since its last run. A new
-- message can be the parent of a reply stored earlier, so the pass looks up
-- replies by the header they answer.
CREATE INDEX IF NOT EXISTS idx_messages_account_in_reply_to
ON messages(account_id, in_reply_to) WHERE in_reply_to IS NOT NULL;
