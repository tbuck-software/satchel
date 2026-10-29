import { isValidRecipient } from '@/utils/recipients';

export interface ComposerSendState {
  initialized: boolean;
  sending: boolean;
  draftLoadFailed: boolean;
  hasAccount: boolean;
  to: readonly string[];
  cc: readonly string[];
  bcc: readonly string[];
  body: string;
  attachmentCount: number;
  scheduledFor: string | null;
}

export type ComposerSendBlocker =
  | { reason: 'loading' }
  | { reason: 'sending' }
  | { reason: 'draftLoadFailed' }
  | { reason: 'noAccount' }
  | { reason: 'noRecipients' }
  | { reason: 'invalidRecipients'; invalid: string[] }
  | { reason: 'empty' }
  | { reason: 'invalidSchedule' };

/**
 * Why Send is unavailable, or `null` when it is not.
 *
 * Any of To, Cc and Bcc is enough — the backend puts all three into the
 * envelope — and a message may consist of attachments alone.
 */
export function composerSendBlocker(state: ComposerSendState, now = Date.now()): ComposerSendBlocker | null {
  if (!state.initialized) return { reason: 'loading' };
  if (state.sending) return { reason: 'sending' };
  // A draft that could not be loaded must not be sent as an empty message.
  if (state.draftLoadFailed) return { reason: 'draftLoadFailed' };
  if (!state.hasAccount) return { reason: 'noAccount' };
  const recipients = [...state.to, ...state.cc, ...state.bcc];
  if (recipients.length === 0) return { reason: 'noRecipients' };
  // A chip the backend cannot turn into a mailbox must not reach the queue.
  const invalid = recipients.filter((recipient) => !isValidRecipient(recipient));
  if (invalid.length > 0) return { reason: 'invalidRecipients', invalid };
  if (state.body.trim().length === 0 && state.attachmentCount === 0) return { reason: 'empty' };
  if (state.scheduledFor != null) {
    const scheduled = new Date(state.scheduledFor).getTime();
    if (!Number.isFinite(scheduled) || scheduled <= now) return { reason: 'invalidSchedule' };
  }
  return null;
}
