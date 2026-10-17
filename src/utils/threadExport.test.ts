import { describe, expect, it } from 'vitest';
import type { Message } from '@/types/mail';
import {
  buildThreadExportDescriptor,
  formatThreadParticipantLine,
  renderLatestThreadMessageEml,
  renderThreadMarkdown,
  renderThreadText,
  sanitizeThreadFilename,
  sanitizeThreadHeader,
  selectThreadExportMessages,
} from './threadExport';

const labels = {
  from: 'From',
  to: 'To',
  cc: 'Cc',
  date: 'Date',
};

const formatDate = (date: Date) => date.toISOString();

function message(id: string, overrides: Partial<Message> = {}): Message {
  return {
    id,
    threadId: 'thread-1',
    from: { name: 'Lena', email: 'lena@example.com' },
    to: [{ name: 'Me', email: 'me@example.com' }],
    subject: 'Project update',
    bodyText: `Body ${id}`,
    timestamp: '2026-08-31T10:00:00.000Z',
    isRead: true,
    attachments: [],
    ...overrides,
  };
}

describe('thread export formatting', () => {
  it('selects only the store-last message for latest exports, including a trailing draft', () => {
    const messages = [message('first'), message('latest'), message('draft', { isDraft: true })];

    expect(selectThreadExportMessages(messages, 'latest').map(({ id }) => id)).toEqual(['draft']);
    expect(selectThreadExportMessages(messages, 'conversation').map(({ id }) => id)).toEqual([
      'first',
      'latest',
      'draft',
    ]);
    expect(selectThreadExportMessages(messages, 'conversation')).toBe(messages);
  });

  it('formats named and unnamed participants and omits an absent Cc line', () => {
    const mail = message('one', {
      from: { name: '', email: 'sender@example.com' },
      to: [{ name: 'Recipient', email: 'recipient@example.com' }],
      cc: [],
    });

    expect(formatThreadParticipantLine(mail, 'from')).toBe('sender@example.com');
    expect(formatThreadParticipantLine(mail, 'to')).toBe('Recipient <recipient@example.com>');
    expect(formatThreadParticipantLine(mail, 'cc')).toBe('');
    expect(renderThreadText('Subject', [mail], 'conversation', { labels, formatDate }))
      .not.toContain('Cc:');
  });

  it('renders the exact Markdown conversation format', () => {
    const messages = [
      message('first', {
        from: { name: 'Lena', email: 'lena@example.com' },
        to: [{ name: '', email: 'me@example.com' }],
        cc: [{ name: 'Team', email: 'team@example.com' }],
        bodyText: 'Hello\n\nWorld',
      }),
      message('second', {
        from: { name: 'Me', email: 'me@example.com' },
        bodyText: 'Reply',
        timestamp: '2026-08-31T11:30:00.000Z',
      }),
    ];

    expect(renderThreadMarkdown('Project update', messages, 'conversation', { labels, formatDate }))
      .toBe(
        '# Project update\n\n\n\n'
        + '## Lena <lena@example.com> · 2026-08-31T10:00:00.000Z\n\n'
        + '**From:** Lena <lena@example.com>  \n'
        + '**To:** me@example.com  \n'
        + '**Cc:** Team <team@example.com>  \n\n'
        + 'Hello\n\nWorld\n\n'
        + '## Me <me@example.com> · 2026-08-31T11:30:00.000Z\n\n'
        + '**From:** Me <me@example.com>  \n'
        + '**To:** Me <me@example.com>  \n\n'
        + 'Reply\n',
      );
  });

  it('renders the exact plain-text latest format', () => {
    const messages = [
      message('first', { bodyText: 'First' }),
      message('latest', {
        from: { name: 'Me', email: 'me@example.com' },
        cc: [{ name: 'Team', email: 'team@example.com' }],
        bodyText: 'Latest',
      }),
    ];

    expect(renderThreadText('Subject', messages, 'latest', { labels, formatDate }))
      .toMatchInlineSnapshot(`
        "Subject

        ========



        From: Me <me@example.com>
        To: Me <me@example.com>
        Cc: Team <team@example.com>
        Date: 2026-08-31T10:00:00.000Z

        Latest
        "
      `);
  });

  it('renders the latest message as an RFC 822 text EML with folded-header protection', () => {
    const mail = message('latest', {
      from: { name: 'Sender\nInjected', email: 'sender@example.com' },
      to: [{ name: 'Recipient', email: 'recipient@example.com' }],
      subject: 'Subject\r\nBcc: attacker@example.com',
      bodyText: 'Body',
      timestamp: '2026-08-31T10:00:00.000Z',
    });

    expect(renderLatestThreadMessageEml([mail])).toBe(
      'From: Sender Injected <sender@example.com>\r\n'
      + 'To: Recipient <recipient@example.com>\r\n'
      + 'Date: Mon, 31 Aug 2026 10:00:00 GMT\r\n'
      + 'Subject: Subject Bcc: attacker@example.com\r\n'
      + 'MIME-Version: 1.0\r\n'
      + 'Content-Type: text/plain; charset=UTF-8\r\n'
      + 'Content-Transfer-Encoding: 8bit\r\n'
      + '\r\n'
      + 'Body',
    );
  });

  it('preserves filename sanitization, fallback, truncation, MIME, and extensions', () => {
    expect(sanitizeThreadHeader('  Subject\r\nInjected  ')).toBe('Subject Injected');
    expect(sanitizeThreadFilename('  <Bad>:/\\Name?*  ', 'Untitled')).toBe('Bad Name');
    expect(sanitizeThreadFilename('   ', 'Untitled')).toBe('Untitled');
    expect(sanitizeThreadFilename('', 'Fallback'.repeat(30))).toHaveLength(90);

    const descriptor = buildThreadExportDescriptor(
      'markdown',
      '  <Bad>:/\\Name?*  ',
      [message('one')],
      { labels, formatDate, filenameFallback: 'Untitled' },
    );
    expect(descriptor).toMatchObject({
      filename: 'Bad Name.md',
      mime: 'text/markdown',
      extension: 'md',
    });
    expect(buildThreadExportDescriptor('eml', 'Subject', [message('one')], {
      labels,
      formatDate,
      filenameFallback: 'Untitled',
    })).toMatchObject({ mime: 'message/rfc822', extension: 'eml', filename: 'Subject.eml' });
    expect(buildThreadExportDescriptor('text', 'Subject', [message('one')], {
      labels,
      formatDate,
      filenameFallback: 'Untitled',
    })).toMatchObject({ mime: 'text/plain', extension: 'txt', filename: 'Subject.txt' });
  });

  it('keeps empty-message output stable', () => {
    expect(renderThreadMarkdown('Empty', [], 'conversation', { labels, formatDate }))
      .toBe('# Empty\n');
    expect(renderThreadText('Empty', [], 'conversation', { labels, formatDate }))
      .toBe('Empty\n\n========\n');
    expect(renderLatestThreadMessageEml([])).toBe('');
  });
});
