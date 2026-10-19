// @vitest-environment jsdom

import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import PaneResizeHandle from './PaneResizeHandle.vue';

function handle(value = 300) {
  return mount(PaneResizeHandle, { props: { label: 'Resize sidebar', value, min: 220, max: 420 } });
}

describe('PaneResizeHandle', () => {
  /** It was a mouse-only separator: no tab stop, no keys, no value. */
  it('is a focusable separator that reports its value and range', () => {
    const attributes = handle().attributes();

    expect(attributes.role).toBe('separator');
    expect(attributes.tabindex).toBe('0');
    expect(attributes['aria-orientation']).toBe('vertical');
    expect(attributes['aria-label']).toBe('Resize sidebar');
    expect(attributes['aria-valuenow']).toBe('300');
    expect(attributes['aria-valuemin']).toBe('220');
    expect(attributes['aria-valuemax']).toBe('420');
  });

  it('steps with the arrow keys, further with Shift, and jumps with Home and End', async () => {
    const wrapper = handle();

    await wrapper.trigger('keydown', { key: 'ArrowRight' });
    await wrapper.trigger('keydown', { key: 'ArrowLeft' });
    await wrapper.trigger('keydown', { key: 'ArrowRight', shiftKey: true });
    await wrapper.trigger('keydown', { key: 'Home' });
    await wrapper.trigger('keydown', { key: 'End' });
    await wrapper.trigger('keydown', { key: 'j' });

    expect(wrapper.emitted('resize')).toEqual([[316], [284], [364], [220], [420]]);
  });

  it('stops at the limits it was given', async () => {
    const wrapper = handle(410);

    await wrapper.trigger('keydown', { key: 'ArrowRight' });

    expect(wrapper.emitted('resize')).toEqual([[420]]);
  });

  it('follows a mouse drag from where it started', async () => {
    const wrapper = handle();

    await wrapper.trigger('mousedown', { clientX: 100 });
    window.dispatchEvent(new MouseEvent('mousemove', { clientX: 140 }));
    window.dispatchEvent(new MouseEvent('mouseup'));
    window.dispatchEvent(new MouseEvent('mousemove', { clientX: 200 }));

    expect(wrapper.emitted('resize')).toEqual([[340]]);
  });
});
