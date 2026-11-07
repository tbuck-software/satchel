// Generated from Rust. Run pnpm ipc:generate.
export type TaskSyncSummary = {
    lists: number;
    pulled: number;
    pushed: number;
    removed: number;
    /**
     * Local edits the server refused because its copy had moved on; the
     * server's version replaced them.
     */
    conflicts: number;
    /**
     * Tasks deleted here that the server still had changed: they came back.
     */
    restored: number;
    /**
     * Pushes that failed and wait for a retry.
     */
    failed: number;
    /**
     * Local changes to lists the server refused as read-only; dropped.
     */
    read_only_dropped: number;
    /**
     * New tasks a read-only list refused; kept as local tasks.
     */
    read_only_kept: number;
    /**
     * Unsent edits of tasks the server deleted, kept as local tasks.
     */
    kept_local: number;
    /**
     * Lists the server dropped that stay, read-only, for their unsent work.
     */
    lists_gone: number;
};
