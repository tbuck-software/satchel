// @vitest-environment jsdom
import { enableAutoUnmount, mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useUiStore } from '@/stores/ui';
import SafeMarkdownBody from './SafeMarkdownBody.vue';

enableAutoUnmount(afterEach);
beforeEach(() => setActivePinia(createPinia()));

describe('SafeMarkdownBody', () => {
  it('renders declared image dimensions as a responsive blocked placeholder', () => {
    const wrapper = mount(SafeMarkdownBody, {
      props: {
        text: '![image](https://tracker.example/image.png){width=655 height=579}',
      },
    });

    const placeholder = wrapper.get('.md-blocked-image--sized');
    expect(placeholder.attributes('style')).toContain('width: min(100%, 655px)');
    expect(placeholder.attributes('style')).toContain('max-width: 100%');
    expect(placeholder.attributes('style')).toContain('aspect-ratio: 655 / 579');
    expect(placeholder.attributes('aria-label')).toBe('Image blocked: image');
    expect(wrapper.html()).not.toContain('tracker.example');
    expect(wrapper.text()).not.toContain('{width=');
  });

  it('loads a validated Markdown image only after remote images are allowed', async () => {
    const wrapper = mount(SafeMarkdownBody, {
      props: {
        text: '![diagram](https://images.example/diagram.png){width=655 height=579}',
      },
    });

    expect(wrapper.find('img').exists()).toBe(false);
    expect(wrapper.html()).not.toContain('images.example');

    await wrapper.setProps({ allowRemoteImages: true });

    const image = wrapper.get('img.md-image');
    const imageButton = wrapper.get('button.md-image-button');
    expect(image.attributes('src')).toBe('https://images.example/diagram.png');
    expect(image.attributes('alt')).toBe('diagram');
    expect(image.attributes('referrerpolicy')).toBe('no-referrer');
    expect(imageButton.attributes('style')).toContain('width: min(100%, 655px)');
    expect(imageButton.attributes('style')).not.toContain('aspect-ratio');
    expect(wrapper.find('.md-blocked-image').exists()).toBe(false);
  });

  it('replaces a failed image with a compact status', async () => {
    const wrapper = mount(SafeMarkdownBody, {
      props: {
        text: '![private diagram](https://git.example/private.png){width=900 height=700}',
        allowRemoteImages: true,
      },
    });

    await wrapper.get('img.md-image').trigger('error');

    const fallback = wrapper.get('.md-image-error');
    expect(wrapper.find('img.md-image').exists()).toBe(false);
    expect(wrapper.find('.md-image-button').exists()).toBe(false);
    expect(fallback.attributes('style')).toBeUndefined();
    expect(fallback.text()).toContain('Image could not be loaded: private diagram');
    expect(fallback.find('button').exists()).toBe(false);
    expect(fallback.text()).not.toContain('Open in browser');
    expect(wrapper.emitted('open-link')).toBeUndefined();

    await wrapper.setProps({
      text: '![image](https://git.example/generic.png){width=900 height=700}',
    });
    await wrapper.get('img.md-image').trigger('error');
    expect(wrapper.get('.md-image-error').text()).not.toContain(': image');

    await wrapper.setProps({
      text: '![replacement](https://git.example/replacement.png){width=900 height=700}',
    });
    expect(wrapper.get('img.md-image').attributes('src'))
      .toBe('https://git.example/replacement.png');
  });

  it('traps Tab and restores opener focus after Escape dismisses the lightbox', async () => {
    const ui = useUiStore();
    const backgroundDismiss = vi.fn();
    ui.registerOverlay({ isOpen: () => true, dismiss: backgroundDismiss });
    const wrapper = mount(SafeMarkdownBody, {
      attachTo: document.body,
      props: {
        text: '![architecture](https://images.example/architecture.png)',
        allowRemoteImages: true,
      },
    });

    const opener = wrapper.get('button.md-image-button');
    await opener.trigger('click');
    await wrapper.vm.$nextTick();

    const lightbox = document.body.querySelector<HTMLElement>('[role="dialog"][aria-label="Image preview"]')!;
    const closeButton = lightbox.querySelector<HTMLButtonElement>('button')!;
    expect(lightbox.querySelector('img')?.getAttribute('src'))
      .toBe('https://images.example/architecture.png');
    expect(ui.overlayDepth).toBe(2);
    expect(document.activeElement).toBe(lightbox);

    for (const shiftKey of [false, true, false]) {
      const tab = new KeyboardEvent('keydown', { key: 'Tab', bubbles: true, cancelable: true, shiftKey });
      document.activeElement?.dispatchEvent(tab);
      expect(tab.defaultPrevented).toBe(true);
      expect(document.activeElement).toBe(closeButton);
    }

    const onWindowEscape = vi.fn();
    const escape = new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true });
    window.addEventListener('keydown', onWindowEscape);
    try {
      closeButton.dispatchEvent(escape);
      expect(escape.defaultPrevented).toBe(true);
      expect(onWindowEscape).not.toHaveBeenCalled();
    } finally {
      window.removeEventListener('keydown', onWindowEscape);
    }
    await wrapper.vm.$nextTick();
    expect(document.body.querySelector('[role="dialog"][aria-label="Image preview"]')).toBeNull();
    expect(document.activeElement).toBe(opener.element);
    expect(ui.overlayDepth).toBe(1);
    expect(backgroundDismiss).not.toHaveBeenCalled();

    // Android Back uses the same stack. Unmounting an open preview removes its entry too.
    await opener.trigger('click');
    expect(ui.dismissTopmostOverlay()).toBe(true);
    await wrapper.vm.$nextTick();
    expect(document.body.querySelector('[role="dialog"][aria-label="Image preview"]')).toBeNull();
    expect(document.activeElement).toBe(opener.element);
    await opener.trigger('click');
    expect(ui.overlayDepth).toBe(2);
    wrapper.unmount();
    expect(ui.overlayDepth).toBe(1);
  });

  it('loads a same-instance GitLab upload after remote images are allowed', () => {
    const wrapper = mount(SafeMarkdownBody, {
      props: {
        text: [
          'Issue: https://git.example.net/group/project/-/issues/42',
          '',
          '![screenshot](/uploads/deadbeefdeadbeefdeadbeefdeadbeef/screenshot.png)',
        ].join('\n'),
        allowRemoteImages: true,
      },
    });

    expect(wrapper.get('img.md-image').attributes('src'))
      .toBe('https://git.example.net/uploads/deadbeefdeadbeefdeadbeefdeadbeef/screenshot.png');
  });

  it('never loads unsafe Markdown image protocols', () => {
    const wrapper = mount(SafeMarkdownBody, {
      props: {
        text: '![unsafe](javascript:alert(1))',
        allowRemoteImages: true,
      },
    });

    expect(wrapper.find('img').exists()).toBe(false);
    expect(wrapper.text()).toContain('Image blocked: unsafe');
    expect(wrapper.html()).not.toContain('javascript:');
  });
});
