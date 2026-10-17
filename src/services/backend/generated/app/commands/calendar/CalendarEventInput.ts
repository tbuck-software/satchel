// Generated from Rust. Run pnpm ipc:generate.
export type CalendarEventInput = {
    id?: string | null;
    account_id: string;
    title: string;
    description?: string | null;
    location?: string | null;
    starts_at: string;
    ends_at: string;
    all_day: boolean;
    attendees: Array<string>;
    send_invites: boolean;
};
