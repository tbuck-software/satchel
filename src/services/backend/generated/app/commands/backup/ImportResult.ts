// Generated from Rust. Run pnpm ipc:generate.
export type ImportResult = {
    path: string;
    message_count: number;
    skipped: number;
    /**
     * Set when the user stopped the run; the messages counted above are in.
     */
    cancelled: boolean;
};
