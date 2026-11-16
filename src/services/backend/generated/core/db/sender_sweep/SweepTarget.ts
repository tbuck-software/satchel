// Generated from Rust. Run pnpm ipc:generate.
import type { WorkflowState } from "../../workflow/states/WorkflowState";
/**
 * One thread a sweep would file, with the state it is filed from.
 */
export type SweepTarget = {
    thread_id: string;
    workflow_state: WorkflowState;
    last_message_date: string;
    /**
     * Done marks a thread read; Undo needs to know which ones were not.
     */
    unread_count: number;
};
