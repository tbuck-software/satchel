// Generated from Rust. Run pnpm ipc:generate.
import type { EditingEventInput } from "./EditingEventInput";
export type FreeBusyInput = {
    /**
     * The account the event belongs to; it is asked about the attendees.
     */
    account_id?: string | null;
    attendees: Array<string>;
    /**
     * RFC 3339 bounds, end exclusive.
     */
    starts_at: string;
    ends_at: string;
    /**
     * IANA zone whole-day events are placed in.
     */
    time_zone: string;
    /**
     * Calendar keys that do not count towards the owner's availability.
     */
    excluded_calendars: Array<string>;
    editing?: EditingEventInput | null;
};
