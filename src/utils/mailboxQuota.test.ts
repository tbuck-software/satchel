import { describe, expect, it } from 'vitest';
import type { BackendAccountQuota } from '@/services/backend';
import {
  QUOTA_CRITICAL_RATIO,
  QUOTA_WARNING_RATIO,
  formatQuotaAmounts,
  formatQuotaPercent,
  fullestQuotaFigure,
  quotaFigures,
  quotaLevel,
  quotaLevelForRatio,
} from './mailboxQuota';

/** Intl separates number and unit with a no-break space; compare it as a space. */
const plain = (text: string) => text.replace(/[\u00a0\u202f]/g, ' ');
const plainAmounts = (amounts: { used: string; limit: string }) => ({ used: plain(amounts.used), limit: plain(amounts.limit) });

function quota(change: Partial<BackendAccountQuota> = {}): BackendAccountQuota {
  return {
    account_id: 'acc',
    storage_used_bytes: null,
    storage_limit_bytes: null,
    messages_used: null,
    messages_limit: null,
    fetched_at: '2026-11-05T08:00:00Z',
    ...change,
  };
}

describe('mailbox quota thresholds', () => {
  it('warns from 90 % and turns critical from 98 %', () => {
    expect(QUOTA_WARNING_RATIO).toBe(0.9);
    expect(QUOTA_CRITICAL_RATIO).toBe(0.98);
    expect(quotaLevelForRatio(0.8999)).toBe('ok');
    expect(quotaLevelForRatio(0.9)).toBe('warning');
    expect(quotaLevelForRatio(0.9799)).toBe('warning');
    expect(quotaLevelForRatio(0.98)).toBe('critical');
    expect(quotaLevelForRatio(1.2)).toBe('critical');
  });

  it('knows nothing without a limit, rather than calling it empty', () => {
    expect(quotaLevel(null)).toBeNull();
    expect(quotaLevel(quota())).toBeNull();
    expect(quotaLevel(quota({ storage_used_bytes: 5, storage_limit_bytes: 0 }))).toBeNull();
    expect(quotaFigures(quota({ storage_used_bytes: 5, storage_limit_bytes: null }))).toEqual([]);
  });

  it('judges by the fuller resource', () => {
    const reading = quota({
      storage_used_bytes: 10, storage_limit_bytes: 100,
      messages_used: 990, messages_limit: 1000,
    });
    expect(quotaFigures(reading).map(entry => entry.kind)).toEqual(['storage', 'messages']);
    expect(fullestQuotaFigure(reading)?.kind).toBe('messages');
    expect(quotaLevel(reading)).toBe('critical');
  });

  it('works from messages alone when the server sets no storage limit', () => {
    expect(quotaLevel(quota({ messages_used: 920, messages_limit: 1000 }))).toBe('warning');
  });

  it('rounds the percent down so it never claims a level not reached', () => {
    expect(formatQuotaPercent(0.979, 'en')).toBe('97%');
    expect(plain(formatQuotaPercent(0.979, 'de'))).toBe('97 %');
    expect(formatQuotaPercent(1.5, 'en')).toBe('150%');
  });

  it('keeps whole percentages whole despite float error', () => {
    expect(formatQuotaPercent(0.29, 'en')).toBe('29%');
    expect(formatQuotaPercent(0.57, 'en')).toBe('57%');
    expect(formatQuotaPercent(0.58, 'en')).toBe('58%');
    expect(formatQuotaPercent(0.9, 'en')).toBe('90%');
    expect(formatQuotaPercent(0.98, 'en')).toBe('98%');
  });

  it('formats storage as bytes and messages as counts, per locale', () => {
    const storage = quotaFigures(quota({ storage_used_bytes: 1_250_000_000, storage_limit_bytes: 2_000_000_000 }))[0]!;
    expect(plainAmounts(formatQuotaAmounts(storage, 'en'))).toEqual({ used: '1.3 GB', limit: '2 GB' });
    expect(plainAmounts(formatQuotaAmounts(storage, 'de'))).toEqual({ used: '1,3 GB', limit: '2 GB' });
    const messages = quotaFigures(quota({ messages_used: 12_345, messages_limit: 50_000 }))[0]!;
    expect(formatQuotaAmounts(messages, 'de')).toEqual({ used: '12.345', limit: '50.000' });
  });
});
