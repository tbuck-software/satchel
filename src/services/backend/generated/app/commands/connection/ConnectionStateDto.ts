// Generated from Rust. Run pnpm ipc:generate.
export type ConnectionStateDto = {
    /**
     * Native code does not infer internet reachability. The webview uses its OS network hint.
     */
    online: boolean | null;
    syncing: boolean;
    last_sync_at: string | null;
    pending_outbox: number;
    /**
     * Subset of `pending_outbox` that is mail waiting to go out, not a queued
     * archive/flag mutation. The status line needs the two apart.
     */
    pending_sends: number;
    failed_outbox: number;
    failed_sends: number;
    failed_remote_mutations: number;
};
