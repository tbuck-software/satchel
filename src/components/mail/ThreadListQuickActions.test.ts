// @vitest-environment jsdom
import { describe, expect, it } from 'vitest';
import { mount } from '@vue/test-utils';
import ThreadListQuickActions from './ThreadListQuickActions.vue';

/**
 * `title` alone is not an accessible name: assistive tech ranks it below
 * `aria-label`, some of it ignores the attribute outright, and a touchscreen
 * never shows the tooltip. A row of icons then reads as "button, button".
 */
describe('thread list quick actions', () => {
  it('offers only the shared Done action for a stack', async () => {
    const wrapper = mount(ThreadListQuickActions, { props: { bundleCount: 2 } });
    expect(wrapper.findAll('button')).toHaveLength(1);
    await wrapper.get('button').trigger('click');
    expect(wrapper.emitted('action')?.[0]?.[0]).toBe('done');
  });
  it('gives every icon button a name a screen reader reads out', () => {
    const wrapper = mount(ThreadListQuickActions);
    const buttons = wrapper.findAll('button');

    expect(buttons).toHaveLength(2);
    for (const button of buttons) {
      expect(button.attributes('aria-label')?.trim()).toBeTruthy();
      // Same source as the tooltip, so the two cannot drift apart.
      expect(button.attributes('aria-label')).toBe(button.attributes('title'));
    }
    expect(buttons.map((button) => button.attributes('aria-label'))).toEqual([
      'Snooze',
      'Done',
    ]);
  });
});
