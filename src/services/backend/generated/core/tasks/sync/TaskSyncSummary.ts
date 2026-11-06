// Generated from Rust. Run pnpm ipc:generate.
export type TaskSyncSummary = {
    lists: number;
    pulled: number;
    pushed: number;
    removed: number;
    /**
     * Local changes the server refused because its copy had moved on.
     */
    conflicts: number;
};
