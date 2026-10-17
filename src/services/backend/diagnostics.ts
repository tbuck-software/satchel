// Auto-split from services/backend.ts — domain module.
import { invoke } from './core';
import type { BackendBuildInfo, BackendDiagnosticsOverview } from '@/types/diagnostics';

export type { BackendBuildInfo, BackendDiagnosticsOverview };


export async function backendDiagnosticsOverview(): Promise<BackendDiagnosticsOverview> {
  return invoke<BackendDiagnosticsOverview>('diagnostics_overview');
}

/** Plain-text report for a bug report, redacted by the backend. */
export async function backendDiagnosticsReport(): Promise<string> {
  return invoke<string>('diagnostics_report');
}

export async function backendAppBuildInfo(): Promise<BackendBuildInfo> {
  return invoke<BackendBuildInfo>('app_build_info');
}

export async function backendOpenLogsFolder(): Promise<void> {
  return invoke<void>('open_logs_folder');
}
