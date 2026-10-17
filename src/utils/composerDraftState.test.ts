import { describe, expect, it } from 'vitest';
import {
  composerDraftSnapshot,
  hasMeaningfulComposerContent,
  type ComposerDraftState,
} from './composerDraftState';

function state(overrides: Partial<ComposerDraftState> = {}): ComposerDraftState {
  return {
    accountId: 'account-1',
    fromAddress: 'Sender@Example.com',
    to: [],
    cc: [],
    bcc: [],
    subject: '',
    body: '',
    bodyHtml: null,
    plainText: false,
    scheduledFor: null,
    signatureId: null,
    signatureVariant: 'auto',
    signatureOverrideText: null,
    signatureOverrideHtml: null,
    attachments: [],
    ...overrides,
  };
}

describe('composerDraftSnapshot', () => {
  it('normalizes identity, recipients, and visible text', () => {
    const snapshot = JSON.parse(composerDraftSnapshot(state({
      to: ['  recipient@example.com  ', '\u200B'],
      cc: ['copy@example.com'],
      subject: '  Subject\u200B  ',
      body: ' Body\r\ntext ',
    })));

    expect(snapshot).toMatchObject({
      fromAddress: 'sender@example.com',
      to: ['recipient@example.com'],
      cc: ['copy@example.com'],
      subject: 'Subject',
      body: 'Body\ntext',
    });
  });

  it('captures a bounded attachment digest instead of its full payload', () => {
    const content = 'abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ';
    const serialized = composerDraftSnapshot(state({
      attachments: [{
        filename: 'image.png',
        mime_type: 'image/png',
        content_base64: content,
        inline: true,
        content_id: 'image-1',
      }],
    }));
    const snapshot = JSON.parse(serialized);

    expect(snapshot.attachments).toEqual([{
      filename: 'image.png',
      mimeType: 'image/png',
      inline: true,
      contentId: 'image-1',
      size: content.length,
      contentDigest: expect.stringMatching(/^[0-9a-f]{16}$/),
    }]);
    expect(serialized).not.toContain(content);
  });

  it('detects attachment changes hidden between identical edges', () => {
    const prefix = 'a'.repeat(24);
    const suffix = 'z'.repeat(24);
    const first = `${prefix}${'b'.repeat(32)}${suffix}`;
    const second = `${prefix}${'c'.repeat(32)}${suffix}`;
    const attachment = {
      filename: 'archive.bin',
      mime_type: 'application/octet-stream',
    };

    expect(composerDraftSnapshot(state({
      attachments: [{ ...attachment, content_base64: first }],
    }))).not.toBe(composerDraftSnapshot(state({
      attachments: [{ ...attachment, content_base64: second }],
    })));
  });

  it('preserves draft fields whose raw representation is meaningful', () => {
    const snapshot = JSON.parse(composerDraftSnapshot(state({
      bodyHtml: '<p>Hello</p>',
      plainText: true,
      scheduledFor: '2026-09-01T12:00:00Z',
      signatureId: 'signature-1',
      signatureVariant: 'short',
      signatureOverrideText: 'Regards',
      signatureOverrideHtml: '<p>Regards</p>',
    })));

    expect(snapshot).toMatchObject({
      bodyHtml: '<p>Hello</p>',
      plainText: true,
      scheduledFor: '2026-09-01T12:00:00Z',
      signatureId: 'signature-1',
      signatureVariant: 'short',
      signatureOverrideText: 'Regards',
      signatureOverrideHtml: '<p>Regards</p>',
    });
  });
});

describe('hasMeaningfulComposerContent', () => {
  it('ignores invisible-only recipient and text fields', () => {
    expect(hasMeaningfulComposerContent(state({
      to: [' ', '\u200B'],
      subject: '\uFEFF',
      body: '\u200B  ',
      bodyHtml: '<p><br></p>',
    }))).toBe(false);
  });

  it.each([
    ['recipient', { bcc: ['recipient@example.com'] }],
    ['subject', { subject: 'Hello' }],
    ['body', { body: 'Hello' }],
    ['attachment', {
      attachments: [{ filename: 'a.txt', mime_type: 'text/plain', content_base64: 'YQ==' }],
    }],
  ] satisfies Array<[string, Partial<ComposerDraftState>]>)('detects a meaningful %s', (_name, value) => {
    expect(hasMeaningfulComposerContent(state(value))).toBe(true);
  });
});
