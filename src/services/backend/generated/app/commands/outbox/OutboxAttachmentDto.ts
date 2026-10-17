// Generated from Rust. Run pnpm ipc:generate.
/**
 * What a queued attachment costs, without its bytes. The list is a projection;
 * carrying every base64 payload through it would be megabytes per refresh.
 */
export type OutboxAttachmentDto = {
    filename: string;
    mime_type: string;
    size: number;
};
