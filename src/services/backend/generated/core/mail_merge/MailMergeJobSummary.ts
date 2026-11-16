// Generated from Rust. Run pnpm ipc:generate.
export type MailMergeJobSummary = {
    id: string;
    account_id: string;
    /**
     * `running`, `paused`, `cancelled` or `completed`.
     */
    status: string;
    pause_reason: string | null;
    subject: string;
    source_label: string;
    delay_seconds: number;
    next_send_at: string | null;
    created_at: string;
    updated_at: string;
    finished_at: string | null;
    total: number;
    pending: number;
    queued: number;
    sent: number;
    failed: number;
    unknown: number;
    cancelled: number;
};
