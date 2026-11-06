// Generated from Rust. Run pnpm ipc:generate.
import type { AttendeeFreeBusy } from "./AttendeeFreeBusy";
import type { BusyBlock } from "./BusyBlock";
export type FreeBusyReport = {
    /**
     * The account owner's busy time across every calendar that counts.
     */
    own: Array<BusyBlock>;
    attendees: Array<AttendeeFreeBusy>;
};
