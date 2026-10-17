// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import UnsubscribeBanner from './UnsubscribeBanner.vue';

const unsubscribe = vi.fn();
const toastError = vi.fn();
const toastSuccess = vi.fn();

vi.mock('@/stores/messages', () => ({
  useMessagesStore: () => ({ unsubscribe }),
}));

vi.mock('@/composables/useToast', () => ({
  useToast: () => ({ error: toastError, success: toastSuccess }),
}));

describe('unsubscribe banner', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it('names the refused target instead of showing the serialized error', async () => {
    unsubscribe.mockRejectedValue(
      new Error(JSON.stringify({ key: 'backend.errors.unsubscribeTargetBlocked' })),
    );
    const wrapper = mount(UnsubscribeBanner, { props: { messageId: 'msg-1' } });

    await wrapper.get('button.primary').trigger('click');
    await Promise.resolve();

    expect(toastError).toHaveBeenCalledTimes(1);
    const message = String(toastError.mock.calls[0]?.[0]);
    expect(message).toContain('not publicly reachable');
    expect(message).not.toContain('backend.errors');
  });

  it('reports the route a successful unsubscribe took', async () => {
    unsubscribe.mockResolvedValue({ method: 'mailto', target: 'leave@list.example' });
    const wrapper = mount(UnsubscribeBanner, { props: { messageId: 'msg-1' } });

    await wrapper.get('button.primary').trigger('click');
    await Promise.resolve();

    expect(toastSuccess).toHaveBeenCalledWith('Unsubscribed via mailto');
  });
});
