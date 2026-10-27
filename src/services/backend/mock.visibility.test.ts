// @vitest-environment jsdom
import { reactive } from 'vue';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { BackendAccountVisibilityPolicy, BackendAccountVisibilityStatus } from './accounts';
import type { BackendThreadPage } from './threads';

const schedule: BackendAccountVisibilityPolicy = {
  mode: 'schedule', weekdays: [1, 3], from: '09:00', to: '17:00', override_date: null, override_visible: null,
};

describe('mock account visibility transport', () => {
  beforeEach(() => {
    vi.resetModules();
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2026, 9, 26, 8, 59, 59));
    localStorage.clear();
    window.history.replaceState({}, '', '/');
  });
  afterEach(() => { vi.useRealTimers(); localStorage.clear(); });

  it('honors weekdays and the inclusive start / exclusive end before paging', async () => {
    const { sendCommand } = await import('./mock');
    await sendCommand('set_account_visibility', { accountId: 'acc-work', policy: reactive({ ...schedule, weekdays: [...schedule.weekdays] }) });
    const visible = async () => (await sendCommand<BackendAccountVisibilityStatus[]>('get_account_visibility'))[0]!.visible;
    expect(await visible()).toBe(false);
    const hidden = await sendCommand<BackendThreadPage>('list_threads', { input: { limit: 1 } });
    expect(hidden.threads).toHaveLength(0);
    expect(hidden.has_more).toBe(false);
    const direct = await sendCommand<BackendThreadPage>('list_threads', { input: { account_id: 'acc-work', limit: 1 } });
    expect(direct.threads).toHaveLength(1);
    vi.setSystemTime(new Date(2026, 9, 26, 9)); expect(await visible()).toBe(true);
    vi.setSystemTime(new Date(2026, 9, 26, 16, 59, 59)); expect(await visible()).toBe(true);
    vi.setSystemTime(new Date(2026, 9, 26, 17)); expect(await visible()).toBe(false);
    vi.setSystemTime(new Date(2026, 9, 27, 12)); expect(await visible()).toBe(false);
    vi.setSystemTime(new Date(2026, 9, 28, 12)); expect(await visible()).toBe(true);
  });

  it('persists a peek through reload and expires it at local midnight', async () => {
    let { sendCommand } = await import('./mock');
    await sendCommand('set_account_visibility', { accountId: 'acc-work', policy: {
      ...schedule, mode: 'manual', override_date: '2026-10-26', override_visible: true,
    } });
    vi.resetModules();
    ({ sendCommand } = await import('./mock'));
    vi.setSystemTime(new Date(2026, 9, 26, 23, 59, 59));
    expect((await sendCommand<BackendAccountVisibilityStatus[]>('get_account_visibility'))[0]!.visible).toBe(true);
    vi.setSystemTime(new Date(2026, 9, 27));
    expect((await sendCommand<BackendAccountVisibilityStatus[]>('get_account_visibility'))[0]!.visible).toBe(false);
  });
});
