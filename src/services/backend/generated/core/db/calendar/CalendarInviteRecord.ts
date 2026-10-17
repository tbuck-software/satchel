// Generated from Rust. Run pnpm ipc:generate.
export type CalendarInviteRecord = {
    id: string;
    message_id: string | null;
    account_id: string;
    uid: string;
    organizer_email: string | null;
    organizer_name: string | null;
    summary: string | null;
    description: string | null;
    location: string | null;
    starts_at: string | null;
    ends_at: string | null;
    all_day: number;
    rsvp_status: string;
    raw_ics: string;
    sequence: number | null;
    method: string | null;
    cancelled: number;
    recurrence_id: string | null;
    recurring: number;
    recurrence_unbounded: number;
    created_at: string;
    updated_at: string;
};
