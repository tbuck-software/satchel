// Generated from Rust. Run pnpm ipc:generate.
import type { DiagnosticsOutboxEntryDto } from "./DiagnosticsOutboxEntryDto";
import type { DiagnosticsOutboxReasonCountDto } from "./DiagnosticsOutboxReasonCountDto";
import type { DiagnosticsOutboxStateCountDto } from "./DiagnosticsOutboxStateCountDto";
export type DiagnosticsNotificationOutboxDto = {
    by_state: Array<DiagnosticsOutboxStateCountDto>;
    reasons_24h: Array<DiagnosticsOutboxReasonCountDto>;
    recent: Array<DiagnosticsOutboxEntryDto>;
};
