-- The account's name in Satchel's own lists ("Work", "Club"). `display_name`
-- stays the name on outgoing mail; NULL here shows that one instead.
ALTER TABLE accounts ADD COLUMN label TEXT;
