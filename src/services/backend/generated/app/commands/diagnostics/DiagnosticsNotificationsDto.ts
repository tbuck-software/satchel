// Generated from Rust. Run pnpm ipc:generate.
import type { DiagnosticsQueueEntryDto } from "./DiagnosticsQueueEntryDto";
import type { DiagnosticsStatusCountDto } from "./DiagnosticsStatusCountDto";
export type DiagnosticsNotificationsDto = {
    by_status: Array<DiagnosticsStatusCountDto>;
    recent: Array<DiagnosticsQueueEntryDto>;
};
