// Generated from Rust. Run pnpm ipc:generate.
import type { PluginCapability } from "../registry/PluginCapability";
export type PluginAuditRecord = {
    plugin_id: string;
    command: string;
    capability: PluginCapability | null;
    decision: string;
    reason: string | null;
    duration_ms: number | null;
    created_at: string;
};
