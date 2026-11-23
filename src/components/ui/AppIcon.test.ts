// @vitest-environment jsdom
import { mount } from '@vue/test-utils';
import { h } from 'vue';
import { describe, expect, it } from 'vitest';
import { Ban } from 'lucide';
import AppIcon from './AppIcon.vue';

function render(props: Record<string, unknown> = {}) {
  return mount(AppIcon, {
    props,
    slots: { default: () => h('path', { d: 'M3 8h10' }) },
  });
}

describe('AppIcon', () => {
  it.each([
    ['sm', '14'],
    ['md', '16'],
    ['lg', '20'],
  ])('renders %s at %spx on the 16 grid', (size, pixels) => {
    const svg = render({ size }).get('svg');

    expect(svg.attributes('width')).toBe(pixels);
    expect(svg.attributes('height')).toBe(pixels);
    expect(svg.attributes('viewBox')).toBe('0 0 16 16');
    expect(svg.attributes('stroke-width')).toBe('1.5');
  });

  it('is hidden from assistive technology; the button carries the name', () => {
    const svg = render().get('svg');

    expect(svg.attributes('aria-hidden')).toBe('true');
    expect(svg.attributes('focusable')).toBe('false');
  });

  it('draws the slot inside the frame', () => {
    expect(render().get('svg path').attributes('d')).toBe('M3 8h10');
  });

  it('scales a glyph drawn on a larger grid without thinning its line', () => {
    const group = render({ grid: 20 }).get('svg g');

    expect(group.attributes('transform')).toBe('scale(0.8)');
    expect(Number(group.attributes('stroke-width')) * 0.8).toBeCloseTo(1.5);
    expect(group.find('path').exists()).toBe(true);
  });

  it('draws a Lucide icon on its 24 grid at the shared line weight', () => {
    const wrapper = mount(AppIcon, { props: { icon: Ban, size: 'lg' } });
    const svg = wrapper.get('svg');
    const group = wrapper.get('svg g');

    expect(svg.attributes('width')).toBe('20');
    expect(svg.attributes('viewBox')).toBe('0 0 16 16');
    expect(group.attributes('transform')).toBe(`scale(${16 / 24})`);
    expect(Number(group.attributes('stroke-width')) * (16 / 24)).toBeCloseTo(1.5);
    expect(group.get('circle').attributes()).toMatchObject({ cx: '12', cy: '12', r: '10' });
    expect(group.get('path').attributes('d')).toBe(Ban[1]![1].d);
    expect(group.get('circle').element.namespaceURI).toBe('http://www.w3.org/2000/svg');
  });
});
