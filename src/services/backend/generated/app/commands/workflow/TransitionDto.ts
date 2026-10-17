// Generated from Rust. Run pnpm ipc:generate.
import type { WorkflowState } from "../../../core/workflow/states/WorkflowState";
export type TransitionDto = {
    thread_id: string;
    from: WorkflowState;
    to: WorkflowState;
    at: string;
};
