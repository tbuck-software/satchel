// Generated from Rust. Run pnpm ipc:generate.
import type { TaskSyncSummary } from "../../../core/tasks/sync/TaskSyncSummary";
export type TasksSyncResult = {
    summary: TaskSyncSummary;
    /**
     * Encoded `{ key, params }` errors, one per account that failed.
     */
    errors: Array<string>;
};
