// Generated from Rust. Run pnpm ipc:generate.
export type MessageFileAttachment = {
    /**
     * Position in the parsed part list — the handle for reading the bytes back.
     */
    index: number;
    filename: string;
    content_type: string;
    size: number;
    content_id: string | null;
    is_inline: boolean;
    /**
     * Inline parts carry their bytes so `cid:` references resolve without a
     * second round trip. Regular attachments stay out of the payload and are
     * fetched on demand by [`read_message_file_attachment`].
     */
    data_url: string | null;
};
