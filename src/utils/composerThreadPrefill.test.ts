// @vitest-environment jsdom
import { describe, expect, it } from 'vitest';
import type { Message } from '@/types/mail';
import {
  findResendSource,
  resendEditorHtml,
  forwardThreadPrefill,
  normalizeOwnedAddresses,
  replyThreadPrefill,
} from './composerThreadPrefill';

function message(id: string, overrides: Partial<Message> = {}): Message {
  return {
    id,
    threadId: 'thread-1',
    from: { name: 'Lena', email: 'lena@example.com' },
    to: [],
    subject: 'Project update',
    bodyText: `Body ${id}`,
    timestamp: '2026-08-31T10:00:00Z',
    isRead: true,
    attachments: [],
    ...overrides,
  };
}

describe('composer thread prefill', () => {
  it('normalizes owned account and alias addresses', () => {
    expect(normalizeOwnedAddresses([' Me@Example.com ', '', 'alias@example.com'])).toEqual(
      new Set(['me@example.com', 'alias@example.com']),
    );
  });

  it('replies to the most recent external sender instead of the latest own message', () => {
    const messages = [
      message('external', {
        from: { name: 'Client', email: 'client@example.com' },
        replyTo: 'desk@example.com',
      }),
      message('own', {
        from: { name: 'Me', email: 'me@example.com' },
        subject: 'Re: Project update',
      }),
    ];

    expect(replyThreadPrefill(
      'reply',
      messages,
      'Thread subject',
      new Set(['me@example.com']),
    )).toEqual({
      subject: 'Re: Project update',
      to: ['desk@example.com'],
      cc: [],
      // In-Reply-To has to name the same message the recipients came from —
      // 'own' is our own sent copy and would hang the reply off the wrong node.
      replyToMessageId: 'external',
      body: expect.stringContaining('Body external'),
      bodyHtml: expect.stringContaining('Body own'),
    });
  });

  it('deduplicates reply-all recipients across To and Cc while excluding owned addresses', () => {
    const source = message('external', {
      from: { name: 'Client', email: 'client@example.com' },
      to: [
        { name: 'Me', email: 'ME@example.com' },
        { name: 'Partner', email: 'partner@example.com' },
        { name: 'Partner duplicate', email: 'PARTNER@example.com' },
      ],
      cc: [
        { name: 'Partner', email: 'partner@example.com' },
        { name: 'Observer', email: 'observer@example.com' },
      ],
    });

    expect(replyThreadPrefill(
      'reply-all',
      [source],
      '',
      new Set(['me@example.com']),
    )).toEqual({
      subject: 'Re: Project update',
      to: ['client@example.com', 'partner@example.com'],
      cc: ['observer@example.com'],
      replyToMessageId: 'external',
      body: expect.stringContaining('Body external'),
      bodyHtml: expect.stringContaining('<blockquote>'),
    });
  });

  it('builds forward content from the latest message and falls back to the thread subject', () => {
    const latest = message('latest', { subject: '', bodyText: 'Forward this' });
    const prefill = forwardThreadPrefill([latest], 'Thread subject');

    expect(prefill.subject).toBe('Fwd: Thread subject');
    expect(prefill.body).toContain('Forward this');
    expect(forwardThreadPrefill([], 'Thread subject')).toEqual({
      subject: 'Fwd: Thread subject',
      body: null,
      bodyHtml: null,
    });
  });

  it('quotes newest first, preserves nested history, and excludes unsent drafts', () => {
    const prefill = replyThreadPrefill('reply', [
      message('old', { bodyText: 'First message\n\n> History outside this thread' }),
      message('new', { bodyText: 'Latest message' }),
      message('draft', { isDraft: true, bodyText: 'Unsent private notes' }),
    ], '', new Set());
    expect(prefill.body!.indexOf('Latest message')).toBeLessThan(prefill.body!.indexOf('First message'));
    expect(prefill.body).toContain('> > History outside this thread');
    expect(prefill.body).not.toContain('Unsent private notes');
    expect(prefill.bodyHtml).not.toContain('Unsent private notes');
  });

  it.each(['reply', 'forward'] as const)('keeps existing quoted history once in a %s', (mode) => {
    const original = 'Can we meet on Tuesday?';
    const messages = [
      message('old', { bodyText: original }),
      message('new', { bodyText: `Tuesday works for me.\n\nOn Monday, Lena wrote:\n> ${original}` }),
    ];
    const prefill = mode === 'forward'
      ? forwardThreadPrefill(messages, '')
      : replyThreadPrefill('reply', messages, '', new Set());
    expect(prefill.body!.split(original)).toHaveLength(2);
    expect(prefill.bodyHtml!.split(original)).toHaveLength(2);
  });

  it('keeps messages whose links differ from the existing quote', () => {
    const messages = [
      message('old', { bodyText: 'Details: https://example.com/new' }),
      message('new', { bodyText: 'Updated details\n\nOn Monday, Lena wrote:\n> Details: https://example.com/old' }),
    ];
    const prefill = replyThreadPrefill('reply', messages, '', new Set());
    expect(prefill.body).toContain('https://example.com/new');
    expect(prefill.body).toContain('https://example.com/old');
  });

  it.each(['reply', 'forward'] as const)('safely quotes HTML-only mail in a %s', (mode) => {
    const messages = [message('html', {
      from: { name: '<img src="https://tracker.test/name">', email: 'sender@example.com' },
      bodyText: '',
      bodyHtml: '<style>tracking</style><h1>HTML-only content</h1><p>A &amp; B</p><img src="https://tracker.test/pixel">',
    })];
    const prefill = mode === 'forward'
      ? forwardThreadPrefill(messages, '')
      : replyThreadPrefill('reply', messages, '', new Set());
    expect(prefill.body).toContain('HTML-only content');
    expect(prefill.body).toContain('A & B');
    const doc = new DOMParser().parseFromString(prefill.bodyHtml!, 'text/html');
    expect(doc.querySelectorAll('img, style, script')).toHaveLength(0);
    expect(doc.body.textContent).toContain('A & B');
    expect(doc.body.textContent).not.toContain('tracking');
  });

  it('resends the latest owned message and otherwise uses the latest message', () => {
    const firstOwn = message('own-1', { from: { name: 'Me', email: 'me@example.com' } });
    const external = message('external');
    const lastOwn = message('own-2', { from: { name: 'Alias', email: 'alias@example.com' } });

    expect(findResendSource(
      [firstOwn, external, lastOwn],
      new Set(['me@example.com', 'alias@example.com']),
    )?.id).toBe('own-2');
    expect(findResendSource([firstOwn, external], new Set(['nobody@example.com']))?.id).toBe(
      'external',
    );
  });
});

describe('resendEditorHtml', () => {
  it('removes quoted, unquoted, entity-encoded and srcset tracking images before editor insertion', () => {
    const cleaned = resendEditorHtml(`<p>Keep the body</p><input type=image src=https://tracker.test/input><script src=https://tracker.test/script></script><img src=https://tracker.test/victim><img src="&#104;ttps://tracker.test/victim"><img src="cid:logo" srcset="https://tracker.test/victim 2x"><table background="https://tracker.test/victim"><tr><td style="background:url(https://tracker.test/victim)">Text</td></tr></table><style>@import 'https://tracker.test/victim';</style>`)!;
    const doc = new DOMParser().parseFromString(cleaned, 'text/html');
    expect(doc.querySelectorAll('input, script')).toHaveLength(0);
    expect(doc.querySelectorAll('img')).toHaveLength(1);
    expect(doc.querySelector('img')?.getAttribute('src')).toBe('cid:logo');
    expect(doc.querySelector('[srcset], [background], [style], style')).toBeNull();
    expect(cleaned).not.toContain('tracker.test');
    expect(cleaned).toContain('Keep the body');
  });
});
