// Generated from Rust. Run pnpm ipc:generate.
export type TaskFieldsInput = {
    title: string;
    notes?: string | null;
    /**
     * `YYYY-MM-DD` or an RFC 3339 instant.
     */
    due_at?: string | null;
    /**
     * RFC 3339 instant.
     */
    reminder_at?: string | null;
    /**
     * 0 none, 1 high, 5 medium, 9 low (iCalendar scale).
     */
    priority: number;
    completed_at?: string | null;
};
