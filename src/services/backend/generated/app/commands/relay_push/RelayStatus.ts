// Generated from Rust. Run pnpm ipc:generate.
import type { PushDevice } from "../background_sync/PushDevice";
export type RelayStatus = {
    phase: string;
    devices: Array<PushDevice>;
    nearby: Array<PushDevice>;
    code: string | null;
    request: string | null;
    name: string | null;
};
