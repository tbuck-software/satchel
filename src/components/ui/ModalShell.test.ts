// @vitest-environment jsdom
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { mount, type VueWrapper } from '@vue/test-utils';
import ModalShell from './ModalShell.vue';
import BottomSheet from './BottomSheet.vue';
import { useUiStore } from '@/stores/ui';

// The shell counts its scroll locks across instances, so a shell left mounted
// by one test would hold the page for the next one.
const shells: VueWrapper[] = [];

interface ShellProps {
  label: string;
  width?: string;
  height?: string;
  dismissOnBackdrop?: boolean;
}

function openShell(props: ShellProps, slots?: Record<string, string>) {
  const wrapper = mount(ModalShell, { attachTo: document.body, props, slots });
  shells.push(wrapper);
  return wrapper;
}

beforeEach(() => {
  setActivePinia(createPinia());
});

afterEach(() => {
  while (shells.length > 0) shells.pop()?.unmount();
});

describe('modal shell', () => {
  it('is a bounded dialog when there is room', () => {
    useUiStore().isSinglePane = false;
    const wrapper = openShell({ label: 'Einstellungen' });

    expect(wrapper.find('.shell-backdrop--full').exists()).toBe(false);
    expect(wrapper.find('.shell-panel').attributes('style')).toContain('--shell-w');
  });

  /**
   * The gutter that reads as breathing room on a desktop is content the user
   * does not get on a phone.
   */
  it('fills the screen when there is not', () => {
    useUiStore().isSinglePane = true;
    const wrapper = openShell({ label: 'Einstellungen' });

    expect(wrapper.find('.shell-backdrop--full').exists()).toBe(true);
    // No width bound to fight the full-bleed rule.
    expect(wrapper.find('.shell-panel').attributes('style')).toBeUndefined();
  });

  it('dismisses on the backdrop but not through the panel', async () => {
    const wrapper = openShell({ label: 'Test' });

    await wrapper.find('.shell-panel').trigger('click');
    expect(wrapper.emitted('dismiss')).toBeUndefined();

    await wrapper.find('.shell-backdrop').trigger('click');
    expect(wrapper.emitted('dismiss')).toHaveLength(1);
  });

  it('leaves the backdrop inert for flows that must not be clicked away', async () => {
    const wrapper = openShell({ label: 'Test', dismissOnBackdrop: false });

    await wrapper.find('.shell-backdrop').trigger('click');
    expect(wrapper.emitted('dismiss')).toBeUndefined();
  });

  /**
   * Six dialogs used to mix four scrims, one of them the dark value of the
   * token copied out by hand. There is one now, and it follows the theme.
   */
  it('dims with the shared backdrop token and no colour of its own', () => {
    const source = readFileSync(join(process.cwd(), 'src/components/ui/ModalShell.vue'), 'utf8');

    expect(source).toContain('background: var(--bg-backdrop);');
    expect(source).not.toMatch(/rgba?\(/);
  });

  it('names itself to assistive tech and closes the page behind it', () => {
    const panel = openShell({ label: 'Einstellungen' }).find('.shell-panel');

    expect(panel.attributes('role')).toBe('dialog');
    expect(panel.attributes('aria-modal')).toBe('true');
    expect(panel.attributes('aria-label')).toBe('Einstellungen');
  });

  it('holds the keyboard inside and gives it back on the way out', async () => {
    const opener = document.createElement('button');
    document.body.append(opener);
    opener.focus();

    const wrapper = openShell(
      { label: 'Test' },
      { default: '<button class="first">a</button><button class="last">b</button>' },
    );
    const backdrop = wrapper.find('.shell-backdrop');
    const first = wrapper.find('.first').element;
    const last = wrapper.find('.last').element as HTMLElement;

    // Nothing inside asked for focus, so the panel takes it and is announced.
    expect(document.activeElement).toBe(wrapper.find('.shell-panel').element);

    last.focus();
    await backdrop.trigger('keydown', { key: 'Tab' });
    expect(document.activeElement).toBe(first);

    await backdrop.trigger('keydown', { key: 'Tab', shiftKey: true });
    expect(document.activeElement).toBe(last);

    wrapper.unmount();
    expect(document.activeElement).toBe(opener);
    opener.remove();
  });

  it('lets a field that asked for focus keep it', () => {
    const wrapper = openShell(
      { label: 'Test' },
      { default: '<button class="first">a</button><input class="search" autofocus>' },
    );

    expect(document.activeElement).toBe(wrapper.find('.search').element);
  });

  /**
   * The composer's body is a `contenteditable`, not an `<input>`. A trap that
   * only looked for form controls tabbed straight past the message.
   */
  it('keeps a rich-text body in the tab cycle', async () => {
    const wrapper = openShell(
      { label: 'Test' },
      {
        default:
          '<button class="first">a</button><div class="body" contenteditable="true"></div>',
      },
    );
    const backdrop = wrapper.find('.shell-backdrop');
    (wrapper.find('.first').element as HTMLElement).focus();

    await backdrop.trigger('keydown', { key: 'Tab' });
    expect(document.activeElement).toBe(wrapper.find('.body').element);
  });

  /**
   * The recipient field answers Tab by committing the typed address as a chip.
   * A trap that moved focus on anyway took the field away mid-word.
   */
  it('leaves a Tab a field inside already answered alone', async () => {
    const wrapper = openShell(
      { label: 'Test' },
      { default: '<input class="chips"><button class="last">b</button>' },
    );
    const chips = wrapper.find('.chips').element as HTMLElement;
    chips.focus();
    chips.addEventListener('keydown', (event) => event.preventDefault(), { once: true });

    chips.dispatchEvent(
      new KeyboardEvent('keydown', { key: 'Tab', bubbles: true, cancelable: true }),
    );
    expect(document.activeElement).toBe(chips);
  });

  it('stops the page behind it from scrolling until the last shell is gone', () => {
    const outer = openShell({ label: 'Outer' });
    expect(document.body.style.overflow).toBe('hidden');

    openShell({ label: 'Inner' }).unmount();
    expect(document.body.style.overflow).toBe('hidden');

    outer.unmount();
    expect(document.body.style.overflow).toBe('');
  });
});

describe('bottom sheet', () => {
  it('rises from the bottom edge on a phone, where the thumb is', () => {
    useUiStore().isSinglePane = true;
    const wrapper = mount(BottomSheet, { props: { label: 'Sortieren', open: true } });

    expect(wrapper.find('.sheet-root--anchored').exists()).toBe(false);
    expect(wrapper.find('.sheet-grip').exists()).toBe(true);
  });

  it('stays an ordinary popover where a pointer has no reach problem', () => {
    useUiStore().isSinglePane = false;
    const wrapper = mount(BottomSheet, { props: { label: 'Sortieren', open: true } });

    expect(wrapper.find('.sheet-root--anchored').exists()).toBe(true);
    // The grab handle promises a gesture that does not exist here.
    expect(wrapper.find('.sheet-grip').exists()).toBe(false);
  });

  it('renders nothing while closed', () => {
    const wrapper = mount(BottomSheet, { props: { label: 'Sortieren', open: false } });
    expect(wrapper.find('.sheet-panel').exists()).toBe(false);
  });

  it('registers as the topmost back-navigation overlay while open', async () => {
    const ui = useUiStore();
    const wrapper = mount(BottomSheet, { props: { label: 'Details', open: true } });

    expect(ui.overlayDepth).toBe(1);
    expect(ui.dismissTopmostOverlay()).toBe(true);
    expect(wrapper.emitted('dismiss')).toHaveLength(1);

    await wrapper.setProps({ open: false });
    expect(ui.overlayDepth).toBe(0);
  });

  it('can be dragged down from the sheet content to dismiss', async () => {
    useUiStore().isSinglePane = true;
    const wrapper = mount(BottomSheet, {
      props: { label: 'Details', open: true },
      slots: { default: '<div class="middle">Inhalt</div>' },
    });
    const sheet = wrapper.find('.sheet-panel');

    await sheet.trigger('pointerdown', {
      pointerType: 'mouse', pointerId: 1, button: 0, clientX: 160, clientY: 220,
    });
    await sheet.trigger('pointermove', {
      pointerType: 'mouse', pointerId: 1, clientX: 160, clientY: 320,
    });
    expect(sheet.attributes('style')).toContain('--sheet-drag-y: 100px');

    await sheet.trigger('pointerup', {
      pointerType: 'mouse', pointerId: 1, clientX: 160, clientY: 320,
    });
    expect(wrapper.emitted('dismiss')).toHaveLength(1);
  });

  it('keeps a downward gesture as content scrolling while the sheet is scrolled', async () => {
    useUiStore().isSinglePane = true;
    const wrapper = mount(BottomSheet, { props: { label: 'Details', open: true } });
    const sheet = wrapper.find('.sheet-panel');
    Object.defineProperty(sheet.element, 'scrollTop', { value: 40, writable: true });

    await sheet.trigger('pointerdown', {
      pointerType: 'mouse', pointerId: 1, button: 0, clientX: 160, clientY: 220,
    });
    await sheet.trigger('pointermove', {
      pointerType: 'mouse', pointerId: 1, clientX: 160, clientY: 320,
    });
    await sheet.trigger('pointerup', {
      pointerType: 'mouse', pointerId: 1, clientX: 160, clientY: 320,
    });

    expect(wrapper.emitted('dismiss')).toBeUndefined();
    expect(sheet.attributes('style')).toContain('--sheet-drag-y: 0px');
  });
});
