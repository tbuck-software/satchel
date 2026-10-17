// Generated from Rust. Run pnpm ipc:generate.
import type { PlatformKind } from "../../../core/runtime_capabilities/PlatformKind";
export type RuntimeCapabilitiesDto = {
    platform: PlatformKind;
    mobile: boolean;
    tray: boolean;
    autostart: boolean;
    single_instance: boolean;
    plugins: boolean;
    native_crypto: boolean;
    background_sync: boolean;
    contacts: boolean;
    file_associations: boolean;
    deep_links: boolean;
    push: boolean;
};
