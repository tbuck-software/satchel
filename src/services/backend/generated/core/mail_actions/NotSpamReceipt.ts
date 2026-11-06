// Generated from Rust. Run pnpm ipc:generate.
/**
 * What Not spam did, and what its Undo hands back.
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
    /**
     * The training receipt; Undo unlearns it.
     */
    undo_token: string | null;
    /**
     * Receipts of the moves back to the Inbox; Undo replays them.
     */
    move_receipts: Array<string>;
    /**
     * Threads the filter had moved, restored through its own receipt; Undo
     * files them in Spam again.
     */
    restored_thread_ids: Array<string>;
};
