// @vitest-environment jsdom
import { beforeEach, describe, expect, it } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { mount } from '@vue/test-utils';
import { useSettingsStore } from '@/stores/settings';
import SwipeSettings from './SwipeSettings.vue';

describe('swipe settings', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    window.localStorage.clear();
  });

  it('shows the four default slots and updates a single assignment', async () => {
    const wrapper = mount(SwipeSettings);
    const settings = useSettingsStore();

    expect(wrapper.get('[data-swipe-setting="rightShort"]').element)
      .toHaveProperty('value', 'toggle-read');
    expect(wrapper.get('[data-swipe-setting="rightLong"]').element)
      .toHaveProperty('value', 'done');
    expect(wrapper.get('[data-swipe-setting="leftShort"]').element)
      .toHaveProperty('value', 'pin');
    expect(wrapper.get('[data-swipe-setting="leftLong"]').element)
      .toHaveProperty('value', 'trash');

    await wrapper.get('[data-swipe-setting="leftShort"]').setValue('aside');

    expect(settings.swipeActions.leftShort).toBe('aside');
    expect(wrapper.get('button.app-button').attributes()).not.toHaveProperty('disabled');
  });
});
