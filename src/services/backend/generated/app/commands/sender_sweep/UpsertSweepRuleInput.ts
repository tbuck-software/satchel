// Generated from Rust. Run pnpm ipc:generate.
export type UpsertSweepRuleInput = {
    account_id: string;
    sender_email: string;
    /**
     * `archive`, `trash` or `move`.
     */
    action: string;
    older_than_days?: number | null;
    keep_latest: boolean;
    target_folder_id?: string | null;
};
