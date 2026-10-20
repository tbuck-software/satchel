// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { mount, type VueWrapper } from '@vue/test-utils';
import PromptDialog from './PromptDialog.vue';
import { usePrompt } from '@/composables/usePrompt';
import { useUiStore } from '@/stores/ui';

/*
 * The in-app replacement for `window.prompt`: renders nothing until asked,
 * resolves with what was typed on submit, and with `null` for every other way
 * out.
 */

let wrapper: VueWrapper | null = null;

function mountDialog() {
  wrapper = mount(PromptDialog, { attachTo: document.body });
  return wrapper;
}

function input() {
  return document.body.querySelector<HTMLInputElement>('.prompt input')!;
}

beforeEach(() => {
  setActivePinia(createPinia());
  usePrompt().answer(null);
});

afterEach(() => {
  wrapper?.unmount();
  wrapper = null;
});

describe('prompt dialog', () => {
  it('renders nothing while nothing is pending', () => {
    mountDialog();
    expect(document.body.querySelector('.prompt')).toBeNull();
  });

  it('starts from the initial value, names the field and resolves with what was typed', async () => {
    const view = mountDialog();
    const answered = usePrompt().ask({
      title: 'Edit tags',
      label: 'Tags (comma separated)',
      initialValue: 'work, later',
      confirmLabel: 'Save',
    });
    await view.vm.$nextTick();

    expect(document.body.querySelector('[role="dialog"]')?.getAttribute('aria-label')).toBe('Edit tags');
    expect(document.body.querySelector('.prompt__field span')?.textContent).toBe('Tags (comma separated)');
    expect(input().value).toBe('work, later');
    expect(input().type).toBe('text');

    input().value = 'work';
    input().dispatchEvent(new Event('input'));
    document.body.querySelector<HTMLFormElement>('.prompt')!
      .dispatchEvent(new Event('submit', { cancelable: true }));

    await expect(answered).resolves.toBe('work');
  });

  it('masks a passphrase and does not carry it over to the next question', async () => {
    const view = mountDialog();
    const first = usePrompt().ask({
      title: 'Unlock certificate',
      label: 'S/MIME passphrase',
      confirmLabel: 'Unlock',
      inputType: 'password',
    });
    await view.vm.$nextTick();
    expect(input().type).toBe('password');
    input().value = 'hunter2';
    input().dispatchEvent(new Event('input'));

    usePrompt().ask({ title: 'Again', label: 'S/MIME passphrase', confirmLabel: 'Unlock' });
    await view.vm.$nextTick();

    await expect(first).resolves.toBeNull();
    expect(input().value).toBe('');
  });

  it('answers null on cancel and on Back, even above an overlay opened earlier', async () => {
    const view = mountDialog();
    const ui = useUiStore();
    let sheetDismissed = false;
    // A sheet that registered after the dialog was mounted, as the profile
    // sheet does: Back must still close the question first.
    ui.registerOverlay({ isOpen: () => true, dismiss: () => { sheetDismissed = true; } });

    const backed = usePrompt().ask({ title: 'T', label: 'L', confirmLabel: 'OK' });
    await view.vm.$nextTick();
    ui.dismissTopmostOverlay();
    await expect(backed).resolves.toBeNull();
    expect(sheetDismissed).toBe(false);

    const cancelled = usePrompt().ask({ title: 'T', label: 'L', confirmLabel: 'OK' });
    await view.vm.$nextTick();
    document.body.querySelectorAll<HTMLButtonElement>('.prompt__actions button')[0]!.click();
    await expect(cancelled).resolves.toBeNull();
  });
});
