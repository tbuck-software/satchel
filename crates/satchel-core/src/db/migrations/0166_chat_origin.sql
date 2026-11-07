-- The message a Delta Chat thread started with, stamped when it is stored:
-- the first chat message of its thread at that moment. Who a reply goes to
-- before the user has written in the chat comes from this message
-- (`db::chats`), not from whatever is oldest in the thread later, which a
-- thread merge or a provider conversation can change.
ALTER TABLE messages ADD COLUMN chat_origin INTEGER NOT NULL DEFAULT 0;
