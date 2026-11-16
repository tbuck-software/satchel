// Generated from Rust. Run pnpm ipc:generate.
import type { FreeDay } from "./FreeDay";
export type FreeSlotsReport = {
    /**
     * The IANA zone the days and working hours were read in.
     */
    time_zone: string;
    days: Array<FreeDay>;
    /**
     * Attendees whose time could not be taken into account.
     */
    unknown: Array<string>;
};
