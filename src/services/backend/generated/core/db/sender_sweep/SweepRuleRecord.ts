// Generated from Rust. Run pnpm ipc:generate.
/**
 * A recurring sweep rule as stored, with the folder name for display.
 */
export type SweepRuleRecord = {
    id: string;
    account_id: string;
    sender_email: string;
    /**
     * `archive`, `trash` or `move`.
     */
    action: string;
    older_than_days: number | null;
    keep_latest: boolean;
    /**
     * Empty on a move rule whose folder went away; such a rule does nothing
     * until it is given a folder again.
     */
    target_folder_id: string | null;
    target_folder_name: string | null;
    last_run_at: string | null;
    last_swept_count: number;
    created_at: string;
    updated_at: string;
};
