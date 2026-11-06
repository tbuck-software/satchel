// Generated from Rust. Run pnpm ipc:generate.
import type { TaskSyncSummary } from "../../tasks/sync/TaskSyncSummary";
export type GroupwareSyncSummary = {
    accounts_checked: number;
    calendars_found: number;
    events_synced: number;
    address_books_found: number;
    contacts_synced: number;
    task_lists_found: number;
    tasks_synced: number;
    /**
     * What the task sync did, for the frontend's notices.
     */
    task_summary: TaskSyncSummary;
    errors: Array<string>;
};
