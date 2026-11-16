-- The From address of the mail that first stored an invite. Updates from
-- other mail must come from it or from the organizer. Older rows read NULL
-- and fall back to their linked message's sender.
ALTER TABLE calendar_invites ADD COLUMN source_sender TEXT;
