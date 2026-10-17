// Generated from Rust. Run pnpm ipc:generate.
import type { PluginCapability } from "../../plugins/registry/PluginCapability";
export type PluginPermissionInput = {
    plugin_id: string;
    capability: PluginCapability;
    granted: boolean;
};
