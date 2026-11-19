// Generated from Rust. Run pnpm ipc:generate.
import type { Nearby } from "./Nearby";
import type { TransferPhase } from "./TransferPhase";
export type TransferStatus = {
    phase: TransferPhase;
    code: string | null;
    address: string | null;
    nearby: Array<Nearby>;
    emails: Array<string>;
    sign_in: Array<string>;
    created: number;
};
