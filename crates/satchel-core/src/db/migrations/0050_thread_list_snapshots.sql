-- Exact first-page projections for cache-first Inbox rendering. The full
-- query refreshes these snapshots; startup can read the small JSON row while
-- the current projection is recomputed in parallel.
CREATE TABLE IF NOT EXISTS thread_list_snapshots (
    cache_key TEXT PRIMARY KEY,
    page_json TEXT NOT NULL,
    updated_at TEXT NOT NULL
) WITHOUT ROWID;
