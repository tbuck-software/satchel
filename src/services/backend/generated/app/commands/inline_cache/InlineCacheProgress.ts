// Generated from Rust. Run pnpm ipc:generate.
export type InlineCacheProgress = {
    /**
     * Attachments the run started with.
     */
    total: number;
    done: number;
    failed: number;
    running: boolean;
    /**
     * Estimated seconds left, once a rate can be measured.
     */
    eta_seconds: number | null;
    /**
     * Set while waiting out a provider rate limit, so the UI can say why
     * nothing is moving.
     */
    paused_seconds: number | null;
    last_error: string | null;
    /**
     * Set once the run ends on its own terms rather than by user request.
     */
    finished_reason: string | null;
};
