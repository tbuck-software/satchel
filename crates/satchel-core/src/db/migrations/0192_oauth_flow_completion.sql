-- Written in the same transaction as a successful authorization's account
-- update. Existing outcomes with no error remain unfinished after upgrade.
ALTER TABLE oauth_flow_outcomes ADD COLUMN completed_connection TEXT;
