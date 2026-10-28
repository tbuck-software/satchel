// @vitest-environment jsdom
import { mount } from '@vue/test-utils';
import { describe, expect, it, vi } from 'vitest';

const harness = vi.hoisted(() => {
  const thread = (id: string, name: string, subject: string) => ({
    id, subject, participants: [{ name, email: `${id}@example.com` }],
  });
  const threads = Array.from({ length: 8 }, (_, index) => thread(`t${index}`, `Sender ${index}`, `Subject ${index}`));
  return { threads, selected: threads.map((item) => item.id), clearSelection: vi.fn() };
});
vi.mock('@/stores/threads', () => ({ useThreadsStore: () => ({
  selectedThreadIds: harness.selected,
  find: (id: string) => harness.threads.find((item) => item.id === id),
  clearSelection: harness.clearSelection,
}) }));
vi.mock('@/composables/useShortcutHints', () => ({ useShortcutHints: () => ({ hint: () => ({ label: 'c' }) }) }));
import ThreadSelectionSummary from './ThreadSelectionSummary.vue';

describe('selection summary', () => {
  it('names what the toolbar acts on, six at most, and clears the selection', async () => {
    const wrapper = mount(ThreadSelectionSummary, { global: { stubs: { Avatar: true } } });

    expect(wrapper.get('h1').text()).toBe('8 conversations selected');
    expect(wrapper.findAll('.selection-summary__item')).toHaveLength(6);
    expect(wrapper.get('.selection-summary__more').text()).toBe('and 2 more');
    await wrapper.get('button').trigger('click');
    expect(harness.clearSelection).toHaveBeenCalledTimes(1);
  });
});
