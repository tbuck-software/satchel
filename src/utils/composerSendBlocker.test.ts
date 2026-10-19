import { describe, expect, it } from 'vitest';
import { composerSendBlocker, type ComposerSendState } from './composerSendBlocker';

function state(overrides: Partial<ComposerSendState> = {}): ComposerSendState {
  return {
    initialized: true,
    sending: false,
    draftLoadFailed: false,
    hasAccount: true,
    to: [],
    cc: [],
    bcc: [],
    body: '',
    attachmentCount: 0,
    ...overrides,
  };
}

const attachment = { attachmentCount: 1 };
const text = { body: 'Hello' };

describe('composerSendBlocker', () => {
  it.each([
    ['To and text', { to: ['a@example.com'], ...text }],
    ['Cc only', { cc: ['a@example.com'], ...text }],
    ['Bcc only', { bcc: ['a@example.com'], ...text }],
    ['attachments without text', { to: ['a@example.com'], ...attachment }],
    ['Bcc and attachments only', { bcc: ['a@example.com'], ...attachment }],
  ] as const)('allows sending with %s', (_label, overrides) => {
    expect(composerSendBlocker(state(overrides))).toBeNull();
  });

  it.each([
    ['loading', { initialized: false, to: ['a@example.com'], ...text }],
    ['sending', { sending: true, to: ['a@example.com'], ...text }],
    ['draftLoadFailed', { draftLoadFailed: true, to: ['a@example.com'], ...text }],
    ['noAccount', { hasAccount: false, to: ['a@example.com'], ...text }],
    ['noRecipients', { ...text, ...attachment }],
    ['empty', { to: ['a@example.com'], body: '  \n ' }],
  ] as const)('reports %s', (reason, overrides) => {
    expect(composerSendBlocker(state(overrides))?.reason).toBe(reason);
  });

  it('names every invalid recipient across To, Cc and Bcc', () => {
    expect(composerSendBlocker(state({
      to: ['a@example.com', '"Buck'],
      bcc: ['not-an-address'],
      ...text,
    }))).toEqual({ reason: 'invalidRecipients', invalid: ['"Buck', 'not-an-address'] });
  });
});
