// @vitest-environment jsdom
import { afterEach, describe, expect, it } from 'vitest';
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
