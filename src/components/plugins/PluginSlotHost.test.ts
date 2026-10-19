// @vitest-environment jsdom
import { defineComponent, h, nextTick, ref } from 'vue';
import { flushPromises, mount } from '@vue/test-utils';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';

const listPlugins = vi.hoisted(() => vi.fn(async (): Promise<unknown[]> => []));
const executePlugin = vi.hoisted(() => vi.fn(async () => 'rendered'));

vi.mock('@/services/backend', () => ({
  backendListPlugins: listPlugins,
  backendExecutePluginCommand: executePlugin,
}));
vi.mock('vue-i18n', () => ({ useI18n: () => ({ t: (key: string) => key }) }));

import PluginSlotHost from './PluginSlotHost.vue';

const plugin = (overrides: Record<string, unknown> = {}) => ({
  id: 'notes', enabled: true, ui_slots: ['thread_view_panel'],
  capabilities: ['read_threads'], granted_capabilities: ['read_threads'],
  ...overrides,
});

describe('PluginSlotHost', () => {
  beforeEach(() => {
    vi.resetAllMocks();
    listPlugins.mockResolvedValue([]);
    executePlugin.mockResolvedValue('rendered');
  });

  it.each(['calendar-sync', 'github-integration'])('does not mount unfinished %s example contributions', async (name) => {
    // Read the shipping manifest, rather than a fixture that can drift from it.
    const manifest = readFileSync(`plugins/${name}/Cargo.toml`, 'utf8');
    const slots = JSON.parse(manifest.match(/^ui_slots = (.*)$/m)?.[1] ?? '[]');
    listPlugins.mockResolvedValue([plugin({ ui_slots: slots })]);
    executePlugin.mockResolvedValue(`${name} command=render-slot payload={}`);
    for (const slotName of ['sidebar_section', 'thread_view_panel', 'assistant_context_panel'] as const) {
      const wrapper = mount(PluginSlotHost, { props: { slotName } });
      await flushPromises();
      expect(wrapper.find('.plugin-slot-card').exists()).toBe(false);
      wrapper.unmount();
    }
    expect(executePlugin).not.toHaveBeenCalled();
  });

  it.each([
    { enabled: false },
    { granted_capabilities: [] },
  ])('does not call unavailable thread contributions: %j', async (overrides) => {
    listPlugins.mockResolvedValue([plugin(overrides)]);
    executePlugin.mockRejectedValueOnce(new Error('read_threads denied'));
    const wrapper = mount(PluginSlotHost, { props: { slotName: 'thread_view_panel', context: { threadId: 'private-thread' } } });
    await flushPromises();
    expect(executePlugin).not.toHaveBeenCalled();
    expect(wrapper.find('.plugin-slot-card').exists()).toBe(false);
    wrapper.unmount();
  });

  it('allows a sidebar contribution without granting message access', async () => {
    listPlugins.mockResolvedValue([plugin({ ui_slots: ['sidebar_section'], granted_capabilities: [] })]);
    const wrapper = mount(PluginSlotHost, { props: { slotName: 'sidebar_section' } });
    await flushPromises();
    expect(executePlugin).toHaveBeenCalledTimes(1);
    expect(wrapper.text()).toContain('rendered');
    wrapper.unmount();
  });

  it('lists the plugins once per mount and re-renders them for a new context', async () => {
    listPlugins.mockResolvedValue([plugin()]);
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
