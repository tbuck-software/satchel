// @vitest-environment jsdom
import { createPinia } from 'pinia';
import { mount } from '@vue/test-utils';
import { afterEach, expect, it, vi } from 'vitest';
import SendLaterPicker from './SendLaterPicker.vue';

afterEach(() => vi.useRealTimers());

it('rejects a past custom schedule and explains how to fix it', async () => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date('2026-10-29T12:00:00'));
  const wrapper = mount(SendLaterPicker, { global: { plugins: [createPinia()] }, props: { value: null } });
  await wrapper.get('input').setValue('2026-10-29T11:59');
  await wrapper.get('.primary').trigger('click');
  expect(wrapper.emitted('select')).toBeUndefined();
  expect(wrapper.get('[role="alert"]').text()).toContain('future');
  expect(wrapper.get('input').attributes('aria-invalid')).toBe('true');
  wrapper.unmount();
});

it('rejects a time that expires while open, then accepts a new future time or clearing', async () => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date('2026-10-29T12:00:00'));
  const wrapper = mount(SendLaterPicker, { global: { plugins: [createPinia()] }, props: { value: '2026-10-29T12:01:00Z' } });
  await wrapper.get('input').setValue('2026-10-29T12:01');
  await vi.advanceTimersByTimeAsync(60_000);
  await wrapper.get('.primary').trigger('click');
  expect(wrapper.emitted('select')).toBeUndefined();
  expect(wrapper.get('[role="alert"]').text()).toContain('future');
  await wrapper.get('input').setValue('2026-10-29T13:00');
  await wrapper.get('.primary').trigger('click');
  expect(wrapper.emitted('select')?.[0]).toEqual([new Date('2026-10-29T13:00').toISOString()]);
  await wrapper.get('.link').trigger('click');
  expect(wrapper.emitted('select')?.[1]).toEqual([null]);
  wrapper.unmount();
});
