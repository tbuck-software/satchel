// @vitest-environment jsdom

import { enableAutoUnmount, flushPromises, mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import EmailRenderer from './EmailRenderer.vue';
import { useImagePolicy } from '@/composables/useImagePolicy';
import { useUiStore } from '@/stores/ui';
import { IMAGE_GUARD_PHONE_ACTIONS } from './imageGuardActions';
import ImageGuardPhoneActions from './ImageGuardPhoneActions.vue';

enableAutoUnmount(afterEach);

const stubs = { ExternalLinkDialog: true, LinkDestinationPreview: true, IsolatedEmailDocument: true };
const html = '<p>Angebote</p><img src="https://cdn.metro.de/a.png"><img src="https://cdn.metro.de/b.png">';

async function settle() {
  await flushPromises();
  await vi.dynamicImportSettled();
  await flushPromises();
}

/** The app shell provides the phone's buttons; the message-file viewer does not. */
async function mountNotice(senderEmail = 'newsletter@metro.de', provided = true) {
  const wrapper = mount(EmailRenderer, {
    props: { html, senderEmail },
    global: {
      stubs,
      provide: provided ? { [IMAGE_GUARD_PHONE_ACTIONS as symbol]: ImageGuardPhoneActions } : {},
    },
    attachTo: document.body,
  });
  await settle();
  return wrapper;
}

describe('EmailRenderer blocked-images notice', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    const policy = useImagePolicy();
    policy.setMode('allowlist');
    // The policy is app-wide state; an allow from one case leaks into the next.
    policy.revoke('newsletter@metro.de');
    policy.revoke('someone@gmail.com');
    policy.revokeDomain('metro.de');
    vi.stubGlobal('requestAnimationFrame', vi.fn(() => 1));
    vi.stubGlobal('cancelAnimationFrame', vi.fn());
  });

  afterEach(() => {
    document.body.innerHTML = '';
  });

  it('keeps the inline links on the desktop', async () => {
    useUiStore().isMobile = false;
    const guard = (await mountNotice()).get('.image-guard');

    expect(guard.classes()).not.toContain('image-guard--phone');
    expect(guard.findAll('.image-guard__action').map((action) => action.text()))
      .toEqual(['Show', 'Allow sender', 'Allow domain @metro.de']);
    expect(guard.findAll('.image-guard__sep')).toHaveLength(2);
  });

  it('is one row on the phone: a Show button and the always-allow choices behind one target', async () => {
    useUiStore().isMobile = true;
    const wrapper = await mountNotice();
    const guard = wrapper.get('.image-guard');

    expect(guard.classes()).toContain('image-guard--phone');
    expect(guard.find('.image-guard__sep').exists()).toBe(false);
    expect(guard.find('.image-guard__action').exists()).toBe(false);
    // A real button, not a text link: the kit gives it the 44pt hit area.
    expect(guard.get('.image-guard__show').classes()).toEqual(
      expect.arrayContaining(['app-button--soft', 'app-button--sm']),
    );
    const choices = guard.get('.image-guard__choices');
    expect(choices.attributes('aria-label')).toBe('Always load images');
    expect(choices.attributes('aria-haspopup')).toBe('dialog');
    expect(choices.attributes('aria-expanded')).toBe('false');

    await choices.trigger('click');
    await settle();

    expect(choices.attributes('aria-expanded')).toBe('true');
    const sheet = document.body.querySelector('.image-choices');
    expect(sheet?.textContent).toContain('Allow sender');
    expect(sheet?.textContent).toContain('newsletter@metro.de');
    expect(sheet?.textContent).toContain('Allow domain @metro.de');

    const allowSender = [...document.body.querySelectorAll<HTMLButtonElement>('.image-choices .menu-item')]
      .find((item) => item.textContent?.includes('Allow sender'));
    allowSender?.click();
    await flushPromises();

    expect(useImagePolicy().isAllowed('newsletter@metro.de')).toBe(true);
    // The notice goes with the block, and the sheet with it.
    expect(wrapper.find('.image-guard').exists()).toBe(false);
    expect(document.body.querySelector('.image-choices')).toBeNull();
  });

  it('shows the images once from the phone row without trusting anyone', async () => {
    useUiStore().isMobile = true;
    const wrapper = await mountNotice();

    await wrapper.get('.image-guard__show').trigger('click');

    expect(wrapper.find('.image-guard').exists()).toBe(false);
    expect(useImagePolicy().isAllowed('newsletter@metro.de')).toBe(false);
  });

  it('keeps the inline links where nobody provides the phone buttons (the viewer)', async () => {
    useUiStore().isMobile = true;
    const guard = (await mountNotice('newsletter@metro.de', false)).get('.image-guard');

    expect(guard.classes()).not.toContain('image-guard--phone');
    expect(guard.findAll('.image-guard__action')).toHaveLength(3);
  });

  it('offers no domain on a shared mailbox provider', async () => {
    useUiStore().isMobile = true;
    const wrapper = await mountNotice('someone@gmail.com');

    await wrapper.get('.image-guard__choices').trigger('click');
    await settle();

    const sheet = document.body.querySelector('.image-choices');
    expect(sheet?.textContent).toContain('Allow sender');
    expect(sheet?.textContent).not.toContain('Allow domain');
  });
});
