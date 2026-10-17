// Generated from Rust. Run pnpm ipc:generate.
export type DiagnosticsQueueEntryDto = {
    id: string;
    account_id: string;
    action_type: string;
    status: string;
    error: string | null;
    attempts: number;
    next_due: string | null;
    updated_at: string;
};
