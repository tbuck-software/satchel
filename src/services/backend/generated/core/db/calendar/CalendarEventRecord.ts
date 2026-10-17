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
    created_at: string;
    updated_at: string;
};
