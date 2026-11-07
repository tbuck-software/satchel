// Generated from Rust. Run pnpm ipc:generate.
/**
 * A bulk move a later batch stopped. The batches before stay applied, with
 * their queued provider moves and their Undo receipt; nothing of the rest
 * changed.
 */
export type StoppedEarly = {
    /**
     * Threads the committed batches moved, in request order.
     */
    applied: Array<string>;
    /**
     * Threads left where they were.
     */
    remaining: Array<string>;
    /**
     * Message key of what stopped the move.
     */
    reason_key: string;
};
