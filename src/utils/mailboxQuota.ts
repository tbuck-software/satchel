import type { BackendAccountQuota } from '@/services/backend';
import { formatBytes } from '@/utils/formatBytes';
import { currentLocale } from '@/i18n/runtime';

/** From here the sidebar marks the account: mail may start bouncing soon. */
export const QUOTA_WARNING_RATIO = 0.9;
/** From here the marker turns red: the next large message may already bounce. */
export const QUOTA_CRITICAL_RATIO = 0.98;

export type QuotaLevel = 'ok' | 'warning' | 'critical';

export interface QuotaFigure {
  kind: 'storage' | 'messages';
  used: number;
  limit: number;
  /** used / limit, not capped: a server can report usage above its limit. */
  ratio: number;
}

function figure(kind: QuotaFigure['kind'], used: number | null, limit: number | null): QuotaFigure | null {
  if (used === null || limit === null || limit <= 0) return null;
  const clamped = Math.max(0, used);
  return { kind, used: clamped, limit, ratio: clamped / limit };
}

/** Storage first, then messages; only resources the server limits. */
export function quotaFigures(quota: BackendAccountQuota | null | undefined): QuotaFigure[] {
  if (!quota) return [];
  return [
    figure('storage', quota.storage_used_bytes, quota.storage_limit_bytes),
    figure('messages', quota.messages_used, quota.messages_limit),
  ].filter((entry): entry is QuotaFigure => entry !== null);
}

/** The resource closest to its limit: the one the next message bounces off. */
export function fullestQuotaFigure(quota: BackendAccountQuota | null | undefined): QuotaFigure | null {
  return quotaFigures(quota).reduce<QuotaFigure | null>(
    (fullest, entry) => (!fullest || entry.ratio > fullest.ratio ? entry : fullest),
    null,
  );
}

export function quotaLevelForRatio(ratio: number): QuotaLevel {
  if (ratio >= QUOTA_CRITICAL_RATIO) return 'critical';
  if (ratio >= QUOTA_WARNING_RATIO) return 'warning';
  return 'ok';
}

/** `null` when nothing is known: no quota is not the same as an empty one. */
export function quotaLevel(quota: BackendAccountQuota | null | undefined): QuotaLevel | null {
  const fullest = fullestQuotaFigure(quota);
  return fullest ? quotaLevelForRatio(fullest.ratio) : null;
}

/**
 * Whole percent, rounded down, so the number never claims a threshold the
 * level has not reached: 97.9 % reads "97 %" while it is still a warning.
 */
export function formatQuotaPercent(ratio: number, locale: string = currentLocale()): string {
  const whole = Math.floor(Math.max(0, ratio) * 100) / 100;
  return new Intl.NumberFormat(locale, { style: 'percent', maximumFractionDigits: 0 }).format(whole);
}

/** The used and limit amounts of a figure: bytes for storage, a count for messages. */
export function formatQuotaAmounts(entry: QuotaFigure, locale: string = currentLocale()): { used: string; limit: string } {
  if (entry.kind === 'storage') return { used: formatBytes(entry.used, locale), limit: formatBytes(entry.limit, locale) };
  const count = new Intl.NumberFormat(locale);
  return { used: count.format(entry.used), limit: count.format(entry.limit) };
}
