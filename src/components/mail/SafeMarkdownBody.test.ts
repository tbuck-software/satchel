// @vitest-environment jsdom
import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import SafeMarkdownBody from './SafeMarkdownBody.vue';

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

  it('opens loaded Markdown images in a dismissible lightbox', async () => {
    const wrapper = mount(SafeMarkdownBody, {
      attachTo: document.body,
      props: {
        text: '![architecture](https://images.example/architecture.png)',
        allowRemoteImages: true,
      },
    });

    await wrapper.get('button.md-image-button').trigger('click');

    const lightbox = document.body.querySelector<HTMLElement>('[role="dialog"][aria-label="Image preview"]');
    expect(lightbox).not.toBeNull();
    expect(lightbox?.querySelector('img')?.getAttribute('src'))
      .toBe('https://images.example/architecture.png');
    expect(document.activeElement).toBe(lightbox);

    lightbox?.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    await wrapper.vm.$nextTick();
    expect(document.body.querySelector('[role="dialog"][aria-label="Image preview"]')).toBeNull();

    wrapper.unmount();
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
