import { describe, expect, it } from 'vitest';
import type { Message } from '@/types/mail';
import { threadMessageVisibility } from './threadMessageVisibility';

function message(id: string, options: { own?: boolean; draft?: boolean } = {}): Message {
  return {
    id,
    threadId: 'thread',
    from: {
      name: options.own ? 'Ich' : 'Lena',
      email: options.own ? 'me@example.com' : 'lena@example.com',
    },
    to: [],
    subject: 'Betreff',
    bodyText: id,
    timestamp: '2026-07-15T05:39:12Z',
    isRead: true,
    isDraft: options.draft,
    attachments: [],
  };
}

const isOwnMessage = (item: Message) => item.from.email === 'me@example.com';

describe('thread message visibility', () => {
  it('keeps an older draft below the latest real message and out of the hidden count', () => {
    const messages = [
      ...Array.from({ length: 48 }, (_, index) => message(`earlier-${index}`)),
      message('draft', { own: true, draft: true }),
      ...Array.from({ length: 6 }, (_, index) => message(`later-${index}`, { own: true })),
      message('latest'),
    ];

    const result = threadMessageVisibility(messages, {
      expanded: false,
      collapseToLatest: false,
      isOwnMessage,
    });

    expect(result.visibleMessages.map((item) => item.id)).toEqual(['latest']);
    expect(result.draftMessages.map((item) => item.id)).toEqual(['draft']);
    expect(result.hiddenMessageCount).toBe(54);
  });

  it('shows all real messages before drafts when the thread is expanded', () => {
    const result = threadMessageVisibility(
      [message('first'), message('draft', { own: true, draft: true }), message('latest')],
      { expanded: true, collapseToLatest: false, isOwnMessage },
    );

    expect(result.visibleMessages.map((item) => item.id)).toEqual(['first', 'latest']);
    expect(result.draftMessages.map((item) => item.id)).toEqual(['draft']);
    expect(result.hiddenMessageCount).toBe(0);
  });

  it('keeps the existing two-message context for the latest sent message', () => {
    const result = threadMessageVisibility(
      [message('received'), message('sent', { own: true }), message('draft', { own: true, draft: true })],
      { expanded: false, collapseToLatest: false, isOwnMessage },
    );

    expect(result.visibleMessages.map((item) => item.id)).toEqual(['received', 'sent']);
    expect(result.draftMessages.map((item) => item.id)).toEqual(['draft']);
    expect(result.hiddenMessageCount).toBe(0);
  });

  it('uses only the latest real message as neutral context in a draft-focused view', () => {
    const result = threadMessageVisibility(
      [message('received'), message('sent', { own: true }), message('draft', { own: true, draft: true })],
      { expanded: false, collapseToLatest: true, isOwnMessage },
    );

    expect(result.visibleMessages.map((item) => item.id)).toEqual(['sent']);
    expect(result.draftMessages.map((item) => item.id)).toEqual(['draft']);
    expect(result.hiddenMessageCount).toBe(1);
  });
});
