// Generated from Rust. Run pnpm ipc:generate.
/**
 * Durable progress survives window reloads and the pauses between bounded sync runs.
 */
export type AccountSyncProgress = {
    account_id: string;
    account_name: string;
    email: string;
    phase: string;
    folder: string | null;
    folder_index: number | null;
    folder_total: number | null;
    backfill_remaining: number;
    backfill_total: number;
    updated_at: string;
};
