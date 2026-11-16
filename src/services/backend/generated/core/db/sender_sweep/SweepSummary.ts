// Generated from Rust. Run pnpm ipc:generate.
/**
 * How much a sweep takes and what it leaves alone, without the ids.
 */
export type SweepSummary = {
    /**
     * Threads the sweep would file.
     */
    count: number;
    /**
     * The sender's Inbox threads with someone else, an own reply or a draft.
     */
    skipped_shared: number;
    /**
     * The sender's own threads the user starred.
     */
    skipped_starred: number;
};
