-- METHOD was parsed but never stored, so a CANCEL looked exactly like a fresh
-- invitation: cancelled meetings stayed in the list as if still scheduled.
ALTER TABLE calendar_invites ADD COLUMN method TEXT;
ALTER TABLE calendar_invites ADD COLUMN cancelled INTEGER NOT NULL DEFAULT 0;
