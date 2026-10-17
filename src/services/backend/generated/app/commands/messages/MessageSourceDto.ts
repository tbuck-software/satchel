// Generated from Rust. Run pnpm ipc:generate.
export type MessageSourceDto = {
    message_id: string;
    subject: string | null;
    /**
     * Raw RFC 822 header block as received. Populated after an original-source
     * fetch, or absent when the provider cannot supply the message anymore.
     */
    raw_headers: string | null;
    body_text: string | null;
    body_html: string | null;
    raw_size: number | null;
    /**
     * Complete RFC 822/MIME source fetched explicitly from the provider.
     */
    raw_source: string | null;
    /**
     * `loaded`, `unavailable` or `unsupported`.
     */
    raw_source_status: string;
};
