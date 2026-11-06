// Generated from Rust. Run pnpm ipc:generate.
/**
 * A language an engine translates into.
 */
export type TranslationLanguage = {
    code: string;
    /**
     * The engine's own (English) name; the webview prefers the name its
     * locale gives the code.
     */
    name: string | null;
};
