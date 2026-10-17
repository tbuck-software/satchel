// Generated from Rust. Run pnpm ipc:generate.
/**
 * Lightweight search hit for the omni panel and the search page.
 *
 * Deliberately not a `MessageDto`: search-as-you-type must not pay for body
 * transport, plugin transforms or PGP/S/MIME decryption per row. Opening a hit
 * loads the full message through the regular thread path.
 */
export type MessageSearchHitDto = {
    id: string;
    thread_id: string;
    subject: string;
    /**
     * FTS `snippet()` output with `SEARCH_MARK_*` around matches, or the
     * stored snippet (no markers) for operator-only queries.
     */
    snippet: string;
    from_name: string;
    from_address: string;
    date: string;
    is_read: boolean;
    /**
     * Thread workflow state ('inbox' | 'todo' | 'waiting' | 'aside' | 'done' | 'snoozed').
     */
    workflow_state: string;
    /**
     * Folder type of the hit message ('sent', 'trash', 'spam', …) or ''.
     */
    folder_type: string;
};
