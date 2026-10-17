// Generated from Rust. Run pnpm ipc:generate.
import type { WorkflowState } from "../../../core/workflow/states/WorkflowState";
export type UpdateThreadStateInput = {
    thread_id: string;
    next_state: WorkflowState;
    snooze_until?: string | null;
};
