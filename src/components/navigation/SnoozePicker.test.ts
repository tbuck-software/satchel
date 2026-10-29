// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { mount, type VueWrapper } from '@vue/test-utils';
import { createPinia } from 'pinia';
import {
  setPersistenceStorageProvider,
  type StorageLike,
} from '@/persistence';
import SnoozePicker from './SnoozePicker.vue';

let wrapper: VueWrapper | undefined;
let restoreProvider: (() => void) | undefined;

function memoryStorage(initial: Record<string, string> = {}): StorageLike & {
  values: Map<string, string>;
} {
  const values = new Map(Object.entries(initial));
  return {
    values,
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => values.set(key, value),
    removeItem: (key) => values.delete(key),
  };
}

afterEach(() => {
  wrapper?.unmount();
  wrapper = undefined;
  restoreProvider?.();
  restoreProvider = undefined;
  document.body.innerHTML = '';
  vi.useRealTimers();
});

describe('SnoozePicker', () => {
  it('teleports the fixed picker outside transformed or clipped callers', () => {
    const host = document.createElement('div');
    host.style.transform = 'translate3d(0, 0, 0)';
    host.style.overflow = 'hidden';
    document.body.append(host);

    wrapper = mount(SnoozePicker, {
      attachTo: host,
      props: { open: true },
      global: { plugins: [createPinia()] },
    });

    const picker = document.body.querySelector<HTMLElement>('.picker');
    expect(picker).not.toBeNull();
    expect(host.contains(picker)).toBe(false);
    expect(picker?.parentElement).toBe(document.body);
  });

  it('is a labelled dialog that takes the focus when it opens', async () => {
    wrapper = mount(SnoozePicker, {
      attachTo: document.body,
      props: { open: true },
      global: { plugins: [createPinia()] },
    });
    await wrapper.vm.$nextTick();

    const picker = document.body.querySelector<HTMLElement>('.picker')!;
    expect(picker.getAttribute('role')).toBe('dialog');
    const titleId = picker.getAttribute('aria-labelledby');
    expect(titleId).toBeTruthy();
    expect(document.getElementById(titleId!)?.textContent).toBe('Snooze');
    expect(picker.contains(document.activeElement)).toBe(true);
  });

  it('says so when the custom time is in the past instead of ignoring Apply', async () => {
    wrapper = mount(SnoozePicker, {
      attachTo: document.body,
      props: { open: true },
      global: { plugins: [createPinia()] },
    });
    const apply = () => document.body.querySelectorAll<HTMLButtonElement>('.actions button')[1]!;
    expect(apply().disabled).toBe(true);

    const input = document.body.querySelector<HTMLInputElement>('input[type="datetime-local"]')!;
    input.value = '2001-01-01T10:00';
    input.dispatchEvent(new Event('input'));
    await wrapper.vm.$nextTick();

    expect(document.body.querySelector('[role="alert"]')?.textContent).toContain('future');
    expect(input.getAttribute('aria-invalid')).toBe('true');
    expect(apply().disabled).toBe(true);
    expect(wrapper.emitted('choose')).toBeUndefined();
  });

  it('hydrates and persists recent manual dates through the persistence boundary', async () => {
    const storage = memoryStorage({
      'satchel.waitingRecentDates': '["2099-01-02T10:00:00.000Z"]',
    });
    restoreProvider = setPersistenceStorageProvider(() => storage);
    wrapper = mount(SnoozePicker, {
      attachTo: document.body,
      props: { open: true },
      global: { plugins: [createPinia()] },
    });

    expect(document.body.querySelectorAll('.recent-item')).toHaveLength(1);

    const input = document.body.querySelector<HTMLInputElement>('input[type="datetime-local"]');
    expect(input).not.toBeNull();
    if (!input) return;
    input.value = '2099-01-03T10:00';
    input.dispatchEvent(new Event('input'));
    await wrapper.vm.$nextTick();

    const apply = document.body.querySelectorAll<HTMLButtonElement>('.actions button')[1];
    expect(apply).toBeDefined();
    apply?.click();
    await wrapper.vm.$nextTick();

    expect(JSON.parse(storage.values.get('satchel.waitingRecentDates') ?? '[]')).toEqual([
      new Date('2099-01-03T10:00').toISOString(),
      '2099-01-02T10:00:00.000Z',
    ]);
  });
});


