// Generated from Rust. Run pnpm ipc:generate.
import type { PushDevice } from "./PushDevice";
export type PushDevices = {
    devices: Array<PushDevice>;
    phase: string;
    command: string | null;
    candidate: string | null;
};
