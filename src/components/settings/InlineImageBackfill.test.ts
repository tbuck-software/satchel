// @vitest-environment jsdom
import { flushPromises, mount } from '@vue/test-utils';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const backend = vi.hoisted(() => ({
  backendInlineImageCacheStatus: vi.fn(),
  backendStartInlineImageBackfill: vi.fn(),
  backendStopInlineImageBackfill: vi.fn(),
  backendSupportsNativeCommands: vi.fn(() => true),
}));

vi.mock('@/services/backend', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@/services/backend')>()),
  ...backend,
}));
vi.mock('@/services/appEvents', () => ({
  listenToBackend: vi.fn(async () => () => {}),
}));

import InlineImageBackfill from './InlineImageBackfill.vue';

const DECRYPTED_KEY = 'backend.errors.decryptedAttachmentUnavailable';

function status(lastError: string | null) {
  return {
    missing: 3,
    missing_bytes: 3000,
    cached: 1,
    estimated_seconds: 1,
    progress: {
      running: false,
      total: 3,
      done: 2,
      failed: 1,
      eta_seconds: null,
      paused_seconds: null,
      last_error: lastError,
      finished_reason: 'completed',
    },
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  backend.backendSupportsNativeCommands.mockReturnValue(true);
});

describe('inline image backfill errors', () => {
  it('shows the last item error as a sentence, not its message key', async () => {
    backend.backendInlineImageCacheStatus.mockResolvedValue(status(DECRYPTED_KEY));
    const wrapper = mount(InlineImageBackfill);
    await flushPromises();

    const error = wrapper.get('.hint.error').text();
    expect(error).toContain('This attachment comes from an encrypted message');
    expect(error).not.toContain(DECRYPTED_KEY);
  });

  it('keeps a provider message that is not a key as it is', async () => {
    backend.backendInlineImageCacheStatus.mockResolvedValue(status('HTTP 503 from upstream'));
    const wrapper = mount(InlineImageBackfill);
    await flushPromises();

    expect(wrapper.get('.hint.error').text()).toBe('HTTP 503 from upstream');
  });

  it('translates a refused start', async () => {
    backend.backendInlineImageCacheStatus.mockResolvedValue(status(null));
    backend.backendStartInlineImageBackfill.mockRejectedValue({
      key: 'backend.errors.inlineBackfillAlreadyRunning',
    });
    const wrapper = mount(InlineImageBackfill);
    await flushPromises();

    const start = wrapper.findAll('button').find((button) => button.text().includes('Fetch now'));
    expect(start).toBeDefined();
    await start!.trigger('click');
    await flushPromises();

    expect(wrapper.get('.hint.error').text()).toBe(
      'The inline image backfill is already running.',
    );
  });
});
