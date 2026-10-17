import { describe, expect, it } from 'vitest';
import {
  attachmentDataUrl,
  hydrateCidReferences,
  inlineOnly,
  serializeCidReferences,
  type CidAttachment,
} from './composerInlineCids';

interface Attachment extends CidAttachment {
  filename: string;
  inline?: boolean;
}

function attachment(overrides: Partial<Attachment> = {}): Attachment {
  return {
    filename: 'logo.png',
    mime_type: 'image/png',
    content_base64: 'AAAA',
    content_id: 'logo@satchel',
    inline: true,
    ...overrides,
  };
}

const DATA_URL = 'data:image/png;base64,AAAA';

describe('attachmentDataUrl', () => {
  it('builds the data URL the editor can load', () => {
    expect(attachmentDataUrl(attachment())).toBe(DATA_URL);
  });
});

describe('hydrateCidReferences', () => {
  it('turns a cid: reference into a data URL', () => {
    const html = '<img src="cid:logo@satchel">';
    expect(hydrateCidReferences(html, [attachment()])).toBe(`<img src="${DATA_URL}">`);
  });

  it('replaces every occurrence', () => {
    const html = '<img src="cid:logo@satchel"><img src="cid:logo@satchel">';
    const output = hydrateCidReferences(html, [attachment()]) ?? '';
    expect(output.match(/data:image\/png/g)).toHaveLength(2);
  });

  it('skips attachments without a Content-ID', () => {
    const html = '<img src="cid:logo@satchel">';
    expect(hydrateCidReferences(html, [attachment({ content_id: null })])).toBe(html);
  });

  it('passes null and empty bodies through', () => {
    expect(hydrateCidReferences(null, [attachment()])).toBeNull();
    expect(hydrateCidReferences('', [attachment()])).toBe('');
  });
});

describe('serializeCidReferences', () => {
  it('turns a data URL back into a cid: reference', () => {
    const html = `<img src="${DATA_URL}">`;
    expect(serializeCidReferences(html, [attachment()])).toBe('<img src="cid:logo@satchel">');
  });

  it('passes null through', () => {
    expect(serializeCidReferences(null, [attachment()])).toBeNull();
  });
});

/**
 * The round trip has to be exact in both directions. A data URL left in the
 * body is sent as-is and arrives as a broken image; a `cid:` reference left in
 * the editor is broken for the sender.
 */
describe('the round trip', () => {
  const attachments = [
    attachment(),
    attachment({ filename: 'sig.png', content_id: 'sig@satchel', content_base64: 'BBBB' }),
  ];

  it('returns to the original markup', () => {
    const wire = '<p>Hallo</p><img src="cid:logo@satchel"><img src="cid:sig@satchel">';
    const hydrated = hydrateCidReferences(wire, attachments);
    expect(hydrated).not.toContain('cid:');
    expect(serializeCidReferences(hydrated, attachments)).toBe(wire);
  });

  it('leaves markup without any reference untouched', () => {
    const html = '<p>Nur Text</p>';
    expect(hydrateCidReferences(html, attachments)).toBe(html);
    expect(serializeCidReferences(html, attachments)).toBe(html);
  });

  /** An unrelated remote image must survive both passes unchanged. */
  it('does not touch a remote image', () => {
    const html = '<img src="https://example.com/pixel.png">';
    expect(serializeCidReferences(hydrateCidReferences(html, attachments), attachments)).toBe(html);
  });
});

describe('inlineOnly', () => {
  /**
   * A file attached normally keeps its Content-ID but must not be pulled into
   * the body: it belongs in the attachment list, not inline in the markup.
   */
  it('keeps inline attachments and drops the rest', () => {
    const attachments = [
      attachment({ filename: 'inline.png' }),
      attachment({ filename: 'anhang.pdf', inline: false }),
      attachment({ filename: 'unmarked.png', inline: undefined }),
    ];
    expect(inlineOnly(attachments).map((entry) => entry.filename)).toEqual(['inline.png']);
  });

  it('does not resolve a non-inline attachment into the body', () => {
    const html = '<img src="cid:logo@satchel">';
    const attachments = [attachment({ inline: false })];
    expect(hydrateCidReferences(html, inlineOnly(attachments))).toBe(html);
  });
});
