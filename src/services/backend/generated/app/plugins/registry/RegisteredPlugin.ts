// Generated from Rust. Run pnpm ipc:generate.
import type { PluginCapability } from "./PluginCapability";
import type { PluginDisabledReason } from "./PluginDisabledReason";
import type { PluginTrustLevel } from "./PluginTrustLevel";
export type RegisteredPlugin = {
    id: string;
    version: string;
    enabled: boolean;
    root_dir: string;
    component_path: string | null;
    api_version: string;
    capabilities: Array<PluginCapability>;
    event_subscriptions: Array<string>;
    ui_slots: Array<string>;
    trust: PluginTrustLevel;
    signature: string | null;
    manifest_errors: Array<string>;
    signature_verified: boolean;
    /**
     * Switched off although the manifest is fine. `enabled` already accounts for it.
     */
    disabled_by: PluginDisabledReason | null;
};
