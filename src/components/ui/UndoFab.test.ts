// @vitest-environment jsdom
import { mount } from '@vue/test-utils';
import { nextTick } from 'vue';
import { afterEach, expect, it, vi } from 'vitest';
import { createI18n } from 'vue-i18n';
import { createPinia, setActivePinia } from 'pinia';
import { useToast } from '@/composables/useToast';
import UndoFab from './UndoFab.vue';

afterEach(() => useToast().clear());

it('defaults to the newest receipt and follows a newly added one', async () => {
  vi.stubGlobal('matchMedia', () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
  const pinia = createPinia();
  setActivePinia(pinia);
  const toast = useToast();
  const older = vi.fn(() => false);
  const newer = vi.fn(() => false);
  const latest = vi.fn(() => false);
  const receipt = (label: string, handler: () => boolean) => toast.info(label, {
    action: { label: 'Undo', present: 'fab', handler },
  });
  receipt('old', older);
  receipt('new', newer);
  const wrapper = mount(UndoFab, {
    props: { variant: 'docked' },
    global: { plugins: [pinia, createI18n({ legacy: false, locale: 'en', messages: { en: { ui: { undoAria: 'Undo {label}', undoTitle: 'Undo {label}', toastQueue: 'Queue {count}', toastQueueAria: 'Queue {count}' } } } })] },
  });
  await wrapper.find('.undo-fab__main').trigger('click');
  expect(newer).toHaveBeenCalledOnce();
  expect(older).not.toHaveBeenCalled();
  receipt('latest', latest);
  await nextTick();
  await wrapper.find('.undo-fab__main').trigger('click');
  expect(latest).toHaveBeenCalledOnce();
  wrapper.unmount();
  vi.unstubAllGlobals();
});
