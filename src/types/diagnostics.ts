import type { DiagnosticsAccountDto as WireDiagnosticsAccountDto } from '../services/backend/generated/app/commands/diagnostics/DiagnosticsAccountDto';
import type { DiagnosticsStatusCountDto as WireDiagnosticsStatusCountDto } from '../services/backend/generated/app/commands/diagnostics/DiagnosticsStatusCountDto';
import type { DiagnosticsQueueEntryDto as WireDiagnosticsQueueEntryDto } from '../services/backend/generated/app/commands/diagnostics/DiagnosticsQueueEntryDto';
import type { DiagnosticsQueueDto as WireDiagnosticsQueueDto } from '../services/backend/generated/app/commands/diagnostics/DiagnosticsQueueDto';
import type { DiagnosticsNotificationsDto as WireDiagnosticsNotificationsDto } from '../services/backend/generated/app/commands/diagnostics/DiagnosticsNotificationsDto';
import type { DiagnosticsOutboxStateCountDto as WireDiagnosticsOutboxStateCountDto } from '../services/backend/generated/app/commands/diagnostics/DiagnosticsOutboxStateCountDto';
import type { DiagnosticsOutboxReasonCountDto as WireDiagnosticsOutboxReasonCountDto } from '../services/backend/generated/app/commands/diagnostics/DiagnosticsOutboxReasonCountDto';
import type { DiagnosticsOutboxEntryDto as WireDiagnosticsOutboxEntryDto } from '../services/backend/generated/app/commands/diagnostics/DiagnosticsOutboxEntryDto';
import type { DiagnosticsNotificationOutboxDto as WireDiagnosticsNotificationOutboxDto } from '../services/backend/generated/app/commands/diagnostics/DiagnosticsNotificationOutboxDto';
import type { DiagnosticsSyncLeaseDto as WireDiagnosticsSyncLeaseDto } from '../services/backend/generated/app/commands/diagnostics/DiagnosticsSyncLeaseDto';
import type { DiagnosticsRateLimitDto as WireDiagnosticsRateLimitDto } from '../services/backend/generated/app/commands/diagnostics/DiagnosticsRateLimitDto';
import type { DiagnosticsOverviewDto as WireDiagnosticsOverviewDto } from '../services/backend/generated/app/commands/diagnostics/DiagnosticsOverviewDto';
/**
 * Wire shape of `diagnostics_overview`. snake_case throughout — these are Rust
 * structs serialised verbatim, not the camelCase domain model in `mail.ts`.
 */

export type BackendDiagnosticsAccount = WireDiagnosticsAccountDto;

export type BackendDiagnosticsStatusCount = WireDiagnosticsStatusCountDto;

export type BackendDiagnosticsQueueEntry = WireDiagnosticsQueueEntryDto;

export type BackendDiagnosticsQueue = WireDiagnosticsQueueDto;

/**
 * Outbound: notification quick-actions queued into `sync_queue` under
 * `notification:<digest>` ids. Says nothing about whether a banner was shown —
 * that is `BackendDiagnosticsNotificationOutbox`.
 */
export type BackendDiagnosticsNotifications = WireDiagnosticsNotificationsDto;

export type BackendDiagnosticsOutboxStateCount = WireDiagnosticsOutboxStateCountDto;

/** One snake_case reason code and how often it decided against a banner. */
export type BackendDiagnosticsOutboxReasonCount = WireDiagnosticsOutboxReasonCountDto;

export type BackendDiagnosticsOutboxEntry = WireDiagnosticsOutboxEntryDto;

/**
 * Inbound: one `notification_outbox` row per ingested message, written by the
 * trigger in the same transaction. `reasons_24h` is the histogram over the
 * reason codes of the last day.
 */
export type BackendDiagnosticsNotificationOutbox = WireDiagnosticsNotificationOutboxDto;

export type BackendDiagnosticsSyncLease = WireDiagnosticsSyncLeaseDto;

export type BackendDiagnosticsRateLimit = WireDiagnosticsRateLimitDto;

export type BackendDiagnosticsOverview = WireDiagnosticsOverviewDto;

/** `app_build_info`: the running binary, with the commit `build.rs` embedded. */
export interface BackendBuildInfo {
  version: string;
  commit: string;
  os: string;
  arch: string;
  debug: boolean;
}
