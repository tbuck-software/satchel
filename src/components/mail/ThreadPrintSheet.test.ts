// @vitest-environment jsdom
import { mount } from '@vue/test-utils';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Message } from '@/types/mail';
import ThreadPrintSheet from './ThreadPrintSheet.vue';

const messages: Message[] = [
  {
    id: 'message-1',
    threadId: 'thread-1',
    from: { name: 'Lena', email: 'lena@example.com' },
    to: [{ name: 'Me', email: 'me@example.com' }],
    subject: 'Design review',
    bodyText: 'First message',
    timestamp: '2026-08-13T09:00:00Z',
    isRead: true,
    attachments: [],
  },
  {
    id: 'message-2',
    threadId: 'thread-1',
    from: { name: 'Me', email: 'me@example.com' },
    to: [{ name: 'Lena', email: 'lena@example.com' }],
    subject: 'Design review',
    bodyText: 'Latest message',
    timestamp: '2026-08-13T10:00:00Z',
    isRead: true,
    attachments: [],
  },
];

afterEach(() => {
  document.body.innerHTML = '';
});

describe('ThreadPrintSheet', () => {
  it('does not render a print sheet while inactive', () => {
    const wrapper = mount(ThreadPrintSheet, {
      props: {
        active: false,
        subject: 'Design review',
        messages,
        formatDate: (value) => value,
        recipientLine: () => '',
      },
    });

    expect(document.body.querySelector('[data-thread-print]')).toBeNull();
    wrapper.unmount();
  });

  it('renders the subject and messages in conversation order', () => {
    const formatDate = vi.fn((value: string) => `formatted:${value}`);
    const recipientLine = vi.fn((message: Message) => message.to.map(({ email }) => email).join(', '));
    const wrapper = mount(ThreadPrintSheet, {
      props: {
        active: true,
        subject: 'Design review',
        messages,
        formatDate,
        recipientLine,
      },
    });

    const sheet = document.body.querySelector<HTMLElement>('[data-thread-print]');
    expect(sheet).not.toBeNull();
    expect(sheet?.querySelector('h1')?.textContent).toBe('Design review');
    expect(sheet?.querySelectorAll('.thread-print-message')).toHaveLength(2);
    expect(sheet?.textContent).toContain('First message');
    expect(sheet?.textContent).toContain('Latest message');
    expect(sheet?.textContent?.indexOf('First message')).toBeLessThan(
      sheet?.textContent?.indexOf('Latest message') ?? -1,
    );
    expect(sheet?.textContent).toContain('formatted:2026-08-13T09:00:00Z');
    expect(sheet?.textContent).toContain('To: me@example.com');
    expect(formatDate).toHaveBeenCalledTimes(2);
    expect(recipientLine).toHaveBeenCalledTimes(2);

    wrapper.unmount();
  });
});
