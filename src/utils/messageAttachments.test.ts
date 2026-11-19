// @vitest-environment jsdom
import { describe, expect, it } from 'vitest';
import type { Message } from '@/types/mail';
import { isSystemAttachment, listedAttachments } from './messageAttachments';

function signatureAttachment(contentType: string): Message['attachments'][number] {
  return {
    id: 'signature',
    filename: 'smime.p7s',
    contentType,
    size: 2523,
    isInline: false,
  };
}

describe('message system attachments', () => {
  it.each([
    'application/pkcs7-signature',
    'application/x-pkcs7-signature',
  ])('never exposes a %s signature through the normal attachment actions', (contentType) => {
    const attachment = signatureAttachment(contentType);

    expect(isSystemAttachment(attachment)).toBe(true);
  });

  it('also recognizes a signature with a generic content type by its filename', () => {
    expect(isSystemAttachment(signatureAttachment('application/octet-stream'))).toBe(true);
  });
});

describe('listed attachments', () => {
  function inline(id: string, contentId: string): Message['attachments'][number] {
    return { id, filename: `${id}.png`, contentType: 'image/png', size: 120_000, isInline: true, contentId };
  }

  it('lists an inline part the body never references', () => {
    // GroupWise forwards screenshots as inline parts and leaves them out of the HTML.
    const message = {
      attachments: [inline('logo', '<logo@x>'), inline('bild1', '<179501432138.316@th-koeln.de>')],
      bodyHtml: '<div>Liebe Kolleginnen</div><img src="cid:logo@x">',
    };

    expect(listedAttachments(message).map((attachment) => attachment.id)).toEqual(['bild1']);
  });

  it('lists an unreferenced inline part of a plain-text message', () => {
    expect(listedAttachments({ attachments: [inline('bild1', '<a@x>')] })).toHaveLength(1);
  });

  it('holds inline parts back until the body says which ones it shows', () => {
    expect(listedAttachments({ attachments: [inline('bild1', '<a@x>')], bodyLoaded: false })).toEqual([]);
  });

  it('counts a reference from a stylesheet, as the paperclip does', () => {
    const message = {
      attachments: [inline('banner', '<banner@x>')],
      bodyHtml: '<td style="background:url(cid:banner@x)">',
    };
    expect(listedAttachments(message)).toEqual([]);
  });

  it('keeps signatures and invites out of the list', () => {
    const parts = [
      { id: 'pgp', filename: 'signature.asc', contentType: 'application/pgp-signature', size: 1 },
      { id: 'invite', filename: 'invite.ics', contentType: 'text/calendar; method=REQUEST', size: 1 },
    ];
    expect(listedAttachments({ attachments: parts })).toEqual([]);
  });
});
