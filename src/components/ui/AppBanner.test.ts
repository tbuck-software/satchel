// @vitest-environment jsdom

import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import AppBanner from './AppBanner.vue';

describe('AppBanner', () => {
  it('announces a failure instead of queueing it behind other output', () => {
    const wrapper = mount(AppBanner, { props: { tone: 'danger', title: 'Sync failed' } });

    expect(wrapper.get('.banner').attributes('role')).toBe('alert');
  });

  it('stays polite for warnings and notices', () => {
    for (const tone of ['warning', 'info'] as const) {
      const wrapper = mount(AppBanner, { props: { tone, title: 'Heads up' } });

      expect(wrapper.get('.banner').attributes('role')).toBe('status');
    }
  });
});
