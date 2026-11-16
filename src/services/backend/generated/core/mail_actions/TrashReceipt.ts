// Generated from Rust. Run pnpm ipc:generate.
import type { StoppedEarly } from "./StoppedEarly";
export type TrashReceipt = {
    undo_token: string | null;
    undo_thread_ids: Array<string>;
    permanently_deleted: Array<string>;
    /**
     * Set when a later batch failed: the receipt covers the threads before it.
     */
    stopped_early?: StoppedEarly;
};
