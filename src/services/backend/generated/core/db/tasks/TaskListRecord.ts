// Generated from Rust. Run pnpm ipc:generate.
export type TaskListRecord = {
    id: string;
    /**
     * None for a local list.
     */
    account_id: string | null;
    /**
     * `local`, `caldav`; later `google`, `microsoft`.
     */
    provider: string;
    /**
     * Empty for the default local list, which the frontend names.
     */
    name: string;
    color: string | null;
    read_only: boolean;
    last_synced_at: string | null;
    open_count: number;
};
