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

  /*
   * The monograms are whole tiles. On the white logo plate they read as a
   * grey disc in a thick white ring, the brightest thing in a dark list.
   */
  it('fills the avatar with a monogram tile and keeps the plate for a bare logo', () => {
    const monogram = mount(Avatar, { props: { name: 'Google Cloud', email: 'billing@cloud.google.com' } });
    expect(monogram.classes()).toContain('avatar--favicon');
    expect(monogram.classes()).not.toContain('avatar--plate');
    expect(monogram.get('img').attributes('style') ?? '').not.toContain('width');
    monogram.unmount();

    const logo = mount(Avatar, { props: { name: 'Stripe', email: 'receipts@stripe.com' } });
    expect(logo.classes()).toContain('avatar--plate');
    expect(logo.attributes('style')).toContain('var(--email-canvas)');
    logo.unmount();
  });
});
