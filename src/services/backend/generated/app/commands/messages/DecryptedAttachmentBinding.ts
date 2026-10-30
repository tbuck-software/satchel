// Generated from Rust. Run pnpm ipc:generate.
export type DecryptedAttachmentBinding = {
    id: string;
    filename: string;
    content_type: string;
    size: number;
    content_id: string | null;
    is_inline: boolean;
    cached_path: string | null;
    data_url: string | null;
};
