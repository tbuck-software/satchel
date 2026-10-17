// Generated from Rust. Run pnpm ipc:generate.
import type { DiagnosticsAccountDto } from "./DiagnosticsAccountDto";
import type { DiagnosticsNotificationOutboxDto } from "./DiagnosticsNotificationOutboxDto";
import type { DiagnosticsNotificationsDto } from "./DiagnosticsNotificationsDto";
import type { DiagnosticsQueueDto } from "./DiagnosticsQueueDto";
import type { DiagnosticsRateLimitDto } from "./DiagnosticsRateLimitDto";
import type { DiagnosticsSyncLeaseDto } from "./DiagnosticsSyncLeaseDto";
export type DiagnosticsOverviewDto = {
    generated_at: string;
    accounts: Array<DiagnosticsAccountDto>;
    queue: DiagnosticsQueueDto;
    notifications: DiagnosticsNotificationsDto;
    notification_outbox: DiagnosticsNotificationOutboxDto;
    sync_leases: Array<DiagnosticsSyncLeaseDto>;
    rate_limits: Array<DiagnosticsRateLimitDto>;
};
