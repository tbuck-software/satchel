import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { BackendDraft } from '@/services/backend';

const harness = vi.hoisted(() => ({
  items: [] as BackendDraft[],
  fetchOne: vi.fn(),
  remove: vi.fn(),
  restore: vi.fn(),
  info: vi.fn(),
  success: vi.fn(),
  error: vi.fn(),
}));

vi.mock('@/i18n/runtime', () => ({ t: (key: string) => key }));
vi.mock('@/stores/drafts', () => ({
  useDraftsStore: () => ({
    get items() { return harness.items; },
    fetchOne: harness.fetchOne,
    remove: harness.remove,
    restore: harness.restore,
  }),
}));
vi.mock('@/composables/useToast', () => ({
  useToast: () => ({ info: harness.info, success: harness.success, error: harness.error }),
}));

import { useDraftActions } from './useDraftActions';

function draft(body: string): BackendDraft {
  return {
    id: 'draft-1',
    account_id: 'account-1',
    to: [], cc: [], bcc: [],
    subject: 'Plan',
    body,
    attachments: [],
    updated_at: '2026-10-19T10:00:00Z',
  } as BackendDraft;
}

async function discardAndUndo() {
  await expect(useDraftActions().discard('draft-1')).resolves.toBe(true);
  const options = harness.info.mock.calls.at(-1)?.[1] as { action?: { handler: () => void } };
  options.action?.handler();
  await Promise.resolve();
  return harness.restore.mock.calls.at(-1)?.[0] as BackendDraft | undefined;
}

describe('useDraftActions undo snapshot', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    harness.remove.mockResolvedValue(undefined);
    harness.restore.mockResolvedValue(null);
  });

  it('restores the version on disk, not the list row loaded at startup', async () => {
    // A popout saved "v2"; the drafts list still holds "v1".
    harness.items = [draft('v1')];
    harness.fetchOne.mockResolvedValue({ ...draft('v2'), remote_error: 'provider save delayed' });

    expect((await discardAndUndo())?.body).toBe('v2');
  });

  it('offers no Undo when a fresh read fails, even with cached content', async () => {
    harness.items = [draft('v1')];
    harness.fetchOne.mockRejectedValue(new Error('offline'));

    expect(await discardAndUndo()).toBeUndefined();
    expect(harness.info.mock.calls.at(-1)?.[1].action).toBeUndefined();
  });
  it.each([null, { ...draft(''), content_error: 'backend.errors.imapUidValidityMissing' }])(
    'does not use cached content or an unavailable payload as an Undo snapshot', async (fresh) => {
      harness.items = [draft('cached secret')];
      harness.fetchOne.mockResolvedValue(fresh);
      expect(await discardAndUndo()).toBeUndefined();
      expect(harness.remove).toHaveBeenCalledWith('draft-1');
      expect(harness.info.mock.calls.at(-1)?.[1].action).toBeUndefined();
    },
  );

});
