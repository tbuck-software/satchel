// Generated from Rust. Run pnpm ipc:generate.
export type MessageTranslationInput = {
    message_id: string;
    /**
     * The language to translate into; the webview resolves "follow the UI".
     */
    target_language: string;
};
