// @vitest-environment jsdom
import { defineComponent, h, nextTick, ref } from 'vue';
import { flushPromises, mount } from '@vue/test-utils';
import { describe, expect, it, vi } from 'vitest';

const listPlugins = vi.hoisted(() => vi.fn(async (): Promise<unknown[]> => []));
const executePlugin = vi.hoisted(() => vi.fn(async () => 'rendered'));

vi.mock('@/services/backend', () => ({
  backendListPlugins: listPlugins,
  backendExecutePluginCommand: executePlugin,
}));
vi.mock('vue-i18n', () => ({ useI18n: () => ({ t: (key: string) => key }) }));

import PluginSlotHost from './PluginSlotHost.vue';

describe('PluginSlotHost', () => {
  it('lists the plugins once per mount and re-renders them for a new context', async () => {
    listPlugins.mockResolvedValue([{ id: 'notes', ui_slots: ['thread_view_panel'] }]);
    const tick = ref(0);
    const threadId = ref('thread-1');
    const Parent = defineComponent({
      setup: () => () => h('div', { 'data-tick': tick.value }, [
        // A fresh object per render, as the reader's template passes it.
        h(PluginSlotHost, { slotName: 'thread_view_panel', context: { threadId: threadId.value } }),
      ]),
    });
    const wrapper = mount(Parent);
    await flushPromises();
    expect(listPlugins).toHaveBeenCalledTimes(1);

    for (let index = 0; index < 5; index += 1) {
      tick.value += 1;
      await nextTick();
    }
    await flushPromises();
    expect(listPlugins).toHaveBeenCalledTimes(1);

    expect(executePlugin).toHaveBeenCalledTimes(1);

    threadId.value = 'thread-2';
    await flushPromises();
    expect(listPlugins).toHaveBeenCalledTimes(1);
    expect(executePlugin).toHaveBeenCalledTimes(2);
    expect(wrapper.text()).toContain('rendered');
    wrapper.unmount();
  });
});
