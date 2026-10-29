// @vitest-environment jsdom
import { mount, flushPromises, type VueWrapper } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import SendLaterMenu from './SendLaterMenu.vue';
import { sendLaterPresets, toLocalMinute } from '@/utils/sendLater';

let wrapper: VueWrapper;
let trigger: HTMLButtonElement;
beforeEach(() => {
  setActivePinia(createPinia());
  trigger = document.createElement('button');
  document.body.append(trigger);
  trigger.focus();
});
afterEach(() => {
  wrapper?.unmount();
  trigger.remove();
  vi.useRealTimers();
});

async function render(value: string | null = null) {
  wrapper = mount(SendLaterMenu, {
    attachTo: document.body,
    props: {
      open: true, anchor: { left: 600, right: 632, top: 800, bottom: 838 }, value,
      onClose: () => { void wrapper.setProps({ open: false }); },
      onSelect: () => { void wrapper.setProps({ open: false }); },
    },
  });
  await flushPromises();
  return [...document.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')];
}

describe('send-later menu', () => {
  it('moves through presets with arrows, wraps and returns focus on Escape', async () => {
    const items = await render();
    expect(items.map(item => item.textContent?.trim())).toEqual([
      'Tomorrow 09:00', 'Tomorrow 15:00', 'Monday 09:00', 'Pick date & time…',
    ]);
    expect(document.activeElement).toBe(items[0]);
    items[0]!.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowUp', bubbles: true }));
    expect(document.activeElement).toBe(items[3]);
    items[3]!.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true }));
    expect(document.activeElement).toBe(items[0]);
    items[0]!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    await flushPromises();
    expect(wrapper.emitted('close')).toHaveLength(1);
    expect(document.activeElement).toBe(trigger);
  });

  it('selects a preset and can remove an existing schedule', async () => {
    const items = await render('2026-12-01T09:00:00Z');
    const expected = sendLaterPresets()[1].date.toISOString();
    items[1]!.click();
    await flushPromises();
    expect(wrapper.emitted('select')).toEqual([[expected]]);
    await wrapper.setProps({ open: true });
    await flushPromises();
    const clear = [...document.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].at(-1)!;
    expect(clear.textContent).toContain('Remove schedule');
    clear.click();
    expect(wrapper.emitted('select')?.at(-1)).toEqual([null]);
  });

  it('opens only the custom picker with a local minute value', async () => {
    vi.useFakeTimers({ toFake: ['Date'] });
    vi.setSystemTime(new Date(2026, 9, 26, 12));
    const saved = new Date(2026, 9, 27, 9, 15, 54);
    const items = await render(saved.toISOString());
    items[3]!.focus();
    items[3]!.click();
    await flushPromises();
    expect(document.querySelector('[role="menu"]')).toBeNull();
    await vi.waitFor(() => expect(document.querySelector('[role="dialog"] input')).not.toBeNull());
    const input = document.querySelector<HTMLInputElement>('[role="dialog"] input')!;
    expect(input.value).toBe(toLocalMinute(saved));
    expect(input.value).toBe('2026-10-27T09:15');
    expect(input.step).toBe('60');
    input.value = '2026-10-28T14:30';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    await flushPromises();
    [...document.querySelectorAll<HTMLButtonElement>('[role="dialog"] button')].find(item => item.textContent === 'Set')!.click();
    await flushPromises();
    expect(wrapper.emitted('select')).toEqual([[new Date('2026-10-28T14:30').toISOString()]]);
    expect(document.activeElement).toBe(trigger);
  });

  it('recomputes relative dates on reopening and keeps Monday in the next week', async () => {
    vi.useFakeTimers({ toFake: ['Date'] });
    vi.setSystemTime(new Date(2026, 9, 26, 23, 59, 54));
    expect(sendLaterPresets().map(item => toLocalMinute(item.date))).toEqual([
      '2026-10-27T09:00', '2026-10-27T15:00', '2026-11-02T09:00',
    ]);
    const items = await render();
    items[0]!.click();
    await flushPromises();
    vi.setSystemTime(new Date(2026, 9, 27, 0, 1));
    await wrapper.setProps({ open: true });
    await flushPromises();
    document.querySelector<HTMLButtonElement>('[role="menuitem"]')!.click();
    expect(wrapper.emitted('select')?.at(-1)).toEqual([new Date(2026, 9, 28, 9).toISOString()]);
  });
});
