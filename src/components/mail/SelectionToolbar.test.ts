// @vitest-environment jsdom
import { flushPromises, mount } from '@vue/test-utils';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const harness = vi.hoisted(() => ({
  ids: ['one', 'two', 'three'],
  archive: vi.fn(), toggleDone: vi.fn(), allDone: vi.fn(() => false), trash: vi.fn(), setRead: vi.fn(), openPicker: vi.fn(), openReminder: vi.fn(),
}));
vi.mock('@/stores/threads', () => ({ useThreadsStore: () => ({
  selectedThreadIds: harness.ids,
  selectedThreads: harness.ids.map(id => ({ id, unreadCount: id === 'one' ? 1 : 0 })),
}) }));
vi.mock('@/stores/ui', () => ({ useUiStore: () => ({ openReminder: harness.openReminder }) }));
vi.mock('@/stores/topics', () => ({ useTopicsStore: () => ({ openPicker: harness.openPicker }) }));
vi.mock('@/composables/useThreadActions', () => ({ useThreadActions: () => harness }));
vi.mock('@/composables/useShortcutHints', () => ({ useShortcutHints: () => ({ hint: () => null }) }));
import SelectionToolbar from './SelectionToolbar.vue';

beforeEach(() => vi.clearAllMocks());

describe('selection toolbar', () => {
  it('offers common actions before the overflow and files through the existing picker', async () => {
    const wrapper = mount(SelectionToolbar, { props: { busy: false } });
    const buttons = wrapper.findAll('.selection-actions button');
    expect(buttons.map(button => button.text() || button.attributes('aria-label'))).toEqual([
      'Done (3)', 'Snooze', 'Delete', 'Mark as read', 'File…', 'More actions',
    ]);
    await buttons[0]!.trigger('click');
    await buttons[1]!.trigger('click');
    await buttons[2]!.trigger('click');
    await buttons[3]!.trigger('click');
    await buttons[4]!.trigger('click');
    await flushPromises();
    expect(harness.toggleDone).toHaveBeenCalledWith(harness.ids);
    expect(harness.openReminder).toHaveBeenCalledWith(harness.ids);
    expect(harness.trash).toHaveBeenCalledWith(harness.ids);
    expect(harness.setRead).toHaveBeenCalledWith(harness.ids, true, { undo: true });
    expect(harness.openPicker).toHaveBeenCalledWith({ itemType: 'thread', itemIds: harness.ids });
    wrapper.unmount();
  });

  it('sends a selection of done mail back to the inbox instead of offering Done', async () => {
    harness.allDone.mockReturnValue(true);
    const wrapper = mount(SelectionToolbar, { props: { busy: false } });
    const primary = wrapper.findAll('.selection-actions button')[0]!;
    expect(primary.text()).toBe('To inbox (3)');
    await primary.trigger('click');
    expect(harness.toggleDone).toHaveBeenCalledWith(harness.ids);
    harness.allDone.mockReturnValue(false);
    wrapper.unmount();
  });
});
