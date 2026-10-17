// Generated from Rust. Run pnpm ipc:generate.
import type { PluginCapability } from "../../plugins/registry/PluginCapability";
import type { PluginDisabledReason } from "../../plugins/registry/PluginDisabledReason";
import type { PluginTrustLevel } from "../../plugins/registry/PluginTrustLevel";
export type PluginDto = {
    id: string;
    version: string;
    enabled: boolean;
    api_version: string;
    capabilities: Array<PluginCapability>;
    event_subscriptions: Array<string>;
    ui_slots: Array<string>;
    trust: PluginTrustLevel;
    signature: string | null;
    signature_verified: boolean;
    manifest_errors: Array<string>;
    /**
     * Why a plugin with a valid manifest is off; `None` when on or broken.
     */
    disabled_by: PluginDisabledReason | null;
    granted_capabilities: Array<PluginCapability>;
    /**
     * Capabilities the runtime actually gates a command on. The rest are
     * manifest declarations with no enforcement behind them yet, and the
     * settings UI has to say so rather than offer eight equal switches.
     */
    enforced_capabilities: Array<PluginCapability>;
};
