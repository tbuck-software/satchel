// Generated from Rust. Run pnpm ipc:generate.
/**
 * What Not spam did.
 */
export type NotSpamReceipt = {
    /**
     * Threads the action taught the filter about.
     */
    thread_ids: Array<string>;
    /**
     * Threads that were in Spam and are on their way back to the Inbox.
     */
    moved_thread_ids: Array<string>;
};
