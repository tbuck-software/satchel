// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Attachment } from '@/types/mail';
import { missingInlineImageSource } from './emailInlineImages';
import {
  clearPreparedEmailBodies,
  prepareEmailBody,
  preparedEmailBodyStats,
  type EmailBodyInput,
} from './preparedEmailBody';

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
    expect(after.document({ autoDark: false, allowRemoteImages: true }).html).toMatch(/src="blob:/);

    expect(prepareEmailBody('m-1', input({ html: '<p>Edited</p>', attachments: [logo] }))).not.toBe(after);
  });

  describe('inline images', () => {
    const logo: Attachment = {
      id: 'att', filename: 'logo.png', contentType: 'image/png', size: 4,
      contentId: 'logo@host', dataUrl: `data:image/png;base64,${'QUFB'.repeat(50_000)}`,
    };
    const options = { autoDark: false, allowRemoteImages: true };

    function stubObjectUrls() {
      let next = 0;
      const created = vi.spyOn(URL, 'createObjectURL').mockImplementation(() => `blob:satchel/${next++}`);
      const revoked = vi.spyOn(URL, 'revokeObjectURL').mockImplementation(() => {});
      return { created, revoked };
    }

    it('keeps the image bytes out of the document, the quotes and the cache', () => {
      const { created } = stubObjectUrls();
      const body = prepareEmailBody('m-1', input({
        html: '<p>Hi</p><img src="cid:logo@host"><blockquote type="cite"><p>Kannst du mir das Angebot bis Freitag schicken? Danke dir!</p><img src="cid:logo@host"></blockquote>',
        attachments: [logo],
      }));
      const document = body.document(options).html;
      const quote = body.quoteDocument(body.extracted.quotes[0]!, options);

      expect(document).toContain('src="blob:satchel/0"');
      expect(quote).toContain('src="blob:satchel/0"');
      expect(created).toHaveBeenCalledTimes(1);
      expect(body.extracted.currentHtml).toContain('cid:logo@host');
      expect(document + quote + (body.extracted.currentHtml ?? '')).not.toContain('QUFB');
      expect(preparedEmailBodyStats().chars).toBeLessThan(10_000);
    });

    it('revokes the URLs once neither the cache nor a renderer holds the body', () => {
      const { revoked } = stubObjectUrls();
      const body = prepareEmailBody('m-1', input({ attachments: [logo] }));
      body.document(options);
      const release = body.retain();

      clearPreparedEmailBodies();
      expect(revoked).not.toHaveBeenCalled();

      release();
      expect(revoked).toHaveBeenCalledWith('blob:satchel/0');
      // Asked again after that, it does not hand out the revoked URL.
      expect(body.document(options).html).toContain('data:image/png;base64,');
    });

    it('releases an uncached body when its renderer lets go', () => {
      const { revoked } = stubObjectUrls();
      const body = prepareEmailBody(undefined, input({ attachments: [logo] }));
      const release = body.retain();
      body.document(options);
      release();
      expect(revoked).toHaveBeenCalledTimes(1);
    });
  });

  it('builds a document again once the public-suffix table arrives, if a link waited for it', async () => {
    vi.resetModules();
    let release!: () => void;
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    vi.doMock('tldts', async () => {
      const actual = await vi.importActual<typeof import('tldts')>('tldts');
      await gate;
      return actual;
    });
    const fresh = await import('./preparedEmailBody');
    const links = await import('./externalLinks');

    const body = fresh.prepareEmailBody('links', input({
      html: '<p><a href="https://shop.example.com/x">example.com</a></p>',
    }));
    const plain = fresh.prepareEmailBody('plain', input({ html: '<p>No links</p>' }));
    const waiting = body.document({ autoDark: false, allowRemoteImages: false }).html;
    const plainDocument = plain.document({ autoDark: false, allowRemoteImages: false });
    expect(waiting).toContain('data-satchel-link-risk="true"');
    expect(links.publicSuffixTableLoaded()).toBe(false);

    release();
    await links.loadPublicSuffixTable();

    const settled = body.document({ autoDark: false, allowRemoteImages: false }).html;
    expect(settled).not.toContain('data-satchel-link-risk');
    expect(plain.document({ autoDark: false, allowRemoteImages: false })).toBe(plainDocument);

    vi.doUnmock('tldts');
    vi.resetModules();
  });

  it('keeps nothing without a cache key', () => {
    expect(prepareEmailBody(undefined, input())).not.toBe(prepareEmailBody(undefined, input()));
  });
});
