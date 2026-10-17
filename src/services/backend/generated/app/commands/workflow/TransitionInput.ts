// Generated from Rust. Run pnpm ipc:generate.
import type { WorkflowState } from "../../../core/workflow/states/WorkflowState";
export type TransitionInput = {
    thread_id: string;
    from: WorkflowState;
    to: WorkflowState;
    snooze_until?: string | null;
};
