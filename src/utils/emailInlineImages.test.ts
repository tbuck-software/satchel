// @vitest-environment jsdom
import { describe, expect, it } from 'vitest';
import type { Attachment } from '@/types/mail';
import {
  applyInlineAttachmentSources,
  cidCandidates,
  cleanCid,
  missingInlineImageSource,
  pendingInlineImageCount,
} from './emailInlineImages';

function attachment(overrides: Partial<Attachment> = {}): Attachment {
  return {
    id: 'att-1',
    filename: 'logo.png',
    contentType: 'image/png',
    size: 128,
    contentId: 'abc123@satchel',
    dataUrl: 'data:image/png;base64,AAAA',
    ...overrides,
  };
}

describe('cleanCid', () => {
  it('strips the scheme, angle brackets and surrounding space', () => {
    expect(cleanCid('  cid:<abc@host>  ')).toBe('abc@host');
  });

  it('decodes percent-encoding', () => {
    expect(cleanCid('cid:image%20one@host')).toBe('image one@host');
  });

  /** A lone `%` is not valid encoding; decoding throws and the raw value wins. */
  it('keeps a malformed encoding rather than throwing', () => {
    expect(cleanCid('cid:100%@host')).toBe('100%@host');
  });
});

describe('cidCandidates', () => {
  it('offers the bare and bracketed spelling plus the filename', () => {
    expect(cidCandidates(attachment())).toEqual([
      'abc123@satchel',
      '<abc123@satchel>',
      'logo.png',
    ]);
  });

  it('falls back to the filename when there is no Content-ID', () => {
    expect(cidCandidates(attachment({ contentId: null }))).toEqual(['logo.png']);
  });
});

describe('applyInlineAttachmentSources', () => {
  it('rejects the issue 91 MIME payload without creating markup', () => {
    const contentType = 'image/png"><link rel=preconnect href=https://u1.t.example>';
    const output = applyInlineAttachmentSources('<img src="cid:x">', [
      attachment({ contentId: 'x', contentType, dataUrl: `data:${contentType};base64,AAAA` }),
    ]);
    const doc = new DOMParser().parseFromString(output, 'text/html');
    expect(doc.querySelectorAll('link, meta')).toHaveLength(0);
    expect(doc.querySelector('img')?.getAttribute('src')).not.toContain('preconnect');
  });

  it('leaves HTML without any cid: reference untouched', () => {
    const html = '<p>Hallo</p>';
    expect(applyInlineAttachmentSources(html, [attachment()])).toBe(html);
  });

  it('rewrites a reference to the attachment data URL', () => {
    const html = '<img src="cid:abc123@satchel">';
    const output = applyInlineAttachmentSources(html, [attachment()]);
    expect(output).toContain('data:image/png;base64,AAAA');
    expect(output).not.toContain('cid:abc123@satchel');
  });

  /**
   * Senders encode partially: the space becomes %20 while the `@` stays
   * literal. Matching a fixed list of candidate spellings missed exactly this
   * case, and the image fell through to the placeholder.
   */
  it.each([
    ['fully literal', 'cid:image one@host'],
    ['space encoded, @ literal', 'cid:image%20one@host'],
    ['fully encoded', 'cid:image%20one%40host'],
    ['bracketed', 'cid:<image one@host>'],
  ])('resolves a reference that is %s', (_label, reference) => {
    const output = applyInlineAttachmentSources(`<img src="${reference}">`, [
      attachment({ contentId: 'image one@host' }),
    ]);
    expect(output).toContain('data:image/png;base64,AAAA');
  });

  it('resolves a reference inside a CSS url() without eating the bracket', () => {
    const output = applyInlineAttachmentSources(
      '<div style="background:url(cid:abc123@satchel)">x</div>',
      [attachment()],
    );
    expect(output).toContain('url(data:image/png;base64,AAAA)');
  });

  it('resolves srcset candidates while retaining their density descriptors', () => {
    const output = applyInlineAttachmentSources(
      '<img src="cid:abc123@satchel" srcset="cid:abc123@satchel 1x, cid:other 2x">',
      [attachment(), attachment({ contentId: 'other', dataUrl: 'data:image/png;base64,BBBB' })],
    );
    const doc = new DOMParser().parseFromString(output, 'text/html');
    expect(doc.querySelector('img')?.getAttribute('srcset')).toBe(
      'data:image/png;base64,AAAA 1x, data:image/png;base64,BBBB 2x',
    );
  });

  it('resolves a reference made by filename', () => {
    const html = '<img src="cid:logo.png">';
    const output = applyInlineAttachmentSources(html, [attachment({ contentId: null })]);
    expect(output).toContain('data:image/png;base64,AAAA');
  });

  it('replaces every occurrence, not just the first', () => {
    const html = '<img src="cid:abc123@satchel"><img src="cid:abc123@satchel">';
    const output = applyInlineAttachmentSources(html, [attachment()]);
    expect(output.match(/data:image\/png/g)).toHaveLength(2);
  });

  it('ignores attachments that carry no bytes', () => {
    const html = '<img src="cid:abc123@satchel">';
    const output = applyInlineAttachmentSources(html, [attachment({ dataUrl: null })]);
    expect(output).not.toContain('base64');
    expect(output).toContain('data:image/svg+xml');
  });

  /**
   * Only images may be substituted. A PDF sharing a Content-ID with an image
   * reference must not end up as the src of an <img>.
   */
  it('ignores non-image attachments', () => {
    const html = '<img src="cid:abc123@satchel">';
    const output = applyInlineAttachmentSources(html, [
      attachment({ contentType: 'application/pdf', dataUrl: 'data:application/pdf;base64,JVBER' }),
    ]);
    expect(output).not.toContain('application/pdf');
    expect(output).toContain('data:image/svg+xml');
  });

  /**
   * An unresolved reference would render as a broken image. The placeholder is
   * a data URL, so showing it still costs no request.
   */
  it('replaces an unresolved reference with the local placeholder', () => {
    const output = applyInlineAttachmentSources('<img src="cid:missing@host">', []);
    expect(output).toContain('data:image/svg+xml');
    expect(output).toContain('Image%20unavailable');
    expect(output).not.toContain('cid:missing@host');
  });

  /** The reader passes the label in the user's language; markup in it stays text. */
  it('labels the placeholder with the caller\'s text', () => {
    const output = applyInlineAttachmentSources(
      '<img src="cid:missing@host">',
      [],
      missingInlineImageSource('Bild <b>nicht</b> verfügbar'),
    );
    expect(decodeURIComponent(output)).toContain('>Bild bnicht/b verfügbar</text>');
  });

  /** The whole point is that nothing is fetched from the network. */
  it('never introduces a remote URL', () => {
    const html = '<img src="cid:abc123@satchel"><img src="cid:missing@host">';
    const output = applyInlineAttachmentSources(html, [attachment()]);
    expect(output).not.toMatch(/src=["']https?:/);
  });

  it('is case-insensitive about the scheme', () => {
    const output = applyInlineAttachmentSources('<img src="CID:abc123@satchel">', [attachment()]);
    expect(output).toContain('data:image/png;base64,AAAA');
  });
});

describe('pendingInlineImageCount', () => {
  it('counts references still waiting for bytes, and missing parts only until listed', () => {
    const referenced = new Set(['abc123@satchel', 'gone@host']);
    const waiting = attachment({ dataUrl: null });
    expect(pendingInlineImageCount(referenced, [waiting], false)).toBe(2);
    expect(pendingInlineImageCount(referenced, [waiting], true)).toBe(1);
    expect(pendingInlineImageCount(referenced, [attachment()], true)).toBe(0);
  });
});
