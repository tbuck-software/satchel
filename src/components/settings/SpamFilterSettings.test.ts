// @vitest-environment jsdom
import { flushPromises, mount } from '@vue/test-utils';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const backend = vi.hoisted(() => ({
  backendGetSpamFilterStatus: vi.fn(),
  backendSetSpamFilterMode: vi.fn(),
}));

vi.mock('@/services/backend', () => backend);

import SpamFilterSettings from './SpamFilterSettings.vue';

function status(overrides: Record<string, unknown> = {}) {
  return {
    mode: 'mark',
    spam_messages: 3,
    ham_messages: 12,
    tokens: 420,
    ready: false,
    min_per_class: 10,
    ...overrides,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  backend.backendGetSpamFilterStatus.mockResolvedValue(status());
  backend.backendSetSpamFilterMode.mockResolvedValue(undefined);
});

describe('spam filter settings', () => {
  it('shows the mode and what the filter has learned', async () => {
    const wrapper = mount(SpamFilterSettings);
    await flushPromises();

    const active = wrapper.find('.segmented__option--active');
    expect(active.text()).toBe('Mark');
    expect(wrapper.get('[data-testid="spam-filter-training"]').text())
      .toBe('3 spam and 12 wanted messages. It judges new mail once it knows 10 of each.');
  });

  it('saves a new mode and keeps the old one when saving fails', async () => {
    const wrapper = mount(SpamFilterSettings);
    await flushPromises();

    const move = wrapper.findAll('.segmented__option').find((option) => option.text() === 'Move to Spam')!;
    await move.trigger('click');
    await flushPromises();
    expect(backend.backendSetSpamFilterMode).toHaveBeenCalledWith('move');
    expect(wrapper.find('.segmented__option--active').text()).toBe('Move to Spam');

    backend.backendSetSpamFilterMode.mockRejectedValueOnce(new Error('nope'));
    const off = wrapper.findAll('.segmented__option').find((option) => option.text() === 'Off')!;
    await off.trigger('click');
    await flushPromises();
    expect(wrapper.find('.segmented__option--active').text()).toBe('Move to Spam');
    expect(wrapper.text()).toContain('nope');
  });

  it('says it is ready once it knows enough of both', async () => {
    backend.backendGetSpamFilterStatus.mockResolvedValue(status({ spam_messages: 40, ham_messages: 200, ready: true }));
    const wrapper = mount(SpamFilterSettings);
    await flushPromises();
    expect(wrapper.get('[data-testid="spam-filter-training"]').text()).toBe('40 spam and 200 wanted messages.');
  });
});
