// Generated from Rust. Run pnpm ipc:generate.
import type { FailedTransitionDto } from "./FailedTransitionDto";
export type BatchTransitionDto = {
    applied: Array<string>;
    failed: Array<FailedTransitionDto>;
};
