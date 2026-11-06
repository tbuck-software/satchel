// Generated from Rust. Run pnpm ipc:generate.
export type MailMergeRecipientRow = {
    id: string;
    position: number;
    email: string;
    /**
     * `pending`, `queued`, `sent`, `failed`, `unknown` or `cancelled`.
     */
    status: string;
    error: string | null;
    attempts: number;
    sent_at: string | null;
};
