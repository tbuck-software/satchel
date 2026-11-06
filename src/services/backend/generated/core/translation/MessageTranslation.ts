// Generated from Rust. Run pnpm ipc:generate.
/**
 * A message's body in another language, ready for the reader.
 */
export type MessageTranslation = {
    message_id: string;
    target_language: string;
    engine: string;
    /**
     * What the engine detected; local detection never fills this.
     */
    source_language: string | null;
    /**
     * Sanitized again on every read. `None` when only text was translated
     * (an engine without markup, or HTML too large for one request).
     */
    body_html: string | null;
    body_text: string | null;
    /**
     * True when it came from the local cache and nothing was sent.
     */
    cached: boolean;
    /**
     * RFC 3339; when the engine produced it.
     */
    created_at: string;
};
