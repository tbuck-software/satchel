// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { mount, type VueWrapper } from '@vue/test-utils';
import ConfirmDialog from './ConfirmDialog.vue';
import { useConfirm } from '@/composables/useConfirm';
import { useUiStore } from '@/stores/ui';

/*
 * The dialog is mounted once for the whole app, so its contract is: it renders
 * nothing until someone asks, and every way out that is not the confirming
 * button answers no.
 */

let wrapper: VueWrapper | null = null;

function mountDialog() {
  // The test setup already installs the real i18n instance.
  wrapper = mount(ConfirmDialog, { attachTo: document.body });
  return wrapper;
}

beforeEach(() => {
  setActivePinia(createPinia());
  useConfirm().answer(false);
});

afterEach(() => {
  wrapper?.unmount();
  wrapper = null;
});

describe('confirm dialog', () => {
  it('renders nothing while nothing is pending', () => {
    expect(mountDialog().find('.confirm').exists()).toBe(false);
  });

  it('shows the question and resolves true on the confirming button', async () => {
    const view = mountDialog();
    const answered = useConfirm().ask({
      title: 'Remove this account?',
      body: 'Syncing stops.',
      detail: 'person@example.test',
      confirmLabel: 'Remove account',
      tone: 'danger',
    });
    await view.vm.$nextTick();

    expect(document.body.textContent).toContain('Remove this account?');
    expect(document.body.textContent).toContain('person@example.test');

    const buttons = document.body.querySelectorAll<HTMLButtonElement>('.confirm__actions button');
    expect(buttons).toHaveLength(2);
    // The destructive answer is the second one; cancel comes first and holds focus.
    expect(buttons[0]!.hasAttribute('autofocus')).toBe(true);
    buttons[1]!.click();

    await expect(answered).resolves.toBe(true);
  });

  it('answers no when it is dismissed', async () => {
    const view = mountDialog();
    const answered = useConfirm().ask({ title: 'T', body: 'B', confirmLabel: 'Yes' });
    await view.vm.$nextTick();

    document.body.querySelector<HTMLElement>('.shell-backdrop')?.click();

    await expect(answered).resolves.toBe(false);
  });

  it('closes on Back before an overlay that opened after the dialog was mounted', async () => {
    const view = mountDialog();
    const ui = useUiStore();
    let sheetDismissed = false;
    // The profile sheet registers long after App.vue mounted the dialog; a
    // question asked from it must still be the first thing Back closes.
    ui.registerOverlay({ isOpen: () => true, dismiss: () => { sheetDismissed = true; } });

    const answered = useConfirm().ask({ title: 'Block?', confirmLabel: 'Block' });
    await view.vm.$nextTick();
    ui.dismissTopmostOverlay();

    await expect(answered).resolves.toBe(false);
    expect(sheetDismissed).toBe(false);
  });

  it('does not orphan the first promise when a second question arrives', async () => {
    mountDialog();
    const { ask } = useConfirm();
    const first = ask({ title: 'First', body: 'B', confirmLabel: 'Yes' });
    const second = ask({ title: 'Second', body: 'B', confirmLabel: 'Yes' });

    await expect(first).resolves.toBe(false);
    useConfirm().answer(true);
    await expect(second).resolves.toBe(true);
  });
});
