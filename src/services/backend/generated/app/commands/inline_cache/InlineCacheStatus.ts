// Generated from Rust. Run pnpm ipc:generate.
import type { InlineCacheProgress } from "./InlineCacheProgress";
export type InlineCacheStatus = {
    /**
     * Inline images without a local copy.
     */
    missing: number;
    /**
     * Their combined size as recorded during sync, in bytes.
     */
    missing_bytes: number;
    cached: number;
    /**
     * Floor for a full run: the pacing delay alone, before any transfer time
     * or rate-limit pause. Enough to name a finishing time before starting.
     */
    estimated_seconds: number;
    progress: InlineCacheProgress;
};
