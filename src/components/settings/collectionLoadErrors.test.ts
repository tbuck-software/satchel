// @vitest-environment jsdom
import { flushPromises, mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { nextTick } from 'vue';
import TemplatesEditor from './TemplatesEditor.vue';
import SmartFoldersEditor from './SmartFoldersEditor.vue';
import TemplatePicker from '@/components/mail/TemplatePicker.vue';
import { useTemplatesStore } from '@/stores/templates';
import { useSmartFoldersStore } from '@/stores/smartFolders';

const backend = vi.hoisted(() => ({
  backendCommandsAvailable: () => true,
  backendListTemplates: vi.fn(), backendListSmartFolders: vi.fn(),
}));
vi.mock('@/services/backend', () => backend);

beforeEach(() => { setActivePinia(createPinia()); vi.resetAllMocks(); });
afterEach(() => vi.useRealTimers());
for (const [component, list, empty] of [
  [TemplatesEditor, backend.backendListTemplates, 'No templates.'],
  [TemplatePicker, backend.backendListTemplates, 'No templates yet. Create them under Settings > Templates.'],
  [SmartFoldersEditor, backend.backendListSmartFolders, 'No smart folders yet.'],
] as const) {
  it(`${component.__name} distinguishes loading and failure from empty and exposes a native retry button`, async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    let fail!: (e: Error) => void;
    list.mockReturnValueOnce(new Promise((_, reject) => { fail = reject; })).mockResolvedValue([]);
    const wrapper = mount(component, { global: { stubs: { ModalShell: { template: '<div><slot /></div>' } } } });
    await nextTick();
    expect(wrapper.find('[role="status"]').exists()).toBe(false);
    await vi.advanceTimersByTimeAsync(160);
    expect(wrapper.get('[role="status"]').text()).toBe('Loading…');
    vi.useRealTimers();
    expect(wrapper.text()).not.toContain(empty);
    fail(new Error('offline'));
    await flushPromises();
    expect(wrapper.get('[role="alert"]').text()).toContain('Try again');
    expect(wrapper.text()).not.toContain(empty);
    const retry = wrapper.get('[role="alert"] button');
    expect(retry.attributes('type')).toBe('button');
    await retry.trigger('click'); // Native button also handles Enter/Space.
    await flushPromises();
    expect(list).toHaveBeenCalledTimes(2);
    expect(wrapper.find('[role="alert"]').exists()).toBe(false);
    expect(wrapper.text()).toContain(empty);
    wrapper.unmount();
  });
}

for (const [component, useStore, list] of [
  [TemplatesEditor, useTemplatesStore, backend.backendListTemplates],
  [TemplatePicker, useTemplatesStore, backend.backendListTemplates],
  [SmartFoldersEditor, useSmartFoldersStore, backend.backendListSmartFolders],
] as const) {
  it(`${component.__name} keeps existing entries usable beside a refresh error`, async () => {
    const entry = { id: 'retained', name: 'Retained item', body_text: 'Text', query: 'is:unread' };
    useStore().items = [entry] as never;
    list.mockRejectedValueOnce(new Error('offline')).mockResolvedValue([entry]);
    const wrapper = mount(component, { global: { stubs: { ModalShell: { template: '<div><slot /></div>' } } } });
    await flushPromises();
    expect(wrapper.text()).toContain('Retained item');
    expect(wrapper.find('[role="alert"]').exists()).toBe(true);
    expect(wrapper.findAll('.list button').length).toBeGreaterThan(0);
    await wrapper.get('[role="alert"] button').trigger('click');
    await flushPromises();
    expect(wrapper.text()).toContain('Retained item');
    expect(wrapper.find('[role="alert"]').exists()).toBe(false);
    wrapper.unmount();
  });
}
