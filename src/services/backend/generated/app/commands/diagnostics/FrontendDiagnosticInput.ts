// Generated from Rust. Run pnpm ipc:generate.
export type FrontendDiagnosticInput = {
    event: string;
    detail: string;
    cache_threads: number;
    cached_messages: number;
    body_chars: number;
    dom_nodes: number;
    iframe_count: number;
    rendered_mail_count: number;
    js_heap_bytes: number;
    /**
     * `error` and `warn` land at that level, so the copied report picks them up.
     */
    level?: string | null;
};
