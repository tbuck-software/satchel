// @vitest-environment jsdom

import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import AccountAvatar from './AccountAvatar.vue';

describe('AccountAvatar', () => {
  it('shows the initial on the account colour', () => {
    const wrapper = mount(AccountAvatar, { props: { name: 'Rowing club', color: '#f0b232' } });
    expect(wrapper.text()).toBe('R');
    expect(wrapper.attributes('style')).toContain('--account-color: #f0b232');
    expect(wrapper.find('img').exists()).toBe(false);
  });

  it('shows two initials once there is room, and none when there is not', () => {
    expect(mount(AccountAvatar, { props: { name: 'Rowing club', size: 44 } }).text()).toBe('RC');
    expect(mount(AccountAvatar, { props: { name: 'Rowing club', size: 12 } }).text()).toBe('');
  });

  it('shows a chosen picture and falls back to initials if it does not load', async () => {
    const wrapper = mount(AccountAvatar, { props: { name: 'Work', avatar: 'data:image/png;base64,iVBORw0KGgo=' } });
    await wrapper.get('img').trigger('error');
    expect(wrapper.find('img').exists()).toBe(false);
    expect(wrapper.text()).toBe('W');
  });

  it('ignores a colour that is not #rrggbb and anything but a data: picture', () => {
    const wrapper = mount(AccountAvatar, { props: { name: 'Work', color: 'url(x)', avatar: 'https://example.test/me.png' } });
    expect(wrapper.attributes('style')).toContain('--account-color: #55a6ff');
    expect(wrapper.find('img').exists()).toBe(false);
  });

  it('dims a resting account and badges it with the moon', () => {
    const wrapper = mount(AccountAvatar, { props: { name: 'Work', resting: true } });
    expect(wrapper.classes()).toContain('account-avatar--resting');
    expect(wrapper.find('.account-avatar__badge svg').exists()).toBe(true);
  });
});
