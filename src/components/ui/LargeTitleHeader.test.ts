// @vitest-environment jsdom

import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import LargeTitleHeader from './LargeTitleHeader.vue';

describe('LargeTitleHeader', () => {
  it('makes the list name the switcher, named for what it does', async () => {
    const wrapper = mount(LargeTitleHeader, {
      props: { title: 'Later', subtitle: 'tom@satchel.dev', switchLabel: 'Later, switch mailbox' },
      slots: { actions: '<span class="probe-action" />' },
    });

    const button = wrapper.get('button');
    expect(button.text()).toBe('Later');
    // The accessible name starts with the visible one (WCAG 2.5.3).
    expect(button.attributes('aria-label')).toBe('Later, switch mailbox');
    expect(button.attributes('aria-haspopup')).toBe('dialog');
    expect(wrapper.find('.probe-action').exists()).toBe(true);
    expect(wrapper.get('.large-title__subtitle').text()).toBe('tom@satchel.dev');

    await button.trigger('click');
    expect(wrapper.emitted('switch')).toHaveLength(1);
  });

  it('leaves the subtitle out when there is none', () => {
    const wrapper = mount(LargeTitleHeader, { props: { title: 'Inbox', switchLabel: 'Inbox, switch mailbox' } });

    expect(wrapper.find('.large-title__subtitle').exists()).toBe(false);
  });

  it('puts the status beside the subtitle, on its line', () => {
    const wrapper = mount(LargeTitleHeader, {
      props: { title: 'Inbox', subtitle: 'tom@satchel.dev', switchLabel: 'Inbox, switch mailbox' },
      slots: { status: '<span class="probe">Offline</span>' },
    });

    const meta = wrapper.get('.large-title__meta');
    expect(meta.get('.large-title__subtitle').text()).toBe('tom@satchel.dev');
    expect(meta.get('.probe').text()).toBe('Offline');
  });
});
