// @vitest-environment jsdom

import { mount as mountComponent } from '@vue/test-utils';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { nextTick } from 'vue';
import { createPinia } from 'pinia';
import { useToast } from '@/composables/useToast';
import ToastStack from './ToastStack.vue';

const toast = useToast();

function mount(component: typeof ToastStack) {
  return mountComponent(component, {
    global: { plugins: [createPinia()] },
  });
}

describe('ToastStack on compact screens', () => {
  beforeEach(() => {
    vi.stubGlobal('matchMedia', vi.fn(() => ({
      matches: true,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    })));
  });

  afterEach(() => {
    toast.clear();
    vi.unstubAllGlobals();
    vi.useRealTimers();
  });

  it('shows an error beside a newer undo instead of letting it expire unseen', async () => {
    toast.error('Synchronisierung fehlgeschlagen');
    toast.success('Erledigt', {
      timeout: 9000,
      action: { label: 'Rückgängig', handler: vi.fn() },
    });
    toast.info('Neue Mail');

    const wrapper = mount(ToastStack);
    await nextTick();

    const rows = wrapper.findAll('.toast');
    expect(rows).toHaveLength(2);
    expect(rows[0]!.text()).toContain('Synchronisierung fehlgeschlagen');
    expect(rows[1]!.text()).toContain('Rückgängig');
    expect(wrapper.get('.stack').text()).not.toContain('Neue Mail');
    wrapper.unmount();
  });

  it('lets the user reach each independent undo without stacking rows', async () => {
    toast.success('Erste Aktion', {
      action: { label: 'Rückgängig', handler: vi.fn() },
    });
    toast.success('Zweite Aktion', {
      action: { label: 'Rückgängig', handler: vi.fn() },
    });
    const wrapper = mount(ToastStack);
    await nextTick();

    expect(wrapper.findAll('.toast')).toHaveLength(1);
    expect(wrapper.get('.stack').text()).toContain('Zweite Aktion');
    const navigator = wrapper.get('.toast__queue');
    expect(navigator.text()).toBe('2');
    await navigator.trigger('click');
    expect(wrapper.get('.stack').text()).toContain('Erste Aktion');
    expect(wrapper.findAll('.toast')).toHaveLength(1);
    wrapper.unmount();
  });

  it('combines equivalent workflow receipts and undoes every source action', async () => {
    vi.stubGlobal('matchMedia', vi.fn(() => ({
      matches: false,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    })));
    const directUndo = vi.fn();
    const batchUndo = vi.fn();
    const groupMessage = (count: number) => `${count} E-Mails erledigt`;
    toast.success('E-Mail erledigt', {
      groupKey: 'workflow:done',
      count: 1,
      groupMessage,
      action: {
        label: 'Rückgängig',
        handler: directUndo,
        batch: { key: 'workflow:restore', items: ['first'], handler: batchUndo },
      },
    });
    toast.success('99 E-Mails erledigt', {
      groupKey: 'workflow:done',
      count: 99,
      groupMessage,
      action: {
        label: 'Rückgängig',
        handler: directUndo,
        batch: {
          key: 'workflow:restore',
          items: Array.from({ length: 99 }, (_, index) => `bulk-${index}`),
          handler: batchUndo,
        },
      },
    });

    const wrapper = mount(ToastStack);
    await nextTick();

    expect(wrapper.findAll('.toast')).toHaveLength(1);
    expect(wrapper.text()).toContain('100 E-Mails erledigt');
    await wrapper.get('.toast__action').trigger('click');
    await vi.waitFor(() => {
      expect(batchUndo).toHaveBeenCalledOnce();
      expect(batchUndo.mock.calls[0]![0]).toHaveLength(100);
    });
    expect(directUndo).not.toHaveBeenCalled();
    expect(toast.state.toasts).toHaveLength(0);
    wrapper.unmount();
  });

  it('reduces a group as the independent undo deadlines expire', async () => {
    vi.useFakeTimers({ toFake: ['setInterval', 'clearInterval', 'Date'] });
    const expiredUndo = vi.fn();
    const currentUndo = vi.fn();
    const groupMessage = (count: number) => `${count} E-Mails erledigt`;
    toast.success('E-Mail erledigt', {
      timeout: 100,
      groupKey: 'workflow:done',
      count: 1,
      groupMessage,
      action: { label: 'Rückgängig', handler: expiredUndo },
    });
    toast.success('2 E-Mails erledigt', {
      timeout: 1000,
      groupKey: 'workflow:done',
      count: 2,
      groupMessage,
      action: { label: 'Rückgängig', handler: currentUndo },
    });
    const wrapper = mount(ToastStack);
    await nextTick();
    expect(wrapper.text()).toContain('3 E-Mails erledigt');

    await vi.advanceTimersByTimeAsync(300);
    expect(wrapper.text()).toContain('2 E-Mails erledigt');
    await wrapper.get('.toast__action').trigger('click');
    await vi.waitFor(() => expect(currentUndo).toHaveBeenCalledOnce());
    expect(expiredUndo).not.toHaveBeenCalled();
    wrapper.unmount();
  });
});

/*
 * VoiceOver often skips a live region that is inserted together with its text,
 * which is how the rows appear. The regions below exist before any toast.
 */
describe('ToastStack announcements', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.stubGlobal('matchMedia', vi.fn(() => ({
      matches: false,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    })));
  });

  afterEach(() => {
    toast.clear();
    vi.unstubAllGlobals();
    vi.useRealTimers();
  });

  it('announces through regions that are already in the page', async () => {
    const wrapper = mount(ToastStack);
    const status = wrapper.get('[role="status"]');
    const alert = wrapper.get('[role="alert"]');
    expect(status.attributes('aria-live')).toBe('polite');
    expect(status.text()).toBe('');

    toast.info('Sync finished.');
    await nextTick();
    await vi.advanceTimersByTimeAsync(150);
    expect(status.text()).toBe('Sync finished.');

    toast.error('Sync failed.');
    await nextTick();
    await vi.advanceTimersByTimeAsync(150);
    expect(alert.text()).toBe('Sync failed.');
    // Rows no longer carry their own live region, or each message would be read twice.
    expect(wrapper.findAll('.toast[role]')).toHaveLength(0);
    wrapper.unmount();
  });

  it('announces an undo receipt shown by the FAB together with its key', async () => {
    const wrapper = mount(ToastStack);

    toast.success('Marked as done', {
      action: { label: 'Undo', handler: vi.fn(), present: 'fab' },
    });
    await nextTick();
    await vi.advanceTimersByTimeAsync(150);

    expect(wrapper.findAll('.toast')).toHaveLength(0);
    expect(wrapper.get('[role="status"]').text()).toBe('Marked as done. Press z to undo.');
    wrapper.unmount();
  });
});
