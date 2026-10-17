// @vitest-environment jsdom
import { beforeEach, describe, expect, it } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { mount } from '@vue/test-utils';
import { nextTick } from 'vue';
import { useContactsStore } from '@/stores/contacts';
import Avatar from './Avatar.vue';

describe('sender avatar loading', () => {
  beforeEach(() => setActivePinia(createPinia()));

  it('shows a newly synced contact photo after the previous image failed', async () => {
    const contacts = useContactsStore();
    contacts.contacts = [{ email: 'ci@github.com', avatar_url: 'data:image/png;base64,broken' }] as never;
    const wrapper = mount(Avatar, { props: { name: 'GitHub', email: 'ci@github.com' } });
    await wrapper.get('img').trigger('error');
    expect(wrapper.find('img').exists()).toBe(false);

    contacts.contacts = [{ email: 'ci@github.com', avatar_url: 'data:image/png;base64,replacement' }] as never;
    await nextTick();
    expect(wrapper.find('img').exists()).toBe(true);
    expect(wrapper.get('img').attributes('src')).toBe('data:image/png;base64,replacement');
    wrapper.unmount();
  });

  it('keeps a cached favicon in logo mode while explicit photos use a circular crop', () => {
    useContactsStore().contacts = [{ email: 'ci@github.com', avatar_url: 'data:image/png;base64,logo', avatar_source: 'favicon' }] as never;
    const wrapper = mount(Avatar, { props: { name: 'GitHub', email: 'ci@github.com' } });
    expect(wrapper.classes()).toContain('avatar--favicon');
    expect(wrapper.get('img').attributes('src')).toBe('/avatars/github.svg');
    wrapper.unmount();

    const photo = mount(Avatar, { props: { name: 'GitHub', email: 'ci@github.com', imageUrl: 'data:image/png;base64,photo' } });
    expect(photo.classes()).toContain('avatar--provided');
    photo.unmount();
  });
});
