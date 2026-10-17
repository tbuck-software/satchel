// Generated from Rust. Run pnpm ipc:generate.
import type { WorkflowState } from "../../../core/workflow/states/WorkflowState";
import type { BatchTransitionEntry } from "./BatchTransitionEntry";
export type BatchTransitionInput = {
    threads: Array<BatchTransitionEntry>;
    to: WorkflowState;
    snooze_until?: string | null;
};
