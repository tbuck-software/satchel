// @vitest-environment jsdom

import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import SectionHeading from './SectionHeading.vue';

describe('SectionHeading', () => {
  it('sets a detail after the label, in its own quieter span', () => {
    const wrapper = mount(SectionHeading, {
      props: { label: 'Mail actions', detail: 'Staging login 500 after deployment' },
    });
    const label = wrapper.get('.section-heading__label');

    expect(label.text()).toBe('Mail actions · Staging login 500 after deployment');
    expect(label.classes()).toContain('section-heading__label--detailed');
    expect(wrapper.get('.section-heading__detail').text()).toBe('· Staging login 500 after deployment');
  });

  it('leaves a plain label alone', () => {
    const wrapper = mount(SectionHeading, { props: { label: 'Navigation' } });

    expect(wrapper.get('.section-heading__label').text()).toBe('Navigation');
    expect(wrapper.find('.section-heading__detail').exists()).toBe(false);
    expect(wrapper.get('.section-heading__label').classes()).not.toContain('section-heading__label--detailed');
  });
});
