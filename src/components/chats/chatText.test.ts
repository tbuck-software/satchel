import { describe, expect, it } from 'vitest';
import type { Message } from '@/types/mail';
import { chatBubbleText, chatDays, chatSnippet, relativeDay } from './chatText';

function message(id: string, email: string, timestamp: string, bodyText: string): Message {
  return {
    id,
    threadId: 'th',
    from: { name: email.split('@')[0]!, email },
    to: [],
    subject: 'Message from x',
    bodyText,
    timestamp,
    isRead: true,
    attachments: [],
  };
}

describe('chatBubbleText', () => {
  it('drops the footer after the "-- " line and quoted lines, and says so', () => {
    expect(chatBubbleText('> earlier\n\nSee you\n-- \nSent with Delta Chat')).toEqual({ text: 'See you', cut: true });
    expect(chatBubbleText('Line one\r\nLine two')).toEqual({ text: 'Line one\nLine two', cut: false });
  });

  it('starts the footer only at "-- " with its space', () => {
    expect(chatBubbleText('Score\n--\n3:1')).toEqual({ text: 'Score\n--\n3:1', cut: false });
  });

  it('keeps the user\'s own quoted lines', () => {
    expect(chatBubbleText('> as I said\nyes', { keepQuotes: true })).toEqual({ text: '> as I said\nyes', cut: false });
  });

  it('keeps a message that is nothing but a quote', () => {
    expect(chatBubbleText('> only a quote')).toEqual({ text: '> only a quote', cut: false });
  });
});

describe('chatSnippet', () => {
  it('ends the preview where the one-line footer starts', () => {
    expect(chatSnippet('Passt, bis gleich -- Sent with my Delta Chat Messenger: https://delta.chat')).toBe('Passt, bis gleich');
    expect(chatSnippet('Nothing to cut')).toBe('Nothing to cut');
  });
});

describe('chatDays', () => {
  const own = new Set(['me@example.test']);

  it('orders by time, groups by day and marks own and continued bubbles', () => {
    const days = chatDays([
      message('b', 'me@example.test', '2026-03-02T10:05:00', 'Yes'),
      message('a', 'lena@example.test', '2026-03-02T10:00:00', 'Coming?'),
      message('c', 'me@example.test', '2026-03-02T10:06:00', 'At 7'),
      message('d', 'lena@example.test', '2026-03-03T09:00:00', 'Morning'),
    ], own);
    expect(days.map((day) => day.key)).toEqual(['2026-03-02', '2026-03-03']);
    expect(days[0]!.bubbles.map((bubble) => [bubble.id, bubble.own, bubble.continued])).toEqual([
      ['a', false, false],
      ['b', true, false],
      ['c', true, true],
    ]);
    expect(days[1]!.bubbles[0]!.continued).toBe(false);
  });

  it('leaves drafts out', () => {
    const draft = { ...message('x', 'me@example.test', '2026-03-02T10:00:00', 'draft'), isDraft: true };
    expect(chatDays([draft], own)).toEqual([]);
  });
});

describe('relativeDay', () => {
  it('names today and yesterday', () => {
    const now = new Date(2026, 2, 10, 12);
    expect(relativeDay(new Date(2026, 2, 10, 1), now)).toBe('today');
    expect(relativeDay(new Date(2026, 2, 9, 23), now)).toBe('yesterday');
    expect(relativeDay(new Date(2026, 2, 8, 23), now)).toBeNull();
  });
});
