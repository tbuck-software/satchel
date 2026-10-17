import { describe, expect, it } from 'vitest';
import type { Message } from '@/types/mail';
import { isSystemAttachment } from './messageAttachments';

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
