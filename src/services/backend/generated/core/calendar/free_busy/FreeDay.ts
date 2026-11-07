// Generated from Rust. Run pnpm ipc:generate.
import type { FreeWindow } from "./FreeWindow";
export type FreeDay = {
    /**
     * `YYYY-MM-DD` in `FreeSlotsReport::time_zone`.
     */
    date: string;
    windows: Array<FreeWindow>;
};
