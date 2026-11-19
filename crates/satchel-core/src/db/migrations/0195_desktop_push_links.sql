CREATE TABLE desktop_push_links (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    credential TEXT NOT NULL,
    approved INTEGER NOT NULL DEFAULT 0,
    pending INTEGER NOT NULL DEFAULT 0
);
