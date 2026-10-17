// Generated from Rust. Run pnpm ipc:generate.
export type OutgoingAttachment = {
    filename: string;
    mime_type: string;
    /**
     * Base64-encoded content (standard alphabet, no URL-safe).
     */
    content_base64: string;
    content_id: string | null;
    inline: boolean;
};
