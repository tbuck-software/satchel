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
    bootstrap_pending: false,
    spam_needed: 47,
    ham_needed: 38,
    move_ready: false,
    move_spam_needed: 197,
    move_ham_needed: 188,
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
    expect(wrapper.get('[data-testid="spam-filter-training"]').text()).toBe('3 spam and 12 wanted messages.');
    expect(wrapper.get('[data-testid="spam-filter-readiness"]').text())
      .toBe('It judges new mail once it has learned 47 more spam and 38 more wanted messages.');
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

  it('waits for the bootstrap before anything else', async () => {
    backend.backendGetSpamFilterStatus.mockResolvedValue(status({ bootstrap_pending: true }));
    const wrapper = mount(SpamFilterSettings);
    await flushPromises();
    expect(wrapper.get('[data-testid="spam-filter-readiness"]').text())
      .toBe('It is still learning from the mail you have already sorted, and judges nothing until it is done.');
  });

  it('says Move only marks until the model is large enough', async () => {
    backend.backendGetSpamFilterStatus.mockResolvedValue(status({
      mode: 'move', spam_messages: 60, ham_messages: 200, ready: true, spam_needed: 0, ham_needed: 0,
      move_spam_needed: 140, move_ham_needed: 0,
    }));
    const wrapper = mount(SpamFilterSettings);
    await flushPromises();
    expect(wrapper.get('[data-testid="spam-filter-readiness"]').text())
      .toBe('Until it has learned 140 more spam and 0 more wanted messages, Move only marks.');
    expect(wrapper.text()).toContain('Your server may delete mail in Spam after a while.');
  });

  it('says it judges once every gate is met', async () => {
    backend.backendGetSpamFilterStatus.mockResolvedValue(status({
      spam_messages: 60, ham_messages: 200, ready: true, spam_needed: 0, ham_needed: 0,
    }));
    const wrapper = mount(SpamFilterSettings);
    await flushPromises();
    expect(wrapper.get('[data-testid="spam-filter-readiness"]').text()).toBe('It judges new mail as it arrives.');
  });
});
