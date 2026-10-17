import { describe, expect, it, vi } from 'vitest';
import {
  buildMessageLink,
  installMessageLinkOpenHandler,
  messageLinkMarkdown,
  parseMessageLink,
  type MessageLink,
} from './messageLinks';

describe('satchel:// links', () => {
  it.each([
    'CAF=x+y/z@mail.gmail.com',
    'a(b)c@host',
    'weird #?% id@host',
    'Ünïcode@host',
  ])('round-trips the Message-ID %s', (messageId) => {
    const url = buildMessageLink({ kind: 'message', messageId });
    expect(url).not.toMatch(/[()\s#?]/);
    expect(parseMessageLink(url)).toEqual({ kind: 'message', messageId });
  });

  it('keeps the @ readable', () => {
    expect(buildMessageLink({ kind: 'message', messageId: 'abc@mail.example.org' }))
      .toBe('satchel://message/abc@mail.example.org');
  });

  it('accepts what other apps do to a link', () => {
    expect(parseMessageLink(' SATCHEL://message/%3Cabc%40host%3E/ ')).toEqual({ kind: 'message', messageId: 'abc@host' });
    expect(parseMessageLink('satchel://message/abc@host?utm=1')).toEqual({ kind: 'message', messageId: 'abc@host' });
    expect(parseMessageLink('satchel://thread/th-1')).toEqual({ kind: 'thread', threadId: 'th-1' });
  });

  it.each([
    'mailto:abc@host',
    'satchel://message/',
    'satchel://message/%3C%3E',
    'satchel://message/%E0%A4%A',
    'satchel://folder/inbox',
    'de.tbuck.satchel://oauth/callback?code=1',
  ])('rejects %s', (url) => {
    expect(parseMessageLink(url)).toBeNull();
  });

  it('writes a Markdown link whose label cannot break out', () => {
    expect(messageLinkMarkdown('Re: [EXT] Invoice\n March', 'satchel://message/a@b'))
      .toBe('[Re: \\[EXT\\] Invoice March](satchel://message/a@b)');
    expect(messageLinkMarkdown('  ', 'satchel://message/a@b')).toBe('<satchel://message/a@b>');
  });

  it('opens links delivered to a running app and ignores other schemes', async () => {
    let deliver: (urls: string[]) => void = () => {};
    const opened: MessageLink[] = [];
    await installMessageLinkOpenHandler(
      (link) => opened.push(link),
      async () => ({
        onOpenUrl: vi.fn(async (handler) => {
          deliver = handler;
          return () => {};
        }),
        getCurrent: vi.fn(async () => null),
      }),
      null,
    );

    deliver(['mailto:a@b.de', 'satchel://message/abc@host']);

    expect(opened).toEqual([{ kind: 'message', messageId: 'abc@host' }]);
  });
});
