// Generated from Rust. Run pnpm ipc:generate.
/**
 * How far an import has got. Shared by both importers so the view has one
 * progress shape to render.
 */
export type ImportProgress = {
    /**
     * `mbox` or `platform`.
     */
    kind: string;
    done: number;
    skipped: number;
    /**
     * Units expected in total: tables for a platform import, and for an mbox
     * an estimate from the bytes read per message so far. Zero until the
     * first messages have been read.
     */
    total: number;
    bytes_done: number;
    bytes_total: number;
    running: boolean;
    eta_seconds: number | null;
    /**
     * `completed`, `cancelled` or `error`, once the run has ended.
     */
    finished_reason: string | null;
    last_error: string | null;
};