it('refreshes tomorrow and the manual minimum when the mounted picker reopens after midnight', async () => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date('2026-10-29T23:59:00'));
  wrapper = mount(SnoozePicker, { attachTo: document.body, props: { open: true }, global: { plugins: [createPinia()] } });
  const tomorrow = () => Array.from(document.querySelectorAll<HTMLButtonElement>('.preset')).find(button => button.textContent?.includes('Tomorrow'))!;
  // Read the preset before closing, so a computed cache already exists.
  expect(tomorrow()).toBeDefined();
  await wrapper.setProps({ open: false });
  vi.setSystemTime(new Date('2026-10-30T00:01:00'));
  await wrapper.setProps({ open: true });
  expect(document.querySelector('input')?.getAttribute('min')).toBe('2026-10-30T00:02');
  tomorrow().click();
  expect(wrapper.emitted('choose')?.[0]).toEqual([new Date('2026-10-31T08:00:00').toISOString()]);
});

it('removes expired recents and presets and explains a manual time expiring while open', async () => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date('2026-10-29T13:59:00'));
  restoreProvider = setPersistenceStorageProvider(() => memoryStorage({
    'satchel.waitingRecentDates': JSON.stringify([
      new Date('2026-10-29T13:58:00').toISOString(),
      new Date('2026-10-29T14:00:00').toISOString(),
    ]),
  }));
  wrapper = mount(SnoozePicker, { attachTo: document.body, props: { open: true }, global: { plugins: [createPinia()] } });
  expect(document.querySelectorAll('.recent-item')).toHaveLength(1);
  const input = document.querySelector<HTMLInputElement>('input')!;
  input.value = '2026-10-29T14:00';
  input.dispatchEvent(new Event('input'));
  await wrapper.vm.$nextTick();
  await vi.advanceTimersByTimeAsync(60_000);
  expect(document.querySelectorAll('.recent-item')).toHaveLength(0);
  expect(document.querySelector('.presets')?.textContent).not.toContain('Today');
  expect(document.querySelector('[role="alert"]')?.textContent).toContain('future');
  expect(wrapper.emitted('choose')).toBeUndefined();
});

it('explains a recent choice that expires between the last clock update and activation', async () => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date('2026-10-29T13:59:00'));
  const storage = memoryStorage({
    'satchel.waitingRecentDates': JSON.stringify([new Date('2026-10-29T14:00:00').toISOString()]),
  });
  restoreProvider = setPersistenceStorageProvider(() => storage);
  wrapper = mount(SnoozePicker, { attachTo: document.body, props: { open: true }, global: { plugins: [createPinia()] } });
  const recent = document.querySelector<HTMLButtonElement>('.recent-item')!;
  vi.setSystemTime(new Date('2026-10-29T14:00:00'));
  recent.click();
  await wrapper.vm.$nextTick();
  expect(wrapper.emitted('choose')).toBeUndefined();
  expect(wrapper.emitted('close')).toBeUndefined();
  expect(document.querySelector('[role="alert"]')?.textContent).toContain('future');
});


it('keeps a valid manual choice available after an expired recent choice', async () => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date('2026-10-29T13:59:00'));
  const storage = memoryStorage({
    'satchel.waitingRecentDates': JSON.stringify([new Date('2026-10-29T14:00:00').toISOString()]),
  });
  restoreProvider = setPersistenceStorageProvider(() => storage);
  wrapper = mount(SnoozePicker, { attachTo: document.body, props: { open: true }, global: { plugins: [createPinia()] } });
  const input = document.querySelector<HTMLInputElement>('input')!;
  input.value = '2026-10-29T14:30';
  input.dispatchEvent(new Event('input'));
  await wrapper.vm.$nextTick();
  const recent = document.querySelector<HTMLButtonElement>('.recent-item')!;
  vi.setSystemTime(new Date('2026-10-29T14:00:00'));
  recent.click();
  await wrapper.vm.$nextTick();
  expect(document.querySelector('[role="alert"]')?.textContent).toContain('future');
  expect(input.getAttribute('aria-invalid')).toBeNull();
  const apply = document.querySelectorAll<HTMLButtonElement>('.actions button')[1]!;
  expect(apply.disabled).toBe(false);
  apply.click();
  await wrapper.vm.$nextTick();
  expect(wrapper.emitted('choose')?.[0]).toEqual([new Date('2026-10-29T14:30').toISOString()]);
});
