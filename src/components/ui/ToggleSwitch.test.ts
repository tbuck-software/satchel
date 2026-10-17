// @vitest-environment jsdom

import { flushPromises, mount } from '@vue/test-utils';
import { describe, expect, it, vi } from 'vitest';
import ToggleSwitch from './ToggleSwitch.vue';

/**
 * A checkbox moves itself the moment it is clicked, and Vue only patches
 * `checked` back when the bound value changed. Three settings panes had built
 * their own `role="switch"` around that (UI-20); the assertions below are the
 * reason they no longer have to.
 */
describe('ToggleSwitch', () => {
  it('reports the value the user asked for', async () => {
    const wrapper = mount(ToggleSwitch, { props: { modelValue: false } });

    await wrapper.get('input').setValue(true);
    await flushPromises();

    expect(wrapper.emitted('update:modelValue')).toEqual([[true]]);
  });

  it('stays where it was when the handler refuses the change', async () => {
    const beforeChange = vi.fn().mockResolvedValue(false);
    const wrapper = mount(ToggleSwitch, { props: { modelValue: false, beforeChange } });
    const input = wrapper.get('input').element as HTMLInputElement;

    await wrapper.get('input').setValue(true);
    await flushPromises();

    expect(beforeChange).toHaveBeenCalledWith(true);
    expect(wrapper.emitted('update:modelValue')).toBeUndefined();
    expect(input.checked).toBe(false);
  });

  it('goes through when the handler agrees', async () => {
    const wrapper = mount(ToggleSwitch, {
      props: { modelValue: false, beforeChange: () => true },
    });

    await wrapper.get('input').setValue(true);
    await flushPromises();

    expect(wrapper.emitted('update:modelValue')).toEqual([[true]]);
  });

  it('does not move for a caller that ignores the event either', async () => {
    // Nobody binds the value back, so the switch has nothing to stand on.
    const wrapper = mount(ToggleSwitch, { props: { modelValue: true } });
    const input = wrapper.get('input').element as HTMLInputElement;

    await wrapper.get('input').setValue(false);
    await flushPromises();

    expect(input.checked).toBe(true);
  });

  it('names itself where the caption sits outside it', () => {
    const wrapper = mount(ToggleSwitch, { props: { modelValue: true, label: 'Töne' } });

    expect(wrapper.get('input').attributes('aria-label')).toBe('Töne');
    expect(wrapper.get('input').attributes('role')).toBe('switch');
  });

  it('will not move while disabled', async () => {
    const wrapper = mount(ToggleSwitch, { props: { modelValue: false, disabled: true } });

    expect(wrapper.get('input').attributes('disabled')).toBeDefined();
    expect(wrapper.classes()).toContain('toggle-switch--disabled');
  });
});
