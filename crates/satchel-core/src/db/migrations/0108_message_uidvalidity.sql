-- A folder's current generation cannot establish an old message's identity.
-- Leave legacy rows unknown until an envelope is fetched again.
ALTER TABLE messages ADD COLUMN remote_uidvalidity INTEGER;
