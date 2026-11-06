// Generated from Rust. Run pnpm ipc:generate.
export type CalendarEventRecord = {
    id: string;
    account_id: string;
    uid: string;
    title: string;
    description: string | null;
    location: string | null;
    starts_at: string;
    ends_at: string;
    all_day: number;
    attendees_json: string;
    source: string;
    collection_href: string | null;
    /**
     * 1 when the provider does not let this account change the event.
     */
    read_only: number;
    /**
     * 1 for one occurrence of a recurring series.
     */
    recurrence_instance: number;
    /**
     * 1 when the description is formatted text the editor cannot keep.
     */
    rich_description: number;
    created_at: string;
    updated_at: string;
};
