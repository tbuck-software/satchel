// Generated from Rust. Run pnpm ipc:generate.
export type PluginMetrics = {
    plugin_id: string;
    calls: number;
    allowed: number;
    denied: number;
    errors: number;
    timeouts: number;
    total_duration_ms: number;
    last_error: string | null;
    last_called_at: string | null;
};
