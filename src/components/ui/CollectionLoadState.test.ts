// @vitest-environment jsdom
import { mount } from '@vue/test-utils';
import { afterEach, expect, it, vi } from 'vitest';
import CollectionLoadState from './CollectionLoadState.vue';

afterEach(() => vi.useRealTimers());
it('delays visible refresh progress, preserves content geometry and removes it when settled', async () => {
  vi.useFakeTimers();
  const wrapper = mount(CollectionLoadState, { props: { loading: true, failed: false, hasContent: true, errorLabel: 'Failed' } });
  expect(wrapper.find('[role="status"]').exists()).toBe(false);
  await vi.advanceTimersByTimeAsync(159);
  expect(wrapper.find('[role="status"]').exists()).toBe(false);
  await vi.advanceTimersByTimeAsync(1);
  expect(wrapper.find('.collection-refresh[role="status"]').exists()).toBe(true);
  expect(wrapper.findAll('.skeleton-row')).toHaveLength(0);
  await wrapper.setProps({ loading: false });
  expect(wrapper.find('[role="status"]').exists()).toBe(false);
  wrapper.unmount();
});
