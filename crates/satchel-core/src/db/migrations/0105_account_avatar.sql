-- A picture the user chose for an account, as a small data: URL. It stays in
-- the file, so the sidebar never asks a server who the account belongs to.
ALTER TABLE accounts ADD COLUMN avatar TEXT;
