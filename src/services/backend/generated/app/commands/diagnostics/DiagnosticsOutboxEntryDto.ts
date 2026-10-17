// Generated from Rust. Run pnpm ipc:generate.
export type DiagnosticsOutboxEntryDto = {
    message_id: string;
    state: string;
    created_at: string;
    delivered_at: string | null;
    /**
     * Reason code, split off the `<code>: <detail>` note.
     */
    reason: string | null;
    detail: string | null;
    subject: string | null;
    from_name: string | null;
    from_address: string | null;
    date: string | null;
    account_id: string | null;
};
