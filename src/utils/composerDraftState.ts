import type { BackendOutgoingAttachment } from '@/types/composer';
import { normalizeVisibleText, normalizedAddresses } from './composerNormalization';

export interface ComposerDraftState {
  accountId: string;
  fromAddress: string;
  to: readonly string[];
  cc: readonly string[];
  bcc: readonly string[];
  subject: string;
  body: string;
  bodyHtml: string | null;
  plainText: boolean;
  scheduledFor: string | null;
  signatureId: string | null;
  signatureVariant: 'auto' | 'long' | 'short' | 'none';
  signatureOverrideText: string | null;
  signatureOverrideHtml: string | null;
  attachments: readonly BackendOutgoingAttachment[];
}

function attachmentContentDigest(value: string): string {
  let first = 0x811c9dc5;
  let second = 0x9e3779b9;
  for (let index = 0; index < value.length; index += 1) {
    const code = value.charCodeAt(index);
    first = Math.imul(first ^ code, 0x01000193);
    second = Math.imul(second ^ code, 0x85ebca6b);
  }
  return [first, second]
    .map((part) => (part >>> 0).toString(16).padStart(8, '0'))
    .join('');
}

/** Stable representation used by the composer close guard. */
export function composerDraftSnapshot(state: ComposerDraftState): string {
  return JSON.stringify({
    accountId: state.accountId,
    fromAddress: state.fromAddress.toLowerCase(),
    to: normalizedAddresses(state.to),
    cc: normalizedAddresses(state.cc),
    bcc: normalizedAddresses(state.bcc),
    subject: normalizeVisibleText(state.subject),
    body: normalizeVisibleText(state.body),
    bodyHtml: state.bodyHtml,
    plainText: state.plainText,
    scheduledFor: state.scheduledFor,
    signatureId: state.signatureId,
    signatureVariant: state.signatureVariant,
    signatureOverrideText: state.signatureOverrideText,
    signatureOverrideHtml: state.signatureOverrideHtml,
    attachments: state.attachments.map((attachment) => ({
      filename: attachment.filename,
      mimeType: attachment.mime_type,
      inline: Boolean(attachment.inline),
      contentId: attachment.content_id ?? null,
      size: attachment.content_base64.length,
      contentDigest: attachmentContentDigest(attachment.content_base64),
    })),
  });
}

/** Whether closing this draft would discard something visible to the sender. */
export function hasMeaningfulComposerContent(state: ComposerDraftState): boolean {
  return normalizedAddresses([...state.to, ...state.cc, ...state.bcc]).length > 0
    || normalizeVisibleText(state.subject).length > 0
    || normalizeVisibleText(state.body).length > 0
    || state.attachments.length > 0;
}
