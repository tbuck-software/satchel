// Generated from Rust. Run pnpm ipc:generate.
import type { BusyBlock } from "./BusyBlock";
export type AttendeeFreeBusy = {
    /**
     * Lower-case address as it was asked for.
     */
    address: string;
    /**
     * False when nobody would say: no calendar access, a provider without
     * free/busy, an address it does not know, or a failed request.
     */
    known: boolean;
    busy: Array<BusyBlock>;
};
