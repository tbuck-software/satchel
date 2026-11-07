// Generated from Rust. Run pnpm ipc:generate.
import type { EditingEventInput } from "./EditingEventInput";
import type { WorkingHoursInput } from "./WorkingHoursInput";
export type FreeSlotsInput = {
    account_id?: string | null;
    attendees: Array<string>;
    /**
     * RFC 3339; nothing earlier is offered.
     */
    from: string;
    working_days: number;
    duration_minutes: number;
    working_hours: WorkingHoursInput;
    time_zone: string;
    excluded_calendars: Array<string>;
    editing?: EditingEventInput | null;
};
