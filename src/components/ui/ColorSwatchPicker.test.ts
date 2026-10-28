// @vitest-environment jsdom

import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import ColorSwatchPicker from './ColorSwatchPicker.vue';

const options = [
  { value: '#55a6ff', label: 'Blue' },
  { value: '#4cd68a', label: 'Green' },
  { value: '#f0b232', label: 'Amber' },
];

describe('ColorSwatchPicker', () => {
  it('is a radio group whose arrow keys choose the next swatch', async () => {
    const wrapper = mount(ColorSwatchPicker, {
      props: { modelValue: '#55A6FF', options, label: 'Colour' },
      attachTo: document.body,
    });
    const swatches = wrapper.findAll('[role="radio"]');
    expect(wrapper.get('[role="radiogroup"]').attributes('aria-label')).toBe('Colour');
    expect(swatches.map((swatch) => swatch.attributes('aria-checked'))).toEqual(['true', 'false', 'false']);
    expect(swatches.map((swatch) => swatch.attributes('tabindex'))).toEqual(['0', '-1', '-1']);

    (swatches[0]!.element as HTMLElement).focus();
    await swatches[0]!.trigger('keydown', { key: 'ArrowLeft' });
    expect(wrapper.emitted('update:modelValue')).toEqual([['#f0b232']]);
    wrapper.unmount();
  });

  it('does not report the colour that is already chosen', async () => {
    const wrapper = mount(ColorSwatchPicker, { props: { modelValue: '#4cd68a', options, label: 'Colour' } });
    await wrapper.findAll('[role="radio"]')[1]!.trigger('click');
    expect(wrapper.emitted('update:modelValue')).toBeUndefined();
  });

  it('marks a colour outside the palette on the custom swatch', async () => {
    const wrapper = mount(ColorSwatchPicker, {
      props: { modelValue: '#123456', options, label: 'Colour', customLabel: 'Custom colour' },
    });
    const custom = wrapper.get('input[type="color"]');
    expect(custom.attributes('aria-label')).toBe('Custom colour');
    expect(wrapper.get('.color-swatch--custom').classes()).toContain('color-swatch--checked');
    await custom.setValue('#ABCDEF');
    expect(wrapper.emitted('update:modelValue')).toEqual([['#abcdef']]);
  });

  it('renders menu radio items inside a menu and leaves the custom picker out', () => {
    const wrapper = mount(ColorSwatchPicker, {
      props: { modelValue: '#55a6ff', options, label: 'Colour', customLabel: 'Custom colour', menu: true },
    });
    expect(wrapper.findAll('[role="menuitemradio"]')).toHaveLength(3);
    expect(wrapper.find('input[type="color"]').exists()).toBe(false);
  });
});
