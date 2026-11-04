// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { flushPromises, mount, type VueWrapper } from '@vue/test-utils';
import type { Message } from '@/types/mail';
import { useUiStore } from '@/stores/ui';
import MessageBubble from './MessageBubble.vue';

/*
 * On the phone the sender block of an open message (avatar, name, "To:" line)
 * is one target for the sender and recipient details: the name's button
 * stretches its hit area over the block (`.message-sender-group--target`), and
 * the message's ⋯ stays a separate control outside it.
 */

let wrapper: VueWrapper | null = null;

function message(): Message {
  return {
    id: 'm-1',
    threadId: 'thread-1',
    from: { name: 'Sarah Kim', email: 'sarah@example.com' },
    to: [{ name: 'Tom', email: 'tom@example.com' }],
    subject: 'Staging login',
    bodyText: 'Body',
    bodyHtml: '<p>Body</p>',
    bodyLoaded: true,
    timestamp: '2026-09-03T10:00:03Z',
    isRead: true,
    attachments: [],
  };
}

function render(props: Record<string, unknown> = {}) {
  return mount(MessageBubble, {
    attachTo: document.body,
    props: { message: message(), canReply: true, ...props },
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

describe('the message header on the phone', () => {
  it('makes the whole sender block one target that opens the details', async () => {
    useUiStore().isMobile = true;
    wrapper = render();

    const group = wrapper.get('.message-sender-group');
    expect(group.classes()).toContain('message-sender-group--target');
    // Avatar, name and To line all sit inside the stretched target…
    expect(group.find('.avatar, [class*="avatar"]').exists()).toBe(true);
    expect(group.find('.sender-name-trigger').exists()).toBe(true);
    expect(group.find('.recipient-summary').text()).toContain('To: Tom');
    // …and the ⋯ does not: it keeps its own target.
    const more = wrapper.get('.message-header [aria-label="Message actions"]');
    expect(group.element.contains(more.element)).toBe(false);

    // The stretched control is the name's button; it names what it opens.
    const trigger = group.get('.sender-name-trigger');
    expect(trigger.attributes('aria-label')).toBe('Show details for Sarah Kim');
    await trigger.trigger('click');
    await flushPromises();

    expect(document.body.querySelector('.recipient-details')).not.toBeNull();
    // The tap does not also count as a tap on the message.
    expect(wrapper.emitted('toggleCollapsed')).toBeUndefined();
  });

  it('leaves a folded message to unfold on a tap', () => {
    useUiStore().isMobile = true;
    wrapper = render({ collapsed: true });

    expect(wrapper.get('.message-sender-group').classes()).not.toContain('message-sender-group--target');
  });

  it('keeps the desktop header as separate links', () => {
    useUiStore().isMobile = false;
    wrapper = render();

    expect(wrapper.get('.message-sender-group').classes()).not.toContain('message-sender-group--target');
  });
});
