// Generated from Rust. Run pnpm ipc:generate.
export type TaskRecord = {
    id: string;
    list_id: string;
    title: string;
    notes: string | null;
    /**
     * `YYYY-MM-DD` or RFC 3339 UTC.
     */
    due_at: string | null;
    /**
     * RFC 3339 UTC.
     */
    reminder_at: string | null;
    /**
     * 0 none, 1 highest … 9 lowest (iCalendar).
     */
    priority: number;
    completed_at: string | null;
    /**
     * `simple`: completing moves the due date to the next occurrence;
     * `complex`: completing ends the series, so the UI asks first.
     */
    recurrence: string | null;
    /**
     * The thread the task was made from, re-found by Message-ID when the
     * stored id no longer exists.
     */
    thread_id: string | null;
    message_id: string | null;
    /**
     * A change that has not reached the server yet.
     */
    pending: boolean;
    created_at: string;
    updated_at: string;
};
