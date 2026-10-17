// Generated from Rust. Run pnpm ipc:generate.
export type FollowUpDto = {
    id: string;
    account_id: string;
    thread_id: string;
    message_id: string | null;
    due_at: string;
    status: string;
    note: string | null;
    subject: string | null;
    snippet: string | null;
    created_at: string;
    updated_at: string;
    resolved_at: string | null;
};
