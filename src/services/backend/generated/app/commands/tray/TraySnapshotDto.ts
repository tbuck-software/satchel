// Generated from Rust. Run pnpm ipc:generate.
import type { TrayCalendarEventDto } from "./TrayCalendarEventDto";
import type { TrayThreadDto } from "./TrayThreadDto";
export type TraySnapshotDto = {
    new_threads: Array<TrayThreadDto>;
    upcoming_events: Array<TrayCalendarEventDto>;
};
