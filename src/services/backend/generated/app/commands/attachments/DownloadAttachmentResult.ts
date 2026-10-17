// Generated from Rust. Run pnpm ipc:generate.
export type DownloadAttachmentResult = {
    filename: string;
    mime_type: string;
    /**
     * Base64 payload, only up to [`MAX_BASE64_PAYLOAD_BYTES`]. Above that it
     * is `None` and the caller reads `cached_path`, which is always set.
     */
    data_base64: string | null;
    cached_path: string | null;
};
