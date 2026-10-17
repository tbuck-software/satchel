// Generated from Rust. Run pnpm ipc:generate.
export type OutgoingAttachmentInput = {
    filename: string;
    mime_type: string;
    /**
     * Standard base64 (not URL-safe).
     */
    content_base64: string;
    content_id: string | null;
    inline: boolean;
};
