// Generated from Rust. Run pnpm ipc:generate.
export type SendMessageResult = {
    status: string;
    thread_id: string;
    message_id: string;
    queued_id: string | null;
    /**
     * End of the Undo Send window, so a composer popout can offer Undo itself.
     */
    undo_until: string | null;
};
