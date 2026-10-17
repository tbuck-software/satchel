import { describe, expect, it } from 'vitest';
import type { DraftInput } from './generated/app/commands/drafts/DraftInput';
import type { OutgoingMessageInput } from './generated/app/commands/messages/OutgoingMessageInput';
import type { DeserializeWireInput } from './wireTypes';

type SendInput = DeserializeWireInput<OutgoingMessageInput, 'core::drafts::outgoing::OutgoingMessageInput'>;
type DraftWireInput = DeserializeWireInput<DraftInput, 'core::drafts::DraftInput'>;

const minimal: SendInput = {
  to: ['friend@example.test'],
  subject: 'hello',
  body: 'body',
  attachments: [{ filename: 'file.txt', mime_type: 'text/plain', content_base64: 'ZmlsZQ==' }],
};

const draftWithOmittedAttachmentDefaults: DraftWireInput = {
  account_id: 'account',
  attachments: [{ filename: 'file.txt', mime_type: 'text/plain', content_base64: 'ZmlsZQ==' }],
};
const draftWithExplicitAttachmentDefaults: DraftWireInput = {
  account_id: 'account',
  attachments: [{ filename: 'file.txt', mime_type: 'text/plain', content_base64: 'ZmlsZQ==', content_id: null, inline: false }],
};

// These assertions run through vue-tsc; defaults must not weaken names or scalar types.
const wrongName: SendInput = {
  to: [], subject: '', body: '',
  // @ts-expect-error Nested serde names remain snake_case.
  attachments: [{ filename: 'file.txt', mimeType: 'text/plain', content_base64: '' }],
};
const wrongScalar: SendInput = {
  to: [], subject: '', body: '',
  // @ts-expect-error A default bool accepts omission, but serde rejects null.
  attachments: [{ filename: 'file.txt', mime_type: 'text/plain', content_base64: '', inline: null }],
};
void wrongName;
void wrongScalar;

describe('deserialization input types', () => {
  it('retains omitted and explicit attachment defaults in draft snapshots', () => {
    expect(draftWithOmittedAttachmentDefaults.attachments?.[0]).not.toHaveProperty('content_id');
    expect(draftWithOmittedAttachmentDefaults.attachments?.[0]).not.toHaveProperty('inline');
    expect(draftWithExplicitAttachmentDefaults.attachments?.[0]).toMatchObject({ content_id: null, inline: false });
  });

  it('accepts omitted serde defaults without adding them to the payload', () => {
    expect(minimal.attachments).toEqual([
      { filename: 'file.txt', mime_type: 'text/plain', content_base64: 'ZmlsZQ==' },
    ]);
    expect(minimal).not.toHaveProperty('cc');
  });
});
