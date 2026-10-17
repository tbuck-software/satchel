-- Sender authentication results parsed from Authentication-Results headers
-- (RFC 8601). Values are the raw verdict tokens (pass/fail/softfail/neutral/
-- none/temperror/permerror); NULL when the message carried no verdict.
ALTER TABLE messages ADD COLUMN auth_spf TEXT;
ALTER TABLE messages ADD COLUMN auth_dkim TEXT;
ALTER TABLE messages ADD COLUMN auth_dmarc TEXT;
