// Generated from Rust. Run pnpm ipc:generate.
export type PendingMailNotification = {
    event_id: string;
    thread_id: string;
    account_id: string;
    account_label: string;
    from_name: string | null;
    from_address: string;
    subject: string;
    preview: string;
};
