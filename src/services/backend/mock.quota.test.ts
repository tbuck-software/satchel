// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { BackendAccountQuota } from './accounts';
import { quotaLevel } from '@/utils/mailboxQuota';

// The columns `list_account_quotas` serialises (AccountQuotaDto in Rust).
const DTO_KEYS = ['account_id', 'fetched_at', 'messages_limit', 'messages_used', 'storage_limit_bytes', 'storage_used_bytes'];

describe('mock account quotas', () => {
  beforeEach(() => { vi.resetModules(); window.history.replaceState({}, '', '/'); });
  afterEach(() => { window.history.replaceState({}, '', '/'); });

  it('answers in the shape of the Rust command, only for accounts with a limit', async () => {
    const { sendCommand } = await import('./mock');
    const rows = await sendCommand<BackendAccountQuota[]>('list_account_quotas');
    expect(rows.map(row => row.account_id)).toEqual(['acc-work']);
    expect(Object.keys(rows[0]!).sort()).toEqual(DTO_KEYS);
    expect(rows.every(row => row.storage_limit_bytes !== null || row.messages_limit !== null)).toBe(true);
    expect(quotaLevel(rows[0])).toBe('ok');
  });

  it('fills the work mailbox on request, to show the sidebar marker', async () => {
    window.history.replaceState({}, '', '/?mockQuota=critical');
    const { sendCommand } = await import('./mock');
    const [row] = await sendCommand<BackendAccountQuota[]>('list_account_quotas');
    expect(quotaLevel(row)).toBe('critical');
  });

  it('hands out copies, not its own state', async () => {
    const { sendCommand } = await import('./mock');
    const [row] = await sendCommand<BackendAccountQuota[]>('list_account_quotas');
    row!.storage_used_bytes = 0;
    const [again] = await sendCommand<BackendAccountQuota[]>('list_account_quotas');
    expect(again!.storage_used_bytes).not.toBe(0);
  });
});
