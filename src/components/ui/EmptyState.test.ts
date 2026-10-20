// @vitest-environment jsdom

import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import AppButton from './AppButton.vue';
import EmptyState from './EmptyState.vue';

/**
 * The kit had rolled its own primary button here — a different radius, a
 * different padding and `opacity: 0.9` where AppButton mixes the fill (UI-13).
 * A library that does not use itself cannot ask features to.
 */
describe('EmptyState', () => {
  it('takes its action from the kit', () => {
    const wrapper = mount(EmptyState, {
      props: { title: 'Nichts hier', actionLabel: 'Neu laden' },
    });
    const action = wrapper.findComponent(AppButton);

    expect(action.exists()).toBe(true);
    expect(action.props('variant')).toBe('primary');
    expect(action.text()).toBe('Neu laden');
  });

  it('reports the click to the caller', async () => {
    const wrapper = mount(EmptyState, {
      props: { title: 'Nichts hier', actionLabel: 'Neu laden' },
    });

    await wrapper.findComponent(AppButton).trigger('click');

    expect(wrapper.emitted('action')).toHaveLength(1);
  });

  it('shows nothing to press when there is nothing to do', () => {
    const wrapper = mount(EmptyState, { props: { title: 'Nichts hier' } });

    expect(wrapper.findComponent(AppButton).exists()).toBe(false);
  });

  it('renders a drawn icon in place of the default glyph', () => {
    const drawn = mount(EmptyState, {
      props: { title: 'Nichts hier' },
      slots: { icon: '<svg class="drawn" viewBox="0 0 16 16" />' },
    });

    expect(drawn.find('svg.drawn').exists()).toBe(true);
    expect(drawn.find('.app-icon').exists()).toBe(false);
  });

  /**
   * The default was a sparkles emoji, which screen readers read out and the
   * theme could not colour. The picture is decoration; the title is the news.
   */
  it('draws a kit glyph by default and hides the picture from assistive tech', () => {
    const wrapper = mount(EmptyState, { props: { title: 'Nichts hier' } });

    expect(wrapper.get('.icon').attributes('aria-hidden')).toBe('true');
    expect(wrapper.find('.icon .app-icon').exists()).toBe(true);
    expect(wrapper.get('.icon').text()).toBe('');
  });
});
