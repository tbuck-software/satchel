// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { mount, type VueWrapper } from '@vue/test-utils';
import type { Message } from '@/types/mail';
import MessageBubble from './MessageBubble.vue';

let wrapper: VueWrapper | null = null;

function htmlMessage(): Message {
  return {
    id: 'm-1',
    threadId: 'thread-1',
    from: { name: 'Sarah', email: 'sarah@example.com' },
    to: [{ name: 'Tom', email: 'tom@example.com' }],
    subject: 'Plan',
    bodyText: 'Body',
    bodyHtml: '<p>Body</p>',
    bodyLoaded: true,
    timestamp: '2026-09-03T10:00:03Z',
    isRead: true,
    attachments: [],
  };
}

function render(props: Record<string, unknown>) {
  return mount(MessageBubble, {
    attachTo: document.body,
    props: { message: htmlMessage(), canReply: true, ...props },
    global: {
      stubs: {
        EmailRenderer: { template: '<div class="email-renderer-stub" />' },
        SenderFilingMenu: true,
      },
    },
  });
}

beforeEach(() => {
  setActivePinia(createPinia());
});

afterEach(() => {
  wrapper?.unmount();
  wrapper = null;
  document.body.innerHTML = '';
});

describe('a folded or far-away message', () => {
  it('builds no renderer while folded, and opens from a click on the row', async () => {
    wrapper = render({ collapsed: true });
    expect(wrapper.find('.email-renderer-stub').exists()).toBe(false);
    expect(wrapper.find('.message-body-host').exists()).toBe(false);

    await wrapper.get('.message-collapsed-snippet').trigger('click');
    expect(wrapper.emitted('toggleCollapsed')).toHaveLength(1);

    // Its buttons keep their own meaning.
    await wrapper.get('.message-collapse-toggle').trigger('click');
    expect(wrapper.emitted('toggleCollapsed')).toHaveLength(2);
  });

  it('keeps a placeholder instead of the renderer away from the viewport', async () => {
    wrapper = render({ collapsed: false, bodyMounted: false });
    expect(wrapper.find('.email-renderer-stub').exists()).toBe(false);
    expect(wrapper.find('.message-body-placeholder').exists()).toBe(true);

    await wrapper.setProps({ bodyMounted: true });
    expect(wrapper.find('.email-renderer-stub').exists()).toBe(true);
    expect(wrapper.find('.message-body-placeholder').exists()).toBe(false);
  });
});
