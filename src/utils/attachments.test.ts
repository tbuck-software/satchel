import { describe, expect, it } from 'vitest';
import { isListWorthyAttachment } from './attachments';
import type { Attachment } from '@/types/mail';

function attachment(overrides: Partial<Attachment>): Attachment {
  return {
    id: 'att-1',
    filename: 'datei.pdf',
    contentType: 'application/pdf',
    size: 100_000,
    ...overrides,
  };
}

describe('isListWorthyAttachment', () => {
  it('keeps ordinary file attachments', () => {
    expect(isListWorthyAttachment(attachment({}))).toBe(true);
  });

  it('drops nameless text/html body parts', () => {
    expect(
      isListWorthyAttachment(
        attachment({ filename: 'attachment', contentType: 'text/html', size: 4_000 }),
      ),
    ).toBe(false);
    expect(
      isListWorthyAttachment(
        attachment({ filename: '', contentType: 'text/plain', size: 887 }),
      ),
    ).toBe(false);
  });

  it('drops small inline images (signature logos)', () => {
    expect(
      isListWorthyAttachment(
        attachment({ filename: 'logo.png', contentType: 'image/png', size: 8_000, isInline: true }),
      ),
    ).toBe(false);
  });

  it('keeps large inline images (embedded screenshots)', () => {
    expect(
      isListWorthyAttachment(
        attachment({
          filename: 'screenshot.png',
          contentType: 'image/png',
          size: 250_000,
          contentId: '<cid-1>',
        }),
      ),
    ).toBe(true);
  });

  it('keeps a named text file that is a real attachment', () => {
    expect(
      isListWorthyAttachment(
        attachment({ filename: 'error-log.txt', contentType: 'text/plain', size: 12_000 }),
      ),
    ).toBe(true);
  });

  it('drops detached S/MIME signatures from aggregate attachment lists', () => {
    expect(
      isListWorthyAttachment(
        attachment({
          filename: 'smime.p7s',
          contentType: 'application/x-pkcs7-signature',
          size: 2_523,
        }),
      ),
    ).toBe(false);
  });
});
