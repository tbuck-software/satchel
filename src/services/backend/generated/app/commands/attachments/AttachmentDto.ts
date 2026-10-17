// Generated from Rust. Run pnpm ipc:generate.
export type AttachmentDto = {
    id: string;
    message_id: string;
    filename: string | null;
    content_type: string;
    size: number | null;
    content_id: string | null;
    is_inline: number;
    cached_path: string | null;
};
