// Generated from Rust. Run pnpm ipc:generate.
export type LoadedMessageBody = {
    body_text: string;
    body_html: string | null;
    /**
     * False when the row already had a body and nothing was fetched.
     */
    fetched: boolean;
};
