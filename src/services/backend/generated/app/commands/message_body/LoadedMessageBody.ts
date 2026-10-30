// Generated from Rust. Run pnpm ipc:generate.
export type LoadedMessageBody = {
    remote_uidvalidity?: number | null;
    body_text: string;
    body_html: string | null;
    /**
     * False when the row already had a body and nothing was fetched.
     */
    fetched: boolean;
};
