// @vitest-environment jsdom
import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import UsageMeter from './UsageMeter.vue';

function fill(wrapper: ReturnType<typeof mount>) {
  return (wrapper.get('.usage-meter__fill').element as HTMLElement).style.width;
}

describe('UsageMeter', () => {
  it('is a named meter that reports its value', () => {
    const wrapper = mount(UsageMeter, { props: { value: 30, max: 120, label: 'Storage', valueText: '30 of 120' } });
    const meter = wrapper.get('[role="meter"]');

    expect(meter.attributes('aria-label')).toBe('Storage');
    expect(meter.attributes('aria-valuemin')).toBe('0');
    expect(meter.attributes('aria-valuemax')).toBe('120');
    expect(meter.attributes('aria-valuenow')).toBe('30');
    expect(meter.attributes('aria-valuetext')).toBe('30 of 120');
    expect(fill(wrapper)).toBe('25%');
  });

  it('keeps the bar inside its track when usage runs over or the limit is unknown', () => {
    expect(fill(mount(UsageMeter, { props: { value: 150, max: 100, label: 'x' } }))).toBe('100%');
    expect(mount(UsageMeter, { props: { value: 150, max: 100, label: 'x' } }).get('[role="meter"]').attributes('aria-valuenow')).toBe('100');
    expect(fill(mount(UsageMeter, { props: { value: 5, max: 0, label: 'x' } }))).toBe('0%');
  });

  it('takes its tone as a class', () => {
    expect(mount(UsageMeter, { props: { value: 1, max: 2, label: 'x', tone: 'danger' } }).classes()).toContain('usage-meter--danger');
  });
});
