// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Attachment } from '@/types/mail';
import { missingInlineImageSource } from './emailInlineImages';
import { clearPreparedEmailBodies, prepareEmailBody, type EmailBodyInput } from './preparedEmailBody';

const html = '<div><p>Hallo</p><img src="https://tracker.example/p.gif"><img src="cid:logo@host"></div>'
  + '<blockquote type="cite"><p>Earlier message</p></blockquote>';

function input(overrides: Partial<EmailBodyInput> = {}): EmailBodyInput {
  return {
    html,
    text: '',
    attachments: [],
    normalizeQuotes: true,
    missingImageSource: missingInlineImageSource(),
    ...overrides,
  };
}

function countParses() {
  return vi.spyOn(DOMParser.prototype, 'parseFromString');
}

afterEach(() => {
  vi.restoreAllMocks();
  clearPreparedEmailBodies();
});

describe('prepareEmailBody', () => {
  it('parses a message once, however often its renderer remounts', () => {
    const parses = countParses();
    const first = prepareEmailBody('m-1', input());
    const document = first.document({ autoDark: false, allowRemoteImages: false });
    void first.markdownAvailable;
    const parsedOnce = parses.mock.calls.length;
    expect(parsedOnce).toBeGreaterThan(0);

    for (let remount = 0; remount < 5; remount += 1) {
      const again = prepareEmailBody('m-1', input());
      expect(again).toBe(first);
      expect(again.document({ autoDark: false, allowRemoteImages: false })).toBe(document);
      void again.markdownAvailable;
    }
    expect(parses.mock.calls.length).toBe(parsedOnce);
  });

  it('keeps one document per view mode and image permission', () => {
    const body = prepareEmailBody('m-1', input());
    const blocked = body.document({ autoDark: false, allowRemoteImages: false });
    const allowed = body.document({ autoDark: false, allowRemoteImages: true });
    const dark = body.document({ autoDark: true, allowRemoteImages: false });

    expect(blocked.blockedCount).toBe(1);
    expect(blocked.html).toContain('data-blocked-src="https://tracker.example/p.gif"');
    expect(blocked.html).not.toMatch(/\ssrc="https:\/\/tracker/);
    expect(allowed.blockedCount).toBe(0);
    expect(allowed.html).toContain('https://tracker.example/p.gif');
    expect(dark.html).not.toBe(blocked.html);
  });

  it('prepares again when the body or its inline images change', () => {
    const before = prepareEmailBody('m-1', input());
    expect(before.document({ autoDark: false, allowRemoteImages: true }).html).toContain('data:image/svg+xml');

    const logo: Attachment = {
      id: 'att', filename: 'logo.png', contentType: 'image/png', size: 4,
      contentId: 'logo@host', dataUrl: 'data:image/png;base64,AAAA',
    };
    const after = prepareEmailBody('m-1', input({ attachments: [logo] }));
    expect(after).not.toBe(before);
    expect(after.document({ autoDark: false, allowRemoteImages: true }).html).toContain('data:image/png;base64,AAAA');

    expect(prepareEmailBody('m-1', input({ html: '<p>Edited</p>', attachments: [logo] }))).not.toBe(after);
  });

  it('keeps nothing without a cache key', () => {
    expect(prepareEmailBody(undefined, input())).not.toBe(prepareEmailBody(undefined, input()));
  });
});
